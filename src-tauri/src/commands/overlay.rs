use tauri::Manager;

/// Toggle the overlay window visibility.
#[tauri::command]
pub fn toggle_overlay_window(app: tauri::AppHandle) -> Result<bool, String> {
    if let Some(window) = app.get_window("overlay") {
        let visible = window.is_visible().unwrap_or(false);
        if visible {
            let _ = window.hide();
            Ok(false)
        } else {
            let _ = window.show();
            Ok(true)
        }
    } else {
        Err("Overlay window not found".into())
    }
}

/// Get current overlay window visibility.
#[tauri::command]
pub fn is_overlay_visible(app: tauri::AppHandle) -> Result<bool, String> {
    if let Some(window) = app.get_window("overlay") {
        Ok(window.is_visible().unwrap_or(false))
    } else {
        Ok(false)
    }
}
