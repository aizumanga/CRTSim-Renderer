// CC0. The host's picture resized to the signal, for hosts that hand over a picture of any size
// (RetroArch hands over the core's own): first its columns to the signal's rows, into floats in
// 8-bit steps, unrounded, then those rows to the signal's columns, rounded, as the image
// crate's resize works and the reference prepares a signal. Nearest takes the one pixel under
// each signal pixel's centre; Lanczos weighs those within three of the signal's pixels.
//
// Reads: HOST_FETCH_ORIGINAL and HOST_ORIGINAL_SIZE, the host's picture; HOST_FETCH_ROWS, the
// first pass's output; HOST_SIGNAL_SIZE; HOST_RESIZE_NEAREST.

#define PI 3.14159265

float sinc(float t) {
    float a = t * PI;
    return t == 0.0 ? 1.0 : sin(a) / a;
}

float lanczos3(float x) { return abs(x) < 3.0 ? sinc(x) * sinc(x / 3.0) : 0.0; }

// Where signal pixel `at` of `count` reads from a line of `extent` pixels: the first pixel and
// one past the last, the centre the weights are measured from, and the scale of their spacing.
float4 span(int at, float extent, float count) {
    float ratio = extent / count;
    float sratio = max(ratio, 1.0);
    float support = HOST_RESIZE_NEAREST ? 0.0 : 3.0 * sratio;
    float centre = (float(at) + 0.5) * ratio;
    float left = clamp(floor(centre - support), 0.0, extent - 1.0);
    float right = clamp(ceil(centre + support), left + 1.0, extent);
    return float4(left, right, centre - 0.5, sratio);
}

float resize_weight(float i, float4 s) {
    return HOST_RESIZE_NEAREST ? 1.0 : lanczos3((i - s.z) / s.w);
}

float3 steps(float4 c) { return floor(c.rgb * 255.0 + V3(0.5)); }

// The first pass's texel at xy: the picture's column xy.x at signal row xy.y.
float4 crtsim_resize_rows(int2 xy) {
    float4 s = span(xy.y, HOST_ORIGINAL_SIZE.y, HOST_SIGNAL_SIZE.y);
    float3 total = V3(0.0);
    float sum = 0.0;
    for (int i = int(s.x); i < int(s.y); i++) {
        float w = resize_weight(float(i), s);
        total += steps(HOST_FETCH_ORIGINAL(int2(xy.x, i))) * w;
        sum += w;
    }
    return float4(total / sum, 255.0);
}

// The picture at the signal's size: its texel at xy.
float4 crtsim_resize_columns(int2 xy) {
    float4 s = span(xy.x, HOST_ORIGINAL_SIZE.x, HOST_SIGNAL_SIZE.x);
    float3 total = V3(0.0);
    float sum = 0.0;
    for (int i = int(s.x); i < int(s.y); i++) {
        float w = resize_weight(float(i), s);
        total += HOST_FETCH_ROWS(int2(i, xy.y)).rgb * w;
        sum += w;
    }
    return float4(quantize(total / sum) / 255.0, 1.0);
}
