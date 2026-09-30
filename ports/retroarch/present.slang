#version 450
// CC0. The glass and bezel with their bloom added, encoded for the display.

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
#include "common.inc"
layout(location = 0) in vec2 vTexCoord;
layout(location = 0) out vec4 FragColor;
layout(set = 0, binding = 2) uniform sampler2D Source;
layout(set = 0, binding = 3) uniform sampler2D CRTSIM_SURFACE;

void main() {
    bool linear = global.CRTSIM_LINEAR > 0.5;
    vec3 base = textureLod(CRTSIM_SURFACE, vTexCoord, 0.0).rgb;
    vec3 blurred = textureLod(Source, vTexCoord, 0.0).rgb;
    float lum = display_luma(blurred, linear);
    vec3 colored = blurred / max(lum, 0.000001) * pow(max(lum, 0.0), global.CRTSIM_BLOOM_POWER);
    vec3 result = base + colored * global.CRTSIM_BLOOM;
    FragColor = vec4(linear ? srgb_encode(result) : result, 1.0);
}
