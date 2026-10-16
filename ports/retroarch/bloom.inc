// CC0. What the bloom's two passes share: passes/bloom.inc, reading the pass before.

layout(std140, set = 0, binding = 0) uniform UBO {
    mat4 MVP;
    vec4 FinalViewportSize;
    float CRTSIM_BLOOM_SPREAD;
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
#define HOST_OUTPUT_SIZE global.FinalViewportSize.xy
#define HOST_SAMPLE_BLOOM_INPUT(uv, second) textureLod(Source, uv, 0.0)
#include "passes/common.inc"
#include "passes/bloom.inc"
