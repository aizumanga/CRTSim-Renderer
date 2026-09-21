"""Package already-built release binaries and their notices. Run from the repository root."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument('--platform', choices=['linux-x86_64', 'windows-x86_64', 'macos-arm64'], required=True)
parser.add_argument('--version', required=True)
args = parser.parse_args()
if not all(c.isalnum() or c in '.-_' for c in args.version):
    raise SystemExit('Version must contain only letters, digits, dots, hyphens and underscores')
root = Path.cwd()
dist = root / 'dist'
dist.mkdir(exist_ok=True)
name = f'CRTSim-Renderer-{args.version}-{args.platform}'
stage = dist / name
stage.mkdir()  # Refuse stale/reused staging folders.
exe = '.exe' if args.platform.startswith('windows') else ''
for binary in ['crtsim-desktop', 'crtsim']:
    shutil.copy2(root / 'target' / 'release' / (binary + exe), stage)
    subprocess.run([str(stage / (binary + exe)), '--help'], check=True, stdout=subprocess.DEVNULL)
for doc in ['README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md']:
    shutil.copy2(root / doc, stage)
shutil.copytree(root / 'docs', stage / 'docs')
shutil.copy2(root / 'assets/original-crtsim/SOURCES.md', stage / 'ORIGINAL_ASSETS.md')
for source, destination in [
    ('SOURCES.md', 'NES_LUTS_SOURCES.md'),
    ('UPSTREAM_README.md', 'NES_LUTS_UPSTREAM_README.md'),
    ('SHA256SUMS', 'NES_LUTS_SHA256SUMS'),
]:
    shutil.copy2(root / 'assets/nes-luts' / source, stage / destination)
# Collect license texts from the exact Cargo.lock dependency sources, including build dependencies.
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1']))
licenses = stage / 'dependency-licenses'
licenses.mkdir()
index = []
for package in metadata['packages']:
    if package['source'] is None:
        continue
    package_root = Path(package['manifest_path']).parent
    destination = licenses / f"{package['name']}-{package['version']}"
    destination.mkdir()
    candidates = set()
    for pattern in ['LICENSE*', 'LICENCE*', 'COPYING*', 'NOTICE*', 'license*', 'licence*', 'copyright*']:
        candidates.update(package_root.glob(pattern))
    if package.get('license_file'):
        candidates.add(package_root / package['license_file'])
    for source in sorted(candidates):
        if source.is_file():
            shutil.copy2(source, destination / source.name)
        elif source.is_dir():
            shutil.copytree(source, destination / source.name, dirs_exist_ok=True)
    index.append({key: package.get(key) for key in ['name', 'version', 'license', 'repository']})
(licenses / 'index.json').write_text(json.dumps(index, indent=2) + '\n', encoding='utf-8')
(stage / 'BUILD.txt').write_text(
    f"Version: {args.version}\nPlatform: {args.platform}\nCommit: "
    + subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip() + '\n', encoding='utf-8')
if args.platform == 'macos-arm64':
    app = stage / 'CRTSim Renderer.app/Contents'
    (app / 'MacOS').mkdir(parents=True)
    shutil.copy2(stage / 'crtsim-desktop', app / 'MacOS/crtsim-desktop')
    import plistlib
    with (app / 'Info.plist').open('wb') as output:
        plistlib.dump({'CFBundleName': 'CRTSim Renderer', 'CFBundleDisplayName': 'CRTSim Renderer',
                      'CFBundleIdentifier': 'org.crtsim.renderer', 'CFBundleExecutable': 'crtsim-desktop',
                      'CFBundlePackageType': 'APPL', 'CFBundleVersion': '1',
                      'CFBundleShortVersionString': '0.1.1', 'NSHighResolutionCapable': True}, output)
    # Ad-hoc signing permits execution on Apple Silicon; it is not Developer ID signing/notarization.
    subprocess.run(['codesign', '--force', '--deep', '--sign', '-', str(app.parent)], check=True)
if exe:
    archive = dist / (name + '.zip')
    # Cargo sources can carry Unix-epoch timestamps, older than ZIP's 1980 minimum.
    # Keep Windows files at the ZIP root. Windows' "Extract All" already creates a
    # directory named after the archive, so another identical directory is needless.
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED, strict_timestamps=False) as output:
        for path in sorted(stage.rglob('*')):
            if path.is_file():
                output.write(path, path.relative_to(stage))
else:
    archive = dist / (name + '.tar.gz')
    with tarfile.open(archive, 'w:gz') as output:
        output.add(stage, arcname=name)
if args.platform == 'linux-x86_64':
    appdir = dist / 'AppDir'
    (appdir / 'usr/bin').mkdir(parents=True)
    for binary in ['crtsim-desktop', 'crtsim']:
        shutil.copy2(stage / binary, appdir / 'usr/bin')
    shutil.copytree(stage, appdir / 'usr/share/doc/crtsim-renderer')
    for binary in ['crtsim-desktop', 'crtsim']:
        (appdir / 'usr/share/doc/crtsim-renderer' / binary).unlink()
    shutil.copy2(root / 'packaging/AppRun', appdir / 'AppRun')
    (appdir / 'AppRun').chmod(0o755)
print(archive)
