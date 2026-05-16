//! Shared overlay state — what the HUD displays.

use crate::ipc::OverlayMessage;

pub struct OverlayState {
    pub status: String,
    pub current_text: String,
    pub last_sentence: String,
    pub volume: f32,
    pub model: String,
    pub visible: bool,
    // Config
    pub opacity: f32,
    pub scale: f32,
    pub smoothing: f32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            status: "idle".into(),
            current_text: String::new(),
            last_sentence: String::new(),
            volume: 0.0,
            model: "sherpa-onnx".into(),
            visible: true,
            opacity: 0.85,
            scale: 1.0,
            smoothing: 0.10,
            pos_x: -0.4,
            pos_y: 0.3,
            pos_z: -1.5,
        }
    }
}

impl OverlayState {
    /// Apply partial update — only overwrite fields that are Some.
    pub fn update(&mut self, msg: &OverlayMessage) {
        if let Some(ref s) = msg.status { self.status = s.clone(); }
        if let Some(ref t) = msg.text { self.current_text = t.clone(); }
        if let Some(ref s) = msg.sentence { self.last_sentence = s.clone(); }
        if let Some(v) = msg.volume { self.volume = v; }
        if let Some(ref m) = msg.model { self.model = m.clone(); }
        if let Some(v) = msg.visible { self.visible = v; }
    }

    /// Apply config update.
    pub fn apply_config(&mut self, msg: &OverlayMessage) {
        if let Some(v) = msg.opacity { self.opacity = v; }
        if let Some(v) = msg.scale { self.scale = v; }
        if let Some(v) = msg.smoothing { self.smoothing = v; }
        if let Some(v) = msg.pos_x { self.pos_x = v; }
        if let Some(v) = msg.pos_y { self.pos_y = v; }
        if let Some(v) = msg.pos_z { self.pos_z = v; }
    }
}
