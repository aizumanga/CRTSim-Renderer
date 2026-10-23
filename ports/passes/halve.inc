// CC0. A mip level of the surface from the one above, for a host that cannot make a target's
// mipmaps itself, as GL's glGenerateMipmap draws one: filtered at each texel's centre, the
// average of the 2x2 texels above it, or blended between them where the level above has an
// odd size. The renderer makes the same levels this way.
//
// Reads: HOST_SAMPLE_LEVEL_ABOVE(uv), the level above, filtered.

float4 crtsim_halve(float2 uv) {
    return float4(HOST_SAMPLE_LEVEL_ABOVE(uv).rgb, 1.0);
}
