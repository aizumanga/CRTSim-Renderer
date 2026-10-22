#[vertex]
#version 450
#include "vertex.inc"

#[fragment]
#version 450
// CC0. The picture's columns at the signal's rows: passes/resize.inc.
#include "shell.inc"
#include "passes/resize.inc"
void main() { frag = crtsim_resize_rows(ivec2(uv * vec2(u.crtsim_original_size.x, u.crtsim_signal_size.y))); }
