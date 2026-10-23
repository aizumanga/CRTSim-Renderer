// CC0. What the bloom's two passes share: passes/bloom.inc, reading the pass before.

layout(std140, set = 0, binding = 0) uniform UBO {
    mat4 MVP;
    vec4 FinalViewportSize;
    float CRTSIM_BLOOM_SPREAD;
    float CRTSIM_BLOOM_MIPMAPS;
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
// RetroArch makes a target's mipmaps only for the pass after it, and librashader not at all, so
// the passes before the first blur halve the surface down to the two levels it reads between.
layout(set = 0, binding = 3) uniform sampler2D CRTSIM_SURFACE;
layout(set = 0, binding = 4) uniform sampler2D CRTSIM_LEVEL4;
layout(set = 0, binding = 5) uniform sampler2D CRTSIM_LEVEL5;

#include "dialect.inc"
#define HOST_OUTPUT_SIZE global.FinalViewportSize.xy
#define HOST_SAMPLE_BLOOM_INPUT(uv, second, lod) bloom_input(uv, second, lod)

// The bloom's input: for the second pass the first's output; for the first the surface, or
// between its levels 4 and 5, which a target a sixteenth of the screen reads between at any
// size of 16 pixels or more.
vec4 bloom_input(vec2 at, bool second, float lod) {
    if (second) return textureLod(Source, at, 0.0);
    if (lod <= 0.0) return textureLod(CRTSIM_SURFACE, at, 0.0);
    return mix(textureLod(CRTSIM_LEVEL4, at, 0.0), textureLod(CRTSIM_LEVEL5, at, 0.0),
        clamp(lod - 4.0, 0.0, 1.0));
}
#include "passes/common.inc"
#include "passes/bloom.inc"
