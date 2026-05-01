# VRC Chat Tool — Project Knowledge Base

**Generated:** 2026-05-01
**Commit:** 246d549
**Branch:** master

## OVERVIEW
Desktop voice-to-text tool for VRChat. Microphone audio → ASR (Tencent Cloud / remote STT / local Sherpa-ONNX) → OSC messages → VRChat chatbox.
**Stack:** Tauri v1 + Rust backend + React 19 (TypeScript) frontend + Vite 8.

## STRUCTURE
```
vrc-chat-tool/
├── src-tauri/              # Rust backend (Tauri app, bin, lib)
│   └── src/
│       ├── main.rs         # Tauri entry, commands, event wiring (690 lines — too large)
│       ├── lib.rs          # Public module declarations
│       ├── audio/          # cpal audio capture + PCM conversion
│       ├── speech/         # ASR providers (Tencent, remote STT, local embedded)
│       ├── osc/            # rosc UDP OSC sender
│       ├── config.rs       # AppConfig + TencentCredentials (YAML)
│       ├── history.rs      # SQLite recognition history
│       ├── trigger.rs      # Always-on STT trigger listener
│       ├── log.rs          # File logger (tmp/app.log)
│       ├── hotkey.rs       # Win32 F10 global hotkey
│       ├── e2e_server.rs   # HTTP API for integration tests
│       └── bin/test_e2e.rs # Standalone E2E test binary
├── src-ui/                 # React 19 frontend (TypeScript)
│   └── src/
│       ├── App.tsx         # Single-page UI (700+ lines)
│       ├── App.css         # Dark theme styles
│       ├── main.tsx        # React entry
│       └── i18n.ts         # zh-CN / en translations
├── crates/stt-server/      # Sherpa-ONNX STT engine crate
├── stt-gui/                # Debugging GUI client (egui, WebSocket)
├── tests/                  # Python E2E tests (no pytest)
├── config.yaml             # App runtime config
├── stt-config.yaml         # Local STT model config
├── tencent_credentials.yaml # Tencent Cloud secrets (gitignored)
├── scripts/gen_test_wav.py # Test WAV generator
└── tmp/                    # Test data, recordings, logs (gitignored)
```

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Add Tauri command | src-tauri/src/main.rs | Add fn + register in `generate_handler![]` |
| Add ASR provider | src-tauri/src/speech/ | New module + `recognizer.rs` enum variant |
| Change OSC behavior | src-tauri/src/osc/sender.rs | rosc UDP, `/chatbox/input` + `/chatbox/typing` |
| Change audio config | src-tauri/src/audio/capture.rs | cpal stream config, resampling |
| Frontend UI changes | src-ui/src/App.tsx | Single large component → extract subcomponents |
| Config fields | src-tauri/src/config.rs | serde_yaml, add to AppConfig + frontend AppConfig interface |
| E2E tests | tests/ (Python) | `test_e2e.py` main, `test_providers.py` multi-provider |
| E2E server API | src-tauri/src/e2e_server.rs | HTTP endpoints: /health, /devices, /inject_stt, /recording/* |
| i18n strings | src-ui/src/i18n.ts | zh + en maps, add key to both |
| Logging | src-tauri/src/log.rs | `log::info("module", "msg")` — writes to stderr + tmp/app.log |

## CONVENTIONS
- **Config search**: CWD → parent dirs → exe dir → project root (`find_config_file()`)
- **Credentials**: Separate file from main config (`TencentCredentials` vs `AppConfig`)
- **Global state**: `static AtomicBool` / `static Mutex<Option<T>>` in main.rs (no dependency injection)
- **Threading**: `thread::spawn` + `Arc<AtomicBool>` stop signals; one `tokio::runtime::Runtime` per recording
- **Events**: Tauri `emit_all("event-name", payload)` — frontend listens via `app.listen()`
- **Naming**: `src-ui/` not `src/` for frontend; `src-tauri/` contains Rust
- **Dual-mode binary**: `main.rs` checks `--e2e` flag → skips Tauri, runs HTTP server

## ANTI-PATTERNS (THIS PROJECT)
- **NEVER** use `as any`, `@ts-ignore`, `@ts-expect-error` for type issues
- **NEVER** suppress errors with `let _ =` without logging — use `match` with `log::error`
- **NEVER** add `#[cfg(test)]` modules that import from `main.rs` (use `lib.rs` for test-exported symbols)

## UNIQUE STYLES
- **Monolithic main.rs**: 690-line file with inline `fn start_recording_inner`, test recording commands, log buffer, and Tauri commands. Extract if adding significant new features.
- **Custom logging**: `log::info/debug/warn/error` writes to both stderr and `tmp/app.log` (not standard `log` crate)
- **Mixed workspace**: Root Cargo.toml declares workspace [src-tauri, crates/stt-server, stt-gui] but src-tauri/Cargo.toml lists stt-server as path dependency
- **Windows-only hotkey**: `RegisterHotKey` via winapi crate, no platform abstraction
- **4 ASR providers**: Tencent (HMAC-SHA1), Local (WebSocket STT), LocalEmbedded (Sherpa-ONNX), None

## COMMANDS
```bash
# Rust (from project root — workspace aware)
cargo check                            # All workspace crates
cargo check -p vrc-chat-tool           # Main app only
cargo test -p vrc-chat-tool            # 24 unit tests
cargo build -p vrc-chat-tool           # Release build

# Frontend (from project root)
npm install                            # Install dependencies
npm run build                          # TypeScript check + Vite production build
npm run tauri dev                      # Development mode (fails if CLI missing)

# Tauri dev (workaround)
node node_modules/@tauri-apps/cli/tauri.js dev

# E2E tests (from project root)
python tests/test_e2e.py               # Audio pipeline + trigger tests
python tests/test_providers.py         # Multi-provider config validation
```

## NOTES
- **Tauri v1** (not v2) — `tauri.conf.json` uses `"tauri"` top-level key and `"allowlist"` (not `"permissions"`)
- **React 19** + **Vite 8** are bleeding-edge versions paired with Tauri v1 — watch for API conflicts
- **config.yaml** + **stt-config.yaml** exist in both project root AND src-tauri/ (copy needed for runtime CWD)
- **Streaming ASR flow**: `AudioCapture::capture_streaming` → mpsc channel → `recognize_pcm_stream` → partial/sentence callbacks → Tauri events
- **Trigger listener**: Always-on STT websocket for voice commands (start/stop recording). Currently default-disabled due to known issues.
- **stt-gui** is a separate debugging tool, not integrated into the main app. Connects to STT WebSocket service for manual testing.
- **E2E server**: Main app in `--e2e` mode exposes HTTP API for Python tests — no Tauri window. Supports `/inject_stt` for simulated STT input.
