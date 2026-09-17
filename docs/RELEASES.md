# Portable releases (Phase 4)

The **Portable packages** workflow builds each pull request and manual workflow run into downloadable Actions artifacts.
A `v*` tag builds the same packages and creates a **draft** GitHub release after all three packaging jobs succeed.
Review and publish the draft manually. This workflow does not create tags or merge pull requests.

## Packages

| Platform | Package | Launch |
| --- | --- | --- |
| Windows x86_64 | Portable ZIP, desktop and CLI | Extract the entire ZIP, open `crtsim-desktop.exe` |
| Linux x86_64 | AppImage | Make executable, then open it |
| Linux x86_64 | Portable tar.gz, desktop and CLI | Extract, run `./crtsim-desktop` |
| macOS Apple Silicon | tar.gz with `.app`, desktop and CLI | Extract, open `CRTSim Renderer.app` |

No Rust installation or repository checkout is required. Effects/assets are embedded.
Each archive includes the README, project license, original asset provenance, dependency license texts/index and commit identifier.
SHA256SUMS files accompany downloads. Builds are not claimed to be byte-for-byte reproducible.

### Linux

AppImages are built on Ubuntu 22.04 (glibc 2.35 baseline). They bundle selected windowing libraries;
your host must still provide a working display session, graphics drivers and desktop file portal.
The tarball uses the host windowing libraries as described in the README. These are portable binaries, not fully static builds.
The AppImage builder is the upstream linuxdeploy continuous release; its downloaded SHA-256 is recorded in the build artifacts.
Graphics drivers, FFmpeg and ffprobe are not bundled. The host FFmpeg runs outside the AppImage's library search path.

```sh
chmod +x CRTSim-Renderer-*-linux-x86_64.AppImage
./CRTSim-Renderer-*-linux-x86_64.AppImage
```

If FUSE is unavailable, use `APPIMAGE_EXTRACT_AND_RUN=1 ./CRTSim-Renderer-...AppImage`
or `--appimage-extract` and launch `squashfs-root/AppRun`.
Pass `--cli` to the AppImage to run the command-line renderer, e.g. `./renderer.AppImage --cli inspect-meshes`.
The AppImage is launched under software Vulkan/Xvfb in CI before its artifact is uploaded.

### Windows and macOS

Windows builds statically link the C runtime; no separate Visual C++ redistributable is needed for that runtime.
The application is unsigned and Windows may display a download warning.
macOS is optional/provisional: Apple Silicon only, ad-hoc signed for execution, without Developer ID signing or notarization.
Gatekeeper may require approval through System Settings → Privacy & Security. Real Mac runtime testing remains necessary.
A Windows installer, Intel/universal Mac builds and Apple signing are deferred.

### Video setup and user data

Install FFmpeg and ffprobe separately and add both to PATH, or set `CRTSIM_FFMPEG` and `CRTSIM_FFPROBE` to their full paths.
They must include the codecs described in [VIDEO_PIPELINE.md](VIDEO_PIPELINE.md). Image rendering works without FFmpeg.
There are no automatic runtime downloads. Personal presets remain in the normal per-user app-data directory;
updating or replacing a portable package does not delete them. Use `CRTSIM_DATA_DIR` for an explicit alternate directory.

## Local packaging

Build with `cargo build --release --locked -p crtsim-desktop -p crtsim-cli`, then run
`python tools/package.py --platform linux-x86_64 --version preview-local` (or the matching Windows/macOS platform).
Run on the matching native OS. A clean `dist/` staging area is required. The workflow contains the additional AppImage build command.
