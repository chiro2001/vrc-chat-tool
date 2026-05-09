/// Always-on local STT trigger listener.
/// Runs in a background thread, captures audio, sends to local STT server,
/// and detects trigger phrases to start/stop recording.
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, Mutex};
use std::thread;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use crate::config::AppConfig;
use crate::audio::capture::AudioCapture;
use crate::speech::local_embedded::LocalEmbeddedRecognizer;
use stt_server::Config as SttConfig;

// --- Trigger State ---
// Consolidates shared state into a single struct instead of 9 separate statics.
// TRIGGER_PAUSED and LATEST_VOLUME are kept separate because they are accessed
// on the hot audio capture callback path (every 200ms) and need minimal overhead.

struct TriggerState {
    detected: bool,
    last_trigger_text: String,
    active: bool,
    stt_status: String,
    last_restart_attempt: Option<std::time::Instant>,
    /// Capture stop signal (set externally to stop the trigger listener's audio stream).
    capture_stop: Option<Arc<AtomicBool>>,
    /// Ring buffer of recently heard texts (for UI echo, max 20 entries).
    heard_texts: Vec<String>,
}

impl TriggerState {
    const fn new() -> Self {
        Self {
            detected: false,
            last_trigger_text: String::new(),
            active: false,
            stt_status: String::new(),
            last_restart_attempt: None,
            capture_stop: None,
            heard_texts: Vec::new(),
        }
    }
}

static TRIGGER_STATE: Mutex<TriggerState> = Mutex::new(TriggerState::new());

/// Hot-path: whether trigger listener audio is paused (recording active).
/// AtomicBool avoids Mutex contention on the audio capture callback.
static TRIGGER_PAUSED: AtomicBool = AtomicBool::new(false);

/// Hot-path: latest volume from trigger listener's audio capture.
static LATEST_VOLUME: Mutex<f32> = Mutex::new(0.0);

const MAX_HEARD_TEXTS: usize = 20;

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

// --- Public API (replaces direct global access) ---

pub fn is_trigger_detected() -> bool {
    let mut state = TRIGGER_STATE.lock().unwrap();
    let detected = state.detected;
    state.detected = false;
    detected
}

pub fn last_trigger_text() -> String {
    TRIGGER_STATE.lock().unwrap().last_trigger_text.clone()
}

/// Drain all heard texts from the trigger listener (for UI echo).
pub fn drain_heard_texts() -> Vec<String> {
    let mut state = TRIGGER_STATE.lock().unwrap();
    if state.heard_texts.is_empty() {
        return Vec::new();
    }
    std::mem::take(&mut state.heard_texts)
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

/// Check whether the trigger listener is currently running.
pub fn is_active() -> bool {
    TRIGGER_STATE.lock().unwrap().active
}

/// Stop the trigger listener's audio capture thread (for clean restart).
/// The STT loop will exit when the capture channel closes.
pub fn stop_capture() {
    let state = TRIGGER_STATE.lock().unwrap();
    if let Some(ref signal) = state.capture_stop {
        signal.store(true, Ordering::SeqCst);
    }
}

/// Get current STT connection status (for UI display).
pub fn stt_status() -> String {
    TRIGGER_STATE.lock().unwrap().stt_status.clone()
}

/// Check whether enough time has passed since the last restart attempt.
/// Returns true if restart is allowed, false if we should wait longer.
pub fn can_restart() -> bool {
    let now = std::time::Instant::now();
    let mut state = TRIGGER_STATE.lock().unwrap();
    match state.last_restart_attempt {
        Some(prev) => {
            if now.duration_since(prev).as_secs() < 5 {
                return false;
            }
        }
        None => {}
    }
    state.last_restart_attempt = Some(now);
    true
}

/// Set stop detected from main recording pipeline (stop phrase detected in ASR output).
pub fn set_stop_detected() {
    let mut state = TRIGGER_STATE.lock().unwrap();
    state.detected = true;
    state.last_trigger_text = "stop".to_string();
    eprintln!("[Trigger] STOP detected from main pipeline");
}

/// Start the trigger listener in a background thread.
/// Returns immediately. The thread runs until stopped externally or app exits.
pub fn start_trigger_listener(config: Arc<AppConfig>) {
    {
        let mut state = TRIGGER_STATE.lock().unwrap();
        if state.active {
            crate::log::debug("trigger", "Listener already running, skipping start");
            return;
        }
        state.active = true;
    }

    let start_phrase = config.trigger_start.clone();
    let stop_phrase = config.trigger_stop.clone();
    let local_url = config.local_stt_url.clone();
    let stt_config_path = config.stt_config_path.clone();
    let trigger_provider = config.trigger_stt_provider.clone();

    if trigger_provider != "local_embedded" && trigger_provider != "local_embedded_hybrid" && local_url.is_empty() {
        crate::log::info("trigger", "No local STT URL configured, trigger listener disabled");
        TRIGGER_STATE.lock().unwrap().active = false;
        return;
    }

    thread::spawn(move || {
        crate::log::info("trigger", "Starting always-on listener...");
        TRIGGER_STATE.lock().unwrap().stt_status = "connecting".to_string();

        // Find an input device
        let capture = match AudioCapture::new() {
            Ok(c) => {
                crate::log::info("trigger", &format!("Audio device opened: {} ({} Hz)", c.name(), c.sample_rate()));
                c
            }
            Err(e) => {
                crate::log::error("trigger", &format!("Failed to open audio device: {}", e));
                TRIGGER_STATE.lock().unwrap().active = false;
                return;
            }
        };

        let (pcm_tx, mut pcm_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        let stop_signal = Arc::new(AtomicBool::new(false));
        let capture_stop = stop_signal.clone();

        // Store capture stop signal so external code can stop it
        {
            TRIGGER_STATE.lock().unwrap().capture_stop = Some(stop_signal.clone());
        }

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
                    if TRIGGER_PAUSED.load(Ordering::Relaxed) {
                        return;
                    }

                    let _ = pcm_tx.blocking_send(chunk);
                },
                capture_stop,
            );
            if let Err(e) = result {
                crate::log::error("trigger", &format!("Audio capture error: {}", e));
            }
            crate::log::debug("trigger", "Audio capture thread exited");
        });

        // --- Choose STT backend ---
        if trigger_provider == "local_embedded" {
            // --- Local embedded STT path ---
            crate::log::info("trigger", &format!(
                "Initializing local embedded STT from: {}",
                stt_config_path
            ));

            let stt_cfg = match SttConfig::from_file(&stt_config_path) {
                Ok(c) => c,
                Err(e) => {
                    crate::log::error("trigger", &format!("Failed to load STT config: {}", e));
                    TRIGGER_STATE.lock().unwrap().stt_status = format!("error: {}", e);
                    stop_signal.store(true, Ordering::SeqCst);
                    TRIGGER_STATE.lock().unwrap().active = false;
                    return;
                }
            };

            let recognizer = match LocalEmbeddedRecognizer::new(stt_cfg) {
                Ok(r) => {
                    TRIGGER_STATE.lock().unwrap().stt_status = "connected".to_string();
                    crate::log::info("trigger", "Local embedded STT engine ready");
                    r
                }
                Err(e) => {
                    crate::log::error("trigger", &format!("Failed to init local STT: {}", e));
                    TRIGGER_STATE.lock().unwrap().stt_status = format!("error: {}", e);
                    stop_signal.store(true, Ordering::SeqCst);
                    TRIGGER_STATE.lock().unwrap().active = false;
                    return;
                }
            };

            let stream = recognizer.create_stream();

            loop {
                match pcm_rx.blocking_recv() {
                    Some(chunk) => {
                        let samples = LocalEmbeddedRecognizer::i16_to_f32(&chunk);
                        if !samples.is_empty() {
                            recognizer.decode(&stream, &samples);

                            if recognizer.is_endpoint(&stream) {
                                if let Some(text) = recognizer.get_text(&stream) {
                                    let trimmed = text.trim();
                                    if !trimmed.is_empty() {
                                        crate::log::debug("trigger", &format!("STT heard: '{}'", trimmed));

                                        // Store for UI echo (ring buffer)
                                        {
                                            let mut state = TRIGGER_STATE.lock().unwrap();
                                            state.heard_texts.push(trimmed.to_string());
                                            if state.heard_texts.len() > MAX_HEARD_TEXTS {
                                                state.heard_texts.remove(0);
                                            }
                                        }

                                        // Check trigger phrases
                                        if matches_trigger(trimmed, &start_phrase) {
                                            crate::log::info("trigger", &format!("START detected: '{}'", trimmed));
                                            let mut state = TRIGGER_STATE.lock().unwrap();
                                            state.detected = true;
                                            state.last_trigger_text = "start".to_string();
                                        } else if matches_trigger(trimmed, &stop_phrase) {
                                            crate::log::info("trigger", &format!("STOP detected: '{}'", trimmed));
                                            let mut state = TRIGGER_STATE.lock().unwrap();
                                            state.detected = true;
                                            state.last_trigger_text = "stop".to_string();
                                        }
                                    }
                                }
                                recognizer.reset(&stream);
                            }
                        }
                    }
                    None => {
                        crate::log::debug("trigger", "PCM channel closed, exiting local STT loop");
                        break;
                    }
                }
            }
        } else if trigger_provider == "local_embedded_hybrid" {
            // --- Local embedded hybrid STT path (Zipformer + SenseVoice) ---
            use crate::speech::local_embedded::LocalEmbeddedHybridRecognizer;
            crate::log::info("trigger", &format!(
                "Initializing hybrid STT from: {}",
                stt_config_path
            ));

            let stt_cfg = match SttConfig::from_file(&stt_config_path) {
                Ok(c) => c,
                Err(e) => {
                    crate::log::error("trigger", &format!("Failed to load STT config: {}", e));
                    TRIGGER_STATE.lock().unwrap().stt_status = format!("error: {}", e);
                    stop_signal.store(true, Ordering::SeqCst);
                    TRIGGER_STATE.lock().unwrap().active = false;
                    return;
                }
            };

            let engine = match LocalEmbeddedHybridRecognizer::new(stt_cfg) {
                Ok(r) => {
                    TRIGGER_STATE.lock().unwrap().stt_status = "connected".to_string();
                    crate::log::info("trigger", "Hybrid STT engine ready");
                    r
                }
                Err(e) => {
                    crate::log::error("trigger", &format!("Failed to init hybrid STT: {}", e));
                    TRIGGER_STATE.lock().unwrap().stt_status = format!("error: {}", e);
                    stop_signal.store(true, Ordering::SeqCst);
                    TRIGGER_STATE.lock().unwrap().active = false;
                    return;
                }
            };

            let mut hb_stream = engine.create_stream();

            loop {
                match pcm_rx.blocking_recv() {
                    Some(chunk) => {
                        let samples = LocalEmbeddedHybridRecognizer::i16_to_f32(&chunk);
                        if !samples.is_empty() {
                            engine.decode(&mut hb_stream, &samples);

                            if engine.is_endpoint(&hb_stream) {
                                if hb_stream.refining {
                                    engine.refine(&mut hb_stream);
                                }
                                let text = engine.get_text(&hb_stream);
                                let trimmed = text.trim().to_string();
                                if !trimmed.is_empty() {
                                    crate::log::debug("trigger", &format!("STT heard: '{}'", trimmed));

                                    {
                                        let mut state = TRIGGER_STATE.lock().unwrap();
                                        state.heard_texts.push(trimmed.clone());
                                        if state.heard_texts.len() > MAX_HEARD_TEXTS {
                                            state.heard_texts.remove(0);
                                        }
                                    }

                                    if matches_trigger(&trimmed, &start_phrase) {
                                        crate::log::info("trigger", &format!("START detected: '{}'", trimmed));
                                        let mut state = TRIGGER_STATE.lock().unwrap();
                                        state.detected = true;
                                        state.last_trigger_text = "start".to_string();
                                    } else if matches_trigger(&trimmed, &stop_phrase) {
                                        crate::log::info("trigger", &format!("STOP detected: '{}'", trimmed));
                                        let mut state = TRIGGER_STATE.lock().unwrap();
                                        state.detected = true;
                                        state.last_trigger_text = "stop".to_string();
                                    }
                                }
                                engine.reset(&mut hb_stream);
                            }
                        }
                    }
                    None => {
                        crate::log::debug("trigger", "PCM channel closed, exiting hybrid STT loop");
                        break;
                    }
                }
            }
        } else {
            // --- Remote STT (WebSocket) path ---
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    crate::log::error("trigger", &format!("Failed to create tokio runtime: {}", e));
                    stop_signal.store(true, Ordering::SeqCst);
                    TRIGGER_STATE.lock().unwrap().active = false;
                    return;
                }
            };

            rt.block_on(async {
                let url = local_url.clone();
                crate::log::info("trigger", &format!("Connecting to local STT: {}", url));

            let ws = match tokio_tungstenite::connect_async(&url).await {
                Ok((ws, _)) => {
                    crate::log::info("trigger", "Connected to local STT");
                    TRIGGER_STATE.lock().unwrap().stt_status = "connected".to_string();
                    ws
                }
                Err(e) => {
                    let msg = format!("Failed to connect to local STT: {}", e);
                    crate::log::error("trigger", &msg);
                    TRIGGER_STATE.lock().unwrap().stt_status = format!("error: {}", e);
                    return;
                }
            };
            let (mut write, mut read) = ws.split();

            loop {
                tokio::select! {
                    chunk = pcm_rx.recv() => {
                        match chunk {
                            Some(data) => {
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
                            None => {
                                crate::log::debug("trigger", "PCM channel closed, exiting STT loop");
                                break;
                            }
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if let Some(t) = resp.get("text").and_then(|v| v.as_str()) {
                                        let trimmed = t.trim();
                                        if trimmed.is_empty() { continue; }

                                        crate::log::debug("trigger", &format!("STT heard: '{}'", trimmed));

                                        // Store for UI echo (ring buffer)
                                        {
                                            let mut state = TRIGGER_STATE.lock().unwrap();
                                            state.heard_texts.push(trimmed.to_string());
                                            if state.heard_texts.len() > MAX_HEARD_TEXTS {
                                                state.heard_texts.remove(0);
                                            }
                                        }

                                        // Check trigger phrases (punctuation-tolerant)
                                        if matches_trigger(trimmed, &start_phrase) {
                                            crate::log::info("trigger", &format!("START detected: '{}'", trimmed));
                                            let mut state = TRIGGER_STATE.lock().unwrap();
                                            state.detected = true;
                                            state.last_trigger_text = "start".to_string();
                                        } else if matches_trigger(trimmed, &stop_phrase) {
                                            crate::log::info("trigger", &format!("STOP detected: '{}'", trimmed));
                                            let mut state = TRIGGER_STATE.lock().unwrap();
                                            state.detected = true;
                                            state.last_trigger_text = "stop".to_string();
                                        }
                                    }
                                }
                            }
                            Some(Ok(Message::Close(_))) => {
                                crate::log::warn("trigger", "STT server closed connection");
                                TRIGGER_STATE.lock().unwrap().stt_status = "disconnected".to_string();
                                break;
                            }
                            Some(Ok(_)) => {
                                // Binary, Ping, Pong — ignore
                            }
                            Some(Err(e)) => {
                                let msg = format!("STT WebSocket error: {}", e);
                                crate::log::error("trigger", &msg);
                                TRIGGER_STATE.lock().unwrap().stt_status = format!("error: {}", e);
                                break;
                            }
                            None => {
                                TRIGGER_STATE.lock().unwrap().stt_status = "disconnected".to_string();
                                break;
                            }
                        }
                    }
                }
            }
        });

        } // else (remote STT path)

        // Cleanup: signal capture to stop if still running, clear state
        stop_signal.store(true, Ordering::SeqCst);
        {
            let mut state = TRIGGER_STATE.lock().unwrap();
            state.capture_stop = None;
            state.active = false;
        }
        crate::log::info("trigger", "Listener stopped");
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
        assert!(matches_trigger("请开始语音识别吧", "开始语音识别"));
        assert!(matches_trigger("请，开始，语音识别吧", "开始语音识别"));
    }

    #[test]
    fn test_drain_heard_texts() {
        {
            let mut state = TRIGGER_STATE.lock().unwrap();
            state.heard_texts.clear();
            state.heard_texts.push("hello".to_string());
            state.heard_texts.push("world".to_string());
        }
        let texts = drain_heard_texts();
        assert_eq!(texts, vec!["hello", "world"]);
        let texts2 = drain_heard_texts();
        assert!(texts2.is_empty());
    }

    #[test]
    fn test_pause_resume() {
        TRIGGER_PAUSED.store(false, Ordering::SeqCst);
        assert!(!TRIGGER_PAUSED.load(Ordering::SeqCst));
        pause_audio();
        assert!(TRIGGER_PAUSED.load(Ordering::SeqCst));
        resume_audio();
        assert!(!TRIGGER_PAUSED.load(Ordering::SeqCst));
    }
}
