# Original chiptune for the showcase: 120 BPM, 17 bars (34 s), Am-F-C-G.
import numpy as np, wave, sys
SR = 44100; BPM = 120; BEAT = 60 / BPM; BAR = 4 * BEAT
DUR = 34.0
N = int(SR * DUR)
mix = np.zeros(N)
def midi(n): return 440 * 2 ** ((n - 69) / 12)
def env(n, a=0.005, d=0.1, s=0.6, r=0.05):
    t = np.arange(n) / SR; L = n / SR
    e = np.where(t < a, t / a, s + (1 - s) * np.exp(-(t - a) / d))
    e *= np.clip((L - t) / r, 0, 1)
    return e
def square(f, n, duty=0.5):
    t = np.arange(n) / SR
    return np.where((t * f) % 1 < duty, 1.0, -1.0)
def tri(f, n):
    t = np.arange(n) / SR
    return 2 * np.abs(2 * ((t * f) % 1) - 1) - 1
def add(sig, at, gain):
    i = int(at * SR); j = min(N, i + len(sig))
    if i < N: mix[i:j] += sig[: j - i] * gain
rng = np.random.default_rng(7)
def noise(n): return rng.uniform(-1, 1, n)

chords = [(57, [57, 60, 64]), (53, [53, 57, 60]), (48, [48, 52, 55]), (55, [55, 59, 62])]  # Am F C G
melody = [  # (beat offset in 2-bar phrase, midi, beats)
    (0, 76, 1), (1, 72, 0.5), (1.5, 74, 0.5), (2, 76, 1), (3, 79, 1),
    (4, 77, 1.5), (5.5, 76, 0.5), (6, 72, 1), (7, 69, 1),
]
melody2 = [
    (0, 72, 1), (1, 74, 0.5), (1.5, 76, 0.5), (2, 79, 1.5), (3.5, 81, 0.5),
    (4, 79, 1), (5, 76, 1), (6, 74, 1.5), (7.5, 71, 0.5),
]
def kick(at, g=0.9):
    n = int(0.18 * SR); t = np.arange(n) / SR
    f = 150 * np.exp(-t * 25) + 45
    ph = 2 * np.pi * np.cumsum(f) / SR
    add(np.sin(ph) * np.exp(-t * 14), at, g)
def snare(at, g=0.45):
    n = int(0.16 * SR); t = np.arange(n) / SR
    add((noise(n) * 0.8 + square(190, n) * 0.3) * np.exp(-t * 22), at, g)
def hat(at, g=0.12, open_=False):
    n = int((0.12 if open_ else 0.03) * SR); t = np.arange(n) / SR
    s = noise(n); s = np.diff(np.concatenate([[0], s]))
    add(s * np.exp(-t * (20 if open_ else 120)), at, g)
def zap(at, up=True, dur=0.45, g=0.35):
    n = int(dur * SR); t = np.arange(n) / SR
    f = (80 + 2500 * (t / dur) ** 2) if up else (2500 * (1 - t / dur) ** 2 + 60)
    ph = 2 * np.pi * np.cumsum(f) / SR
    s = np.sign(np.sin(ph)) * 0.6 + noise(n) * 0.25
    add(s * env(n, 0.005, 0.3, 0.7, 0.05), at, g)
def riser(at, dur, g=0.18):
    n = int(dur * SR); t = np.arange(n) / SR
    add(noise(n) * (t / dur) ** 2, at, g)

nbars = 17
for bar in range(nbars):
    t0 = bar * BAR
    root, tri_notes = chords[bar % 4]
    last = bar == nbars - 1
    intro = bar < 2
    # arpeggio (square 12.5% duty), 16ths
    for k in range(16):
        if last and k >= 8: break
        nn = tri_notes[k % 3] + 12 * (1 if (k // 3) % 2 else 0)
        n = int(BEAT / 4 * SR)
        g = 0.07 if not intro else 0.04 + 0.03 * (bar * 16 + k) / 32
        add(square(midi(nn), n, 0.125) * env(n, 0.002, 0.05, 0.3, 0.01), t0 + k * BEAT / 4, g)
    if intro:
        if bar == 1: riser(t0, BAR, 0.16)
        continue
    # bass (triangle), 8ths with octave bounce
    for k in range(8):
        if last and k >= 4: break
        nn = root - 12 + (12 if k % 2 else 0)
        n = int(BEAT / 2 * SR * 0.9)
        add(tri(midi(nn), n) * env(n, 0.003, 0.1, 0.7, 0.02), t0 + k * BEAT / 2, 0.32)
    # drums
    for b in range(4):
        if last and b >= 2: break
        kick(t0 + b * BEAT, 0.8)
        if b in (1, 3): snare(t0 + b * BEAT)
        hat(t0 + b * BEAT + BEAT / 2, 0.12)
        hat(t0 + b * BEAT + BEAT / 4, 0.05); hat(t0 + b * BEAT + 3 * BEAT / 4, 0.05)
    # lead (square 25% duty + slight vibrato via second detuned voice)
    if 2 <= bar < nbars - 1:
        phrase = melody if (bar // 4) % 2 == 0 else melody2
        half = (bar % 2) * 4
        for off, nn, ln in phrase:
            if half <= off < half + 4:
                n = int(ln * BEAT * SR * 0.92)
                s = square(midi(nn), n, 0.25) * 0.7 + square(midi(nn) * 1.004, n, 0.5) * 0.3
                add(s * env(n, 0.005, 0.15, 0.55, 0.03), t0 + (off - half) * BEAT, 0.11)
    if bar == 12: riser(t0 + 2 * BEAT, 2 * BEAT, 0.12)  # before UI section
# final chord stab and crash at 32 s
for nn in [57, 60, 64, 69]:
    n = int(1.6 * SR)
    add(square(midi(nn), n, 0.25) * env(n, 0.005, 0.5, 0.0, 0.1), 32.0, 0.07)
n = int(1.2 * SR); add(noise(n) * np.exp(-np.arange(n) / SR * 3), 32.0, 0.15)
kick(32.0, 1.0)
zap(0.0, True, 0.5, 0.3)        # power on
zap(33.35, False, 0.5, 0.3)     # power off
# light echo + soft clip + fade
d = int(0.375 * SR); echo = np.zeros(N); echo[d:] = mix[:-d] * 0.18
mix = mix + echo
mix = np.tanh(mix * 1.1) * 0.85
fade = int(0.15 * SR); mix[-fade:] *= np.linspace(1, 0, fade)
pcm = (mix * 32767).astype(np.int16)
st = np.stack([pcm, pcm], 1)
with wave.open(sys.argv[1], "wb") as w:
    w.setnchannels(2); w.setsampwidth(2); w.setframerate(SR); w.writeframes(st.tobytes())
print("peak", np.abs(mix).max())
