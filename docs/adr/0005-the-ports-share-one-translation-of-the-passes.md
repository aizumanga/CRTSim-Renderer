# 5. The ports share one translation of the passes, and OBS gets a port too

Status: accepted. It extends [ADR 3](0003-games-get-shader-ports-not-the-renderer.md), whose
shader ports it adds ReShade and OBS to.

## Context

The RetroArch port was the effect translated once, into GLSL, and held to the renderer by the
parity test through librashader. ReShade reads its own HLSL-like FX language and OBS its own
HLSL-like effects; RetroArch, GLSL. Translating the passes again for each would make three
copies of the same arithmetic to keep in step with the renderer, and only one of them tested:
there is no librashader for ReShade.

OBS was planned as a host of the renderer itself, through an `encode` entry point drawing
into the host's own texture. That needs the renderer's wgpu device to share textures with
OBS's Direct3D, OpenGL or Metal one, through each platform's interop.

## Decision

The passes are written once, in `ports/passes`, in the part of HLSL that ReShade's FX and
OBS's effects read as it is, and that GLSL reads through a header of macros
(`ports/retroarch/dialect.inc`). A pass reads what it needs from its host through `HOST_`
macros, which each host's shell defines: a parameter, an image's texel or sample, a size,
the frame's count and length. Each port is a shell around them: its uniforms and textures,
those macros and its passes' entry points.

OBS gets a shader port as the games do: a native filter, in C, running the passes as
techniques of an effect, which keeps the signal between frames as OBS's own shader filters
cannot. The renderer is not embedded in OBS.

## Consequences

- The RetroArch parity test checks the passes as GLSL, and a parity test of the OBS filter,
  in OBS itself on Xvfb, checks them as OBS's effects, through OBS's own translation to
  OpenGL. ReShade is compiled with ReShade's own compiler for every shipped look; its shell is
  not run, since nothing runs ReShade headless.
- The passes keep to what every dialect reads: no vector built from one number, no comparison
  of whole vectors, no arrays, no texture passed to a function, constants as macros, and no
  macro without parameters defined as something in parentheses, which OBS's preprocessor
  reads as its parameters. `ports/passes/common.inc` lists these.
- Hosts that hand over a picture of any size (ReShade, OBS) resize it to the signal in two
  passes of their own, `ports/passes/resize.inc`, as the image crate resizes; RetroArch hands
  over the core's picture.
- The OBS plugin links OBS's library and is built per platform and OBS version from
  `ports/obs`. Releases build it for Linux; elsewhere it is built from source.
- A look's parameters are one table for every port (`crtsim_core::ports`), each parameter
  naming the hosts that read it.

## Rejected: the renderer inside OBS

Sharing textures between wgpu and OBS's graphics device needs Vulkan external memory, Direct3D
shared handles or IOSurfaces, each with its own synchronisation, for every OBS backend. The
shader port draws the same picture with none of that, and its test runs the plugin as OBS does.
