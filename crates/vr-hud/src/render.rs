//! Overlay rendering using fontdue + set_raw_data.
//! Resolution & font sizes scale together via SCALE factor.
//! Base: 1024×256 px, 24px font → in VR ~0.8m wide.

const SCALE: f32 = 1.0;
const TEX_W: usize = (1024.0 * SCALE) as usize;
const TEX_H: usize = (256.0 * SCALE) as usize;
const FONT_SIZE: f32 = 24.0 * SCALE;
const SMALL_SIZE: f32 = 18.0 * SCALE;

use crate::state::OverlayState;
use fontdue::Font;
use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};
use openvr::overlay::OverlayHandle;

pub struct OverlayRenderer {
    font: Font,
    pixels: Vec<u8>,
}

impl OverlayRenderer {
    pub fn new() -> anyhow::Result<Self> {
        let font_data = std::fs::read("C:\\Windows\\Fonts\\simhei.ttf")?;
        let font = Font::from_bytes(font_data, fontdue::FontSettings::default())
            .map_err(|e| anyhow::anyhow!("Failed to load font: {}", e))?;
        let pixels = vec![0u8; TEX_W * TEX_H * 4];
        Ok(Self { font, pixels })
    }

    /// Render one frame of the overlay and submit to OpenVR.
    pub fn render_frame(
        &mut self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
        state: &OverlayState,
    ) -> anyhow::Result<()> {
        // Clear background
        self.clear_background();

        let mut y: f32 = 8.0;

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
        y = self.render_text(&status_text, FONT_SIZE, 12.0, y, status_color);
        y += 12.0;

        // Separator
        self.draw_separator(y as usize);
        y += 8.0;

        // Current recognition text
        if !state.current_text.is_empty() {
            y = self.render_text(&state.current_text, FONT_SIZE, 12.0, y, [220, 220, 220, 255]);
            y += 6.0;
        }

        // Last sentence
        if !state.last_sentence.is_empty() {
            let _ = self.render_text(&format!("> {}", state.last_sentence), SMALL_SIZE, 12.0, y, [180, 200, 180, 255]);
        }

        // Upload to overlay
        overlay
            .set_raw_data(handle, &self.pixels, TEX_W, TEX_H, 4)
            .map_err(|e| anyhow::anyhow!("set_raw_data: {:?}", e))
    }

    fn clear_background(&mut self) {
        for i in 0..TEX_W * TEX_H {
            self.pixels[i * 4 + 0] = 20;
            self.pixels[i * 4 + 1] = 20;
            self.pixels[i * 4 + 2] = 30;
            self.pixels[i * 4 + 3] = 180;
        }
    }

    fn render_text(&mut self, text: &str, size: f32, x: f32, y: f32, color: [u8; 4]) -> f32 {
        let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
        layout.reset(&LayoutSettings {
            x,
            y,
            max_width: Some(TEX_W as f32 - x - 8.0),
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
                    if px >= TEX_W || py >= TEX_H { continue; }
                    let i = py * TEX_W + px;
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
        if y >= TEX_H { return; }
        let i = y * TEX_W;
        for x in 0..TEX_W {
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
