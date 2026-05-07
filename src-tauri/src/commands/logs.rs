use vrc_chat_tool::state;

#[tauri::command]
pub fn get_recent_logs() -> Vec<state::LogEntry> {
    state::LOG_BUFFER.lock().unwrap().clone()
}

#[tauri::command]
pub fn clear_logs() {
    state::LOG_BUFFER.lock().unwrap().clear();
}
