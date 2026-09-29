#version 450
// CC0. The curved glass: for each pixel, where a ray from the camera meets the screen, a cap of
// a sphere of radius 4 centred 4 units behind its middle, and how that point is lit. The
// reference renderer draws the glass with this same pass.

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

const vec3 GLASS_CENTRE = vec3(4.0, 0.0, 0.0);
const float GLASS_RADIUS = 4.0;
// The outline is a superellipse in UV space; its edge shading follows the same curves inward.
const float GLASS_CORNER = 11.54;

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

vec3 crt(vec2 uv) {
    vec2 scaled = (uv - vec2(0.5)) * uv_scale() + vec2(0.5);
    vec2 mask_uv = scaled * mask_repeats();
    vec2 texels = vec2(textureSize(CRTSIM_SHADOW_MASK, 0));
    float footprint = max(length(dFdx(mask_uv) * texels), length(dFdy(mask_uv) * texels));
    float lod = global.CRTSIM_MASK_ANTIALIAS > 0.5 ? max(log2(max(footprint, 0.000001)), 0.0) : 0.0;
    vec3 grid = textureLod(CRTSIM_SHADOW_MASK, mask_uv, lod).rgb;
    grid = mix(vec3(1.0), grid + vec3(global.CRTSIM_MASK_BRIGHTNESS), global.CRTSIM_MASK_OPACITY);
    vec2 pos = (scaled - vec2(0.5)) / global.CRTSIM_OVERSCAN;
    pos = pos + pos * global.CRTSIM_BARREL * dot(pos, pos) + vec2(0.5);
    vec3 emissive = black_border(pos) * grid;
    return mix(vec3(display_luma(emissive, linear_light())), emissive, global.CRTSIM_SATURATION);
}

vec3 safe_normalize(vec3 v) { return dot(v, v) < 0.000000000001 ? vec3(0.0) : normalize(v); }

void main() {
    vec2 ndc = vec2(vTexCoord.x * 2.0 - 1.0, 1.0 - vTexCoord.y * 2.0);
    // The camera looks along +x from 1/tan(fov/2) in front, with z up and -y to the right.
    float spread = tan(radians(global.CRTSIM_FOV) * 0.5);
    vec3 camera = vec3(-1.0 / spread, 0.0, 0.0);
    vec3 ray = normalize(vec3(1.0, -ndc.x * spread * global.OutputSize.x / global.OutputSize.y,
        ndc.y * spread));
    vec3 to = camera - GLASS_CENTRE;
    float b = dot(to, ray);
    float reach = b * b - dot(to, to) + GLASS_RADIUS * GLASS_RADIUS;
    vec3 hit = camera + ray * (-b - sqrt(max(reach, 0.0)));
    vec2 uv = vec2((4.0 / 3.0 - hit.y) * 0.375, (1.0 - hit.z) * 0.5);
    vec2 edge = abs(uv * 2.0 - vec2(1.0));
    float radius = pow(pow(edge.x, GLASS_CORNER) + pow(edge.y, GLASS_CORNER), 1.0 / GLASS_CORNER);
    float vignette = 1.0 - 0.5 * pow(radius, 6.0);

    vec3 n = safe_normalize((hit - GLASS_CENTRE) / GLASS_RADIUS);
    vec3 cam = safe_normalize(camera - hit);
    vec3 light = safe_normalize(vec3(global.CRTSIM_LIGHT_X, global.CRTSIM_LIGHT_Y,
        global.CRTSIM_LIGHT_Z) - hit);
    float diffuse = max(dot(n, light), 0.0);
    vec3 halfvec = safe_normalize(light + cam);
    float spec = pow(max(dot(n, halfvec), 0.0), global.CRTSIM_SPECULAR_POWER);
    float fres = pow(max(1.0 - dot(cam, n), 0.0), 2.0) * global.CRTSIM_RIM;
    // Everything above runs for every pixel, so the mask's derivatives stay defined.
    vec3 color = crt(uv) + vec3(0.175, 0.15, 0.2) * diffuse * global.CRTSIM_DIFFUSE
        + vec3(0.25) * spec * global.CRTSIM_SPECULAR + vec3(0.45, 0.4, 0.5) * fres;
    color *= mix(1.0, vignette, global.CRTSIM_DIMMING);
    bool glass = reach >= 0.0 && radius <= 1.0;
    FragColor = vec4(glass ? color : vec3(0.0), 1.0);
}
