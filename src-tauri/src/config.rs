use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use anyhow::Result;

/// Tencent Cloud credentials — stored in a SEPARATE file to avoid leaking secrets in git.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TencentCredentials {
    pub app_id: String,
    pub secret_id: String,
    pub secret_key: String,
}

impl Default for TencentCredentials {
    fn default() -> Self {
        Self {
            app_id: "REDACTED_APPID".to_string(),
            secret_id: "REDACTED_SECRET_ID".to_string(),
            secret_key: "REDACTED_SECRET_KEY".to_string(),
        }
    }
}

impl TencentCredentials {
    pub fn load(path: &str) -> Self {
        let p = Path::new(path);
        if !p.exists() {
            eprintln!("Credentials file '{}' not found, using defaults", path);
            return Self::default();
        }
        match fs::read_to_string(p) {
            Ok(content) => match serde_yaml::from_str(&content) {
                Ok(creds) => creds,
                Err(e) => {
                    eprintln!("Failed to parse '{}': {}, using defaults", path, e);
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("Failed to read '{}': {}, using defaults", path, e);
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &str) -> Result<()> {
        let content = serde_yaml::to_string(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

/// Application configuration — stored in config.yaml (gitignored).
/// Credentials are kept in a separate file (referenced by credentials_file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub tencent_credentials_file: String,
    pub osc_host: String,
    pub osc_port: u16,
    pub osc_line_count: usize,
    pub osc_retention_secs: u64,
    pub osc_remove_period: bool,
    pub asr_provider: String,
    pub local_stt_url: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tencent_credentials_file: ".tencent_credentials.yaml".to_string(),
            osc_host: "127.0.0.1".to_string(),
            osc_port: 9000,
            osc_line_count: 2,
            osc_retention_secs: 5,
            osc_remove_period: true,
            asr_provider: "tencent".to_string(),
            local_stt_url: "ws://192.168.101.7:8765".to_string(),
        }
    }
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let path = Path::new("config.yaml");
        if !path.exists() {
            eprintln!("config.yaml not found, using default config");
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)?;
        match serde_yaml::from_str(&content) {
            Ok(config) => Ok(config),
            Err(e) => {
                eprintln!("Failed to parse config.yaml: {}, using default config", e);
                Ok(Self::default())
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let content = serde_yaml::to_string(self)?;
        fs::write("config.yaml", content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_roundtrip() {
        let creds = TencentCredentials::default();
        let yaml = serde_yaml::to_string(&creds).unwrap();
        let deserialized: TencentCredentials = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(creds.app_id, deserialized.app_id);
    }
}
