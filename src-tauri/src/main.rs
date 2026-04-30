#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use vrc_chat_tool::config;
use vrc_chat_tool::audio;
use vrc_chat_tool::speech;
use vrc_chat_tool::osc;

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
fn save_config(cfg: config::AppConfig) -> Result<(), String> {
    if let Err(e) = cfg.save() {
        return Err(format!("Failed to save config: {}", e));
    }
    *CURRENT_CONFIG.lock().unwrap() = Some(cfg);
    Ok(())
}

#[tauri::command]
fn list_audio_devices() -> Result<Vec<audio::capture::AudioDeviceInfo>, String> {
    audio::capture::AudioCapture::list_devices()
        .map_err(|e| format!("Failed to list devices: {}", e))
}

#[tauri::command]
fn start_recording(app: tauri::AppHandle, device_index: Option<usize>) -> Result<(), String> {
    // 1. Load config
    let cfg = CURRENT_CONFIG
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Config not loaded".to_string())?;

    // 2. Validate credentials
    if cfg.tencent_app_id.is_empty()
        || cfg.tencent_secret_id.is_empty()
        || cfg.tencent_secret_key.is_empty()
    {
        return Err("Please configure Tencent Cloud credentials first".to_string());
    }

    SHOULD_STOP.store(false, Ordering::SeqCst);

    // 3. Spawn background thread for the recording pipeline
    thread::spawn(move || {
        emit_log(&app, "info", "recorder", "Recording started");
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

            // Show typing indicator in VRChat
            let osc_typing = osc::sender::OscSender::new(cfg.osc_host.clone(), cfg.osc_port);
            let _ = osc_typing.send_typing(true);

            // Build recognizer
            let recognizer = speech::streaming::StreamingRecognizer::new(
                cfg.tencent_app_id.clone(),
                cfg.tencent_secret_id.clone(),
                cfg.tencent_secret_key.clone(),
            );

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
                    eprintln!("Audio capture error: {}", e);
                }
            });

            emit_log(&app, "debug", "audio", "Audio capture stream opened");

            // Run streaming ASR in tokio runtime — sends chunks in real-time,
            // emits partial results via callback, returns final accumulated text
            let recognized_text = rt.block_on(async {
                let app_sentence = app.clone();
                let osc_for_sentence = Arc::new(osc::sender::OscSender::with_config(
                    cfg.osc_host.clone(), cfg.osc_port,
                    cfg.osc_line_count, cfg.osc_retention_secs, cfg.osc_remove_period,
                ));
                let osc_for_partial = osc_for_sentence.clone();
                let osc_s = osc_for_sentence.clone();
                recognizer.recognize_pcm_stream(
                    pcm_rx,
                    stop_signal_for_asr,
                    16000,
                    move |partial_text: &str| {
                        let _ = app_for_partial.emit_all("recording-partial", partial_text.to_string());
                        let _ = osc_for_partial.send_partial(partial_text);
                    },
                    move |sentence_text: &str| {
                        let _ = app_sentence.emit_all("recording-sentence", sentence_text.to_string());
                        let _ = osc_s.send_chatbox(sentence_text);
                        history::add_entry(sentence_text, "asr");
                    },
                ).await
            })?;

            // Wait for capture thread to finish
            let _ = capture_thread.join();

            // Final OSC: just turn off typing indicator (sentences already sent in real-time)
            let osc = osc::sender::OscSender::new(cfg.osc_host.clone(), cfg.osc_port);
            osc.send_typing(false)?;

            Ok(recognized_text)
        })();

        match result {
            Ok(text) => {
                emit_log(&app, "info", "asr", &format!("Recognition result: {}", text));
                let _ = app.emit_all("recording-complete", text);
            }
            Err(e) => {
                emit_log(&app, "error", "recorder", &format!("Error: {}", e));
                let _ = app.emit_all("recording-error", format!("{}", e));
            }
        }
    });

    Ok(())
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

// --- Main Entry ---
fn main() {
    // Check for E2E test mode (BEFORE Tauri init)
    if std::env::args().any(|a| a == "--e2e") {
        e2e_server::run_e2e_server().expect("E2E server failed");
        return;
    }

    // Load config on startup
    let config = config::AppConfig::load().unwrap_or_default();
    *CURRENT_CONFIG.lock().unwrap() = Some(config);

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
