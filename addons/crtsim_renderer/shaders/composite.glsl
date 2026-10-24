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
// CC0. The composite signal over the glow of the frame before: passes/composite.inc.
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
    float crtsim_bloom_level;
    float CRTSIM_SIGNAL_WIDTH;
    float CRTSIM_SIGNAL_HEIGHT;
    float CRTSIM_RESIZE;
    float CRTSIM_TABLE;
    float CRTSIM_TABLE_NEAREST;
    float CRTSIM_LINEAR;
    float CRTSIM_PHASE;
    float CRTSIM_FLIP_ARTIFACTS;
    float CRTSIM_GLOW_FLOOR;
    float CRTSIM_GLOW_PER_FRAME;
    float CRTSIM_INTERLACE;
    float CRTSIM_FIT;
    float CRTSIM_SCREEN_ONLY;
    float CRTSIM_REFLECTION_AS_SCREEN;
    float CRTSIM_FACETED_GLASS;
    float CRTSIM_BLOOM_MIPMAPS;
    float CRTSIM_FXAA;
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
    float CRTSIM_BLOOM_SPREAD_UP;
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
    float CRTSIM_BRIGHTNESS;
    float CRTSIM_STATIC_AMOUNT;
    float CRTSIM_STATIC_CYCLE;
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
// The two of the surface's mip levels the bloom's first blur reads between, as CRTSimEffect
// halves the surface into textures of their own: level crtsim_bloom_level and the one below.
layout(set = 0, binding = 12) uniform sampler2D bloom_level;
layout(set = 0, binding = 13) uniform sampler2D bloom_below;

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
// parentheses, which OBS's preprocessor would read as its parameters; and nothing is named u,
// the block Godot's port reads its parameters from.
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
// CC0. The composite signal: artifacts, ringing and bleed on the picture, over the glow of the
// ticks before it, which this pass reads back from its own last output.
//
// Reads: HOST_FETCH_PREPARED, the prepared signal; HOST_FETCH_PREVIOUS, this pass's output the
// frame before, when HOST_HAS_PREVIOUS says there is one at this size (a new signal size starts
// the glow again); HOST_FETCH_ARTIFACTS and HOST_ARTIFACT_SIZE, the artifact pattern;
// HOST_SIGNAL_SIZE, HOST_FRAME_COUNT and HOST_FRAME_TICKS.

float3 prepared_at(int2 xy) {
    int2 last = int2(HOST_SIGNAL_SIZE) - int2(1, 1);
    return HOST_FETCH_PREPARED(clamp(xy, int2(0, 0), last)).rgb;
}

// TV static's clock, in sixtieths of a second: a port counts each frame as one.
float static_clock() { return float(HOST_FRAME_COUNT % 16777216); }

// How strong the static is this frame: its amount, or with a cycle, the amount over a channel
// change's course: static, fading to the picture, the picture, fading back.
float static_strength() {
    float cycle = P(CRTSIM_STATIC_CYCLE);
    if (cycle <= 0.0) return P(CRTSIM_STATIC_AMOUNT);
    float course = frac(static_clock() / 60.0 / cycle);
    float fade = clamp((0.5 - course) / 0.1333333, 0.0, 1.0)
        + clamp((course - 0.8666667) / 0.1333333, 0.0, 1.0);
    return P(CRTSIM_STATIC_AMOUNT) * clamp(course < 0.3666667 ? 1.0 : fade, 0.0, 1.0);
}

// TV static, the renderer's own: white noise drawn anew each sixtieth of a second, squared,
// rolling under a darker band, over the picture wavering across and slipping down as the static
// thickens.
float static_noise(int2 xy, uint clock) {
    uint h = uint(xy.x) * 0x8da6b343u ^ uint(xy.y) * 0xd8163841u ^ clock * 0xcb1ab31fu;
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    return float(h ^ (h >> 16u)) / 4294967295.0;
}

// The picture's texel at xy as the static shows it, which is the prepared signal without any.
float3 picture_at(int2 xy, float strength) {
    if (strength <= 0.0) return prepared_at(xy);
    float2 size = HOST_SIGNAL_SIZE;
    float2 uv = (float2(xy) + V2(0.5)) / size;
    float t = static_clock() / 60.0;
    float row = uv.y;
    float waver = (sin(6.2831853 * (row * 2.3 + t * 1.7)) + 0.5 * sin(6.2831853 * (row * 5.9
        - t * 3.1)) + 0.25 * sin(6.2831853 * (row * 13.7 + t * 7.3))) * 0.012 * strength;
    float slip = strength * strength * 0.03;
    float3 shown = prepared_at(int2(floor(float2(uv.x + waver, uv.y - slip) * size)));
    int2 texel = clamp(xy, int2(0, 0), int2(size) - int2(1, 1));
    float noise = static_noise(texel, uint(static_clock()));
    float band = (frac(row - t * 0.37) - 0.5) / 0.08;
    float lit = noise * (1.0 - 0.55 * exp(-band * band));
    float snow = min(lit * lit * 1.4, 1.0);
    return lerp(shown, V3(snow), V3(strength));
}

// The last tick's signal, or black when there is none.
float3 previous_at(int2 xy) {
    if (!(HOST_HAS_PREVIOUS)) return V3(0.0);
    int2 last = int2(HOST_SIGNAL_SIZE) - int2(1, 1);
    return HOST_FETCH_PREVIOUS(clamp(xy, int2(0, 0), last)).rgb;
}

// How far this tick shows the second artifact pattern over the first: stable phase shows their
// average; A and B one each, moved towards it by the blending as Super Win the Game's NTSC
// Blending does, so 1 is the average; alternating swaps them every tick, as an NTSC set's did.
float artifact_phase() {
    float a = 0.5 * P(CRTSIM_NTSC_BLENDING);
    float b = 1.0 - a;
    int mode = int(P(CRTSIM_PHASE) + 0.5);
    if (mode == 1) return a;
    if (mode == 2) return b;
    if (mode == 3) return (HOST_FRAME_COUNT % 2) == 0 ? a : b;
    return 0.5;
}

// The persistence settings are how much glow survives a tick at 60 a second. A frame lasting
// longer or shorter keeps that much for its own length, when the host says how long it was,
// unless the glow fades once a frame, as Super Win the Game's does.
float3 persistence() {
    float3 per_tick = float3(P(CRTSIM_PERSISTENCE_R), P(CRTSIM_PERSISTENCE_G),
        P(CRTSIM_PERSISTENCE_B));
    float ticks = HOST_FRAME_TICKS;
    if (ticks > 0.0 && P(CRTSIM_GLOW_PER_FRAME) < 0.5) return pow(per_tick, V3(ticks));
    return per_tick;
}

// The level 8-bit history holds a channel's glow at once lit, with the glow floor on: the
// brightest level its persistence rounds back to itself, none where a frame of no time keeps
// all of it.
float floor_level(float kept) {
    return kept < 1.0 ? min(floor(0.5 / (1.0 - kept)), 255.0) / 255.0 : 0.0;
}

float3 glow_floor(float3 kept) {
    if (P(CRTSIM_GLOW_FLOOR) < 0.5) return V3(0.0);
    return float3(floor_level(kept.x), floor_level(kept.y), floor_level(kept.z));
}

// The difference between this pixel's luma and its two neighbours d pixels away.
float ring(int2 xy, int d, float brightness, float strength) {
    float lb = luma(picture_at(xy - int2(d, 0), strength));
    float rb = luma(picture_at(xy + int2(d, 0), strength));
    return 2.0 * brightness - lb - rb;
}

// The signal's texel at xy.
float4 crtsim_composite(int2 xy) {
    int rows = int(HOST_SIGNAL_SIZE.y);
    // Flipped, the pattern runs from the bottom up, so the second sample is the row above.
    int2 at = P(CRTSIM_FLIP_ARTIFACTS) > 0.5 ? int2(xy.x, rows - 1 - xy.y) : xy;
    float2 auv = (float2(at) + V2(0.5)) / float2(256.0, 224.0);
    float2 tile = HOST_ARTIFACT_SIZE;
    float t = artifact_phase();
    float3 a = lerp(HOST_FETCH_ARTIFACTS(int2(floor(frac(auv) * tile))).rgb,
        HOST_FETCH_ARTIFACTS(int2(floor(frac(auv + float2(0.0, 1.0 / 224.0)) * tile))).rgb,
        V3(t));
    float strength = static_strength();
    float3 left = picture_at(xy - int2(1, 0), strength);
    float3 right = picture_at(xy + int2(1, 0), strength);
    float3 cur = picture_at(xy, strength);
    float3 color = clamp(cur + (left + right - 2.0 * cur) * a * P(CRTSIM_ARTIFACTS), V3(0.0),
        V3(1.0));
    float brightness = luma(color);
    float offset = 0.0;
    offset += ring(xy, 1, brightness, strength) * 1.0;
    offset += ring(xy, 2, brightness, strength) * (-0.3162277);
    offset += ring(xy, 3, brightness, strength) * 0.1;
    color = clamp(color + offset * P(CRTSIM_SHARPNESS) * lerp(V3(1.0), a, V3(P(CRTSIM_ARTIFACTS))),
        V3(0.0), V3(1.0));
    // Interlaced: the beam skips the other field's rows this tick; they only decay below. Auto
    // interlaces a picture of 400 rows or more, as a set drew a 480- or 576-line one.
    int interlace = int(P(CRTSIM_INTERLACE) + 0.5);
    bool interlaced = interlace == 1 || (interlace == 2 && rows >= 400);
    if (interlaced && (xy.y % 2) != (HOST_FRAME_COUNT % 2)) color = V3(0.0);
    float3 kept = persistence();
    float3 held = glow_floor(kept);
    float3 pl = max(previous_at(xy - int2(1, 0)), held);
    float3 pr = max(previous_at(xy + int2(1, 0)), held);
    float3 pc = max(previous_at(xy), held);
    float bleed = P(CRTSIM_BLEED);
    color = max(color, kept * (pc + (pl + pr) * bleed) / (1.0 + 2.0 * bleed));
    return float4(clamp(color, V3(0.0), V3(1.0)), 1.0);
}
void main() { frag = crtsim_composite(ivec2(uv * u.crtsim_signal_size)); }
