#!/usr/bin/env python3
"""Read C strings / floats at virtual addresses of a 32-bit ELF. usage: elfstr.py ELF [s|f|i]:0xADDR ..."""
import struct, sys
d = open(sys.argv[1], 'rb').read()
phoff, = struct.unpack_from('<I', d, 28); phnum, = struct.unpack_from('<H', d, 44)
segs = []
for i in range(phnum):
    t, off, va, pa, fsz, msz = struct.unpack_from('<IIIIII', d, phoff + 32*i)
    if t == 1: segs.append((va, off, fsz))
def v2o(a):
    for va, off, fsz in segs:
        if va <= a < va + fsz: return off + a - va
    raise KeyError(hex(a))
for arg in sys.argv[2:]:
    kind, a = arg.split(':'); a = int(a, 16); o = v2o(a)
    if kind == 's': print(arg, repr(d[o:d.index(b'\0', o)].decode('latin1')))
    elif kind == 'f': print(arg, struct.unpack_from('<f', d, o)[0])
    elif kind == 'd': print(arg, struct.unpack_from('<d', d, o)[0])
    elif kind == 'i': print(arg, struct.unpack_from('<i', d, o)[0])
