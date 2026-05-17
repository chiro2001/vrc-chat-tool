//! Overlay rendering using fontdue CPU rasterization.
//!
//! Two backends (auto-detected at startup):
//!   D3D11  — GPU texture upload via Map/Unmap + SetOverlayTexture (flicker-free)
//!   RawData — set_raw_data fallback (works everywhere, may flicker)
//!
//! Detection logic: try VR_GetGenericInterface("IVROverlay_027") → check SetOverlayTexture fn ptr → create D3D11 device
//! If any step fails, fall back to RawData.

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

#[derive(Clone, Copy, PartialEq)]
enum Backend {
    D3D11,
    RawData,
}

pub struct OverlayRenderer {
    font: Font,
    pixels: Vec<u8>,
    scale: f32,
    tex_w: usize,
    backend: Backend,
    // D3D11 fields (only valid when backend == D3D11)
    device: Option<ID3D11Device>,
    context: Option<ID3D11DeviceContext>,
    texture: Option<ID3D11Texture2D>,
    overlay_fn_table: Option<&'static openvr_sys::VR_IVROverlay_FnTable>,
}

// ── Backend detection ──

fn try_get_overlay_fn_table() -> anyhow::Result<&'static openvr_sys::VR_IVROverlay_FnTable> {
    let magic = b"FnTable:IVROverlay_027\0";
    let mut error = 0i32;
    let ptr = unsafe { openvr_sys::VR_GetGenericInterface(magic.as_ptr() as *const i8, &mut error) };
    if error != 0 || ptr == 0 {
        anyhow::bail!("VR_GetGenericInterface(IVROverlay_027) failed (error={error})");
    }
    Ok(unsafe { &*(ptr as *const _) })
}

// ── D3D11 helpers (only compiled/used for D3D11 backend) ──

fn create_d3d11_device() -> anyhow::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let hr = unsafe {
        D3D11CreateDevice(
            None, Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None, D3D11_SDK_VERSION,
            Some(&mut device), None, Some(&mut context),
        )
    };
    hr.map_err(|e| anyhow::anyhow!("D3D11CreateDevice: {e}"))?;
    let device = device.ok_or_else(|| anyhow::anyhow!("null device"))?;
    let context = context.ok_or_else(|| anyhow::anyhow!("null context"))?;
    Ok((device, context))
}

fn create_dynamic_texture(device: &ID3D11Device, w: u32, h: u32) -> anyhow::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w, Height: h, MipLevels: 1, ArraySize: 1,
        Format: Common::DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 2, // D3D11_RESOURCE_MISC_SHARED — required for cross-process sharing
    };
    let mut texture: Option<ID3D11Texture2D> = None;
    unsafe {
        device.CreateTexture2D(&desc, None, Some(&mut texture))
            .map_err(|e| anyhow::anyhow!("CreateTexture2D: {e}"))?;
    }
    texture.ok_or_else(|| anyhow::anyhow!("null texture"))
}

// ── Renderer ──

impl OverlayRenderer {
    pub fn new(scale: f32) -> anyhow::Result<Self> {
        let font_data = std::fs::read("C:\\Windows\\Fonts\\simhei.ttf")?;
        let font = Font::from_bytes(font_data, fontdue::FontSettings::default())
            .map_err(|e| anyhow::anyhow!("font: {e}"))?;

        // Probe: is SetOverlayTexture available?
        let mut device = None;
        let mut context = None;
        let texture = None;
        let mut fn_table = None;
        let backend;

        match try_get_overlay_fn_table() {
            Ok(table) if table.SetOverlayTexture.is_some() => {
                match create_d3d11_device() {
                    Ok((d, c)) => {
                        tracing::info!("Backend: D3D11 (SetOverlayTexture)");
                        device = Some(d);
                        context = Some(c);
                        fn_table = Some(table);
                        backend = Backend::D3D11;
                    }
                    Err(e) => {
                        tracing::warn!("D3D11 device failed ({e}), falling back to set_raw_data");
                        backend = Backend::RawData;
                    }
                }
            }
            Ok(_) => {
                tracing::warn!("SetOverlayTexture fn ptr is None, falling back to set_raw_data");
                backend = Backend::RawData;
            }
            Err(e) => {
                tracing::warn!("IVROverlay_027 not available ({e}), falling back to set_raw_data");
                backend = Backend::RawData;
            }
        }

        let mut r = Self {
            font, pixels: Vec::new(), scale: 0.0, tex_w: 0,
            backend, device, context, texture, overlay_fn_table: fn_table,
        };
        r.set_scale(scale);
        Ok(r)
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.tex_w = ((BASE_W as f32 * scale) as usize).max(1); // clamp: prevent overflow
        self.pixels = vec![0u8; self.tex_w * MAX_H * 4];

        if self.backend == Backend::D3D11 {
            if let Some(ref device) = self.device {
                self.texture = create_dynamic_texture(device, self.tex_w as u32, MAX_H as u32).ok();
                if self.texture.is_none() {
                    tracing::warn!("D3D11 texture recreation failed at scale {scale}");
                }
            }
        }
    }

    // ── Public render API ──

    pub fn render_frame(
        &mut self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
        state: &OverlayState,
    ) -> anyhow::Result<()> {
        // CPU rasterization (identical for both backends)
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
            "stop" => "\u{25cf} 未录音", "recording" => "\u{25cf} 录音中",
            "recognizing" => "\u{25cf} 识别中", _ => "\u{25cf} 就绪",
        };
        y = self.render_text(&format!("{}    后端: {}", label, state.model), font_size, 12.0 * self.scale, y, status_color);

        if state.status == "stop" {
            return self.present(overlay, handle, (y + 4.0 * self.scale) as usize);
        }

        y += sep_pad;
        self.draw_separator(y as usize);
        y += sep_pad;

        let content_top = y as usize;
        let content_h = font_size as usize + 8;
        self.fill_rect(0, content_top, self.tex_w, content_h.min(MAX_H.saturating_sub(content_top)));

        if !state.last_sentence.is_empty() {
            y = self.render_text(&format!("> {}", state.last_sentence), font_size, 12.0 * self.scale, y, [120, 220, 120, 255]);
        } else if !state.current_text.is_empty() {
            y = self.render_text(&state.current_text, font_size, 12.0 * self.scale, y, [255, 255, 255, 255]);
        }

        self.present(overlay, handle, (y + 4.0 * self.scale) as usize)
    }

    pub fn render_disconnected(
        &mut self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
    ) -> anyhow::Result<()> {
        self.fill_rect_transparent(0, 0, self.tex_w, MAX_H);
        let y = self.render_text("等待主程序连接...", 40.0 * self.scale, 12.0 * self.scale, 4.0 * self.scale, [180, 180, 180, 200]);
        self.present(overlay, handle, (y + 4.0 * self.scale) as usize)
    }

    // ── CPU rasterization helpers ──

    fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize) {
        let my = (y + h).min(MAX_H); let mx = (x + w).min(self.tex_w);
        for row in y..my {
            let s = row * self.tex_w + x; let e = (row * self.tex_w + mx).min(self.pixels.len() / 4);
            for i in s..e { self.pixels[i*4+0]=30; self.pixels[i*4+1]=20; self.pixels[i*4+2]=20; self.pixels[i*4+3]=180; }
        }
    }

    fn fill_rect_transparent(&mut self, x: usize, y: usize, w: usize, h: usize) {
        let my = (y + h).min(MAX_H); let mx = (x + w).min(self.tex_w);
        for row in y..my {
            let s = row * self.tex_w + x; let e = (row * self.tex_w + mx).min(self.pixels.len() / 4);
            for i in s..e { self.pixels[i * 4 + 3] = 0; }
        }
    }

    fn render_text(&mut self, text: &str, size: f32, x: f32, y: f32, color: [u8; 4]) -> f32 {
        let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
        layout.reset(&LayoutSettings { x, y, max_width: Some(self.tex_w as f32 - x - 8.0), ..LayoutSettings::default() });
        layout.append(&[&self.font], &TextStyle::new(text, size, 0));
        for glyph in layout.glyphs() {
            let (_, bitmap) = self.font.rasterize(glyph.parent, size);
            let (gx, gy, gw, gh) = (glyph.x as usize, glyph.y as usize, glyph.width as usize, glyph.height as usize);
            for row in 0..gh { for col in 0..gw {
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
            }}
        }
        y + layout.height() as f32
    }

    fn draw_separator(&mut self, y: usize) {
        if y >= MAX_H { return; }
        let i = y * self.tex_w;
        for x in 0..self.tex_w {
            let p = (i + x) * 4;
            self.pixels[p+0]=90; self.pixels[p+1]=80; self.pixels[p+2]=80; self.pixels[p+3]=200;
        }
    }

    // ── Present: dispatch based on backend ──

    fn present(
        &self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
        content_h: usize,
    ) -> anyhow::Result<()> {
        match self.backend {
            Backend::D3D11 => self.d3d11_present(handle, content_h),
            Backend::RawData => self.rawdata_present(overlay, handle, content_h),
        }
    }

    fn d3d11_present(&self, handle: OverlayHandle, _content_h: usize) -> anyhow::Result<()> {
        let tex = self.texture.as_ref()
            .ok_or_else(|| anyhow::anyhow!("D3D11 texture not initialized"))?;
        let context = self.context.as_ref()
            .ok_or_else(|| anyhow::anyhow!("D3D11 context not initialized"))?;
        let table = self.overlay_fn_table
            .ok_or_else(|| anyhow::anyhow!("overlay fn table not initialized"))?;

        // Upload all rows via UpdateSubresource (no Map needed for DEFAULT usage)
        let row_bytes = (self.tex_w * 4) as u32;
        let total_bytes = self.tex_w * MAX_H * 4;
        let slice = &self.pixels[..total_bytes];

        unsafe {
            context.UpdateSubresource(
                tex,
                0,                                // DstSubresource
                None,                             // pDstBox (None = full texture)
                slice.as_ptr() as *const c_void,  // pSrcData
                row_bytes,                        // SrcRowPitch
                0,                                // SrcDepthPitch
            );
        }

        let mut vr_tex = openvr_sys::Texture_t {
            handle: tex.as_raw() as *mut c_void,
            eType: openvr_sys::ETextureType_TextureType_DirectX,
            eColorSpace: openvr_sys::EColorSpace_ColorSpace_Auto,
        };

        let set_tex = table.SetOverlayTexture
            .ok_or_else(|| anyhow::anyhow!("SetOverlayTexture fn ptr is None"))?;

        let ret: i32 = unsafe { set_tex(handle.0, &mut vr_tex) };
        if ret != 0 {
            anyhow::bail!("SetOverlayTexture error {ret}");
        }
        Ok(())
    }

    fn rawdata_present(
        &self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
        content_h: usize,
    ) -> anyhow::Result<()> {
        let h = content_h.clamp(64, MAX_H);
        let total_bytes = self.tex_w * h * 4;
        let slice = &self.pixels[..total_bytes];
        overlay.set_raw_data(handle, slice, self.tex_w, h, 4)
            .map_err(|e| anyhow::anyhow!("set_raw_data: {:?}", e))
    }
}
