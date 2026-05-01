# stt-gui/ — STT Debugging GUI

## OVERVIEW
Standalone desktop GUI client for testing STT WebSocket services. Built with eframe/egui. Not integrated into the main Tauri app.

## STRUCTURE
```
stt-gui/
├── Cargo.toml
└── src/
    ├── main.rs           # eframe app entry
    ├── app.rs            # Egui UI layout + state
    ├── audio.rs          # cpal microphone capture
    └── worker.rs         # WebSocket STT client
```

## WHERE TO LOOK
| Task | File | Notes |
|------|------|-------|
| UI layout | app.rs | Egui panels: controls, results, log |
| Mic capture | audio.rs | cpal input stream → PCM chunks |
| WS communication | worker.rs | Connect to STT URL, send audio, receive text |
| WAV playback | audio.rs | Read WAV file, stream as simulated mic input |

## CONVENTIONS
- **Purpose**: Manual debugging tool — connect to remote TTS or local sherpa-onnx server
- **Naming conflict**: `stt-gui` is unrelated to `crates/stt-server` despite naming similarity
- **Dependencies**: eframe, egui, cpal, tokio-tungstenite — separate dependency tree from main app

## NOTES
- NOT part of the main application build
- Runs independently: `cargo run -p stt-gui`
- Connects to any WebSocket STT service (not just stt-server)
- Useful for testing STT service availability and quality without launching full Tauri app
