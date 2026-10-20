# 3. Games get the effect as shader ports, not the renderer

Status: accepted. [ADR 6](0006-bevy-embeds-the-renderer-godot-gets-a-compositor-port.md)
narrows it: Bevy embeds the renderer, and Godot gets a shader port.

Games get the CRT as a **shader port**: the effect rewritten in the host's own shader language
and run by the host on every frame, first as a RetroArch shader preset (`.slangp`), then as a
ReShade effect. The wgpu renderer is not embedded in games for now.

## Considered options

- **The renderer as a library inside wgpu games** (a Bevy plugin, custom engines). It runs this
  exact renderer, but only in games written in Rust on wgpu, a small audience.
- **A capture overlay** that shows any game's window through the renderer, as ShaderGlass does.
  It needs the same work as the library route, plus window capture on every platform.
- **Ports to Godot, Unity or Unreal.** Each engine needs its own port and its own upkeep.

A RetroArch preset reaches every libretro core, ares and ShaderGlass, which overlays any
Windows game; ReShade reaches almost any PC game. Neither host can draw meshes, so the ported
effect is made of full-screen passes only.

## Consequences

- The renderer's glass becomes a full-screen pass too: the screen is a sphere cap, ray-traced
  exactly, so the renderer is the port's reference and the golden images hold both. The bezel,
  an irregular mesh, is baked into textures.
- The RetroArch port is checked against the renderer in CI through librashader, which runs
  `.slangp` presets on wgpu. There is no such harness for ReShade, so it shares its passes with
  the checked ports ([ADR 5](0005-the-ports-share-one-translation-of-the-passes.md)).
- The renderer still waits on the GPU every frame and still takes only CPU images. That is
  fine for the app, and it is the first thing to change if a game ever embeds the renderer.
