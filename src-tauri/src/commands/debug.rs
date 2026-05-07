use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use std::fs;
use tauri::Manager;
use vrc_chat_tool::state;
use vrc_chat_tool::audio;

// --- Helpers ---

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

// --- Types ---

#[derive(serde::Serialize)]
pub struct RecordingInfo {
    pub filename: String,
    pub path: String,
    pub size_bytes: u64,
    pub created: String,
}

// --- Commands ---

#[tauri::command]
pub fn save_test_recording(pcm_data: Vec<u8>, filename: String) -> Result<String, String> {
    let dir = state::recordings_dir();
    let filepath = dir.join(&filename);

    let header = audio::capture::create_wav_header(pcm_data.len() as u32, 16000, 16, 1);
    let mut wav_data = header;
    wav_data.extend_from_slice(&pcm_data);

    fs::write(&filepath, &wav_data)
        .map_err(|e| format!("Failed to save: {}", e))?;

    Ok(filepath.to_string_lossy().to_string())
}

#[tauri::command]
pub fn list_test_recordings() -> Result<Vec<RecordingInfo>, String> {
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
pub fn delete_test_recording(filename: String) -> Result<(), String> {
    let filepath = state::recordings_dir().join(&filename);
    fs::remove_file(&filepath)
        .map_err(|e| format!("Failed to delete: {}", e))
}

#[tauri::command]
pub fn start_test_recording(app: tauri::AppHandle, device_index: Option<usize>) -> Result<(), String> {
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
