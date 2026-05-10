//! vrc-chat-hud — Standalone SteamVR overlay companion process.
//!
//! Architecture:
//!   1. Initialize OpenVR (VRApplication_Overlay)
//!   2. Create a HUD overlay (HMD-locked, semi-transparent)
//!   3. Setup D3D11 device + imgui renderer
//!   4. Connect to main process via named pipe IPC
//!   5. Event loop: poll IPC → render text → update overlay texture
//!
//! Requires: SteamVR runtime + LLVM/libclang (for openvr_sys build)

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod ipc;
mod render;
mod state;

use state::OverlayState;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("vrc-chat-hud starting");

    // 1. OpenVR initialization
    let ctx = unsafe {
        openvr::init(openvr::ApplicationType::Overlay)
    }?;
    tracing::info!("OpenVR initialized");

    // 2. Create overlay
    let overlay = ctx.overlay()?;
    let hud_handle = overlay.create_overlay(
        "com.vrcchattool.hud",
        "VRC Chat HUD",
    )?;
    tracing::info!("Overlay created");

    // 3. Position: HMD-locked, front 2m, up 0.15m
    let transform = openvr::HmdMatrix34_t {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.15],
            [0.0, 0.0, 1.0, -2.0],
        ],
    };
    overlay.set_transform_tracked_device_relative(
        hud_handle,
        openvr::TrackedDeviceIndex::Hmd,
        &transform,
    )?;
    overlay.set_overlay_width_in_meters(hud_handle, 1.2)?;
    overlay.show_overlay(hud_handle)?;
    tracing::info!("Overlay positioned");

    // 4. Setup D3D11 + imgui renderer
    let renderer = render::OverlayRenderer::new()?;

    // 5. IPC client
    let ipc = ipc::IpcClient::connect("\\\\.\\pipe\\vrc-chat-hud")?;
    tracing::info!("IPC connected");

    // 6. Shared state
    let state = Arc::new(Mutex::new(OverlayState::default()));

    // 7. Event loop
    let app = ctx.applications()?;
    let _app_id = app.launch_dashboard_overlay("com.vrcchattool.dashboard")?;

    let mut last_render = Instant::now();
    let render_interval = Duration::from_millis(16); // ~60 FPS when visible
    let idle_interval = Duration::from_millis(100);   // 10 FPS when not visible

    loop {
        let is_visible = overlay.is_overlay_visible(hud_handle)?;

        // Poll IPC (non-blocking, parse any available messages)
        ipc.poll(|msg: ipc::OverlayMessage| {
            if let Ok(mut s) = state.lock() {
                s.update(msg);
            }
        });

        // Poll OpenVR overlay events
        let mut event = openvr::VREvent_t::default();
        while ctx.system()?.poll_next_event(&mut event) {
            // Dashboard overlay events handled by OpenVR
        }

        let interval = if is_visible {
            if last_render.elapsed() >= render_interval {
                let s = state.lock().unwrap();
                renderer.render_frame(hud_handle, &overlay, &s)?;
                drop(s);
                last_render = Instant::now();
            }
            Duration::from_millis(1)
        } else {
            idle_interval
        };

        std::thread::sleep(interval);
    }
}
