//! Shared overlay state — what the HUD displays.

use crate::ipc::OverlayMessage;

pub struct OverlayState {
    pub status: String,
    pub current_text: String,
    pub last_sentence: String,
    pub volume: f32,
    pub model: String,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            status: "idle".into(),
            current_text: String::new(),
            last_sentence: String::new(),
            volume: 0.0,
            model: "sherpa-onnx".into(),
        }
    }
}

impl OverlayState {
    pub fn update(&mut self, msg: OverlayMessage) {
        self.status = msg.status;
        self.current_text = msg.text;
        if !msg.sentence.is_empty() {
            self.last_sentence = msg.sentence;
        }
        self.volume = msg.volume;
        if !msg.model.is_empty() {
            self.model = msg.model;
        }
    }
}
