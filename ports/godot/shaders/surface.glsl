#[vertex]
#version 450
#include "vertex.inc"

#[fragment]
#version 450
// CC0. The curved glass and its bezel, lit: passes/surface.inc.
#include "shell.inc"
#include "passes/surface.inc"
void main() { frag = crtsim_surface(uv); }
