// CC0. The bloom's blur: seven taps in a hexagon, turned a quarter between its two passes.
//
// Reads: HOST_SAMPLE_BLOOM_INPUT(uv), the pass's input filtered; HOST_OUTPUT_SIZE.

float3 bloom_tap(float2 uv, float2 offset, bool swap, float2 scale) {
    float2 turned = swap ? offset.yx : offset;
    return HOST_SAMPLE_BLOOM_INPUT(uv + turned * scale).rgb;
}

// The blur's pixel at uv; the second pass swaps the hexagon's axes.
float4 crtsim_bloom(float2 uv, bool swap) {
    float2 output_size = HOST_OUTPUT_SIZE;
    float2 scale = float2(output_size.y / output_size.x, 1.0) * P(CRTSIM_BLOOM_SPREAD);
    float3 total = V3(0.0);
    total += bloom_tap(uv, float2(0.0, 0.0), swap, scale);
    total += bloom_tap(uv, float2(0.0, 1.0), swap, scale);
    total += bloom_tap(uv, float2(0.0, -1.0), swap, scale);
    total += bloom_tap(uv, float2(-0.866025, 0.5), swap, scale);
    total += bloom_tap(uv, float2(-0.866025, -0.5), swap, scale);
    total += bloom_tap(uv, float2(0.866025, 0.5), swap, scale);
    total += bloom_tap(uv, float2(0.866025, -0.5), swap, scale);
    return float4(total / 7.0, 1.0);
}
