//! Overlay rendering: fontdue → CPU buffer → D3D11 texture → set_overlay_texture.
//! Uses GPU texture update via UpdateSubresource to eliminate set_raw_data flicker.

use crate::state::OverlayState;
use fontdue::Font;
use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};
use openvr::overlay::OverlayHandle;

use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Dxgi::Common::*;

const TEX_W: usize = 1024;
const TEX_H: usize = 256;

pub struct OverlayRenderer {
    font: Font,
    /// CPU-side pixel buffer (RGBA)
    cpu_buf: Vec<u8>,
    /// D3D11 texture
    texture: ID3D11Texture2D,
    /// D3D11 device context
    context: ID3D11DeviceContext,
    scale: f32,
}

impl OverlayRenderer {
    pub fn new(scale: f32) -> anyhow::Result<Self> {
        let font_data = std::fs::read("C:\\Windows\\Fonts\\simhei.ttf")?;
        let font = Font::from_bytes(font_data, fontdue::FontSettings::default())
            .map_err(|e| anyhow::anyhow!("font: {}", e))?;

        // Create D3D11 device
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        unsafe {
            D3D11CreateDevice(
                None, D3D_DRIVER_TYPE_HARDWARE, None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None, D3D11_SDK_VERSION,
                Some(&mut device), None, Some(&mut context),
            )?;
        }
        let device = device.unwrap();
        let context = context.unwrap();

        // Create D3D11 texture (DYNAMIC for CPU write)
        let desc = D3D11_TEXTURE2D_DESC {
            Width: TEX_W as u32,
            Height: TEX_H as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: 0,
        };
        let mut texture: Option<ID3D11Texture2D> = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))?; }
        let texture = texture.unwrap();

        let cpu_buf = vec![0u8; TEX_W * TEX_H * 4];
        Ok(Self { font, cpu_buf, texture, context, scale })
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
    }

    /// Full HUD render — CPU render to buffer, upload to D3D11 texture, submit to overlay.
    pub fn render_frame(
        &mut self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
        state: &OverlayState,
    ) -> anyhow::Result<()> {
        self.cpu_render(state);

        // Upload CPU buffer to D3D11 texture via Map/Unmap
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(
                &self.texture, 0, D3D11_MAP_WRITE_DISCARD, 0,
                Some(&mut mapped),
            )?;
            let dst = mapped.pData as *mut u8;
            let row_pitch = mapped.RowPitch as usize;
            for y in 0..TEX_H {
                let src_start = y * TEX_W * 4;
                let dst_start = y * row_pitch;
                std::ptr::copy_nonoverlapping(
                    self.cpu_buf.as_ptr().add(src_start),
                    dst.add(dst_start),
                    TEX_W * 4,
                );
            }
            self.context.Unmap(&self.texture, 0);
        }

        // Submit to OpenVR overlay via set_overlay_texture FFI
        self.submit_texture(overlay, handle);

        Ok(())
    }

    /// Minimal disconnected-state render.
    pub fn render_disconnected(
        &mut self,
        overlay: &mut openvr::Overlay,
        handle: OverlayHandle,
    ) -> anyhow::Result<()> {
        // Fill with transparent
        for i in 0..self.cpu_buf.len() {
            self.cpu_buf[i] = 0;
        }
        let size = 40.0 * self.scale;
        self.render_text("等待主程序连接...", size, 12.0, 4.0, [180, 180, 180, 200]);

        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&self.texture, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            let dst = mapped.pData as *mut u8;
            let row_pitch = mapped.RowPitch as usize;
            for y in 0..TEX_H {
                let src = y * TEX_W * 4;
                let dst_start = y * row_pitch;
                std::ptr::copy_nonoverlapping(self.cpu_buf.as_ptr().add(src), dst.add(dst_start), TEX_W * 4);
            }
            self.context.Unmap(&self.texture, 0);
        }

        self.submit_texture(overlay, handle);
        Ok(())
    }

    /// CPU-side rendering to self.cpu_buf.
    fn cpu_render(&mut self, state: &OverlayState) {
        // Clear background
        for i in 0..TEX_W * TEX_H {
            self.cpu_buf[i * 4 + 0] = 20;
            self.cpu_buf[i * 4 + 1] = 20;
            self.cpu_buf[i * 4 + 2] = 30;
            self.cpu_buf[i * 4 + 3] = 180;
        }

        let font_size = 48.0 * self.scale;
        let mut y: f32 = 4.0 * self.scale;

        let status_color = match state.status.as_str() {
            "recording" => [100u8, 220, 80, 255],
            "recognizing" => [255, 180, 50, 255],
            "stop" => [120, 120, 140, 200],
            _ => [80, 200, 80, 255],
        };
        let label = match state.status.as_str() {
            "stop" => "● 未录音", "recording" => "● 录音中",
            "recognizing" => "● 识别中", _ => "● 就绪",
        };
        let status_text = format!("{}    后端: {}", label, state.model);
        y = self.render_text(&status_text, font_size, 12.0 * self.scale, y, status_color);

        if state.status == "stop" { return; }

        y += 4.0 * self.scale;
        self.draw_separator(y as usize);
        y += 4.0 * self.scale;

        if !state.last_sentence.is_empty() {
            let colored = format!("> {}", state.last_sentence);
            y = self.render_text(&colored, font_size, 12.0 * self.scale, y, [120, 220, 120, 255]);
        } else if !state.current_text.is_empty() {
            y = self.render_text(&state.current_text, font_size, 12.0 * self.scale, y, [255, 255, 255, 255]);
        }
    }

    fn render_text(&mut self, text: &str, size: f32, x: f32, y: f32, color: [u8; 4]) -> f32 {
        let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
        layout.reset(&LayoutSettings {
            x, y,
            max_width: Some(TEX_W as f32 - x - 8.0),
            ..LayoutSettings::default()
        });
        layout.append(&[&self.font], &TextStyle::new(text, size, 0));

        for glyph in layout.glyphs() {
            let (_, bitmap) = self.font.rasterize(glyph.parent, size);
            let gx = glyph.x as usize;
            let gy = glyph.y as usize;
            let gw = glyph.width as usize;
            for row in 0..glyph.height {
                for col in 0..gw {
                    let alpha = bitmap[row * gw + col];
                    if alpha == 0 { continue; }
                    let px = gx + col;
                    let py = gy + row;
                    if px >= TEX_W || py >= TEX_H { continue; }
                    let i = py * TEX_W + px;
                    let a = alpha as f32 / 255.0;
                    self.cpu_buf[i * 4 + 0] = (self.cpu_buf[i * 4 + 0] as f32 * (1.0 - a) + color[0] as f32 * a) as u8;
                    self.cpu_buf[i * 4 + 1] = (self.cpu_buf[i * 4 + 1] as f32 * (1.0 - a) + color[1] as f32 * a) as u8;
                    self.cpu_buf[i * 4 + 2] = (self.cpu_buf[i * 4 + 2] as f32 * (1.0 - a) + color[2] as f32 * a) as u8;
                    self.cpu_buf[i * 4 + 3] = 255;
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
            self.cpu_buf[p + 0] = 80;
            self.cpu_buf[p + 1] = 80;
            self.cpu_buf[p + 2] = 90;
            self.cpu_buf[p + 3] = 200;
        }
    }

    /// Submit D3D11 texture to OpenVR overlay via raw FFI (set_overlay_texture).
    fn submit_texture(&self, overlay: &openvr::Overlay, handle: OverlayHandle) {
        // Access the internal function table via pointer cast (Overlay is newtype over fn table)
        let fn_table: &openvr_sys::VR_IVROverlay_FnTable = unsafe {
            &*(overlay as *const openvr::Overlay as *const openvr_sys::VR_IVROverlay_FnTable)
        };

        let ovr_texture = openvr_sys::Texture_t {
            handle: &self.texture as *const _ as *mut std::ffi::c_void,
            eType: openvr_sys::ETextureType_TextureType_DirectX,
            eColorSpace: openvr_sys::EColorSpace_ColorSpace_Auto,
        };

        unsafe {
            if let Some(set_tex) = fn_table.SetOverlayTexture {
                set_tex(handle.0, &raw const ovr_texture as *mut _);
            }
        }
    }
}
