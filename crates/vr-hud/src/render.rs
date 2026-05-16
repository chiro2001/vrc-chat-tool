//! Overlay rendering using fontdue + set_raw_data.
//! Resolution & font sizes scale together via `scale` factor.
//! Base: 1024×256 px, 24px font → in VR ~0.8m wide.

use crate::state::OverlayState;
use fontdue::Font;
use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};
use openvr::overlay::OverlayHandle;

pub struct OverlayRenderer {
    font: Font,
    pixels: Vec<u8>,
    scale: f32,
    tex_w: usize,
    tex_h: usize,
}

impl OverlayRenderer {
    pub fn new(scale: f32) -> anyhow::Result<Self> {
        let font_data = std::fs::read("C:\\Windows\\Fonts\\simhei.ttf")?;
        let font = Font::from_bytes(font_data, fontdue::FontSettings::default())
            .map_err(|e| anyhow::anyhow!("Failed to load font: {}", e))?;
        let tex_w = (1024.0 * scale) as usize;
        let tex_h = (256.0 * scale) as usize;
        let pixels = vec![0u8; tex_w * tex_h * 4];
        Ok(Self { font, pixels, scale, tex_w, tex_h })
    }

    /// Full HUD render (connected state).
    pub fn render_frame(
        &mut self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
        state: &OverlayState,
    ) -> anyhow::Result<()> {
        self.clear_background(false);

        let font_size = 24.0 * self.scale;
        let small_size = 18.0 * self.scale;
        let mut y: f32 = 8.0 * self.scale;

        // Status indicator
        let status_color = match state.status.as_str() {
            "recording" => [100u8, 220, 80, 255],
            "recognizing" => [255, 180, 50, 255],
            _ => [80, 200, 80, 255],
        };
        let status_text = format!("● {}  音量: {:.0}%  模型: {}",
            status_label(&state.status),
            state.volume * 100.0,
            state.model,
        );
        y = self.render_text(&status_text, font_size, 12.0 * self.scale, y, status_color);
        y += 12.0 * self.scale;

        // Separator
        self.draw_separator(y as usize);
        y += 8.0 * self.scale;

        // Current recognition text
        if !state.current_text.is_empty() {
            y = self.render_text(&state.current_text, font_size, 12.0 * self.scale, y, [220, 220, 220, 255]);
            y += 6.0 * self.scale;
        }

        // Last sentence
        if !state.last_sentence.is_empty() {
            let _ = self.render_text(&format!("> {}", state.last_sentence), small_size, 12.0 * self.scale, y, [180, 200, 180, 255]);
        }

        overlay
            .set_raw_data(handle, &self.pixels, self.tex_w, self.tex_h, 4)
            .map_err(|e| anyhow::anyhow!("set_raw_data: {:?}", e))
    }

    /// Minimal disconnected-state render — single line, transparent background.
    pub fn render_disconnected(
        &mut self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
    ) -> anyhow::Result<()> {
        self.clear_background(true);

        let font_size = 20.0 * self.scale;
        self.render_text("等待主程序连接...", font_size, 12.0 * self.scale, 4.0 * self.scale, [180, 180, 180, 200]);

        overlay
            .set_raw_data(handle, &self.pixels, self.tex_w, self.tex_h, 4)
            .map_err(|e| anyhow::anyhow!("set_raw_data: {:?}", e))
    }

    fn clear_background(&mut self, transparent: bool) {
        let alpha = if transparent { 0 } else { 180 };
        for i in 0..self.tex_w * self.tex_h {
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
                    if px >= self.tex_w || py >= self.tex_h { continue; }
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
        if y >= self.tex_h { return; }
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

fn status_label(status: &str) -> &str {
    match status {
        "recording" => "录音中",
        "recognizing" => "识别中",
        "error" => "错误",
        _ => "就绪",
    }
}
