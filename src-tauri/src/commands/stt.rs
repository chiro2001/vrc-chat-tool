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
    /// Backend: "sherpa-onnx" (transducer) or "hybrid" (CTC+SenseVoice)
    pub backend: &'static str,
    /// For hybrid models: second model info (SenseVoice)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sv_model: Option<SvModelInfo>,
}

#[derive(Clone, serde::Serialize)]
pub struct SvModelInfo {
    pub name: &'static str,
    pub display_name: &'static str,
    pub size_bytes: u64,
    pub model_file: &'static str,
    pub tokens_file: &'static str,
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
    // ── Standard backend (sherpa-onnx / transducer) ──
    AvailableModel {
        name: "sherpa-onnx-streaming-zipformer-zh-14M-2023-02-23",
        display_name: "中文 Zipformer 14M (标准) — ~74 MB",
        size_bytes: 74_004_050,
        backend: "sherpa-onnx",
        sv_model: None,
        files: ModelFiles {
            encoder: "encoder-epoch-99-avg-1.onnx",
            decoder: "decoder-epoch-99-avg-1.onnx",
            joiner: "joiner-epoch-99-avg-1.onnx",
            tokens: "tokens.txt",
        },
    },
    AvailableModel {
        name: "sherpa-onnx-streaming-zipformer-zh-14M-2023-02-23-mobile",
        display_name: "中文 Zipformer 14M (轻量/CPU) — ~52 MB",
        size_bytes: 54_250_000,
        backend: "sherpa-onnx",
        sv_model: None,
        files: ModelFiles {
            encoder: "encoder-epoch-99-avg-1.int8.onnx",
            decoder: "decoder-epoch-99-avg-1.int8.onnx",
            joiner: "joiner-epoch-99-avg-1.int8.onnx",
            tokens: "tokens.txt",
        },
    },
    AvailableModel {
        name: "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20",
        display_name: "中英双语 Zipformer — ~72 MB",
        size_bytes: 72_000_000,
        backend: "sherpa-onnx",
        sv_model: None,
        files: ModelFiles {
            encoder: "encoder-epoch-99-avg-1.onnx",
            decoder: "decoder-epoch-99-avg-1.onnx",
            joiner: "joiner-epoch-99-avg-1.onnx",
            tokens: "tokens.txt",
        },
    },
    // ── Hybrid backend (streaming + SenseVoice refinement) ──
    AvailableModel {
        name: "sherpa-onnx-streaming-zipformer-small-ctc-zh-int8-2025-04-01",
        display_name: "混合 CTC int8 [推荐] — ~176 MB",
        size_bytes: 21_000_000 + 155_000_000,
        backend: "hybrid",
        sv_model: Some(SvModelInfo {
            name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17",
            display_name: "SenseVoice 多语言 int8 — ~156 MB",
            size_bytes: 155_500_000,
            model_file: "model.int8.onnx",
            tokens_file: "tokens.txt",
        }),
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
    eprintln!("[check_stt_model] target_dir: {:?}, backend: {}", target_dir, config.asr.backend);

    let mut missing: Vec<String> = Vec::new();

    if config.asr.backend == "hybrid" {
        let streaming_is_ctc = config.asr.streaming_model == "zipformer-small-ctc";

        if streaming_is_ctc {
            // Check CTC (streaming) model files using config encoder field
            let ctc_dir = &config.asr.ctc_model_dir;
            let ctc_model = config.asr.encoder.as_str();
            for (label, file) in &[("ctc_model", ctc_model), ("ctc_tokens", "tokens.txt")] {
                let p = ctc_dir.join(file);
                let exists = p.exists();
                eprintln!("[check_stt_model] {} -> {}", p.display(), if exists { "OK" } else { "MISSING" });
                if !exists { missing.push(format!("ctc/{}", file)); }
            }
        } else {
            // Transducer streaming: check standard model files in model_dir
            let required = [
                config.asr.encoder.as_str(),
                config.asr.decoder.as_str(),
                config.asr.joiner.as_str(),
                config.asr.tokens.as_str(),
            ];
            for f in required.iter().filter(|f| !f.is_empty()) {
                let p = target_dir.join(f);
                let exists = p.exists();
                eprintln!("[check_stt_model] {} -> {}", p.display(), if exists { "OK" } else { "MISSING" });
                if !exists { missing.push(f.to_string()); }
            }
        }
        // Check SenseVoice (offline) model files
        let sv_dir = &config.asr.sv_model_dir;
        for (label, file) in &[("sv_model", config.asr.sv_model.as_str()), ("sv_tokens", config.asr.sv_tokens.as_str())] {
            let p = sv_dir.join(file);
            let exists = p.exists();
            eprintln!("[check_stt_model] {} -> {}", p.display(), if exists { "OK" } else { "MISSING" });
            if !exists { missing.push(format!("sv/{}", file)); }
        }
    } else {
        // Standard transducer model files
        let required = [
            config.asr.encoder.as_str(),
            config.asr.decoder.as_str(),
            config.asr.joiner.as_str(),
            config.asr.tokens.as_str(),
        ];
        for f in required.iter().filter(|f| !f.is_empty()) {
            let p = target_dir.join(f);
            let exists = p.exists();
            eprintln!("[check_stt_model] {} -> {}", p.display(), if exists { "OK" } else { "MISSING" });
            if !exists { missing.push(f.to_string()); }
        }
    }

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
        asr["backend"] = serde_yaml::Value::String(model.backend.to_string());
        asr["streaming_model"] = serde_yaml::Value::String(
            if model.backend == "hybrid" { "zipformer-small-ctc" } else { "transducer" }.to_string()
        );
        asr["model_name"] = serde_yaml::Value::String(model.name.to_string());
        asr["encoder"] = serde_yaml::Value::String(model.files.encoder.to_string());
        asr["decoder"] = serde_yaml::Value::String(model.files.decoder.to_string());
        asr["joiner"] = serde_yaml::Value::String(model.files.joiner.to_string());
        asr["tokens"] = serde_yaml::Value::String(model.files.tokens.to_string());

        if let Some(ref sv) = model.sv_model {
            asr["ctc_model_dir"] = serde_yaml::Value::String(format!("./models/{}", model.name));
            asr["sv_model_dir"] = serde_yaml::Value::String(format!("./models/{}", sv.name));
            asr["sv_model"] = serde_yaml::Value::String(sv.model_file.to_string());
            asr["sv_tokens"] = serde_yaml::Value::String(sv.tokens_file.to_string());
        }
    }

    let new_yaml = serde_yaml::to_string(&config)
        .map_err(|e| format!("Failed to serialize YAML: {}", e))?;

    std::fs::write(&stt_config_path, new_yaml)
        .map_err(|e| format!("Failed to write {}: {}", stt_config_path, e))?;

    eprintln!("[set_stt_model] model={} backend={}", model.name, model.backend);
    Ok(())
}

#[tauri::command]
pub fn set_stt_backend(stt_config_path: String, backend: String, provider: Option<String>) -> Result<(), String> {
    let content = std::fs::read_to_string(&stt_config_path)
        .map_err(|e| format!("Failed to read {}: {}", stt_config_path, e))?;

    let mut config: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|e| format!("Failed to parse YAML: {}", e))?;

    if let Some(asr) = config.get_mut("asr") {
        asr["backend"] = serde_yaml::Value::String(backend.clone());
        asr["streaming_model"] = serde_yaml::Value::String(
            if backend == "hybrid" { "zipformer-small-ctc" } else { "transducer" }.to_string()
        );
        if let Some(ref p) = provider {
            asr["provider"] = serde_yaml::Value::String(p.clone());
        }
    }

    let new_yaml = serde_yaml::to_string(&config)
        .map_err(|e| format!("Failed to serialize YAML: {}", e))?;

    std::fs::write(&stt_config_path, new_yaml)
        .map_err(|e| format!("Failed to write {}: {}", stt_config_path, e))?;

    Ok(())
}
