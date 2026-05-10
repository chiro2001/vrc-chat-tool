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

    // --- Hybrid engine fields ---
    /// Backend to use: "sherpa-onnx" (traditional) or "hybrid" (zipformer + SenseVoice)
    #[serde(default = "default_backend")]
    pub backend: String,
    /// Streaming model type: "transducer" or "zipformer-small-ctc"
    #[serde(default = "default_streaming_model")]
    pub streaming_model: String,
    /// Path to Zipformer CTC model directory (used when streaming_model == "zipformer-small-ctc")
    #[serde(default = "default_ctc_model_dir")]
    pub ctc_model_dir: PathBuf,
    /// Path to SenseVoice model directory
    #[serde(default = "default_sv_model_dir")]
    pub sv_model_dir: PathBuf,
    /// SenseVoice model filename (relative to sv_model_dir)
    #[serde(default = "default_sv_model")]
    pub sv_model: String,
    /// SenseVoice tokens filename (relative to sv_model_dir)
    #[serde(default = "default_sv_tokens")]
    pub sv_tokens: String,
    /// Language for SenseVoice (e.g., "zh", "auto")
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_num_threads() -> i32 {
    6
}
fn default_sample_rate() -> i32 {
    16000
}
fn default_backend() -> String {
    "sherpa-onnx".into()
}
fn default_streaming_model() -> String {
    "transducer".into()
}
fn default_ctc_model_dir() -> PathBuf {
    PathBuf::from("./models/sherpa-onnx-streaming-zipformer-small-ctc-zh-int8-2025-04-01")
}
fn default_sv_model_dir() -> PathBuf {
    PathBuf::from("./models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17")
}
fn default_sv_model() -> String {
    "model.int8.onnx".into()
}
fn default_sv_tokens() -> String {
    "tokens.txt".into()
}
fn default_language() -> String {
    "zh".into()
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

/// Search for a config file. Order:
/// 1. Current working directory
/// 2. Each parent directory up to root
/// 3. Executable's directory (for production builds)
/// 4. Executable's parent dir → grandparent (for dev: target/debug → project root)
fn find_config_file(name: &str) -> Option<PathBuf> {
    // 1. CWD-relative
    let cwd = Path::new(name);
    if cwd.exists() {
        return Some(cwd.to_path_buf());
    }

    // 2. Search upward from CWD
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = cwd.as_path();
        loop {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
            match dir.parent() {
                Some(parent) => dir = parent,
                None => break,
            }
        }
    }

    // 3. Exe directory
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidate = exe_dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
            // 4. Exe's parent → grandparent (target/debug → target → project dir)
            if let Some(parent) = exe_dir.parent() {
                let candidate = parent.join(name);
                if candidate.exists() {
                    return Some(candidate);
                }
                if let Some(grandparent) = parent.parent() {
                    let candidate = grandparent.join(name);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                    // 6. Exe's grandparent subdirectories (workspace crate pattern)
                    //    e.g. target/debug → target → project-root, then crates/stt-server/
                    for subdir in &["crates/stt-server", "src-tauri"] {
                        let candidate = grandparent.join(subdir).join(name);
                        if candidate.exists() {
                            return Some(candidate);
                        }
                    }
                }
            }
        }
    }

    None
}

impl Config {
    /// Load configuration from a YAML file.
    ///
    /// If the exact path doesn't exist, searches CWD → parent dirs → exe dir
    /// for a file named `config.yaml` (or whatever `path` specifies).
    ///
    /// Resolves relative model paths against the config file's parent directory,
    /// matching the Python version's behavior.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let requested = path.as_ref();
        let resolved = if requested.exists() {
            requested.to_path_buf()
        } else if requested.is_absolute() {
            // Absolute path that doesn't exist — no fallback
            anyhow::bail!("Config file not found: {}", requested.display());
        } else {
            // Try auto-discovery
            let name = requested
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("config.yaml");
            find_config_file(name)
                .ok_or_else(|| anyhow::anyhow!("Config file not found: {}", requested.display()))?
        };

        let content =
            std::fs::read_to_string(&resolved).with_context(|| format!("Failed to read {}", resolved.display()))?;
        let mut config: Config =
            serde_yaml::from_str(&content).with_context(|| "Failed to parse YAML config")?;

        // Resolve relative model paths against config file's parent directory
        let config_dir = resolved
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();

        if config.asr.model_dir.is_relative() {
            config.asr.model_dir = config_dir.join(&config.asr.model_dir);
        }
        if config.asr.ctc_model_dir.is_relative() {
            config.asr.ctc_model_dir = config_dir.join(&config.asr.ctc_model_dir);
        }
        if config.asr.sv_model_dir.is_relative() {
            config.asr.sv_model_dir = config_dir.join(&config.asr.sv_model_dir);
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

    /// Full path to the CTC model file (uses encoder filename from config).
    pub fn resolved_ctc_model(&self) -> PathBuf {
        self.asr.ctc_model_dir.join(&self.asr.encoder)
    }

    /// Full path to the CTC tokens file (tokens.txt).
    pub fn resolved_ctc_tokens(&self) -> PathBuf {
        self.asr.ctc_model_dir.join("tokens.txt")
    }

    /// Full path to the SenseVoice model file.
    pub fn resolved_sv_model(&self) -> PathBuf {
        self.asr.sv_model_dir.join(&self.asr.sv_model)
    }

    /// Full path to the SenseVoice tokens file.
    pub fn resolved_sv_tokens(&self) -> PathBuf {
        self.asr.sv_model_dir.join(&self.asr.sv_tokens)
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
        if self.asr.backend == "hybrid" {
            // Validate streaming model paths
            if self.asr.streaming_model == "zipformer-small-ctc" {
                for (label, path) in &[
                    ("ctc_model", self.resolved_ctc_model()),
                    ("ctc_tokens", self.resolved_ctc_tokens()),
                ] {
                    if !path.exists() {
                        anyhow::bail!(
                            "CTC model file not found: {} ({})",
                            path.display(),
                            label
                        );
                    }
                }
            } else {
                // Transducer paths for hybrid (when not using CTC)
                for (label, path) in &[
                    ("encoder", self.resolved_encoder()),
                    ("decoder", self.resolved_decoder()),
                    ("joiner", self.resolved_joiner()),
                    ("tokens", self.resolved_tokens()),
                ] {
                    if !path.exists() {
                        anyhow::bail!(
                            "ASR model file not found: {} ({})",
                            path.display(),
                            label
                        );
                    }
                }
            }
            // Validate SenseVoice paths
            for (label, path) in &[
                ("sv_model", self.resolved_sv_model()),
                ("sv_tokens", self.resolved_sv_tokens()),
            ] {
                if !path.exists() {
                    anyhow::bail!(
                        "SenseVoice model file not found: {} ({})",
                        path.display(),
                        label
                    );
                }
            }
        } else {
            // Traditional sherpa-onnx backend validation
            for (label, path) in &[
                ("encoder", self.resolved_encoder()),
                ("decoder", self.resolved_decoder()),
                ("joiner", self.resolved_joiner()),
                ("tokens", self.resolved_tokens()),
            ] {
                if !path.exists() {
                    anyhow::bail!(
                        "ASR model file not found: {} ({})",
                        path.display(),
                        label
                    );
                }
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
