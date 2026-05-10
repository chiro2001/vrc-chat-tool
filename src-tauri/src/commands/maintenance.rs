use std::fs;
use std::path::Path;

/// Return total disk usage of the models directory in bytes.
#[tauri::command]
pub fn get_models_disk_usage() -> Result<u64, String> {
    let models_dir = Path::new("./models");
    if !models_dir.exists() {
        return Ok(0);
    }
    dir_size(models_dir).map_err(|e| format!("Failed to calculate size: {}", e))
}

fn dir_size(path: &Path) -> Result<u64, std::io::Error> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total += dir_size(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

/// Delete all downloaded models from ./models/.
#[tauri::command]
pub fn delete_downloaded_models() -> Result<String, String> {
    let models_dir = Path::new("./models");
    if !models_dir.exists() {
        return Ok("No models to delete".to_string());
    }

    let size = dir_size(models_dir).map_err(|e| format!("{}", e))?;
    let size_mb = size as f64 / 1_048_576.0;

    // Remove individual subdirectories to handle locked files better
    for entry in fs::read_dir(models_dir).map_err(|e| format!("{}", e))? {
        let entry = entry.map_err(|e| format!("{}", e))?;
        let path = entry.path();
        if path.is_dir() {
            let _ = remove_dir_contents(&path);
            fs::remove_dir(&path).ok();
        } else {
            fs::remove_file(&path).ok();
        }
    }

    Ok(format!("Deleted {:.1} MB of model files", size_mb))
}

fn remove_dir_contents(path: &Path) -> Result<(), std::io::Error> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            remove_dir_contents(&entry_path)?;
            fs::remove_dir(&entry_path)?;
        } else {
            let _ = fs::remove_file(&entry_path);
        }
    }
    Ok(())
}

/// Delete all data: config, models, database, logs. Does NOT delete .env.
#[tauri::command]
pub fn delete_all_data() -> Result<String, String> {
    let mut deleted: Vec<&str> = Vec::new();

    // config.yaml
    if Path::new("config.yaml").exists() {
        fs::remove_file("config.yaml").map_err(|e| format!("config.yaml: {}", e))?;
        deleted.push("config.yaml");
    }

    // stt-config.yaml
    if Path::new("stt-config.yaml").exists() {
        fs::remove_file("stt-config.yaml").map_err(|e| format!("stt-config.yaml: {}", e))?;
        deleted.push("stt-config.yaml");
    }

    // models/
    if Path::new("./models").exists() {
        fs::remove_dir_all("./models").map_err(|e| format!("models/: {}", e))?;
        deleted.push("models/");
    }

    // History database
    let db_path = Path::new("history.db");
    if db_path.exists() {
        fs::remove_file(db_path).map_err(|e| format!("history.db: {}", e))?;
        deleted.push("history.db");
    }

    // Log files in tmp/
    let tmp_dir = Path::new("./tmp");
    if tmp_dir.exists() {
        for entry in fs::read_dir(tmp_dir).map_err(|e| format!("tmp/: {}", e))? {
            let entry = entry.map_err(|e| format!("tmp entry: {}", e))?;
            if entry.file_name().to_string_lossy().ends_with(".log") {
                fs::remove_file(entry.path())
                    .map_err(|e| format!("log {}: {}", entry.path().display(), e))?;
            }
        }
        deleted.push("tmp/*.log");
    }

    Ok(format!("Deleted: {}", deleted.join(", ")))
}

/// Reset Tencent Cloud API usage counter to zero
#[tauri::command]
pub fn reset_tencent_usage() -> Result<u64, String> {
    let mut config_guard = vrc_chat_tool::state::CURRENT_CONFIG.lock().unwrap();
    if let Some(ref mut c) = *config_guard {
        c.tencent_usage_seconds = 0;
        c.save().map_err(|e| format!("{}", e))?;
    }
    Ok(0)
}
