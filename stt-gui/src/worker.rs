//! Background worker — manages WebSocket connection and audio streaming.
//!
//! Runs on a separate OS thread with its own tokio runtime.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use crate::audio;

use futures_util::StreamExt;

/// Events sent from the background worker to the GUI thread.
pub enum WorkerEvent {
    Connected,
    Disconnected,
    Error(String),
    Partial(String),
    Final { text: String, segment: u32 },
    Volume(f32),
    Log { level: String, message: String },
}

/// Manages the background thread and audio streaming.
pub struct StreamWorker {
    url: String,
    event_tx: mpsc::Sender<WorkerEvent>,
    stop_tx: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StreamWorker {
    pub fn new(url: String, event_tx: mpsc::Sender<WorkerEvent>) -> Self {
        Self {
            url,
            event_tx,
            stop_tx: None,
            thread: None,
        }
    }

    /// Start streaming from microphone.
    ///
    /// `device_name` — the name of the input device to use, or None for default.
    pub fn start_mic(&mut self, device_name: Option<String>) {
        self.stop_internal();

        let (stop_tx, stop_rx) = mpsc::channel();
        self.stop_tx = Some(stop_tx);

        let url = self.url.clone();
        let event_tx = self.event_tx.clone();

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if let Err(e) = run_mic_stream(&url, event_tx, stop_rx, device_name).await {
                    // Error already sent via WorkerEvent::Error in run_mic_stream
                    let _ = e;
                }
            });
        });

        self.thread = Some(handle);
    }

    /// Start streaming from a WAV file.
    pub fn start_file(&mut self, path: &Path) {
        self.stop_internal();

        let (stop_tx, stop_rx) = mpsc::channel();
        self.stop_tx = Some(stop_tx);

        let url = self.url.clone();
        let event_tx = self.event_tx.clone();
        let path_buf = path.to_path_buf();

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if let Err(e) = run_file_stream(&url, &path_buf, event_tx, stop_rx).await {
                    let _ = e;
                }
            });
        });

        self.thread = Some(handle);
    }

    /// Signal the worker to stop.
    pub fn stop(&mut self) {
        self.stop_internal();
    }

    fn stop_internal(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for StreamWorker {
    fn drop(&mut self) {
        self.stop_internal();
    }
}

// ---------------------------------------------------------------------------
// Mic stream
// ---------------------------------------------------------------------------

async fn run_mic_stream(
    url: &str,
    event_tx: mpsc::Sender<WorkerEvent>,
    stop_rx: mpsc::Receiver<()>,
    device_name: Option<String>,
) -> anyhow::Result<()> {
    use tokio_tungstenite::connect_async;
    use futures_util::SinkExt;

    let _ = event_tx.send(WorkerEvent::Log {
        level: "info".into(),
        message: format!("Connecting to {} ...", url),
    });

    let (ws_stream, _) = connect_async(url).await.map_err(|e| {
        let msg = format!("Connection failed: {}", e);
        let _ = event_tx.send(WorkerEvent::Error(msg.clone()));
        anyhow::anyhow!(msg)
    })?;

    let _ = event_tx.send(WorkerEvent::Connected);
    let _ = event_tx.send(WorkerEvent::Log {
        level: "info".into(),
        message: "Connected to server".into(),
    });

    let (mut write, read) = ws_stream.split();

    // Start microphone capture with optional device selection
    let (audio_tx, audio_rx) = mpsc::channel::<Vec<f32>>();
    let stop_fn = match audio::start_mic_capture(audio_tx, device_name.as_deref()) {
        Ok(f) => f,
        Err(e) => {
            let msg = format!("Microphone setup failed: {}", e);
            let _ = event_tx.send(WorkerEvent::Log {
                level: "error".into(),
                message: msg.clone(),
            });
            let _ = event_tx.send(WorkerEvent::Error(msg.clone()));
            // Close WebSocket gracefully before returning
            let _ = write.close().await;
            return Err(anyhow::anyhow!(msg));
        }
    };

    let _ = event_tx.send(WorkerEvent::Log {
        level: "info".into(),
        message: "Microphone started (16kHz mono float32)".into(),
    });

    // Spawn receiver task
    let recv_event_tx = event_tx.clone();
    let recv_handle = tokio::spawn(async move {
        recv_worker(read, recv_event_tx).await;
    });

    // Send loop
    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        match audio_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(samples) => {
                // Calculate volume
                let vol: f32 = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
                let vol = vol.sqrt().min(1.0);
                let _ = event_tx.send(WorkerEvent::Volume(vol));

                // Send as bytes
                let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                if write.send(tokio_tungstenite::tungstenite::Message::Binary(bytes)).await.is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Stop microphone capture before sending silence
    stop_fn();

    // Send silence to flush VAD
    let _ = event_tx.send(WorkerEvent::Log {
        level: "info".into(),
        message: "Sending silence to flush VAD...".into(),
    });
    let silence = vec![0.0f32; 1600];
    let silence_bytes: Vec<u8> = silence.iter().flat_map(|s| s.to_le_bytes()).collect();
    for _ in 0..30 {
        let _ = write.send(tokio_tungstenite::tungstenite::Message::Binary(silence_bytes.clone())).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let _ = write.close().await;
    recv_handle.await.ok();

    let _ = event_tx.send(WorkerEvent::Disconnected);
    let _ = event_tx.send(WorkerEvent::Log {
        level: "info".into(),
        message: "Microphone stopped".into(),
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// File stream
// ---------------------------------------------------------------------------

async fn run_file_stream(
    url: &str,
    path: &Path,
    event_tx: mpsc::Sender<WorkerEvent>,
    stop_rx: mpsc::Receiver<()>,
) -> anyhow::Result<()> {
    use tokio_tungstenite::connect_async;
    use futures_util::SinkExt;

    let _ = event_tx.send(WorkerEvent::Log {
        level: "info".into(),
        message: format!("Reading WAV file: {}", path.display()),
    });

    let (samples, sr) = audio::read_wav(path).map_err(|e| {
        let msg = format!("Failed to read WAV: {}", e);
        let _ = event_tx.send(WorkerEvent::Error(msg.clone()));
        anyhow::anyhow!(msg)
    })?;

    let chunk_size = 1600; // 100ms at 16kHz
    let total_chunks = (samples.len() + chunk_size - 1) / chunk_size;
    let duration = samples.len() as f32 / sr as f32;

    let _ = event_tx.send(WorkerEvent::Log {
        level: "info".into(),
        message: format!("{} chunks, {:.1}s, sr={}Hz", total_chunks, duration, sr),
    });

    let _ = event_tx.send(WorkerEvent::Log {
        level: "info".into(),
        message: format!("Connecting to {} ...", url),
    });

    let (ws_stream, _) = connect_async(url).await.map_err(|e| {
        let msg = format!("Connection failed: {}", e);
        let _ = event_tx.send(WorkerEvent::Error(msg.clone()));
        anyhow::anyhow!(msg)
    })?;

    let _ = event_tx.send(WorkerEvent::Connected);
    let (mut write, read) = ws_stream.split();

    let recv_event_tx = event_tx.clone();
    let recv_handle = tokio::spawn(async move {
        recv_worker(read, recv_event_tx).await;
    });

    // Send chunks
    for chunk in samples.chunks(chunk_size) {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        let mut padded = vec![0.0f32; chunk_size];
        padded[..chunk.len()].copy_from_slice(chunk);

        let bytes: Vec<u8> = padded.iter().flat_map(|s| s.to_le_bytes()).collect();
        if write.send(tokio_tungstenite::tungstenite::Message::Binary(bytes)).await.is_err() {
            break;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Send silence to flush VAD
    let _ = event_tx.send(WorkerEvent::Log {
        level: "info".into(),
        message: "Sending silence to flush VAD...".into(),
    });
    let silence = vec![0.0f32; chunk_size];
    let silence_bytes: Vec<u8> = silence.iter().flat_map(|s| s.to_le_bytes()).collect();
    for _ in 0..30 {
        let _ = write.send(tokio_tungstenite::tungstenite::Message::Binary(silence_bytes.clone())).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let _ = write.close().await;
    recv_handle.await.ok();

    let _ = event_tx.send(WorkerEvent::Disconnected);
    let _ = event_tx.send(WorkerEvent::Log {
        level: "info".into(),
        message: "File send complete".into(),
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// Shared receiver
// ---------------------------------------------------------------------------

async fn recv_worker(
    mut read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    event_tx: mpsc::Sender<WorkerEvent>,
) {
    use futures_util::StreamExt;

    while let Some(msg) = read.next().await {
        match msg {
            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                    let txt = value.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    let is_final = value.get("is_final").and_then(|v| v.as_bool()).unwrap_or(false);
                    let segment = value.get("segment").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

                    if is_final {
                        let _ = event_tx.send(WorkerEvent::Final {
                            text: txt.to_string(),
                            segment,
                        });
                    } else {
                        let _ = event_tx.send(WorkerEvent::Partial(txt.to_string()));
                    }
                }
            }
            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => break,
            Err(_) => break,
            _ => {}
        }
    }
}
