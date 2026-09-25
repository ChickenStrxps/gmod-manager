"""Synthesize a soft lo-fi bed for the product intro.

Deterministic and license-free: warm electric-piano chords, a round bass,
a lazy brushed beat and vinyl-like hiss. Writes a 44.1 kHz stereo WAV.

    python promo/tools/music.py OUT.wav SECONDS
"""

import sys
import wave

import numpy as np

RATE = 44_100
BPM = 78
BEAT = 60 / BPM

# Imaj7 - vi9 - IVmaj7 - V6/9 in C, voiced low and close.
CHORDS = [
    [48, 52, 55, 59, 64],
    [45, 52, 55, 59, 62],
    [41, 48, 52, 57, 60],
    [43, 50, 55, 57, 62],
]


def hz(note: float) -> float:
    return 440.0 * 2 ** ((note - 69) / 12)


def env(n: int, attack: float, decay: float) -> np.ndarray:
    t = np.arange(n) / RATE
    return np.minimum(1.0, t / attack) * np.exp(-t / decay)


def epiano(note: float, seconds: float, rng: np.random.Generator) -> np.ndarray:
    n = int(seconds * RATE)
    t = np.arange(n) / RATE
    f = hz(note) * (1 + rng.uniform(-0.0015, 0.0015))
    wobble = 1 + 0.002 * np.sin(2 * np.pi * 0.35 * t)
    tone = (
        np.sin(2 * np.pi * f * t * wobble)
        + 0.35 * np.sin(2 * np.pi * 2 * f * t) * np.exp(-t * 3)
        + 0.12 * np.sin(2 * np.pi * 3.01 * f * t) * np.exp(-t * 6)
    )
    return tone * env(n, 0.012, 1.8)


def kick(n: int) -> np.ndarray:
    t = np.arange(n) / RATE
    return np.sin(2 * np.pi * (50 + 60 * np.exp(-t * 30)) * t) * np.exp(-t * 11)


def brush(n: int, rng: np.random.Generator) -> np.ndarray:
    noise = rng.standard_normal(n)
    noise = np.convolve(noise, np.ones(6) / 6, mode="same")
    return noise * env(n, 0.004, 0.09)


def lowpass(x: np.ndarray, alpha: float) -> np.ndarray:
    y = np.empty_like(x)
    acc = 0.0
    for i, v in enumerate(x):
        acc += alpha * (v - acc)
        y[i] = acc
    return y


def render(seconds: float) -> np.ndarray:
    rng = np.random.default_rng(7)
    total = int(seconds * RATE) + RATE * 3
    left = np.zeros(total)
    right = np.zeros(total)
    bar = BEAT * 4
    bars = int(np.ceil(seconds / bar)) + 1

    def add(sig: np.ndarray, start: float, gain: float, pan: float = 0.0) -> None:
        i = int(start * RATE)
        if i >= total:
            return
        sig = sig[: total - i]
        left[i : i + len(sig)] += sig * gain * (1 - pan) ** 0.5
        right[i : i + len(sig)] += sig * gain * (1 + pan) ** 0.5

    for b in range(bars):
        chord = CHORDS[b % len(CHORDS)]
        t0 = b * bar
        for k, note in enumerate(chord):
            strum = k * 0.018
            add(epiano(note, bar + 1.2, rng), t0 + strum, 0.09, pan=(k - 2) * 0.18)
        # Gentle top-line pluck on the "and" of 2 and on 4.
        top = chord[-1] + 12
        add(epiano(top, 1.4, rng), t0 + BEAT * 1.5, 0.045, pan=0.3)
        add(epiano(top - 3, 1.4, rng), t0 + BEAT * 3, 0.04, pan=-0.3)
        # Bass.
        n = int(bar * RATE)
        tb = np.arange(n) / RATE
        bass = np.sin(2 * np.pi * hz(chord[0] - 12) * tb) * env(n, 0.02, 1.4)
        add(bass, t0, 0.22)
        # Beat: kick on 1 and the "and" of 3, brush on 2 and 4.
        if b > 0:
            for beat in (0, 2.5):
                add(kick(int(0.4 * RATE)), t0 + beat * BEAT, 0.28)
            for beat in (1, 3):
                add(brush(int(0.25 * RATE), rng), t0 + beat * BEAT, 0.05, pan=0.2)
            for beat in (0.5, 1.5, 2.5, 3.5):
                add(brush(int(0.08 * RATE), rng), t0 + beat * BEAT + rng.uniform(0, 0.02), 0.018, pan=-0.3)

    hiss = rng.standard_normal(total) * 0.004
    left += hiss
    right += np.roll(hiss, 97)
    stereo = np.stack([lowpass(left, 0.35), lowpass(right, 0.35)], axis=1)
    stereo = stereo[: int(seconds * RATE)]
    fade = int(2.5 * RATE)
    stereo[:RATE] *= np.linspace(0, 1, RATE)[:, None]
    stereo[-fade:] *= np.linspace(1, 0, fade)[:, None]
    stereo /= max(1e-9, np.abs(stereo).max()) / 0.8
    return stereo


def main() -> None:
    out, seconds = sys.argv[1], float(sys.argv[2])
    data = (render(seconds) * 32767).astype("<i2")
    with wave.open(out, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(data.tobytes())


if __name__ == "__main__":
    main()
