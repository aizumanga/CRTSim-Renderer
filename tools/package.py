"""Package already-built release binaries and their notices. Run from the repository root."""
import argparse
import json
from pathlib import Path

import licenses as licenses_of
import shutil
import subprocess
import tarfile
import zipfile

parser = argparse.ArgumentParser()
# `web` packages the web app that tools/build_web.py left in web/dist, for a website to serve.
# `retroarch` and `reshade` package the shipped looks as those ports' presets, written by the release CLI.
# `obs-linux-x86_64` packages the OBS filter that CMake built into build/obs, with its data folder.
parser.add_argument('--platform', choices=['linux-x86_64', 'windows-x86_64', 'macos-arm64', 'web', 'retroarch',
                                           'reshade', 'obs-linux-x86_64'],
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
cli = str(root / 'target' / 'release' / 'crtsim')
# The vendored assets live in the renderer's crate, which embeds them.
ASSETS = root / 'crates' / 'crtsim-core' / 'assets'


if args.platform in ('retroarch', 'reshade', 'obs-linux-x86_64'):
    # One folder to copy where its host looks. The shaders, textures and plugin are CC0 and hold
    # no dependency's code; the plugin links OBS's own library only as OBS loads it. So the
    # package carries only the original assets' sources.
    folder = stage / 'crtsim-renderer'
    if args.platform == 'obs-linux-x86_64':
        # Laid out as OBS finds a user's plugins: <name>/bin/64bit beside <name>/data.
        (folder / 'bin' / '64bit').mkdir(parents=True)
        shutil.copy2(root / 'build' / 'obs' / 'crtsim-renderer.so', folder / 'bin' / '64bit')
        subprocess.run([cli, 'obs-plugin-data', '--output-dir', str(folder / 'data')], check=True)
        shutil.copy2(root / 'ports' / 'obs' / 'README.md', folder)
    else:
        subprocess.run([cli, f'export-{args.platform}', '--output-dir', str(folder)], check=True,
                       stdout=subprocess.DEVNULL)
    shutil.copy2(root / 'LICENSE', folder)
    shutil.copy2(ASSETS / 'original-crtsim/SOURCES.md', folder / 'ORIGINAL_ASSETS.md')
    archive = dist / (name + '.zip')
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as output:
        for path in sorted(folder.rglob('*')):
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
shutil.copy2(ASSETS / 'original-crtsim/SOURCES.md', stage / 'ORIGINAL_ASSETS.md')
for source, destination in [
    ('SOURCES.md', 'NES_LUTS_SOURCES.md'),
    ('UPSTREAM_README.md', 'NES_LUTS_UPSTREAM_README.md'),
    ('SHA256SUMS', 'NES_LUTS_SHA256SUMS'),
]:
    shutil.copy2(ASSETS / 'nes-luts' / source, stage / destination)
# The license texts of what ships: the app and the CLI (see tools/licenses.py).
metadata = licenses_of.metadata()
# The app's own version, as it reports it in its status bar and compares with the latest release
# when asked to check for updates. A release tag must name that same version.
app_version = next(p['version'] for p in metadata['packages'] if p['name'] == 'crtsim-app' and p['source'] is None)
if args.version.startswith('v') and args.version != f'v{app_version}':
    raise SystemExit(f'Release {args.version} does not match the version in Cargo.toml, {app_version}')
licenses = stage / 'dependency-licenses'
licenses.mkdir()
index = licenses_of.collect(metadata, licenses_of.shipped(metadata, ('crtsim-app', 'crtsim-cli')), licenses)
if web:
    # A website serves every file it holds, so the web package gathers the texts into one file
    # rather than a folder per dependency, with each text written out once.
    (stage / 'dependency-licenses.txt').write_text(licenses_of.one_file(index, licenses), encoding='utf-8')
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
                      'CFBundlePackageType': 'APPL', 'CFBundleVersion': app_version,
                      'CFBundleShortVersionString': app_version, 'NSHighResolutionCapable': True}, output)
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
