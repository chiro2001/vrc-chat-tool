//! STT Service Test GUI — eframe / egui desktop app.
//!
//! Connect to a streaming STT WebSocket server, send microphone or WAV file
//! audio, and display real-time recognition results.
//!
//! Mirrors the Python `stt-gui/stt_gui.py` CustomTkinter app.
//!
//! ## Test mode (for E2E automation)
//!
//! Pass `--test-timeout <seconds>` to run in headless test mode:
//!   - Auto-starts recording on launch
//!   - Collects recognition segments
//!   - Saves results as JSON to `--test-output <path>` (default: `tmp/stt_test_result.json`)
//!   - Exits after timeout or when recording finishes
//!
//! Example:
//!   stt-gui --test-timeout 15 --url ws://127.0.0.1:8765 --device "CABLE" --test-output tmp/result.json

mod app;
mod audio;
mod worker;

use app::{SttGuiApp, TestConfig};
use clap::Parser;

/// STT Service Test GUI — eframe / egui desktop app for WebSocket STT debugging.
#[derive(Parser, Debug)]
#[command(name = "stt-gui")]
struct Args {
    /// STT server WebSocket URL
    #[arg(long, default_value = "ws://127.0.0.1:8765")]
    url: String,

    /// Audio input device name (substring match, e.g. "CABLE" for VB-Cable)
    #[arg(long)]
    device: Option<String>,

    /// Audio source type for test mode: "mic" (default) or "file"
    #[arg(long, default_value = "mic")]
    source: String,

    /// WAV file path for --source file mode
    #[arg(long)]
    wav: Option<String>,

    /// Enable test mode with auto-recording and timeout (in seconds)
    #[arg(long, value_name = "SECONDS")]
    test_timeout: Option<u64>,

    /// Path to save test results JSON (default: tmp/stt_test_result.json)
    #[arg(long, default_value = "tmp/stt_test_result.json")]
    test_output: String,
}

fn main() -> Result<(), eframe::Error> {
    let args = Args::parse();

    let test_config = args.test_timeout.map(|timeout_secs| TestConfig {
        url: args.url.clone(),
        device: args.device.clone(),
        source: args.source.clone(),
        wav_path: args.wav.clone(),
        timeout_secs,
        output_path: args.test_output.clone(),
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 820.0])
            .with_min_inner_size([560.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "STT Service Test Tool",
        options,
        Box::new(move |_cc| {
            let app = if let Some(ref tc) = test_config {
                let mut a = SttGuiApp::default();
                a.set_test_config(tc.clone());
                // Override server URL from CLI
                a.set_server_url(tc.url.clone());
                a
            } else if args.url != "ws://127.0.0.1:8765" {
                let mut a = SttGuiApp::default();
                a.set_server_url(args.url.clone());
                a
            } else {
                SttGuiApp::default()
            };
            Ok(Box::new(app))
        }),
    )
}
