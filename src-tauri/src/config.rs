use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub tencent_app_id: String,
    pub tencent_secret_id: String,
    pub tencent_secret_key: String,
    pub osc_host: String,
    pub osc_port: u16,
    pub osc_line_count: usize,
    pub osc_retention_secs: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tencent_app_id: "REDACTED_APPID".to_string(),
            tencent_secret_id: "REDACTED_SECRET_ID".to_string(),
            tencent_secret_key: "REDACTED_SECRET_KEY".to_string(),
            osc_host: "127.0.0.1".to_string(),
            osc_port: 9000,
            osc_line_count: 2,
            osc_retention_secs: 5,
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
    fn test_default_values() {
        let config = AppConfig::default();
        assert_eq!(config.tencent_app_id, "REDACTED_APPID");
        assert_eq!(config.tencent_secret_id, "REDACTED_SECRET_ID");
        assert_eq!(config.tencent_secret_key, "REDACTED_SECRET_KEY");
        assert_eq!(config.osc_host, "127.0.0.1");
        assert_eq!(config.osc_port, 9000);
    }

    #[test]
    fn test_yaml_roundtrip() {
        let config = AppConfig::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config.tencent_app_id, deserialized.tencent_app_id);
        assert_eq!(config.tencent_secret_id, deserialized.tencent_secret_id);
        assert_eq!(config.tencent_secret_key, deserialized.tencent_secret_key);
        assert_eq!(config.osc_host, deserialized.osc_host);
        assert_eq!(config.osc_port, deserialized.osc_port);
    }

    #[test]
    fn test_save_and_load() {
        let config = AppConfig::default();
        config.save().unwrap();
        let loaded = AppConfig::load().unwrap();
        assert_eq!(config.tencent_app_id, loaded.tencent_app_id);
        assert_eq!(config.tencent_secret_id, loaded.tencent_secret_id);
        assert_eq!(config.tencent_secret_key, loaded.tencent_secret_key);
        assert_eq!(config.osc_host, loaded.osc_host);
        assert_eq!(config.osc_port, loaded.osc_port);
        // clean up
        let _ = fs::remove_file("config.yaml");
    }
}
