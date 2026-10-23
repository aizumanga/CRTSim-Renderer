// CC0. The composite signal: artifacts, ringing and bleed on the picture, over the glow of the
// ticks before it, which this pass reads back from its own last output.
//
// Reads: HOST_FETCH_PREPARED, the prepared signal; HOST_FETCH_PREVIOUS, this pass's output the
// frame before, when HOST_HAS_PREVIOUS says there is one at this size (a new signal size starts
// the glow again); HOST_FETCH_ARTIFACTS and HOST_ARTIFACT_SIZE, the artifact pattern;
// HOST_SIGNAL_SIZE, HOST_FRAME_COUNT and HOST_FRAME_TICKS.

float3 prepared_at(int2 xy) {
    int2 last = int2(HOST_SIGNAL_SIZE) - int2(1, 1);
    return HOST_FETCH_PREPARED(clamp(xy, int2(0, 0), last)).rgb;
}

// TV static's clock, in sixtieths of a second: a port counts each frame as one.
float static_clock() { return float(HOST_FRAME_COUNT % 16777216); }

// How strong the static is this frame: its amount, or with a cycle, the amount over a channel
// change's course: static, fading to the picture, the picture, fading back.
float static_strength() {
    float cycle = P(CRTSIM_STATIC_CYCLE);
    if (cycle <= 0.0) return P(CRTSIM_STATIC);
    float u = frac(static_clock() / 60.0 / cycle);
    float fade = clamp((0.5 - u) / 0.1333333, 0.0, 1.0) + clamp((u - 0.8666667) / 0.1333333, 0.0,
        1.0);
    return P(CRTSIM_STATIC) * clamp(u < 0.3666667 ? 1.0 : fade, 0.0, 1.0);
}

// TV static, the renderer's own: white noise drawn anew each sixtieth of a second, squared,
// rolling under a darker band, over the picture wavering across and slipping down as the static
// thickens.
float static_noise(int2 xy, uint clock) {
    uint h = uint(xy.x) * 0x8da6b343u ^ uint(xy.y) * 0xd8163841u ^ clock * 0xcb1ab31fu;
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    return float(h ^ (h >> 16u)) / 4294967295.0;
}

// The picture's texel at xy as the static shows it, which is the prepared signal without any.
float3 picture_at(int2 xy, float strength) {
    if (strength <= 0.0) return prepared_at(xy);
    float2 size = HOST_SIGNAL_SIZE;
    float2 uv = (float2(xy) + V2(0.5)) / size;
    float t = static_clock() / 60.0;
    float row = uv.y;
    float waver = (sin(6.2831853 * (row * 2.3 + t * 1.7)) + 0.5 * sin(6.2831853 * (row * 5.9
        - t * 3.1)) + 0.25 * sin(6.2831853 * (row * 13.7 + t * 7.3))) * 0.012 * strength;
    float slip = strength * strength * 0.03;
    float3 shown = prepared_at(int2(floor(float2(uv.x + waver, uv.y - slip) * size)));
    int2 texel = clamp(xy, int2(0, 0), int2(size) - int2(1, 1));
    float noise = static_noise(texel, uint(static_clock()));
    float band = (frac(row - t * 0.37) - 0.5) / 0.08;
    float lit = noise * (1.0 - 0.55 * exp(-band * band));
    float snow = min(lit * lit * 1.4, 1.0);
    return lerp(shown, V3(snow), V3(strength));
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
// longer or shorter keeps that much for its own length, when the host says how long it was,
// unless the glow fades once a frame, as Super Win the Game's does.
float3 persistence() {
    float3 per_tick = float3(P(CRTSIM_PERSISTENCE_R), P(CRTSIM_PERSISTENCE_G),
        P(CRTSIM_PERSISTENCE_B));
    float ticks = HOST_FRAME_TICKS;
    if (ticks > 0.0 && P(CRTSIM_GLOW_PER_FRAME) < 0.5) return pow(per_tick, V3(ticks));
    return per_tick;
}

// The level 8-bit history holds a channel's glow at once lit, with the glow floor on: the
// brightest level its persistence rounds back to itself, none where a frame of no time keeps
// all of it.
float floor_level(float kept) {
    return kept < 1.0 ? min(floor(0.5 / (1.0 - kept)), 255.0) / 255.0 : 0.0;
}

float3 glow_floor(float3 kept) {
    if (P(CRTSIM_GLOW_FLOOR) < 0.5) return V3(0.0);
    return float3(floor_level(kept.x), floor_level(kept.y), floor_level(kept.z));
}

// The difference between this pixel's luma and its two neighbours d pixels away.
float ring(int2 xy, int d, float brightness, float strength) {
    float lb = luma(picture_at(xy - int2(d, 0), strength));
    float rb = luma(picture_at(xy + int2(d, 0), strength));
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
    float strength = static_strength();
    float3 left = picture_at(xy - int2(1, 0), strength);
    float3 right = picture_at(xy + int2(1, 0), strength);
    float3 cur = picture_at(xy, strength);
    float3 color = clamp(cur + (left + right - 2.0 * cur) * a * P(CRTSIM_ARTIFACTS), V3(0.0),
        V3(1.0));
    float brightness = luma(color);
    float offset = 0.0;
    offset += ring(xy, 1, brightness, strength) * 1.0;
    offset += ring(xy, 2, brightness, strength) * (-0.3162277);
    offset += ring(xy, 3, brightness, strength) * 0.1;
    color = clamp(color + offset * P(CRTSIM_SHARPNESS) * lerp(V3(1.0), a, V3(P(CRTSIM_ARTIFACTS))),
        V3(0.0), V3(1.0));
    // Interlaced: the beam skips the other field's rows this tick; they only decay below. Auto
    // interlaces a picture of 400 rows or more, as a set drew a 480- or 576-line one.
    int interlace = int(P(CRTSIM_INTERLACE) + 0.5);
    bool interlaced = interlace == 1 || (interlace == 2 && rows >= 400);
    if (interlaced && (xy.y % 2) != (HOST_FRAME_COUNT % 2)) color = V3(0.0);
    float3 kept = persistence();
    float3 held = glow_floor(kept);
    float3 pl = max(previous_at(xy - int2(1, 0)), held);
    float3 pr = max(previous_at(xy + int2(1, 0)), held);
    float3 pc = max(previous_at(xy), held);
    float bleed = P(CRTSIM_BLEED);
    color = max(color, kept * (pc + (pl + pr) * bleed) / (1.0 + 2.0 * bleed));
    return float4(clamp(color, V3(0.0), V3(1.0)), 1.0);
}
