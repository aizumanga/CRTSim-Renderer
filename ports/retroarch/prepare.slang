#version 450
// CC0. The colour table (a LUT or the NES palette) and the YIQ grade, applied to the core's
// picture at its own size, in whole 8-bit steps as the reference's prepare step works.

#include "parameters.inc"

layout(std140, set = 0, binding = 0) uniform UBO {
    mat4 MVP;
    float CRTSIM_TABLE;
    float CRTSIM_TABLE_NEAREST;
    float CRTSIM_LUT_STRENGTH;
    float CRTSIM_HUE;
    float CRTSIM_CHROMA;
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
// An N×N×N table laid out as N slices side by side: red across a slice, green down, blue by
// slice. Values are sampled at the lattice and blended here, as a 3D texture would.
layout(set = 0, binding = 3) uniform sampler2D CRTSIM_COLOUR_TABLE;

// Rounds half away from zero, as the reference does.
vec3 quantize(vec3 v) { return floor(clamp(v, vec3(0.0), vec3(255.0)) + vec3(0.5)); }

vec3 lattice(ivec3 at, int size) {
    return texelFetch(CRTSIM_COLOUR_TABLE, ivec2(at.x + at.z * size, at.y), 0).rgb;
}

vec3 apply_table(vec3 k) {
    int size = textureSize(CRTSIM_COLOUR_TABLE, 0).y;
    int last = size - 1;
    vec3 xyz = clamp(k / 255.0, vec3(0.0), vec3(1.0)) * float(last);
    ivec3 lo = ivec3(floor(xyz));
    ivec3 hi = min(lo + ivec3(1), ivec3(last));
    vec3 f = xyz - vec3(lo);
    // Nearest in red and green, as Super Win the Game reads its palette's table, blends only
    // blue.
    if (global.CRTSIM_TABLE_NEAREST > 0.5) {
        lo.xy = ivec2(floor(xyz.xy + vec2(0.5)));
        hi.xy = lo.xy;
        f.xy = vec2(0.0);
    }
    vec3 out_ = vec3(0.0);
    for (int z = 0; z < 2; z++) {
        for (int y = 0; y < 2; y++) {
            for (int x = 0; x < 2; x++) {
                ivec3 at = ivec3(x == 1 ? hi.x : lo.x, y == 1 ? hi.y : lo.y, z == 1 ? hi.z : lo.z);
                float w = (x == 1 ? f.x : 1.0 - f.x) * (y == 1 ? f.y : 1.0 - f.y)
                    * (z == 1 ? f.z : 1.0 - f.z);
                out_ += lattice(at, size) * w;
            }
        }
    }
    float s = global.CRTSIM_LUT_STRENGTH;
    return quantize((out_ * s + (k / 255.0) * (1.0 - s)) * 255.0);
}

vec3 apply_grade(vec3 k) {
    vec3 c = k / 255.0;
    float h = radians(global.CRTSIM_HUE);
    float hs = sin(h), hc = cos(h);
    float y = 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
    float i = 0.596 * c.r - 0.274 * c.g - 0.322 * c.b;
    float q = 0.211 * c.r - 0.523 * c.g + 0.312 * c.b;
    float ii = (i * hc - q * hs) * global.CRTSIM_CHROMA;
    float qq = (i * hs + q * hc) * global.CRTSIM_CHROMA;
    vec3 rgb = vec3(y + 0.956 * ii + 0.621 * qq, y - 0.272 * ii - 0.647 * qq,
        y - 1.106 * ii + 1.703 * qq);
    return quantize(rgb * 255.0);
}

void main() {
    ivec2 xy = ivec2(vTexCoord * vec2(textureSize(Source, 0)));
    vec3 k = floor(texelFetch(Source, xy, 0).rgb * 255.0 + vec3(0.5));
    if (global.CRTSIM_TABLE > 0.5) k = apply_table(k);
    if (global.CRTSIM_HUE != 0.0 || global.CRTSIM_CHROMA != 1.0) k = apply_grade(k);
    FragColor = vec4(k / 255.0, 1.0);
}
