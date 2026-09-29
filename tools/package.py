"""Package already-built release binaries and their notices. Run from the repository root."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import zipfile

parser = argparse.ArgumentParser()
# `web` packages the web app that tools/build_web.py left in web/dist, for a website to serve.
# `retroarch` packages the shipped looks as RetroArch shader presets, written by the release CLI.
parser.add_argument('--platform', choices=['linux-x86_64', 'windows-x86_64', 'macos-arm64', 'web', 'retroarch'],
                    required=True)
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
if args.platform == 'retroarch':
    # One folder to copy into RetroArch's shaders folder. The shaders and textures are CC0 and
    # hold no dependency's code, so the package carries only the original assets' sources.
    shaders = stage / 'crtsim-renderer'
    subprocess.run([str(root / 'target' / 'release' / 'crtsim'), 'export-retroarch', '--output-dir', str(shaders)],
                   check=True, stdout=subprocess.DEVNULL)
    shutil.copy2(root / 'LICENSE', shaders)
    shutil.copy2(root / 'assets/original-crtsim/SOURCES.md', shaders / 'ORIGINAL_ASSETS.md')
    archive = dist / (name + '.zip')
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as output:
        for path in sorted(shaders.rglob('*')):
            if path.is_file():
                output.write(path, path.relative_to(stage))
    print(archive)
    raise SystemExit
web = args.platform == 'web'
exe = '.exe' if args.platform.startswith('windows') else ''
if web:
    built = root / 'web' / 'dist'
    for page in ['index.html', 'crtsim_app.js', 'crtsim_app_bg.wasm']:
        shutil.copy2(built / page, stage)
else:
    for binary in ['crtsim-desktop', 'crtsim']:
        shutil.copy2(root / 'target' / 'release' / (binary + exe), stage)
        subprocess.run([str(stage / (binary + exe)), '--help'], check=True, stdout=subprocess.DEVNULL)
for doc in ['LICENSE', 'THIRD_PARTY_NOTICES.md'] if web else ['README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md']:
    shutil.copy2(root / doc, stage)
if not web:
    shutil.copytree(root / 'docs', stage / 'docs')
shutil.copy2(root / 'assets/original-crtsim/SOURCES.md', stage / 'ORIGINAL_ASSETS.md')
for source, destination in [
    ('SOURCES.md', 'NES_LUTS_SOURCES.md'),
    ('UPSTREAM_README.md', 'NES_LUTS_UPSTREAM_README.md'),
    ('SHA256SUMS', 'NES_LUTS_SHA256SUMS'),
]:
    shutil.copy2(root / 'assets/nes-luts' / source, stage / destination)
# Collect license texts from the exact Cargo.lock dependency sources, including build dependencies,
# of what ships: the app and the CLI. Test-only crates, such as crtsim-ports and the librashader
# it runs the shader ports with, are never built into a package.
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1']))
nodes = {node['id']: node for node in metadata['resolve']['nodes']}
shipped = set()
pending = [p['id'] for p in metadata['packages'] if p['name'] in ('crtsim-app', 'crtsim-cli') and p['source'] is None]
while pending:
    package_id = pending.pop()
    if package_id in shipped:
        continue
    shipped.add(package_id)
    pending += [dep['pkg'] for dep in nodes[package_id]['deps']
                if any(kind['kind'] != 'dev' for kind in dep['dep_kinds'])]
licenses = stage / 'dependency-licenses'
licenses.mkdir()
index = []
for package in metadata['packages']:
    if package['source'] is None or package['id'] not in shipped:
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
if web:
    # A website serves every file it holds, so the web package gathers the texts into one file
    # rather than a folder per dependency, with each text written out once.
    texts = []
    first = {}
    for entry in index:
        package = f"{entry['name']} {entry['version']}"
        folder = licenses / f"{entry['name']}-{entry['version']}"
        texts.append(f"{'=' * 78}\n{package} ({entry['license']})\n"
                     f"{entry['repository'] or ''}\n{'=' * 78}\n")
        for file in sorted(p for p in folder.rglob('*') if p.is_file()):
            text = file.read_text(encoding='utf-8', errors='replace').rstrip()
            license_name = file.relative_to(folder)
            if text in first:
                texts.append(f"--- {license_name}: the same text as {first[text]} ---\n\n")
            else:
                first[text] = f"{package}'s {license_name}"
                texts.append(f"--- {license_name} ---\n{text}\n\n")
    (stage / 'dependency-licenses.txt').write_text(''.join(texts), encoding='utf-8')
    shutil.rmtree(licenses)
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
if exe or web:
    archive = dist / (name + '.zip')
    # Cargo sources can carry Unix-epoch timestamps, older than ZIP's 1980 minimum.
    # Keep Windows files at the ZIP root. Windows' "Extract All" already creates a
    # directory named after the archive, so another identical directory is needless. The web
    # app's files are at the root too, as the folder a site serves them from.
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
