//! Global hotkey listener — registers F13 as a toggle for start/stop recording.
//! Uses RegisterHotKey / GetMessageW via winapi in a background thread.

use std::sync::Mutex;
use std::thread;
use tauri::{AppHandle, Manager};

/// Tracks whether the hotkey listener thread is currently active
static HOTKEY_ACTIVE: Mutex<bool> = Mutex::new(false);
/// Thread handle for the hotkey message loop
static HOTKEY_THREAD_HANDLE: Mutex<Option<thread::JoinHandle<()>>> = Mutex::new(None);

const VK_TOGGLE: u32 = 0x79; // F10

/// Start the global hotkey listener. Safe to call multiple times (idempotent).
pub fn start(app: AppHandle) {
    {
        let mut active = HOTKEY_ACTIVE.lock().unwrap();
        if *active {
            return;
        }
        *active = true;
    }

    let handle = thread::spawn(move || {
        hotkey_message_loop(app);
    });

    *HOTKEY_THREAD_HANDLE.lock().unwrap() = Some(handle);
}

/// Stop the global hotkey listener. Safe to call even if not running.
pub fn stop() {
    *HOTKEY_ACTIVE.lock().unwrap() = false;
    if let Some(handle) = HOTKEY_THREAD_HANDLE.lock().unwrap().take() {
        let _ = handle.join();
    }
}

/// Returns whether the hotkey listener is currently active
pub fn is_active() -> bool {
    *HOTKEY_ACTIVE.lock().unwrap()
}

fn hotkey_message_loop(app: AppHandle) {
    unsafe {
        use winapi::um::winuser::{
            RegisterHotKey, UnregisterHotKey, GetMessageW, TranslateMessage,
            DispatchMessageW, MOD_NOREPEAT, WM_HOTKEY, MSG,
        };

        // Register F10 with NULL hwnd → thread-local WM_HOTKEY delivery
        let result = RegisterHotKey(
            std::ptr::null_mut(),
            1,
            MOD_NOREPEAT as u32,
            VK_TOGGLE,
        );

        if result == 0 {
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(-1);
            eprintln!("[hotkey] Failed to register F13 hotkey (err={})", err);
            *HOTKEY_ACTIVE.lock().unwrap() = false;
            return;
        }

        eprintln!("[hotkey] F10 global hotkey registered");

        let mut msg: MSG = std::mem::zeroed();
        loop {
            if !(*HOTKEY_ACTIVE.lock().unwrap()) {
                break;
            }

            // GetMessageW blocks until a message arrives, returns 0 on WM_QUIT
            let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if ret == 0 || ret == -1 {
                break;
            }

            if msg.message == WM_HOTKEY && msg.wParam == 1 {
                eprintln!("[hotkey] F10 pressed — toggling");
                let _ = app.emit_all("hotkey-toggle", "");
            }

            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        UnregisterHotKey(std::ptr::null_mut(), 1);
        eprintln!("[hotkey] Listener stopped");
    }
}
