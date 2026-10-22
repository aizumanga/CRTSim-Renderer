#[vertex]
#version 450
#include "vertex.inc"

#[fragment]
#version 450
// CC0. The bloom's first blur: passes/bloom.inc.
#include "shell.inc"
#include "passes/bloom.inc"
void main() { frag = crtsim_bloom(uv, false); }
