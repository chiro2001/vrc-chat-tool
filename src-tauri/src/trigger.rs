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
static TRIGGER_PAUSED: AtomicBool = AtomicBool::new(false);

/// Ring buffer of recently heard texts (for UI echo, max 20 entries).
static HEARD_TEXTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
const MAX_HEARD_TEXTS: usize = 20;

/// Latest volume computed by trigger listener's audio capture.
static LATEST_VOLUME: Mutex<f32> = Mutex::new(0.0);

/// Strip common Chinese and ASCII punctuation from text for matching.
/// Preserves alphanumeric characters and whitespace.
pub fn strip_punctuation(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_ascii_punctuation()
                && !matches!(c,
                    '，' | '。' | '、' | '；' | '：' | '？' | '！'
                    | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}'
                    | '「' | '」' | '【' | '】' | '（' | '）' | '《' | '》' | '…'
                    | '·' | '～' | '〈' | '〉' | '｛' | '｝'
                )
        })
        .collect()
}

/// Check if text contains a trigger phrase, ignoring punctuation.
pub fn matches_trigger(text: &str, phrase: &str) -> bool {
    let clean_text = strip_punctuation(text);
    let clean_phrase = strip_punctuation(phrase);
    clean_text.contains(&clean_phrase)
}

pub fn is_trigger_detected() -> bool {
    TRIGGER_DETECTED.swap(false, Ordering::SeqCst)
}

pub fn last_trigger_text() -> String {
    LAST_TRIGGER_TEXT.lock().unwrap().clone()
}

/// Drain all heard texts from the trigger listener (for UI echo).
pub fn drain_heard_texts() -> Vec<String> {
    let mut buf = HEARD_TEXTS.lock().unwrap();
    if buf.is_empty() {
        return Vec::new();
    }
    std::mem::take(&mut *buf)
}

/// Get the latest volume from the trigger listener's audio capture.
pub fn latest_trigger_volume() -> f32 {
    *LATEST_VOLUME.lock().unwrap()
}

/// Pause trigger listener's audio sending (recording is active — main pipeline handles STT).
pub fn pause_audio() {
    TRIGGER_PAUSED.store(true, Ordering::SeqCst);
    eprintln!("[Trigger] Audio paused (recording active)");
}

/// Resume trigger listener's audio sending (recording stopped).
pub fn resume_audio() {
    TRIGGER_PAUSED.store(false, Ordering::SeqCst);
    eprintln!("[Trigger] Audio resumed");
}

/// Check if trigger listener's audio is paused.
pub fn is_paused() -> bool {
    TRIGGER_PAUSED.load(Ordering::Relaxed)
}

/// Set stop detected from main recording pipeline (stop phrase detected in ASR output).
pub fn set_stop_detected() {
    TRIGGER_DETECTED.store(true, Ordering::SeqCst);
    *LAST_TRIGGER_TEXT.lock().unwrap() = "stop".to_string();
    eprintln!("[Trigger] STOP detected from main pipeline");
}

/// Start the trigger listener in a background thread.
/// Returns immediately. The thread runs until the app exits.
pub fn start_trigger_listener(config: Arc<AppConfig>) {
    let start_phrase = config.trigger_start.clone();
    let stop_phrase = config.trigger_stop.clone();
    let local_url = config.local_stt_url.clone();

    if local_url.is_empty() {
        eprintln!("[Trigger] No local STT URL configured, trigger listener disabled");
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
                    // Always compute volume for UI display
                    if chunk.len() >= 2 {
                        let sum: f64 = chunk.chunks(2)
                            .map(|pair| {
                                let sample = i16::from_le_bytes([pair[0], pair[1]]) as f64;
                                sample * sample
                            }).sum();
                        let rms = (sum / (chunk.len() / 2) as f64).sqrt();
                        let volume = ((rms / 32768.0).min(1.0)) as f32;
                        *LATEST_VOLUME.lock().unwrap() = volume;
                    }

                    // Skip sending audio to STT if trigger listener is paused
                    // (main recording pipeline handles audio during active recording)
                    if TRIGGER_PAUSED.load(Ordering::Relaxed) {
                        return;
                    }

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

                                        // Store for UI echo (ring buffer)
                                        {
                                            let mut buf = HEARD_TEXTS.lock().unwrap();
                                            buf.push(trimmed.to_string());
                                            if buf.len() > MAX_HEARD_TEXTS {
                                                buf.remove(0);
                                            }
                                        }

                                        // Check trigger phrases (punctuation-tolerant)
                                        if matches_trigger(trimmed, &start_phrase) {
                                            eprintln!("[Trigger] START detected: '{}'", trimmed);
                                            TRIGGER_DETECTED.store(true, Ordering::SeqCst);
                                            *LAST_TRIGGER_TEXT.lock().unwrap() = "start".to_string();
                                        } else if matches_trigger(trimmed, &stop_phrase) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_punctuation_ascii() {
        assert_eq!(strip_punctuation("hello, world!"), "hello world");
        assert_eq!(strip_punctuation("no-punctuation"), "nopunctuation");
        assert_eq!(strip_punctuation("abc123"), "abc123");
    }

    #[test]
    fn test_strip_punctuation_chinese() {
        assert_eq!(strip_punctuation("开始，语音识别"), "开始语音识别");
        assert_eq!(strip_punctuation("结束。语音识别"), "结束语音识别");
        assert_eq!(strip_punctuation("开始、语音识别！"), "开始语音识别");
        assert_eq!(strip_punctuation("今天天气怎么样"), "今天天气怎么样");
    }

    #[test]
    fn test_strip_punctuation_mixed() {
        assert_eq!(strip_punctuation("「开始」语音识别……"), "开始语音识别");
        assert_eq!(strip_punctuation("《结束》语音识别？"), "结束语音识别");
    }

    #[test]
    fn test_strip_punctuation_empty() {
        assert_eq!(strip_punctuation(""), "");
        assert_eq!(strip_punctuation("。。。"), "");
    }

    #[test]
    fn test_matches_trigger_exact() {
        assert!(matches_trigger("开始语音识别", "开始语音识别"));
        assert!(matches_trigger("结束语音识别", "结束语音识别"));
        assert!(!matches_trigger("今天天气怎么样", "开始语音识别"));
    }

    #[test]
    fn test_matches_trigger_with_punctuation() {
        assert!(matches_trigger("开始，语音识别", "开始语音识别"));
        assert!(matches_trigger("开始。语音识别", "开始语音识别"));
        assert!(matches_trigger("开始、语音识别！", "开始语音识别"));
        assert!(matches_trigger("结束，语音识别", "结束语音识别"));
        assert!(matches_trigger("麻烦，开始，语音识别一下", "开始语音识别"));
    }

    #[test]
    fn test_matches_trigger_in_sentence() {
        // Trigger phrase appears within a longer sentence
        assert!(matches_trigger("请开始语音识别吧", "开始语音识别"));
        assert!(matches_trigger("请，开始，语音识别吧", "开始语音识别"));
    }

    #[test]
    fn test_drain_heard_texts() {
        {
            let mut buf = HEARD_TEXTS.lock().unwrap();
            buf.clear();
            buf.push("hello".to_string());
            buf.push("world".to_string());
        }
        let texts = drain_heard_texts();
        assert_eq!(texts, vec!["hello", "world"]);
        // Should be empty after drain
        let texts2 = drain_heard_texts();
        assert!(texts2.is_empty());
    }

    #[test]
    fn test_pause_resume() {
        assert!(!TRIGGER_PAUSED.load(Ordering::SeqCst));
        pause_audio();
        assert!(TRIGGER_PAUSED.load(Ordering::SeqCst));
        resume_audio();
        assert!(!TRIGGER_PAUSED.load(Ordering::SeqCst));
    }
}
