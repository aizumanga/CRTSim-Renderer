"""Compare the renderer's colour management with lcms, through ImageMagick, on real profiles.

Tags one picture with each RGB profile, makes a CMYK JPEG with an ICC profile, converts each to
sRGB with ImageMagick (lcms) and with `crtsim render --debug-dir` (whose clean.png, at the
native signal with nothing else changed, is the picture as read), and prints how far apart they
are. Needs ImageMagick 6 built with lcms, and the profiles of Debian's or Ubuntu's colord-data
and icc-profiles-free. Run from the repository root after `cargo build -p crtsim-cli`.
"""
import pathlib
import struct
import subprocess
import sys
import tempfile
import zlib

ICC = pathlib.Path('/usr/share/color/icc')
RGB = ['colord/AdobeRGB1998.icc', 'colord/ProPhotoRGB.icc', 'colord/AppleRGB.icc', 'colord/Rec709.icc']
CMYK = 'colord/FOGRA39L_coated.icc'
CLI = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/crtsim').resolve()


def tag(source, profile, out):
    """`source`, a PNG, with `profile` embedded in place of its own colour chunks."""
    data = source.read_bytes()

    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))

    tagged, at = data[:8], 8
    while at < len(data):
        size = struct.unpack('>I', data[at:at + 4])[0]
        kind, body = data[at + 4:at + 8], data[at + 8:at + 8 + size]
        at += 12 + size
        if kind in (b'cHRM', b'gAMA', b'sRGB', b'iCCP'):
            continue
        tagged += chunk(kind, body)
        if kind == b'IHDR':
            tagged += chunk(b'iCCP', b'profile\0\0' + zlib.compress(profile.read_bytes()))
    out.write_bytes(tagged)


def pixels(path):
    return subprocess.run(['convert', str(path), '-depth', '8', 'rgb:-'], check=True,
                          capture_output=True).stdout


def compare(name, source, work):
    reference = work / f'{name}-lcms.png'
    subprocess.run(['convert', str(source), '-profile', str(ICC / 'sRGB.icc'), '-depth', '8',
                    f'PNG24:{reference}'], check=True)
    debug = work / f'{name}-debug'
    subprocess.run([str(CLI), 'render', '-i', str(source), '-o', str(work / f'{name}-crt.png'),
                    '--signal', 'native', '--size', '64x48', '--debug-dir', str(debug)],
                   check=True, capture_output=True)
    ours, theirs = pixels(debug / 'clean.png'), pixels(reference)
    steps = sorted(max(abs(a - b) for a, b in zip(ours[i:i + 3], theirs[i:i + 3]))
                   for i in range(0, len(ours), 3))
    share = lambda limit: sum(s <= limit for s in steps) / len(steps) * 100
    print(f'{name:16} most {steps[-1]:2} steps; within 1: {share(1):5.1f}%, 2: {share(2):5.1f}%, '
          f'4: {share(4):5.1f}%')


with tempfile.TemporaryDirectory() as folder:
    work = pathlib.Path(folder)
    base = work / 'base.png'
    subprocess.run(['convert', '-size', '256x256', '-seed', '7', 'plasma:fractal', '(', '-size',
                    '256x64', 'gradient:red-blue', ')', '-append', '-depth', '8', f'PNG24:{base}'],
                   check=True)
    for profile in RGB:
        name = pathlib.Path(profile).stem
        tagged = work / f'{name}.png'
        tag(base, ICC / profile, tagged)
        compare(name, tagged, work)
    cmyk = work / 'cmyk.jpg'
    subprocess.run(['convert', str(base), '-profile', str(ICC / 'sRGB.icc'), '-profile',
                    str(ICC / CMYK), '-quality', '100', '-sampling-factor', '1x1', str(cmyk)],
                   check=True)
    compare('FOGRA39 CMYK', cmyk, work)
