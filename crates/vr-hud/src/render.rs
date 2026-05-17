//! Overlay rendering using fontdue CPU rasterization + D3D11 GPU texture.
//!
//! Fontdue renders to a CPU buffer. On each content change:
//!   1. Map the D3D11 dynamic texture
//!   2. Copy CPU pixels -> GPU memory
//!   3. Unmap
//!   4. Call SetOverlayTexture (IVROverlay_027 via openvr_sys::VR_GetGenericInterface)
//!
//! This replaces set_raw_data (which flickers aggressively with dynamic content).

use crate::state::OverlayState;
use fontdue::Font;
use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};
use openvr::overlay::OverlayHandle;
use std::ffi::c_void;

use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D;
use windows::Win32::Graphics::Dxgi::Common;
use windows::Win32::Foundation::HMODULE;
use windows::core::Interface;

use openvr_sys;

const BASE_W: usize = 1024;
const MAX_H: usize = 128;

pub struct OverlayRenderer {
    font: Font,
    pixels: Vec<u8>,
    scale: f32,
    tex_w: usize,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    texture: Option<ID3D11Texture2D>,
}

fn create_d3d11_device() -> anyhow::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;

    let hr = unsafe {
        D3D11CreateDevice(
            None,
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    };
    hr.map_err(|e| anyhow::anyhow!("D3D11CreateDevice failed: {e}"))?;

    let device = device.ok_or_else(|| anyhow::anyhow!("D3D11CreateDevice returned null device"))?;
    let context = context.ok_or_else(|| anyhow::anyhow!("D3D11CreateDevice returned null context"))?;
    Ok((device, context))
}

fn create_dynamic_texture(device: &ID3D11Device, width: u32, height: u32) -> anyhow::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: Common::DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
    };

    let mut texture: Option<ID3D11Texture2D> = None;
    unsafe {
        let hr = device.CreateTexture2D(&desc, None, Some(&mut texture));
        hr.map_err(|e| anyhow::anyhow!("CreateTexture2D failed: {e}"))?;
    }
    texture.ok_or_else(|| anyhow::anyhow!("CreateTexture2D returned null"))
}

impl OverlayRenderer {
    pub fn new(scale: f32) -> anyhow::Result<Self> {
        let font_data = std::fs::read("C:\\Windows\\Fonts\\simhei.ttf")?;
        let font = Font::from_bytes(font_data, fontdue::FontSettings::default())
            .map_err(|e| anyhow::anyhow!("Failed to load font: {}", e))?;

        let (device, context) = create_d3d11_device()?;

        let mut r = Self {
            font, pixels: Vec::new(), scale: 0.0, tex_w: 0,
            device, context, texture: None,
        };
        r.set_scale(scale);
        Ok(r)
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.tex_w = (BASE_W as f32 * scale) as usize;
        self.pixels = vec![0u8; self.tex_w * MAX_H * 4];

        self.texture = match create_dynamic_texture(&self.device, self.tex_w as u32, MAX_H as u32) {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!("Failed to recreate D3D11 texture at scale {scale}: {e}");
                None
            }
        };
    }

    pub fn render_frame(
        &mut self,
        _overlay: &openvr::Overlay,
        handle: OverlayHandle,
        state: &OverlayState,
    ) -> anyhow::Result<()> {
        // CPU rasterization - same as before
        let bar_h = (56.0 * self.scale) as usize;
        self.fill_rect(0, 0, self.tex_w, bar_h.min(MAX_H));
        self.fill_rect(0, bar_h, self.tex_w, MAX_H - bar_h);

        let font_size = 48.0 * self.scale;
        let sep_pad = 4.0 * self.scale;
        let mut y: f32 = 4.0 * self.scale;

        let status_color = match state.status.as_str() {
            "recording" => [100u8, 220, 80, 255],
            "recognizing" => [255, 180, 50, 255],
            "stop" => [120, 120, 140, 200],
            _ => [80, 200, 80, 255],
        };
        let label = match state.status.as_str() {
            "stop" => "\u{25cf} 未录音",
            "recording" => "\u{25cf} 录音中",
            "recognizing" => "\u{25cf} 识别中",
            _ => "\u{25cf} 就绪",
        };
        let status_text = format!("{}    后端: {}", label, state.model);
        y = self.render_text(&status_text, font_size, 12.0 * self.scale, y, status_color);

        if state.status == "stop" {
            return self.upload_and_present(handle, (y + 4.0 * self.scale) as usize);
        }

        y += sep_pad;
        self.draw_separator(y as usize);
        y += sep_pad;

        let content_top = y as usize;
        let content_h = font_size as usize + 8;
        self.fill_rect(0, content_top, self.tex_w, content_h.min(MAX_H.saturating_sub(content_top)));

        if !state.last_sentence.is_empty() {
            let colored = format!("> {}", state.last_sentence);
            y = self.render_text(&colored, font_size, 12.0 * self.scale, y, [120, 220, 120, 255]);
        } else if !state.current_text.is_empty() {
            y = self.render_text(&state.current_text, font_size, 12.0 * self.scale, y, [255, 255, 255, 255]);
        }

        self.upload_and_present(handle, (y + 4.0 * self.scale) as usize)
    }

    pub fn render_disconnected(
        &mut self,
        _overlay: &openvr::Overlay,
        handle: OverlayHandle,
    ) -> anyhow::Result<()> {
        self.fill_rect_transparent(0, 0, self.tex_w, MAX_H);
        let font_size = 40.0 * self.scale;
        let y = self.render_text("等待主程序连接...", font_size, 12.0 * self.scale, 4.0 * self.scale, [180, 180, 180, 200]);
        self.upload_and_present(handle, (y + 4.0 * self.scale) as usize)
    }

    // ---- CPU rasterization ----

    fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize) {
        let max_y = (y + h).min(MAX_H);
        let max_x = (x + w).min(self.tex_w);
        for row in y..max_y {
            let start = row * self.tex_w + x;
            let end = (row * self.tex_w + max_x).min(self.pixels.len() / 4);
            for i in start..end {
                self.pixels[i * 4 + 0] = 20;
                self.pixels[i * 4 + 1] = 20;
                self.pixels[i * 4 + 2] = 30;
                self.pixels[i * 4 + 3] = 180;
            }
        }
    }

    fn fill_rect_transparent(&mut self, x: usize, y: usize, w: usize, h: usize) {
        let max_y = (y + h).min(MAX_H);
        let max_x = (x + w).min(self.tex_w);
        for row in y..max_y {
            let start = row * self.tex_w + x;
            let end = (row * self.tex_w + max_x).min(self.pixels.len() / 4);
            for i in start..end {
                self.pixels[i * 4 + 3] = 0;
            }
        }
    }

    fn render_text(&mut self, text: &str, size: f32, x: f32, y: f32, color: [u8; 4]) -> f32 {
        let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
        layout.reset(&LayoutSettings {
            x, y,
            max_width: Some(self.tex_w as f32 - x - 8.0),
            ..LayoutSettings::default()
        });
        layout.append(&[&self.font], &TextStyle::new(text, size, 0));

        for glyph in layout.glyphs() {
            let (_, bitmap) = self.font.rasterize(glyph.parent, size);
            let (gx, gy, gw, gh) = (glyph.x as usize, glyph.y as usize, glyph.width as usize, glyph.height as usize);
            for row in 0..gh {
                for col in 0..gw {
                    let alpha = bitmap[row * gw + col];
                    if alpha == 0 { continue; }
                    let (px, py) = (gx + col, gy + row);
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

    // ---- D3D11 upload + SetOverlayTexture ----

    fn upload_and_present(&self, handle: OverlayHandle, content_h: usize) -> anyhow::Result<()> {
        let h = content_h.clamp(64, MAX_H);
        let texture = self.texture.as_ref()
            .ok_or_else(|| anyhow::anyhow!("D3D11 texture not initialized"))?;

        let total_bytes = self.tex_w * h * 4;
        let slice = &self.pixels[..total_bytes];

        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(
                texture, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped),
            ).map_err(|e| anyhow::anyhow!("D3D11 Map failed: {e}"))?;

            let src_row = self.tex_w * 4;
            let dst_row = mapped.RowPitch as usize;
            let dst = mapped.pData as *mut u8;
            for row in 0..h {
                std::ptr::copy_nonoverlapping(
                    slice.as_ptr().add(row * src_row),
                    dst.add(row * dst_row),
                    src_row,
                );
            }

            self.context.Unmap(texture, 0);
        }

        let tex = openvr_sys::Texture_t {
            handle: texture.as_raw() as *mut c_void,
            eType: openvr_sys::ETextureType_TextureType_DirectX,
            eColorSpace: openvr_sys::EColorSpace_ColorSpace_Auto,
        };

        self.call_set_overlay_texture(handle, &tex)
    }

    fn call_set_overlay_texture(&self, handle: OverlayHandle, tex: &openvr_sys::Texture_t) -> anyhow::Result<()> {
        let magic = b"FnTable:IVROverlay_027\0";
        let mut error = 0i32;
        let ptr = unsafe {
            openvr_sys::VR_GetGenericInterface(magic.as_ptr() as *const i8, &mut error)
        };
        if error != 0 || ptr == 0 {
            anyhow::bail!("VR_GetGenericInterface(IVROverlay_027) failed: error={error}");
        }

        let table: &openvr_sys::VR_IVROverlay_FnTable = unsafe { &*(ptr as *const _) };
        let set_texture = table.SetOverlayTexture
            .ok_or_else(|| anyhow::anyhow!("SetOverlayTexture fn ptr is None"))?;

        let err: i32 = unsafe {
            set_texture(handle.0, tex as *const openvr_sys::Texture_t as *mut _)
        };
        if err != 0 {
            anyhow::bail!("SetOverlayTexture failed with error code {}", err);
        }
        Ok(())
    }
}
