#[vertex]
#version 450
// CC0. One triangle over the whole target, with uv running from 0 at its top left to 1 at its
// bottom right, for each pass of the Godot port to draw every pixel of its target.

layout(location = 0) out vec2 uv;

void main() {
    uv = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}

#[fragment]
#version 450
// CC0. The glass and bezel with their bloom, given back to Godot in linear light: passes/present.inc.
// CC0. What every pass of the Godot port reads, and how the passes every port of CRTSim
// Renderer shares, in passes/, read it: CRTSimEffect binds the same uniforms for each pass,
// which draws into its own target from pass_input. A port of J. Kyle Pittman's CRTSim, from
// https://github.com/aizumanga/CRTSim-Renderer.

// The sizes, the frame and every parameter, as one block CRTSimEffect fills each frame;
// written by the exporter.
// CC0. Written by CRTSim Renderer's exporter from its settings table; edit that, not this.
layout(set = 0, binding = 0, std140) uniform Parameters {
    vec2 crtsim_original_size;
    vec2 crtsim_signal_size;
    vec2 crtsim_output_size;
    float crtsim_frame_count;
    float crtsim_frame_ticks;
    float crtsim_has_previous;
    float CRTSIM_SIGNAL_WIDTH;
    float CRTSIM_SIGNAL_HEIGHT;
    float CRTSIM_RESIZE;
    float CRTSIM_TABLE;
    float CRTSIM_TABLE_NEAREST;
    float CRTSIM_LINEAR;
    float CRTSIM_PHASE;
    float CRTSIM_FLIP_ARTIFACTS;
    float CRTSIM_INTERLACE;
    float CRTSIM_FIT;
    float CRTSIM_SCREEN_ONLY;
    float CRTSIM_REFLECTION_AS_SCREEN;
    float CRTSIM_MASK_ANTIALIAS;
    float CRTSIM_MASK_COLUMNS;
    float CRTSIM_MASK_ROWS;
    float CRTSIM_PIXEL_ASPECT;
    float CRTSIM_LUT_STRENGTH;
    float CRTSIM_HUE;
    float CRTSIM_CHROMA;
    float CRTSIM_SATURATION;
    float CRTSIM_SHARPNESS;
    float CRTSIM_BLEED;
    float CRTSIM_ARTIFACTS;
    float CRTSIM_NTSC_BLENDING;
    float CRTSIM_BARREL;
    float CRTSIM_OVERSCAN;
    float CRTSIM_TUBE_ASPECT;
    float CRTSIM_DIMMING;
    float CRTSIM_FOV;
    float CRTSIM_MASK_OPACITY;
    float CRTSIM_MASK_BRIGHTNESS;
    float CRTSIM_BLOOM;
    float CRTSIM_BLOOM_POWER;
    float CRTSIM_BLOOM_SPREAD;
    float CRTSIM_DIFFUSE;
    float CRTSIM_SPECULAR;
    float CRTSIM_SPECULAR_POWER;
    float CRTSIM_RIM;
    float CRTSIM_REFLECTION;
    float CRTSIM_BEZEL_R;
    float CRTSIM_BEZEL_G;
    float CRTSIM_BEZEL_B;
    float CRTSIM_BACKDROP_R;
    float CRTSIM_BACKDROP_G;
    float CRTSIM_BACKDROP_B;
    float CRTSIM_LIGHT_X;
    float CRTSIM_LIGHT_Y;
    float CRTSIM_LIGHT_Z;
    float CRTSIM_PERSISTENCE_R;
    float CRTSIM_PERSISTENCE_G;
    float CRTSIM_PERSISTENCE_B;
} u;

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

layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 frag;

// CC0. HLSL's names for GLSL's types and functions, so RetroArch reads the passes every port of
// CRTSim Renderer shares, in passes/, as they are written; and P, how they read a parameter.

#define float2 vec2
#define float3 vec3
#define float4 vec4
#define int2 ivec2
#define int3 ivec3
#define int4 ivec4
#define lerp mix
#define frac fract
#define ddx dFdx
#define ddy dFdy
#define P(name) global.name
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
#define HOST_SAMPLE_BLOOM_INPUT(uv, second) textureLod(pass_input, uv, 0.0)
#define HOST_SAMPLE_SURFACE(uv) textureLod(surface_image, uv, 0.0)
#define HOST_SAMPLE_BLOOM(uv) textureLod(pass_input, uv, 0.0)

// CC0. The CRT's passes, written once for every shader port of CRTSim Renderer
// (https://github.com/aizumanga/CRTSim-Renderer): the same arithmetic as shaders/crtsim.wgsl
// and shaders/prepare.wgsl there, which is the reference the ports are tested against. A port
// of J. Kyle Pittman's CRTSim.
//
// The passes are written in the part of HLSL that ReShade's FX language and OBS's effects read
// as it is, and that RetroArch's GLSL reads through the names its port defines in dialect.inc.
// So no vector is made from one number (V2 and V3 spell it out), no comparison is made of whole
// vectors, no array is written, no texture is a function's argument, and no global constant is
// declared: constants are macros. And no macro without parameters is defined as something in
// parentheses, which OBS's preprocessor would read as its parameters.
//
// Each host defines what the passes read of it before including them, as HOST_ macros:
//
//   P(NAME)                 a parameter's value, as a float or an int
//   HOST_LINEAR             true when rendering in linear light
//   HOST_FRAME_COUNT        an int: the frames drawn before this one
//   HOST_FRAME_TICKS        a float: how many sixtieths of a second this frame lasts, 0 when
//                           the host does not say
//   HOST_PICTURE_ASPECT     a float: the shape the picture is meant to be shown at
//   HOST_SIGNAL_SIZE        a float2: the signal's size, which the picture is prepared at
//   HOST_OUTPUT_SIZE        a float2: the size of the screen the tube is drawn on
//   HOST_..._SIZE           a float2 per image the passes read texel by texel
//   HOST_FETCH_...(xy)      a float4: an image's texel at an int2
//   HOST_SAMPLE_...(uv)     a float4: an image sampled as its pass says, at mip level 0
//
// and each pass file lists those it reads.

#define V2(x) float2(x, x)
#define V3(x) float3(x, x, x)

float luma(float3 c) { return dot(c, float3(0.299, 0.587, 0.114)); }

float3 srgb_decode(float3 c) {
    float3 curve = pow((max(c, V3(0.0)) + V3(0.055)) / 1.055, V3(2.4));
    return lerp(curve, c / 12.92, step(c, V3(0.04045)));
}

float3 srgb_encode(float3 c) {
    float3 curve = 1.055 * pow(max(c, V3(0.0)), V3(1.0 / 2.4)) - V3(0.055);
    return lerp(curve, c * 12.92, step(c, V3(0.0031308)));
}

// Luma as the display sees it: Rec. 709 weights on linear light, NTSC's on gamma-encoded.
float display_luma(float3 c, bool linear_light) {
    return linear_light ? dot(c, float3(0.2126, 0.7152, 0.0722)) : luma(c);
}

// Rounds half away from zero, as the reference does, to whole 8-bit steps.
float3 quantize(float3 v) { return floor(clamp(v, V3(0.0), V3(255.0)) + V3(0.5)); }
// CC0. The glass and bezel with their bloom added, encoded for the display.
//
// Reads: HOST_SAMPLE_SURFACE(uv), the surface; HOST_SAMPLE_BLOOM(uv), its blur; HOST_LINEAR.

// The screen's pixel at uv.
float4 crtsim_present(float2 uv) {
    float3 base = HOST_SAMPLE_SURFACE(uv).rgb;
    float3 blurred = HOST_SAMPLE_BLOOM(uv).rgb;
    float lum = display_luma(blurred, HOST_LINEAR);
    float3 colored = blurred / max(lum, 0.000001) * pow(max(lum, 0.0), P(CRTSIM_BLOOM_POWER));
    float3 result = base + colored * P(CRTSIM_BLOOM);
    return float4(HOST_LINEAR ? srgb_encode(result) : result, 1.0);
}
void main() { frag = vec4(srgb_decode(crtsim_present(uv).rgb), 1.0); }
