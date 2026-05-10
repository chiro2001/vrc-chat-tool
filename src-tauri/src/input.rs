//! Keyboard text input via Windows SendInput API.
//! Injects recognized text as Unicode keystrokes into the currently focused window.
//! Uses existing winapi dependency (zero new crates).

use winapi::um::winuser::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_UNICODE, KEYEVENTF_KEYUP,
};
use crate::log;

/// Inject text into the currently focused application using SendInput.
/// Sends each UTF-16 code unit as a Unicode keyboard event pair (press + release).
pub fn inject_text(text: &str) -> anyhow::Result<()> {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    if utf16.is_empty() {
        return Ok(());
    }

    let mut inputs: [INPUT; 2] = [unsafe { std::mem::zeroed() }, unsafe { std::mem::zeroed() }];

    for ch in &utf16 {
        inputs[0].type_ = INPUT_KEYBOARD;
        unsafe {
            *inputs[0].u.ki_mut() = KEYBDINPUT {
                wVk: 0,
                wScan: *ch,
                dwFlags: KEYEVENTF_UNICODE,
                time: 0,
                dwExtraInfo: 0,
            };
        }

        inputs[1].type_ = INPUT_KEYBOARD;
        unsafe {
            *inputs[1].u.ki_mut() = KEYBDINPUT {
                wVk: 0,
                wScan: *ch,
                dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            };
        }

        let sent = unsafe { SendInput(2, inputs.as_mut_ptr(), std::mem::size_of::<INPUT>() as i32) };
        if sent != 2 {
            anyhow::bail!("SendInput failed: sent {} of 2 events", sent);
        }
    }

    log::debug("input", &format!("Injected {} chars as keyboard input", utf16.len()));
    Ok(())
}
