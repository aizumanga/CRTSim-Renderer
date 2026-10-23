// CC0. The glass and bezel, smoothed by FXAA when asked, with their bloom added, encoded for the
// display and brightened.
//
// Reads: HOST_SAMPLE_SURFACE(uv), the surface, filtered; HOST_SAMPLE_BLOOM(uv), its blur;
// HOST_OUTPUT_SIZE, the surface's size; HOST_LINEAR.

float3 surface_at(float2 uv) { return HOST_SAMPLE_SURFACE(uv).rgb; }

// FXAA's luma, from red and green as the game's FXAA weighs them, in gamma space.
float fxaa_luma(float3 c) {
    float3 g = HOST_LINEAR ? srgb_encode(c) : c;
    return g.g * (0.587 / 0.299) + g.r;
}

// Super Win the Game's FXAA: Timothy Lottes' FXAA 3 quality, as the game sets its four levels.
// A pixel on an edge, found by the luma range across its neighbours, is filtered along the edge
// by how far it lies from the edge's two ends, found by stepping along it, and blended towards
// its neighbours' average where the edge is finer than a pixel.
float3 fxaa(float2 uv, int level) {
    float2 px = V2(1.0) / HOST_OUTPUT_SIZE;
    float edge = 0.25;
    float least = 1.0 / 12.0;
    int steps = 4;
    if (level == 2) { edge = 1.0 / 6.0; least = 1.0 / 14.0; steps = 8; }
    if (level == 3) { edge = 0.125; least = 1.0 / 16.0; steps = 16; }
    if (level == 4) { edge = 1.0 / 16.0; least = 1.0 / 32.0; steps = 32; }
    float3 centre = surface_at(uv);
    float m = fxaa_luma(centre);
    float n = fxaa_luma(surface_at(uv + float2(0.0, -px.y)));
    float w = fxaa_luma(surface_at(uv + float2(-px.x, 0.0)));
    float e = fxaa_luma(surface_at(uv + float2(px.x, 0.0)));
    float s = fxaa_luma(surface_at(uv + float2(0.0, px.y)));
    float lowest = min(m, min(min(n, w), min(s, e)));
    float highest = max(m, max(max(n, w), max(s, e)));
    float range = highest - lowest;
    if (range < max(least, highest * edge)) return centre;
    float nw = fxaa_luma(surface_at(uv + float2(-px.x, -px.y)));
    float ne = fxaa_luma(surface_at(uv + float2(px.x, -px.y)));
    float sw = fxaa_luma(surface_at(uv + float2(-px.x, px.y)));
    float se = fxaa_luma(surface_at(uv + float2(px.x, px.y)));
    float average = (n + w + e + s) * 0.25;
    float subpixel = min(0.75, clamp(abs(average - m) / range - 0.25, 0.0, 1.0) / 0.75);
    float across = abs(nw - 2.0 * n + ne) + 2.0 * abs(w - 2.0 * m + e) + abs(sw - 2.0 * s + se);
    float down = abs(nw - 2.0 * w + sw) + 2.0 * abs(n - 2.0 * m + s) + abs(ne - 2.0 * e + se);
    // Along a horizontal edge the step is up or down; along a vertical one, left or right.
    bool horizontal = down >= across;
    float side = horizontal ? -px.y : -px.x;
    if (!horizontal) {
        n = w;
        s = e;
    }
    float gradient = abs(n - m);
    float pair = (n + m) * 0.5;
    if (gradient < abs(s - m)) {
        gradient = abs(s - m);
        pair = (s + m) * 0.5;
        side = -side;
    }
    float threshold = gradient * 0.25;
    float2 along = horizontal ? float2(px.x, 0.0) : float2(0.0, px.y);
    float2 start = uv + (horizontal ? float2(0.0, side * 0.5) : float2(side * 0.5, 0.0));
    float2 to_n = start - along;
    float2 to_p = start + along;
    float end_n = 0.0;
    float end_p = 0.0;
    bool done_n = false;
    bool done_p = false;
    for (int i = 0; i < 32; i++) {
        if (i >= steps) break;
        if (!done_n) {
            end_n = fxaa_luma(surface_at(to_n));
            done_n = abs(end_n - pair) >= threshold;
        }
        if (!done_n) to_n -= along;
        if (!done_p) {
            end_p = fxaa_luma(surface_at(to_p));
            done_p = abs(end_p - pair) >= threshold;
        }
        if (!done_p) to_p += along;
    }
    float far_n = horizontal ? uv.x - to_n.x : uv.y - to_n.y;
    float far_p = horizontal ? to_p.x - uv.x : to_p.y - uv.y;
    bool nearer_n = far_n < far_p;
    float end = nearer_n ? end_n : end_p;
    // Only the end whose luma lies across the pair from this pixel's moves it.
    if ((m - pair < 0.0) == (end - pair < 0.0)) side = 0.0;
    float offset = (0.5 - (nearer_n ? far_n : far_p) / (far_p + far_n) + subpixel * 0.125) * side;
    float3 filtered = surface_at(uv + (horizontal ? float2(0.0, offset) : float2(offset, 0.0)));
    float filtered_luma = fxaa_luma(filtered) + 1.0 / (65536.0 * 256.0);
    return filtered * min(4.0, lerp(filtered_luma, average, subpixel) / filtered_luma);
}

// A colour with its luma raised to `power`, its hue kept, as the game's ColorPow does; black
// stays black.
float3 color_pow(float3 c, float power) {
    float lum = luma(c);
    return c / max(lum, 0.000001) * pow(max(lum, 0.0), power);
}

// The screen's pixel at uv.
float4 crtsim_present(float2 uv) {
    int level = int(P(CRTSIM_FXAA) + 0.5);
    float3 base = level > 0 ? fxaa(uv, level) : surface_at(uv);
    float3 blurred = HOST_SAMPLE_BLOOM(uv).rgb;
    float lum = display_luma(blurred, HOST_LINEAR);
    float3 colored = blurred / max(lum, 0.000001) * pow(max(lum, 0.0), P(CRTSIM_BLOOM_POWER));
    float3 result = base + colored * P(CRTSIM_BLOOM);
    float3 shown = HOST_LINEAR ? srgb_encode(result) : result;
    // Brightness at 0 leaves the picture exactly as it is.
    float power = exp2(-P(CRTSIM_BRIGHTNESS));
    return float4(power == 1.0 ? shown : color_pow(shown, power), 1.0);
}
