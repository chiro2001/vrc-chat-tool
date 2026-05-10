//! Named pipe IPC client for receiving overlay data from the main process.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::time::Duration;
use winapi::um::fileapi::{CreateFileA, OPEN_EXISTING};
use winapi::um::handleapi::CloseHandle;
use winapi::um::namedpipeapi::SetNamedPipeHandleState;
use winapi::um::winbase::{PIPE_READMODE_MESSAGE, PIPE_NOWAIT, FILE_FLAG_OVERLAPPED};
use winapi::shared::winerror::ERROR_PIPE_BUSY;
use winapi::ctypes::c_void;

/// Message from main process to overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayMessage {
    pub status: String,         // "idle" | "recording" | "recognizing"
    pub text: String,           // current recognition text
    pub sentence: String,       // last completed sentence
    pub volume: f32,            // 0.0 - 1.0
    pub model: String,          // current ASR model name
}
pub struct IpcClient {
    handle: *mut c_void,
    buffer: Vec<u8>,
}

impl IpcClient {
    /// Connect to the named pipe server (main process).
    pub fn connect(pipe_name: &str) -> anyhow::Result<Self> {
        let name = std::ffi::CString::new(pipe_name)?;

        // Wait for pipe server
        loop {
            let handle = unsafe {
                CreateFileA(
                    name.as_ptr(),
                    0x80000000, // GENERIC_READ
                    0,          // no sharing
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    std::ptr::null_mut(),
                )
            };

            if handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
                // Set non-blocking message-read mode
                unsafe {
                    let mut mode: u32 = PIPE_READMODE_MESSAGE | PIPE_NOWAIT;
                    SetNamedPipeHandleState(handle, &mut mode, std::ptr::null_mut(), std::ptr::null_mut());
                }
                return Ok(Self {
                    handle,
                    buffer: vec![0u8; 4096],
                });
            }

            let err = unsafe { winapi::um::errhandlingapi::GetLastError() };
            if err != ERROR_PIPE_BUSY {
                anyhow::bail!("Failed to connect to pipe (error {})", err);
            }

            // Wait and retry
            if unsafe {
                winapi::um::namedpipeapi::WaitNamedPipeA(
                    name.as_ptr(),
                    5000, // 5s timeout
                )
            } == 0
            {
                anyhow::bail!("Pipe server not available after timeout");
            }
        }
    }

    /// Poll for new messages. Calls `on_message` for each complete JSON message received.
    pub fn poll<F: FnMut(OverlayMessage)>(&self, mut on_message: F) {
        let mut bytes_read: u32 = 0;
        let result = unsafe {
            winapi::um::fileapi::ReadFile(
                self.handle,
                self.buffer.as_mut_ptr() as *mut c_void,
                self.buffer.len() as u32,
                &mut bytes_read,
                std::ptr::null_mut(),
            )
        };

        if result != 0 && bytes_read > 0 {
            let data = &self.buffer[..bytes_read as usize];
            // Messages are newline-delimited JSON
            for line in data.split(|&b| b == b'\n') {
                if line.is_empty() {
                    continue;
                }
                if let Ok(msg) = serde_json::from_slice::<OverlayMessage>(line) {
                    on_message(msg);
                }
            }
        }
    }
}

impl Drop for IpcClient {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { CloseHandle(self.handle) };
        }
    }
}

// SAFETY: Pipe handle is Send + Sync across threads
unsafe impl Send for IpcClient {}
unsafe impl Sync for IpcClient {}
