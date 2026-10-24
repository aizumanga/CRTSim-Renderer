#version 450
// CC0. A mip level of the surface from the one above, down to the two the bloom's first blur
// reads between: passes/halve.inc.

layout(std140, set = 0, binding = 0) uniform UBO {
    mat4 MVP;
} global;

#pragma stage vertex
layout(location = 0) in vec4 Position;
layout(location = 1) in vec2 TexCoord;
layout(location = 0) out vec2 vTexCoord;
void main() {
    gl_Position = global.MVP * Position;
    vTexCoord = TexCoord;
}

#pragma stage fragment
layout(location = 0) in vec2 vTexCoord;
layout(location = 0) out vec4 FragColor;
layout(set = 0, binding = 2) uniform sampler2D Source;

#include "dialect.inc"
#define HOST_SAMPLE_LEVEL_ABOVE(uv) textureLod(Source, uv, 0.0)
#include "passes/common.inc"
#include "passes/halve.inc"
void main() { FragColor = crtsim_halve(vTexCoord); }
