"""Lay a published release's packages out as itch.io channels, one folder each, for butler to push.

    python tools/itch.py --assets DIR --version v1.5.0 --out DIR [--only CHANNEL ...]

DIR holds the release's downloads, as `gh release download` leaves them. Each channel's folder is
what an itch.io download of it holds: the desktop app unpacked, with an .itch.toml telling the
itch app what to launch, or a package as people install it. It prints one line per channel,
its name and its folder, separated by a tab; a package the release does not have is skipped,
saying so on standard error. itch.io tells a channel's system by its name: windows, mac, linux.
"""
import argparse
import pathlib
import shutil
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
parser.add_argument('--assets', type=pathlib.Path, required=True)
parser.add_argument('--version', required=True)
parser.add_argument('--out', type=pathlib.Path, required=True, help='a NEW folder')
parser.add_argument('--only', nargs='*', help='only these channels')
args = parser.parse_args()
name = f'CRTSim-Renderer-{args.version}'

# The channel, the release's file it is made from, and how: unpacked, the file as it is, or
# unpacked with what the itch app launches.
CHANNELS = [
    ('windows', f'{name}-windows-x86_64.zip', 'crtsim-desktop.exe'),
    ('mac', f'{name}-macos-arm64.tar.gz', 'CRTSim Renderer.app'),
    # The AppImage carries the libraries the tarball leaves to the system.
    ('linux', f'{name}-linux-x86_64.AppImage', f'{name}-linux-x86_64.AppImage'),
    # Played in the browser once its upload is marked so on itch.io.
    ('web', f'{name}-web.zip', None),
    ('editors-windows', f'{name}-editors-windows-x86_64.zip', None),
    ('editors-mac', f'{name}-editors-macos-arm64.zip', None),
    ('editors-linux', f'{name}-editors-linux-x86_64.zip', None),
    ('aseprite', f'{name}.aseprite-extension', None),
    ('retroarch', f'{name}-retroarch.zip', None),
    ('reshade', f'{name}-reshade.zip', None),
    ('obs-linux', f'{name}-obs-linux-x86_64.zip', None),
]


def unpack(archive, folder):
    """Unpacks `archive` into `folder` with the system's tools, which keep executable bits,
    and lifts the contents of a single top folder, as the tarballs and some ZIPs have."""
    if archive.name.endswith('.tar.gz'):
        subprocess.run(['tar', '-xzf', str(archive), '-C', str(folder)], check=True)
    else:
        subprocess.run(['unzip', '-q', str(archive), '-d', str(folder)], check=True)
    entries = list(folder.iterdir())
    if len(entries) == 1 and entries[0].is_dir():
        top = entries[0].rename(folder.parent / (folder.name + '.top'))
        folder.rmdir()
        top.rename(folder)


known = {channel for channel, _, _ in CHANNELS}
unknown = set(args.only or []) - known
if unknown:
    raise SystemExit(f'No channel is called {", ".join(sorted(unknown))}; they are {", ".join(sorted(known))}')
args.out.mkdir()  # Refuse a folder left by an earlier run.
for channel, file, launch in CHANNELS:
    if args.only and channel not in args.only:
        continue
    source = args.assets / file
    if not source.is_file():
        print(f'{channel}: skipped, the release has no {file}', file=sys.stderr)
        continue
    folder = args.out / channel
    folder.mkdir()
    if file.endswith('.AppImage') or file.endswith('.aseprite-extension'):
        # butler would unpack an .aseprite-extension, which is a ZIP: it goes as the file.
        shutil.copy2(source, folder / file)
        if file.endswith('.AppImage'):
            (folder / file).chmod(0o755)
    else:
        unpack(source, folder)
    if launch:
        if not (folder / launch).exists():
            raise SystemExit(f'{channel}: {file} has no {launch}')
        (folder / '.itch.toml').write_text(
            f'[[actions]]\nname = "play"\npath = "{launch}"\n', encoding='utf-8')
    print(f'{channel}\t{folder}')
