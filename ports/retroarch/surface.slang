#version 450
// CC0. The curved glass and its bezel: for each pixel, where a ray from the camera meets the
// screen, a cap of a sphere of radius 4 centred 4 units behind its middle within the outline its
// mesh baked, or the bezel, baked from its mesh seen straight on, whichever it meets first, and
// how that point is lit. The reference renderer draws them with this same pass.

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
#include "common.inc"
layout(location = 0) in vec2 vTexCoord;
layout(location = 0) out vec4 FragColor;
layout(set = 0, binding = 2) uniform sampler2D Source;
layout(set = 0, binding = 3) uniform sampler2D CRTSIM_SHADOW_MASK;
// The bezel's depth, shade and reflection; its UV; its normal. 16-bit values are split over two
// channels, high byte first. The first is also sampled filtered, for its depth's high byte.
layout(set = 0, binding = 4) uniform sampler2D CRTSIM_BEZEL_SHAPE;
layout(set = 0, binding = 5) uniform sampler2D CRTSIM_BEZEL_UV;
layout(set = 0, binding = 6) uniform sampler2D CRTSIM_BEZEL_NORMAL;
// The glass's shade and the distance to its outline, baked from its mesh across its UVs, the
// distance positive inside and within ±GLASS_EDGE. Texel (i, j) is UV (i, j) / (size - 1).
layout(set = 0, binding = 7) uniform sampler2D CRTSIM_GLASS;

const vec3 GLASS_CENTRE = vec3(4.0, 0.0, 0.0);
const float GLASS_RADIUS = 4.0;
const float GLASS_EDGE = 0.015625;
const vec2 BEZEL_HALF = vec2(1.6533334, 1.32);
const float BEZEL_NEAR = -0.1;
const float BEZEL_FAR = 0.33;
const vec2 BEZEL_OPENING = vec2(1.24, 0.95);
// Steps across the bezel's depth, then halvings of the step a ray first passes the surface in.
const int BEZEL_STEPS = 32;
const int BEZEL_HALVINGS = 7;
const float NOTHING = 1e9;

bool linear_light() { return global.CRTSIM_LINEAR > 0.5; }

// The picture's shape as the core means it to be shown, else as the preset says.
float picture_aspect() {
#ifdef _HAS_ORIGINALASPECT_UNIFORMS
    if (global.OriginalAspect > 0.0) return global.OriginalAspect;
#endif
    return global.CRTSIM_ASPECT;
}

// How the picture sits on the 4:3 tube: as the original placed it, fitted inside, filling it,
// or stretched over it.
vec2 uv_scale() {
    float ratio = picture_aspect() / (4.0 / 3.0);
    int fit = int(global.CRTSIM_FIT + 0.5);
    if (fit == 3) return vec2(1.0);
    if (fit == 1) return ratio >= 1.0 ? vec2(1.0, ratio) : vec2(1.0 / ratio, 1.0);
    if (fit == 2) return ratio >= 1.0 ? vec2(1.0 / ratio, 1.0) : vec2(1.0, ratio);
    return vec2(1.0, ratio);
}

// The mask's columns and rows across the screen: set by the preset, or one column for every two
// of the picture's and a row for each of its rows.
vec2 mask_repeats() {
    if (global.CRTSIM_MASK_COLUMNS > 0.0 && global.CRTSIM_MASK_ROWS > 0.0)
        return vec2(global.CRTSIM_MASK_COLUMNS, global.CRTSIM_MASK_ROWS);
    return vec2(global.SourceSize.x / 2.0, global.SourceSize.y);
}

// The signal with D3D9's black border, filtered bilinearly across it as the original was.
vec3 border_load(ivec2 xy) {
    ivec2 size = ivec2(global.SourceSize.xy);
    if (any(lessThan(xy, ivec2(0))) || any(greaterThanEqual(xy, size))) return vec3(0.0);
    vec3 c = texelFetch(Source, xy, 0).rgb;
    return linear_light() ? srgb_decode(c) : c;
}

vec3 black_border(vec2 uv) {
    vec2 pixel = uv * global.SourceSize.xy - vec2(0.5);
    ivec2 xy = ivec2(floor(pixel));
    vec2 f = fract(pixel);
    return mix(mix(border_load(xy), border_load(xy + ivec2(1, 0)), f.x),
        mix(border_load(xy + ivec2(0, 1)), border_load(xy + ivec2(1, 1)), f.x), f.y);
}

// The signal at `uv` through the mask, on the glass or reflected in the bezel; `across` and
// `down` are how uv changes from one pixel to the next, which say how small the mask is drawn.
vec3 crt(vec2 uv, vec2 across, vec2 down, bool bezel) {
    vec2 scaled = (uv - vec2(0.5)) * uv_scale() + vec2(0.5);
    // The public source reflects half the mask's rows in the bezel, and the overscan
    // unreciprocated; Super Win the Game reflects them as the screen shows them.
    bool as_screen = !bezel || global.CRTSIM_REFLECTION_AS_SCREEN > 0.5;
    vec2 density = mask_repeats() * (as_screen ? vec2(1.0) : vec2(1.0, 0.5));
    vec2 mask_uv = scaled * density;
    vec2 scale = uv_scale() * density * vec2(textureSize(CRTSIM_SHADOW_MASK, 0));
    float footprint = max(length(across * scale), length(down * scale));
    float lod = global.CRTSIM_MASK_ANTIALIAS > 0.5 ? max(log2(max(footprint, 0.000001)), 0.0) : 0.0;
    vec3 grid = textureLod(CRTSIM_SHADOW_MASK, mask_uv, lod).rgb;
    grid = mix(vec3(1.0), grid + vec3(global.CRTSIM_MASK_BRIGHTNESS), global.CRTSIM_MASK_OPACITY);
    float over = as_screen ? 1.0 / global.CRTSIM_OVERSCAN : global.CRTSIM_OVERSCAN;
    vec2 pos = (scaled - vec2(0.5)) * over;
    pos = pos + pos * global.CRTSIM_BARREL * dot(pos, pos) + vec2(0.5);
    vec3 emissive = black_border(pos) * grid;
    return mix(vec3(display_luma(emissive, linear_light())), emissive, global.CRTSIM_SATURATION);
}

vec3 safe_normalize(vec3 v) { return dot(v, v) < 0.000000000001 ? vec3(0.0) : normalize(v); }

// A point of the glass or the bezel, with what shading it needs.
struct Surface {
    vec3 normal;
    float shade;
    vec2 uv;
    float reflection;
    vec3 camera;
    vec3 light;
};

vec4 lit(Surface v, vec2 across, vec2 down, bool bezel) {
    vec3 n = safe_normalize(v.normal);
    vec3 cam = safe_normalize(v.camera);
    vec3 light = safe_normalize(v.light);
    float diffuse = max(dot(n, light), 0.0);
    vec3 halfvec = safe_normalize(light + cam);
    float spec = pow(max(dot(n, halfvec), 0.0), global.CRTSIM_SPECULAR_POWER);
    float fres = pow(max(1.0 - dot(cam, n), 0.0), 2.0) * global.CRTSIM_RIM;
    vec3 color = crt(v.uv, across, down, bezel);
    if (bezel) {
        float hemi = (dot(n, vec3(0.0, 0.0, 1.0)) * 0.5 + 0.5) * 0.4 + 0.3;
        vec3 body = vec3(global.CRTSIM_BEZEL_R, global.CRTSIM_BEZEL_G, global.CRTSIM_BEZEL_B);
        if (linear_light()) body = srgb_decode(body);
        color = body * (diffuse + hemi) * global.CRTSIM_DIFFUSE + vec3(0.25) * spec * global.CRTSIM_SPECULAR
            + color * v.reflection * global.CRTSIM_REFLECTION + vec3(0.15) * fres;
    } else {
        color += vec3(0.175, 0.15, 0.2) * diffuse * global.CRTSIM_DIFFUSE
            + vec3(0.25) * spec * global.CRTSIM_SPECULAR + vec3(0.45, 0.4, 0.5) * fres;
    }
    return vec4(color * mix(1.0, v.shade, global.CRTSIM_DIMMING), 1.0);
}

float pair(float hi, float lo) { return (round(hi * 255.0) * 256.0 + round(lo * 255.0)) / 65534.0; }

// Where (y, z) falls on the bezel's grid, in texels.
vec2 bezel_texel(vec2 yz) {
    vec2 size = vec2(textureSize(CRTSIM_BEZEL_SHAPE, 0));
    return (BEZEL_HALF - yz) / (2.0 * BEZEL_HALF) * size - vec2(0.5);
}

vec4 bezel_load(sampler2D t, ivec2 xy) {
    return texelFetch(t, clamp(xy, ivec2(0), textureSize(t, 0) - ivec2(1)), 0);
}

float bezel_depth_at(ivec2 xy) {
    ivec2 size = textureSize(CRTSIM_BEZEL_SHAPE, 0);
    vec4 texel = bezel_load(CRTSIM_BEZEL_SHAPE, xy);
    bool empty = any(lessThan(xy, ivec2(0))) || any(greaterThanEqual(xy, size))
        || (texel.r > 0.999 && texel.g > 0.999);
    return empty ? NOTHING : mix(BEZEL_NEAR, BEZEL_FAR, pair(texel.r, texel.g));
}

// The bezel's depth at (y, z), blended between texels; nothing where any of them is empty.
float bezel_depth(vec2 yz) {
    vec2 t = bezel_texel(yz);
    ivec2 xy = ivec2(floor(t));
    vec2 f = fract(t);
    float a = bezel_depth_at(xy), b = bezel_depth_at(xy + ivec2(1, 0));
    float c = bezel_depth_at(xy + ivec2(0, 1)), d = bezel_depth_at(xy + ivec2(1, 1));
    if (max(max(a, b), max(c, d)) >= NOTHING) return NOTHING;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// A two-value image's texels decoded, blended between texels at texel position g.
vec2 pairs(sampler2D t, vec2 g) {
    ivec2 xy = ivec2(floor(g));
    vec2 f = fract(g);
    vec4 k[4] = vec4[4](bezel_load(t, xy), bezel_load(t, xy + ivec2(1, 0)),
        bezel_load(t, xy + ivec2(0, 1)), bezel_load(t, xy + ivec2(1, 1)));
    vec2 v[4];
    for (int i = 0; i < 4; i++) v[i] = vec2(pair(k[i].r, k[i].g), pair(k[i].b, k[i].a));
    return mix(mix(v[0], v[1], f.x), mix(v[2], v[3], f.x), f.y);
}

vec2 bezel_pairs(sampler2D t, vec2 yz) { return pairs(t, bezel_texel(yz)); }

// The glass's shade and distance to its outline at uv.
vec2 glass_baked(vec2 uv) {
    vec2 last = vec2(textureSize(CRTSIM_GLASS, 0)) - vec2(1.0);
    vec2 v = pairs(CRTSIM_GLASS, clamp(uv, vec2(0.0), vec2(1.0)) * last);
    return vec2(v.x, mix(-GLASS_EDGE, GLASS_EDGE, v.y));
}

vec2 bezel_bytes(vec2 yz) {
    vec2 g = bezel_texel(yz);
    ivec2 xy = ivec2(floor(g));
    vec2 f = fract(g);
    vec2 a = bezel_load(CRTSIM_BEZEL_SHAPE, xy).ba, b = bezel_load(CRTSIM_BEZEL_SHAPE, xy + ivec2(1, 0)).ba;
    vec2 c = bezel_load(CRTSIM_BEZEL_SHAPE, xy + ivec2(0, 1)).ba;
    vec2 d = bezel_load(CRTSIM_BEZEL_SHAPE, xy + ivec2(1, 1)).ba;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// The point a ray from `camera` along `ray` meets at depth x.
vec3 along(vec3 camera, vec3 ray, float x) { return camera + ray * ((x - camera.x) / ray.x); }

// Where the ray first meets the bezel, NOTHING when it does not: stepping across its depths on
// the depth's high byte, filtered by the sampler, then halving the step it passed the surface
// in on the full depth. It takes no derivatives, so it may stop at the first step inside.
float bezel_hit(vec3 camera, vec3 ray) {
    float step = (BEZEL_FAR - BEZEL_NEAR) / float(BEZEL_STEPS);
    vec2 size = vec2(textureSize(CRTSIM_BEZEL_SHAPE, 0));
    float before = BEZEL_NEAR - step, after = NOTHING;
    for (int i = 0; i <= BEZEL_STEPS; i++) {
        float x = BEZEL_NEAR + step * float(i);
        vec2 at = (bezel_texel(along(camera, ray, x).yz) + vec2(0.5)) / size;
        float coarse = mix(BEZEL_NEAR, BEZEL_FAR,
            textureLod(CRTSIM_BEZEL_SHAPE, at, 0.0).r * 255.0 * 256.0 / 65534.0);
        if (x >= coarse && all(greaterThanEqual(at, vec2(0.0))) && all(lessThanEqual(at, vec2(1.0)))) {
            after = x;
            break;
        }
        before = x;
    }
    float low = before, high = after;
    for (int i = 0; i < BEZEL_HALVINGS; i++) {
        float middle = 0.5 * (low + high);
        bool inside = middle >= bezel_depth(along(camera, ray, middle).yz);
        high = inside ? middle : high;
        low = inside ? low : middle;
    }
    bool found = after < NOTHING && bezel_depth(along(camera, ray, high).yz) < NOTHING;
    return found ? high : NOTHING;
}

// Whether a ray can meet the bezel at all: not if it stays within the opening's clear box
// across the bezel's whole depth.
bool may_meet_bezel(vec3 camera, vec3 ray) {
    vec2 near = abs(along(camera, ray, BEZEL_NEAR).yz), far = abs(along(camera, ray, BEZEL_FAR).yz);
    return any(greaterThanEqual(max(near, far), BEZEL_OPENING));
}

void main() {
    vec2 ndc = vec2(vTexCoord.x * 2.0 - 1.0, 1.0 - vTexCoord.y * 2.0);
    // The camera looks along +x from 1/tan(fov/2) in front, with z up and -y to the right.
    float spread = tan(radians(global.CRTSIM_FOV) * 0.5);
    vec3 camera = vec3(-1.0 / spread, 0.0, 0.0);
    vec3 ray = normalize(vec3(1.0, -ndc.x * spread * global.OutputSize.x / global.OutputSize.y,
        ndc.y * spread));
    vec3 light_at = vec3(global.CRTSIM_LIGHT_X, global.CRTSIM_LIGHT_Y, global.CRTSIM_LIGHT_Z);

    vec3 to = camera - GLASS_CENTRE;
    float b = dot(to, ray);
    float reach = b * b - dot(to, to) + GLASS_RADIUS * GLASS_RADIUS;
    vec3 glass_at = camera + ray * (-b - sqrt(max(reach, 0.0)));
    Surface glass;
    glass.uv = vec2((4.0 / 3.0 - glass_at.y) * 0.375, (1.0 - glass_at.z) * 0.5);
    vec2 baked = glass_baked(glass.uv);
    glass.normal = (glass_at - GLASS_CENTRE) / GLASS_RADIUS;
    glass.shade = baked.x;
    glass.reflection = 0.0;
    glass.camera = camera - glass_at;
    glass.light = light_at - glass_at;
    bool outside = any(lessThan(glass.uv, vec2(0.0))) || any(greaterThan(glass.uv, vec2(1.0)))
        || baked.y < 0.0;
    float glass_depth = (reach < 0.0 || outside) ? NOTHING : glass_at.x;

    // Only rays that may meet the bezel step across it. Nothing here takes a derivative, so
    // the branch leaves the mask's derivatives below defined.
    float bezel_depth = NOTHING;
    if (global.CRTSIM_SCREEN_ONLY < 0.5 && may_meet_bezel(camera, ray))
        bezel_depth = bezel_hit(camera, ray);
    vec3 bezel_at = along(camera, ray, min(bezel_depth, BEZEL_FAR));
    Surface bezel;
    bezel.uv = bezel_pairs(CRTSIM_BEZEL_UV, bezel_at.yz);

    // How each surface's uv changes between pixels, taken here, where every pixel runs, so that
    // each pixel need only shade the surface it shows.
    vec2 glass_across = dFdx(glass.uv), glass_down = dFdy(glass.uv);
    vec2 bezel_across = dFdx(bezel.uv), bezel_down = dFdy(bezel.uv);
    if (bezel_depth < glass_depth) {
        vec2 facing = bezel_pairs(CRTSIM_BEZEL_NORMAL, bezel_at.yz) * 2.0 - vec2(1.0);
        bezel.normal = vec3(-sqrt(max(1.0 - dot(facing, facing), 0.0)), facing);
        vec2 bytes = bezel_bytes(bezel_at.yz);
        bezel.shade = bytes.x;
        bezel.reflection = bytes.y;
        bezel.camera = camera - bezel_at;
        bezel.light = light_at - bezel_at;
        FragColor = lit(bezel, bezel_across, bezel_down, true);
    } else if (glass_depth < NOTHING) {
        FragColor = lit(glass, glass_across, glass_down, false);
    } else {
        vec3 backdrop = vec3(global.CRTSIM_BACKDROP_R, global.CRTSIM_BACKDROP_G, global.CRTSIM_BACKDROP_B);
        FragColor = vec4(linear_light() ? srgb_decode(backdrop) : backdrop, 1.0);
    }
}
