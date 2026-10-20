#[vertex]
#version 450
#include "vertex.inc"

#[fragment]
#version 450
// CC0. The camera's picture, which Godot holds in linear light, encoded as the 8-bit sRGB picture the renderer is handed.
#include "shell.inc"

void main() { frag = vec4(srgb_encode(clamp(texelFetch(pass_input, ivec2(uv * u.crtsim_original_size), 0).rgb, V3(0.0), V3(1.0))), 1.0); }
