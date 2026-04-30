"""
Full E2E test: TTS speech → VB-Cable → App HTTP API → ASR → verify
Requirements: VB-Cable installed, app compiled (cargo build), config.yaml with Tencent creds
"""
import subprocess
import sys
import time
import json
import urllib.request
import threading
import struct
import wave
import math
import os
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
TMP_DIR = PROJECT_ROOT / "tmp"
TMP_DIR.mkdir(exist_ok=True)

APP_BINARY = PROJECT_ROOT / "src-tauri" / "target" / "debug" / "vrc-chat-tool.exe"
APP_SRC = PROJECT_ROOT / "src-tauri"
API_BASE = "http://127.0.0.1:9876"


def generate_tts_wav(text, output_path, rate=16000):
    """Generate a WAV file with Chinese speech using Windows SAPI TTS"""
    import subprocess

    # Convert path to absolute with forward slashes for PowerShell
    abs_path = str(Path(output_path).resolve()).replace('\\', '\\\\')

    ps_script = f'''
Add-Type -AssemblyName System.Speech
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$synth.Rate = 0
$synth.Volume = 100
$synth.SetOutputToWaveFile("{abs_path}")
$synth.Speak("{text}")
$synth.Dispose()
'''
    result = subprocess.run(
        ["powershell", "-NoProfile", "-Command", ps_script],
        capture_output=True, text=True, timeout=30,
        encoding="utf-8",
    )

    if result.returncode != 0:
        raise RuntimeError(f"TTS failed: {result.stderr}")

    if not Path(output_path).exists():
        raise RuntimeError(f"TTS output file not created: {output_path}")

    # Verify output
    with wave.open(str(output_path), 'rb') as wf:
        sr = wf.getframerate()
        nf = wf.getnframes()
        print(f"  TTS generated: {output_path} ({nf} frames, {sr}Hz)")


def http_post(endpoint, data=None):
    """Send HTTP POST request to the E2E test API"""
    url = f"{API_BASE}{endpoint}"
    req = urllib.request.Request(url, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            return json.loads(resp.read().decode())
    except Exception as e:
        return {"status": "error", "message": str(e)}


def http_get(endpoint):
    """Send HTTP GET request to the E2E test API"""
    url = f"{API_BASE}{endpoint}"
    try:
        with urllib.request.urlopen(url, timeout=5) as resp:
            return json.loads(resp.read().decode())
    except Exception as e:
        return {"status": "error", "message": str(e)}


def play_wav_to_vb_cable(wav_path, device_index=6):
    """Play WAV file through VB-Cable output device"""
    import sounddevice as sd
    import numpy as np

    with wave.open(str(wav_path), 'rb') as wf:
        sample_rate = wf.getframerate()
        n_channels = wf.getnchannels()
        n_frames = wf.getnframes()
        raw = wf.readframes(n_frames)

    dtype = np.int16
    audio = np.frombuffer(raw, dtype=dtype).astype(np.float32) / 32768.0
    if n_channels > 1:
        audio = audio.reshape(-1, n_channels)

    print(f"  Playing {wav_path.name} ({n_frames} frames, {sample_rate}Hz) through device {device_index}...")
    sd.play(audio, samplerate=sample_rate, device=device_index)
    sd.wait()
    print("  Playback complete")


def build_app():
    """Build the Rust app"""
    print("Building app...")
    result = subprocess.run(
        ["cargo", "build"],
        cwd=APP_SRC,
        capture_output=True, text=True, timeout=180
    )
    if result.returncode != 0:
        print("Build FAILED:")
        print(result.stderr[-500:])
        return False
    print("Build OK")
    return True


def start_app():
    """Start the app in E2E test mode as a background process"""
    print(f"Starting app in E2E mode...")
    process = subprocess.Popen(
        [str(APP_BINARY), "--e2e"],
        cwd=PROJECT_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    # Wait for server to be ready
    for i in range(30):
        status = http_get("/status")
        if status.get("status") in ("idle", "recording"):
            print(f"  App ready (status: {status['status']})")
            return process
        time.sleep(0.5)

    process.kill()
    raise RuntimeError("App failed to start within timeout")


def run_e2e_test(test_text, expected_keywords, test_name):
    """Run a single E2E test"""
    print(f"\n{'='*50}")
    print(f"Test: {test_name}")
    print(f"Text: {test_text}")
    print(f"Expected keywords: {expected_keywords}")

    # 1. Generate TTS WAV
    wav_path = TMP_DIR / f"e2e_tts_{test_name.replace(' ', '_')}.wav"
    generate_tts_wav(test_text, wav_path)

    # 2. Start recording via HTTP API
    print("  Starting recording...")
    resp = http_post("/start")
    if resp.get("status") != "ok":
        print(f"  FAIL: Could not start recording: {resp}")
        return False
    print(f"  Recording started: {resp}")
    time.sleep(0.5)

    # 3. Play TTS audio through VB-Cable in background
    t = threading.Thread(target=play_wav_to_vb_cable, args=(wav_path,), daemon=True)
    t.start()
    time.sleep(0.3)  # Let playback start

    # 4. Wait for playback to finish
    t.join(timeout=15)

    # 5. Stop recording
    print("  Stopping recording...")
    resp = http_post("/stop")
    print(f"  Stop response: {resp}")

    # 6. Wait for ASR to complete
    print("  Waiting for ASR result...")
    result_text = ""
    for i in range(60):  # Up to 60 seconds
        time.sleep(1)
        resp = http_get("/result")
        if resp.get("recording") == False and resp.get("text"):
            result_text = resp.get("text", "")
            break

    print(f"  ASR result: '{result_text}'")

    # 7. Verify result
    result_lower = result_text.lower()
    passed = True
    for keyword in expected_keywords:
        if keyword.lower() not in result_lower:
            print(f"  FAIL: Expected keyword '{keyword}' not found in result")
            passed = False

    status = "PASS" if passed else "FAIL"
    print(f"  {status}: '{result_text}' vs expected keywords {expected_keywords}")
    return passed


def main():
    print("=" * 60)
    print("VRC Chat Tool - Full E2E Test (TTS + HTTP API)")
    print("=" * 60)

    # Build
    if not build_app():
        sys.exit(1)

    # Start app in E2E mode
    app_process = start_app()

    try:
        tests = [
            {
                "name": "chinese_hello",
                "text": "你好世界",
                "keywords": ["你好", "世界"],
            },
            {
                "name": "chinese_weather",
                "text": "今天天气很好",
                "keywords": ["天气", "好"],
            },
        ]

        results = []
        for test in tests:
            passed = run_e2e_test(test["text"], test["keywords"], test["name"])
            results.append((test["name"], passed))
            time.sleep(1)  # Gap between tests

        # Summary
        print("\n" + "=" * 60)
        passed = sum(1 for _, ok in results if ok)
        print(f"RESULTS: {passed}/{len(results)} tests passed")
        for name, ok in results:
            print(f"  [{'PASS' if ok else 'FAIL'}] {name}")
        print("=" * 60)

        if passed < len(results):
            sys.exit(1)
    finally:
        app_process.kill()
        app_process.wait(timeout=5)


if __name__ == "__main__":
    main()
