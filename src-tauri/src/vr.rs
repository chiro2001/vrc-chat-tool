//! SteamVR controller input monitoring (Phase 3).
//! Runs a background thread polling OpenVR digital actions for recording toggle,
//! with double-click detection support.
//!
//! Requires: SteamVR runtime installed and running.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use openvr::{init, ApplicationType};

use crate::log;

static VR_ACTIVE: AtomicBool = AtomicBool::new(false);
static VR_RUNNING: AtomicBool = AtomicBool::new(false);

/// Start VR controller input polling in a background thread.
/// Uses VRApplication_Background (no rendering needed, no conflict with WebView).
/// Exits silently if SteamVR is not running.
pub fn start_controller_listener(app: tauri::AppHandle) {
    if VR_RUNNING.swap(true, Ordering::SeqCst) {
        log::debug("vr", "Controller listener already running");
        return;
    }

    thread::spawn(move || {
        log::info("vr", "Starting VR controller listener");

        // Initialize OpenVR in background mode
        let ctx = match unsafe { init(ApplicationType::Background) } {
            Ok(ctx) => {
                log::info("vr", "OpenVR initialized (Background)");
                VR_ACTIVE.store(true, Ordering::SeqCst);
                ctx
            }
            Err(e) => {
                log::error("vr", &format!("OpenVR init failed (SteamVR not running?): {}", e));
                VR_RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        };

        // Get input subsystem
        let input = match ctx.input() {
            Ok(i) => i,
            Err(e) => {
                log::error("vr", &format!("OpenVR input subsystem failed: {}", e));
                VR_RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        };

        // Set action manifest
        if let Err(e) = input.set_action_manifest(std::path::Path::new("action_manifest.json")) {
            log::warn("vr", &format!("Failed to set action manifest: {}", e));
        }

        let toggle_handle = match input.get_action_handle("/actions/main/in/ToggleRecording") {
            Ok(h) => h,
            Err(e) => {
                log::error("vr", &format!("Action handle failed: {}", e));
                VR_RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        };

        let main_set = match input.get_action_set_handle("/actions/main") {
            Ok(h) => h,
            Err(e) => {
                log::error("vr", &format!("Action set handle failed: {}", e));
                VR_RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        };

        // Double-click detector
        let mut last_press: Option<Instant> = None;
        const DOUBLE_CLICK_MS: u64 = 400;

        // Poll loop (20 Hz)
        loop {
            if !VR_RUNNING.load(Ordering::Relaxed) {
                break;
            }

            match input.update_actions(&mut [
                openvr::input::VRActiveActionSet::new(main_set),
            ]) {
                Ok(()) => {}
                Err(e) => {
                    log::error("vr", &format!("Update actions failed: {}", e));
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
            }

            match input.get_digital_action_data(toggle_handle, openvr::input::VRActiveActionSet::new(main_set)) {
                Ok(data) => {
                    if data.b_state && data.b_changed {
                        // Double-click detection
                        let now = Instant::now();
                        let is_double = last_press
                            .map(|t| now.duration_since(t).as_millis() < DOUBLE_CLICK_MS as u128)
                            .unwrap_or(false);

                        last_press = Some(now);

                        if is_double {
                            log::info("vr", "VR controller: double-click detected — toggle recording");
                            let _ = app.emit_all("hotkey-toggle", "");
                        }
                    }
                }
                Err(e) => {
                    log::error("vr", &format!("Get action data failed: {}", e));
                }
            }

            std::thread::sleep(Duration::from_millis(50));
        }

        VR_ACTIVE.store(false, Ordering::SeqCst);
        log::info("vr", "VR controller listener stopped");
    });
}

/// Stop the VR controller listener.
pub fn stop_controller_listener() {
    VR_RUNNING.store(false, Ordering::SeqCst);
}

/// Check if VR controller listener is currently active.
pub fn is_vr_active() -> bool {
    VR_ACTIVE.load(Ordering::Relaxed)
}
