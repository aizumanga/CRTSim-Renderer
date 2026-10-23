// CC0. The bloom's blur: seven taps in a hexagon, turned a quarter between its two passes.
//
// Reads: HOST_SAMPLE_BLOOM_INPUT(uv, second, lod), the pass's input filtered: for the first pass
// the surface at mip level lod, for the second the first's output; HOST_OUTPUT_SIZE, the
// surface's size.

// The surface's mip level the first pass reads, with the bloom from mipmaps: the one GL picks
// for a target a sixteenth of it each way, rounded down, as Super Win the Game reads it: level 4,
// and towards 5 where a side does not divide by 16. The public source reads the surface itself.
float bloom_lod() {
    float2 size = HOST_OUTPUT_SIZE;
    float2 ratio = size / max(floor(size / 16.0), V2(1.0));
    return P(CRTSIM_BLOOM_MIPMAPS) > 0.5 ? log2(max(max(ratio.x, ratio.y), 1.0)) : 0.0;
}

float3 bloom_tap(float2 uv, float2 offset, bool swap, float2 scale, float lod) {
    float2 turned = swap ? offset.yx : offset;
    return HOST_SAMPLE_BLOOM_INPUT(uv + turned * scale, swap, lod).rgb;
}

// The blur's pixel at uv; the second pass swaps the hexagon's axes.
float4 crtsim_bloom(float2 uv, bool swap) {
    float2 output_size = HOST_OUTPUT_SIZE;
    float2 scale = float2(output_size.y / output_size.x, 1.0) * P(CRTSIM_BLOOM_SPREAD);
    float lod = swap ? 0.0 : bloom_lod();
    float3 total = V3(0.0);
    total += bloom_tap(uv, float2(0.0, 0.0), swap, scale, lod);
    total += bloom_tap(uv, float2(0.0, 1.0), swap, scale, lod);
    total += bloom_tap(uv, float2(0.0, -1.0), swap, scale, lod);
    total += bloom_tap(uv, float2(-0.866025, 0.5), swap, scale, lod);
    total += bloom_tap(uv, float2(-0.866025, -0.5), swap, scale, lod);
    total += bloom_tap(uv, float2(0.866025, 0.5), swap, scale, lod);
    total += bloom_tap(uv, float2(0.866025, -0.5), swap, scale, lod);
    return float4(total / 7.0, 1.0);
}
