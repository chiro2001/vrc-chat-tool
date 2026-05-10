use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;

/// Search for a config file. Order:
/// 1. Current working directory
/// 2. Each parent directory up to root
/// 3. Executable's directory (for production builds)
/// 4. Executable's parent directory (for dev: target/debug → target → project root)
fn find_config_file(name: &str) -> Option<PathBuf> {
    let cwd = Path::new(name);
    if cwd.exists() {
        eprintln!("[config] Found {} in CWD", name);
        return Some(cwd.to_path_buf());
    }

    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = cwd.as_path();
        loop {
            let candidate = dir.join(name);
            if candidate.exists() {
                eprintln!("[config] Found {} in {}", name, dir.display());
                return Some(candidate);
            }
            match dir.parent() {
                Some(parent) => dir = parent,
                None => break,
            }
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidate = exe_dir.join(name);
            if candidate.exists() {
                eprintln!("[config] Found {} in exe dir: {}", name, exe_dir.display());
                return Some(candidate);
            }
            if let Some(parent) = exe_dir.parent() {
                let candidate = parent.join(name);
                if candidate.exists() {
                    eprintln!("[config] Found {} in exe parent: {}", name, parent.display());
                    return Some(candidate);
                }
                if let Some(grandparent) = parent.parent() {
                    let candidate = grandparent.join(name);
                    if candidate.exists() {
                        eprintln!("[config] Found {} in exe grandparent: {}", name, grandparent.display());
                        return Some(candidate);
                    }
                }
            }
        }
    }

    eprintln!("[config] {} not found in CWD, parents, or exe dirs", name);
    None
}

/// Load environment variables from .env file (for credentials).
/// Returns quietly if .env is not found.
fn load_dotenv() {
    if let Some(env_path) = find_config_file(".env") {
        match dotenvy::from_path(&env_path) {
            Ok(_) => eprintln!("[config] Loaded .env from {}", env_path.display()),
            Err(e) => eprintln!("[config] Failed to load .env: {}", e),
        }
    } else {
        eprintln!("[config] .env not found, using defaults for credentials");
    }
}

/// Application configuration — stored in config.yaml (gitignored).
/// Credentials are loaded from .env on startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    // ---- ASR ----
    pub asr_provider: String,
    pub asr_backend: String,
    pub onnx_provider: String,
    pub local_stt_url: String,
    pub stt_config_path: String,

    // ---- Tencent credentials (from .env) ----
    pub tencent_app_id: String,
    pub tencent_secret_id: String,
    pub tencent_secret_key: String,

    // ---- OSC ----
    pub osc_enabled: bool,
    pub osc_host: String,
    pub osc_port: u16,
    pub osc_line_count: usize,
    pub osc_retention_secs: u64,
    pub osc_remove_period: bool,

    // ---- Trigger listener ----
    pub trigger_listener_enabled: bool,
    pub trigger_stt_provider: String,
    pub trigger_start: String,
    pub trigger_stop: String,

    // ---- Hotkey ----
    pub global_hotkey_enabled: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            asr_provider: "local_embedded".to_string(),
            asr_backend: "sherpa-onnx".to_string(),
            onnx_provider: "cpu".to_string(),
            local_stt_url: "ws://192.168.101.7:8765".to_string(),
            stt_config_path: "stt-config.yaml".to_string(),

            tencent_app_id: String::new(),
            tencent_secret_id: String::new(),
            tencent_secret_key: String::new(),

            osc_enabled: true,
            osc_host: "127.0.0.1".to_string(),
            osc_port: 9000,
            osc_line_count: 2,
            osc_retention_secs: 5,
            osc_remove_period: true,

            trigger_listener_enabled: false,
            trigger_stt_provider: "local".to_string(),
            trigger_start: "开始语音识别".to_string(),
            trigger_stop: "结束语音识别".to_string(),

            global_hotkey_enabled: true,
        }
    }
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        load_dotenv();

        let path = find_config_file("config.yaml")
            .unwrap_or_else(|| PathBuf::from("config.yaml"));

        let mut config = match fs::read_to_string(&path) {
            Ok(content) => match serde_yaml::from_str(&content) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("Failed to parse {}: {}, using default config", path.display(), e);
                    Self::default()
                }
            },
            Err(_) => {
                eprintln!("config.yaml not found, using default config");
                Self::default()
            }
        };

        // Override credentials from .env if present
        if config.tencent_app_id.is_empty() {
            if let Ok(val) = std::env::var("TENCENT_APP_ID") { config.tencent_app_id = val; }
        }
        if config.tencent_secret_id.is_empty() {
            if let Ok(val) = std::env::var("TENCENT_SECRET_ID") { config.tencent_secret_id = val; }
        }
        if config.tencent_secret_key.is_empty() {
            if let Ok(val) = std::env::var("TENCENT_SECRET_KEY") { config.tencent_secret_key = val; }
        }

        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let content = serde_yaml::to_string(self)?;
        fs::write("config.yaml", content)?;
        Ok(())
    }
}
