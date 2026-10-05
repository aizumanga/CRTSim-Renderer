#!/usr/bin/env python3
"""List/extract NERD .npk v2 packages (layout inferred from ValkyrieVersion.npk).
Header: b'.npk', u32 version(2), u32 index_offset, u32 index_size.
Index: entries of 256 bytes: char name[248], u32 offset, u32 size."""
import struct, sys, os
def entries(path):
    d = open(path, 'rb').read()
    magic, ver, ioff, isz = struct.unpack_from('<4sIII', d, 0)
    assert magic == b'.npk' and ver == 2, (magic, ver)
    assert ioff + isz == len(d), (ioff, isz, len(d))
    for i in range(isz // 256):
        e = d[ioff + i*256: ioff + (i+1)*256]
        name = e[:248].split(b'\0')[0].decode('latin1')
        off, sz = struct.unpack_from('<II', e, 248)
        yield name, off, sz, d[off:off+sz]
if __name__ == '__main__':
    path, out = sys.argv[1], (sys.argv[2] if len(sys.argv) > 2 else None)
    for name, off, sz, blob in entries(path):
        print(f'{off:10d} {sz:9d}  {name}')
        if out:
            p = os.path.join(out, name.replace('\\', '/'))
            os.makedirs(os.path.dirname(p) or out, exist_ok=True)
            open(p, 'wb').write(blob)
