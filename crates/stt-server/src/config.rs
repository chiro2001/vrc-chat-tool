//! Configuration loading from YAML for the STT server.
//!
//! Mirrors the Python `config.yaml` structure with resolved model paths.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Top-level configuration for the STT server.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub asr: AsrConfig,
    #[serde(default)]
    pub vad: VadConfig,
    #[serde(default)]
    pub punctuation: PunctuationConfig,
}

/// Server binding configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    8765
}
fn default_max_connections() -> usize {
    1
}

/// ASR model configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AsrConfig {
    pub model_dir: PathBuf,
    pub model_name: String,
    pub encoder: String,
    pub decoder: String,
    pub joiner: String,
    pub tokens: String,
    #[serde(default = "default_num_threads")]
    pub num_threads: i32,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: i32,
    /// ONNX execution provider (e.g., "cpu", "cuda")
    #[serde(default)]
    pub provider: Option<String>,
}

fn default_num_threads() -> i32 {
    6
}
fn default_sample_rate() -> i32 {
    16000
}

/// VAD (Voice Activity Detection) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct VadConfig {
    #[serde(default = "default_true")]
    pub enable_endpoint_detection: bool,
    /// Sentence-ending silence threshold (seconds).
    #[serde(default = "default_rule1_silence")]
    pub rule1_min_trailing_silence: f32,
    /// Sub-phrase boundary silence threshold (seconds).
    #[serde(default = "default_rule2_silence")]
    pub rule2_min_trailing_silence: f32,
    /// Minimum utterance length (seconds).
    #[serde(default = "default_rule3_length")]
    pub rule3_min_utterance_length: f32,
}

fn default_true() -> bool {
    true
}
fn default_rule1_silence() -> f32 {
    1.2
}
fn default_rule2_silence() -> f32 {
    0.6
}
fn default_rule3_length() -> f32 {
    200.0
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            enable_endpoint_detection: true,
            rule1_min_trailing_silence: 1.2,
            rule2_min_trailing_silence: 0.6,
            rule3_min_utterance_length: 200.0,
        }
    }
}

/// Punctuation restoration model configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct PunctuationConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub model_dir: PathBuf,
    #[serde(default)]
    pub model_name: String,
    #[serde(default)]
    pub model_file: String,
}

impl Default for PunctuationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model_dir: PathBuf::from("./models"),
            model_name: String::new(),
            model_file: String::new(),
        }
    }
}

impl Config {
    /// Load configuration from a YAML file.
    ///
    /// Resolves relative model paths against the config file's parent directory,
    /// matching the Python version's behavior.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content =
            std::fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
        let mut config: Config =
            serde_yaml::from_str(&content).with_context(|| "Failed to parse YAML config")?;

        // Resolve relative model paths against config file's parent directory
        let config_dir = path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();

        if config.asr.model_dir.is_relative() {
            config.asr.model_dir = config_dir.join(&config.asr.model_dir);
        }
        if config.punctuation.enabled && config.punctuation.model_dir.is_relative() {
            config.punctuation.model_dir = config_dir.join(&config.punctuation.model_dir);
        }

        Ok(config)
    }

    /// Path to the resolved ASR model directory.
    pub fn model_dir(&self) -> &Path {
        &self.asr.model_dir
    }

    /// Full path to the ASR model sub-directory (model_dir/model_name).
    pub fn asr_model_path(&self) -> PathBuf {
        self.asr.model_dir.join(&self.asr.model_name)
    }

    /// Resolved path to the encoder model file.
    pub fn resolved_encoder(&self) -> PathBuf {
        self.asr_model_path().join(&self.asr.encoder)
    }

    /// Resolved path to the decoder model file.
    pub fn resolved_decoder(&self) -> PathBuf {
        self.asr_model_path().join(&self.asr.decoder)
    }

    /// Resolved path to the joiner model file.
    pub fn resolved_joiner(&self) -> PathBuf {
        self.asr_model_path().join(&self.asr.joiner)
    }

    /// Resolved path to the tokens file.
    pub fn resolved_tokens(&self) -> PathBuf {
        self.asr_model_path().join(&self.asr.tokens)
    }

    /// Resolved path to the punctuation model, if enabled.
    pub fn resolved_punctuation_model(&self) -> Option<PathBuf> {
        if !self.punctuation.enabled || self.punctuation.model_name.is_empty() {
            return None;
        }
        let path = self
            .punctuation
            .model_dir
            .join(&self.punctuation.model_name)
            .join(&self.punctuation.model_file);
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    /// Validate that all required model files exist.
    pub fn validate_model_paths(&self) -> Result<()> {
        for (label, path) in &[
            ("encoder", self.resolved_encoder()),
            ("decoder", self.resolved_decoder()),
            ("joiner", self.resolved_joiner()),
            ("tokens", self.resolved_tokens()),
        ] {
            if !path.exists() {
                anyhow::bail!("ASR model file not found: {} ({})", path.display(), label);
            }
        }
        if self.punctuation.enabled {
            if let Some(ref p) = self.resolved_punctuation_model() {
                if !p.exists() {
                    tracing::warn!(
                        "Punctuation model configured but not found at: {}",
                        p.display()
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 9999
  max_connections: 2

asr:
  model_dir: "./test_models"
  model_name: "test-zipformer"
  encoder: "encoder.onnx"
  decoder: "decoder.onnx"
  joiner: "joiner.onnx"
  tokens: "tokens.txt"
  num_threads: 4
  sample_rate: 16000

vad:
  enable_endpoint_detection: true
  rule1_min_trailing_silence: 1.5
  rule2_min_trailing_silence: 0.8
  rule3_min_utterance_length: 300.0

punctuation:
  enabled: true
  model_dir: "./test_models"
  model_name: "test-punct"
  model_file: "model.onnx"
"#;

    #[test]
    fn test_parse_full_config() {
        let config: Config = serde_yaml::from_str(TEST_CONFIG).unwrap();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 9999);
        assert_eq!(config.server.max_connections, 2);
        assert_eq!(config.asr.model_name, "test-zipformer");
        assert_eq!(config.asr.num_threads, 4);
        assert_eq!(config.vad.rule1_min_trailing_silence, 1.5);
        assert!(config.punctuation.enabled);
    }

    #[test]
    fn test_parse_minimal_config() {
        let minimal = r#"
server: {}
asr:
  model_dir: "./models"
  model_name: "test"
  encoder: "e.onnx"
  decoder: "d.onnx"
  joiner: "j.onnx"
  tokens: "t.txt"
"#;
        let config: Config = serde_yaml::from_str(minimal).unwrap();
        assert_eq!(config.server.port, 8765); // default
        assert_eq!(config.vad.enable_endpoint_detection, true);
        assert!(!config.punctuation.enabled);
    }

    #[test]
    fn test_path_resolution() {
        let config: Config = serde_yaml::from_str(TEST_CONFIG).unwrap();
        let resolved = config.resolved_encoder();
        assert!(resolved.ends_with("test_models/test-zipformer/encoder.onnx"));
    }

    #[test]
    fn test_punctuation_disabled_by_default() {
        let minimal = r#"
server: {}
asr:
  model_dir: "./models"
  model_name: "test"
  encoder: "e.onnx"
  decoder: "d.onnx"
  joiner: "j.onnx"
  tokens: "t.txt"
"#;
        let config: Config = serde_yaml::from_str(minimal).unwrap();
        assert!(config.resolved_punctuation_model().is_none());
    }
}
