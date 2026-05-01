# crates/stt-server/ — Sherpa-ONNX STT Engine

## OVERVIEW
Standalone Rust crate wrapping sherpa-rs for in-process speech-to-text. Provides library API (SttEngine) and optional HTTP server binary.

## STRUCTURE
```
crates/stt-server/
├── Cargo.toml
└── src/
    ├── lib.rs            # Public exports
    ├── config.rs         # Config struct (YAML deserialization)
    ├── engine.rs         # SttEngine — OnlineRecognizer wrapper
    ├── server.rs         # Optional HTTP/WebSocket server
    ├── download.rs       # Model download helper
    └── main.rs           # Binary entry (HTTP server mode)
```

## WHERE TO LOOK
| Task | File | Key API |
|------|------|---------|
| Load model config | config.rs | `Config::from_file(path) -> Result<Config>` |
| Create engine | engine.rs | `SttEngine::new(config) -> Result<SttEngine>` |
| Create stream | engine.rs | `create_stream() -> Stream` (one per session) |
| Decode audio | engine.rs | `decode(stream, samples: &[f32])` |
| Check endpoint | engine.rs | `is_endpoint(stream) -> bool` |
| Get recognized text | engine.rs | `get_text(stream) -> String` |
| Reset stream | engine.rs | `reset(stream)` |
| Signal end of input | engine.rs | `input_finished(stream)` |
| Add punctuation | engine.rs | `add_punctuation(text) -> String` |
| Run HTTP server | main.rs | Binary entry, WebSocket endpoint for audio streaming |

## CONVENTIONS
- **Config format**: YAML with model_dir, encoder/decoder/joiner/tokens paths, VAD config, punctuation model
- **Sample rate**: Always 16000 Hz (required by sherpa-onnx)
- **Thread safety**: `SttEngine` is `Send + Sync` (wrapped in `Arc`)
- **Stream lifecycle**: Create → decode → is_endpoint → get_text → reset (or input_finished for final)

## NOTES
- Depends on sherpa-rs (heavy dependency, downloads ONNX model files)
- HTTP server uses actix-web + WebSocket for streaming audio
- Model files NOT committed — download via `download.rs` or manual placement
