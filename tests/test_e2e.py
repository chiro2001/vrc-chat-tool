"""
End-to-end audio pipeline test using VB-Cable.
Flow: Generate test tone -> Play through VB-Cable output -> Capture via Rust binary -> Verify
"""
import subprocess
import sys
import time
import struct
import wave
import math
import os
import threading
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
TMP_DIR = PROJECT_ROOT / "tmp"
TMP_DIR.mkdir(exist_ok=True)

# VB-Cable device indices (from sounddevice)
VB_CABLE_PLAYBACK_INDEX = 6   # CABLE Input (play TO this device)
VB_CABLE_CAPTURE_INDEX = 1    # CABLE Output (capture FROM this device, sounddevice index)
RUST_BIN = PROJECT_ROOT / "src-tauri" / "target" / "debug" / "test_e2e.exe"


def generate_test_tone(path, freq=440.0, duration=3.0, sample_rate=16000, amplitude=0.5):
    """Generate a sine wave WAV file with a specific frequency"""
    n_samples = int(sample_rate * duration)
    with wave.open(str(path), 'w') as wf:
        wf.setnchannels(1)
        wf.setsampwidth(2)
        wf.setframerate(sample_rate)
        for i in range(n_samples):
            sample = int(amplitude * 32767.0 * math.sin(2 * math.pi * freq * i / sample_rate))
            wf.writeframes(struct.pack('<h', sample))
    print(f"Generated test tone: {path} ({freq}Hz, {duration}s)")


def estimate_frequency(samples, sample_rate):
    """Estimate dominant frequency using zero-crossing rate"""
    if len(samples) < 2:
        return 0.0
    crossings = 0
    for i in range(1, len(samples)):
        if (samples[i-1] >= 0 and samples[i] < 0) or (samples[i-1] < 0 and samples[i] >= 0):
            crossings += 1
    duration = len(samples) / sample_rate
    freq = crossings / 2.0 / duration
    return freq


def estimate_frequency_fft(samples, sample_rate):
    """Estimate dominant frequency using FFT (more accurate)"""
    import numpy as np
    n = len(samples)
    if n < 256:
        return 0.0
    window = np.hanning(n)
    fft = np.abs(np.fft.rfft(np.array(samples) * window))
    freqs = np.fft.rfftfreq(n, 1.0 / sample_rate)
    # Find peak in 200-2000Hz range (typical test tones)
    mask = (freqs >= 200) & (freqs <= 2000)
    if mask.any():
        peak_idx = np.argmax(fft[mask])
        return freqs[mask][peak_idx]
    return 0.0


def play_wav_to_vb_cable(wav_path, device_index=VB_CABLE_PLAYBACK_INDEX):
    """Play WAV file through VB-Cable output device"""
    import sounddevice as sd
    import numpy as np

    with wave.open(str(wav_path), 'rb') as wf:
        sample_rate = wf.getframerate()
        n_channels = wf.getnchannels()
        n_frames = wf.getnframes()
        raw = wf.readframes(n_frames)

    # Convert to numpy array
    dtype = np.int16
    audio = np.frombuffer(raw, dtype=dtype).astype(np.float32) / 32768.0
    if n_channels > 1:
        audio = audio.reshape(-1, n_channels)

    print(f"Playing {wav_path.name} ({n_frames} frames, {sample_rate}Hz) through device {device_index}...")

    try:
        sd.play(audio, samplerate=sample_rate, device=device_index)
        sd.wait()
        print("Playback complete")
    except Exception as e:
        print(f"Playback error: {e}")
        raise


def verify_captured_wav(wav_path, expected_freq=440.0, expected_duration=3.0, tolerance=0.5, freq_tolerance=20.0):
    """Verify captured WAV file has expected characteristics including frequency.
    Returns True if ALL checks pass, False otherwise."""
    with wave.open(str(wav_path), 'rb') as wf:
        sample_rate = wf.getframerate()
        n_frames = wf.getnframes()
        raw = wf.readframes(n_frames)

    n_samples = len(raw) // 2
    samples = struct.unpack(f'<{n_samples}h', raw)

    all_ok = True

    # Check non-empty
    if n_samples == 0:
        print("  FAIL: Captured audio is empty")
        return False
    print(f"  Samples: {n_samples}, Rate: {sample_rate}Hz")

    # Check sample rate
    if sample_rate not in (16000, 44100, 48000, 96000):
        print(f"  FAIL: Unexpected sample rate: {sample_rate}")
        all_ok = False
    else:
        print(f"  Sample rate: {sample_rate}Hz OK")

    # Check duration
    actual_duration = n_samples / sample_rate
    duration_ok = abs(actual_duration - expected_duration) < tolerance
    status = "OK" if duration_ok else f"WRONG ({actual_duration:.2f}s vs expected ~{expected_duration}s)"
    print(f"  Duration: {actual_duration:.2f}s (expected ~{expected_duration}s) {status}")
    if not duration_ok:
        all_ok = False

    # Check for signal (not silence) using RMS
    rms = math.sqrt(sum(s*s for s in samples) / len(samples))
    signal_ok = rms > 100
    status = "OK" if signal_ok else f"TOO QUIET (RMS={rms:.0f})"
    print(f"  RMS: {rms:.0f} {status}")
    if not signal_ok:
        all_ok = False

    # Frequency check using FFT
    estimated_freq = estimate_frequency_fft(samples, sample_rate)
    freq_ok = abs(estimated_freq - expected_freq) < freq_tolerance
    status = "OK" if freq_ok else f"WRONG (expected ~{expected_freq}Hz, got {estimated_freq:.0f}Hz)"
    print(f"  Frequency (FFT): {estimated_freq:.0f}Hz (expected ~{expected_freq}Hz) {status}")
    if not freq_ok:
        all_ok = False

    # Also zero-crossing estimate for comparison
    zc_freq = estimate_frequency(samples, sample_rate)
    print(f"  Frequency (zero-crossing): {zc_freq:.0f}Hz")

    return all_ok


def build_rust_binary():
    """Build the Rust e2e test binary"""
    print("Building Rust test binary...")
    result = subprocess.run(
        ["cargo", "build", "--bin", "test_e2e"],
        cwd=PROJECT_ROOT / "src-tauri",
        capture_output=True,
        text=True,
        timeout=120
    )
    if result.returncode != 0:
        print("Build FAILED:")
        print(result.stderr[-1000:])
        return False
    print("Build OK")
    return True


def run_rust_capture(duration_secs=3, output_wav="tmp/e2e_capture.wav"):
    """Run the Rust capture binary"""
    output_path = PROJECT_ROOT / output_wav
    output_path.parent.mkdir(exist_ok=True)

    print(f"Running Rust capture for {duration_secs}s...")
    result = subprocess.run(
        [str(RUST_BIN), str(duration_secs), str(output_path)],
        capture_output=True,
        text=True,
        timeout=duration_secs + 10
    )
    print(result.stdout)
    if result.returncode != 0:
        print("STDERR:", result.stderr)
        return None
    return output_path


def run_single_test(freq, duration, capture_secs):
    """Run a single test tone and verify"""
    test_wav = TMP_DIR / f"e2e_test_{freq}hz.wav"
    capture_wav = TMP_DIR / f"e2e_capture_{freq}hz.wav"

    generate_test_tone(test_wav, freq=freq, duration=duration, sample_rate=16000, amplitude=0.5)

    playback_thread = threading.Thread(
        target=play_wav_to_vb_cable,
        args=(test_wav,),
        daemon=True
    )
    playback_thread.start()
    time.sleep(0.3)

    captured = run_rust_capture(duration_secs=capture_secs, output_wav=f"tmp/e2e_capture_{int(freq)}hz.wav")
    playback_thread.join(timeout=capture_secs + 2)

    if captured is None or not captured.exists():
        print("FAIL: Capture failed")
        return False

    return verify_captured_wav(captured, expected_freq=freq, expected_duration=duration, tolerance=1.0)


def main():
    print("=" * 60)
    print("VRC Chat Tool - E2E Audio Pipeline Test")
    print("=" * 60)

    # 1. Build Rust binary
    if not build_rust_binary():
        sys.exit(1)

    tests = [
        {"name": "440Hz / 3s", "freq": 440.0, "duration": 3.0, "capture_secs": 4},
        {"name": "1000Hz / 2s", "freq": 1000.0, "duration": 2.0, "capture_secs": 3},
    ]

    results = []
    for test in tests:
        print(f"\n--- Test: {test['name']} ---")
        result = run_single_test(test["freq"], test["duration"], test["capture_secs"])
        results.append(result)

    # Summary
    print("\n" + "=" * 60)
    passed = sum(1 for r in results if r)
    print(f"RESULTS: {passed}/{len(results)} tests passed")
    for i, (r, t) in enumerate(zip(results, tests)):
        print(f"  [{('PASS' if r else 'FAIL')}] {t['name']}")
    print("=" * 60)

    sys.exit(0 if all(results) else 1)


if __name__ == "__main__":
    main()
