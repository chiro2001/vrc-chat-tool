//! Hybrid ASR engine: Zipformer streaming + SenseVoice refinement.
//!
//!   - Zipformer (Sherpa-ONNX OnlineRecognizer): frame-by-frame streaming,
//!     real-time partials, VAD.
//!   - SenseVoice (Sherpa-ONNX OfflineRecognizer): per-segment refinement,
//!     ~0.1s per sentence.
//!
//! Serial execution — SenseVoice runs only between Zipformer endpoints
//! (~0.1s gap).

use anyhow::Result;
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig,
    OnlineRecognizer, OnlineRecognizerConfig, OnlineStream,
};

use crate::config::Config;

/// Compute RMS energy of a float32 sample buffer.
fn rms_energy(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Per-session state for the hybrid engine.
pub struct HybridStream {
    /// Online (streaming) recognition stream for Zipformer.
    pub z_stream: OnlineStream,
    /// Accumulated audio for the current segment (fed to SenseVoice).
    pub seg_audio: Vec<f32>,
    /// SenseVoice refinement in progress.
    pub refining: bool,
    /// Refined text result from SenseVoice.
    pub refined_text: String,
    /// Audio buffered during refinement (replayed through streaming model after reset).
    pub buffered: Vec<f32>,
    /// Segment index (monotonically increasing).
    pub seg_idx: u32,
    /// Last partial text sent to the client (for dedup).
    pub last_partial: String,
    /// Whether speech energy has been detected in the current segment.
    pub had_speech: bool,
    /// Consecutive silence sample count.
    pub silence_samples: usize,
}

/// Hybrid ASR engine: Zipformer CTC streaming + SenseVoice offline refinement.
///
/// Thread-safe: `HybridEngine` is `Send + Sync` and can be shared across
/// async tasks via `Arc`.
pub struct HybridEngine {
    recognizer: OnlineRecognizer,
    sensevoice: OfflineRecognizer,
    sample_rate: i32,
}

// SAFETY: sherpa-onnx C library is thread-safe for single-object usage.
unsafe impl Send for HybridEngine {}
unsafe impl Sync for HybridEngine {}

impl HybridEngine {
    /// Create a new hybrid engine from configuration.
    ///
    /// Loads the streaming model (Zipformer CTC or transducer) and the
    /// SenseVoice offline refinement model.
    pub fn new(config: &Config) -> Result<Self> {
        let asr = &config.asr;
        let sample_rate = asr.sample_rate;
        let num_threads = asr.num_threads;

        // -- 1. Streaming model (Zipformer CTC or transducer) --
        let stream_model = &asr.streaming_model;

        tracing::info!(
            "Initializing hybrid streaming model (backend={}, streaming_model={}, threads={})...",
            asr.backend,
            stream_model,
            num_threads,
        );

        let mut recog_config = OnlineRecognizerConfig::default();
        recog_config.model_config.num_threads = num_threads;
        recog_config.model_config.provider =
            Some(asr.provider.clone().unwrap_or_else(|| "cpu".into()));
        recog_config.model_config.debug = false;
        recog_config.feat_config.sample_rate = sample_rate;

        if stream_model == "zipformer-small-ctc" {
            let model_path = config.resolved_ctc_model();
            let tokens_path = config.resolved_ctc_tokens();

            tracing::info!("Loading Zipformer CTC model from: {}", model_path.display());
            tracing::info!("  tokens: {}", tokens_path.display());

            recog_config.model_config.zipformer2_ctc.model =
                Some(model_path.to_string_lossy().to_string());
            recog_config.model_config.tokens =
                Some(tokens_path.to_string_lossy().to_string());
            recog_config.model_config.model_type = Some("zipformer2_ctc".into());
        } else {
            // Transducer fallback (e.g., "transducer")
            tracing::info!("Loading transducer streaming model for hybrid (model: {})", asr.model_name);

            recog_config.model_config.transducer.encoder =
                Some(config.resolved_encoder().to_string_lossy().to_string());
            recog_config.model_config.transducer.decoder =
                Some(config.resolved_decoder().to_string_lossy().to_string());
            recog_config.model_config.transducer.joiner =
                Some(config.resolved_joiner().to_string_lossy().to_string());
            recog_config.model_config.tokens =
                Some(config.resolved_tokens().to_string_lossy().to_string());
        }

        // VAD / endpoint detection
        recog_config.enable_endpoint = config.vad.enable_endpoint_detection;
        recog_config.rule1_min_trailing_silence = config.vad.rule1_min_trailing_silence;
        recog_config.rule2_min_trailing_silence = config.vad.rule2_min_trailing_silence;
        recog_config.rule3_min_utterance_length = config.vad.rule3_min_utterance_length;

        recog_config.decoding_method = Some("greedy_search".to_string());

        let recognizer = OnlineRecognizer::create(&recog_config)
            .ok_or_else(|| anyhow::anyhow!("Failed to create OnlineRecognizer for hybrid engine"))?;

        // -- 2. SenseVoice offline recognizer --
        let sv_model_path = config.resolved_sv_model();
        let sv_tokens_path = config.resolved_sv_tokens();

        tracing::info!(
            "Loading SenseVoice model from: {} (tokens: {}, language: {})",
            sv_model_path.display(),
            sv_tokens_path.display(),
            asr.language,
        );

        let mut offline_config = OfflineRecognizerConfig::default();
        offline_config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(sv_model_path.to_string_lossy().to_string()),
            language: Some(asr.language.clone()),
            use_itn: true,
        };
        offline_config.model_config.tokens =
            Some(sv_tokens_path.to_string_lossy().to_string());
        offline_config.model_config.num_threads = num_threads;
        offline_config.model_config.provider =
            Some(asr.provider.clone().unwrap_or_else(|| "cpu".into()));
        offline_config.feat_config.sample_rate = sample_rate;

        let sensevoice = OfflineRecognizer::create(&offline_config)
            .ok_or_else(|| anyhow::anyhow!("Failed to create OfflineRecognizer for SenseVoice"))?;

        tracing::info!("HybridEngine ready (sample_rate={})", sample_rate);

        Ok(Self {
            recognizer,
            sensevoice,
            sample_rate,
        })
    }

    // ---------------------------------------------------------------
    // Internal streaming helpers
    // ---------------------------------------------------------------

    fn _streaming_decode(&self, zs: &OnlineStream, samples: &[f32]) {
        zs.accept_waveform(self.sample_rate, samples);
        while self.recognizer.is_ready(zs) {
            self.recognizer.decode(zs);
        }
    }

    fn _streaming_is_endpoint(&self, zs: &OnlineStream) -> bool {
        self.recognizer.is_endpoint(zs)
    }

    fn _streaming_get_text(&self, zs: &OnlineStream) -> String {
        self.recognizer
            .get_result(zs)
            .map(|r| r.text)
            .unwrap_or_default()
    }

    fn _streaming_reset(&self, zs: &OnlineStream) {
        self.recognizer.reset(zs);
    }

    fn _streaming_create(&self) -> OnlineStream {
        self.recognizer.create_stream()
    }

    // ---------------------------------------------------------------
    // Public API
    // ---------------------------------------------------------------

    /// Get the configured sample rate.
    pub fn sample_rate(&self) -> i32 {
        self.sample_rate
    }

    /// Create a new hybrid stream for a client session.
    pub fn create_stream(&self) -> HybridStream {
        HybridStream {
            z_stream: self._streaming_create(),
            seg_audio: Vec::new(),
            refining: false,
            refined_text: String::new(),
            buffered: Vec::new(),
            seg_idx: 0,
            last_partial: String::new(),
            had_speech: false,
            silence_samples: 0,
        }
    }

    /// Feed audio samples into the hybrid stream.
    ///
    /// If refining is in progress, audio is buffered for later replay.
    /// Otherwise, audio goes through the Zipformer streaming decoder.
    /// Energy-based speech detection drives the silence-based endpoint
    /// that triggers SenseVoice refinement.
    pub fn decode(&self, stream: &mut HybridStream, samples: &[f32]) {
        // Accumulate segment audio
        stream.seg_audio.extend_from_slice(samples);

        // Energy-based VAD
        let energy = rms_energy(samples);
        if energy >= 0.005 {
            stream.had_speech = true;
            stream.silence_samples = 0;
        } else {
            stream.silence_samples += samples.len();
        }

        // If refining, buffer audio for later fast-forward
        if stream.refining {
            stream.buffered.extend_from_slice(samples);
            return;
        }

        // Run streaming decode
        self._streaming_decode(&stream.z_stream, samples);

        // Check if we should trigger SenseVoice refinement:
        //   - 1.2s of consecutive silence
        //   - at least 0.5s of buffered audio
        //   - speech was detected in this segment
        let silence_sec = stream.silence_samples as f32 / self.sample_rate as f32;
        let buf_dur = stream.seg_audio.len() as f32 / self.sample_rate as f32;

        if silence_sec >= 1.2 && buf_dur >= 0.5 && stream.had_speech {
            stream.refining = true;
        }
    }

    /// Get the current recognized text.
    ///
    /// Returns the SenseVoice-refined text if available, otherwise the
    /// Zipformer streaming partial.
    pub fn get_text(&self, stream: &HybridStream) -> String {
        if !stream.refined_text.is_empty() {
            return stream.refined_text.clone();
        }
        self._streaming_get_text(&stream.z_stream)
    }

    /// Check if an endpoint is ready for delivery.
    ///
    /// Returns `true` if refinement is in progress (results pending) or
    /// if refined text is available.
    pub fn is_endpoint(&self, stream: &HybridStream) -> bool {
        stream.refining || !stream.refined_text.is_empty()
    }

    /// Reset the stream after consuming a segment.
    ///
    /// If `refined_text` was set: clear it, increment segment index, reset
    /// the Zipformer stream, and fast-forward any buffered audio.
    /// If `refining` was set without result: cancel refinement.
    /// Otherwise: plain Zipformer stream reset.
    pub fn reset(&self, stream: &mut HybridStream) {
        if !stream.refined_text.is_empty() {
            stream.refined_text.clear();
            stream.seg_idx += 1;
            self._streaming_reset(&stream.z_stream);
            self._fast_forward(stream);
        } else if stream.refining {
            stream.refining = false;
        } else {
            self._streaming_reset(&stream.z_stream);
        }
    }

    /// Add punctuation to text.
    ///
    /// Returns the text as-is (SenseVoice already includes punctuation).
    pub fn add_punctuation(&self, text: &str) -> String {
        text.to_string()
    }

    /// Release resources (hook for graceful shutdown).
    pub fn free(&self) {
        tracing::info!("Releasing hybrid engine resources");
    }

    // ---------------------------------------------------------------
    // Internal refinement
    // ---------------------------------------------------------------

    /// Run SenseVoice on the current segment audio.
    ///
    /// Blocking call (~0.1s). If audio is shorter than 0.3s, refinement
    /// is skipped.
    pub fn refine(&self, stream: &mut HybridStream) -> String {
        let audio = std::mem::take(&mut stream.seg_audio);
        let dur = audio.len() as f32 / self.sample_rate as f32;

        if dur < 0.3 {
            stream.refining = false;
            return String::new();
        }

        let t0 = std::time::Instant::now();

        let text = match self._run_sensevoice(&audio) {
            Ok(text) => text,
            Err(e) => {
                tracing::error!("SenseVoice failed: {}", e);
                // Fallback to streaming result
                self._streaming_get_text(&stream.z_stream)
                    .trim()
                    .to_string()
            }
        };

        let elapsed = t0.elapsed();
        let rtf = elapsed.as_secs_f32() / dur;
        tracing::info!(
            "Refined {:.1}s in {:.3}s (RTF={:.4}): {}",
            dur,
            elapsed.as_secs_f32(),
            rtf,
            text,
        );

        if !text.is_empty() {
            stream.refined_text = text.clone();
        }
        stream.had_speech = false;
        stream.silence_samples = 0;

        text
    }

    /// Run SenseVoice on audio data.
    fn _run_sensevoice(&self, audio: &[f32]) -> Result<String> {
        let s = self.sensevoice.create_stream();
        s.accept_waveform(self.sample_rate, audio);
        self.sensevoice.decode(&s);
        let text = s
            .get_result()
            .map(|r| r.text.trim().to_string())
            .unwrap_or_default();
        Ok(text)
    }

    /// Fast-forward buffered audio through the streaming decoder after reset.
    ///
    /// Processes buffered audio in 1600-sample chunks (100ms at 16kHz),
    /// then clears the buffer and resets the refinement flag.
    fn _fast_forward(&self, stream: &mut HybridStream) {
        let buf = std::mem::take(&mut stream.buffered);
        if buf.is_empty() {
            stream.refining = false;
            return;
        }

        // Process in 100ms chunks (1600 samples at 16kHz)
        for chunk in buf.chunks(1600) {
            self._streaming_decode(&stream.z_stream, chunk);
        }

        stream.had_speech = false;
        stream.silence_samples = 0;
        stream.refining = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rms_energy_silence() {
        let silence = vec![0.0f32; 1600];
        let energy = rms_energy(&silence);
        assert!(energy < 0.0001);
    }

    #[test]
    fn test_rms_energy_speech() {
        let speech: Vec<f32> = (0..1600).map(|i| (i as f32 / 1600.0) * 0.1).collect();
        let energy = rms_energy(&speech);
        assert!(energy > 0.0);
        assert!(energy < 0.1);
    }

    #[test]
    fn test_rms_energy_empty() {
        assert_eq!(rms_energy(&[]), 0.0);
    }

    #[test]
    fn test_hybrid_stream_initial_state() {
        // Can't test without real models, but verify HybridStream can be created
        // if we had an engine. At least verify the struct layout.
        let _ = std::mem::size_of::<HybridStream>();
        let _ = std::mem::size_of::<HybridEngine>();
    }
}
