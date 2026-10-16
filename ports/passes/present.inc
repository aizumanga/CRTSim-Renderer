// CC0. The glass and bezel with their bloom added, encoded for the display.
//
// Reads: HOST_SAMPLE_SURFACE(uv), the surface; HOST_SAMPLE_BLOOM(uv), its blur; HOST_LINEAR.

// The screen's pixel at uv.
float4 crtsim_present(float2 uv) {
    float3 base = HOST_SAMPLE_SURFACE(uv).rgb;
    float3 blurred = HOST_SAMPLE_BLOOM(uv).rgb;
    float lum = display_luma(blurred, HOST_LINEAR);
    float3 colored = blurred / max(lum, 0.000001) * pow(max(lum, 0.0), P(CRTSIM_BLOOM_POWER));
    float3 result = base + colored * P(CRTSIM_BLOOM);
    return float4(HOST_LINEAR ? srgb_encode(result) : result, 1.0);
}
