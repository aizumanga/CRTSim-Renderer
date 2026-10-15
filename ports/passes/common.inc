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
