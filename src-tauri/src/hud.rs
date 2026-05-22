//! VR HUD process lifecycle management.
//! Extracted from main.rs so ipc_server can trigger restarts.

use std::process::{Child, Command};
use std::sync::Mutex;
use crate::log;

static HUD_CHILD: Mutex<Option<Child>> = Mutex::new(None);

/// Spawn the VR HUD companion process (vrc-chat-hud.exe).
/// Searches exe dir, sibling release/debug dirs for the binary.
/// Refuses to spawn if SteamVR/admin elevation mismatch is detected.
pub fn spawn() {
    let (compat_ok, compat_msg) = crate::config::check_steamvr_compat();
    if !compat_ok {
        log::warn("hud", &format!("Refusing to spawn HUD: {}", compat_msg));
        return;
    }

    let exe_dir = match std::env::current_exe() {
        Ok(p) => p.parent().map(|p| p.to_path_buf()),
        Err(_) => None,
    };
    if let Some(dir) = exe_dir {
        let candidates = [
            dir.join("vrc-chat-hud.exe"),
            dir.parent().map(|p| p.join("release").join("vrc-chat-hud.exe")).unwrap_or_default(),
            dir.parent().map(|p| p.join("debug").join("vrc-chat-hud.exe")).unwrap_or_default(),
        ];
        for hud in &candidates {
            if hud.exists() {
                log::info("hud", &format!("Spawning VR HUD: {}", hud.display()));
                match Command::new(hud).spawn() {
                    Ok(child) => {
                        *HUD_CHILD.lock().unwrap() = Some(child);
                        return;
                    }
                    Err(e) => log::warn("hud", &format!("Failed to spawn HUD: {}", e)),
                }
            }
        }
        log::warn("hud", &format!(
            "HUD binary not found. Tried:\n  {}\n  {}\n  {}",
            candidates[0].display(),
            candidates[1].display(),
            candidates[2].display(),
        ));
    }
}

/// Kill the VR HUD process gracefully via IPC bye, then wait for exit.
/// Does NOT force-kill — that would leak OpenVR GPU resources.
pub fn kill() {
    if let Some(child) = HUD_CHILD.lock().unwrap().take() {
        log::info("hud", "Stopping VR HUD (sending bye via IPC)");
        crate::ipc_server::stop_overlay_ipc();
        std::thread::spawn(move || {
            let mut child = child;
            for _ in 0..150 {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        log::info("hud", &format!(
                            "VR HUD exited gracefully (code: {:?})",
                            status.code()
                        ));
                        return;
                    }
                    _ => std::thread::sleep(std::time::Duration::from_millis(100)),
                }
            }
            log::error("hud", "VR HUD did not exit after 15s — OpenVR resources may leak");
        });
    }
}

/// Check if the HUD process is still running.
pub fn is_running() -> bool {
    HUD_CHILD.lock().unwrap().is_some()
}
