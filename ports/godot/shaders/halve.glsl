#[vertex]
#version 450
#include "vertex.inc"

#[fragment]
#version 450
// CC0. A mip level of the surface from the one above, for the bloom: passes/halve.inc.
#include "shell.inc"
#include "passes/halve.inc"
void main() { frag = crtsim_halve(uv); }
