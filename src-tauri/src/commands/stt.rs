use tauri::Manager;

// --- Types ---

#[derive(Clone, serde::Serialize)]
pub struct SttModelStatus {
    pub exists: bool,
    pub model_name: String,
    pub missing_files: Vec<String>,
    pub model_dir: String,
}

#[derive(Clone, serde::Serialize)]
pub struct DownloadProgress {
    pub phase: String,
    pub current: u64,
    pub total: u64,
}

/// A Sherpa-ONNX model available for download.
#[derive(Clone, serde::Serialize)]
pub struct AvailableModel {
    pub name: &'static str,
    pub display_name: &'static str,
    pub size_bytes: u64,
    pub files: ModelFiles,
}

#[derive(Clone, serde::Serialize)]
pub struct ModelFiles {
    pub encoder: &'static str,
    pub decoder: &'static str,
    pub joiner: &'static str,
    pub tokens: &'static str,
}

/// Registry of supported models for download.
pub const SUPPORTED_MODELS: &[AvailableModel] = &[
    AvailableModel {
        name: "sherpa-onnx-streaming-zipformer-zh-14M-2023-02-23",
        display_name: "中文 Zipformer 14M (2023) — ~74 MB",
        size_bytes: 74_004_050,
        files: ModelFiles {
            encoder: "encoder-epoch-99-avg-1.onnx",
            decoder: "decoder-epoch-99-avg-1.onnx",
            joiner: "joiner-epoch-99-avg-1.onnx",
            tokens: "tokens.txt",
        },
    },
    AvailableModel {
        name: "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20",
        display_name: "中英双语 Zipformer (2023) — ~72 MB",
        size_bytes: 72_000_000,
        files: ModelFiles {
            encoder: "encoder-epoch-99-avg-1.onnx",
            decoder: "decoder-epoch-99-avg-1.onnx",
            joiner: "joiner-epoch-99-avg-1.onnx",
            tokens: "tokens.txt",
        },
    },
    AvailableModel {
        name: "sherpa-onnx-streaming-zipformer-ctc-small-2024-03-18",
        display_name: "CTC Small 多语言 (2024) — ~176 MB",
        size_bytes: 184_604_253,
        files: ModelFiles {
            encoder: "model.int8.onnx",
            decoder: "tokens.txt",
            joiner: "",
            tokens: "tokens.txt",
        },
    },
];

// --- Commands ---

#[tauri::command]
pub fn check_stt_model(stt_config_path: String) -> Result<SttModelStatus, String> {
    eprintln!("[check_stt_model] checking config: {}", stt_config_path);
    let config = match stt_server::Config::from_file(&stt_config_path) {
        Ok(c) => c,
        Err(e) => {
            let err_msg = format!("{}", e);
            eprintln!("[check_stt_model] load failed: {}", err_msg);
            if err_msg.contains("not found") {
                eprintln!("[check_stt_model] creating default stt-config.yaml");
                let default_yaml = include_str!("../../../stt-config.yaml");
                if let Err(w) = std::fs::write(&stt_config_path, default_yaml) {
                    return Err(format!("Failed to create default stt-config.yaml: {}", w));
                }
                stt_server::Config::from_file(&stt_config_path)
                    .map_err(|e2| format!("Failed after creating default config: {}", e2))?
            } else {
                return Err(format!("Failed to load STT config: {}", e));
            }
        }
    };

    let target_dir = config.asr_model_path();
    eprintln!("[check_stt_model] target_dir: {:?}", target_dir);
    let required = [
        config.asr.encoder.as_str(),
        config.asr.decoder.as_str(),
        config.asr.joiner.as_str(),
        config.asr.tokens.as_str(),
    ];

    let missing: Vec<String> = required
        .iter()
        .filter(|f| {
            let p = target_dir.join(f);
            let exists = p.exists();
            eprintln!("[check_stt_model] {} -> {}", p.display(), if exists { "OK" } else { "MISSING" });
            !exists
        })
        .map(|f| f.to_string())
        .collect();

    eprintln!("[check_stt_model] missing: {:?}, exists: {}", missing, missing.is_empty());
    Ok(SttModelStatus {
        exists: missing.is_empty(),
        model_name: config.asr.model_name.clone(),
        missing_files: missing,
        model_dir: target_dir.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub fn download_stt_model(app: tauri::AppHandle, stt_config_path: String, force: bool) -> Result<(), String> {
    let config = stt_server::Config::from_file(&stt_config_path)
        .map_err(|e| format!("Failed to load STT config: {}", e))?;

    std::thread::spawn(move || {
        let app = app.clone();
        let result = stt_server::download_models_with_progress(
            &config,
            force,
            true,
            &|phase, current, total| {
                let _ = app.emit_all("stt-model-download-progress", DownloadProgress {
                    phase: phase.to_string(),
                    current,
                    total,
                });
            },
        );

        match result {
            Ok(()) => {
                let _ = app.emit_all("stt-model-download-complete", "");
            }
            Err(e) => {
                let _ = app.emit_all("stt-model-download-error", format!("{}", e));
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub fn get_available_models() -> Vec<AvailableModel> {
    SUPPORTED_MODELS.to_vec()
}

#[tauri::command]
pub fn set_stt_model(stt_config_path: String, model_name: String) -> Result<(), String> {
    let model = SUPPORTED_MODELS.iter()
        .find(|m| m.name == model_name)
        .ok_or_else(|| format!("Unknown model: {}", model_name))?;

    let content = std::fs::read_to_string(&stt_config_path)
        .map_err(|e| format!("Failed to read {}: {}", stt_config_path, e))?;

    let mut config: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|e| format!("Failed to parse YAML: {}", e))?;

    if let Some(asr) = config.get_mut("asr") {
        asr["model_name"] = serde_yaml::Value::String(model.name.to_string());
        asr["encoder"] = serde_yaml::Value::String(model.files.encoder.to_string());
        asr["decoder"] = serde_yaml::Value::String(model.files.decoder.to_string());
        asr["joiner"] = serde_yaml::Value::String(model.files.joiner.to_string());
        asr["tokens"] = serde_yaml::Value::String(model.files.tokens.to_string());
    }

    let new_yaml = serde_yaml::to_string(&config)
        .map_err(|e| format!("Failed to serialize YAML: {}", e))?;

    std::fs::write(&stt_config_path, new_yaml)
        .map_err(|e| format!("Failed to write {}: {}", stt_config_path, e))?;

    Ok(())
}
