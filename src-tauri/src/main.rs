#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use vrc_chat_tool::config;
use vrc_chat_tool::audio;
use vrc_chat_tool::speech;
use vrc_chat_tool::osc;
use vrc_chat_tool::trigger;
use vrc_chat_tool::hotkey;
use vrc_chat_tool::log;

mod e2e_server;
mod history;

use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::thread;
use tauri::Manager;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// --- Global State ---
static CURRENT_CONFIG: Mutex<Option<config::AppConfig>> = Mutex::new(None);
static SHOULD_STOP: AtomicBool = AtomicBool::new(false);
static IS_RECORDING: AtomicBool = AtomicBool::new(false);

// --- Log System ---
static LOG_BUFFER: Mutex<Vec<LogEntry>> = Mutex::new(Vec::new());
const MAX_LOG_ENTRIES: usize = 200;

#[derive(Clone, serde::Serialize)]
struct LogEntry {
    timestamp: u64,
    level: String,
    message: String,
    module: String,
}

fn emit_log(app: &tauri::AppHandle, level: &str, module: &str, message: &str) {
    let entry = LogEntry {
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
        level: level.to_string(),
        message: message.to_string(),
        module: module.to_string(),
    };

    {
        let mut buf = LOG_BUFFER.lock().unwrap();
        buf.push(entry.clone());
        if buf.len() > MAX_LOG_ENTRIES {
            buf.remove(0);
        }
    }

    let _ = app.emit_all("log-entry", entry);
}

// --- Recording Test Commands ---

fn recordings_dir() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    dir.push("tmp");
    dir.push("recordings");
    let _ = fs::create_dir_all(&dir);
    dir
}

#[tauri::command]
fn save_test_recording(pcm_data: Vec<u8>, filename: String) -> Result<String, String> {
    let dir = recordings_dir();
    let filepath = dir.join(&filename);

    let header = audio::capture::create_wav_header(pcm_data.len() as u32, 16000, 16, 1);
    let mut wav_data = header;
    wav_data.extend_from_slice(&pcm_data);

    fs::write(&filepath, &wav_data)
        .map_err(|e| format!("Failed to save: {}", e))?;

    Ok(filepath.to_string_lossy().to_string())
}

#[derive(serde::Serialize)]
struct RecordingInfo {
    filename: String,
    path: String,
    size_bytes: u64,
    created: String,
}

#[tauri::command]
fn list_test_recordings() -> Result<Vec<RecordingInfo>, String> {
    let dir = recordings_dir();
    let mut recordings = Vec::new();

    if dir.exists() {
        for entry in fs::read_dir(&dir).map_err(|e| format!("Failed: {}", e))? {
            let entry = entry.map_err(|e| format!("Failed: {}", e))?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "wav") {
                if let Ok(metadata) = entry.metadata() {
                    let created = metadata.created()
                        .map(|t| format!("{:?}", t))
                        .unwrap_or_else(|_| "unknown".to_string());
                    recordings.push(RecordingInfo {
                        filename: entry.file_name().to_string_lossy().to_string(),
                        path: path.to_string_lossy().to_string(),
                        size_bytes: metadata.len(),
                        created,
                    });
                }
            }
        }
    }

    recordings.sort_by(|a, b| b.filename.cmp(&a.filename));
    Ok(recordings)
}

#[tauri::command]
fn delete_test_recording(filename: String) -> Result<(), String> {
    let filepath = recordings_dir().join(&filename);
    fs::remove_file(&filepath)
        .map_err(|e| format!("Failed to delete: {}", e))
}

#[tauri::command]
fn get_recent_logs() -> Vec<LogEntry> {
    LOG_BUFFER.lock().unwrap().clone()
}

#[tauri::command]
fn clear_logs() {
    LOG_BUFFER.lock().unwrap().clear();
}

#[tauri::command]
fn start_test_recording(app: tauri::AppHandle, device_index: Option<usize>) -> Result<(), String> {
    SHOULD_STOP.store(false, Ordering::SeqCst);

    let app_clone = app.clone();
    thread::spawn(move || {
        let capture = match device_index {
            Some(idx) => audio::capture::AudioCapture::new_by_index(idx),
            None => audio::capture::AudioCapture::new(),
        };

        let capture = match capture {
            Ok(c) => c,
            Err(e) => {
                let _ = app_clone.emit_all("recording-error", format!("{}", e));
                emit_log(&app_clone, "error", "audio", &format!("Failed to open device: {}", e));
                return;
            }
        };

        let pcm_buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
        let pcm_buffer_clone = pcm_buffer.clone();
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_signal_clone = stop_signal.clone();
        let app_emit = app_clone.clone();

        let _ = app_clone.emit_all("recording-started", "");
        emit_log(&app_clone, "info", "audio", "Test recording started");

        let capture_thread = thread::spawn(move || {
            let result = capture.capture_streaming(
                move |chunk: Vec<u8>| {
                    pcm_buffer_clone.lock().unwrap().extend_from_slice(&chunk);
                    let sum: f64 = chunk.chunks(2)
                        .map(|pair| {
                            let sample = i16::from_le_bytes([pair[0], pair[1]]) as f64;
                            sample * sample
                        }).sum();
                    let rms = (sum / (chunk.len() / 2) as f64).sqrt();
                    let volume = ((rms / 32768.0).min(1.0)) as f32;
                    let _ = app_emit.emit_all("volume-update", volume);
                },
                stop_signal_clone,
            );
            if let Err(e) = result {
                eprintln!("Audio capture error: {}", e);
            }
        });

        while !SHOULD_STOP.load(Ordering::SeqCst) {
            thread::sleep(std::time::Duration::from_millis(100));
        }
        stop_signal.store(true, Ordering::SeqCst);
        let _ = capture_thread.join();

        let pcm_data = pcm_buffer.lock().unwrap().clone();
        if pcm_data.is_empty() {
            emit_log(&app_clone, "warn", "audio", "No audio data captured in test recording");
            let _ = app_clone.emit_all("recording-error", "No audio data captured");
            return;
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH).unwrap().as_secs();
        let filename = format!("recording_{}.wav", timestamp);

        match save_test_recording(pcm_data, filename.clone()) {
            Ok(path) => {
                emit_log(&app_clone, "info", "audio", &format!("Saved: {}", filename));
                let _ = app_clone.emit_all("recording-complete", path);
            }
            Err(e) => {
                emit_log(&app_clone, "error", "audio", &format!("Save failed: {}", e));
                let _ = app_clone.emit_all("recording-error", e);
            }
        }
    });

    Ok(())
}

// --- Tauri Commands ---

#[tauri::command]
fn get_config() -> Option<config::AppConfig> {
    CURRENT_CONFIG.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(app: tauri::AppHandle, config: config::AppConfig) -> Result<(), String> {
    // Check if hotkey setting changed
    let hotkey_was_enabled = CURRENT_CONFIG.lock().unwrap()
        .as_ref().map(|c| c.global_hotkey_enabled).unwrap_or(false);

    if let Err(e) = config.save() {
        return Err(format!("Failed to save config: {}", e));
    }
    *CURRENT_CONFIG.lock().unwrap() = Some(config.clone());

    // Start or stop global hotkey based on new config
    if config.global_hotkey_enabled && !hotkey_was_enabled {
        hotkey::start(app.clone());
    } else if !config.global_hotkey_enabled && hotkey_was_enabled {
        hotkey::stop();
    }

    Ok(())
}

#[tauri::command]
fn reset_config() -> Result<config::AppConfig, String> {
    let default_config = config::AppConfig::default();
    if let Err(e) = default_config.save() {
        return Err(format!("Failed to reset config: {}", e));
    }
    *CURRENT_CONFIG.lock().unwrap() = Some(default_config.clone());
    Ok(default_config)
}

#[tauri::command]
fn list_audio_devices() -> Result<Vec<audio::capture::AudioDeviceInfo>, String> {
    audio::capture::AudioCapture::list_devices()
        .map_err(|e| format!("Failed to list devices: {}", e))
}

/// Inner recording pipeline (shared between Tauri command and trigger).
/// Spawns a background thread, returns immediately.
fn start_recording_inner(
    app: tauri::AppHandle,
    device_index: Option<usize>,
    cfg: &config::AppConfig,
) -> Result<(), String> {
    // Prevent concurrent recordings (double-start from trigger + manual click)
    if IS_RECORDING.swap(true, Ordering::SeqCst) {
        return Err("Recording already in progress".to_string());
    }

    // Load credentials if using Tencent Cloud
    let tencent_creds = if cfg.asr_provider == "tencent" {
        let creds = config::TencentCredentials::load(&cfg.tencent_credentials_file);
        if creds.app_id.is_empty() || creds.secret_id.is_empty() || creds.secret_key.is_empty() {
            IS_RECORDING.store(false, Ordering::SeqCst);
            return Err("Please configure Tencent Cloud credentials first".to_string());
        }
        Some(creds)
    } else {
        None
    };

    // Stop trigger listener's audio capture to free the audio device
    // (trigger listener's cpal stream competes with recording's cpal stream)
    let trigger_was_active = trigger::is_active();
    if trigger_was_active {
        log::debug("recorder", "Stopping trigger capture before recording");
        trigger::stop_capture();
        thread::sleep(std::time::Duration::from_millis(200));
    }

    SHOULD_STOP.store(false, Ordering::SeqCst);

    // Pause trigger listener's audio sending — main pipeline handles STT during recording
    trigger::pause_audio();

    let cfg = cfg.clone();
    let trigger_stop_partial = cfg.trigger_stop.clone();
    let trigger_stop_sentence = cfg.trigger_stop.clone();
    let local_stt_url = cfg.local_stt_url.clone();
    let trigger_start = cfg.trigger_start.clone();
    let trigger_stop_phrase = cfg.trigger_stop.clone();
    thread::spawn(move || {
        log::info("recorder", "Recording started");
        let result: Result<String, anyhow::Error> = (|| -> anyhow::Result<String> {
            // Create audio capture
            let capture = match device_index {
                Some(idx) => audio::capture::AudioCapture::new_by_index(idx)?,
                None => audio::capture::AudioCapture::new()?,
            };

            // Create channel for streaming audio chunks from capture to ASR
            let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
            let stop_signal = Arc::new(AtomicBool::new(false));

            // Bridge: monitor global SHOULD_STOP and propagate to local stop_signal
            let s_sig = stop_signal.clone();
            thread::spawn(move || {
                while !SHOULD_STOP.load(Ordering::SeqCst) {
                    thread::sleep(std::time::Duration::from_millis(100));
                }
                s_sig.store(true, Ordering::SeqCst);
                SHOULD_STOP.store(false, Ordering::SeqCst);
            });

            let stop_signal_for_capture = stop_signal.clone();
            let stop_signal_for_asr = stop_signal.clone();

            // Clone app for use in capture callback and partial result callback
            let app_for_volume = app.clone();
            let app_for_partial = app.clone();

            // Emit started event
            let _ = app.emit_all("recording-started", "");

            // Show typing indicator in VRChat (if OSC enabled)
            if cfg.osc_enabled {
                let osc_typing = osc::sender::OscSender::new(cfg.osc_host.clone(), cfg.osc_port);
                let _ = osc_typing.send_typing(true);
            }

            // Build recognizer (Tencent Cloud or Local STT)
            let recognizer = if cfg.asr_provider == "local" {
                speech::recognizer::Recognizer::Local(
                    speech::local::LocalRecognizer::new(cfg.local_stt_url.clone())
                )
            } else {
                let c = tencent_creds.as_ref().unwrap();
                speech::recognizer::Recognizer::Tencent(
                    speech::streaming::StreamingRecognizer::new(
                        c.app_id.clone(),
                        c.secret_id.clone(),
                        c.secret_key.clone(),
                    )
                )
            };

            let rt = tokio::runtime::Runtime::new()?;

            // Spawn audio capture in a sub-thread
            let capture_thread = thread::spawn(move || {
                let result = capture.capture_streaming(
                    move |chunk: Vec<u8>| {
                        // Calculate and emit volume (simple RMS-based)
                        let sum: f64 = chunk
                            .chunks(2)
                            .map(|pair| {
                                let sample = i16::from_le_bytes([pair[0], pair[1]]) as f64;
                                sample * sample
                            })
                            .sum();
                        let rms = (sum / (chunk.len() / 2) as f64).sqrt();
                        let volume = ((rms / 32768.0).min(1.0)) as f32;
                        let _ = app_for_volume.emit_all("volume-update", volume);

                        // Forward audio chunk to ASR via channel
                        let _ = pcm_tx.blocking_send(chunk);
                    },
                    stop_signal_for_capture,
                );
                if let Err(e) = result {
                    log::error("audio", &format!("Capture error: {}", e));
                }
            });

            log::debug("audio", "Audio capture stream opened");

            // Run streaming ASR via unified recognizer
            let osc_enabled = cfg.osc_enabled;
            let recognized_text = rt.block_on(async {
                let app_sentence = app.clone();
                let osc_for_sentence = if osc_enabled {
                    Some(Arc::new(osc::sender::OscSender::with_config(
                        cfg.osc_host.clone(), cfg.osc_port,
                        cfg.osc_line_count, cfg.osc_retention_secs, cfg.osc_remove_period,
                    )))
                } else {
                    None
                };
                let osc_for_partial = osc_for_sentence.clone();
                let osc_s = osc_for_sentence.clone();
                recognizer.recognize_pcm_stream(
                    pcm_rx,
                    stop_signal_for_asr,
                    move |partial_text: &str| {
                        let _ = app_for_partial.emit_all("recording-partial", partial_text.to_string());
                        if let Some(ref osc) = osc_for_partial {
                            let _ = osc.send_partial(partial_text);
                        }
                        // Check stop trigger phrase in ASR output
                        if trigger::matches_trigger(partial_text, &trigger_stop_partial) {
                            log::info("recorder", &format!("STOP detected in partial: '{}'", partial_text));
                            SHOULD_STOP.store(true, Ordering::SeqCst);
                        }
                    },
                    move |sentence_text: &str| {
                        let _ = app_sentence.emit_all("recording-sentence", sentence_text.to_string());
                        if let Some(ref osc) = osc_s {
                            let _ = osc.send_chatbox(sentence_text);
                        }
                        history::add_entry(sentence_text, "asr");
                        // Check stop trigger phrase in sentence output
                        if trigger::matches_trigger(sentence_text, &trigger_stop_sentence) {
                            log::info("recorder", &format!("STOP detected in sentence: '{}'", sentence_text));
                            SHOULD_STOP.store(true, Ordering::SeqCst);
                        }
                    },
                ).await
            })?;

            // Wait for capture thread to finish
            let _ = capture_thread.join();

            if osc_enabled {
                let osc = osc::sender::OscSender::new(cfg.osc_host.clone(), cfg.osc_port);
                let _ = osc.send_typing(false);
                // Clear the chatbox display after stopping
                let _ = osc.clear_chatbox();
            }

            Ok(recognized_text)
        })();

        // Always resume trigger listener audio after recording stops
        trigger::resume_audio();

        // Restart trigger listener if it was stopped for this recording
        if trigger_was_active {
            log::debug("recorder", "Restarting trigger listener");
            trigger::start_trigger_listener(Arc::new(config::AppConfig {
                trigger_start,
                trigger_stop: trigger_stop_phrase,
                local_stt_url,
                ..config::AppConfig::load().unwrap_or_default()
            }));
        }

        IS_RECORDING.store(false, Ordering::SeqCst);

        match result {
            Ok(text) => {
                log::info("asr", &format!("Recognition result: {}", text));
                let _ = app.emit_all("recording-complete", text);
            }
            Err(e) => {
                let msg = format!("{}", e);
                log::error("recorder", &format!("Error: {}", msg));
                let _ = app.emit_all("recording-error", msg);
            }
        }
    });

    Ok(())
}

#[tauri::command]
fn start_recording(app: tauri::AppHandle, device_index: Option<usize>) -> Result<(), String> {
    let cfg = CURRENT_CONFIG
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Config not loaded".to_string())?;

    start_recording_inner(app, device_index, &cfg)
}

#[tauri::command]
fn get_recognition_history() -> Vec<history::HistoryEntry> {
    history::get_recent(100)
}

#[tauri::command]
fn clear_recognition_history() {
    history::clear_all();
}

#[tauri::command]
fn stop_recording() -> Result<(), String> {
    SHOULD_STOP.store(true, Ordering::SeqCst);
    Ok(())
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
    // Initialize file logger
    log::init("tmp/app.log");

    // Check for E2E test mode (BEFORE Tauri init)
    if std::env::args().any(|a| a == "--e2e") {
        e2e_server::run_e2e_server().expect("E2E server failed");
        return;
    }

    // Load config on startup
    let config = config::AppConfig::load().unwrap_or_default();
    *CURRENT_CONFIG.lock().unwrap() = Some(config.clone());

    // Enable global hotkey (F10) at startup if configured
    if config.global_hotkey_enabled {
        log::debug("main", "Hotkey enabled, deferring to Tauri setup");
    }

    // Start always-on trigger listener (local STT for voice commands)
    // Always start if local_stt_url is configured, regardless of asr_provider
    if !config.local_stt_url.is_empty() {
        log::info("main", &format!("Starting trigger listener, STT URL: {}", config.local_stt_url));
        trigger::start_trigger_listener(Arc::new(config.clone()));
    } else {
        log::info("main", "No local STT URL configured, trigger listener disabled");
    }

    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle();

            // Start global hotkey (F10) if enabled in config
            {
                let cfg = CURRENT_CONFIG.lock().unwrap();
                if let Some(ref c) = *cfg {
                    if c.global_hotkey_enabled {
                        log::info("main", "Starting F10 global hotkey");
                        hotkey::start(app_handle.clone());
                    } else {
                        log::info("main", "Global hotkey disabled in config");
                    }
                }
            }

            // Poll trigger detection and events in a background thread
            let app_handle = app.handle();
            log::info("main", "Starting trigger polling thread (200ms)");
            thread::spawn(move || {
                loop {
                    // Emit trigger listener's heard texts for UI echo
                    for text in trigger::drain_heard_texts() {
                        let _ = app_handle.emit_all("trigger-heard", text);
                    }

                    // Emit trigger listener's volume for UI meter
                    // Only when not recording (main pipeline handles volume during recording)
                    if !trigger::is_paused() {
                        let vol = trigger::latest_trigger_volume();
                        let _ = app_handle.emit_all("volume-update", vol);
                    }

                    if trigger::is_trigger_detected() {
                        let text = trigger::last_trigger_text();
                        log::info("trigger", &format!("Action: {}", text));
                        match text.as_str() {
                            "stop" => {
                                SHOULD_STOP.store(true, Ordering::SeqCst);
                            }
                            _ => {
                                // start: invoke recording with proper error handling
                                let cfg = CURRENT_CONFIG.lock().unwrap().clone();
                                if let Some(cfg) = cfg {
                                    match start_recording_inner(
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
            get_config,
            save_config,
            reset_config,
            list_audio_devices,
            start_recording,
            stop_recording,
            save_test_recording,
            list_test_recordings,
            delete_test_recording,
            get_recent_logs,
            clear_logs,
            start_test_recording,
            get_recognition_history,
            clear_recognition_history,
            get_saved_device_index,
            save_device_index,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
