"""The license texts of what ships, from the exact Cargo.lock dependency sources, including build
dependencies. Test-only crates, such as crtsim-ports and the librashader it runs the shader
ports with, are never built into anything that ships.

As a script, writes the texts of the dependencies of the crates it names into one file, as the
service's image carries them:

    python3 tools/licenses.py crtsim-service > dependency-licenses.txt
"""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def metadata():
    return json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1']))


def shipped(metadata, crates):
    """The ids of the workspace's `crates` and everything they build with."""
    nodes = {node['id']: node for node in metadata['resolve']['nodes']}
    found = set()
    pending = [p['id'] for p in metadata['packages'] if p['name'] in crates and p['source'] is None]
    while pending:
        package_id = pending.pop()
        if package_id in found:
            continue
        found.add(package_id)
        pending += [dep['pkg'] for dep in nodes[package_id]['deps']
                    if any(kind['kind'] != 'dev' for kind in dep['dep_kinds'])]
    return found


def collect(metadata, ids, licenses):
    """Copies each dependency's license files into a folder of its own in `licenses`, and returns
    the index of them, which it also writes there as index.json."""
    index = []
    for package in metadata['packages']:
        if package['source'] is None or package['id'] not in ids:
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
    return index


def one_file(index, licenses):
    """The texts gathered into one file rather than a folder per dependency, each text written
    out once."""
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
    return ''.join(texts)


if __name__ == '__main__':
    if len(sys.argv) < 2:
        raise SystemExit('Name the crates whose dependencies ship, such as crtsim-service')
    data = metadata()
    with tempfile.TemporaryDirectory() as folder:
        licenses = Path(folder)
        sys.stdout.write(one_file(collect(data, shipped(data, sys.argv[1:]), licenses), licenses))
