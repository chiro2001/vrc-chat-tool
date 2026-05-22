use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::cell::RefCell;
use std::time::Instant;
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
use stt_server::{VadFilter, VadDecision};

fn model_display_name(provider: &str, config: &config::AppConfig) -> String {
    i18n::provider_short(provider, &config.language)
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

        // VAD filter — only for backends where local VAD reduces API cost/engine load.
        // Remote STT (local) handles VAD server-side → skip.
        let use_vad = is_tencent; // Only Tencent: cost reduction. local_embedded has own VAD.
        let vad_enabled = cfg.vad_enabled && use_vad;

        // Track actual audio bytes sent to Tencent API for usage billing
        let bytes_sent: Arc<std::sync::atomic::AtomicU64> = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let bytes_sent_clone = bytes_sent.clone();

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
            let app_for_vad = app.clone();

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
                let vad: RefCell<Option<VadFilter>> = RefCell::new(
                    if vad_enabled { Some(VadFilter::default_16000()) } else { None }
                );
                let flush_remaining: RefCell<usize> = RefCell::new(0usize);
                const FLUSH_TARGET_SAMPLES: usize = 16000;
                let last_send: RefCell<Instant> = RefCell::new(Instant::now());

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

                        // Energy for overlay IPC status
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

                        // VAD gating — only for Tencent (cost reduction).
                        // local_embedded has its own endpoint detection in the engine
                        // and needs to see silence for sentence boundaries.
                        if let Some(ref mut v) = *vad.borrow_mut() {
                            let prev_speech = v.is_speech();
                            let decision = v.process_i16(&chunk);
                            let now_speech = v.is_speech();

                            // Emit VAD state change to frontend
                            if now_speech != prev_speech {
                                let status = if now_speech { "speech" } else { "silence" };
                                let _ = app_for_vad.emit_all("vad-status-change", status);
                            }

                            // Speech→Silence transition: start flush countdown
                            if prev_speech && !now_speech {
                                *flush_remaining.borrow_mut() = FLUSH_TARGET_SAMPLES;
                            }

                            let flushing = *flush_remaining.borrow_mut();

                            if decision == VadDecision::Speech || flushing > 0 {
                                let send_bytes = if decision == VadDecision::Speech {
                                    chunk.len()
                                } else {
                                    // Flush mode: send silence, capped at remaining
                                    let chunk_samples = chunk.len() / 2;
                                    let samples_to_send = flushing.min(chunk_samples);
                                    *flush_remaining.borrow_mut() -= samples_to_send;
                                    samples_to_send * 2 // back to bytes
                                };
                                bytes_sent_clone.fetch_add(send_bytes as u64, Ordering::Relaxed);
                                if send_bytes < chunk.len() {
                                    let _ = pcm_tx.blocking_send(vec![0u8; send_bytes]);
                                } else {
                                    let _ = pcm_tx.blocking_send(chunk);
                                }
                                *last_send.borrow_mut() = Instant::now();
                            } else {
                                // Silence with flush done → drop to save cost.
                                // But Tencent API disconnects after 15s of no data (error 4008).
                                // Send keep-alive silence every 5s.
                                let elapsed = last_send.borrow().elapsed();
                                if elapsed.as_secs() >= 5 {
                                    let keepalive: Vec<u8> = vec![0u8; 640]; // 20ms silence @ 16kHz 16bit mono
                                    bytes_sent_clone.fetch_add(keepalive.len() as u64, Ordering::Relaxed);
                                    let _ = pcm_tx.blocking_send(keepalive);
                                    *last_send.borrow_mut() = Instant::now();
                                }
                            }
                        } else {
                            // VAD disabled → send all
                            if is_tencent {
                                bytes_sent_clone.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                            }
                            let _ = pcm_tx.blocking_send(chunk);
                        }
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
            });
            // Always stop capture and join, even on error, to release audio device
            state::SHOULD_STOP.store(true, Ordering::SeqCst);
            let _ = capture_thread.join();

            recognized_text.map_err(|e| anyhow::anyhow!("{}", e))
        })();

        trigger::resume_audio();
        state::IS_RECORDING.store(false, Ordering::SeqCst);

        // Track Tencent Cloud API usage based on actual audio bytes sent
        // 16kHz 16-bit mono = 32000 bytes per second
        if is_tencent {
            let sent_bytes = bytes_sent.load(Ordering::Relaxed);
            // Convert to seconds (round up partial seconds)
            let audio_secs = (sent_bytes + 31999) / 32000;
            if audio_secs > 0 {
                let total = {
                    let mut config_guard = state::CURRENT_CONFIG.lock().unwrap();
                    if let Some(ref mut c) = *config_guard {
                        c.tencent_usage_seconds += audio_secs;
                        let _ = c.save();
                        c.tencent_usage_seconds
                    } else {
                        audio_secs
                    }
                };
                let _ = app.emit_all("tencent-usage-updated", total);
                log::info("tencent", &format!("Audio sent: {} bytes (~{}s), cumulative: {}s", sent_bytes, audio_secs, total));
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
