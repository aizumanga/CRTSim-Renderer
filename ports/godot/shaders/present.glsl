#[vertex]
#version 450
#include "vertex.inc"

#[fragment]
#version 450
// CC0. The glass and bezel with their bloom, given back to Godot in linear light: passes/present.inc.
#include "shell.inc"
#include "passes/present.inc"
void main() { frag = vec4(srgb_decode(crtsim_present(uv).rgb), 1.0); }
