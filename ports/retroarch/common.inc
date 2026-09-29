// CC0. Shared by the passes of the CRTSim Renderer port for RetroArch: the same arithmetic as
// shaders/crtsim.wgsl and shaders/prepare.wgsl in https://github.com/aizumanga/CRTSim-Renderer,
// which is the reference this port is tested against. Port of J. Kyle Pittman's CRTSim.

float luma(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }

vec3 srgb_decode(vec3 c) {
    return mix(pow((max(c, vec3(0.0)) + vec3(0.055)) / 1.055, vec3(2.4)), c / 12.92,
        vec3(lessThanEqual(c, vec3(0.04045))));
}

vec3 srgb_encode(vec3 c) {
    return mix(1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - vec3(0.055), c * 12.92,
        vec3(lessThanEqual(c, vec3(0.0031308))));
}

// Luma as the display sees it: Rec. 709 weights on linear light, NTSC's on gamma-encoded.
float display_luma(vec3 c, bool linear) {
    return linear ? dot(c, vec3(0.2126, 0.7152, 0.0722)) : luma(c);
}
