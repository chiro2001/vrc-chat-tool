use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Deserialize)]
pub struct RecogResponse {
    pub code: i32,
    pub message: String,
    #[serde(rename = "voice_id")]
    pub voice_id: Option<String>,
    pub result: Option<RecogResult>,
}

#[derive(Debug, Deserialize)]
pub struct RecogResult {
    pub slice_type: i32,
    pub index: i32,
    pub start_time: i64,
    pub end_time: i64,
    pub voice_text_str: String,
    pub word_list: Option<serde_json::Value>,
}

pub struct StreamingRecognizer {
    app_id: String,
    secret_id: String,
    secret_key: String,
    engine_model: String,
    need_vad: bool,
}

impl StreamingRecognizer {
    pub fn new(app_id: String, secret_id: String, secret_key: String, need_vad: bool) -> Self {
        Self {
            app_id,
            secret_id,
            secret_key,
            engine_model: "16k_zh".to_string(),
            need_vad,
        }
    }

    /// Build the ASR WebSocket URL using the stored credentials and config
    pub fn build_asr_url(&self, sample_rate: u32) -> String {
        let audio_format = match sample_rate {
            16000 => 1u8,
            8000 => 2u8,
            _ => 1u8,
        };
        crate::speech::tencent::build_asr_url(
            &self.app_id,
            &self.secret_id,
            &self.secret_key,
            &self.engine_model,
            audio_format,
            self.need_vad,
        )
    }

    pub async fn recognize_pcm(
        &self,
        pcm_data: Vec<u8>,
        sample_rate: u32,
    ) -> anyhow::Result<String> {
        let audio_format = match sample_rate {
            16000 => 1u8,
            8000 => 2u8,
            _ => 1u8,
        };

        let ws_url = crate::speech::tencent::build_asr_url(
            &self.app_id,
            &self.secret_id,
            &self.secret_key,
            &self.engine_model,
            audio_format,
            true,
        );
        eprintln!("[ASR] Connecting to: {}", ws_url);

        let url = url::Url::parse(&ws_url)
            .context("Failed to parse ASR URL")?;
        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| anyhow::anyhow!("ASR WebSocket connection failed: {}", e))?;
        let (mut write, mut read) = ws_stream.split();

        // Send PCM chunks (6400 bytes each ≈ 200ms at 16kHz 16bit mono)
        for chunk in pcm_data.chunks(6400) {
            write
                .send(Message::Binary(chunk.to_vec()))
                .await
                .context("Failed to send audio chunk")?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Signal end of audio (ignore error if already closed)
        let _ = write.send(Message::Text("{\"type\":\"end\"}".into())).await;
        let _ = write.close().await;

        // Read responses until we get final result (gracefully)
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(resp) = serde_json::from_str::<RecogResponse>(&text) {
                        if resp.code != 0 {
                            eprintln!("[ASR] Error response: {} - {}", resp.code, resp.message);
                        }
                        if let Some(result) = resp.result {
                            match result.slice_type {
                                0 => eprintln!("[ASR] Started"),
                                1 => eprintln!("[ASR] Partial: {}", result.voice_text_str),
                                2 => return Ok(result.voice_text_str),
                                _ => {}
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }

        Err(anyhow::anyhow!("No final result received"))
    }

    /// Streaming ASR recognition — sends chunks as they arrive from the channel.
    /// Returns the final accumulated recognized text.
    /// `on_partial` is called for each real-time partial result (slice_type=1).
    /// `on_sentence` is called when a sentence boundary is detected (API final OR 1.5s silence).
    pub async fn recognize_pcm_stream(
        &self,
        mut pcm_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        stop_signal: Arc<AtomicBool>,
        sample_rate: u32,
        on_partial: impl Fn(&str) + Send + 'static,
        on_sentence: impl Fn(&str) + Send + 'static,
    ) -> anyhow::Result<String> {
        let ws_url = self.build_asr_url(sample_rate);
        eprintln!("[ASR Stream] Connecting to: {}", ws_url);

        let url = url::Url::parse(&ws_url)
            .context("Failed to parse ASR URL")?;
        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| anyhow::anyhow!("ASR WebSocket connection failed: {}", e))?;
        let (mut write, mut read) = ws_stream.split();

        let mut full_text = String::new();
        let mut sentence_text = String::new();
        let mut last_audio_time = tokio::time::Instant::now();
        let silence_timeout = std::time::Duration::from_millis(1500);
        let tick = std::time::Duration::from_millis(200);

        loop {
            tokio::select! {
                // Check stop signal
                _ = tokio::time::sleep(tick), if stop_signal.load(Ordering::Relaxed) => {
                    break;
                }
                // Silence-based sentence boundary detection
                _ = tokio::time::sleep(tick) => {
                    if !sentence_text.is_empty()
                        && last_audio_time.elapsed() > silence_timeout {
                        eprintln!("[ASR Stream] Silence boundary: {}", sentence_text);
                        on_sentence(&sentence_text);
                        sentence_text.clear();
                    }
                }
                // Receive PCM chunk from channel
                chunk_opt = pcm_rx.recv() => {
                    last_audio_time = tokio::time::Instant::now();
                    match chunk_opt {
                        Some(chunk) => {
                            // Send chunk to ASR
                            write.send(Message::Binary(chunk)).await?;

                            // Read any available responses (non-blocking spirit)
                            while let Ok(resp_msg) = tokio::time::timeout(
                                std::time::Duration::from_millis(50),
                                read.next()
                            ).await {
                                match resp_msg {
                                    Some(Ok(Message::Text(text))) => {
                                        if let Ok(resp) = serde_json::from_str::<RecogResponse>(&text) {
                                            if resp.code != 0 {
                                                eprintln!("[ASR Stream] Error: {} - {}", resp.code, resp.message);
                                            }
                                            if let Some(result) = resp.result {
                                                match result.slice_type {
                                                    0 => eprintln!("[ASR Stream] Started"),
                                                    1 => {
                                                        // Partial — emit via callback
                                                        on_partial(&result.voice_text_str);
                                                        sentence_text = result.voice_text_str.clone();
                                                        last_audio_time = tokio::time::Instant::now();
                                                    }
                                                    2 => {
                                                        // API final sentence — emit immediately
                                                        let s = result.voice_text_str.trim().to_string();
                                                        if !s.is_empty() {
                                                            eprintln!("[ASR Stream] API final: {}", s);
                                                            on_sentence(&s);
                                                        full_text.push_str(&s);
                                                        full_text.push('\n');
                                                        }
                                                        sentence_text.clear();
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                    Some(Ok(Message::Close(_))) => break,
                                    _ => break,
                                }
                            }
                        }
                        None => {
                            // Channel closed — exit loop
                            break;
                        }
                    }
                }
            }
        }

        // Send end signal (ignore error if connection already closed)
        let _ = write.send(Message::Text("{\"type\":\"end\"}".into())).await;
        let _ = write.close().await;

        // Read final results gracefully
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(resp) = serde_json::from_str::<RecogResponse>(&text) {
                        if resp.code != 0 {
                            eprintln!("[ASR Stream] Final error: {} - {}", resp.code, resp.message);
                        }
                        if let Some(result) = resp.result {
                            if result.slice_type == 2 {
                                full_text.push_str(&result.voice_text_str);
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }

        Ok(full_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recog_response_parsing() {
        let json = r#"{"code":0,"message":"success","voice_id":"test123","result":{"slice_type":2,"index":0,"start_time":0,"end_time":1000,"voice_text_str":"你好世界"}}"#;
        let resp: RecogResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert_eq!(resp.result.as_ref().unwrap().slice_type, 2);
        assert_eq!(resp.result.unwrap().voice_text_str, "你好世界");
    }

    #[test]
    fn test_recog_response_error() {
        let json = r#"{"code":4001,"message":"invalid appid"}"#;
        let resp: RecogResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 4001);
        assert!(resp.result.is_none());
    }

    #[test]
    fn test_build_asr_url_format() {
        let recognizer = StreamingRecognizer::new(
            "12345".into(),
            "test_id".into(),
            "test_key".into(),
            true,
        );
        let url = recognizer.build_asr_url(16000);
        assert!(url.starts_with("wss://"));
        assert!(url.contains("signature="));
        assert!(url.contains("engine_model_type=16k_zh"));
    }
}
