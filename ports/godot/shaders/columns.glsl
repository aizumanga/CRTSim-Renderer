#[vertex]
#version 450
#include "vertex.inc"

#[fragment]
#version 450
// CC0. The picture at the signal's size: passes/resize.inc.
#include "shell.inc"
#include "passes/resize.inc"
void main() { frag = crtsim_resize_columns(ivec2(uv * u.crtsim_signal_size)); }
