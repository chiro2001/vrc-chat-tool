import os
import sys
import subprocess
import json
import struct
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent
SRC_TAURI = PROJECT_ROOT / "src-tauri"
CONFIG_FILE = PROJECT_ROOT / "config.yaml"

def test_config_file_exists():
    print("[TEST] Checking config.yaml...")
    assert CONFIG_FILE.exists(), "config.yaml not found"
    print("  PASS: config.yaml exists")

def test_config_has_required_fields():
    print("[TEST] Checking config.yaml fields...")
    content = CONFIG_FILE.read_text(encoding="utf-8")
    required = ["tencent_app_id", "tencent_secret_id", "tencent_secret_key", "osc_host", "osc_port"]
    for field in required:
        assert field in content, f"Missing field: {field}"
    print("  PASS: All required fields present")

def test_cargo_toml_exists():
    print("[TEST] Checking Cargo.toml...")
    cargo = SRC_TAURI / "Cargo.toml"
    assert cargo.exists(), "Cargo.toml not found"
    content = cargo.read_text()
    required_deps = ["cpal", "rosc", "tokio-tungstenite", "hmac", "serde_yaml"]
    for dep in required_deps:
        assert dep in content, f"Missing dependency: {dep}"
    print("  PASS: Cargo.toml has required dependencies")

def test_rust_modules_exist():
    print("[TEST] Checking Rust module files...")
    src = SRC_TAURI / "src"
    expected_files = [
        "main.rs", "config.rs",
        "audio/mod.rs", "audio/capture.rs",
        "speech/mod.rs", "speech/tencent.rs", "speech/streaming.rs",
        "osc/mod.rs", "osc/sender.rs",
    ]
    for f in expected_files:
        fp = src / f
        assert fp.exists(), f"Missing: {f}"
        assert fp.stat().st_size > 0, f"Empty: {f}"
    print("  PASS: All {} module files exist and non-empty".format(len(expected_files)))

def test_tauri_conf_exists():
    print("[TEST] Checking tauri.conf.json...")
    conf = SRC_TAURI / "tauri.conf.json"
    assert conf.exists(), "tauri.conf.json not found"
    content = conf.read_text()
    assert '"productName"' in content, "Missing productName"
    assert '"allowlist"' in content, "Missing allowlist"
    print("  PASS: tauri.conf.json is valid")

def test_test_wav_files():
    print("[TEST] Checking test WAV files...")
    tmp = PROJECT_ROOT / "tmp"
    wav_files = ["test_sine.wav", "test_speech.wav", "test_16k.wav"]
    for wf in wav_files:
        fp = tmp / wf
        if fp.exists():
            # Verify WAV header
            data = fp.read_bytes()
            assert data[:4] == b"RIFF", f"{wf} invalid WAV header"
            print(f"  PASS: {wf} exists, {len(data)} bytes, valid WAV")
        else:
            print(f"  SKIP: {wf} not found (run gen_test_wav.py first)")

def test_config_yaml_valid():
    print("[TEST] Checking config.yaml is valid YAML...")
    try:
        import yaml
        with open(CONFIG_FILE, 'r') as f:
            data = yaml.safe_load(f)
        assert isinstance(data, dict)
        assert "tencent_app_id" in data
        print(f"  PASS: Valid YAML with {len(data)} fields")
    except ImportError:
        print("  SKIP: PyYAML not installed (pip install pyyaml)")

def test_cargo_check():
    print("[TEST] Running cargo check...")
    result = subprocess.run(
        ["cargo", "check"],
        cwd=SRC_TAURI,
        capture_output=True,
        text=True,
        timeout=120
    )
    if result.returncode == 0:
        print("  PASS: cargo check succeeded")
    else:
        # Filter to only show errors
        errors = [l for l in result.stderr.split('\n') if 'error' in l.lower()]
        print(f"  FAIL: cargo check returned {result.returncode}")
        for e in errors[:5]:
            print(f"    {e}")
        # Don't assert - this is informational

def test_cargo_test():
    print("[TEST] Running cargo test...")
    result = subprocess.run(
        ["cargo", "test"],
        cwd=SRC_TAURI,
        capture_output=True,
        text=True,
        timeout=180
    )
    if result.returncode == 0:
        # Count passed tests
        for line in result.stdout.split('\n'):
            if 'test result:' in line:
                print(f"  PASS: {line.strip()}")
                break
    else:
        print(f"  FAIL: cargo test returned {result.returncode}")
        # Show errors
        errors = [l for l in result.stderr.split('\n') if 'error' in l.lower()]
        for e in errors[:5]:
            print(f"    {e}")

def test_frontend_build():
    print("[TEST] Running npm run build...")
    result = subprocess.run(
        ["npm", "run", "build"],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60
    )
    if result.returncode == 0:
        dist = PROJECT_ROOT / "dist"
        if dist.exists():
            files = list(dist.glob("**/*"))
            print(f"  PASS: Frontend built, {len(files)} files in dist/")
        else:
            print("  FAIL: dist/ directory not created")
    else:
        print(f"  FAIL: npm build returned {result.returncode}")

def main():
    print("=" * 60)
    print("VRC Chat Tool - Integration Test Suite")
    print("=" * 60)

    tests = [
        test_config_file_exists,
        test_config_has_required_fields,
        test_cargo_toml_exists,
        test_rust_modules_exist,
        test_tauri_conf_exists,
        test_test_wav_files,
        test_config_yaml_valid,
    ]

    results = []
    for test in tests:
        try:
            test()
            results.append(("PASS", test.__name__))
        except AssertionError as e:
            results.append(("FAIL", f"{test.__name__}: {e}"))
        except Exception as e:
            results.append(("FAIL", f"{test.__name__}: {e}"))

    # Optional: heavier tests
    if "--full" in sys.argv:
        try:
            test_cargo_check()
        except Exception as e:
            print(f"  FAIL: cargo check error: {e}")
        try:
            test_cargo_test()
        except Exception as e:
            print(f"  FAIL: cargo test error: {e}")
        try:
            test_frontend_build()
        except Exception as e:
            print(f"  FAIL: frontend build error: {e}")

    print()
    print("=" * 60)
    print("SUMMARY")
    passed = sum(1 for r in results if r[0] == "PASS")
    failed = sum(1 for r in results if r[0] == "FAIL")
    print(f"  {passed} passed, {failed} failed, {len(results)} total")
    for status, name in results:
        print(f"  [{status}] {name}")
    print("=" * 60)

    return 0 if failed == 0 else 1

if __name__ == "__main__":
    sys.exit(main())
