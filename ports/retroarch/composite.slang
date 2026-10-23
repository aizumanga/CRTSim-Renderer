#version 450
// CC0. The composite signal over the glow of the ticks before it, which this pass reads back as
// its own feedback: passes/composite.inc.

#include "parameters.inc"

layout(std140, set = 0, binding = 0) uniform UBO {
    mat4 MVP;
    vec4 SourceSize;
    vec4 CRTSIM_SIGNALFeedbackSize;
    uint FrameCount;
#ifdef _HAS_FRAMETIME_UNIFORMS
    uint FrameTimeDelta;
#endif
    float CRTSIM_SHARPNESS;
    float CRTSIM_BLEED;
    float CRTSIM_ARTIFACTS;
    float CRTSIM_NTSC_BLENDING;
    float CRTSIM_FLIP_ARTIFACTS;
    float CRTSIM_PHASE;
    float CRTSIM_PERSISTENCE_R;
    float CRTSIM_PERSISTENCE_G;
    float CRTSIM_PERSISTENCE_B;
    float CRTSIM_INTERLACE;
    float CRTSIM_GLOW_FLOOR;
    float CRTSIM_GLOW_PER_FRAME;
    float CRTSIM_STATIC;
    float CRTSIM_STATIC_CYCLE;
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
layout(set = 0, binding = 3) uniform sampler2D CRTSIM_SIGNALFeedback;
layout(set = 0, binding = 4) uniform sampler2D CRTSIM_ARTIFACT_PATTERN;

#include "dialect.inc"
#define HOST_SIGNAL_SIZE global.SourceSize.xy
#define HOST_FETCH_PREPARED(xy) texelFetch(Source, xy, 0)
// Feedback of another size is a signal size that has changed.
#define HOST_HAS_PREVIOUS (global.CRTSIM_SIGNALFeedbackSize.xy == global.SourceSize.xy)
#define HOST_FETCH_PREVIOUS(xy) texelFetch(CRTSIM_SIGNALFeedback, xy, 0)
#define HOST_FETCH_ARTIFACTS(xy) texelFetch(CRTSIM_ARTIFACT_PATTERN, xy, 0)
#define HOST_ARTIFACT_SIZE vec2(textureSize(CRTSIM_ARTIFACT_PATTERN, 0))
#define HOST_FRAME_COUNT int(global.FrameCount)
#ifdef _HAS_FRAMETIME_UNIFORMS
#define HOST_FRAME_TICKS float(60.0 * float(global.FrameTimeDelta) / 1000000.0)
#else
#define HOST_FRAME_TICKS 0.0
#endif
#include "passes/common.inc"
#include "passes/composite.inc"

void main() { FragColor = crtsim_composite(ivec2(vTexCoord * global.SourceSize.xy)); }
