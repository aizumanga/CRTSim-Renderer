// CC0. What every pass of the Godot port reads, and how the passes every port of CRTSim
// Renderer shares, in passes/, read it: CRTSimEffect binds the same uniforms for each pass,
// which draws into its own target from pass_input. A port of J. Kyle Pittman's CRTSim, from
// https://github.com/aizumanga/CRTSim-Renderer.

// The sizes, the frame and every parameter, as one block CRTSimEffect fills each frame;
// written by the exporter.
#include "parameters.inc"

// What the pass reads: the camera's picture, its rows resized, the picture at the signal's
// size, the prepared signal, the signal, the surface, or the first bloom.
layout(set = 0, binding = 1) uniform sampler2D pass_input;
// The surface again, for present, whose input is the bloom.
layout(set = 0, binding = 2) uniform sampler2D surface_image;
// The last frame's signal, when crtsim_has_previous says there is one.
layout(set = 0, binding = 3) uniform sampler2D previous_signal;
layout(set = 0, binding = 4) uniform sampler2D table_image;
layout(set = 0, binding = 5) uniform sampler2D artifacts_image;
layout(set = 0, binding = 6) uniform sampler2D mask_image;
layout(set = 0, binding = 7) uniform sampler2D bezel_shape_image;
// The same image, read filtered for the depth's high byte a ray steps on.
layout(set = 0, binding = 8) uniform sampler2D bezel_shape_filtered;
layout(set = 0, binding = 9) uniform sampler2D bezel_uv_image;
layout(set = 0, binding = 10) uniform sampler2D bezel_normal_image;
layout(set = 0, binding = 11) uniform sampler2D glass_image;
// The two of the surface's mip levels the bloom's first blur reads between, as CRTSimEffect
// halves the surface into textures of their own: level crtsim_bloom_level and the one below.
layout(set = 0, binding = 12) uniform sampler2D bloom_level;
layout(set = 0, binding = 13) uniform sampler2D bloom_below;

layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 frag;

#include "dialect.inc"
#undef P
#define P(name) u.name
#define HOST_LINEAR (u.CRTSIM_LINEAR > 0.5)
#define HOST_FRAME_COUNT int(u.crtsim_frame_count)
#define HOST_FRAME_TICKS u.crtsim_frame_ticks
#define HOST_PICTURE_ASPECT (u.crtsim_signal_size.x / u.crtsim_signal_size.y * u.CRTSIM_PIXEL_ASPECT)
#define HOST_SIGNAL_SIZE u.crtsim_signal_size
#define HOST_OUTPUT_SIZE u.crtsim_output_size
#define HOST_ORIGINAL_SIZE u.crtsim_original_size
#define HOST_RESIZE_NEAREST (u.CRTSIM_RESIZE < 0.5)
#define HOST_FETCH_ORIGINAL(xy) texelFetch(pass_input, xy, 0)
#define HOST_FETCH_ROWS(xy) texelFetch(pass_input, xy, 0)
#define HOST_FETCH_PICTURE(xy) texelFetch(pass_input, xy, 0)
#define HOST_TABLE_SIZE float(textureSize(table_image, 0).y)
#define HOST_FETCH_TABLE(xy) texelFetch(table_image, xy, 0)
#define HOST_FETCH_PREPARED(xy) texelFetch(pass_input, xy, 0)
#define HOST_HAS_PREVIOUS (u.crtsim_has_previous > 0.5)
#define HOST_FETCH_PREVIOUS(xy) texelFetch(previous_signal, xy, 0)
#define HOST_ARTIFACT_SIZE vec2(textureSize(artifacts_image, 0))
#define HOST_FETCH_ARTIFACTS(xy) texelFetch(artifacts_image, xy, 0)
#define HOST_FETCH_SIGNAL(xy) texelFetch(pass_input, xy, 0)
#define HOST_MASK_SIZE vec2(textureSize(mask_image, 0))
#define HOST_SAMPLE_MASK(uv, lod) textureLod(mask_image, uv, lod)
#define HOST_BEZEL_SIZE vec2(textureSize(bezel_shape_image, 0))
#define HOST_FETCH_BEZEL_SHAPE(xy) texelFetch(bezel_shape_image, xy, 0)
#define HOST_SAMPLE_BEZEL_SHAPE(uv) textureLod(bezel_shape_filtered, uv, 0.0)
#define HOST_FETCH_BEZEL_UV(xy) texelFetch(bezel_uv_image, xy, 0)
#define HOST_FETCH_BEZEL_NORMAL(xy) texelFetch(bezel_normal_image, xy, 0)
#define HOST_GLASS_SIZE vec2(textureSize(glass_image, 0))
#define HOST_FETCH_GLASS(xy) texelFetch(glass_image, xy, 0)
#define HOST_SAMPLE_BLOOM_INPUT(uv, second, lod) bloom_input(uv, second, lod)
#define HOST_SAMPLE_SURFACE(uv) textureLod(surface_image, uv, 0.0)
#define HOST_SAMPLE_BLOOM(uv) textureLod(pass_input, uv, 0.0)
#define HOST_SAMPLE_LEVEL_ABOVE(uv) textureLod(pass_input, uv, 0.0)

// The bloom's input: for the second pass the first's output, for the first the surface, or
// between two of its levels.
vec4 bloom_input(vec2 at, bool second, float lod) {
    if (second || lod <= 0.0) return textureLod(pass_input, at, 0.0);
    return mix(textureLod(bloom_level, at, 0.0), textureLod(bloom_below, at, 0.0),
        clamp(lod - u.crtsim_bloom_level, 0.0, 1.0));
}

#include "passes/common.inc"
