#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{atomic::Ordering, Arc};
use std::thread;
use tauri::Manager;
use vrc_chat_tool::config;
use vrc_chat_tool::trigger;
use vrc_chat_tool::hotkey;
use vrc_chat_tool::log;
use vrc_chat_tool::state;

mod e2e_server;
mod history;
mod commands;

// --- Tauri Commands ---

#[tauri::command]
fn get_recognition_history() -> Vec<history::HistoryEntry> {
    history::get_recent(100)
}

#[tauri::command]
fn clear_recognition_history() {
    history::clear_all();
}

#[tauri::command]
fn get_saved_device_index() -> u32 {
    history::get_audio_device_index() as u32
}

#[tauri::command]
fn save_device_index(device_idx: u32) {
    history::set_audio_device_index(device_idx as usize);
}

// --- Main Entry ---
fn main() {
    log::init("tmp/app.log");

    if std::env::args().any(|a| a == "--e2e") {
        e2e_server::run_e2e_server().expect("E2E server failed");
        return;
    }

    let config = config::AppConfig::load().unwrap_or_default();
    *state::CURRENT_CONFIG.lock().unwrap() = Some(config.clone());

    if config.global_hotkey_enabled {
        log::debug("main", "Hotkey enabled, deferring to Tauri setup");
    }

    if config.trigger_listener_enabled
        && (config.trigger_stt_provider == "local_embedded" || !config.local_stt_url.is_empty())
    {
        log::info("main", &format!(
            "Starting trigger listener (provider: {}, url: {})",
            config.trigger_stt_provider, config.local_stt_url
        ));
        trigger::start_trigger_listener(Arc::new(config.clone()));
    } else if !config.trigger_listener_enabled {
        log::info("main", "Trigger listener disabled in config");
    } else {
        log::info("main", "No STT provider configured, trigger listener disabled");
    }

    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle();

            {
                let cfg = state::CURRENT_CONFIG.lock().unwrap();
                if let Some(ref c) = *cfg {
                    if c.global_hotkey_enabled {
                        log::info("main", "Starting F10 global hotkey");
                        hotkey::start(app_handle.clone());
                    } else {
                        log::info("main", "Global hotkey disabled in config");
                    }

                    if c.vr_controller_enabled {
                        log::info("main", "Starting VR controller listener");
                        vrc_chat_tool::vr::start_controller_listener(app_handle.clone());
                    }
                }
            }

            let app_handle = app.handle();
            log::info("main", "Starting trigger polling thread (200ms)");
            // Start overlay IPC server (for vrc-chat-hud.exe companion)
            vrc_chat_tool::ipc_server::start_overlay_ipc();

            thread::spawn(move || {
                let mut last_stt_status = String::new();
                loop {
                    for text in trigger::drain_heard_texts() {
                        let _ = app_handle.emit_all("trigger-heard", text);
                    }

                    if !trigger::is_paused() {
                        let vol = trigger::latest_trigger_volume();
                        let _ = app_handle.emit_all("volume-update", vol);
                    }

                    {
                        let cfg = state::CURRENT_CONFIG.lock().unwrap();
                        let listener_enabled = cfg.as_ref().map(|c| c.trigger_listener_enabled).unwrap_or(false);
                        if listener_enabled {
                            let stt_status = trigger::stt_status();
                            if stt_status != last_stt_status {
                                last_stt_status = stt_status.clone();
                                let _ = app_handle.emit_all("trigger-stt-status", &stt_status);
                                log::info("trigger", &format!("STT status changed: {}", stt_status));
                            }
                        } else {
                            let disabled_status = "disabled";
                            if last_stt_status != disabled_status {
                                last_stt_status = disabled_status.to_string();
                                let _ = app_handle.emit_all("trigger-stt-status", disabled_status);
                            }
                        }
                    }

                    if !trigger::is_active() && trigger::can_restart() {
                        let cfg = state::CURRENT_CONFIG.lock().unwrap().clone();
                        if let Some(ref c) = cfg {
                            if c.trigger_listener_enabled
                                && (c.trigger_stt_provider == "local_embedded" || !c.local_stt_url.is_empty())
                            {
                                log::warn("trigger", "Listener died, attempting restart");
                                trigger::start_trigger_listener(Arc::new(c.clone()));
                            }
                        }
                    }

                    if trigger::is_trigger_detected() {
                        let text = trigger::last_trigger_text();
                        log::info("trigger", &format!("Action: {}", text));
                        match text.as_str() {
                            "stop" => {
                                state::SHOULD_STOP.store(true, Ordering::SeqCst);
                            }
                            _ => {
                                let cfg = state::CURRENT_CONFIG.lock().unwrap().clone();
                                if let Some(cfg) = cfg {
                                    match commands::recording::start_recording_inner(
                                        app_handle.clone(),
                                        None,
                                        &cfg,
                                    ) {
                                        Ok(()) => {
                                            log::info("trigger", "Recording started via trigger phrase");
                                        }
                                        Err(e) => {
                                            log::error("trigger", &format!("Failed to start: {}", e));
                                            let _ = app_handle.emit_all("recording-error", e);
                                        }
                                    }
                                } else {
                                    log::error("trigger", "Config not loaded, cannot start recording");
                                }
                            }
                        }
                    }
                    thread::sleep(std::time::Duration::from_millis(200));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::config::get_config,
            commands::config::save_config,
            commands::config::reset_config,
            commands::recording::list_audio_devices,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::debug::save_test_recording,
            commands::debug::list_test_recordings,
            commands::debug::delete_test_recording,
            commands::logs::get_recent_logs,
            commands::logs::clear_logs,
            commands::debug::start_test_recording,
            get_recognition_history,
            clear_recognition_history,
            get_saved_device_index,
            save_device_index,
            commands::stt::check_stt_model,
            commands::stt::download_stt_model,
            commands::stt::get_available_models,
            commands::stt::set_stt_model,
            commands::stt::set_stt_backend,
            commands::maintenance::get_models_disk_usage,
            commands::maintenance::delete_downloaded_models,
            commands::maintenance::delete_all_data,
            commands::maintenance::reset_tencent_usage,
            commands::overlay::toggle_overlay_window,
            commands::overlay::is_overlay_visible,
        ])
        .on_window_event(|event| {
            // Close overlay when main window is destroyed
            if let tauri::WindowEvent::Destroyed = event.event() {
                if let Some(overlay) = event.window().app_handle().get_window("overlay") {
                    let _ = overlay.close();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
