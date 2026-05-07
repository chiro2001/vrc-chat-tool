#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use vrc_chat_tool::config;
use vrc_chat_tool::audio;
use vrc_chat_tool::speech;
use vrc_chat_tool::osc;
use vrc_chat_tool::trigger;
use vrc_chat_tool::hotkey;
use vrc_chat_tool::log;
use vrc_chat_tool::state;

mod e2e_server;
mod history;
mod commands;

use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::thread;
use tauri::Manager;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

// --- Log System (emit depends on tauri::AppHandle) ---
fn emit_log(app: &tauri::AppHandle, level: &str, module: &str, message: &str) {
    let entry = state::LogEntry {
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        level: level.to_string(),
        message: message.to_string(),
        module: module.to_string(),
    };

    {
        let mut buf = state::LOG_BUFFER.lock().unwrap();
        buf.push(entry.clone());
        if buf.len() > state::MAX_LOG_ENTRIES {
            buf.remove(0);
        }
    }

    let _ = app.emit_all("log-entry", entry);
}

#[tauri::command]
fn save_test_recording(pcm_data: Vec<u8>, filename: String) -> Result<String, String> {
    let dir = state::recordings_dir();
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

// --- STT Model Status & Download ---

#[derive(Clone, serde::Serialize)]
struct SttModelStatus {
    exists: bool,
    model_name: String,
    missing_files: Vec<String>,
    model_dir: String,
}

#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    phase: String,
    current: u64,
    total: u64,
}

/// A Sherpa-ONNX model available for download.
#[derive(Clone, serde::Serialize)]
struct AvailableModel {
    /// Model release name (used in URL and directory name).
    name: &'static str,
    /// Human-readable label for UI.
    display_name: &'static str,
    /// Approximate download size in bytes.
    size_bytes: u64,
    /// ONNX file names inside the archive.
    files: ModelFiles,
}

#[derive(Clone, serde::Serialize)]
struct ModelFiles {
    encoder: &'static str,
    decoder: &'static str,
    joiner: &'static str,
    tokens: &'static str,
}

/// Registry of supported models for download.
const SUPPORTED_MODELS: &[AvailableModel] = &[
    AvailableModel {
        name: "sherpa-onnx-streaming-zipformer-zh-14M-2023-02-23",
        display_name: "中文 Zipformer 14M (2023) — ~74 MB",
        size_bytes: 74_004_050,
        files: ModelFiles {
            encoder: "encoder-epoch-99-avg-1.onnx",
            decoder: "decoder-epoch-99-avg-1.onnx",
            joiner: "joiner-epoch-99-avg-1.onnx",
            tokens: "tokens.txt",
        },
    },
    AvailableModel {
        name: "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20",
        display_name: "中英双语 Zipformer (2023) — ~72 MB",
        size_bytes: 72_000_000,
        files: ModelFiles {
            encoder: "encoder-epoch-99-avg-1.onnx",
            decoder: "decoder-epoch-99-avg-1.onnx",
            joiner: "joiner-epoch-99-avg-1.onnx",
            tokens: "tokens.txt",
        },
    },
    AvailableModel {
        name: "sherpa-onnx-streaming-zipformer-ctc-small-2024-03-18",
        display_name: "CTC Small 多语言 (2024) — ~176 MB",
        size_bytes: 184_604_253,
        files: ModelFiles {
            encoder: "model.int8.onnx",
            decoder: "tokens.txt",     // CTC uses single model file
            joiner: "",
            tokens: "tokens.txt",
        },
    },
];

#[tauri::command]
fn list_test_recordings() -> Result<Vec<RecordingInfo>, String> {
    let dir = state::recordings_dir();
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
    let filepath = state::recordings_dir().join(&filename);
    fs::remove_file(&filepath)
        .map_err(|e| format!("Failed to delete: {}", e))
}

#[tauri::command]
fn start_test_recording(app: tauri::AppHandle, device_index: Option<usize>) -> Result<(), String> {
    state::SHOULD_STOP.store(false, Ordering::SeqCst);

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

        while !state::SHOULD_STOP.load(Ordering::SeqCst) {
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
    if state::IS_RECORDING.swap(true, Ordering::SeqCst) {
        return Err("Recording already in progress".to_string());
    }

    // Load credentials if using Tencent Cloud
    let tencent_creds = if cfg.asr_provider == "tencent" {
        let creds = config::TencentCredentials::load(&cfg.tencent_credentials_file);
        if creds.app_id.is_empty() || creds.secret_id.is_empty() || creds.secret_key.is_empty() {
            state::IS_RECORDING.store(false, Ordering::SeqCst);
            return Err("Please configure Tencent Cloud credentials first".to_string());
        }
        Some(creds)
    } else {
        None
    };

    state::SHOULD_STOP.store(false, Ordering::SeqCst);

    // Pause trigger listener's STT audio sending during recording
    // (trigger capture keeps running for volume display; WebSocket stays connected)
    trigger::pause_audio();

    let cfg = cfg.clone();
    let trigger_stop_partial = cfg.trigger_stop.clone();
    let trigger_stop_sentence = cfg.trigger_stop.clone();
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

            // Bridge: monitor global state::SHOULD_STOP and propagate to local stop_signal
            let s_sig = stop_signal.clone();
            thread::spawn(move || {
                while !state::SHOULD_STOP.load(Ordering::SeqCst) {
                    thread::sleep(std::time::Duration::from_millis(100));
                }
                s_sig.store(true, Ordering::SeqCst);
                state::SHOULD_STOP.store(false, Ordering::SeqCst);
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

            // Build recognizer (Tencent Cloud, Local STT, or Local Embedded)
            let recognizer = if cfg.asr_provider == "local" {
                speech::recognizer::Recognizer::Local(
                    speech::local::LocalRecognizer::new(cfg.local_stt_url.clone())
                )
                } else if cfg.asr_provider == "local_embedded" {
                    let engine_cfg = speech::local_embedded::LocalEmbeddedRecognizer::from_config_file(&cfg.stt_config_path)
                        .map_err(|e| anyhow::anyhow!(
                            "无法加载 STT 模型: {}\n\n请检查:\n\
                             1. 设置中\"STT 模型配置路径\"是否正确\n\
                             2. 模型文件是否存在（可在设置中下载模型）",
                            e,
                        ))?;
                    speech::recognizer::Recognizer::LocalEmbedded(engine_cfg)
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
                            state::SHOULD_STOP.store(true, Ordering::SeqCst);
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
                            state::SHOULD_STOP.store(true, Ordering::SeqCst);
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

        state::IS_RECORDING.store(false, Ordering::SeqCst);

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
    let cfg = state::CURRENT_CONFIG
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
    state::SHOULD_STOP.store(true, Ordering::SeqCst);
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

// --- STT Model Commands ---

/// Check whether STT model files exist on disk.
#[tauri::command]
fn check_stt_model(stt_config_path: String) -> Result<SttModelStatus, String> {
    eprintln!("[check_stt_model] checking config: {}", stt_config_path);
    let config = match stt_server::Config::from_file(&stt_config_path) {
        Ok(c) => c,
        Err(e) => {
            let err_msg = format!("{}", e);
            eprintln!("[check_stt_model] load failed: {}", err_msg);
            if err_msg.contains("not found") {
                eprintln!("[check_stt_model] creating default stt-config.yaml");
                let default_yaml = include_str!("../../stt-config.yaml");
                if let Err(w) = std::fs::write(&stt_config_path, default_yaml) {
                    return Err(format!("Failed to create default stt-config.yaml: {}", w));
                }
                stt_server::Config::from_file(&stt_config_path)
                    .map_err(|e2| format!("Failed after creating default config: {}", e2))?
            } else {
                return Err(format!("Failed to load STT config: {}", e));
            }
        }
    };

    let target_dir = config.asr_model_path();
    eprintln!("[check_stt_model] target_dir: {:?}", target_dir);
    let required = [
        config.asr.encoder.as_str(),
        config.asr.decoder.as_str(),
        config.asr.joiner.as_str(),
        config.asr.tokens.as_str(),
    ];

    let missing: Vec<String> = required
        .iter()
        .filter(|f| {
            let p = target_dir.join(f);
            let exists = p.exists();
            eprintln!("[check_stt_model] {} -> {}", p.display(), if exists { "OK" } else { "MISSING" });
            !exists
        })
        .map(|f| f.to_string())
        .collect();

    eprintln!("[check_stt_model] missing: {:?}, exists: {}", missing, missing.is_empty());
    Ok(SttModelStatus {
        exists: missing.is_empty(),
        model_name: config.asr.model_name.clone(),
        missing_files: missing,
        model_dir: target_dir.to_string_lossy().to_string(),
    })
}

/// Download STT models in a background thread with progress events.
///
/// Emits:
/// - `stt-model-download-progress` → `{ phase, current, total }`
/// - `stt-model-download-complete` → `""`
/// - `stt-model-download-error` → error message string
#[tauri::command]
fn download_stt_model(app: tauri::AppHandle, stt_config_path: String, force: bool) -> Result<(), String> {
    let config = stt_server::Config::from_file(&stt_config_path)
        .map_err(|e| format!("Failed to load STT config: {}", e))?;

    std::thread::spawn(move || {
        let app = app.clone();
        let result = stt_server::download_models_with_progress(
            &config,
            force,
            true, // no_punct — punctuation disabled for integrated mode
            &|phase, current, total| {
                let _ = app.emit_all("stt-model-download-progress", DownloadProgress {
                    phase: phase.to_string(),
                    current,
                    total,
                });
            },
        );

        match result {
            Ok(()) => {
                let _ = app.emit_all("stt-model-download-complete", "");
            }
            Err(e) => {
                let _ = app.emit_all("stt-model-download-error", format!("{}", e));
            }
        }
    });

    Ok(())
}

/// List available STT models that can be selected for download.
#[tauri::command]
fn get_available_models() -> Vec<AvailableModel> {
    SUPPORTED_MODELS.to_vec()
}

/// Switch the STT model by updating stt-config.yaml.
#[tauri::command]
fn set_stt_model(stt_config_path: String, model_name: String) -> Result<(), String> {
    let model = SUPPORTED_MODELS.iter()
        .find(|m| m.name == model_name)
        .ok_or_else(|| format!("Unknown model: {}", model_name))?;

    // Read existing config, update, write back
    let content = std::fs::read_to_string(&stt_config_path)
        .map_err(|e| format!("Failed to read {}: {}", stt_config_path, e))?;

    let mut config: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|e| format!("Failed to parse YAML: {}", e))?;

    if let Some(asr) = config.get_mut("asr") {
        asr["model_name"] = serde_yaml::Value::String(model.name.to_string());
        asr["encoder"] = serde_yaml::Value::String(model.files.encoder.to_string());
        asr["decoder"] = serde_yaml::Value::String(model.files.decoder.to_string());
        asr["joiner"] = serde_yaml::Value::String(model.files.joiner.to_string());
        asr["tokens"] = serde_yaml::Value::String(model.files.tokens.to_string());
    }

    let new_yaml = serde_yaml::to_string(&config)
        .map_err(|e| format!("Failed to serialize YAML: {}", e))?;

    std::fs::write(&stt_config_path, new_yaml)
        .map_err(|e| format!("Failed to write {}: {}", stt_config_path, e))?;

    Ok(())
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
    *state::CURRENT_CONFIG.lock().unwrap() = Some(config.clone());

    // Enable global hotkey (F10) at startup if configured
    if config.global_hotkey_enabled {
        log::debug("main", "Hotkey enabled, deferring to Tauri setup");
    }

    // Start always-on trigger listener if enabled AND provider configured
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

            // Start global hotkey (F10) if enabled in config
            {
                let cfg = state::CURRENT_CONFIG.lock().unwrap();
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
                let mut last_stt_status = String::new();
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

                    // Emit STT status only when trigger listener is enabled in config
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

                    // Auto-restart trigger listener if it died and still enabled
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
                                // start: invoke recording with proper error handling
                                let cfg = state::CURRENT_CONFIG.lock().unwrap().clone();
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
            commands::config::get_config,
            commands::config::save_config,
            commands::config::reset_config,
            commands::config::get_tencent_credentials,
            commands::config::save_tencent_credentials,
            list_audio_devices,
            start_recording,
            stop_recording,
            save_test_recording,
            list_test_recordings,
            delete_test_recording,
            commands::logs::get_recent_logs,
            commands::logs::clear_logs,
            start_test_recording,
            get_recognition_history,
            clear_recognition_history,
            get_saved_device_index,
            save_device_index,
            check_stt_model,
            download_stt_model,
            get_available_models,
            set_stt_model,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
