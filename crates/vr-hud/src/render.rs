//! Overlay rendering using fontdue + set_raw_data.
//! Dynamically sizes texture based on content height.
//!
//! Status modes:
//!   stop        — only status line, minimal height
//!   idle        — status + last sentence (if any)
//!   recognizing — status + separator + live text + last sentence (if any)

use crate::state::OverlayState;
use fontdue::Font;
use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};
use openvr::overlay::OverlayHandle;

const BASE_W: usize = 1024;
const MAX_H: usize = 512; // max texture height in pixels

pub struct OverlayRenderer {
    font: Font,
    pixels: Vec<u8>,
    scale: f32,
    tex_w: usize,
}

impl OverlayRenderer {
    pub fn new(scale: f32) -> anyhow::Result<Self> {
        let font_data = std::fs::read("C:\\Windows\\Fonts\\simhei.ttf")?;
        let font = Font::from_bytes(font_data, fontdue::FontSettings::default())
            .map_err(|e| anyhow::anyhow!("Failed to load font: {}", e))?;
        let mut r = Self { font, pixels: Vec::new(), scale: 0.0, tex_w: 0 };
        r.set_scale(scale);
        Ok(r)
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.tex_w = (BASE_W as f32 * scale) as usize;
        self.pixels = vec![0u8; self.tex_w * MAX_H * 4];
    }

    /// Full HUD render (connected state).
    pub fn render_frame(
        &mut self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
        state: &OverlayState,
    ) -> anyhow::Result<()> {
        let font_size = 48.0 * self.scale;
        let sep_pad = 4.0 * self.scale;

        self.clear_background(false);

        let mut y: f32 = 4.0 * self.scale;

        // ═══ Status line (always shown) ═══
        let status_color = match state.status.as_str() {
            "recording" => [100u8, 220, 80, 255],
            "recognizing" => [255, 180, 50, 255],
            "stop" => [120, 120, 140, 200],
            _ => [80, 200, 80, 255],
        };
        let label = match state.status.as_str() {
            "stop" => "● 未录音",
            "recording" => "● 录音中",
            "recognizing" => "● 识别中",
            _ => "● 就绪",
        };
        let status_text = format!("{}    后端: {}",
            label, state.model);
        y = self.render_text(&status_text, font_size, 12.0 * self.scale, y, status_color);

        // If stop — nothing else to render
        if state.status == "stop" {
            let content_h = (y + 4.0 * self.scale) as usize;
            return self.upload(overlay, handle, content_h);
        }

        y += sep_pad;
        self.draw_separator(y as usize);
        y += sep_pad;

        // ═══ Main content — single line, sentence has priority ═══
        if !state.last_sentence.is_empty() {
            // Finalized sentence (OSC result) — colored with ">" prefix
            let colored = format!("> {}", state.last_sentence);
            y = self.render_text(&colored, font_size, 12.0 * self.scale, y, [120, 220, 120, 255]);
        } else if !state.current_text.is_empty() {
            // Live recognition — white text while waiting for sentence
            y = self.render_text(&state.current_text, font_size, 12.0 * self.scale, y, [255, 255, 255, 255]);
        }

        let content_h = (y + 4.0 * self.scale) as usize;
        self.upload(overlay, handle, content_h)
    }

    /// Minimal disconnected-state render.
    pub fn render_disconnected(
        &mut self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
    ) -> anyhow::Result<()> {
        self.clear_background(true);
        let font_size = 40.0 * self.scale;
        let y = self.render_text("等待主程序连接...", font_size, 12.0 * self.scale, 4.0 * self.scale, [180, 180, 180, 200]);
        let content_h = (y + 4.0 * self.scale) as usize;
        self.upload(overlay, handle, content_h)
    }

    /// Upload rendered content to overlay at fixed MAX_H height.
    /// Extra rows are filled transparent to prevent visual artifacts.
    fn upload(
        &mut self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
        content_h: usize,
    ) -> anyhow::Result<()> {
        // Fill unused rows with transparent to avoid stale content
        for i in content_h.min(MAX_H)..MAX_H {
            let row_start = i * self.tex_w * 4;
            for j in 0..self.tex_w {
                self.pixels[row_start + j * 4 + 3] = 0;
            }
        }
        let total_bytes = self.tex_w * MAX_H * 4;
        let slice = &self.pixels[..total_bytes];
        overlay
            .set_raw_data(handle, slice, self.tex_w, MAX_H, 4)
            .map_err(|e| anyhow::anyhow!("set_raw_data: {:?}", e))
    }

    fn clear_background(&mut self, transparent: bool) {
        let alpha = if transparent { 0 } else { 180 };
        for i in 0..self.tex_w * MAX_H {
            self.pixels[i * 4 + 0] = 20;
            self.pixels[i * 4 + 1] = 20;
            self.pixels[i * 4 + 2] = 30;
            self.pixels[i * 4 + 3] = alpha;
        }
    }

    fn render_text(&mut self, text: &str, size: f32, x: f32, y: f32, color: [u8; 4]) -> f32 {
        let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
        layout.reset(&LayoutSettings {
            x,
            y,
            max_width: Some(self.tex_w as f32 - x - 8.0),
            ..LayoutSettings::default()
        });
        layout.append(&[&self.font], &TextStyle::new(text, size, 0));

        for glyph in layout.glyphs() {
            let (_, bitmap) = self.font.rasterize(glyph.parent, size);
            let gx = glyph.x as usize;
            let gy = glyph.y as usize;
            let gw = glyph.width as usize;
            let gh = glyph.height as usize;

            for row in 0..gh {
                for col in 0..gw {
                    let alpha = bitmap[row * gw + col];
                    if alpha == 0 { continue; }
                    let px = gx + col;
                    let py = gy + row;
                    if px >= self.tex_w || py >= MAX_H { continue; }
                    let i = py * self.tex_w + px;
                    let a = alpha as f32 / 255.0;
                    let bg = &mut self.pixels[i * 4..i * 4 + 3];
                    bg[0] = (bg[0] as f32 * (1.0 - a) + color[0] as f32 * a) as u8;
                    bg[1] = (bg[1] as f32 * (1.0 - a) + color[1] as f32 * a) as u8;
                    bg[2] = (bg[2] as f32 * (1.0 - a) + color[2] as f32 * a) as u8;
                    self.pixels[i * 4 + 3] = 255;
                }
            }
        }

        y + layout.height() as f32
    }

    fn draw_separator(&mut self, y: usize) {
        if y >= MAX_H { return; }
        let i = y * self.tex_w;
        for x in 0..self.tex_w {
            let p = (i + x) * 4;
            self.pixels[p + 0] = 80;
            self.pixels[p + 1] = 80;
            self.pixels[p + 2] = 90;
            self.pixels[p + 3] = 200;
        }
    }
}
