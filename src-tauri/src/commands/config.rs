use std::sync::Arc;
use vrc_chat_tool::config;
use vrc_chat_tool::state;
use vrc_chat_tool::trigger;
use vrc_chat_tool::hotkey;
use vrc_chat_tool::log;

#[tauri::command]
pub fn get_config() -> Option<config::AppConfig> {
    state::CURRENT_CONFIG.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_config(app: tauri::AppHandle, config: config::AppConfig) -> Result<(), String> {
    let old_config = state::CURRENT_CONFIG.lock().unwrap().clone();
    let hotkey_was_enabled = old_config.as_ref().map(|c| c.global_hotkey_enabled).unwrap_or(false);
    let trigger_was_enabled = old_config.as_ref().map(|c| c.trigger_listener_enabled).unwrap_or(false);

    if let Err(e) = config.save() {
        return Err(format!("Failed to save config: {}", e));
    }
    *state::CURRENT_CONFIG.lock().unwrap() = Some(config.clone());

    if config.global_hotkey_enabled && !hotkey_was_enabled {
        hotkey::start(app.clone());
    } else if !config.global_hotkey_enabled && hotkey_was_enabled {
        hotkey::stop();
    }

    if config.trigger_listener_enabled && !trigger_was_enabled {
        let can_start = config.trigger_stt_provider == "local_embedded" || config.trigger_stt_provider == "local_embedded_hybrid" || !config.local_stt_url.is_empty();
        if can_start {
            log::info("main", "Trigger listener enabled, starting...");
            trigger::start_trigger_listener(Arc::new(config.clone()));
        }
    } else if !config.trigger_listener_enabled && trigger_was_enabled {
        log::info("main", "Trigger listener disabled, stopping...");
        trigger::stop_capture();
    }

    Ok(())
}

#[tauri::command]
pub fn reset_config() -> Result<config::AppConfig, String> {
    let default_config = config::AppConfig::default();
    if let Err(e) = default_config.save() {
        return Err(format!("Failed to reset config: {}", e));
    }
    *state::CURRENT_CONFIG.lock().unwrap() = Some(default_config.clone());
    Ok(default_config)
}

#[tauri::command]
pub fn get_tencent_credentials() -> Result<config::TencentCredentials, String> {
    let cfg = state::CURRENT_CONFIG.lock().unwrap();
    let credentials_file = cfg.as_ref()
        .map(|c| c.tencent_credentials_file.clone())
        .unwrap_or_else(|| "tencent_credentials.yaml".to_string());
    Ok(config::TencentCredentials::load(&credentials_file))
}

#[tauri::command]
pub fn save_tencent_credentials(app_id: String, secret_id: String, secret_key: String) -> Result<(), String> {
    let cfg = state::CURRENT_CONFIG.lock().unwrap();
    let credentials_file = cfg.as_ref()
        .map(|c| c.tencent_credentials_file.clone())
        .unwrap_or_else(|| "tencent_credentials.yaml".to_string());
    let creds = config::TencentCredentials { app_id, secret_id, secret_key };
    creds.save(&credentials_file).map_err(|e| format!("Failed to save: {}", e))
}
