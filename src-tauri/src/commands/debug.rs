use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use std::fs;
use tauri::Manager;
use vrc_chat_tool::state;
use vrc_chat_tool::audio;

// --- VAD state for test recording ---
struct VadState {
    speech_buffer: Vec<u8>,
    silence_samples: usize,
    had_speech: bool,
    total_speech_samples: usize,
    total_silence_samples: usize,
}

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
        let vad_buffer = Arc::new(Mutex::new(Vec::<u8>::new()));  // post-VAD audio
        let vad_buffer_clone = vad_buffer.clone();
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_signal_clone = stop_signal.clone();
        let app_emit = app_clone.clone();

        let _ = app_clone.emit_all("recording-started", "");
        emit_log(&app_clone, "info", "audio", "Test recording started (with VAD)");

        // Energy-based VAD state
        let vad_state = Arc::new(Mutex::new(VadState {
            speech_buffer: Vec::new(),
            silence_samples: 0,
            had_speech: false,
            total_speech_samples: 0,
            total_silence_samples: 0,
        }));
        let vad_state_clone = vad_state.clone();

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
                    
                    // Energy-based VAD: RMS >= 5% = speech
                    let energy = rms / 32767.0;
                    let mut vs = vad_state_clone.lock().unwrap();
                    if energy >= 0.005 {
                        vs.speech_buffer.extend_from_slice(&chunk);
                        vs.silence_samples = 0;
                        vs.had_speech = true;
                    } else {
                        vs.silence_samples += chunk.len() / 2; // samples
                        // Keep trailing silence up to 0.3s for natural endpoint
                        if vs.speech_buffer.len() > 0 && vs.silence_samples < 16000 * 3 / 10 {
                            vs.speech_buffer.extend_from_slice(&chunk);
                        }
                        // Flush segment when silence exceeds threshold after speech
                        if vs.silence_samples >= 16000 * 3 / 10 && vs.had_speech && !vs.speech_buffer.is_empty() {
                            let mut out = vad_buffer_clone.lock().unwrap();
                            out.extend_from_slice(&vs.speech_buffer);
                            vs.total_speech_samples += vs.speech_buffer.len() / 2;
                            vs.speech_buffer.clear();
                            vs.had_speech = false;
                        }
                    }
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

        // Flush remaining speech buffer
        {
            let mut vs = vad_state.lock().unwrap();
            if vs.had_speech && !vs.speech_buffer.is_empty() {
                vad_buffer.lock().unwrap().extend_from_slice(&vs.speech_buffer);
                vs.total_speech_samples += vs.speech_buffer.len() / 2;
            }
        }

        let vad_data = vad_buffer.lock().unwrap().clone();
        let raw_pcm = pcm_buffer.lock().unwrap().clone();
        let vs = vad_state.lock().unwrap();

        // Use VAD-filtered audio if available, otherwise fall back to raw
        let save_data = if vad_data.len() > 1600 { // at least 100ms
            &vad_data
        } else {
            &raw_pcm
        };

        if save_data.is_empty() {
            emit_log(&app_clone, "warn", "audio", "No audio data captured in test recording");
            let _ = app_clone.emit_all("recording-error", "No audio data captured");
            return;
        }

        let speech_sec = vs.total_speech_samples as f64 / 16000.0;
        let total_sec = (raw_pcm.len() / 2) as f64 / 16000.0;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH).unwrap().as_secs();
        let filename = format!("recording_{}.wav", timestamp);

        match save_test_recording(save_data.clone(), filename.clone()) {
            Ok(path) => {
                emit_log(&app_clone, "info", "audio",
                    &format!("Saved: {} (VAD: {:.1}s speech / {:.1}s total)",
                        filename, speech_sec, total_sec));
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
