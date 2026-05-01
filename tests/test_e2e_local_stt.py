"""
E2E test for local STT pipeline: stt-server + stt-gui + VB-Cable.

Flow:
    1. Start stt-server (WebSocket STT server with sherpa-onnx)
    2. Generate Chinese TTS test audio via Windows SAPI
    3. Start stt-gui in test mode (auto-record from VB-Cable, save results, exit)
    4. Play TTS WAV through VB-Cable → stt-gui captures → sends to stt-server
    5. Wait for stt-gui to finish (timeout or completion)
    6. Read recognition result JSON
    7. Verify expected keywords in recognized text
    8. Cleanup all processes

Requirements:
    - VB-Cable installed
    - sounddevice, numpy, soundfile Python packages
    - stt-server model downloaded (cargo run -p stt-server -- download)
    - Rust binaries built (cargo build -p stt-server -p stt-gui)
"""

import subprocess
import sys
import time
import json
import threading
import os
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
TMP_DIR = PROJECT_ROOT / "tmp"
TMP_DIR.mkdir(exist_ok=True)

STT_SERVER_BIN = PROJECT_ROOT / "crates" / "stt-server"
STT_GUI_BIN = PROJECT_ROOT / "stt-gui"
STT_SERVER_URL = "ws://127.0.0.1:8765"

RESULT_FILE = TMP_DIR / "stt_e2e_result.json"


def find_vb_cable_device(direction="output"):
    """Find VB-Cable device index by name using sounddevice.

    direction: 'output' = CABLE Input (playback), 'input' = CABLE Output (capture)
    """
    import sounddevice as sd
    devs = sd.query_devices()
    search = "CABLE Input" if direction == "output" else "CABLE Output"
    for d in devs:
        if search in d.get("name", "") and d.get(f"max_{direction}_channels", 0) > 0:
            return d["index"]
    raise RuntimeError(f"VB-Cable {direction} device not found. Is VB-Cable installed?")


def play_wav_to_vb_cable(wav_path, device_index=None):
    """Play WAV file through VB-Cable output device."""
    import sounddevice as sd
    import soundfile as sf

    if device_index is None:
        device_index = find_vb_cable_device("output")

    data, sr = sf.read(str(wav_path))
    print(f"  Playing {wav_path.name} ({len(data)} samples, {sr}Hz) through device {device_index}...")
    sd.play(data, sr, device=device_index)
    sd.wait()
    print("  Playback complete")


def generate_tts_wav(text, output_path, rate=16000):
    """Generate a WAV file with Chinese speech using Windows SAPI TTS."""
    abs_path = str(Path(output_path).resolve()).replace("\\", "\\\\")

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
    )

    if result.returncode != 0:
        raise RuntimeError(f"TTS failed: {result.stderr}")

    if not Path(output_path).exists():
        raise RuntimeError(f"TTS output file not created: {output_path}")

    import wave
    with wave.open(str(output_path), "rb") as wf:
        print(f"  TTS generated: {output_path} ({wf.getnframes()} frames, {wf.getframerate()}Hz)")


def wait_for_port(host, port, timeout=30):
    """Wait until a TCP port is accepting connections."""
    import socket
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            sock = socket.create_connection((host, port), timeout=1)
            sock.close()
            return True
        except (ConnectionRefusedError, OSError):
            time.sleep(0.5)
    return False


def start_stt_server():
    """Start stt-server as a background process. Returns Popen or None."""
    print(f"\n[1/5] Starting stt-server...")
    try:
        proc = subprocess.Popen(
            ["cargo", "run", "-p", "stt-server", "--", "run"],
            cwd=PROJECT_ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        print(f"  stt-server PID: {proc.pid}")

        # Wait for WebSocket port
        if wait_for_port("127.0.0.1", 8765, timeout=30):
            print("  stt-server ready on ws://127.0.0.1:8765")
            return proc
        else:
            print("  ERROR: stt-server failed to start within timeout")
            # Print stderr for debugging
            try:
                _, stderr = proc.communicate(timeout=1)
                if stderr:
                    print(f"  stderr: {stderr.decode('utf-8', errors='replace')[-500:]}")
            except subprocess.TimeoutExpired:
                pass
            proc.kill()
            return None
    except Exception as e:
        print(f"  ERROR starting stt-server: {e}")
        return None


def start_stt_gui_test(test_wav_path, timeout_secs=15):
    """Start stt-gui in test mode. Returns Popen.

    stt-gui will:
        - Auto-connect to stt-server
        - Auto-start recording from VB-Cable
        - Collect recognition results
        - Save to RESULT_FILE and exit
    """
    print(f"\n[3/5] Starting stt-gui in test mode (timeout={timeout_secs}s)...")

    # Clean previous result
    if RESULT_FILE.exists():
        RESULT_FILE.unlink()

    cmd = [
        "cargo", "run", "-p", "stt-gui", "--",
        "--test-timeout", str(timeout_secs),
        "--url", STT_SERVER_URL,
        "--device", "CABLE",
        "--test-output", str(RESULT_FILE),
    ]

    print(f"  Command: {' '.join(cmd)}")

    proc = subprocess.Popen(
        cmd,
        cwd=PROJECT_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    print(f"  stt-gui PID: {proc.pid}")
    return proc


def run_e2e_test(test_text, expected_keywords, test_name, timeout_secs=20):
    """Run a single E2E test.

    1. Start stt-server
    2. Generate TTS WAV
    3. Start stt-gui in test mode
    4. Play TTS through VB-Cable
    5. Wait for results
    6. Verify
    """
    print(f"\n{'='*60}")
    print(f"Test: {test_name}")
    print(f"Text: '{test_text}'")
    print(f"Expected keywords: {expected_keywords}")
    print(f"{'='*60}")

    # ── 1. Start stt-server ──
    server_proc = start_stt_server()
    if server_proc is None:
        print("  SKIP: stt-server could not start")
        return False

    try:
        # ── 2. Generate TTS WAV ──
        print(f"\n[2/5] Generating TTS audio...")
        wav_path = TMP_DIR / f"e2e_local_{test_name.replace(' ', '_')}.wav"
        generate_tts_wav(test_text, wav_path)

        # ── 3. Start stt-gui in test mode ──
        gui_proc = start_stt_gui_test(wav_path, timeout_secs=timeout_secs)
        time.sleep(2.0)  # Wait for stt-gui to initialize and start recording

        # ── 4. Play TTS through VB-Cable ──
        print(f"\n[4/5] Playing TTS through VB-Cable...")
        play_thread = threading.Thread(
            target=play_wav_to_vb_cable,
            args=(wav_path,),
            daemon=True,
        )
        play_thread.start()
        # Wait for playback to finish
        play_thread.join(timeout=timeout_secs)

        # ── 5. Wait for stt-gui to finish ──
        print(f"\n[5/5] Waiting for stt-gui to complete (timeout={timeout_secs}s)...")
        deadline = time.time() + timeout_secs + 5
        result = None
        while time.time() < deadline:
            if RESULT_FILE.exists():
                try:
                    with open(RESULT_FILE, "r", encoding="utf-8") as f:
                        result = json.load(f)
                    break
                except (json.JSONDecodeError, IOError) as e:
                    print(f"  Waiting for valid result file... ({e})")
            time.sleep(0.5)

        if result is None:
            print("  ERROR: stt-gui did not produce result file within timeout")
            # Print stt-gui stderr for debugging
            try:
                _, stderr = gui_proc.communicate(timeout=1)
                if stderr:
                    print(f"  stt-gui stderr: {stderr.decode('utf-8', errors='replace')[-1000:]}")
            except subprocess.TimeoutExpired:
                pass
            return False

        # ── 6. Verify results ──
        print(f"\n  Result JSON: {json.dumps(result, ensure_ascii=False, indent=2)}")

        status = result.get("status", "unknown")
        segments = result.get("segments", [])
        all_text = " ".join(s.get("text", "") for s in segments)
        partial = result.get("partial", "")

        print(f"  Status: {status}")
        print(f"  Segments: {len(segments)}")
        print(f"  Combined text: '{all_text}'")
        print(f"  Partial text: '{partial}'")

        # Check if we got any recognition
        if not all_text and not partial:
            print(f"  WARN: No text recognized (status={status})")
            print(f"  This may be normal if the model hasn't been downloaded or the STT")
            print(f"  server failed to process the audio. Check stt-server logs.")

        # Verify keywords in combined text
        passed = True
        search_text = (all_text + " " + partial).lower()
        for keyword in expected_keywords:
            if keyword.lower() not in search_text:
                print(f"  FAIL: Expected keyword '{keyword}' not found")
                passed = False
            else:
                print(f"  OK: Found keyword '{keyword}'")

        status_str = "PASS" if passed else "FAIL"
        print(f"\n  [{status_str}] {test_name}")

        return passed

    finally:
        # ── Cleanup ──
        print("\n  Cleaning up processes...")
        for proc, name in [(server_proc, "stt-server")]:
            if proc and proc.poll() is None:
                proc.kill()
                try:
                    proc.wait(timeout=5)
                    print(f"  Stopped {name}")
                except subprocess.TimeoutExpired:
                    print(f"  Force killed {name}")
        # Also try to find and kill any lingering stt-gui
        try:
            if gui_proc and gui_proc.poll() is None:
                gui_proc.kill()
                gui_proc.wait(timeout=5)
        except (NameError, subprocess.TimeoutExpired):
            pass


def main():
    print("=" * 60)
    print("VRC Chat Tool - E2E Local STT Test")
    print("(stt-server + stt-gui + VB-Cable)")
    print("=" * 60)

    # ── Pre-flight checks ──
    # Check VB-Cable
    try:
        vb_out = find_vb_cable_device("output")
        vb_in = find_vb_cable_device("input")
        print(f"VB-Cable found: playback={vb_out}, capture={vb_in}")
    except RuntimeError as e:
        print(f"ERROR: {e}")
        print("Please install VB-Cable from https://vb-audio.com/Cable/")
        sys.exit(1)

    # Check TTS capability
    try:
        generate_tts_wav("测试", TMP_DIR / "_tts_check.wav")
        (TMP_DIR / "_tts_check.wav").unlink()
        print("Windows SAPI TTS: OK")
    except Exception as e:
        print(f"ERROR: Windows SAPI TTS not available: {e}")
        sys.exit(1)

    # ── Run tests ──
    tests = [
        {
            "name": "chinese_hello",
            "text": "你好世界",
            "keywords": ["你好", "世界"],
            "timeout": 20,
        },
        {
            "name": "chinese_weather",
            "text": "今天天气很好",
            "keywords": ["天气", "好"],
            "timeout": 20,
        },
    ]

    results = []
    for test in tests:
        passed = run_e2e_test(
            test["text"],
            test["keywords"],
            test["name"],
            timeout_secs=test["timeout"],
        )
        results.append((test["name"], passed))

    # ── Summary ──
    print("\n" + "=" * 60)
    print("E2E LOCAL STT TEST RESULTS")
    print("=" * 60)
    passed = sum(1 for _, ok in results if ok)
    total = len(results)
    print(f"Passed: {passed}/{total}")
    for name, ok in results:
        print(f"  [{'PASS' if ok else 'FAIL'}] {name}")
    print("=" * 60)

    sys.exit(0 if passed == total else 1)


if __name__ == "__main__":
    main()
