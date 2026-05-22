//! Named pipe IPC server — sends overlay data to vrc-chat-hud.exe.
//! Duplex pipe with handshake protocol.
//!
//! Protocol:
//!   1. Server creates pipe, waits for client
//!   2. Server sends {"type":"hello"}
//!   3. Client responds {"type":"ack"}
//!   4. Server starts pushing data at ~30Hz
//!   5. On shutdown: server sends {"type":"bye"}, client exits

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use serde::Serialize;

use crate::log;

static IPC_RUNNING: AtomicBool = AtomicBool::new(false);

/// Message between main process and VR HUD.
#[derive(Clone, Serialize)]
pub struct OverlayMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sentence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    // HUD config (sent once after handshake)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smoothing: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_x: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_y: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_z: Option<f32>,
}

impl Default for OverlayMessage {
    fn default() -> Self {
        Self {
            msg_type: "data".into(),
            status: None, visible: None, text: None,
            sentence: None, volume: None, model: None,
            opacity: None, scale: None, smoothing: None,
            pos_x: None, pos_y: None, pos_z: None,
        }
    }
}

pub static OVERLAY_MSG: Mutex<OverlayMessage> = Mutex::new(OverlayMessage {
    msg_type: String::new(),
    status: None, visible: None, text: None,
    sentence: None, volume: None, model: None,
    opacity: None, scale: None, smoothing: None,
    pos_x: None, pos_y: None, pos_z: None,
});

/// HUD configuration — set by main process before first client connects.
static HUD_CONFIG: Mutex<Option<OverlayMessage>> = Mutex::new(None);

/// Set HUD configuration parameters (opacity, scale, smoothing, position).
pub fn set_hud_config(opacity: f32, scale: f32, smoothing: f32, x: f32, y: f32, z: f32) {
    *HUD_CONFIG.lock().unwrap() = Some(OverlayMessage {
        msg_type: "config".into(),
        opacity: Some(opacity),
        scale: Some(scale),
        smoothing: Some(smoothing),
        pos_x: Some(x),
        pos_y: Some(y),
        pos_z: Some(z),
        ..Default::default()
    });
}

/// Start the named pipe server for VR HUD IPC.
pub fn start_overlay_ipc() {
    if IPC_RUNNING.swap(true, Ordering::SeqCst) {
        log::debug("ipc", "Overlay IPC already running");
        return;
    }

    thread::spawn(move || {
        use winapi::um::winbase::{
            CreateNamedPipeA, PIPE_TYPE_MESSAGE, PIPE_READMODE_MESSAGE,
            PIPE_WAIT, PIPE_UNLIMITED_INSTANCES, PIPE_ACCESS_DUPLEX,
        };
        use winapi::um::namedpipeapi::ConnectNamedPipe;
        use winapi::um::fileapi::{ReadFile, WriteFile};
        use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
        use winapi::ctypes::c_void;
        use std::ffi::CString;

        let pipe_name = CString::new("\\\\.\\pipe\\vrc-chat-hud").unwrap();

        while IPC_RUNNING.load(Ordering::Relaxed) {
            // Create duplex pipe
            let handle = unsafe {
                CreateNamedPipeA(
                    pipe_name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,  // read + write
                    PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    4096, 4096, 0,
                    std::ptr::null_mut(),
                )
            };

            if handle == INVALID_HANDLE_VALUE {
                let err = unsafe { winapi::um::errhandlingapi::GetLastError() };
                log::error("ipc", &format!("CreateNamedPipe failed: {}, retrying...", err));
                thread::sleep(Duration::from_secs(2));
                continue;
            }

            log::info("ipc", "Waiting for VR HUD client...");
            let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
            if connected == 0 {
                let err = unsafe { winapi::um::errhandlingapi::GetLastError() };
                if err != 535 { // ERROR_PIPE_CONNECTED
                    log::error("ipc", &format!("ConnectNamedPipe failed: {}", err));
                    unsafe { CloseHandle(handle) };
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
            }
            log::info("ipc", "VR HUD client connected, handshaking...");

            // Handshake: send hello
            let hello = serde_json::to_vec(&OverlayMessage { msg_type: "hello".into(), ..Default::default() })
                .unwrap_or_default();
            let mut hello_data = hello.clone();
            hello_data.push(b'\n');
            let mut written: u32 = 0;
            let ok = unsafe {
                WriteFile(handle, hello_data.as_ptr() as *const c_void, hello_data.len() as u32, &mut written, std::ptr::null_mut())
            };
            if ok == 0 {
                log::error("ipc", "Failed to send hello");
                unsafe { CloseHandle(handle) };
                continue;
            }

            // Wait for ack
            let mut ack_buf = [0u8; 256];
            let mut read: u32 = 0;
            let ok = unsafe {
                ReadFile(handle, ack_buf.as_mut_ptr() as *mut c_void, ack_buf.len() as u32, &mut read, std::ptr::null_mut())
            };
            if ok != 0 && read > 0 {
                let resp = std::str::from_utf8(&ack_buf[..read as usize]).unwrap_or("");
                if !resp.contains("\"ack\"") {
                    log::warn("ipc", &format!("Unexpected handshake response: {}", resp));
                }
            } else {
                log::error("ipc", "No handshake response from HUD");
                unsafe { CloseHandle(handle) };
                continue;
            }
            log::info("ipc", "Handshake complete, sending config...");

            // Send HUD config
            if let Some(ref cfg) = *HUD_CONFIG.lock().unwrap() {
                let cfg_json = serde_json::to_vec(cfg).unwrap_or_default();
                let mut cfg_data = cfg_json.clone();
                cfg_data.push(b'\n');
                let mut w: u32 = 0;
                unsafe { WriteFile(handle, cfg_data.as_ptr() as *const c_void, cfg_data.len() as u32, &mut w, std::ptr::null_mut()); }
            }

            log::info("ipc", "Pushing data");

            // Data loop
            let interval = Duration::from_millis(33);
            loop {
                if !IPC_RUNNING.load(Ordering::Relaxed) {
                    // Send bye before closing — triggers graceful HUD shutdown
                    let bye = b"{\"type\":\"bye\"}\n";
                    let mut w: u32 = 0;
                    unsafe { WriteFile(handle, bye.as_ptr() as *const c_void, bye.len() as u32, &mut w, std::ptr::null_mut()); }
                    log::info("ipc", "Sent bye, closing pipe");
                    break;
                }

                let msg = OVERLAY_MSG.lock().unwrap().clone();
                let json = serde_json::to_string(&msg).unwrap_or_default();
                let mut data = json.into_bytes();
                data.push(b'\n');

                written = 0;
                let ok = unsafe {
                    WriteFile(handle, data.as_ptr() as *const c_void, data.len() as u32, &mut written, std::ptr::null_mut())
                };
                if ok == 0 {
                    log::info("ipc", "VR HUD client disconnected");
                    // If HUD process died externally, spawn a new one for next connection
                    if !crate::hud::is_running() {
                        log::info("ipc", "HUD process not running, spawning new one");
                        crate::hud::spawn();
                    }
                    break;
                }

                thread::sleep(interval);
            }

            unsafe { CloseHandle(handle) };
        }
    });
}

/// Stop the overlay IPC server. Sends bye before closing.
pub fn stop_overlay_ipc() {
    IPC_RUNNING.store(false, Ordering::SeqCst);
}
