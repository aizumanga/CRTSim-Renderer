#!/usr/bin/env python3
"""Recover NConfigFile registrations from objdump -M intel output of a registerConfigVars() function.
Tracks a tiny symbolic state (registers, [esp+N] slots, NHashedString objects) linearly; no branches are modelled,
which suffices for these straight-line registration functions. usage: configvars.py ELF dis.txt FUNC_ADDR"""
import re, struct, subprocess, sys
elf, dis, start = sys.argv[1], sys.argv[2], sys.argv[3].lower().lstrip('0x')
d = open(elf, 'rb').read()
phoff, = struct.unpack_from('<I', d, 28); phnum, = struct.unpack_from('<H', d, 44)
segs = [struct.unpack_from('<IIIIII', d, phoff+32*i) for i in range(phnum)]
def rd(a, n):
    for t, off, va, pa, fsz, msz in segs:
        if t == 1 and va <= a < va+fsz: return d[off+a-va: off+a-va+n]
def cstr(a):
    b = rd(a, 256); return b[:b.index(b'\0')].decode('latin1') if b else hex(a)
f32 = lambda v: round(struct.unpack('<f', struct.pack('<I', v & 0xffffffff))[0], 6)
syms = {}
for line in subprocess.run(['nm', '-C', elf], capture_output=True, text=True).stdout.splitlines():
    p = line.split(' ', 2)
    if len(p) == 3 and p[0]: syms[int(p[0], 16)] = p[2]
lines, on = [], False
for l in open(dis):
    if re.match(r'^0*%s <' % start, l): on = True; continue
    if on and l.strip() == '': break
    if on: lines.append(l.rstrip())
reg, stk, hs = {}, {}, {}
def val(op):
    op = op.strip()
    m = re.fullmatch(r'0x[0-9a-f]+', op)
    if m: return int(op, 16)
    m = re.fullmatch(r'DWORD PTR \[esp\+(0x[0-9a-f]+)\]', op) or re.fullmatch(r'DWORD PTR \[esp\]()', op)
    if m: return stk.get(int(m.group(1) or '0', 16))
    if op in reg: return reg[op]
    m = re.fullmatch(r'DWORD PTR ds:(0x[0-9a-f]+)', op)
    if m: return ('mem', int(m.group(1), 16))
    return ('?', op)
def slot(op):
    m = re.fullmatch(r'DWORD PTR \[esp(?:\+(0x[0-9a-f]+))?\]', op.strip())
    return int(m.group(1) or '0', 16) if m else None
def show(v):
    if isinstance(v, tuple) and v[0] == 'stack': return hs.get(v[1], '<stack %#x>' % v[1])
    if isinstance(v, tuple) and v[0] == 'this': return 'this+%#x' % v[1]
    return v
out = []
for l in lines:
    m = re.match(r'\s*([0-9a-f]+):\s+(\S+)\s*(.*)', l)
    if not m: continue
    addr, mn, ops = m.groups()
    if mn == 'lea':
        r, src = ops.split(',', 1)
        m2 = re.fullmatch(r'\[(esp|edi|ebx|esi|eax|ebp)(?:\+(0x[0-9a-f]+))?\]', src)
        if m2:
            base, off = m2.group(1), int(m2.group(2) or '0', 16)
            reg[r] = ('stack', off) if base == 'esp' else ('this', off) if base == 'edi' else ('?', src)
    elif mn == 'mov':
        dst, src = ops.split(',', 1)
        s = slot(dst)
        if s is not None: stk[s] = val(src)
        else: reg[dst] = val(src)
    elif mn == 'call':
        tgt = ops.split('<', 1)[-1].rstrip('>')
        if 'NHashedString::NHashedString(char const*' in tgt:
            o = stk.get(0); 
            if isinstance(o, tuple) and o[0] == 'stack': hs[o[1]] = cstr(stk[4])
        elif 'RegisterConfigVariable' in tgt or 'InstrumentForSlider' in tgt:
            a = [stk.get(i) for i in range(0, 0x30, 4)]
            sec, name = show(a[1]), show(a[2])
            kind = tgt.split('(')[1].split(')')[0].split(',')
            if 'InstrumentForSlider' in tgt:
                out.append(f'  slider {sec}.{name}: step={f32(a[3])} min={f32(a[4])} max={f32(a[5])}')
            else:
                t = kind[2].strip()
                ptr = show(a[3])
                if t == 'float*': dv = f32(a[4]); cb = a[5]
                elif t in ('int*',): dv = a[4]; cb = a[5]
                elif t == 'bool*': dv = a[4] & 0xff if isinstance(a[4], int) else a[4]; cb = a[5]
                elif t == 'NVec4*': dv = tuple(f32(x) if isinstance(x, int) else x for x in a[4:8]); cb = a[8]
                else: dv = show(a[4]); cb = a[5]
                cbn = syms.get(cb, cb) if isinstance(cb, int) and cb else ''
                out.append(f'{sec}.{name} [{t[:-1]}] {ptr} default={dv} {cbn}')
        reg.pop('eax', None)
print('\n'.join(out))
