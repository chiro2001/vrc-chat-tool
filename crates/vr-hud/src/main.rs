//! vrc-chat-hud — Standalone SteamVR overlay companion process.
//!
//! Architecture:
//!   1. Initialize OpenVR (VRApplication_Overlay)
//!   2. Create a HUD overlay (HMD-locked)
//!   3. Setup fontdue-based renderer
//!   4. Connect to main process via named pipe IPC (optional)
//!   5. Event loop: poll IPC → render text → upload to overlay via set_raw_data
//!
//! Requires: SteamVR runtime

mod ipc;
mod render;
mod state;

use std::sync::{Arc, Mutex};
use std::time::Duration;
use openvr::tracked_device_index;
use openvr::pose::Matrix3x4;
use state::OverlayState;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("vrc-chat-hud starting");

    // 1. OpenVR initialization
    let ctx = unsafe { openvr::init(openvr::ApplicationType::Overlay) }?;
    tracing::info!("OpenVR initialized");

    // 2. Create overlay
    let mut overlay = ctx.overlay()?;
    let handle = overlay
        .create_overlay("com.vrcchattool.hud", "VRC Chat HUD")
        .map_err(|e| anyhow::anyhow!("create_overlay: {:?}", e))?;
    tracing::info!("Overlay created");

    // 3. Position: HMD-locked, front 1.5m, up 0.1m
    let transform = Matrix3x4([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.1],
        [0.0, 0.0, 1.0, -1.5],
    ]);
    overlay
        .set_transform_tracked_device_relative(handle, tracked_device_index::HMD, &transform)
        .map_err(|e| anyhow::anyhow!("set_transform: {:?}", e))?;
    overlay
        .set_width(handle, 0.8)
        .map_err(|e| anyhow::anyhow!("set_width: {:?}", e))?;
    overlay
        .set_opacity(handle, 0.85)
        .map_err(|e| anyhow::anyhow!("set_opacity: {:?}", e))?;

    // 4. Setup fontdue renderer
    let mut renderer = render::OverlayRenderer::new()?;

    // 5. Shared state with mock data
    let state = Arc::new(Mutex::new(OverlayState::default()));
    {
        let mut s = state.lock().unwrap();
        s.status = "idle".into();
        s.current_text = "vrc-chat-hud 就绪".into();
        s.model = "sherpa-onnx".into();
        s.volume = 0.0;
    }

    // Render initial frame BEFORE showing (overlay needs a texture to display)
    {
        let s = state.lock().unwrap();
        renderer.render_frame(&mut overlay, handle, &s)?;
    }

    // Show overlay after initial texture is set
    overlay
        .set_visibility(handle, true)
        .map_err(|e| anyhow::anyhow!("show_overlay: {:?}", e))?;
    tracing::info!("Overlay positioned and visible");

    // 6. IPC client (optional — use mock state if not connected)
    let mut ipc = ipc::IpcClient::connect("\\\\.\\pipe\\vrc-chat-hud")
        .map_err(|e| {
            tracing::warn!("IPC not available ({}), using mock state", e);
            e
        })
        .ok();

    // 7. Event loop
    let mut last_state_snapshot = String::new();

    loop {
        // Poll OpenVR events
        if let Ok(sys) = ctx.system() {
            while let Some(_event) = sys.poll_next_event() {
                // ignore for now
            }
        }

        // Poll IPC (if connected)
        if let Some(ref mut ipc) = ipc {
            if let Ok(mut s) = state.lock() {
                ipc.poll(|msg: ipc::OverlayMessage| {
                    s.update(msg);
                });
            }
        }

        // Only re-render when state changes
        let current_snapshot = {
            let s = state.lock().unwrap();
            format!("{}|{}|{}|{:.2}|{}",
                s.status, s.current_text, s.last_sentence, s.volume, s.model)
        };

        if current_snapshot != last_state_snapshot {
            last_state_snapshot = current_snapshot;
            if let Ok(s) = state.lock() {
                if let Err(e) = renderer.render_frame(&mut overlay, handle, &s) {
                    tracing::warn!("Render skipped: {}", e);
                }
            }
        }

        std::thread::sleep(Duration::from_millis(50));
    }
}
