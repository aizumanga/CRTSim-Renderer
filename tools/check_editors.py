"""Render a clip through the editor effects in programs that run them, and compare each frame
with the command line's render of the same clip as a video.

FFmpeg's frei0r filter and MLT's melt run the frei0r filter. NatronRenderer runs the OpenFX
effect over the whole clip; over its last frame alone, which reaches back through the clip for
its glow; and over a still with its artifacts alternating, whose frames must alternate. Run from
the repository root after `cargo build -p crtsim-cli -p crtsim-frei0r -p crtsim-ofx`, on Linux,
with FFmpeg built with frei0r and MLT's melt. `--natron` names an unpacked Natron for the OpenFX
checks, which are skipped without it.
"""
import argparse
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
SIZE = (320, 240)
FPS = 24
FRAMES = 12
LOOK = 'General image'


def run(command, **kwargs):
    subprocess.run([str(c) for c in command], check=True, **kwargs)


def frames(directory):
    return sorted(pathlib.Path(directory).glob('*.png'))


def pixels(path):
    """A frame's RGB bytes. Natron's writer puts its first frame on the project's canvas, at the
    bottom left, so a larger frame is cropped there."""
    width, height = SIZE
    return subprocess.run(
        ['ffmpeg', '-loglevel', 'error', '-i', str(path), '-vf', f'crop={width}:{height}:0:ih-{height}',
         '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'],
        check=True, capture_output=True).stdout


def apart(a, b):
    """The most any channel of two frames differs, in 8-bit steps."""
    a, b = pixels(a), pixels(b)
    assert len(a) == len(b) == SIZE[0] * SIZE[1] * 3, 'frames of the clip\'s size'
    return max(abs(x - y) for x, y in zip(a, b))


failures = []


def expect(name, steps, most):
    verdict = 'ok' if steps <= most else 'FAILED'
    print(f'{name:44} {steps:3} steps apart, at most {most}: {verdict}')
    if steps > most:
        failures.append(name)


def compare(name, made, reference, most, first_two_most=None):
    """`made`'s frames against the reference's. A frei0r filter learns the frame rate from its
    second frame, so its first two are let differ more."""
    made = frames(made)
    if len(made) != len(reference):
        failures.append(name)
        print(f'{name}: {len(made)} frames, not {len(reference)}: FAILED')
        return
    for i, (ours, theirs) in enumerate(zip(made, reference)):
        expect(f'{name} frame {i}', apart(ours, theirs),
               first_two_most if i < 2 and first_two_most else most)


def natron(args, work, plugin, **settings):
    output = work / settings.pop('output')
    output.mkdir()
    environment = dict(os.environ, OFX_PLUGIN_PATH=str(plugin), XDG_RUNTIME_DIR=str(work),
                       CRTSIM_OUTPUT=str(output / 'out####.png'),
                       **{f'CRTSIM_{k.upper()}': str(v) for k, v in settings.items()})
    # Natron's Linux builds carry an older libstdc++ than Mesa's Vulkan drivers need.
    libraries = subprocess.run(['ldconfig', '-p'], capture_output=True, text=True).stdout
    for line in libraries.splitlines():
        if line.strip().startswith('libstdc++.so.6 ') and 'x86-64' in line:
            environment['LD_PRELOAD'] = line.split('=>')[-1].strip()
            break
    run([args.natron / 'NatronRenderer', ROOT / 'tools/editors/natron_render.py'],
        env=environment, cwd=work, stdout=subprocess.DEVNULL)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    # Natron runs in the work folder, so the paths are made absolute.
    absolute = lambda path: pathlib.Path(path).resolve()
    parser.add_argument('--target', type=absolute, default=ROOT / 'target/debug')
    parser.add_argument('--natron', type=absolute, help='an unpacked Natron for Linux')
    parser.add_argument('--keep', type=pathlib.Path, help='a NEW directory to keep the frames in')
    args = parser.parse_args()
    work = pathlib.Path(tempfile.mkdtemp(prefix='crtsim-editors-'))
    try:
        check(args, work)
    finally:
        if args.keep:
            shutil.copytree(work, args.keep)
        shutil.rmtree(work)
    if failures:
        sys.exit(f'{len(failures)} checks failed')


def check(args, work):
    width, height = SIZE
    source = work / 'source'
    source.mkdir()
    run(['ffmpeg', '-loglevel', 'error', '-f', 'lavfi', '-i', f'testsrc2=size={width}x{height}:rate={FPS}',
         '-frames:v', FRAMES, '-start_number', 0, source / 'in%04d.png'])
    clip = work / 'clip.mkv'
    run(['ffmpeg', '-loglevel', 'error', '-framerate', FPS, '-start_number', 0, '-i', source / 'in%04d.png',
         '-c:v', 'ffv1', '-pix_fmt', 'bgr0', clip])

    crtsim = args.target / 'crtsim'
    reference = work / 'reference'
    reference.mkdir()
    run([crtsim, 'render', '-i', clip, '-o', work / 'reference.mkv', '--preset', LOOK, '--size',
         'match-input', '--quality', 'lossless', '--audio', 'mute'], stdout=subprocess.DEVNULL)
    run(['ffmpeg', '-loglevel', 'error', '-i', work / 'reference.mkv', '-start_number', 0,
         reference / 'f%04d.png'])
    reference = frames(reference)

    frei0r = work / 'frei0r'
    frei0r.mkdir()
    shutil.copy(args.target / 'libcrtsim_frei0r.so', frei0r / 'crtsim.so')
    with_frei0r = dict(os.environ, FREI0R_PATH=str(frei0r))

    made = work / 'ffmpeg'
    made.mkdir()
    run(['ffmpeg', '-loglevel', 'error', '-i', clip, '-vf', f'frei0r=crtsim:{LOOK}', '-start_number', 0,
         made / 'f%04d.png'], env=with_frei0r)
    compare('frei0r in FFmpeg', made, reference, 4, 16)

    # melt crashes on a profile with no description.
    profile = work / 'profile'
    profile.write_text(f'description=The clip\nwidth={width}\nheight={height}\n'
                       f'frame_rate_num={FPS}\nframe_rate_den=1\nprogressive=1\n'
                       'sample_aspect_num=1\nsample_aspect_den=1\n'
                       f'display_aspect_num={width}\ndisplay_aspect_den={height}\ncolorspace=601\n')
    made = work / 'melt'
    made.mkdir()
    run(['melt', '-silent', '-profile', profile, clip, '-filter', 'frei0r.crtsim', f'Look={LOOK}',
         '-consumer', f'avformat:{made}/f%04d.png', 'start_number=0', 'vcodec=png'],
        env=with_frei0r, stderr=subprocess.DEVNULL)
    compare('frei0r in melt', made, reference, 4, 16)

    if not args.natron:
        print('OpenFX: skipped, no --natron')
        return
    plugin = work / 'ofx'
    binary = plugin / 'CRTSim.ofx.bundle/Contents/Linux-x86-64'
    binary.mkdir(parents=True)
    shutil.copy(args.target / 'libcrtsim_ofx.so', binary / 'CRTSim.ofx')
    sequence = {'input': source / 'in####.png', 'input_first': 0, 'input_last': FRAMES - 1}
    whole = natron(args, work, plugin, output='natron', first=0, last=FRAMES - 1, **sequence)
    compare('OpenFX in Natron', whole, reference, 1)
    last = natron(args, work, plugin, output='natron-last', first=FRAMES - 1, last=FRAMES - 1,
                  **sequence)
    expect('OpenFX in Natron, the last frame alone', apart(frames(last)[0], reference[-1]), 1)
    still = {'input': source / 'in0000.png', 'input_first': 0, 'input_last': 0}
    alternating = frames(natron(args, work, plugin, output='natron-alternating', first=0, last=3,
                                timing=1, **still))
    expect('OpenFX alternating: frames 0 and 2 alike', apart(alternating[0], alternating[2]), 1)
    steps = apart(alternating[0], alternating[1])
    print(f'{"OpenFX alternating: frames 0 and 1 differ":44} {steps:3} steps apart: '
          f'{"ok" if steps > 16 else "FAILED"}')
    if steps <= 16:
        failures.append('alternating')


if __name__ == '__main__':
    main()
