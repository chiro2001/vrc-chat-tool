use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::thread;
use tauri::Manager;
use vrc_chat_tool::config;
use vrc_chat_tool::audio;
use vrc_chat_tool::speech;
use vrc_chat_tool::osc;
use vrc_chat_tool::trigger;
use vrc_chat_tool::log;
use vrc_chat_tool::state;
use crate::history;

use vrc_chat_tool::i18n;

fn model_display_name(provider: &str, config: &config::AppConfig) -> String {
    i18n::provider_short(provider, &config.language)
}

/// Track active speech duration for Tencent billing estimate
struct TencentUsageTracker {
    active_samples: u64,    // cumulative speech samples detected
    base_seconds: u64,      // previously accumulated seconds from config
    last_was_speech: bool,  // previous chunk had speech
}

#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<audio::capture::AudioDeviceInfo>, String> {
    audio::capture::AudioCapture::list_devices()
        .map_err(|e| format!("Failed to list devices: {}", e))
}

/// Inner recording pipeline (shared between Tauri command and trigger).
/// Spawns a background thread, returns immediately.
pub(crate) fn start_recording_inner(
    app: tauri::AppHandle,
    device_index: Option<usize>,
    cfg: &config::AppConfig,
) -> Result<(), String> {
    if state::IS_RECORDING.swap(true, Ordering::SeqCst) {
        return Err("Recording already in progress".to_string());
    }

    let tencent_creds = if cfg.asr_provider == "tencent" {
        if cfg.tencent_app_id.is_empty() || cfg.tencent_secret_id.is_empty() || cfg.tencent_secret_key.is_empty() {
            state::IS_RECORDING.store(false, Ordering::SeqCst);
            return Err("Please configure Tencent Cloud credentials first".to_string());
        }
        Some((cfg.tencent_app_id.clone(), cfg.tencent_secret_id.clone(), cfg.tencent_secret_key.clone()))
    } else {
        None
    };

    state::SHOULD_STOP.store(false, Ordering::SeqCst);
    trigger::pause_audio();

    let cfg = cfg.clone();
    let trigger_stop_partial = cfg.trigger_stop.clone();
    let trigger_stop_sentence = cfg.trigger_stop.clone();
        thread::spawn(move || {
        let is_tencent = cfg.asr_provider == "tencent";
        log::info("recorder", &format!("Recording started (kb={})", state::CURRENT_CONFIG.lock().unwrap().as_ref().map(|c| c.keyboard_input_enabled).unwrap_or(false)));

        // Read stt config for detailed model info
        let stt_cfg = stt_server::Config::from_file(&cfg.stt_config_path).ok();
        let model_name = stt_cfg.as_ref().map(|c| c.asr.model_name.as_str()).unwrap_or("N/A");
        let streaming = stt_cfg.as_ref().map(|c| c.asr.streaming_model.as_str()).unwrap_or("N/A");
        log::info("recorder", &format!(
            "Config: provider={} backend={} streaming={} model={} sample_rate=16000",
            cfg.asr_provider, cfg.asr_backend, streaming, model_name
        ));

        // Update overlay IPC — initially idle (waiting for speech)
        *vrc_chat_tool::ipc_server::OVERLAY_MSG.lock().unwrap() = vrc_chat_tool::ipc_server::OverlayMessage {
            msg_type: "data".into(),
            status: Some("idle".into()),
            text: None,
            sentence: None,
            volume: Some(0.0),
            model: Some(model_display_name(&cfg.asr_provider, &cfg)),
            ..Default::default()
        };

        // VAD-based usage tracking for Tencent
        let base_seconds = if is_tencent {
            state::CURRENT_CONFIG.lock().unwrap()
                .as_ref().map(|c| c.tencent_usage_seconds).unwrap_or(0)
        } else { 0 };
        let usage_tracker: Arc<Mutex<TencentUsageTracker>> = Arc::new(Mutex::new(TencentUsageTracker {
            active_samples: 0,
            base_seconds,
            last_was_speech: false,
        }));
        let usage_tracker_clone = usage_tracker.clone();
        let result: Result<String, anyhow::Error> = (|| -> anyhow::Result<String> {
            let capture = match device_index {
                Some(idx) => audio::capture::AudioCapture::new_by_index(idx)?,
                None => audio::capture::AudioCapture::new()?,
            };

            let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
            let stop_signal = Arc::new(AtomicBool::new(false));

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

            let app_for_volume = app.clone();
            let app_for_partial = app.clone();

            let _ = app.emit_all("recording-started", "");

            let recognizer = if cfg.asr_provider == "local" {
                speech::recognizer::Recognizer::Local(
                    speech::local::LocalRecognizer::new(cfg.local_stt_url.clone())
                )
                } else if cfg.asr_provider == "local_embedded" {
                    speech::recognizer::Recognizer::LocalEmbeddedHybrid(
                        speech::local_embedded::LocalEmbeddedHybridRecognizer::from_config_file(&cfg.stt_config_path)
                            .map_err(|e| anyhow::anyhow!(
                                "无法加载混合 STT 模型: {}\n\n请检查:\n\
                                 1. stt-config.yaml 中 backend 是否设为 \"hybrid\"\n\
                                 2. 模型文件是否存在（可在设置中下载模型）",
                                e,
                            ))?
                    )
            } else {
                let (ref app_id, ref secret_id, ref secret_key) = tencent_creds.as_ref().unwrap();
                speech::recognizer::Recognizer::Tencent(
                    speech::streaming::StreamingRecognizer::new(
                        app_id.clone(),
                        secret_id.clone(),
                        secret_key.clone(),
                    )
                )
            };

            let rt = tokio::runtime::Runtime::new()?;

            let capture_thread = thread::spawn(move || {
                let result = capture.capture_streaming(
                    move |chunk: Vec<u8>| {
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

                        // Detect speech for VAD-based status
                        let energy = rms / 32767.0;
                        let has_speech = energy >= 0.005;

                        // Update overlay IPC volume + VAD status
                        if let Ok(mut msg) = vrc_chat_tool::ipc_server::OVERLAY_MSG.lock() {
                            msg.volume = Some(volume);
                            if has_speech {
                                msg.status = Some("recognizing".into());
                            } else if msg.status.as_deref() == Some("recognizing") {
                                msg.status = Some("idle".into());
                            }
                        }

                        // Track VAD-based speech duration for Tencent billing
                        if is_tencent {
                            let energy = rms / 32767.0;
                            let has_speech = energy >= 0.005;
                            let samples = (chunk.len() / 2) as u64;
                            let mut tracker = usage_tracker_clone.lock().unwrap();
                            if has_speech {
                                tracker.active_samples += samples;
                                if !tracker.last_was_speech {
                                    tracker.last_was_speech = true;
                                }
                            } else if tracker.last_was_speech {
                                tracker.last_was_speech = false;
                            }
                            // Emit current total (base + session) every chunk
                            let total = tracker.base_seconds + (tracker.active_samples / 16000);
                            let _ = app_for_volume.emit_all("tencent-usage-updated", total);
                        }

                        let _ = pcm_tx.blocking_send(chunk);
                    },
                    stop_signal_for_capture,
                );
                if let Err(e) = result {
                    log::error("audio", &format!("Capture error: {}", e));
                }
            });

            log::debug("audio", "Audio capture stream opened");

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
                        let kb = state::CURRENT_CONFIG.lock().unwrap().as_ref().map(|c| c.keyboard_input_enabled).unwrap_or(false);
                        if !kb {
                            let _ = app_for_partial.emit_all("recording-partial", partial_text.to_string());
                            if let Ok(mut msg) = vrc_chat_tool::ipc_server::OVERLAY_MSG.lock() {
                                msg.status = Some("recognizing".into());
                                msg.text = Some(partial_text.to_string());
                                msg.sentence = None;
                            }
                            if let Some(ref osc) = osc_for_partial {
                                let _ = osc.send_partial(partial_text);
                            }
                        }
                        if trigger::matches_trigger(partial_text, &trigger_stop_partial) {
                            log::info("recorder", &format!("STOP detected in partial: '{}'", partial_text));
                            state::SHOULD_STOP.store(true, Ordering::SeqCst);
                        }
                    },
                    move |sentence_text: &str| {
                        // Read kb_only from CURRENT_CONFIG for real-time toggle
                        let kb_enabled = state::CURRENT_CONFIG.lock().unwrap()
                            .as_ref().map(|c| c.keyboard_input_enabled).unwrap_or(false);

                        if kb_enabled {
                            // Keyboard-only mode: inject raw text, skip OSC/history/events
                            log::info("input", &format!("KB-only injecting: {}", sentence_text));
                            if let Err(e) = vrc_chat_tool::input::inject_text(sentence_text) {
                                log::error("input", &format!("Keyboard injection failed: {}", e));
                            }
                            return;
                        }

                        let clean_text = osc::sender::OscSender::strip_trailing_punctuation(sentence_text);
                        if clean_text.is_empty() { return; }
                        let _ = app_sentence.emit_all("recording-sentence", clean_text.clone());
                        if let Ok(mut msg) = vrc_chat_tool::ipc_server::OVERLAY_MSG.lock() {
                            msg.text = None;
                            msg.sentence = Some(clean_text.clone());
                            msg.status = Some("idle".into());
                        }
                        if let Some(ref osc) = osc_s {
                            let _ = osc.send_chatbox(&clean_text);
                        }
                        history::add_entry(&clean_text, "asr");
                        log::info("asr", &format!("Sentence: {}", clean_text));

                        if trigger::matches_trigger(&clean_text, &trigger_stop_sentence) {
                            log::info("recorder", &format!("STOP detected in sentence: '{}'", sentence_text));
                            state::SHOULD_STOP.store(true, Ordering::SeqCst);
                        }
                    },
                ).await
            })?;

            let _ = capture_thread.join();

            Ok(recognized_text)
        })();

        trigger::resume_audio();
        state::IS_RECORDING.store(false, Ordering::SeqCst);

        // Track Tencent Cloud API usage time (VAD-based speech duration)
        if is_tencent {
            let tracker = usage_tracker.lock().unwrap();
            let speech_secs = tracker.active_samples / 16000;
            if speech_secs > 0 {
                let total = {
                    let mut config_guard = state::CURRENT_CONFIG.lock().unwrap();
                    if let Some(ref mut c) = *config_guard {
                        c.tencent_usage_seconds += speech_secs;
                        let _ = c.save();
                        c.tencent_usage_seconds
                    } else {
                        speech_secs
                    }
                };
                let _ = app.emit_all("tencent-usage-updated", total);
            }
        }

        match result {
            Ok(text) => {
                log::info("asr", &format!("Recognition result: {}", text));
                let _ = app.emit_all("recording-complete", text);
                // Reset overlay IPC to stop
                if let Ok(mut msg) = vrc_chat_tool::ipc_server::OVERLAY_MSG.lock() {
                    msg.status = Some("stop".into());
                    msg.text = None;
                    msg.sentence = None;
                    msg.volume = Some(0.0);
                }
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
pub fn start_recording(app: tauri::AppHandle, device_index: Option<usize>) -> Result<(), String> {
    let cfg = state::CURRENT_CONFIG
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Config not loaded".to_string())?;

    start_recording_inner(app, device_index, &cfg)
}

#[tauri::command]
pub fn stop_recording() -> Result<(), String> {
    state::SHOULD_STOP.store(true, Ordering::SeqCst);
    Ok(())
}
