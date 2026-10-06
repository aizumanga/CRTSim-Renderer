#!/usr/bin/env python3
# Original chiptune tracks in several moods, one per showcase clip, synthesized from scratch.
# Usage: musicgen.py STYLE SECONDS OUT.wav   (STYLE: see STYLES below)
import sys, wave
import numpy as np

SR = 44100

def midi(n): return 440 * 2 ** ((n - 69) / 12)

def env(n, a=0.005, d=0.1, s=0.6, r=0.05):
    t = np.arange(n) / SR; L = n / SR
    e = np.where(t < a, t / a, s + (1 - s) * np.exp(-(t - a) / d))
    return e * np.clip((L - t) / r, 0, 1)

def square(f, n, duty=0.5):
    return np.where((np.arange(n) / SR * f) % 1 < duty, 1.0, -1.0)

def tri(f, n):
    return 2 * np.abs(2 * ((np.arange(n) / SR * f) % 1) - 1) - 1

def sine(f, n):
    return np.sin(2 * np.pi * f * np.arange(n) / SR)

WAVES = {"sq12": lambda f, n: square(f, n, 0.125), "sq25": lambda f, n: square(f, n, 0.25),
         "sq50": lambda f, n: square(f, n, 0.5), "tri": tri, "sine": sine,
         "chorus": lambda f, n: 0.6 * square(f, n, 0.25) + 0.4 * square(f * 1.006, n, 0.5)}

# Chords as (bass root, chord tones), MIDI numbers.
Am, F, C, G = (57, [57, 60, 64]), (53, [53, 57, 60]), (48, [48, 52, 55]), (55, [55, 59, 62])
Em, Cm_, D, B = (52, [52, 55, 59]), (48, [48, 52, 55]), (50, [50, 54, 57]), (47, [47, 51, 54])
Fm, Db, Eb, Cmaj = (53, [53, 56, 60]), (49, [49, 53, 56]), (51, [51, 55, 58]), (48, [48, 52, 55])
Fsm, Dm_, A, E = (54, [54, 57, 61]), (50, [50, 54, 57]), (57, [57, 61, 64]), (52, [52, 56, 59])
Dm9, G13, Cmaj9, Am11 = (50, [53, 57, 60, 64]), (55, [53, 57, 59, 64]), (48, [52, 55, 59, 62]), (45, [55, 60, 62, 64])
Fmaj7, Em7, Dm7, Cmaj7 = (53, [57, 60, 64, 65]), (52, [55, 59, 62, 64]), (50, [53, 57, 60, 62]), (48, [52, 55, 59, 60])

STYLES = {
    # bpm, chords (one per bar), arp wave, bass, drums, lead wave, lead phrase (beat, note, beats) over 8 beats
    "reel": dict(bpm=120, chords=[Am, F, C, G], arp="sq12", bass="octaves", drums="full", lead="sq25",
                 phrase=[(0, 76, 1), (1, 72, .5), (1.5, 74, .5), (2, 76, 1), (3, 79, 1), (4, 77, 1.5), (5.5, 76, .5), (6, 72, 1), (7, 69, 1)],
                 intro=8),
    "arcade": dict(bpm=150, chords=[Em, Cm_, D, B], arp="sq12", bass="octaves", drums="full", lead="sq25",
                   phrase=[(0, 76, .5), (.5, 79, .5), (1, 83, 1), (2, 81, .5), (2.5, 79, .5), (3, 78, 1), (4, 76, .5), (4.5, 74, .5), (5, 72, 1), (6, 74, 1), (7, 75, 1)],
                   intro=2),
    "terminal": dict(bpm=84, chords=[Dm9, G13, Cmaj9, Am11], arp="sine", bass="whole", drums="none", lead=None,
                     phrase=[], intro=0, arp_rate=2, arp_gain=0.09),
    "demoscene": dict(bpm=120, chords=[Fm, Db, Eb, Cmaj], arp="sq12", bass="offbeat", drums="four", lead="chorus",
                      phrase=[(0, 72, 1.5), (1.5, 75, .5), (2, 77, 1), (3, 80, 1), (4, 79, 1.5), (5.5, 77, .5), (6, 75, 1), (7, 72, 1)],
                      intro=0),
    "rpg": dict(bpm=96, chords=[C, G, Am, F], arp="tri", bass="quarters", drums="soft", lead="sq50",
                phrase=[(0, 72, 1.5), (1.5, 74, .5), (2, 76, 2), (4, 79, 1), (5, 77, .5), (5.5, 76, .5), (6, 74, 2)],
                intro=0, arp_rate=2),
    "racer": dict(bpm=144, chords=[Fsm, Dm_, A, E], arp="sq25", bass="driving", drums="full", lead="sq25",
                  phrase=[(0, 78, 1), (1, 81, 1), (2, 85, 1.5), (3.5, 83, .5), (4, 81, 1), (5, 80, 1), (6, 78, 1.5), (7.5, 76, .5)],
                  intro=0),
    "broadcast": dict(bpm=104, chords=[Fmaj7, Em7, Dm7, Cmaj7], arp="chorus", bass="quarters", drums="soft", lead="sine",
                      phrase=[(0, 84, 1), (1, 81, 1), (2, 79, 2), (4, 77, 1), (5, 79, 1), (6, 76, 2)],
                      intro=0, arp_rate=2, tone=1.6),
}

def track(style, dur, seed=7):
    p = STYLES[style]
    N = int(SR * dur); mix = np.zeros(N); rng = np.random.default_rng(seed)
    beat = 60 / p["bpm"]; bar = 4 * beat

    def add(sig, at, gain):
        i = int(at * SR); j = min(N, i + len(sig))
        if 0 <= i < N: mix[i:j] += sig[: j - i] * gain

    def noise(n): return rng.uniform(-1, 1, n)

    def kick(at, g):
        n = int(0.18 * SR); t = np.arange(n) / SR
        add(np.sin(2 * np.pi * np.cumsum(150 * np.exp(-t * 25) + 45) / SR) * np.exp(-t * 14), at, g)

    def snare(at, g):
        n = int(0.16 * SR); t = np.arange(n) / SR
        add((noise(n) * 0.8 + square(190, n) * 0.3) * np.exp(-t * 22), at, g)

    def hat(at, g, length=0.03, decay=120):
        n = int(length * SR); s = np.diff(np.concatenate([[0], noise(n)]))
        add(s * np.exp(-np.arange(n) / SR * decay), at, g)

    if p.get("tone"):  # a test tone under a test pattern
        n = int(p["tone"] * SR); add(sine(1000, n) * env(n, 0.01, 1, 1, 0.05), 0, 0.12)
    rate = p.get("arp_rate", 4)
    for b in range(int(np.ceil(dur / bar))):
        t0 = b * bar
        root, tones = p["chords"][b % len(p["chords"])]
        for k in range(4 * rate):
            nn = tones[k % len(tones)] + (12 if (k // len(tones)) % 2 else 0)
            n = int(beat / rate * SR)
            add(WAVES[p["arp"]](midi(nn), n) * env(n, 0.002, 0.06, 0.35, 0.01), t0 + k * beat / rate, p.get("arp_gain", 0.07))
        bass = p["bass"]
        steps = {"octaves": [(k / 2, root - 12 + 12 * (k % 2), .45) for k in range(8)],
                 "driving": [(k / 2, root - 12, .45) for k in range(8)],
                 "offbeat": [(k + .5, root - 12, .4) for k in range(4)] + [(k, root - 24, .4) for k in range(4)],
                 "quarters": [(k, root - 12 + (7 if k == 2 else 0), .9) for k in range(4)],
                 "whole": [(0, root - 12, 4)]}[bass]
        for at, nn, ln in steps:
            n = int(ln * beat * SR)
            add(tri(midi(nn), n) * env(n, 0.003, 0.2, 0.7, 0.05), t0 + at * beat, 0.32)
        if b * 4 >= p["intro"] and p["drums"] != "none":
            for k in range(4):
                tb = t0 + k * beat
                if p["drums"] in ("full", "four"): kick(tb, 0.8)
                elif k in (0, 2): kick(tb, 0.5)
                if k in (1, 3) and p["drums"] != "four": snare(tb, 0.4 if p["drums"] == "full" else 0.18)
                if k in (1, 3) and p["drums"] == "four": snare(tb, 0.35)
                hat(tb + beat / 2, 0.12 if p["drums"] != "soft" else 0.06)
                if p["drums"] in ("full", "four"):
                    hat(tb + beat / 4, 0.05); hat(tb + 3 * beat / 4, 0.05)
        if p["lead"] and b * 4 >= p["intro"]:
            half = (b % 2) * 4
            for off, nn, ln in p["phrase"]:
                if half <= off < half + 4:
                    n = int(ln * beat * SR * 0.92)
                    add(WAVES[p["lead"]](midi(nn), n) * env(n, 0.005, 0.15, 0.55, 0.03), t0 + (off - half) * beat, 0.11)
    d = int(0.75 * beat * SR); echo = np.zeros(N); echo[d:] = mix[:-d] * 0.2
    mix = np.tanh((mix + echo) * 1.1) * 0.85
    fade = int(0.4 * SR); mix[-fade:] *= np.linspace(1, 0, fade)
    mix[: int(0.01 * SR)] *= np.linspace(0, 1, int(0.01 * SR))
    return mix

if __name__ == "__main__":
    style, seconds, out = sys.argv[1], float(sys.argv[2]), sys.argv[3]
    pcm = (track(style, seconds) * 32767).astype(np.int16)
    with wave.open(out, "wb") as w:
        w.setnchannels(2); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes(np.stack([pcm, pcm], 1).tobytes())
