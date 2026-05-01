//! STT Service Test GUI — eframe / egui desktop app.
//!
//! Connect to a streaming STT WebSocket server, send microphone or WAV file
//! audio, and display real-time recognition results.
//!
//! Mirrors the Python `stt-gui/stt_gui.py` CustomTkinter app.

mod app;
mod audio;
mod worker;

use app::SttGuiApp;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 820.0])
            .with_min_inner_size([560.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "STT Service Test Tool",
        options,
        Box::new(|_cc| Ok(Box::new(SttGuiApp::default()))),
    )
}
