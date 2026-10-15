#version 450
// CC0. The glass and bezel with their bloom added, encoded for the display: passes/present.inc.

#include "parameters.inc"

layout(std140, set = 0, binding = 0) uniform UBO {
    mat4 MVP;
    float CRTSIM_BLOOM;
    float CRTSIM_BLOOM_POWER;
    float CRTSIM_LINEAR;
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
layout(set = 0, binding = 3) uniform sampler2D CRTSIM_SURFACE;

#include "dialect.inc"
#define HOST_LINEAR (global.CRTSIM_LINEAR > 0.5)
#define HOST_SAMPLE_SURFACE(uv) textureLod(CRTSIM_SURFACE, uv, 0.0)
#define HOST_SAMPLE_BLOOM(uv) textureLod(Source, uv, 0.0)
#include "passes/common.inc"
#include "passes/present.inc"

void main() { FragColor = crtsim_present(vTexCoord); }
