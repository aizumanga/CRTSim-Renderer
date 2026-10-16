#version 450
// CC0. The colour table (a LUT or the NES palette) and the YIQ grade, applied to the core's
// picture at its own size: passes/prepare.inc.

#include "parameters.inc"

layout(std140, set = 0, binding = 0) uniform UBO {
    mat4 MVP;
    float CRTSIM_TABLE;
    float CRTSIM_TABLE_NEAREST;
    float CRTSIM_LUT_STRENGTH;
    float CRTSIM_HUE;
    float CRTSIM_CHROMA;
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
layout(set = 0, binding = 3) uniform sampler2D CRTSIM_COLOUR_TABLE;

#include "dialect.inc"
#define HOST_FETCH_PICTURE(xy) texelFetch(Source, xy, 0)
#define HOST_FETCH_TABLE(xy) texelFetch(CRTSIM_COLOUR_TABLE, xy, 0)
#define HOST_TABLE_SIZE float(textureSize(CRTSIM_COLOUR_TABLE, 0).y)
#include "passes/common.inc"
#include "passes/prepare.inc"

void main() { FragColor = crtsim_prepare(ivec2(vTexCoord * vec2(textureSize(Source, 0)))); }
