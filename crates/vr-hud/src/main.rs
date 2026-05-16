//! vrc-chat-hud — Standalone SteamVR overlay companion process.
//!
//! Architecture:
//!   1. Initialize OpenVR (VRApplication_Overlay)
//!   2. Create a HUD overlay with head-lag smoothing
//!   3. Setup fontdue-based renderer
//!   4. Connect to main process via named pipe IPC (optional)
//!   5. Event loop: poll IPC → render text → smooth HMD pose → set absolute transform
//!
//! Head-lag smoothing: overlay position follows HMD with exponential smoothing,
//! filtering out high-frequency head jitter for better text readability.
//!
//! Requires: SteamVR runtime

mod ipc;
mod render;
mod state;

use std::sync::{Arc, Mutex};
use std::time::Duration;
use openvr::{tracked_device_index, TrackingUniverseOrigin};
use openvr::pose::Matrix3x4;
use state::OverlayState;

// ── Tunable parameters ──────────────────────────────────────────

/// Smoothing factor for head-lag: 0=hard lock, 1=instant follow.
const SMOOTHING: f32 = 0.10;

/// Render scale: controls texture resolution and font size proportionally.
const SCALE: f32 = 1.0;

/// Local offset from HMD to overlay (HMD-local space, meters).
/// Upper-left of FOV: left ~40cm, up ~30cm, forward ~1.5m.
const LOCAL_X: f32 = -0.4;
const LOCAL_Y: f32 = 0.3;
const LOCAL_Z: f32 = -1.5;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("vrc-chat-hud starting");

    // 1. OpenVR initialization
    let ctx = unsafe { openvr::init(openvr::ApplicationType::Overlay) }?;
    let system = ctx.system()?;
    tracing::info!("OpenVR initialized");

    // 2. Create overlay
    let mut overlay = ctx.overlay()?;
    let handle = overlay
        .create_overlay("vrcchat.hud", "VRC Chat HUD")
        .map_err(|e| anyhow::anyhow!("create_overlay: {:?}", e))?;
    tracing::info!("Overlay created");

    overlay
        .set_width(handle, 0.6)
        .map_err(|e| anyhow::anyhow!("set_width: {:?}", e))?;
    overlay
        .set_opacity(handle, 0.85)
        .map_err(|e| anyhow::anyhow!("set_opacity: {:?}", e))?;

    // 3. Setup fontdue renderer
    let mut renderer = render::OverlayRenderer::new(SCALE)?;

    // 4. Shared state
    let state = Arc::new(Mutex::new(OverlayState::default()));

    // Render disconnected state initially
    renderer.render_disconnected(&mut overlay, handle)?;
    overlay
        .set_visibility(handle, true)
        .map_err(|e| anyhow::anyhow!("show_overlay: {:?}", e))?;
    tracing::info!("Overlay visible (disconnected)");

    // Initialize smoothed HMD pose
    let mut smoothed = get_hmd_pose(&system);

    // 5. IPC client — connect in background
    let mut ipc = ipc::IpcClient::connect("\\\\.\\pipe\\vrc-chat-hud")
        .map_err(|e| {
            tracing::warn!("IPC not available ({}), showing waiting state", e);
            e
        })
        .ok();

    let mut connected = ipc.is_some();

    // 6. Event loop
    let mut last_state_snapshot = String::new();

    loop {
        // Poll OpenVR events
        while let Some(_event) = system.poll_next_event() {}

        // Poll IPC (if connected)
        if let Some(ref mut ipc) = ipc {
            if let Ok(mut s) = state.lock() {
                ipc.poll(|msg: ipc::OverlayMessage| {
                    s.update(msg);
                });
            }
        }

        // Smooth HMD pose and update overlay transform
        update_transform(&system, &mut overlay, handle, &mut smoothed);

        // Render based on connection state
        if connected {
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
        } else {
            // Not connected yet — retry connection
            match ipc::IpcClient::connect("\\\\.\\pipe\\vrc-chat-hud") {
                Ok(c) => {
                    tracing::info!("IPC connected, switching to live HUD");
                    ipc = Some(c);
                    connected = true;
                }
                Err(_) => {
                    let _ = renderer.render_disconnected(&mut overlay, handle);
                }
            }
        }

        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Read current HMD absolute pose. Returns identity on failure.
fn get_hmd_pose(system: &openvr::System) -> [[f32; 4]; 3] {
    let poses = system.device_to_absolute_tracking_pose(
        TrackingUniverseOrigin::Standing, 0.0,
    );
    let pose = &poses[tracked_device_index::HMD.0 as usize];
    if pose.device_is_connected() && pose.pose_is_valid() {
        *pose.device_to_absolute_tracking()
    } else {
        [[1.0, 0.0, 0.0, 0.0],
         [0.0, 1.0, 0.0, 0.0],
         [0.0, 0.0, 1.0, 0.0]]
    }
}

/// Apply exponential smoothing to HMD pose, compute overlay world pose, set absolute transform.
fn update_transform(
    system: &openvr::System,
    overlay: &mut openvr::Overlay,
    handle: openvr::overlay::OverlayHandle,
    smoothed: &mut [[f32; 4]; 3],
) {
    let current = get_hmd_pose(system);

    // Exponential smoothing on all 12 matrix entries
    for i in 0..3 {
        for j in 0..4 {
            smoothed[i][j] = smoothed[i][j] * (1.0 - SMOOTHING) + current[i][j] * SMOOTHING;
        }
    }

    // Compute overlay world position = smoothed_HMD_pos + smoothed_HMD_rot * local_offset
    let ox = smoothed[0][3]
        + smoothed[0][0] * LOCAL_X + smoothed[0][1] * LOCAL_Y + smoothed[0][2] * LOCAL_Z;
    let oy = smoothed[1][3]
        + smoothed[1][0] * LOCAL_X + smoothed[1][1] * LOCAL_Y + smoothed[1][2] * LOCAL_Z;
    let oz = smoothed[2][3]
        + smoothed[2][0] * LOCAL_X + smoothed[2][1] * LOCAL_Y + smoothed[2][2] * LOCAL_Z;

    let abs = Matrix3x4([
        [smoothed[0][0], smoothed[0][1], smoothed[0][2], ox],
        [smoothed[1][0], smoothed[1][1], smoothed[1][2], oy],
        [smoothed[2][0], smoothed[2][1], smoothed[2][2], oz],
    ]);

    if let Err(e) = overlay.set_transform_absolute(handle, TrackingUniverseOrigin::Standing, &abs) {
        tracing::warn!("set_transform_absolute: {:?}", e);
    }
}
