//! vrc-chat-hud — Standalone SteamVR overlay companion process.
//!
//! Architecture:
//!   1. OpenVR init → create overlay → show "waiting" state
//!   2. Connect to main process via named pipe
//!   3. Handshake → receive config → enter live HUD loop
//!   4. Event loop: poll IPC → apply data/config → smooth HMD → render
//!
//! Head-lag smoothing: exponential smoothing on HMD absolute pose.
//! Config: opacity, scale, smoothing, position — from main process.

mod ipc;
mod render;
mod state;

use std::sync::{Arc, Mutex};
use std::time::Duration;
use openvr::{tracked_device_index, TrackingUniverseOrigin};
use openvr::pose::Matrix3x4;
use state::OverlayState;

/// Initialize OpenVR as Overlay application type.
fn init_openvr() -> anyhow::Result<openvr::Context> {
    tracing::info!("Initializing OpenVR as Overlay...");
    unsafe { openvr::init(openvr::ApplicationType::Overlay) }
        .map_err(|e| anyhow::anyhow!("OpenVR init failed: {:?}", e))
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Install Ctrl+C handler for graceful shutdown
    ctrlc::set_handler(|| {
        tracing::info!("Ctrl+C received, exiting");
        std::process::exit(0);
    }).ok();
    tracing::info!("vrc-chat-hud starting (PID={})", std::process::id());

    // 1. OpenVR — try multiple app types, detect HMD first
    if !openvr::is_runtime_installed() {
        anyhow::bail!("SteamVR runtime not installed");
    }
    if !openvr::is_hmd_present() {
        anyhow::bail!("HMD not detected");
    }

    let ctx = init_openvr()?;
    let system = ctx.system()?;
    let mut overlay = ctx.overlay()?;
    let overlay_key = format!("vrcchat.hud.{}", std::process::id());
    let handle = overlay
        .create_overlay(&overlay_key, "VRC Chat HUD")
        .map_err(|e| anyhow::anyhow!("create_overlay: {:?}", e))?;
    overlay
        .set_width(handle, 0.6)
        .map_err(|e| anyhow::anyhow!("set_width: {:?}", e))?;
    tracing::info!("Overlay created (key={})", overlay_key);

    // 2. State + renderer (default scale)
    let state = Arc::new(Mutex::new(OverlayState::default()));
    let mut renderer = render::OverlayRenderer::new(state.lock().unwrap().scale)?;

    // Show disconnected state
    renderer.render_disconnected(&mut overlay, handle)?;
    overlay
        .set_opacity(handle, 0.85)
        .map_err(|e| anyhow::anyhow!("set_opacity: {:?}", e))?;
    overlay
        .set_visibility(handle, true)
        .map_err(|e| anyhow::anyhow!("show_overlay: {:?}", e))?;
    tracing::info!("Overlay visible (disconnected)");

    // Initial smoothed HMD pose
    let mut smoothed = [[1.0f32, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
    if let Some(p) = get_hmd_pose(&system) {
        smoothed = p;
    }

    // 3. Connect to main process
    let (mut ipc, mut connected) = connect_to_server();

    // 4. Event loop
    let mut last_snapshot = String::new();
    let mut reconnect_backoff_ms: u64 = 1000;
    let mut was_connected = false;
    let mut last_visible = true;

    loop {
        // Poll OpenVR events
        while let Some(_event) = system.poll_next_event() {}

        // Smooth HMD and update transform (uses current config from state)
        {
            let s = state.lock().unwrap();
            update_transform(&system, &mut overlay, handle, &mut smoothed, &s);
        }

        if connected {
            if let Some(ref mut c) = ipc {
                for event in c.poll() {
                    match event {
                        ipc::HudEvent::Bye => {
                            tracing::info!("Received bye, shutting down");
                            return Ok(());
                        }
                        ipc::HudEvent::Config(msg) => {
                            let mut s = state.lock().unwrap();
                            s.apply_config(&msg);
                            renderer.set_scale(s.scale);
                            overlay
                                .set_opacity(handle, s.opacity)
                                .map_err(|e| tracing::warn!("set_opacity: {:?}", e))
                                .ok();
                            tracing::info!(
                                "Config: opacity={:.2} scale={:.1} smoothing={:.2} pos=({:.2},{:.2},{:.2})",
                                s.opacity, s.scale, s.smoothing, s.pos_x, s.pos_y, s.pos_z
                            );
                        }
                        ipc::HudEvent::Data(msg) => {
                            let mut s = state.lock().unwrap();
                            s.update(&msg);
                        }
                        _ => {}
                    }
                }
            } else {
                // Pipe disconnected, go back to waiting
                connected = false;
                tracing::warn!("Pipe lost, reconnecting...");
            }
        } else {
            // Not connected — retry with backoff
            std::thread::sleep(Duration::from_millis(reconnect_backoff_ms));
            reconnect_backoff_ms = (reconnect_backoff_ms * 2).min(16000);
            let result = connect_to_server();
            ipc = result.0;
            connected = result.1;
            if connected {
                reconnect_backoff_ms = 1000;
            }
        }

        // Handle visibility changes (only when state actually changes)
        if connected && !was_connected {
            let _ = overlay.set_visibility(handle, true);
            was_connected = true;
        } else if !connected && was_connected {
            was_connected = false;
        }

        let snap;
        {
            let s = state.lock().unwrap();
            if connected && s.visible != last_visible {
                last_visible = s.visible;
                let _ = overlay.set_visibility(handle, s.visible);
            }
            snap = if connected {
                format!("{}|{}|{}|{:.1}|{}|{}",
                    s.status, s.current_text, s.last_sentence, s.volume, s.model, s.visible)
            } else {
                String::new()
            };
        }

        // Render on state change
        if connected && snap != last_snapshot {
            last_snapshot = snap;
            let s = state.lock().unwrap();
            if let Err(e) = renderer.render_frame(&mut overlay, handle, &s) {
                tracing::warn!("Render: {}", e);
            }
        } else if !connected {
            let _ = renderer.render_disconnected(&mut overlay, handle);
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Connect to main process, perform handshake.
/// Returns (client, is_connected).
fn connect_to_server() -> (Option<ipc::IpcClient>, bool) {
    match ipc::IpcClient::connect("\\\\.\\pipe\\vrc-chat-hud") {
        Ok(client) => {
            // Wait for hello (blocking ReadFile on first connect)
            // The server sends hello immediately after connect.
            // We send ack and return connected.
            client.send_ack();
            tracing::info!("IPC connected, handshake complete");
            (Some(client), true)
        }
        Err(e) => {
            tracing::warn!("IPC connect failed: {}", e);
            (None, false)
        }
    }
}

fn get_hmd_pose(system: &openvr::System) -> Option<[[f32; 4]; 3]> {
    let poses = system.device_to_absolute_tracking_pose(
        TrackingUniverseOrigin::Standing, 0.0,
    );
    let pose = &poses[tracked_device_index::HMD.0 as usize];
    if pose.device_is_connected() && pose.pose_is_valid() {
        Some(*pose.device_to_absolute_tracking())
    } else {
        None
    }
}

fn update_transform(
    system: &openvr::System,
    overlay: &mut openvr::Overlay,
    handle: openvr::overlay::OverlayHandle,
    smoothed: &mut [[f32; 4]; 3],
    config: &OverlayState,
) {
    if let Some(current) = get_hmd_pose(system) {
        let alpha = config.smoothing;
        for i in 0..3 {
            for j in 0..4 {
                smoothed[i][j] = smoothed[i][j] * (1.0 - alpha) + current[i][j] * alpha;
            }
        }
    }

    let ox = smoothed[0][3]
        + smoothed[0][0] * config.pos_x + smoothed[0][1] * config.pos_y + smoothed[0][2] * config.pos_z;
    let oy = smoothed[1][3]
        + smoothed[1][0] * config.pos_x + smoothed[1][1] * config.pos_y + smoothed[1][2] * config.pos_z;
    let oz = smoothed[2][3]
        + smoothed[2][0] * config.pos_x + smoothed[2][1] * config.pos_y + smoothed[2][2] * config.pos_z;

    let abs = Matrix3x4([
        [smoothed[0][0], smoothed[0][1], smoothed[0][2], ox],
        [smoothed[1][0], smoothed[1][1], smoothed[1][2], oy],
        [smoothed[2][0], smoothed[2][1], smoothed[2][2], oz],
    ]);

    if let Err(e) = overlay.set_transform_absolute(handle, TrackingUniverseOrigin::Standing, &abs) {
        tracing::warn!("set_transform_absolute: {:?}", e);
    }
}
