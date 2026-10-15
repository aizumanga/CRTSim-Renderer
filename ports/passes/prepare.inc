// CC0. The colour table (a LUT or the NES palette) and the YIQ grade, applied to the picture at
// the signal's size, in whole 8-bit steps as the reference's prepare step works.
//
// Reads: HOST_FETCH_PICTURE, the picture at the signal's size; HOST_FETCH_TABLE and
// HOST_TABLE_SIZE, an N×N×N table laid out as N slices side by side, red across a slice, green
// down, blue by slice, whose size is N. Its values are read at the lattice and blended here, as
// a 3D texture would.

float3 lattice(int3 at, int size) {
    return HOST_FETCH_TABLE(int2(at.x + at.z * size, at.y)).rgb;
}

float3 apply_table(float3 k) {
    int size = int(HOST_TABLE_SIZE);
    int last = size - 1;
    float3 xyz = clamp(k / 255.0, V3(0.0), V3(1.0)) * float(last);
    int3 lo = int3(floor(xyz));
    int3 hi = min(lo + int3(1, 1, 1), int3(last, last, last));
    float3 f = xyz - float3(lo);
    // Nearest in red and green, as Super Win the Game reads its palette's table, blends only
    // blue.
    if (P(CRTSIM_TABLE_NEAREST) > 0.5) {
        lo.xy = int2(floor(xyz.xy + V2(0.5)));
        hi.xy = lo.xy;
        f.xy = V2(0.0);
    }
    float3 sum = V3(0.0);
    for (int z = 0; z < 2; z++) {
        for (int y = 0; y < 2; y++) {
            for (int x = 0; x < 2; x++) {
                int3 at = int3(x == 1 ? hi.x : lo.x, y == 1 ? hi.y : lo.y, z == 1 ? hi.z : lo.z);
                float w = (x == 1 ? f.x : 1.0 - f.x) * (y == 1 ? f.y : 1.0 - f.y)
                    * (z == 1 ? f.z : 1.0 - f.z);
                sum += lattice(at, size) * w;
            }
        }
    }
    float s = P(CRTSIM_LUT_STRENGTH);
    return quantize((sum * s + (k / 255.0) * (1.0 - s)) * 255.0);
}

float3 apply_grade(float3 k) {
    float3 c = k / 255.0;
    float h = radians(P(CRTSIM_HUE));
    float hs = sin(h);
    float hc = cos(h);
    float y = 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
    float i = 0.596 * c.r - 0.274 * c.g - 0.322 * c.b;
    float q = 0.211 * c.r - 0.523 * c.g + 0.312 * c.b;
    float ii = (i * hc - q * hs) * P(CRTSIM_CHROMA);
    float qq = (i * hs + q * hc) * P(CRTSIM_CHROMA);
    float3 rgb = float3(y + 0.956 * ii + 0.621 * qq, y - 0.272 * ii - 0.647 * qq,
        y - 1.106 * ii + 1.703 * qq);
    return quantize(rgb * 255.0);
}

// The prepared signal's texel at xy.
float4 crtsim_prepare(int2 xy) {
    float3 k = floor(HOST_FETCH_PICTURE(xy).rgb * 255.0 + V3(0.5));
    if (P(CRTSIM_TABLE) > 0.5) k = apply_table(k);
    if (P(CRTSIM_HUE) != 0.0 || P(CRTSIM_CHROMA) != 1.0) k = apply_grade(k);
    return float4(k / 255.0, 1.0);
}
