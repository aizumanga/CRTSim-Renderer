#version 450
// CC0. The bloom's first blur, into a sixteenth of the screen.
#include "parameters.inc"
#include "bloom.inc"
void main() { FragColor = blur(vTexCoord, false); }
