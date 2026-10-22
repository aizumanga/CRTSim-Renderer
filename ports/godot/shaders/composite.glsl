#[vertex]
#version 450
#include "vertex.inc"

#[fragment]
#version 450
// CC0. The composite signal over the glow of the frame before: passes/composite.inc.
#include "shell.inc"
#include "passes/composite.inc"
void main() { frag = crtsim_composite(ivec2(uv * u.crtsim_signal_size)); }
