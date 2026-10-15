// CC0. The composite signal: artifacts, ringing and bleed on the picture, over the glow of the
// ticks before it, which this pass reads back from its own last output.
//
// Reads: HOST_FETCH_PREPARED, the prepared signal; HOST_FETCH_PREVIOUS, this pass's output the
// frame before, when HOST_HAS_PREVIOUS says there is one at this size (a new signal size starts
// the glow again); HOST_FETCH_ARTIFACTS and HOST_ARTIFACT_SIZE, the artifact pattern;
// HOST_SIGNAL_SIZE, HOST_FRAME_COUNT and HOST_FRAME_MICROSECONDS.

float3 prepared_at(int2 xy) {
    int2 last = int2(HOST_SIGNAL_SIZE) - int2(1, 1);
    return HOST_FETCH_PREPARED(clamp(xy, int2(0, 0), last)).rgb;
}

// The last tick's signal, or black when there is none.
float3 previous_at(int2 xy) {
    if (!(HOST_HAS_PREVIOUS)) return V3(0.0);
    int2 last = int2(HOST_SIGNAL_SIZE) - int2(1, 1);
    return HOST_FETCH_PREVIOUS(clamp(xy, int2(0, 0), last)).rgb;
}

// How far this tick shows the second artifact pattern over the first: stable phase shows their
// average; A and B one each, moved towards it by the blending as Super Win the Game's NTSC
// Blending does, so 1 is the average; alternating swaps them every tick, as an NTSC set's did.
float artifact_phase() {
    float a = 0.5 * P(CRTSIM_NTSC_BLENDING);
    float b = 1.0 - a;
    int mode = int(P(CRTSIM_PHASE) + 0.5);
    if (mode == 1) return a;
    if (mode == 2) return b;
    if (mode == 3) return (HOST_FRAME_COUNT % 2) == 0 ? a : b;
    return 0.5;
}

// The persistence settings are how much glow survives a tick at 60 a second. A frame lasting
// longer or shorter keeps that much for its own length, when the host says how long it was.
float3 persistence() {
    float3 per_tick = float3(P(CRTSIM_PERSISTENCE_R), P(CRTSIM_PERSISTENCE_G),
        P(CRTSIM_PERSISTENCE_B));
    float microseconds = HOST_FRAME_MICROSECONDS;
    if (microseconds > 0.0) return pow(per_tick, V3(60.0 * microseconds / 1000000.0));
    return per_tick;
}

// The difference between this pixel's luma and its two neighbours d pixels away.
float ring(int2 xy, int d, float brightness) {
    float lb = luma(prepared_at(xy - int2(d, 0)));
    float rb = luma(prepared_at(xy + int2(d, 0)));
    return 2.0 * brightness - lb - rb;
}

// The signal's texel at xy.
float4 crtsim_composite(int2 xy) {
    int rows = int(HOST_SIGNAL_SIZE.y);
    // Flipped, the pattern runs from the bottom up, so the second sample is the row above.
    int2 at = P(CRTSIM_FLIP_ARTIFACTS) > 0.5 ? int2(xy.x, rows - 1 - xy.y) : xy;
    float2 auv = (float2(at) + V2(0.5)) / float2(256.0, 224.0);
    float2 tile = HOST_ARTIFACT_SIZE;
    float t = artifact_phase();
    float3 a = lerp(HOST_FETCH_ARTIFACTS(int2(floor(frac(auv) * tile))).rgb,
        HOST_FETCH_ARTIFACTS(int2(floor(frac(auv + float2(0.0, 1.0 / 224.0)) * tile))).rgb,
        V3(t));
    float3 left = prepared_at(xy - int2(1, 0));
    float3 right = prepared_at(xy + int2(1, 0));
    float3 cur = prepared_at(xy);
    float3 color = clamp(cur + (left + right - 2.0 * cur) * a * P(CRTSIM_ARTIFACTS), V3(0.0),
        V3(1.0));
    float brightness = luma(color);
    float offset = 0.0;
    offset += ring(xy, 1, brightness) * 1.0;
    offset += ring(xy, 2, brightness) * (-0.3162277);
    offset += ring(xy, 3, brightness) * 0.1;
    color = clamp(color + offset * P(CRTSIM_SHARPNESS) * lerp(V3(1.0), a, V3(P(CRTSIM_ARTIFACTS))),
        V3(0.0), V3(1.0));
    // Interlaced: the beam skips the other field's rows this tick; they only decay below. Auto
    // interlaces a picture of 400 rows or more, as a set drew a 480- or 576-line one.
    int interlace = int(P(CRTSIM_INTERLACE) + 0.5);
    bool interlaced = interlace == 1 || (interlace == 2 && rows >= 400);
    if (interlaced && (xy.y % 2) != (HOST_FRAME_COUNT % 2)) color = V3(0.0);
    float3 pl = previous_at(xy - int2(1, 0));
    float3 pr = previous_at(xy + int2(1, 0));
    float3 pc = previous_at(xy);
    float bleed = P(CRTSIM_BLEED);
    color = max(color, persistence() * (pc + (pl + pr) * bleed) / (1.0 + 2.0 * bleed));
    return float4(clamp(color, V3(0.0), V3(1.0)), 1.0);
}
