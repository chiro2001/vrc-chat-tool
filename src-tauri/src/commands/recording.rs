use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
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
        log::info("recorder", "Recording started");
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

            if cfg.osc_enabled {
                let osc_typing = osc::sender::OscSender::new(cfg.osc_host.clone(), cfg.osc_port);
                let _ = osc_typing.send_typing(true);
            }

            let recognizer = if cfg.asr_provider == "local" {
                speech::recognizer::Recognizer::Local(
                    speech::local::LocalRecognizer::new(cfg.local_stt_url.clone())
                )
                } else if cfg.asr_provider == "local_embedded" {
                    if cfg.asr_backend == "hybrid" {
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
                        speech::recognizer::Recognizer::LocalEmbedded(
                            speech::local_embedded::LocalEmbeddedRecognizer::from_config_file(&cfg.stt_config_path)
                                .map_err(|e| anyhow::anyhow!(
                                    "无法加载 STT 模型: {}\n\n请检查:\n\
                                     1. 设置中\"STT 模型配置路径\"是否正确\n\
                                     2. 模型文件是否存在（可在设置中下载模型）",
                                    e,
                                ))?
                        )
                    }
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
                        let _ = app_for_partial.emit_all("recording-partial", partial_text.to_string());
                        if let Some(ref osc) = osc_for_partial {
                            let _ = osc.send_partial(partial_text);
                        }
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
                        if trigger::matches_trigger(sentence_text, &trigger_stop_sentence) {
                            log::info("recorder", &format!("STOP detected in sentence: '{}'", sentence_text));
                            state::SHOULD_STOP.store(true, Ordering::SeqCst);
                        }
                    },
                ).await
            })?;

            let _ = capture_thread.join();

            if osc_enabled {
                let osc = osc::sender::OscSender::new(cfg.osc_host.clone(), cfg.osc_port);
                let _ = osc.send_typing(false);
                let _ = osc.clear_chatbox();
            }

            Ok(recognized_text)
        })();

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
