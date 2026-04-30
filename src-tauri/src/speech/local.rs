/// Local streaming STT provider — connects to streaming-stt-server via WebSocket.
/// Sends float32 PCM audio, receives JSON partial/final results.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct LocalSttResponse {
    text: Option<String>,
    is_final: Option<bool>,
    segment: Option<i32>,
}

pub struct LocalRecognizer {
    server_url: String,
}

impl LocalRecognizer {
    pub fn new(server_url: String) -> Self {
        Self { server_url }
    }

    /// Convert i16 PCM to f32 samples (normalized to [-1, 1])
    fn i16_to_f32(pcm: &[u8]) -> Vec<f32> {
        pcm.chunks_exact(2)
            .map(|pair| {
                let sample = i16::from_le_bytes([pair[0], pair[1]]);
                (sample as f32) / 32768.0
            })
            .collect()
    }

    pub async fn recognize_pcm_stream(
        &self,
        mut pcm_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        stop_signal: Arc<AtomicBool>,
        on_partial: impl Fn(&str) + Send + 'static,
        on_sentence: impl Fn(&str) + Send + 'static,
    ) -> anyhow::Result<String> {
        let url = self.server_url.clone();
        eprintln!("[Local STT] Connecting to: {}", url);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| anyhow::anyhow!("Local STT connection failed: {}", e))?;
        let (mut write, mut read) = ws_stream.split();
        eprintln!("[Local STT] Connected");

        let mut full_text = String::new();
        let tick = std::time::Duration::from_millis(100);

        loop {
            tokio::select! {
                _ = tokio::time::sleep(tick), if stop_signal.load(Ordering::Relaxed) => {
                    break;
                }
                chunk_opt = pcm_rx.recv() => {
                    match chunk_opt {
                        Some(chunk) => {
                            // Convert i16 to f32 and send
                            let f32_samples = Self::i16_to_f32(&chunk);
                            if !f32_samples.is_empty() {
                                let bytes: Vec<u8> = f32_samples.iter()
                                    .flat_map(|s| s.to_le_bytes())
                                    .collect();
                                let _ = write.send(Message::Binary(bytes)).await;
                            }

                            // Read any available responses
                            while let Ok(Some(msg)) = tokio::time::timeout(
                                std::time::Duration::from_millis(50),
                                read.next(),
                            ).await {
                                match msg {
                                    Ok(Message::Text(text)) => {
                                        if let Ok(resp) = serde_json::from_str::<LocalSttResponse>(&text) {
                                            if let Some(t) = resp.text {
                                                if resp.is_final.unwrap_or(false) {
                                                    let s = t.trim().to_string();
                                                    if !s.is_empty() {
                                                        eprintln!("[Local STT] Final: {}", s);
                                                        on_sentence(&s);
                                                        full_text.push_str(&s);
                                                        full_text.push('\n');
                                                    }
                                                } else {
                                                    eprintln!("[Local STT] Partial: {}", t);
                                                    on_partial(&t);
                                                }
                                            }
                                        }
                                    }
                                    Ok(Message::Close(_)) | Err(_) => break,
                                    _ => {}
                                }
                            }
                        }
                        None => break,
                    }
                }
            }
        }

        // Send silence to trigger VAD flush (3 seconds of zeros)
        eprintln!("[Local STT] Sending silence to flush VAD...");
        let silence = vec![0.0f32; 1600]; // 100ms silence
        let silence_bytes: Vec<u8> = silence.iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        for _ in 0..30 {
            let _ = write.send(Message::Binary(silence_bytes.clone())).await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // Read remaining responses
        while let Ok(Some(msg)) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            read.next(),
        ).await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(resp) = serde_json::from_str::<LocalSttResponse>(&text) {
                        if let Some(t) = resp.text {
                            if resp.is_final.unwrap_or(false) {
                                let s = t.trim().to_string();
                                if !s.is_empty() {
                                    eprintln!("[Local STT] Late final: {}", s);
                                    on_sentence(&s);
                                    full_text.push_str(&s);
                                    full_text.push('\n');
                                }
                            }
                        }
                    }
                }
                _ => break,
            }
        }

        let _ = write.close().await;
        Ok(full_text)
    }
}
