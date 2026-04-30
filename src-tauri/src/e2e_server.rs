use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tiny_http::{Header, Method, Response, Server};

use vrc_chat_tool::audio::capture::AudioCapture;
use vrc_chat_tool::config::AppConfig;
use vrc_chat_tool::osc::sender::OscSender;
use vrc_chat_tool::speech::streaming::StreamingRecognizer;

static SHOULD_STOP_E2E: AtomicBool = AtomicBool::new(false);

/// Start the E2E HTTP test server. Blocks until the server is shut down.
pub fn run_e2e_server() -> Result<(), Box<dyn std::error::Error>> {
    let server = Server::http("127.0.0.1:9876")
        .map_err(|e| format!("Failed to start E2E server: {}", e))?;
    eprintln!("[E2E] HTTP test server started on http://127.0.0.1:9876");

    // Shared state
    let last_result: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let is_recording = Arc::new(AtomicBool::new(false));
    let config = AppConfig::load()?;

    for request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().clone();

        // CORS header for Python client
        let cors_header = Header::from_bytes(
            &b"Access-Control-Allow-Origin"[..],
            &b"*"[..],
        )
        .unwrap();

        match (method, url.as_str()) {
            (Method::Options, _) => {
                // CORS preflight
                let response = Response::from_string("ok").with_header(cors_header);
                let _ = request.respond(response);
            }

            (Method::Post, "/start") => {
                if is_recording.load(Ordering::SeqCst) {
                    let response = Response::from_string(r#"{"status":"error","message":"already recording"}"#)
                        .with_status_code(409)
                        .with_header(cors_header);
                    let _ = request.respond(response);
                    continue;
                }

                let cfg = config.clone();
                let result_clone = last_result.clone();
                let is_rec_clone = is_recording.clone();

                SHOULD_STOP_E2E.store(false, Ordering::SeqCst);
                is_rec_clone.store(true, Ordering::SeqCst);

                thread::spawn(move || {
                    let result = run_recording_pipeline(&cfg);
                    match result {
                        Ok(text) => {
                            *result_clone.lock().unwrap() = text;
                        }
                        Err(e) => {
                            *result_clone.lock().unwrap() = format!("ERROR: {}", e);
                        }
                    }
                    is_rec_clone.store(false, Ordering::SeqCst);
                });

                let response = Response::from_string(r#"{"status":"ok","message":"recording started"}"#)
                    .with_header(cors_header);
                let _ = request.respond(response);
            }

            (Method::Post, "/stop") => {
                SHOULD_STOP_E2E.store(true, Ordering::SeqCst);
                let response = Response::from_string(r#"{"status":"ok","message":"stop signal sent"}"#)
                    .with_header(cors_header);
                let _ = request.respond(response);
            }

            (Method::Get, "/result") => {
                let text = last_result.lock().unwrap().clone();
                let recording = is_recording.load(Ordering::SeqCst);
                let json = serde_json::json!({
                    "status": if recording { "recording" } else { "idle" },
                    "text": text,
                    "recording": recording,
                }).to_string();
                let response = Response::from_string(json).with_header(cors_header);
                let _ = request.respond(response);
            }

            (Method::Get, "/status") => {
                let recording = is_recording.load(Ordering::SeqCst);
                let json = format!(
                    r#"{{"status":"{}","recording":{}}}"#,
                    if recording { "recording" } else { "idle" },
                    recording
                );
                let response = Response::from_string(json).with_header(cors_header);
                let _ = request.respond(response);
            }

            _ => {
                let response = Response::from_string(r#"{"error":"not found"}"#)
                    .with_status_code(404)
                    .with_header(cors_header);
                let _ = request.respond(response);
            }
        }
    }

    Ok(())
}

/// Run the full recording pipeline: capture from VB-Cable -> ASR -> return text
fn run_recording_pipeline(cfg: &AppConfig) -> Result<String, Box<dyn std::error::Error>> {
    // Find VB-Cable device by name for E2E testing
    let devices = AudioCapture::list_devices()
        .map_err(|e| format!("Failed to list devices: {}", e))?;
    let vb_cable = devices.iter()
        .find(|d| d.name.to_uppercase().contains("CABLE"))
        .ok_or("VB-Cable device not found. Is VB-Cable installed?")?;
    eprintln!("[E2E] Using VB-Cable device: {} (index {})", vb_cable.name, vb_cable.index);

    let capture = AudioCapture::new_by_index(vb_cable.index)?;

    let (pcm_tx, mut pcm_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
    let stop_signal = Arc::new(AtomicBool::new(false));

    // Bridge: monitor SHOULD_STOP_E2E
    let s_sig = stop_signal.clone();
    thread::spawn(move || {
        while !SHOULD_STOP_E2E.load(Ordering::SeqCst) {
            thread::sleep(std::time::Duration::from_millis(100));
        }
        s_sig.store(true, Ordering::SeqCst);
    });

    let capture_stop = stop_signal.clone();
    let asr_stop = stop_signal.clone();

    // Spawn audio capture
    let capture_thread = thread::spawn(move || {
        let result = capture.capture_streaming(
            move |chunk: Vec<u8>| {
                let _ = pcm_tx.blocking_send(chunk);
            },
            capture_stop,
        );
        if let Err(e) = result {
            eprintln!("[E2E] Audio capture error: {}", e);
        }
    });

    let _ = capture_thread.join();

    // Collect all PCM data from channel (batch mode)
    let mut all_pcm = Vec::new();
    while let Ok(chunk) = pcm_rx.try_recv() {
        all_pcm.extend_from_slice(&chunk);
    }
    // Get remaining data from buffer
    drop(pcm_rx);

    eprintln!("[E2E] Collected {} bytes of PCM data", all_pcm.len());
    if all_pcm.is_empty() {
        return Err("No audio data captured".into());
    }

    // Run ASR in batch mode
    let recognizer = StreamingRecognizer::new(
        cfg.tencent_app_id.clone(),
        cfg.tencent_secret_id.clone(),
        cfg.tencent_secret_key.clone(),
    );

    let rt = tokio::runtime::Runtime::new()?;
    let recognized_text = rt.block_on(async {
        recognizer.recognize_pcm(all_pcm, 16000).await
    })?;

    // Send via OSC
    let osc = OscSender::new(cfg.osc_host.clone(), cfg.osc_port);
    let _ = osc.send_typing(true);
    let _ = osc.send_chatbox(&recognized_text);
    let _ = osc.send_typing(false);

    eprintln!("[E2E] Recognition result: {}", recognized_text);
    Ok(recognized_text)
}
