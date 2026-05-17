//! vrc-chat-hud — Standalone SteamVR overlay companion process.
//!
//! D3D11 GPU texture rendering (auto-detected, falls back to set_raw_data).
//! Single overlay — D3D11 texture swap is flicker-free, no double-buffering needed.

mod ipc;
mod render;
mod state;

use std::sync::{Arc, Mutex};
use std::time::Duration;
use openvr::{tracked_device_index, TrackingUniverseOrigin};
use openvr::pose::Matrix3x4;
use openvr::overlay::OverlayHandle;
use state::OverlayState;

const SCALE: f32 = 1.0;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    ctrlc::set_handler(|| {
        tracing::info!("Ctrl+C received, exiting");
        std::process::exit(0);
    }).ok();
    tracing::info!("vrc-chat-hud starting (PID={})", std::process::id());

    if !openvr::is_runtime_installed() { anyhow::bail!("SteamVR not installed"); }
    if !openvr::is_hmd_present() { anyhow::bail!("HMD not detected"); }

    // Try multiple ApplicationTypes for OpenVR init
    let (ctx, init_type) = init_openvr()?;
    tracing::info!("OpenVR initialized with {:?}", init_type);

    let system = ctx.system()?;
    let mut overlay = ctx.overlay()?;

    // Single overlay
    let pid = std::process::id();
    let handle = overlay.create_overlay(&format!("vrcchat.hud.{}", pid), "VRC Chat HUD")
        .map_err(|e| anyhow::anyhow!("create_overlay: {:?}", e))?;

    overlay.set_width(handle, 0.6).map_err(|e| anyhow::anyhow!("set_width: {:?}", e))?;
    overlay.set_opacity(handle, 0.85).map_err(|e| anyhow::anyhow!("set_opacity: {:?}", e))?;

    let state = Arc::new(Mutex::new(OverlayState::default()));
    let mut renderer = render::OverlayRenderer::new(SCALE)?;

    // Show disconnected state
    renderer.render_disconnected(&mut overlay, handle)?;
    overlay.set_visibility(handle, true).map_err(|e| anyhow::anyhow!("show overlay: {:?}", e))?;

    // Initial HMD pose
    let mut smoothed = [[1.0f32, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
    if let Some(p) = get_hmd_pose(&system) { smoothed = p; }

    let (mut ipc, mut connected) = connect_to_server();
    let mut last_snapshot = String::new();
    let mut reconnect_ms: u64 = 1000;
    let mut was_connected = false;
    let mut last_visible = true;

    loop {
        while let Some(_) = system.poll_next_event() {}

        if connected {
            match ipc {
                Some(ref mut c) => {
                    for evt in c.poll() {
                        match evt {
                            ipc::HudEvent::Bye => return Ok(()),
                            ipc::HudEvent::Config(msg) => {
                                let mut s = state.lock().unwrap();
                                s.apply_config(&msg);
                                renderer.set_scale(s.scale);
                                overlay.set_opacity(handle, s.opacity)
                                    .map_err(|e| tracing::warn!("set_opacity: {:?}", e)).ok();
                            }
                            ipc::HudEvent::Data(msg) => state.lock().unwrap().update(&msg),
                            _ => {}
                        }
                    }
                }
                None => connected = false,
            }
        } else {
            std::thread::sleep(Duration::from_millis(reconnect_ms));
            reconnect_ms = (reconnect_ms * 2).min(16000);
            let (c, ok) = connect_to_server();
            ipc = c; connected = ok;
            if ok { reconnect_ms = 1000; }
        }

        // Visibility transitions
        if connected && !was_connected {
            let _ = overlay.set_visibility(handle, true);
            was_connected = true;
        }

        let snap;
        {
            let s = state.lock().unwrap();
            if s.visible != last_visible {
                last_visible = s.visible;
                let v = s.visible && connected;
                let _ = overlay.set_visibility(handle, v);
            }
            snap = if connected {
                format!("{}|{}|{}|{:.1}|{}|{}",
                    s.status, s.current_text, s.last_sentence, s.volume, s.model, s.visible)
            } else { String::new() };
        }

        // Render on content change
        if connected && snap != last_snapshot {
            last_snapshot = snap;
            let s = state.lock().unwrap();
            if let Err(e) = renderer.render_frame(&mut overlay, handle, &s) {
                tracing::warn!("Render: {e}");
            }
        } else if !connected {
            let _ = renderer.render_disconnected(&mut overlay, handle);
        }

        // Update transform on the single overlay
        {
            let s = state.lock().unwrap();
            update_transform(&system, &mut overlay, handle, &mut smoothed, &s);
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Try OpenVR init with multiple ApplicationTypes, preferring Overlay.
fn init_openvr() -> anyhow::Result<(openvr::Context, openvr::ApplicationType)> {
    let types = [
        openvr::ApplicationType::Overlay,
        openvr::ApplicationType::Scene,
        openvr::ApplicationType::Background,
    ];
    for &ty in &types {
        match unsafe { openvr::init(ty) } {
            Ok(ctx) => return Ok((ctx, ty)),
            Err(e) => tracing::warn!("openvr::init({:?}) failed: {e}", ty),
        }
    }
    anyhow::bail!("OpenVR init failed with all ApplicationTypes")
}

fn connect_to_server() -> (Option<ipc::IpcClient>, bool) {
    match ipc::IpcClient::connect("\\\\.\\pipe\\vrc-chat-hud") {
        Ok(c) => { c.send_ack(); (Some(c), true) }
        Err(e) => { tracing::warn!("IPC: {e}"); (None, false) }
    }
}

fn get_hmd_pose(system: &openvr::System) -> Option<[[f32; 4]; 3]> {
    let poses = system.device_to_absolute_tracking_pose(TrackingUniverseOrigin::Standing, 0.0);
    let pose = &poses[tracked_device_index::HMD.0 as usize];
    if pose.device_is_connected() && pose.pose_is_valid() {
        Some(*pose.device_to_absolute_tracking())
    } else { None }
}

fn update_transform(
    system: &openvr::System,
    overlay: &mut openvr::Overlay,
    handle: OverlayHandle,
    smoothed: &mut [[f32; 4]; 3],
    config: &OverlayState,
) {
    if let Some(current) = get_hmd_pose(system) {
        let a = config.smoothing;
        for i in 0..3 { for j in 0..4 { smoothed[i][j] = smoothed[i][j] * (1.0 - a) + current[i][j] * a; } }
    }
    let ox = smoothed[0][3] + smoothed[0][0]*config.pos_x + smoothed[0][1]*config.pos_y + smoothed[0][2]*config.pos_z;
    let oy = smoothed[1][3] + smoothed[1][0]*config.pos_x + smoothed[1][1]*config.pos_y + smoothed[1][2]*config.pos_z;
    let oz = smoothed[2][3] + smoothed[2][0]*config.pos_x + smoothed[2][1]*config.pos_y + smoothed[2][2]*config.pos_z;
    let abs = Matrix3x4([
        [smoothed[0][0], smoothed[0][1], smoothed[0][2], ox],
        [smoothed[1][0], smoothed[1][1], smoothed[1][2], oy],
        [smoothed[2][0], smoothed[2][1], smoothed[2][2], oz],
    ]);
    if let Err(e) = overlay.set_transform_absolute(handle, TrackingUniverseOrigin::Standing, &abs) {
        tracing::warn!("transform: {:?}", e);
    }
}
