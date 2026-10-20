#version 450
// CC0. The curved glass and its bezel, lit, drawn on the screen: passes/surface.inc.

#include "parameters.inc"

layout(std140, set = 0, binding = 0) uniform UBO {
    mat4 MVP;
    vec4 SourceSize;
    vec4 OutputSize;
#ifdef _HAS_ORIGINALASPECT_UNIFORMS
    float OriginalAspect;
#endif
    float CRTSIM_ASPECT;
    float CRTSIM_FIT;
    float CRTSIM_OVERSCAN;
    float CRTSIM_BARREL;
    float CRTSIM_TUBE_ASPECT;
    float CRTSIM_SATURATION;
    float CRTSIM_MASK_BRIGHTNESS;
    float CRTSIM_MASK_OPACITY;
    float CRTSIM_MASK_COLUMNS;
    float CRTSIM_MASK_ROWS;
    float CRTSIM_MASK_ANTIALIAS;
    float CRTSIM_DIMMING;
    float CRTSIM_DIFFUSE;
    float CRTSIM_SPECULAR;
    float CRTSIM_SPECULAR_POWER;
    float CRTSIM_RIM;
    float CRTSIM_LIGHT_X;
    float CRTSIM_LIGHT_Y;
    float CRTSIM_LIGHT_Z;
    float CRTSIM_FOV;
    float CRTSIM_LINEAR;
    float CRTSIM_SCREEN_ONLY;
    float CRTSIM_REFLECTION;
    float CRTSIM_BEZEL_R;
    float CRTSIM_BEZEL_G;
    float CRTSIM_BEZEL_B;
    float CRTSIM_REFLECTION_AS_SCREEN;
    float CRTSIM_BACKDROP_R;
    float CRTSIM_BACKDROP_G;
    float CRTSIM_BACKDROP_B;
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
layout(set = 0, binding = 3) uniform sampler2D CRTSIM_SHADOW_MASK;
layout(set = 0, binding = 4) uniform sampler2D CRTSIM_BEZEL_SHAPE;
layout(set = 0, binding = 5) uniform sampler2D CRTSIM_BEZEL_UV;
layout(set = 0, binding = 6) uniform sampler2D CRTSIM_BEZEL_NORMAL;
layout(set = 0, binding = 7) uniform sampler2D CRTSIM_GLASS;

// The picture's shape as the core means it to be shown, else as the preset says.
float picture_aspect() {
#ifdef _HAS_ORIGINALASPECT_UNIFORMS
    if (global.OriginalAspect > 0.0) return global.OriginalAspect;
#endif
    return global.CRTSIM_ASPECT;
}

#include "dialect.inc"
#define HOST_LINEAR (global.CRTSIM_LINEAR > 0.5)
#define HOST_PICTURE_ASPECT picture_aspect()
#define HOST_SIGNAL_SIZE global.SourceSize.xy
#define HOST_OUTPUT_SIZE global.OutputSize.xy
#define HOST_FETCH_SIGNAL(xy) texelFetch(Source, xy, 0)
#define HOST_SAMPLE_MASK(uv, lod) textureLod(CRTSIM_SHADOW_MASK, uv, lod)
#define HOST_MASK_SIZE vec2(textureSize(CRTSIM_SHADOW_MASK, 0))
#define HOST_FETCH_BEZEL_SHAPE(xy) texelFetch(CRTSIM_BEZEL_SHAPE, xy, 0)
#define HOST_SAMPLE_BEZEL_SHAPE(uv) textureLod(CRTSIM_BEZEL_SHAPE, uv, 0.0)
#define HOST_FETCH_BEZEL_UV(xy) texelFetch(CRTSIM_BEZEL_UV, xy, 0)
#define HOST_FETCH_BEZEL_NORMAL(xy) texelFetch(CRTSIM_BEZEL_NORMAL, xy, 0)
#define HOST_BEZEL_SIZE vec2(textureSize(CRTSIM_BEZEL_SHAPE, 0))
#define HOST_FETCH_GLASS(xy) texelFetch(CRTSIM_GLASS, xy, 0)
#define HOST_GLASS_SIZE vec2(textureSize(CRTSIM_GLASS, 0))
#include "passes/common.inc"
#include "passes/surface.inc"

void main() { FragColor = crtsim_surface(vTexCoord); }
