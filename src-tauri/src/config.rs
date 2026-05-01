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
    // 1. CWD-relative
    let cwd = Path::new(name);
    if cwd.exists() {
        eprintln!("[config] Found {} in CWD", name);
        return Some(cwd.to_path_buf());
    }

    // 2. Search upward from CWD
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

    // 3. Exe directory
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidate = exe_dir.join(name);
            if candidate.exists() {
                eprintln!("[config] Found {} in exe dir: {}", name, exe_dir.display());
                return Some(candidate);
            }
            // 4. Exe's parent (for target/debug → target → project dir)
            if let Some(parent) = exe_dir.parent() {
                let candidate = parent.join(name);
                if candidate.exists() {
                    eprintln!("[config] Found {} in exe parent: {}", name, parent.display());
                    return Some(candidate);
                }
                // 5. Grandparent (for target/debug → target → project-dir)
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
            app_id: "".to_string(),
            secret_id: "".to_string(),
            secret_key: "".to_string(),
        }
    }
}

impl TencentCredentials {
    pub fn load(path: &str) -> Self {
        let resolved = find_config_file(path);

        let p = match resolved {
            Some(r) => r,
            None => {
                eprintln!("Credentials file '{}' not found, using defaults", path);
                return Self::default();
            }
        };

        match fs::read_to_string(&p) {
            Ok(content) => match serde_yaml::from_str(&content) {
                Ok(creds) => creds,
                Err(e) => {
                    eprintln!("Failed to parse '{}': {}, using defaults", p.display(), e);
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("Failed to read '{}': {}, using defaults", p.display(), e);
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
    pub osc_enabled: bool,
    pub trigger_start: String,
    pub trigger_stop: String,
    pub global_hotkey_enabled: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tencent_credentials_file: "tencent_credentials.yaml".to_string(),
            osc_host: "127.0.0.1".to_string(),
            osc_port: 9000,
            osc_line_count: 2,
            osc_retention_secs: 5,
            osc_remove_period: true,
            asr_provider: "tencent".to_string(),
            local_stt_url: "ws://192.168.101.7:8765".to_string(),
            osc_enabled: true,
            trigger_start: "开始语音识别".to_string(),
            trigger_stop: "结束语音识别".to_string(),
            global_hotkey_enabled: true,
        }
    }
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let path = find_config_file("config.yaml")
            .unwrap_or_else(|| PathBuf::from("config.yaml"));

        match fs::read_to_string(&path) {
            Ok(content) => match serde_yaml::from_str(&content) {
                Ok(config) => Ok(config),
                Err(e) => {
                    eprintln!("Failed to parse {}: {}, using default config", path.display(), e);
                    Ok(Self::default())
                }
            },
            Err(_) => {
                eprintln!("config.yaml not found, using default config");
                Ok(Self::default())
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let content = serde_yaml::to_string(self)?;
        // Save to CWD (same place load() searched first)
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

    #[test]
    fn test_find_config_file_cwd() {
        // Create a temp file in CWD and verify find_config_file finds it
        let name = "__test_find_cwd.yaml";
        fs::write(name, "test").unwrap();
        let found = find_config_file(name);
        fs::remove_file(name).ok();
        assert!(found.is_some());
    }

    #[test]
    fn test_find_config_file_not_found() {
        let found = find_config_file("__nonexistent_file_12345.yaml");
        assert!(found.is_none());
    }

    #[test]
    fn test_appconfig_default() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.tencent_credentials_file, "tencent_credentials.yaml");
        assert_eq!(cfg.osc_host, "127.0.0.1");
        assert_eq!(cfg.osc_port, 9000);
    }
}
