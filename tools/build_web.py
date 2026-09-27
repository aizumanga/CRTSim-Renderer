"""Builds the web app into web/dist: the page, its WebAssembly module and the module's JavaScript.

Needs the wasm32-unknown-unknown target (`rustup target add wasm32-unknown-unknown`) and
wasm-bindgen's command line at the version Cargo.lock pins
(`cargo install wasm-bindgen-cli --version <that version> --locked`).
"""
import re
import shutil
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parent.parent
dist = root / 'web' / 'dist'

# The command line refuses a module made by another version of the wasm-bindgen crate.
lock = (root / 'Cargo.lock').read_text()
pinned = re.search(r'name = "wasm-bindgen"\nversion = "([^"]+)"', lock).group(1)
installed = subprocess.run(['wasm-bindgen', '--version'], capture_output=True, text=True,
                           check=True).stdout.split()[-1]
if installed != pinned:
    raise SystemExit(f'wasm-bindgen {installed} is installed; Cargo.lock needs {pinned}: '
                     f'cargo install wasm-bindgen-cli --version {pinned} --locked')

# A cdylib only for this build, so the desktop's own builds do not make one too.
subprocess.run(['cargo', 'rustc', '--locked', '--release', '-p', 'crtsim-app', '--lib',
                '--target', 'wasm32-unknown-unknown', '--crate-type', 'cdylib'],
               cwd=root, check=True)
module = root / 'target' / 'wasm32-unknown-unknown' / 'release' / 'crtsim_app.wasm'
shutil.rmtree(dist, ignore_errors=True)
subprocess.run(['wasm-bindgen', '--target', 'web', '--no-typescript', '--out-dir', str(dist),
                str(module)], check=True)
shutil.copy2(root / 'web' / 'index.html', dist / 'index.html')
for file in sorted(dist.iterdir()):
    print(f'{file.stat().st_size / 1e6:8.2f} MB  {file.relative_to(root)}')
