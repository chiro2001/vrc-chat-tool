//! Named pipe IPC server — sends overlay data to vrc-chat-hud.exe.
//! Runs in the main Tauri process, accepting a single client connection.
//! Messages are newline-delimited JSON sent at ~30 Hz when recording.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use serde::Serialize;

use crate::log;

static IPC_RUNNING: AtomicBool = AtomicBool::new(false);

/// Shared state that the recording pipeline updates.
#[derive(Clone, Serialize)]
pub struct OverlayMessage {
    pub status: String,
    pub text: String,
    pub sentence: String,
    pub volume: f32,
    pub model: String,
}

/// Shared overlay message, updated by recording pipeline.
pub static OVERLAY_MSG: Mutex<OverlayMessage> = Mutex::new(OverlayMessage {
    status: String::new(),
    text: String::new(),
    sentence: String::new(),
    volume: 0.0,
    model: String::new(),
});

/// Start the named pipe server for VR HUD IPC.
pub fn start_overlay_ipc() {
    if IPC_RUNNING.swap(true, Ordering::SeqCst) {
        log::debug("ipc", "Overlay IPC already running");
        return;
    }

    thread::spawn(move || {
        log::info("ipc", "Starting overlay IPC server");

        use winapi::um::winbase::{CreateNamedPipeA, PIPE_TYPE_MESSAGE, PIPE_READMODE_MESSAGE, PIPE_WAIT, PIPE_UNLIMITED_INSTANCES};
        use winapi::um::namedpipeapi::ConnectNamedPipe;
        use winapi::um::fileapi::WriteFile;
        use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
        use winapi::ctypes::c_void;
        use std::ffi::CString;

        let pipe_name = CString::new("\\\\.\\pipe\\vrc-chat-hud").unwrap();

        loop {
            if !IPC_RUNNING.load(Ordering::Relaxed) {
                break;
            }

            let handle = unsafe {
                CreateNamedPipeA(
                    pipe_name.as_ptr(),
                    0x40000000,  // PIPE_ACCESS_OUTBOUND
                    PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    4096,
                    4096,
                    0,
                    std::ptr::null_mut(),
                )
            };

            if handle == INVALID_HANDLE_VALUE {
                log::error("ipc", "Failed to create named pipe");
                break;
            }

            log::info("ipc", "Waiting for VR HUD client...");
            let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };

            if connected == 0 {
                let err = unsafe { winapi::um::errhandlingapi::GetLastError() };
                if err != 535 {
                    log::error("ipc", &format!("ConnectNamedPipe failed: {}", err));
                    unsafe { CloseHandle(handle) };
                    continue;
                }
            }

            log::info("ipc", "VR HUD client connected");

            let interval = Duration::from_millis(33);
            loop {
                if !IPC_RUNNING.load(Ordering::Relaxed) {
                    break;
                }

                let msg = OVERLAY_MSG.lock().unwrap().clone();
                let json = serde_json::to_string(&msg).unwrap_or_default();
                let mut data = json.into_bytes();
                data.push(b'\n');

                let mut written: u32 = 0;
                let ok = unsafe {
                    WriteFile(
                        handle,
                        data.as_ptr() as *const c_void,
                        data.len() as u32,
                        &mut written,
                        std::ptr::null_mut(),
                    )
                };

                if ok == 0 {
                    log::info("ipc", "VR HUD client disconnected");
                    break;
                }

                thread::sleep(interval);
            }

            unsafe { CloseHandle(handle) };
        }
    });
}

/// Stop the overlay IPC server.
pub fn stop_overlay_ipc() {
    IPC_RUNNING.store(false, Ordering::SeqCst);
}
