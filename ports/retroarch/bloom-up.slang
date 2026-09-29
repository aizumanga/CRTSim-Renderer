#version 450
// CC0. The bloom's second blur, back at the screen's size.
#include "parameters.inc"
#include "bloom.inc"
void main() { FragColor = blur(vTexCoord, true); }
