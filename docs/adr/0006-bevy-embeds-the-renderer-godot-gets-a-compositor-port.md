# 6. Bevy embeds the renderer; Godot gets a compositor port

Status: accepted. It narrows [ADR 3](0003-games-get-shader-ports-not-the-renderer.md), whose
"the renderer is not embedded in games" no longer holds for Bevy, and adds Godot to the shader
ports of [ADR 5](0005-the-ports-share-one-translation-of-the-passes.md).

## Context

ADR 3 kept the renderer out of games because it waited on the GPU every frame and took only
CPU images, and because only games written in Rust on wgpu could use it. Since then the
renderer draws a **host frame** (`Renderer::encode`): it records a frame from a texture on the
host's own device into another, into the host's own encoder, and waits for nothing. Bevy 0.20
renders on wgpu 30, the version the renderer is on, so a Bevy game's device, its camera's
textures and its command encoder are the renderer's own types.

Godot 4 renders on its own RenderingDevice (Vulkan, Direct3D 12 or Metal), which wgpu cannot
share textures with but through each platform's interop, as ADR 5 found for OBS. Its
`CompositorEffect` runs a script's own passes on a camera's picture every frame, with textures
of its own that persist between frames, which is all a shader port needs.

## Decision

- **Bevy** gets the renderer itself: `crtsim-bevy`, a plugin that draws each camera holding a
  `Crt` through the CRT with `Renderer::encode`, on Bevy's device, after Bevy's own
  post-processing. Each camera is a host sequence. It is published to crates.io with each
  release, at the app's version, for the Bevy release whose wgpu the renderer is on.
- **Godot** gets a shader port: `ports/godot`, an add-on whose `CompositorEffect` runs the
  shared passes as compute shaders, through the same GLSL dialect header the RetroArch port
  uses. It is published as a zip with each release, for the Godot Asset Library.

## Consequences

- A Bevy release that moves to another wgpu needs the renderer to move with it before
  `crtsim-bevy` can follow; until then the plugin stays on the Bevy release before.
- A Bevy game draws exactly the app's picture, every look and setting included, and its
  settings screen can offer `crtsim_core::settings::SETTINGS`.
- Godot's compositor effects run only with its Forward+ and Mobile renderers, so a game on the
  Compatibility renderer, which includes Godot's web exports, cannot use the port.
- The Godot port is checked against the renderer in CI by running Godot itself on software
  Vulkan, as the OBS port is checked in OBS.
