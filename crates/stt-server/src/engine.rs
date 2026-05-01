//! STT ASR engine — wraps Sherpa-ONNX OnlineRecognizer and optional punctuation.
//!
//! Provides the core inference logic: create streams, decode audio, detect
//! endpoints, add punctuation. This module is the library core — it has no
//! I/O dependencies beyond sherpa-onnx.

use anyhow::Result;
use sherpa_onnx::{
    OfflinePunctuation, OfflinePunctuationConfig, OfflinePunctuationModelConfig,
    OnlineRecognizer, OnlineRecognizerConfig, OnlineStream,
};
use std::sync::OnceLock;

use crate::config::Config;

/// Holds the Sherpa-ONNX OnlineRecognizer and optional punctuation model.
///
/// Implements `Send + Sync` — can be shared across async tasks via `Arc`.
pub struct SttEngine {
    recognizer: OnlineRecognizer,
    sample_rate: i32,
    /// Lazy-loaded punctuation model (None if disabled or not found).
    punctuation: OnceLock<Option<OfflinePunctuation>>,
    /// Path to punctuation model file (set at construction, used by OnceLock).
    punctuation_path: Option<std::path::PathBuf>,
}

impl SttEngine {
    /// Create a new STT engine from configuration.
    ///
    /// Initializes the OnlineRecognizer with transducer model files from the config.
    /// The punctuation model is loaded lazily on first use.
    pub fn new(config: &Config) -> Result<Self> {
        let asr_cfg = &config.asr;

        tracing::info!(
            "Initializing OnlineRecognizer from transducer (model: {}, threads: {})...",
            asr_cfg.model_name,
            asr_cfg.num_threads,
        );

        let encoder_path = config.resolved_encoder();
        let decoder_path = config.resolved_decoder();
        let joiner_path = config.resolved_joiner();
        let tokens_path = config.resolved_tokens();

        // Validate paths exist before passing to sherpa-onnx
        for (label, path) in &[
            ("encoder", &encoder_path),
            ("decoder", &decoder_path),
            ("joiner", &joiner_path),
            ("tokens", &tokens_path),
        ] {
            if !path.exists() {
                anyhow::bail!(
                    "ASR model file not found: {} ({})",
                    path.display(),
                    label
                );
            }
        }

        let mut recog_config = OnlineRecognizerConfig::default();
        recog_config.model_config.transducer.encoder =
            Some(encoder_path.to_string_lossy().to_string());
        recog_config.model_config.transducer.decoder =
            Some(decoder_path.to_string_lossy().to_string());
        recog_config.model_config.transducer.joiner =
            Some(joiner_path.to_string_lossy().to_string());
        recog_config.model_config.tokens =
            Some(tokens_path.to_string_lossy().to_string());
        recog_config.model_config.num_threads = asr_cfg.num_threads;
        recog_config.model_config.provider = asr_cfg.provider.clone();
        recog_config.model_config.debug = false;
        recog_config.feat_config.sample_rate = asr_cfg.sample_rate;

        // VAD / endpoint detection
        recog_config.enable_endpoint = config.vad.enable_endpoint_detection;
        recog_config.rule1_min_trailing_silence = config.vad.rule1_min_trailing_silence;
        recog_config.rule2_min_trailing_silence = config.vad.rule2_min_trailing_silence;
        recog_config.rule3_min_utterance_length = config.vad.rule3_min_utterance_length;

        recog_config.decoding_method = Some("greedy_search".to_string());

        let recognizer = OnlineRecognizer::create(&recog_config)
            .ok_or_else(|| anyhow::anyhow!("Failed to create OnlineRecognizer"))?;

        let punctuation_path = config.resolved_punctuation_model();

        tracing::info!(
            "STT Engine initialized (sample_rate={}, endpoint_detection={})",
            asr_cfg.sample_rate,
            config.vad.enable_endpoint_detection,
        );

        Ok(Self {
            recognizer,
            sample_rate: asr_cfg.sample_rate,
            punctuation: OnceLock::new(),
            punctuation_path,
        })
    }

    /// Get the configured sample rate.
    pub fn sample_rate(&self) -> i32 {
        self.sample_rate
    }

    // ---------------------------------------------------------------
    // Stream lifecycle
    // ---------------------------------------------------------------

    /// Create a new recognition stream for a client session.
    pub fn create_stream(&self) -> OnlineStream {
        self.recognizer.create_stream()
    }

    /// Feed audio samples into the stream and run the decode loop.
    ///
    /// `samples` should be float32 PCM data.
    pub fn decode(&self, stream: &OnlineStream, samples: &[f32]) {
        stream.accept_waveform(self.sample_rate, samples);
        while self.recognizer.is_ready(stream) {
            self.recognizer.decode(stream);
        }
    }

    /// Get the current recognition text from the stream.
    ///
    /// Returns `None` if no result is available yet.
    pub fn get_text(&self, stream: &OnlineStream) -> Option<String> {
        self.recognizer.get_result(stream).map(|r| r.text)
    }

    /// Check if an endpoint (sentence boundary) has been detected.
    pub fn is_endpoint(&self, stream: &OnlineStream) -> bool {
        self.recognizer.is_endpoint(stream)
    }

    /// Reset the stream after an endpoint — starts a new utterance segment.
    pub fn reset(&self, stream: &OnlineStream) {
        self.recognizer.reset(stream);
    }

    /// Signal that no more input will be provided (flush tail context).
    pub fn input_finished(&self, stream: &OnlineStream) {
        stream.input_finished();
    }

    // ---------------------------------------------------------------
    // Punctuation
    // ---------------------------------------------------------------

    /// Add punctuation to text using the CT-Transformer model, if available.
    ///
    /// Returns `None` if punctuation is disabled or the model couldn't be loaded.
    pub fn add_punctuation(&self, text: &str) -> Option<String> {
        if text.trim().is_empty() {
            return Some(text.to_string());
        }

        let punct = self.punctuation.get_or_init(|| self.load_punctuation());

        match punct {
            Some(ref p) => p.add_punctuation(text),
            None => None,
        }
    }

    /// Lazy-load the punctuation model. Called once by `OnceLock`.
    fn load_punctuation(&self) -> Option<OfflinePunctuation> {
        let path = self.punctuation_path.as_ref()?;
        tracing::info!("Loading punctuation model from {} ...", path.display());

        if !path.exists() {
            tracing::warn!("Punctuation model file not found: {}", path.display());
            return None;
        }

        let model_config = OfflinePunctuationModelConfig {
            ct_transformer: Some(path.to_string_lossy().to_string()),
            num_threads: 1,
            ..Default::default()
        };

        let config = OfflinePunctuationConfig {
            model: model_config,
        };

        let punct = OfflinePunctuation::create(&config)?;
        tracing::info!("Punctuation model loaded successfully");
        Some(punct)
    }

    /// Check if punctuation is available (model was loaded successfully).
    pub fn has_punctuation(&self) -> bool {
        self.punctuation.get_or_init(|| self.load_punctuation()).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_creation_missing_model() {
        // Test that engine creation fails with non-existent model paths
        let yaml = r#"
server:
  host: "127.0.0.1"
  port: 8765
asr:
  model_dir: "/nonexistent/models"
  model_name: "fake-model"
  encoder: "encoder.onnx"
  decoder: "decoder.onnx"
  joiner: "joiner.onnx"
  tokens: "tokens.txt"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let result = SttEngine::new(&config);
        assert!(result.is_err());
    }
}
