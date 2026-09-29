#version 450
// CC0. The composite signal: artifacts, ringing and bleed on the picture, over the glow of the
// ticks before it, which this pass reads back as its own feedback.

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
    float CRTSIM_PHASE;
    float CRTSIM_PERSISTENCE_R;
    float CRTSIM_PERSISTENCE_G;
    float CRTSIM_PERSISTENCE_B;
    float CRTSIM_INTERLACE;
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
layout(set = 0, binding = 3) uniform sampler2D CRTSIM_SIGNALFeedback;
layout(set = 0, binding = 4) uniform sampler2D CRTSIM_ARTIFACT_PATTERN;

vec3 source_at(ivec2 xy) {
    ivec2 last = ivec2(global.SourceSize.xy) - ivec2(1);
    return texelFetch(Source, clamp(xy, ivec2(0), last), 0).rgb;
}

// The last tick's signal, or black when there is none at this size: a new signal size starts
// the glow again.
vec3 previous_at(ivec2 xy) {
    if (global.CRTSIM_SIGNALFeedbackSize.xy != global.SourceSize.xy) return vec3(0.0);
    ivec2 last = ivec2(global.SourceSize.xy) - ivec2(1);
    return texelFetch(CRTSIM_SIGNALFeedback, clamp(xy, ivec2(0), last), 0).rgb;
}

// How far this tick shows the second artifact pattern over the first: stable phase shows their
// average; A and B one each, moved towards the other by the blending; alternating swaps them
// every tick, as an NTSC set's did.
float phase() {
    float a = global.CRTSIM_NTSC_BLENDING, b = 1.0 - global.CRTSIM_NTSC_BLENDING;
    int mode = int(global.CRTSIM_PHASE + 0.5);
    if (mode == 1) return a;
    if (mode == 2) return b;
    if (mode == 3) return (global.FrameCount % 2u) == 0u ? a : b;
    return 0.5;
}

// The persistence settings are how much glow survives a tick at 60 a second. A frame lasting
// longer or shorter keeps that much for its own length, when RetroArch says how long it was.
vec3 persistence() {
    vec3 per_tick = vec3(global.CRTSIM_PERSISTENCE_R, global.CRTSIM_PERSISTENCE_G,
        global.CRTSIM_PERSISTENCE_B);
#ifdef _HAS_FRAMETIME_UNIFORMS
    if (global.FrameTimeDelta > 0u)
        return pow(per_tick, vec3(60.0 * float(global.FrameTimeDelta) / 1000000.0));
#endif
    return per_tick;
}

void main() {
    ivec2 xy = ivec2(vTexCoord * global.SourceSize.xy);
    vec2 auv = (vec2(xy) + vec2(0.5)) / vec2(256.0, 224.0);
    vec2 tile = vec2(textureSize(CRTSIM_ARTIFACT_PATTERN, 0));
    vec3 a = mix(texelFetch(CRTSIM_ARTIFACT_PATTERN, ivec2(floor(fract(auv) * tile)), 0).rgb,
        texelFetch(CRTSIM_ARTIFACT_PATTERN, ivec2(floor(fract(auv + vec2(0.0, 1.0 / 224.0)) * tile)), 0).rgb,
        phase());
    vec3 left = source_at(xy - ivec2(1, 0));
    vec3 right = source_at(xy + ivec2(1, 0));
    vec3 cur = source_at(xy);
    vec3 color = clamp(cur + (left + right - 2.0 * cur) * a * global.CRTSIM_ARTIFACTS, 0.0, 1.0);
    float brt = luma(color);
    float offset = 0.0;
    float weights[3] = float[3](1.0, -0.3162277, 0.1);
    for (int i = 0; i < 3; i++) {
        float lb = luma(source_at(xy - ivec2(i + 1, 0)));
        float rb = luma(source_at(xy + ivec2(i + 1, 0)));
        offset += (2.0 * brt - lb - rb) * weights[i];
    }
    color = clamp(color + offset * global.CRTSIM_SHARPNESS * mix(vec3(1.0), a, global.CRTSIM_ARTIFACTS),
        0.0, 1.0);
    // Interlaced: the beam skips the other field's rows this tick; they only decay below. Auto
    // interlaces a picture of 400 rows or more, as a set drew a 480- or 576-line one.
    int interlace = int(global.CRTSIM_INTERLACE + 0.5);
    bool interlaced = interlace == 1 || (interlace == 2 && global.SourceSize.y >= 400.0);
    if (interlaced && uint(xy.y) % 2u != global.FrameCount % 2u) color = vec3(0.0);
    vec3 pl = previous_at(xy - ivec2(1, 0));
    vec3 pr = previous_at(xy + ivec2(1, 0));
    vec3 pc = previous_at(xy);
    float bleed = global.CRTSIM_BLEED;
    color = max(color, persistence() * (pc + (pl + pr) * bleed) / (1.0 + 2.0 * bleed));
    FragColor = vec4(clamp(color, 0.0, 1.0), 1.0);
}
