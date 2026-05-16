//! Named pipe IPC client — duplex, handshake, bye handling.
//!
//! Protocol:
//!   1. Connect to pipe
//!   2. Receive {"type":"hello"} → respond {"type":"ack"}
//!   3. Receive {"type":"config", ...} → apply config
//!   4. Receive {"type":"data", ...} → partial state update
//!   5. Receive {"type":"bye"} → trigger graceful shutdown

use serde::{Deserialize, Serialize};
use winapi::um::fileapi::{CreateFileA, ReadFile, WriteFile, OPEN_EXISTING};
use winapi::um::handleapi::CloseHandle;
use winapi::um::namedpipeapi::SetNamedPipeHandleState;
use winapi::um::winbase::{
    PIPE_READMODE_MESSAGE, PIPE_NOWAIT, FILE_FLAG_OVERLAPPED,
    WaitNamedPipeA,
};
use winapi::shared::winerror::ERROR_PIPE_BUSY;
use winapi::ctypes::c_void;

/// Message from main process to overlay.
/// All fields optional — HUD only updates what's present.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OverlayMessage {
    #[serde(rename = "type", default)]
    pub msg_type: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub visible: Option<bool>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub sentence: Option<String>,
    #[serde(default)]
    pub volume: Option<f32>,
    #[serde(default)]
    pub model: Option<String>,
    // HUD config
    #[serde(default)]
    pub opacity: Option<f32>,
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub smoothing: Option<f32>,
    #[serde(default)]
    pub pos_x: Option<f32>,
    #[serde(default)]
    pub pos_y: Option<f32>,
    #[serde(default)]
    pub pos_z: Option<f32>,
}

/// Enum for parsed messages.
pub enum HudEvent {
    Connected,
    Bye,
    Data(OverlayMessage),
    Config(OverlayMessage),
}

pub struct IpcClient {
    handle: *mut c_void,
    buffer: Vec<u8>,
}

impl IpcClient {
    pub fn connect(pipe_name: &str) -> anyhow::Result<Self> {
        let name = std::ffi::CString::new(pipe_name)?;

        let handle = loop {
            let h = unsafe {
                CreateFileA(
                    name.as_ptr(),
                    0xC0000000, // GENERIC_READ | GENERIC_WRITE
                    0, std::ptr::null_mut(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    std::ptr::null_mut(),
                )
            };

            if h != winapi::um::handleapi::INVALID_HANDLE_VALUE {
                unsafe {
                    let mut mode: u32 = PIPE_READMODE_MESSAGE | PIPE_NOWAIT;
                    SetNamedPipeHandleState(h, &mut mode, std::ptr::null_mut(), std::ptr::null_mut());
                }
                break h;
            }

            let err = unsafe { winapi::um::errhandlingapi::GetLastError() };
            if err != ERROR_PIPE_BUSY {
                anyhow::bail!("Pipe connect failed (error {})", err);
            }
            unsafe { WaitNamedPipeA(name.as_ptr(), 5000); }
        };

        Ok(Self { handle, buffer: vec![0u8; 4096] })
    }

    /// Send an ack response to the server.
    pub fn send_ack(&self) {
        let ack = b"{\"type\":\"ack\"}\n";
        let mut written: u32 = 0;
        unsafe {
            WriteFile(self.handle, ack.as_ptr() as *const c_void, ack.len() as u32, &mut written, std::ptr::null_mut());
        }
    }

    /// Poll for incoming messages. Returns parsed events.
    pub fn poll(&mut self) -> Vec<HudEvent> {
        let mut events = Vec::new();
        let mut bytes_read: u32 = 0;
        let result = unsafe {
            ReadFile(self.handle, self.buffer.as_mut_ptr() as *mut c_void, self.buffer.len() as u32, &mut bytes_read, std::ptr::null_mut())
        };

        if result != 0 && bytes_read > 0 {
            let data = &self.buffer[..bytes_read as usize];
            for line in data.split(|&b| b == b'\n') {
                if line.is_empty() { continue; }
                if let Ok(msg) = serde_json::from_slice::<OverlayMessage>(line) {
                    match msg.msg_type.as_str() {
                        "hello" => events.push(HudEvent::Connected),
                        "bye" => events.push(HudEvent::Bye),
                        "config" => events.push(HudEvent::Config(msg)),
                        _ => events.push(HudEvent::Data(msg)),
                    }
                }
            }
        }

        events
    }
}

impl Drop for IpcClient {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { CloseHandle(self.handle) };
        }
    }
}

unsafe impl Send for IpcClient {}
unsafe impl Sync for IpcClient {}
