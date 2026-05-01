# src-tauri/src/ — Rust Backend

## OVERVIEW
Core application logic. Audio capture → ASR processing → OSC output. Tauri command handlers expose functionality to React frontend.

## STRUCTURE
```
src-tauri/src/
├── main.rs              # Tauri entry, 8 commands, event wiring, global state (690 lines)
├── lib.rs               # `pub mod` declarations for all submodules
├── config.rs            # AppConfig + TencentCredentials (serde_yaml)
├── audio/
│   └── capture.rs       # cpal AudioCapture, resampling, PCM conversion, WAV headers
├── speech/
│   ├── tencent.rs       # Tencent Cloud ASR V1 HMAC-SHA1 signing + URL builder
│   ├── streaming.rs     # StreamingRecognizer (WebSocket, recognize_pcm_stream)
│   ├── local.rs         # LocalRecognizer (WebSocket to remote STT server)
│   ├── local_embedded.rs# LocalEmbeddedRecognizer (Sherpa-ONNX, in-process)
│   ├── recognizer.rs    # Recognizer enum (Tencent | Local | LocalEmbedded)
│   └── mod.rs
├── osc/
│   └── sender.rs        # OscSender: send_chatbox, send_typing, clear_chatbox, multi-line buffer
├── history.rs           # SQLite recognition history (rusqlite, bundled)
├── trigger.rs           # Always-on STT trigger listener for voice commands
├── log.rs               # File logger (stderr + tmp/app.log)
├── hotkey.rs            # Win32 F10 global hotkey (RegisterHotKey)
├── e2e_server.rs        # HTTP API server (actix-web) for E2E tests
└── bin/
    └── test_e2e.rs      # Standalone E2E test binary
```

## WHERE TO LOOK
| Task | File | Function/Symbol |
|------|------|-----------------|
| Add Tauri command | main.rs | Add `#[tauri::command]` fn + register in `generate_handler![]` |
| Start/stop recording | main.rs | `start_recording_inner()`, `stop_recording()` |
| Recording pipeline | main.rs:266-478 | `start_recording_inner()` — spawns capture thread + tokio ASR |
| Audio capture | audio/capture.rs | `AudioCapture::capture_streaming(on_chunk, stop_signal)` |
| Tencent ASR auth | speech/tencent.rs | `generate_signature()` — HMAC-SHA1, Base64 |
| ASR provider dispatch | speech/recognizer.rs | `Recognizer` enum, `recognize_pcm_stream()` |
| OSC message format | osc/sender.rs | `send_chatbox(text)` → `/chatbox/input` [string, bool] |
| Config YAML keys | config.rs | `AppConfig` struct (snake_case in YAML) |
| Trigger phrase detect | trigger.rs | `matches_trigger(text, phrase)` — punctuation-tolerant |
| E2E HTTP endpoints | e2e_server.rs | /health, /devices, /inject_stt, /recording/start, /recording/stop |
| Log to file | log.rs | `log::info/debug/warn/error(module, message)` |

## CONVENTIONS
- **Global state**: `static ATOMIC_BOOL` + `static Mutex<Option<T>>` at top of main.rs
- **Thread communication**: `Arc<AtomicBool>` for stop signals, `tokio::sync::mpsc` for PCM chunks
- **Error handling**: `anyhow::Result` internally, `Result<T, String>` for Tauri commands
- **Event naming**: kebab-case Tauri events: `recording-started`, `recording-partial`, `recording-complete`, `trigger-heard`, `log-entry`
- **Config loading**: `AppConfig::load()` searches CWD → parents → exe dir → project root
- **Credentials**: Separate `TencentCredentials::load()` from `AppConfig`

## ANTI-PATTERNS
- **NEVER** `let _ =` discard errors in thread handlers — `match` + `log::error` + `emit_all("recording-error")`
- **NEVER** mutate `SHOULD_STOP` from capture callback directly — use `store(true, SeqCst)`
- **NEVER** add `pub mod` in lib.rs without verifying the module compiles as a lib target
