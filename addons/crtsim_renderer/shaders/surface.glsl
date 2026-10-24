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
// CC0. The curved glass and its bezel, lit: passes/surface.inc.
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
// CC0. The curved glass and its bezel: for each pixel, where a ray from the camera meets the
// screen, a cap of a sphere of radius 4 centred 4 units behind its middle within the outline its
// mesh baked, or the bezel, baked from its mesh seen straight on, whichever it meets first, and
// how that point is lit. The reference renderer draws them with this same pass.
//
// Reads: HOST_FETCH_SIGNAL and HOST_SIGNAL_SIZE, the composite signal; HOST_SAMPLE_MASK(uv,
// lod), the shadow mask, repeating, filtered with mipmaps, and HOST_MASK_SIZE;
// HOST_FETCH_BEZEL_SHAPE, HOST_FETCH_BEZEL_UV, HOST_FETCH_BEZEL_NORMAL and HOST_BEZEL_SIZE, the
// bezel's depth, shade and reflection, its UV and its normal, 16-bit values split over two
// channels, high byte first; HOST_SAMPLE_BEZEL_SHAPE(uv), the first filtered, for its depth's
// high byte; HOST_FETCH_GLASS and HOST_GLASS_SIZE, two maps baked from the glass's mesh across
// its UVs, one above the other, texel (i, j) of each at UV (i, j) / (size - 1): the glass's shade
// and the distance to its outline, positive inside and within ±GLASS_EDGE, then how far behind
// the sphere its flat triangles lie, up to GLASS_FACET; HOST_OUTPUT_SIZE, HOST_PICTURE_ASPECT and
// HOST_LINEAR.
//
// A tube wider or narrower than 4:3 (CRTSIM_TUBE_ASPECT) stretches the glass's sphere across,
// and moves the bezel's sides as `original_y` says.

#define GLASS_CENTRE float3(4.0, 0.0, 0.0)
#define GLASS_RADIUS 4.0
#define GLASS_EDGE 0.015625
#define GLASS_FACET 0.00048828125
#define BEZEL_HALF float2(1.6533334, 1.32)
#define BEZEL_NEAR float(-0.1)
#define BEZEL_FAR 0.33
#define BEZEL_OPENING_Y 1.24
#define BEZEL_OPENING_Z 0.95
// Steps across the bezel's depth, then halvings of the step a ray first passes the surface in.
#define BEZEL_STEPS 32
#define BEZEL_HALVINGS 7
#define NOTHING 1e9

// A tube other than the original's 4:3 is as much wider, or narrower, either side: its glass
// the original's stretched across, its bezel's corners and sides moved out, or in, and its top
// and bottom running between them, as the original's middle. A look's 4:3, written as a
// rounded number, is the original's exactly.
float tube_aspect() {
    return abs(P(CRTSIM_TUBE_ASPECT) - 4.0 / 3.0) < 0.0005 ? 4.0 / 3.0 : P(CRTSIM_TUBE_ASPECT);
}
float widened() { return tube_aspect() - 4.0 / 3.0; }
float stretched() { return tube_aspect() / (4.0 / 3.0); }
// Where y across a tube this wide falls across the original's.
float original_y(float y) { return sign(y) * max(abs(y) - widened(), 0.0); }

// How the picture sits on the tube: as the original placed it, fitted inside, filling it, or
// stretched over it.
float2 uv_scale() {
    float ratio = HOST_PICTURE_ASPECT / tube_aspect();
    int fit = int(P(CRTSIM_FIT) + 0.5);
    if (fit == 3) return V2(1.0);
    if (fit == 1) return ratio >= 1.0 ? float2(1.0, ratio) : float2(1.0 / ratio, 1.0);
    if (fit == 2) return ratio >= 1.0 ? float2(1.0 / ratio, 1.0) : float2(1.0, ratio);
    return float2(1.0, ratio);
}

// The mask's columns and rows across the screen: set by the look, or one column for every two
// of the signal's and a row for each of its rows.
float2 mask_repeats() {
    if (P(CRTSIM_MASK_COLUMNS) > 0.0 && P(CRTSIM_MASK_ROWS) > 0.0)
        return float2(P(CRTSIM_MASK_COLUMNS), P(CRTSIM_MASK_ROWS));
    return float2(HOST_SIGNAL_SIZE.x / 2.0, HOST_SIGNAL_SIZE.y);
}

// The signal with D3D9's black border, filtered bilinearly across it as the original was.
float3 border_load(int2 xy) {
    int2 size = int2(HOST_SIGNAL_SIZE);
    if (xy.x < 0 || xy.y < 0 || xy.x >= size.x || xy.y >= size.y) return V3(0.0);
    float3 c = HOST_FETCH_SIGNAL(xy).rgb;
    return HOST_LINEAR ? srgb_decode(c) : c;
}

float3 black_border(float2 uv) {
    float2 pixel = uv * HOST_SIGNAL_SIZE - V2(0.5);
    int2 xy = int2(floor(pixel));
    float2 f = frac(pixel);
    return lerp(lerp(border_load(xy), border_load(xy + int2(1, 0)), V3(f.x)),
        lerp(border_load(xy + int2(0, 1)), border_load(xy + int2(1, 1)), V3(f.x)), V3(f.y));
}

// The signal at `uv` through the mask, on the glass or reflected in the bezel; `across` and
// `down` are how uv changes from one pixel to the next, which say how small the mask is drawn.
float3 crt(float2 uv, float2 across, float2 down, bool bezel) {
    float2 fitted = uv_scale();
    float2 scaled = (uv - V2(0.5)) * fitted + V2(0.5);
    // The public source reflects half the mask's rows in the bezel, and the overscan
    // unreciprocated; Super Win the Game reflects them as the screen shows them.
    bool as_screen = !bezel || P(CRTSIM_REFLECTION_AS_SCREEN) > 0.5;
    float2 density = mask_repeats() * (as_screen ? V2(1.0) : float2(1.0, 0.5));
    float2 mask_uv = scaled * density;
    float2 scale = fitted * density * HOST_MASK_SIZE;
    float footprint = max(length(across * scale), length(down * scale));
    float lod = P(CRTSIM_MASK_ANTIALIAS) > 0.5 ? max(log2(max(footprint, 0.000001)), 0.0) : 0.0;
    float3 grid = HOST_SAMPLE_MASK(mask_uv, lod).rgb;
    grid = lerp(V3(1.0), grid + V3(P(CRTSIM_MASK_BRIGHTNESS)), V3(P(CRTSIM_MASK_OPACITY)));
    float over = as_screen ? 1.0 / P(CRTSIM_OVERSCAN) : P(CRTSIM_OVERSCAN);
    float2 pos = (scaled - V2(0.5)) * over;
    pos = pos + pos * P(CRTSIM_BARREL) * dot(pos, pos) + V2(0.5);
    float3 emissive = black_border(pos) * grid;
    return lerp(V3(display_luma(emissive, HOST_LINEAR)), emissive, V3(P(CRTSIM_SATURATION)));
}

float3 safe_normalize(float3 v) { return dot(v, v) < 0.000000000001 ? V3(0.0) : normalize(v); }

// A point of the glass or the bezel, with what shading it needs.
struct Surface {
    float3 normal;
    float shade;
    float2 uv;
    float reflection;
    float3 camera;
    float3 light;
};

float4 light_surface(Surface v, float2 across, float2 down, bool bezel) {
    float3 n = safe_normalize(v.normal);
    float3 cam = safe_normalize(v.camera);
    float3 light = safe_normalize(v.light);
    float diffuse = max(dot(n, light), 0.0);
    float3 halfway = safe_normalize(light + cam);
    float spec = pow(max(dot(n, halfway), 0.0), P(CRTSIM_SPECULAR_POWER));
    // Squared by multiplying, as the original does, which pow leaves undefined below 0.
    float grazing = 1.0 - dot(cam, n);
    float fres = grazing * grazing * P(CRTSIM_RIM);
    float3 color = crt(v.uv, across, down, bezel);
    if (bezel) {
        float hemi = (dot(n, float3(0.0, 0.0, 1.0)) * 0.5 + 0.5) * 0.4 + 0.3;
        float3 body = float3(P(CRTSIM_BEZEL_R), P(CRTSIM_BEZEL_G), P(CRTSIM_BEZEL_B));
        if (HOST_LINEAR) body = srgb_decode(body);
        color = body * (diffuse + hemi) * P(CRTSIM_DIFFUSE) + V3(0.25) * spec * P(CRTSIM_SPECULAR)
            + color * v.reflection * P(CRTSIM_REFLECTION) + V3(0.15) * fres;
    } else {
        color += float3(0.175, 0.15, 0.2) * diffuse * P(CRTSIM_DIFFUSE)
            + V3(0.25) * spec * P(CRTSIM_SPECULAR) + float3(0.45, 0.4, 0.5) * fres;
    }
    return float4(color * lerp(1.0, v.shade, P(CRTSIM_DIMMING)), 1.0);
}

float pair(float hi, float lo) { return (round(hi * 255.0) * 256.0 + round(lo * 255.0)) / 65534.0; }

float2 pairs_of(float4 k) { return float2(pair(k.r, k.g), pair(k.b, k.a)); }

// Four neighbouring texels' pairs decoded and blended by f.
float2 blend_pairs(float4 k00, float4 k10, float4 k01, float4 k11, float2 f) {
    return lerp(lerp(pairs_of(k00), pairs_of(k10), V2(f.x)),
        lerp(pairs_of(k01), pairs_of(k11), V2(f.x)), V2(f.y));
}

// Where (y, z) falls on the bezel's grid, in texels.
float2 bezel_texel(float2 yz) {
    return (BEZEL_HALF - float2(original_y(yz.x), yz.y)) / (2.0 * BEZEL_HALF) * HOST_BEZEL_SIZE
        - V2(0.5);
}

int2 bezel_clamp(int2 xy) { return clamp(xy, int2(0, 0), int2(HOST_BEZEL_SIZE) - int2(1, 1)); }
float4 bezel_shape_at(int2 xy) { return HOST_FETCH_BEZEL_SHAPE(bezel_clamp(xy)); }
float4 bezel_uv_at(int2 xy) { return HOST_FETCH_BEZEL_UV(bezel_clamp(xy)); }
float4 bezel_normal_at(int2 xy) { return HOST_FETCH_BEZEL_NORMAL(bezel_clamp(xy)); }

float bezel_depth_at(int2 xy) {
    int2 size = int2(HOST_BEZEL_SIZE);
    float4 texel = bezel_shape_at(xy);
    bool empty = xy.x < 0 || xy.y < 0 || xy.x >= size.x || xy.y >= size.y
        || (texel.r > 0.999 && texel.g > 0.999);
    return empty ? NOTHING : lerp(BEZEL_NEAR, BEZEL_FAR, pair(texel.r, texel.g));
}

// The bezel's depth at (y, z), blended between texels; nothing where any of them is empty.
float bezel_depth(float2 yz) {
    float2 t = bezel_texel(yz);
    int2 xy = int2(floor(t));
    float2 f = frac(t);
    float a = bezel_depth_at(xy);
    float b = bezel_depth_at(xy + int2(1, 0));
    float c = bezel_depth_at(xy + int2(0, 1));
    float d = bezel_depth_at(xy + int2(1, 1));
    if (max(max(a, b), max(c, d)) >= NOTHING) return NOTHING;
    return lerp(lerp(a, b, f.x), lerp(c, d, f.x), f.y);
}

float2 bezel_uv(float2 yz) {
    float2 g = bezel_texel(yz);
    int2 xy = int2(floor(g));
    return blend_pairs(bezel_uv_at(xy), bezel_uv_at(xy + int2(1, 0)), bezel_uv_at(xy + int2(0, 1)),
        bezel_uv_at(xy + int2(1, 1)), frac(g));
}

float2 bezel_facing(float2 yz) {
    float2 g = bezel_texel(yz);
    int2 xy = int2(floor(g));
    return blend_pairs(bezel_normal_at(xy), bezel_normal_at(xy + int2(1, 0)),
        bezel_normal_at(xy + int2(0, 1)), bezel_normal_at(xy + int2(1, 1)), frac(g));
}

// The bezel's shade and reflection at (y, z).
float2 bezel_bytes(float2 yz) {
    float2 g = bezel_texel(yz);
    int2 xy = int2(floor(g));
    float2 f = frac(g);
    float2 a = bezel_shape_at(xy).ba;
    float2 b = bezel_shape_at(xy + int2(1, 0)).ba;
    float2 c = bezel_shape_at(xy + int2(0, 1)).ba;
    float2 d = bezel_shape_at(xy + int2(1, 1)).ba;
    return lerp(lerp(a, b, V2(f.x)), lerp(c, d, V2(f.x)), V2(f.y));
}

float4 glass_at_texel(int2 xy) {
    return HOST_FETCH_GLASS(clamp(xy, int2(0, 0), int2(HOST_GLASS_SIZE) - int2(1, 1)));
}

// The glass's two values at uv, from its map `rows` down.
float2 glass_pairs(float2 uv, float rows) {
    float2 half_size = float2(HOST_GLASS_SIZE.x, HOST_GLASS_SIZE.y * 0.5);
    float2 g = clamp(uv, V2(0.0), V2(1.0)) * (half_size - V2(1.0)) + float2(0.0, rows);
    int2 xy = int2(floor(g));
    return blend_pairs(glass_at_texel(xy), glass_at_texel(xy + int2(1, 0)),
        glass_at_texel(xy + int2(0, 1)), glass_at_texel(xy + int2(1, 1)), frac(g));
}

// The glass's shade and distance to its outline at uv.
float2 glass_baked(float2 uv) {
    float2 v = glass_pairs(uv, 0.0);
    return float2(v.x, lerp(-GLASS_EDGE, GLASS_EDGE, v.y));
}

// How far behind the sphere the mesh's facet lies at uv, along the view.
float glass_facet(float2 uv) { return glass_pairs(uv, HOST_GLASS_SIZE.y * 0.5).x * GLASS_FACET; }

// The point a ray from `camera` along `ray` meets at depth x.
float3 along(float3 camera, float3 ray, float x) { return camera + ray * ((x - camera.x) / ray.x); }

// Where the ray first meets the bezel, NOTHING when it does not: stepping across its depths on
// the depth's high byte, filtered by the sampler, then halving the step it passed the surface
// in on the full depth. At the bezel's outer rim the filtered byte blends in the empty texels
// past it, so a step can seem inside where the full depth is still empty; the march goes on
// from there. It takes no derivatives, so it may stop at the first step inside.
float bezel_hit(float3 camera, float3 ray) {
    float stride = (BEZEL_FAR - BEZEL_NEAR) / float(BEZEL_STEPS);
    float before = BEZEL_NEAR - stride;
    for (int i = 0; i <= BEZEL_STEPS; i++) {
        float x = BEZEL_NEAR + stride * float(i);
        float2 at = (bezel_texel(along(camera, ray, x).yz) + V2(0.5)) / HOST_BEZEL_SIZE;
        float coarse = lerp(BEZEL_NEAR, BEZEL_FAR,
            HOST_SAMPLE_BEZEL_SHAPE(at).r * 255.0 * 256.0 / 65534.0);
        if (x >= coarse && at.x >= 0.0 && at.y >= 0.0 && at.x <= 1.0 && at.y <= 1.0) {
            float low = before;
            float high = x;
            for (int j = 0; j < BEZEL_HALVINGS; j++) {
                float middle = 0.5 * (low + high);
                bool inside = middle >= bezel_depth(along(camera, ray, middle).yz);
                high = inside ? middle : high;
                low = inside ? low : middle;
            }
            if (bezel_depth(along(camera, ray, high).yz) < NOTHING) return high;
        }
        before = x;
    }
    return NOTHING;
}

// Whether a ray can meet the bezel at all: not if it stays within the opening's clear box
// across the bezel's whole depth.
bool may_meet_bezel(float3 camera, float3 ray) {
    float2 widest = max(abs(along(camera, ray, BEZEL_NEAR).yz), abs(along(camera, ray, BEZEL_FAR).yz));
    return widest.x >= BEZEL_OPENING_Y + widened() || widest.y >= BEZEL_OPENING_Z;
}

// The surface's pixel at uv, from the screen's top left.
float4 crtsim_surface(float2 uv) {
    float2 ndc = float2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    // The camera looks along +x from 1/tan(fov/2) in front, with z up and -y to the right. A
    // tube wider than the picture fits across it, as the game fits its 4:3 tube across tate's
    // portrait picture.
    float2 output_size = HOST_OUTPUT_SIZE;
    float framing = max(1.0, tube_aspect() / (output_size.x / output_size.y));
    float spread = tan(radians(P(CRTSIM_FOV)) * 0.5);
    float3 camera = float3(-1.0 / spread, 0.0, 0.0);
    spread *= framing;
    // The ray where the glass is the original's sphere, across `stretched` times narrower;
    // along any ray, the bezel steps by depth alone, so it takes the ray stretched back.
    float s = stretched();
    float3 traced = normalize(float3(1.0, -ndc.x * spread * output_size.x / output_size.y / s,
        ndc.y * spread));
    float3 ray = float3(traced.x, traced.y * s, traced.z);
    float3 light_at = float3(P(CRTSIM_LIGHT_X), P(CRTSIM_LIGHT_Y), P(CRTSIM_LIGHT_Z));

    float3 to = camera - GLASS_CENTRE;
    float b = dot(to, traced);
    float reach = b * b - dot(to, to) + GLASS_RADIUS * GLASS_RADIUS;
    float3 sphere_at = camera + traced * (-b - sqrt(max(reach, 0.0)));
    if (P(CRTSIM_FACETED_GLASS) > 0.5) {
        // On to the mesh's flat triangle there, as far behind as the facet lies: one step of
        // Newton's along the ray, which lands on it within a ten-thousandth of a signal pixel
        // on average.
        float facet = glass_facet(float2((4.0 / 3.0 - sphere_at.y) * 0.375,
            (1.0 - sphere_at.z) * 0.5));
        float3 from_centre = sphere_at - GLASS_CENTRE;
        sphere_at += traced * (facet * from_centre.x / min(dot(traced, from_centre), -0.000001));
    }
    float3 glass_at = float3(sphere_at.x, sphere_at.y * s, sphere_at.z);
    Surface glass;
    // The picture spans the whole glass; its outline and shading keep the original's corners.
    glass.uv = float2((4.0 / 3.0 - sphere_at.y) * 0.375, (1.0 - sphere_at.z) * 0.5);
    float2 baked = glass_baked(float2((4.0 / 3.0 - original_y(glass_at.y)) * 0.375, glass.uv.y));
    glass.normal = (sphere_at - GLASS_CENTRE) / GLASS_RADIUS * float3(1.0, 1.0 / s, 1.0);
    glass.shade = baked.x;
    glass.reflection = 0.0;
    glass.camera = camera - glass_at;
    glass.light = light_at - glass_at;
    bool outside = glass.uv.x < 0.0 || glass.uv.y < 0.0 || glass.uv.x > 1.0 || glass.uv.y > 1.0
        || baked.y < 0.0;
    float glass_depth = (reach < 0.0 || outside) ? NOTHING : glass_at.x;

    // Only rays that may meet the bezel step across it. Nothing here takes a derivative, so
    // the branch leaves the mask's derivatives below defined.
    float bezel_depth_here = NOTHING;
    if (P(CRTSIM_SCREEN_ONLY) < 0.5 && may_meet_bezel(camera, ray))
        bezel_depth_here = bezel_hit(camera, ray);
    float3 bezel_at = along(camera, ray, min(bezel_depth_here, BEZEL_FAR));
    Surface bezel;
    bezel.uv = bezel_uv(bezel_at.yz);
    if (widened() != 0.0) {
        // The point of the picture the bezel reflects, moved with it: its UV is the screen's
        // across the original's glass.
        float screen_y = 4.0 / 3.0 - bezel.uv.x * 8.0 / 3.0 + bezel_at.y - original_y(bezel_at.y);
        bezel.uv.x = (tube_aspect() - screen_y) / (2.0 * tube_aspect());
    }

    // How each surface's uv changes between pixels, taken here, where every pixel runs, so that
    // each pixel need only shade the surface it shows.
    float2 glass_across = ddx(glass.uv);
    float2 glass_down = ddy(glass.uv);
    float2 bezel_across = ddx(bezel.uv);
    float2 bezel_down = ddy(bezel.uv);
    if (bezel_depth_here < glass_depth) {
        float2 facing = bezel_facing(bezel_at.yz) * 2.0 - V2(1.0);
        bezel.normal = float3(-sqrt(max(1.0 - dot(facing, facing), 0.0)), facing.x, facing.y);
        float2 bytes = bezel_bytes(bezel_at.yz);
        bezel.shade = bytes.x;
        bezel.reflection = bytes.y;
        bezel.camera = camera - bezel_at;
        bezel.light = light_at - bezel_at;
        return light_surface(bezel, bezel_across, bezel_down, true);
    }
    if (glass_depth < NOTHING) return light_surface(glass, glass_across, glass_down, false);
    float3 backdrop = float3(P(CRTSIM_BACKDROP_R), P(CRTSIM_BACKDROP_G), P(CRTSIM_BACKDROP_B));
    return float4(HOST_LINEAR ? srgb_decode(backdrop) : backdrop, 1.0);
}
void main() { frag = crtsim_surface(uv); }
