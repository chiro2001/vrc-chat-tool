//! Local embedded STT provider — directly wraps SttEngine for in-process ASR.
//!
//! Unlike `local.rs` which connects via WebSocket, this module bypasses the
//! network entirely and calls the Sherpa-ONNX engine directly. This reduces
//! latency and eliminates the need for a separate server process.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use stt_server::{Config, SttEngine};

pub struct LocalEmbeddedRecognizer {
    engine: Arc<SttEngine>,
}

impl LocalEmbeddedRecognizer {
    /// Create a new embedded recognizer from a `stt_server::Config`.
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let engine = Arc::new(SttEngine::new(&config)?);
        Ok(Self { engine })
    }

    /// Convenience: create from a YAML config path.
    pub fn from_config_file(path: &str) -> anyhow::Result<Self> {
        let config = Config::from_file(path)?;
        Self::new(config)
    }

    /// Convert i16 PCM bytes to f32 samples (normalized to [-1, 1]).
    fn i16_to_f32(pcm: &[u8]) -> Vec<f32> {
        pcm.chunks_exact(2)
            .map(|pair| {
                let sample = i16::from_le_bytes([pair[0], pair[1]]);
                (sample as f32) / 32768.0
            })
            .collect()
    }

    /// Run streaming recognition on i16 PCM chunks.
    ///
    /// - `pcm_rx` — channel receiving raw i16 mono PCM (typical: 100ms/1600 samples)
    /// - `stop_signal` — set to `true` to trigger graceful shutdown (flush VAD, return final text)
    /// - `on_partial` — called with intermediate recognition text
    /// - `on_sentence` — called when a sentence endpoint is detected (with punctuation if available)
    ///
    /// Returns the full accumulated text.
    pub async fn recognize_pcm_stream(
        &self,
        mut pcm_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        stop_signal: Arc<AtomicBool>,
        on_partial: impl Fn(&str) + Send + 'static,
        on_sentence: impl Fn(&str) + Send + 'static,
    ) -> anyhow::Result<String> {
        let stream = self.engine.create_stream();
        let mut full_text = String::new();
        let mut last_partial = String::new();
        let mut _segment: u32 = 0;

        let _tick = std::time::Duration::from_millis(100);
        let sync_timeout = std::time::Duration::from_millis(20);

        loop {
            let should_stop = stop_signal.load(Ordering::Relaxed);

            match tokio::time::timeout(sync_timeout, pcm_rx.recv()).await {
                Ok(Some(chunk)) => {
                    // Convert i16 → f32 and feed to engine
                    let samples = Self::i16_to_f32(&chunk);
                    if !samples.is_empty() {
                        self.engine.decode(&stream, &samples);

                        // Check results
                        if self.engine.is_endpoint(&stream) {
                            if let Some(text) = self.engine.get_text(&stream) {
                                if !text.trim().is_empty() {
                                    let final_text = self
                                        .engine
                                        .add_punctuation(&text)
                                        .unwrap_or_else(|| text.clone());

                                    if !final_text.trim().is_empty() {
                                        on_sentence(&final_text);
                                        full_text.push_str(&final_text);
                                        full_text.push('\n');
                                        _segment += 1;
                                    }
                                }
                            }
                            self.engine.reset(&stream);
                            last_partial.clear();
                        } else if let Some(text) = self.engine.get_text(&stream) {
                            let t = text.trim().to_string();
                            if !t.is_empty() && t != last_partial {
                                on_partial(&t);
                                last_partial = t;
                            }
                        }
                    }
                }
                Ok(None) => break, // channel closed
                Err(_) => {}       // timeout, continue
            }

            if should_stop {
                break;
            }
        }

        // Flush: send silence to push VAD endpoint
        let silence_len = (self.engine.sample_rate() as f32 * 0.3) as usize;
        let silence = vec![0.0f32; silence_len];

        for _ in 0..10 {
            self.engine.decode(&stream, &silence);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        self.engine.input_finished(&stream);

        // Collect any remaining final text
        if let Some(text) = self.engine.get_text(&stream) {
            if !text.trim().is_empty() {
                let final_text = self
                    .engine
                    .add_punctuation(&text)
                    .unwrap_or_else(|| text.clone());

                if !final_text.trim().is_empty() {
                    on_sentence(&final_text);
                    full_text.push_str(&final_text);
                    full_text.push('\n');
                }
            }
        }

        self.engine.reset(&stream);

        Ok(full_text)
    }
}
