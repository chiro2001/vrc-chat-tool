//! D3D11 + imgui overlay rendering.
//! Creates a D3D11 texture, renders imgui frame into it,
//! and submits to the OpenVR overlay via SetOverlayTexture.

use crate::state::OverlayState;

pub struct OverlayRenderer {
    device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    shader_resource_view: windows::Win32::Graphics::Direct3D11::ID3D11ShaderResourceView,
    imgui: imgui::Context,
    platform: imgui_winit_support::WinitPlatform,
    render_target: Option<(u32, u32)>,
}

impl OverlayRenderer {
    /// Initialize D3D11 device and imgui context using the GPU that SteamVR uses.
    pub fn new() -> anyhow::Result<Self> {
        // Create D3D11 device
        let (device, context) = create_d3d11_device()?;

        // Create render target texture (1024x256 pixels → ~1.2m wide)
        let width: u32 = 1024;
        let height: u32 = 256;
        let (texture, shader_resource_view) =
            create_overlay_texture(&device, width, height)?;

        // Init imgui
        let mut imgui = imgui::Context::create();
        imgui.set_ini_filename(None);
        imgui.fonts().add_font(&[imgui::FontSource::DefaultFontData { config: None }]);

        let style = imgui.style_mut();
        style.alpha = 0.85;
        style.colors[imgui::style::Color::WindowBg as usize] = [0.0, 0.0, 0.0, 0.6];
        style.colors[imgui::style::Color::Text as usize] = [0.88, 0.88, 0.88, 1.0];

        let platform = imgui_winit_support::WinitPlatform::init(&mut imgui);

        Ok(Self {
            device,
            context,
            texture,
            shader_resource_view,
            imgui,
            platform,
            render_target: Some((width, height)),
        })
    }

    /// Render one frame of imgui into the overlay texture and submit to OpenVR.
    pub fn render_frame(
        &self,
        handle: openvr::sys::VROverlayHandle_t,
        overlay: &openvr::overlay::Overlay<openvr::overlay::Handle>,
        state: &OverlayState,
    ) -> anyhow::Result<()> {
        let ui = self.imgui.frame();

        // Build UI
        imgui::Window::new("##hud")
            .position([10.0, 10.0], imgui::Condition::Always)
            .size([1000.0, 240.0], imgui::Condition::Always)
            .no_title_bar(true)
            .no_resize(true)
            .no_move(true)
            .no_scrollbar(true)
            .draw_background(true)
            .build(&ui, || {
                // Status indicator
                let status_color = match state.status.as_str() {
                    "recording" => [0.9, 0.2, 0.2, 1.0],
                    "recognizing" => [1.0, 0.7, 0.2, 1.0],
                    _ => [0.3, 0.8, 0.3, 1.0],
                };
                ui.text_colored(status_color, format!("● {}", status_label(&state.status)));

                ui.same_line_with_pos(200.0);
                ui.text(format!("音量: {:.0}%", state.volume * 100.0));

                ui.same_line_with_pos(400.0);
                ui.text(format!("模型: {}", state.model));

                ui.separator();

                // Recognition text
                if !state.current_text.is_empty() {
                    ui.text_wrapped(&state.current_text);
                }

                // Last sentence
                if !state.last_sentence.is_empty() {
                    ui.separator();
                    ui.text_wrapped(&state.last_sentence);
                }
            });

        // Render to texture via the renderer
        // (In full implementation: create render target view, render imgui draw data,
        //  submit texture handle to OpenVR)
        let _ = ui;
        let _ = handle;
        let _ = overlay;

        // Placeholder: submit texture to overlay
        // let tex_handle: *mut std::ffi::c_void = &self.texture as *const _ as *mut _;
        // overlay.set_overlay_texture(handle, tex_handle)?;

        Ok(())
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

fn create_d3d11_device() -> anyhow::Result<(
    windows::Win32::Graphics::Direct3D11::ID3D11Device,
    windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
)> {
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Direct3D::*;
    use windows::Win32::Graphics::Dxgi::Common::*;

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;

    unsafe {
        D3D11CreateDevice(
            None, // default adapter
            D3D_DRIVER_TYPE_HARDWARE,
            None, // no software rasterizer
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None, // default feature level
            D3D11_SDK_VERSION,
            Some(&mut device),
            None, // feature level
            Some(&mut context),
        )?;
    }

    Ok((device.unwrap(), context.unwrap()))
}

fn create_overlay_texture(
    device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
    width: u32,
    height: u32,
) -> anyhow::Result<(
    windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    windows::Win32::Graphics::Direct3D11::ID3D11ShaderResourceView,
)> {
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::*;

    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };

    let texture = unsafe {
        let mut tex: Option<ID3D11Texture2D> = None;
        device.CreateTexture2D(&desc, None, Some(&mut tex))?;
        tex.unwrap()
    };

    let srv = unsafe {
        let mut view: Option<ID3D11ShaderResourceView> = None;
        device.CreateShaderResourceView(&texture, None, Some(&mut view))?;
        view.unwrap()
    };

    Ok((texture, srv))
}
