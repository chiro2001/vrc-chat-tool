//! Local embedded STT providers — in-process ASR without network.
//!
//! Two variants:
//! - `LocalEmbeddedRecognizer` — wraps `SttEngine` (transducer-based)
//! - `LocalEmbeddedHybridRecognizer` — wraps `HybridEngine` (Zipformer CTC + SenseVoice)
//!
//! Unlike `local.rs` which connects via WebSocket, these bypass the network
//! entirely and call the Sherpa-ONNX engine directly, reducing latency.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use stt_server::{Config, HybridEngine, HybridStream, OnlineStream, SttEngine};

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

    // --- Low-level streaming API (for trigger listener and continuous use) ---

    /// Create an independent recognition stream.
    ///
    /// Each stream tracks its own audio session. Call `reset()` after each
    /// endpoint to start a new utterance.
    pub fn create_stream(&self) -> OnlineStream {
        self.engine.create_stream()
    }

    /// Feed float32 PCM samples into the stream. Sample rate must match the
    /// engine's configured rate (16000).
    pub fn decode(&self, stream: &OnlineStream, samples: &[f32]) {
        self.engine.decode(stream, samples)
    }

    /// Check if an endpoint (sentence boundary) has been detected.
    pub fn is_endpoint(&self, stream: &OnlineStream) -> bool {
        self.engine.is_endpoint(stream)
    }

    /// Get the current recognition text from the stream.
    /// Returns `None` if no result is available yet.
    pub fn get_text(&self, stream: &OnlineStream) -> Option<String> {
        self.engine.get_text(stream)
    }

    /// Reset the stream after an endpoint — starts a new utterance segment.
    pub fn reset(&self, stream: &OnlineStream) {
        self.engine.reset(stream)
    }

    /// Signal that no more input will be provided.
    pub fn input_finished(&self, stream: &OnlineStream) {
        self.engine.input_finished(stream)
    }

    /// Get the engine's sample rate.
    pub fn sample_rate(&self) -> i32 {
        self.engine.sample_rate()
    }

    // --- High-level streaming API (for recording pipeline) ---

    /// Convert i16 PCM bytes to f32 samples (normalized to [-1, 1]).
    pub fn i16_to_f32(pcm: &[u8]) -> Vec<f32> {
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

/// Local embedded hybrid recognizer — wraps `HybridEngine` for in-process ASR
/// with Zipformer CTC streaming + SenseVoice offline refinement.
pub struct LocalEmbeddedHybridRecognizer {
    engine: Arc<HybridEngine>,
}

impl LocalEmbeddedHybridRecognizer {
    /// Create a new hybrid recognizer from a `stt_server::Config`.
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let engine = Arc::new(HybridEngine::new(&config)?);
        Ok(Self { engine })
    }

    /// Convenience: create from a YAML config path.
    pub fn from_config_file(path: &str) -> anyhow::Result<Self> {
        let config = Config::from_file(path)?;
        Self::new(config)
    }

    // --- Low-level streaming API ---

    /// Create an independent hybrid recognition stream.
    pub fn create_stream(&self) -> HybridStream {
        self.engine.create_stream()
    }

    /// Feed float32 PCM samples into the stream.
    pub fn decode(&self, stream: &mut HybridStream, samples: &[f32]) {
        self.engine.decode(stream, samples)
    }

    /// Check if an endpoint is ready for delivery.
    pub fn is_endpoint(&self, stream: &HybridStream) -> bool {
        self.engine.is_endpoint(stream)
    }

    /// Get the current recognized text.
    pub fn get_text(&self, stream: &HybridStream) -> String {
        self.engine.get_text(stream)
    }

    /// Reset the stream after consuming a segment.
    pub fn reset(&self, stream: &mut HybridStream) {
        self.engine.reset(stream)
    }

    /// Run SenseVoice refinement on the current segment.
    pub fn refine(&self, stream: &mut HybridStream) -> String {
        self.engine.refine(stream)
    }

    /// Get the engine's sample rate.
    pub fn sample_rate(&self) -> i32 {
        self.engine.sample_rate()
    }

    // --- High-level streaming API (for recording pipeline) ---

    /// Run hybrid streaming recognition on i16 PCM chunks.
    ///
    /// Same interface as `LocalEmbeddedRecognizer::recognize_pcm_stream`
    /// but uses the hybrid engine (Zipformer CTC + SenseVoice refinement).
    pub async fn recognize_pcm_stream(
        &self,
        mut pcm_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        stop_signal: Arc<AtomicBool>,
        on_partial: impl Fn(&str) + Send + 'static,
        on_sentence: impl Fn(&str) + Send + 'static,
    ) -> anyhow::Result<String> {
        let mut hb_stream = self.engine.create_stream();
        let mut full_text = String::new();
        let mut _segment: u32 = 0;

        let sync_timeout = std::time::Duration::from_millis(20);

        loop {
            let should_stop = stop_signal.load(Ordering::Relaxed);

            match tokio::time::timeout(sync_timeout, pcm_rx.recv()).await {
                Ok(Some(chunk)) => {
                    let samples = Self::i16_to_f32(&chunk);
                    if !samples.is_empty() {
                        self.engine.decode(&mut hb_stream, &samples);

                        if self.engine.is_endpoint(&hb_stream) {
                            // Run SenseVoice refinement if triggered
                            if hb_stream.refining {
                                self.engine.refine(&mut hb_stream);
                            }

                            let text = self.engine.get_text(&hb_stream);
                            if !text.trim().is_empty() {
                                let final_text = self.engine.add_punctuation(&text);
                                if !final_text.trim().is_empty() {
                                    on_sentence(&final_text);
                                    full_text.push_str(&final_text);
                                    full_text.push('\n');
                                    _segment += 1;
                                }
                            }
                            self.engine.reset(&mut hb_stream);
                        } else {
                            let text = self.engine.get_text(&hb_stream);
                            let t = text.trim().to_string();
                            if !t.is_empty() {
                                on_partial(&t);
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => {}
            }

            if should_stop {
                break;
            }
        }

        // Flush: send silence and refine if needed
        let silence_len = (self.engine.sample_rate() as f32 * 0.3) as usize;
        let silence = vec![0.0f32; silence_len];

        for _ in 0..10 {
            self.engine.decode(&mut hb_stream, &silence);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        if hb_stream.refining {
            self.engine.refine(&mut hb_stream);
        }

        // Collect any remaining final text
        let text = self.engine.get_text(&hb_stream);
        if !text.trim().is_empty() {
            let final_text = self.engine.add_punctuation(&text);
            if !final_text.trim().is_empty() {
                on_sentence(&final_text);
                full_text.push_str(&final_text);
                full_text.push('\n');
            }
        }

        self.engine.reset(&mut hb_stream);

        Ok(full_text)
    }

    /// Convert i16 PCM bytes to f32 samples (normalized to [-1, 1]).
    pub fn i16_to_f32(pcm: &[u8]) -> Vec<f32> {
        pcm.chunks_exact(2)
            .map(|pair| {
                let sample = i16::from_le_bytes([pair[0], pair[1]]);
                (sample as f32) / 32768.0
            })
            .collect()
    }
}
