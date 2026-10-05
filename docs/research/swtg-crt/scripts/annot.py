#!/usr/bin/env python3
"""Print one function from objdump output with rodata strings / float immediates annotated.
usage: annot.py ELF dis.txt FUNC_ADDR [--brief]   (--brief drops guard/unwind/dtor noise)"""
import re, struct, sys
elf, dis, start = sys.argv[1], sys.argv[2], sys.argv[3].lower().replace('0x', '')
brief = '--brief' in sys.argv
d = open(elf, 'rb').read()
phoff, = struct.unpack_from('<I', d, 28); phnum, = struct.unpack_from('<H', d, 44)
segs = [struct.unpack_from('<IIIIII', d, phoff+32*i) for i in range(phnum)]
def rd(a, n):
    for t, off, va, pa, fsz, msz in segs:
        if t == 1 and va <= a < va+fsz: return d[off+a-va: off+a-va+n]
def ann(v, ctx):
    notes = []
    if 0x83c0000 <= v < 0x8400000:
        b = rd(v, 80)
        if b:
            s = b.split(b'\0')[0]
            if len(s) >= 2 and all(32 <= c < 127 for c in s): notes.append(repr(s.decode()))
            elif 'ds:' in ctx or 'PTR' in ctx:
                notes.append('f=%g' % struct.unpack('<f', b[:4])[0])
    elif 'esp+' in ctx and 0x30000000 <= v < 0x50000000 or 0xb0000000 <= v < 0xd0000000:
        notes.append('f=%g' % struct.unpack('<f', struct.pack('<I', v))[0])
    return notes
# pre-pass: static NHashedString slots initialised from string literals (mov [esp+4],str; mov [esp],slot; call ctor)
hsnames = {}
_on = False; _last_str = None; _last_slot = None
for l in open(dis):
    if re.match(r'^0*%s <' % start, l): _on = True; continue
    if not _on: continue
    if l.strip() == '': break
    m = re.search(r'mov    DWORD PTR \[esp\+0x4\],(0x[0-9a-f]+)', l)
    if m: _last_str = int(m.group(1), 16)
    m = re.search(r'mov    DWORD PTR \[esp\],(0x84[0-9a-f]+)', l)
    if m: _last_slot = int(m.group(1), 16)
    if 'NHashedString::NHashedString(char const*' in l and _last_str and _last_slot:
        b = rd(_last_str, 80)
        if b: hsnames[_last_slot] = b.split(b'\0')[0].decode('latin1')
on = False
for l in open(dis):
    if re.match(r'^0*%s <' % start, l): on = True; print(l.rstrip()); continue
    if not on: continue
    if l.strip() == '': break
    if brief and re.search(r'__cxa_guard|_Unwind_Resume|NHashedString::~|nop|xchg   ax,ax|lea    esi,\[esi', l): continue
    notes = []
    for m in re.finditer(r'ds:(0x84[0-9a-f]+)', l):
        a = int(m.group(1), 16)
        for base in (a, a-8):
            if base in hsnames: notes.append('HS<%s>' % hsnames[base] + ('' if base == a else '.hash'))
    for m in re.finditer(r'0x([0-9a-f]{7,8})', l):
        notes += ann(int(m.group(1), 16), l)
    l = l.rstrip()
    l = re.sub(r'\(NShaderParamMapping\*, NHashedString, NShaderParamInformant\*\)', '(..)', l)
    print(l + ('    ; ' + ' '.join(notes) if notes else ''))
