/// Always-on local STT trigger listener.
/// Runs in a background thread, captures audio, sends to local STT server,
/// and detects trigger phrases to start/stop recording.
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, Mutex};
use std::thread;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use crate::config::AppConfig;
use crate::audio::capture::AudioCapture;

static TRIGGER_DETECTED: AtomicBool = AtomicBool::new(false);
static LAST_TRIGGER_TEXT: Mutex<String> = Mutex::new(String::new());

pub fn is_trigger_detected() -> bool {
    TRIGGER_DETECTED.swap(false, Ordering::SeqCst)
}

pub fn last_trigger_text() -> String {
    LAST_TRIGGER_TEXT.lock().unwrap().clone()
}

/// Start the trigger listener in a background thread.
/// Returns immediately. The thread runs until the app exits.
pub fn start_trigger_listener(config: Arc<AppConfig>) {
    let start_phrase = config.trigger_start.clone();
    let stop_phrase = config.trigger_stop.clone();
    let local_url = config.local_stt_url.clone();

    if local_url.is_empty() || config.asr_provider != "local" {
        eprintln!("[Trigger] No local STT configured, trigger listener disabled");
        return;
    }

    thread::spawn(move || {
        eprintln!("[Trigger] Starting always-on listener...");

        // Find an input device
        let capture = match AudioCapture::new() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[Trigger] Failed to open audio device: {}", e);
                return;
            }
        };

        let (pcm_tx, mut pcm_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        let stop_signal = Arc::new(AtomicBool::new(false));
        let capture_stop = stop_signal.clone();

        // Spawn audio capture thread
        thread::spawn(move || {
            let result = capture.capture_streaming(
                move |chunk: Vec<u8>| {
                    let _ = pcm_tx.blocking_send(chunk);
                },
                capture_stop,
            );
            if let Err(e) = result {
                eprintln!("[Trigger] Audio capture error: {}", e);
            }
        });

        // Run the STT WebSocket client
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let url = local_url.clone();
            eprintln!("[Trigger] Connecting to local STT: {}", url);

            let ws = match tokio_tungstenite::connect_async(&url).await {
                Ok((ws, _)) => ws,
                Err(e) => {
                    eprintln!("[Trigger] Failed to connect: {}", e);
                    return;
                }
            };
            let (mut write, mut read) = ws.split();
            eprintln!("[Trigger] Connected to local STT");

            loop {
                tokio::select! {
                    chunk = pcm_rx.recv() => {
                        match chunk {
                            Some(data) => {
                                // Convert i16 PCM to f32 for local STT
                                let f32_samples: Vec<f32> = data
                                    .chunks_exact(2)
                                    .map(|pair| {
                                        let sample = i16::from_le_bytes([pair[0], pair[1]]);
                                        (sample as f32) / 32768.0
                                    })
                                    .collect();
                                let bytes: Vec<u8> = f32_samples.iter()
                                    .flat_map(|s| s.to_le_bytes())
                                    .collect();

                                let _ = write.send(Message::Binary(bytes)).await;
                            }
                            None => break,
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if let Some(t) = resp.get("text").and_then(|v| v.as_str()) {
                                        let trimmed = t.trim();
                                        if trimmed.is_empty() { continue; }

                                        // Check trigger phrases
                                        if trimmed.contains(&start_phrase) {
                                            eprintln!("[Trigger] START detected: '{}'", trimmed);
                                            TRIGGER_DETECTED.store(true, Ordering::SeqCst);
                                        } else if trimmed.contains(&stop_phrase) {
                                            eprintln!("[Trigger] STOP detected: '{}'", trimmed);
                                            TRIGGER_DETECTED.store(true, Ordering::SeqCst);
                                            *LAST_TRIGGER_TEXT.lock().unwrap() = "stop".to_string();
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        stop_signal.store(true, Ordering::SeqCst);
    });
}
