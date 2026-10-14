"""Write the package managers' manifests for a published release, from its SHA256SUMS files.

    python tools/manifests.py --version v1.1.0 --sums DIR [--winget DIR]

DIR holds the release's SHA256SUMS-windows-x86_64.txt and SHA256SUMS-macos-arm64.txt. The Scoop
manifest (bucket/) and the Homebrew cask (Casks/) are rewritten in place, so this repository is
itself a Scoop bucket and a Homebrew tap. The winget manifests, which belong in winget's own
repository, are written to the folder --winget names. Run from the repository root.
"""
import argparse
import json
from pathlib import Path

REPOSITORY = 'aizumanga/CRTSim-Renderer'
HOMEPAGE = f'https://github.com/{REPOSITORY}'
DESCRIPTION = "Renders images and video through J. Kyle Pittman's CRTSim, a simulated CRT television"
PACKAGES = 'https://github.com/{repository}/releases/download/v{version}/CRTSim-Renderer-v{version}-{platform}'

parser = argparse.ArgumentParser()
parser.add_argument('--version', required=True, help='The release tag, such as v1.1.0')
parser.add_argument('--sums', required=True, type=Path, help="The folder holding the release's SHA256SUMS files")
parser.add_argument('--winget', type=Path, help='Where to write the winget manifests')
args = parser.parse_args()
if not args.version.startswith('v') or not all(c.isdigit() or c == '.' for c in args.version[1:]):
    raise SystemExit('The version must be a release tag such as v1.1.0')
version = args.version[1:]
root = Path.cwd()


def checksum(platform, suffix):
    """The SHA-256 the release's SHA256SUMS file gives its package for `platform`."""
    name = f'CRTSim-Renderer-v{version}-{platform}{suffix}'
    for line in (args.sums / f'SHA256SUMS-{platform}.txt').read_text(encoding='utf-8').splitlines():
        digest, _, file = line.partition('  ')
        if file == name:
            return digest
    raise SystemExit(f'SHA256SUMS-{platform}.txt does not list {name}')


def url(platform, suffix, version=version):
    return PACKAGES.format(repository=REPOSITORY, version=version, platform=platform) + suffix


windows = checksum('windows-x86_64', '.zip')
mac = checksum('macos-arm64', '.tar.gz')

# Scoop: `scoop bucket add crtsim https://github.com/aizumanga/CRTSim-Renderer`. checkver and
# autoupdate let `scoop` and its maintainers' tools find a newer release by themselves too.
scoop = {
    'version': version,
    'description': DESCRIPTION + '.',
    'homepage': HOMEPAGE,
    'license': 'CC0-1.0',
    'notes': [
        'Video needs FFmpeg and ffprobe, which CRTSim Renderer finds where Scoop installs them:',
        '    scoop install ffmpeg',
    ],
    'suggest': {'FFmpeg': 'main/ffmpeg'},
    'architecture': {'64bit': {'url': url('windows-x86_64', '.zip'), 'hash': windows}},
    'bin': 'crtsim.exe',
    'shortcuts': [['crtsim-desktop.exe', 'CRTSim Renderer']],
    'checkver': {'github': HOMEPAGE},
    'autoupdate': {'architecture': {'64bit': {
        'url': url('windows-x86_64', '.zip', version='$version'),
        'hash': {'url': '$baseurl/SHA256SUMS-windows-x86_64.txt'},
    }}},
}
bucket = root / 'bucket'
bucket.mkdir(exist_ok=True)
(bucket / 'crtsim-renderer.json').write_text(json.dumps(scoop, indent=4) + '\n', encoding='utf-8')

# Homebrew: `brew tap aizumanga/crtsim https://github.com/aizumanga/CRTSim-Renderer`.
folder = f'CRTSim-Renderer-v#{{version}}-macos-arm64'
cask = f'''cask "crtsim-renderer" do
  version "{version}"
  sha256 "{mac}"

  url "{HOMEPAGE}/releases/download/v#{{version}}/{folder}.tar.gz"
  name "CRTSim Renderer"
  desc "{DESCRIPTION.replace("J. Kyle Pittman's ", "")}"
  homepage "{HOMEPAGE}"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on arch: :arm64

  app "{folder}/CRTSim Renderer.app"
  binary "{folder}/crtsim"

  zap trash: "~/Library/Application Support/CRTSim-Renderer"

  caveats <<~EOS
    CRTSim Renderer is ad-hoc signed, without an Apple Developer ID. If macOS will not open it,
    allow it in System Settings, Privacy & Security.

    Video needs FFmpeg and ffprobe:
      brew install ffmpeg
  EOS
end
'''
casks = root / 'Casks'
casks.mkdir(exist_ok=True)
(casks / 'crtsim-renderer.rb').write_text(cask, encoding='utf-8')

# winget: a portable ZIP whose two programs get command-line aliases.
if args.winget:
    identifier = 'aizumanga.CRTSimRenderer'
    folder = args.winget / version
    folder.mkdir(parents=True, exist_ok=True)
    header = f'PackageIdentifier: {identifier}\nPackageVersion: {version}\n'
    footer = 'ManifestVersion: 1.6.0\n'
    (folder / f'{identifier}.yaml').write_text(
        f'# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json\n'
        f'{header}DefaultLocale: en-US\nManifestType: version\n{footer}', encoding='utf-8')
    (folder / f'{identifier}.installer.yaml').write_text(
        f'# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json\n'
        f'{header}InstallerType: zip\nNestedInstallerType: portable\nNestedInstallerFiles:\n'
        f'- RelativeFilePath: crtsim-desktop.exe\n  PortableCommandAlias: crtsim-desktop\n'
        f'- RelativeFilePath: crtsim.exe\n  PortableCommandAlias: crtsim\n'
        f'Installers:\n- Architecture: x64\n  InstallerUrl: {url("windows-x86_64", ".zip")}\n'
        f'  InstallerSha256: {windows.upper()}\nManifestType: installer\n{footer}', encoding='utf-8')
    (folder / f'{identifier}.locale.en-US.yaml').write_text(
        f'# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json\n'
        f'{header}PackageLocale: en-US\nPublisher: aizumanga\nPublisherUrl: https://github.com/aizumanga\n'
        f'PackageName: CRTSim Renderer\nPackageUrl: {HOMEPAGE}\nLicense: CC0-1.0\n'
        f'LicenseUrl: {HOMEPAGE}/blob/v{version}/LICENSE\nShortDescription: {DESCRIPTION}.\n'
        f'Tags:\n- crt\n- shader\n- retro\n- video\n'
        f'ReleaseNotesUrl: {HOMEPAGE}/releases/tag/v{version}\nManifestType: defaultLocale\n{footer}',
        encoding='utf-8')
    print(folder)
