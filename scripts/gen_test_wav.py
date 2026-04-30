#!/usr/bin/env python3
"""Generate synthetic test WAV files and download a reference WAV for audio testing.

Outputs (relative to script location ../tmp/):
  - test_sine.wav: 16kHz 16-bit mono, 440Hz sine, ~2s
  - test_speech.wav: 16kHz 16-bit mono, speech-like AM sine, ~3s
  - test_16k.wav: downloaded from Mozilla DeepSpeech smoke test

Usage:
    python scripts/gen_test_wav.py
"""

import os
import sys
import struct
import math
import wave
import urllib.request

SAMPLE_RATE = 16000
NUM_CHANNELS = 1
SAMPLE_WIDTH = 2  # 16-bit
MAX_AMPLITUDE = 0.8
MAX_16BIT = int(MAX_AMPLITUDE * 32767)


def _script_dir() -> str:
    """Return the directory containing this script."""
    return os.path.dirname(os.path.abspath(__file__))


def _tmp_dir() -> str:
    """Return the tmp/ directory (sibling of scripts/)."""
    d = os.path.join(os.path.dirname(_script_dir()), "tmp")
    os.makedirs(d, exist_ok=True)
    return d


def write_wav(filepath: str, samples: list[int], duration_sec: float):
    """Write a 16-bit mono PCM WAV file from integer samples."""
    n_frames = int(SAMPLE_RATE * duration_sec)
    # Trim/pad samples to exact frame count
    samples = samples[:n_frames]
    if len(samples) < n_frames:
        samples += [0] * (n_frames - len(samples))

    with wave.open(filepath, "w") as wf:
        wf.setnchannels(NUM_CHANNELS)
        wf.setsampwidth(SAMPLE_WIDTH)
        wf.setframerate(SAMPLE_RATE)
        wf.writeframes(struct.pack(f"<{len(samples)}h", *samples))

    actual_dur = len(samples) / SAMPLE_RATE
    print(f"Created: {filepath}  ({actual_dur:.2f}s, {SAMPLE_RATE}Hz, 16-bit mono)")


def gen_sine() -> list[int]:
    """440 Hz sine wave, ~2 seconds."""
    duration = 2.0
    n = int(SAMPLE_RATE * duration)
    samples = []
    for i in range(n):
        t = i / SAMPLE_RATE
        val = int(MAX_16BIT * math.sin(2 * math.pi * 440.0 * t))
        samples.append(val)
    return samples


def gen_speech_like() -> list[int]:
    """Amplitude-modulated sine: 200 Hz carrier x 5 Hz modulator, ~3 seconds.

    Produces a rough approximation of voiced speech harmonics with
    rhythmic amplitude variation.
    """
    duration = 3.0
    carrier_freq = 200.0
    mod_freq = 5.0
    n = int(SAMPLE_RATE * duration)
    samples = []
    for i in range(n):
        t = i / SAMPLE_RATE
        # Amplitude modulation: envelope oscillates between 0.1 and 1.0
        envelope = 0.5 + 0.5 * math.sin(2 * math.pi * mod_freq * t)
        # Add a bit of higher harmonics for buzziness
        val = int(
            MAX_16BIT
            * envelope
            * (
                0.6 * math.sin(2 * math.pi * carrier_freq * t)
                + 0.3 * math.sin(2 * math.pi * carrier_freq * 2 * t)
                + 0.1 * math.sin(2 * math.pi * carrier_freq * 3 * t)
            )
        )
        samples.append(val)
    return samples


def download_ref_wav(output_path: str):
    """Download the Mozilla DeepSpeech smoke-test WAV file.

    Respects HTTP_PROXY / HTTPS_PROXY environment variables.
    """
    url = (
        "https://github.com/mozilla/DeepSpeech/raw/master/"
        "data/smoke_test/LDC93S1_pcms16le_1_16000.wav"
    )

    proxy_handlers = []
    for var, proto in [("HTTP_PROXY", "http"), ("HTTPS_PROXY", "https")]:
        proxy_url = os.environ.get(var) or os.environ.get(var.lower())
        if proxy_url:
            proxy_handlers.append(
                urllib.request.ProxyHandler({proto: proxy_url})
            )

    opener = urllib.request.build_opener(*proxy_handlers) if proxy_handlers else None

    print(f"Downloading: {url}")
    try:
        if opener:
            with opener.open(url, timeout=30) as resp:
                data = resp.read()
        else:
            with urllib.request.urlopen(url, timeout=30) as resp:
                data = resp.read()
        with open(output_path, "wb") as f:
            f.write(data)
        size_kb = len(data) / 1024
        print(f"Downloaded: {output_path}  ({size_kb:.1f} KB)")
    except Exception as e:
        print(f"ERROR downloading {url}: {e}", file=sys.stderr)
        sys.exit(1)


def main():
    tmp = _tmp_dir()

    # 1. Sine wave
    sine_path = os.path.join(tmp, "test_sine.wav")
    write_wav(sine_path, gen_sine(), 2.0)

    # 2. Speech-like
    speech_path = os.path.join(tmp, "test_speech.wav")
    write_wav(speech_path, gen_speech_like(), 3.0)

    # 3. Reference download
    ref_path = os.path.join(tmp, "test_16k.wav")
    download_ref_wav(ref_path)

    print("\nAll test WAV files ready in:", tmp)


if __name__ == "__main__":
    main()
