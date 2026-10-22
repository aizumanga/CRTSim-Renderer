#[vertex]
#version 450
#include "vertex.inc"

#[fragment]
#version 450
// CC0. The colour table and the YIQ grade: passes/prepare.inc.
#include "shell.inc"
#include "passes/prepare.inc"
void main() { frag = crtsim_prepare(ivec2(uv * u.crtsim_signal_size)); }
