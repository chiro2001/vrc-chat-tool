//! Model download module — downloads Sherpa-ONNX model archives from GitHub Releases.
//!
//! Mirrors the Python `download_models.py` functionality in pure Rust.

use anyhow::{Context as _, Result};
use bzip2::read::BzDecoder;
use std::io::{BufReader, Write};
use std::path::Path;
use std::time::Duration;
use tar::Archive;

use crate::config::Config;

/// Archive download source URLs.
const RELEASES_BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download";
const USER_AGENT: &str = "stt-server/0.1";

/// Required files for ASR model validation.
/// These are DEFAULT names; the actual verification uses config-specified filenames.
const ASR_REQUIRED_DEFAULT: &[&str] = &[
    "encoder.int8.onnx",
    "decoder.onnx",
    "joiner.int8.onnx",
    "tokens.txt",
];

/// Required files for punctuation model validation.
const PUNCT_REQUIRED_DEFAULT: &[&str] = &["model.int8.onnx"];

/// Build the required files list from config fields.
fn asr_required_from_config(config: &Config) -> Vec<&str> {
    vec![
        config.asr.encoder.as_str(),
        config.asr.decoder.as_str(),
        config.asr.joiner.as_str(),
        config.asr.tokens.as_str(),
    ]
}

fn punct_required_from_config(config: &Config) -> Vec<&str> {
    if config.punctuation.enabled && !config.punctuation.model_file.is_empty() {
        vec![config.punctuation.model_file.as_str()]
    } else {
        vec!["model.int8.onnx"]
    }
}

/// Download and extract ASR and punctuation models.
///
/// Returns `Ok(())` if all models are present and valid after completion.
pub fn download_models(config: &Config, force: bool, no_punct: bool) -> Result<()> {
    let model_dir = &config.asr.model_dir;
    std::fs::create_dir_all(model_dir).context("Failed to create model directory")?;

    let asr_files = asr_required_from_config(config);

    // --- ASR model ---
    let asr_url = format!(
        "{}/asr-models/{}.tar.bz2",
        RELEASES_BASE, config.asr.model_name,
    );
    download_single_model(
        &asr_url,
        &config.asr.model_name,
        model_dir,
        &asr_files,
        force,
    )?;

    // --- Punctuation model ---
    if !no_punct
        && config.punctuation.enabled
        && !config.punctuation.model_name.is_empty()
    {
        let punct_files = punct_required_from_config(config);
        let punct_url = format!(
            "{}/punctuation-models/{}.tar.bz2",
            RELEASES_BASE, config.punctuation.model_name,
        );
        download_single_model(
            &punct_url,
            &config.punctuation.model_name,
            model_dir,
            &punct_files,
            force,
        )?;
    } else if no_punct {
        eprintln!("[download] Punctuation model download skipped (--no-punct)");
    } else {
        tracing::info!("Punctuation model download skipped (disabled in config)");
    }

    eprintln!("[download] All models downloaded and verified successfully.");
    Ok(())
}

/// Download, extract, and verify a single model archive.
fn download_single_model(
    url: &str,
    model_name: &str,
    model_dir: &Path,
    required_files: &[&str],
    force: bool,
) -> Result<()> {
    let target_dir = model_dir.join(model_name);

    // Skip if already present and valid
    if !force && target_dir.is_dir() {
        let missing = verify_model_files(&target_dir, required_files);
        if missing.is_empty() {
            tracing::info!("Model already exists, skipping: {}", model_name);
            return Ok(());
        }
        tracing::warn!(
            "Missing files for {}: {:?} — re-downloading",
            model_name,
            missing,
        );
    }

    // Remove existing if force
    if force && target_dir.is_dir() {
        tracing::info!("Removing existing directory: {}", target_dir.display());
        std::fs::remove_dir_all(&target_dir)
            .with_context(|| format!("Failed to remove {}", target_dir.display()))?;
    }

    tracing::info!("Downloading model: {} ...", model_name);

    // Stream download with retries
    let archive_bytes = download_with_retry(url, 3)?;
    tracing::info!("Downloaded {} MB", archive_bytes.len() / (1024 * 1024));

    // Extract from memory
    extract_tar_bz2(&archive_bytes, model_dir, model_name)?;

    // Verify
    let missing = verify_model_files(&target_dir, required_files);
    if !missing.is_empty() {
        anyhow::bail!(
            "Verification failed for {} — missing files: {:?}",
            model_name,
            missing,
        );
    }
    tracing::info!("Model verified successfully: {}", model_name);

    Ok(())
}

/// Download a URL with exponential backoff retry.
fn download_with_retry(url: &str, max_retries: u32) -> Result<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(600))
        .build()?;

    let mut last_error = None;
    for attempt in 1..=max_retries {
        match client.get(url).send() {
            Ok(response) => {
                if !response.status().is_success() {
                    anyhow::bail!(
                        "HTTP {} from {}",
                        response.status().as_u16(),
                        url,
                    );
                }
                let bytes = response.bytes()?;
                return Ok(bytes.to_vec());
            }
            Err(e) => {
                last_error = Some(e);
                if attempt < max_retries {
                    let wait = Duration::from_secs(2u64.pow(attempt));
                    tracing::warn!(
                        "Download attempt {}/{} failed: {}. Retrying in {}s...",
                        attempt,
                        max_retries,
                        last_error.as_ref().unwrap(),
                        wait.as_secs(),
                    );
                    std::thread::sleep(wait);
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "All {} download attempts failed. Last error: {}",
        max_retries,
        last_error.unwrap(),
    ))
}

/// Extract a .tar.bz2 byte buffer into model_dir/model_name.
fn extract_tar_bz2(data: &[u8], model_dir: &Path, model_name: &str) -> Result<()> {
    let decompressor = BzDecoder::new(BufReader::new(data));
    let mut archive = Archive::new(decompressor);

    let target = model_dir.join(model_name);

    // The sherpa-onnx archives contain a single top-level directory named after
    // the model. Extract into a temp location first, then move to proper name.
    let tmp_extract = model_dir.join(format!(".tmp_{}", model_name));
    if tmp_extract.exists() {
        std::fs::remove_dir_all(&tmp_extract)?;
    }
    std::fs::create_dir_all(&tmp_extract)?;

    archive.unpack(&tmp_extract).context("Failed to extract archive")?;

    // Check if there's a single top-level directory
    let entries: Vec<_> = std::fs::read_dir(&tmp_extract)?
        .filter_map(|e| e.ok())
        .collect();

    if entries.len() == 1 && entries[0].file_type()?.is_dir() {
        // Single directory → rename it to model_name
        let src = entries[0].path();
        if target.exists() {
            std::fs::remove_dir_all(&target)?;
        }
        std::fs::rename(&src, &target)?;
    } else {
        // Multiple files → move them all into target
        if target.exists() {
            std::fs::remove_dir_all(&target)?;
        }
        std::fs::create_dir_all(&target)?;
        for entry in &entries {
            let dest = target.join(entry.file_name());
            std::fs::rename(entry.path(), dest)?;
        }
    }

    // Cleanup temp
    if tmp_extract.exists() {
        let _ = std::fs::remove_dir_all(&tmp_extract);
    }

    Ok(())
}

/// Verify that all required files exist in the model directory.
fn verify_model_files(model_dir: &Path, required: &[&str]) -> Vec<String> {
    let mut missing = Vec::new();
    for name in required {
        if !model_dir.join(name).exists() {
            missing.push(name.to_string());
        }
    }
    missing
}

// ---------------------------------------------------------------------------
// Progress-aware download API — for use by Tauri frontend integration
// ---------------------------------------------------------------------------

/// Download models with progress reporting via a callback.
///
/// The callback receives `(phase: &str, current: u64, total: u64)`:
/// - `phase`: "connecting", "download_asr", "extract_asr", "verify_asr",
///            "download_punct", "extract_punct", "verify_punct", "complete"
/// - `current` / `total`: bytes for download phases, 0-based for extract/verify
pub fn download_models_with_progress<F>(
    config: &Config,
    force: bool,
    no_punct: bool,
    on_progress: &F,
) -> Result<()>
where
    F: Fn(&str, u64, u64),
{
    let model_dir = &config.asr.model_dir;
    std::fs::create_dir_all(model_dir).context("Failed to create model directory")?;

    on_progress("connecting", 0, 0);

    let asr_files = asr_required_from_config(config);

    // --- ASR model ---
    let asr_url = format!(
        "{}/asr-models/{}.tar.bz2",
        RELEASES_BASE, config.asr.model_name,
    );
    eprintln!("[download] ASR URL: {}", asr_url);
    download_single_model_with_progress(
        &asr_url,
        &config.asr.model_name,
        model_dir,
        &asr_files,
        force,
        on_progress,
        "asr",
    )?;

    // --- Punctuation model ---
    if !no_punct
        && config.punctuation.enabled
        && !config.punctuation.model_name.is_empty()
    {
        let punct_files = punct_required_from_config(config);
        let punct_url = format!(
            "{}/punctuation-models/{}.tar.bz2",
            RELEASES_BASE, config.punctuation.model_name,
        );
        download_single_model_with_progress(
            &punct_url,
            &config.punctuation.model_name,
            model_dir,
            &punct_files,
            force,
            on_progress,
            "punct",
        )?;
    }

    eprintln!("[download] Download complete, all models verified.");
    on_progress("complete", 0, 0);
    Ok(())
}

/// Progress-aware single model download.
fn download_single_model_with_progress<F>(
    url: &str,
    model_name: &str,
    model_dir: &Path,
    required_files: &[&str],
    force: bool,
    on_progress: &F,
    label: &str,
) -> Result<()>
where
    F: Fn(&str, u64, u64),
{
    let target_dir = model_dir.join(model_name);

    // Skip if already present and valid
    if !force && target_dir.is_dir() {
        let missing = verify_model_files(&target_dir, required_files);
        if missing.is_empty() {
            on_progress(&format!("download_{}", label), 100, 100);
            on_progress(&format!("extract_{}", label), 1, 1);
            on_progress(&format!("verify_{}", label), 1, 1);
            return Ok(());
        }
    }

    // Remove existing if force
    if force && target_dir.is_dir() {
        std::fs::remove_dir_all(&target_dir)
            .with_context(|| format!("Failed to remove {}", target_dir.display()))?;
    }

    on_progress(&format!("download_{}", label), 0, 0);

    let archive_bytes = download_with_retry_progress(url, 3, |current, total| {
        on_progress(&format!("download_{}", label), current, total);
    })?;

    on_progress(&format!("extract_{}", label), 0, 1);
    extract_tar_bz2(&archive_bytes, model_dir, model_name)?;
    on_progress(&format!("extract_{}", label), 1, 1);

    on_progress(&format!("verify_{}", label), 0, 1);
    let missing = verify_model_files(&target_dir, required_files);
    if !missing.is_empty() {
        anyhow::bail!(
            "Verification failed for {} — missing files: {:?}",
            model_name,
            missing,
        );
    }
    on_progress(&format!("verify_{}", label), 1, 1);

    Ok(())
}

/// Download a URL with retry, calling a progress closure on each chunk.
fn download_with_retry_progress<F>(
    url: &str,
    max_retries: u32,
    on_progress: F,
) -> Result<Vec<u8>>
where
    F: Fn(u64, u64),
{
    eprintln!("[download] Starting download: {}", url);

    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(600))
        .build()?;

    let mut last_error = None;
    for attempt in 1..=max_retries {
        eprintln!("[download] Attempt {}/{}", attempt, max_retries);
        match client.get(url).send() {
            Ok(mut response) => {
                let status = response.status();
                eprintln!("[download] HTTP {}", status.as_u16());
                if !status.is_success() {
                    let err = anyhow::anyhow!(
                        "HTTP {} from {}",
                        status.as_u16(),
                        url,
                    );
                    eprintln!("[download] Error: {}", err);
                    return Err(err);
                }

                let total = response.content_length().unwrap_or(0);
                eprintln!("[download] Content-Length: {} bytes ({:.1} MB)", total, total as f64 / 1048576.0);
                let mut writer = ProgressWriter {
                    inner: if total > 0 {
                        Vec::with_capacity(total as usize)
                    } else {
                        Vec::new()
                    },
                    downloaded: 0,
                    total,
                    on_progress: &on_progress,
                };

                response
                    .copy_to(&mut writer)
                    .context("Failed to read response body")?;

                return Ok(writer.inner);
            }
            Err(e) => {
                last_error = Some(e);
                if attempt < max_retries {
                    let wait = Duration::from_secs(2u64.pow(attempt));
                    tracing::warn!(
                        "Download attempt {}/{} failed: {}. Retrying in {}s...",
                        attempt,
                        max_retries,
                        last_error.as_ref().unwrap(),
                        wait.as_secs(),
                    );
                    std::thread::sleep(wait);
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "All {} download attempts failed. Last error: {}",
        max_retries,
        last_error.unwrap(),
    ))
}

/// Writer adapter that reports download progress.
struct ProgressWriter<'a, F: Fn(u64, u64)> {
    inner: Vec<u8>,
    downloaded: u64,
    total: u64,
    on_progress: &'a F,
}

impl<F: Fn(u64, u64)> Write for ProgressWriter<'_, F> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)?;
        self.downloaded += buf.len() as u64;
        (self.on_progress)(self.downloaded, self.total);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Check basic network connectivity by HEAD-requesting github.com.
pub fn check_network_connectivity() -> bool {
    let client = match reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    for host in &["https://github.com", "https://google.com"] {
        if client.head(*host).send().is_ok() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_model_files() {
        let tmp = std::env::temp_dir().join("stt_test_verify");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // No files → all missing
        let missing = verify_model_files(&tmp, ASR_REQUIRED_DEFAULT);
        assert_eq!(missing.len(), ASR_REQUIRED_DEFAULT.len());

        // Create one file → fewer missing
        std::fs::write(tmp.join("encoder.int8.onnx"), b"fake").unwrap();
        let missing = verify_model_files(&tmp, ASR_REQUIRED_DEFAULT);
        assert_eq!(missing.len(), ASR_REQUIRED_DEFAULT.len() - 1);
        assert!(!missing.contains(&"encoder.int8.onnx".to_string()));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
