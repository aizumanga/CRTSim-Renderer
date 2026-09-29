// CC0. The bloom's blur: seven taps in a hexagon, turned a quarter between its two passes.

layout(std140, set = 0, binding = 0) uniform UBO {
    mat4 MVP;
    vec4 FinalViewportSize;
    float CRTSIM_BLOOM_SPREAD;
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

vec4 blur(vec2 uv, bool swap) {
    vec2 offsets[7] = vec2[7](vec2(0.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0),
        vec2(-0.866025, 0.5), vec2(-0.866025, -0.5), vec2(0.866025, 0.5), vec2(0.866025, -0.5));
    vec2 scale = vec2(global.FinalViewportSize.y / global.FinalViewportSize.x, 1.0)
        * global.CRTSIM_BLOOM_SPREAD;
    vec3 total = vec3(0.0);
    for (int i = 0; i < 7; i++) {
        vec2 off = swap ? offsets[i].yx : offsets[i];
        total += textureLod(Source, uv + off * scale, 0.0).rgb;
    }
    return vec4(total / 7.0, 1.0);
}
