"""Compiles the ReShade port as each look in an export sets it, with ReShade's own compiler, at a few
screen sizes, and validates what it makes. Run from anywhere:

    python3 tools/reshadefx/check_presets.py CHECK EXPORT_DIR

CHECK is the program build.sh makes; EXPORT_DIR is what `crtsim export-reshade` wrote."""
import subprocess
import sys
from pathlib import Path

check, export = sys.argv[1], Path(sys.argv[2])
shaders = export / 'reshade-shaders' / 'Shaders'
# Screens of the shapes games draw at, and one whose sides divide unevenly.
sizes = [(1920, 1080), (1280, 720), (1366, 768), (800, 600)]
presets = sorted(export.glob('*.ini'))
if not presets:
    raise SystemExit(f'no presets in {export}')
failed = False
for preset in presets:
    definitions = []
    for line in preset.read_text().splitlines():
        if line.startswith('PreprocessorDefinitions='):
            definitions = [d for d in line.split('=', 1)[1].split(',') if d]
    for width, height in sizes:
        spirv = export / 'check.spv'
        command = [check, '-I', str(shaders), '-D', '__RESHADE__=40901', '-D', f'BUFFER_WIDTH={width}',
                   '-D', f'BUFFER_HEIGHT={height}', '-D', 'BUFFER_RCP_WIDTH=(1.0 / BUFFER_WIDTH)',
                   '-D', 'BUFFER_RCP_HEIGHT=(1.0 / BUFFER_HEIGHT)', '-D', 'BUFFER_COLOR_DEPTH=8',
                   '-o', str(spirv)]
        for definition in definitions:
            command += ['-D', definition]
        command.append(str(shaders / 'CRTSimRenderer.fx'))
        compiled = subprocess.run(command, capture_output=True, text=True)
        valid = compiled.returncode == 0 and subprocess.run(
            ['spirv-val', '--target-env', 'vulkan1.1', str(spirv)]).returncode == 0
        print(f'{preset.name} at {width}x{height}: {"ok" if valid else "FAILED"}')
        if not valid:
            print(compiled.stdout + compiled.stderr)
            failed = True
if failed:
    raise SystemExit('the ReShade port does not compile as every look sets it')
