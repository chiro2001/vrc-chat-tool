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


def verify_captured_wav(wav_path, expected_freq=440.0, expected_duration=3.0, tolerance=0.5):
    """Verify captured WAV file has expected characteristics"""
    with wave.open(str(wav_path), 'rb') as wf:
        sample_rate = wf.getframerate()
        n_frames = wf.getnframes()
        raw = wf.readframes(n_frames)

    n_samples = len(raw) // 2
    samples = struct.unpack(f'<{n_samples}h', raw)

    # Check non-empty
    assert n_samples > 0, "Captured audio is empty"
    print(f"  Samples: {n_samples}, Rate: {sample_rate}Hz")

    # Check duration
    actual_duration = n_samples / sample_rate
    assert abs(actual_duration - expected_duration) < tolerance, \
        f"Duration mismatch: {actual_duration}s vs expected ~{expected_duration}s"
    print(f"  Duration: {actual_duration:.2f}s (expected ~{expected_duration}s) OK")

    # Check for signal (not silence) using RMS
    rms = math.sqrt(sum(s*s for s in samples) / len(samples))
    assert rms > 100, f"Signal too quiet: RMS={rms:.0f}"
    print(f"  RMS: {rms:.0f} (signal detected) OK")

    # Check sample rate
    assert sample_rate in (16000, 44100, 48000, 96000), f"Unexpected sample rate: {sample_rate}"
    print(f"  Sample rate: {sample_rate}Hz OK")

    return True


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


def main():
    print("=" * 60)
    print("VRC Chat Tool - E2E Audio Pipeline Test")
    print("=" * 60)

    # 1. Build Rust binary
    if not build_rust_binary():
        sys.exit(1)

    # 2. Generate test tone
    test_wav = TMP_DIR / "e2e_test_tone.wav"
    generate_test_tone(test_wav, freq=440.0, duration=3.0)

    # 3. Start playback in background thread
    print("\nStarting playback thread...")
    playback_thread = threading.Thread(
        target=play_wav_to_vb_cable,
        args=(test_wav,),
        daemon=True
    )
    playback_thread.start()

    # Small delay to ensure playback is ready before capture starts
    time.sleep(0.3)

    # 4. Run capture (blocking)
    print("\nStarting capture...")
    captured = run_rust_capture(duration_secs=4)  # extra second for safety

    playback_thread.join(timeout=5)

    if captured is None or not captured.exists():
        print("\nFAIL: Capture failed or no output file")
        sys.exit(1)

    # 5. Verify captured audio
    print(f"\nVerifying captured audio: {captured}")
    try:
        verify_captured_wav(captured, expected_freq=440.0, expected_duration=3.0, tolerance=1.0)
        print("\n" + "=" * 60)
        print("ALL TESTS PASSED")
        print("=" * 60)
        sys.exit(0)
    except AssertionError as e:
        print(f"\nFAIL: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()
