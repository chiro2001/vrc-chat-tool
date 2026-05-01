//! Audio I/O helpers — microphone capture and WAV file reading.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// List available audio input devices via cpal.
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();

    match host.input_devices() {
        Ok(devices) => {
            let names: Vec<String> = devices
                .filter_map(|d: cpal::Device| d.name().ok())
                .collect();
            if names.is_empty() {
                vec!["Default".to_string()]
            } else {
                names
            }
        }
        Err(_) => vec!["Default".to_string()],
    }
}

/// Read a WAV file and return (samples: Vec<f32>, sample_rate: u32).
///
/// Handles stereo → mono conversion and integer → float32 normalization.
pub fn read_wav(path: &std::path::Path) -> anyhow::Result<(Vec<f32>, u32)> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as usize;

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            let floats: Vec<f32> = reader.samples::<f32>().filter_map(|s| s.ok()).collect();
            if channels == 2 {
                // Stereo → mono (average channels)
                floats
                    .chunks_exact(2)
                    .map(|pair| (pair[0] + pair[1]) * 0.5)
                    .collect()
            } else {
                floats
            }
        }
        hound::SampleFormat::Int => {
            let ints: Vec<i16> = reader.samples::<i16>().filter_map(|s| s.ok()).collect();
            let scale = 32768.0f32;
            if channels == 2 {
                ints.chunks_exact(2)
                    .map(|pair| ((pair[0] as f32 + pair[1] as f32) * 0.5) / scale)
                    .collect()
            } else {
                ints.iter().map(|&s| (s as f32) / scale).collect()
            }
        }
    };

    Ok((samples, sample_rate))
}

/// Spawn a microphone capture thread.
///
/// Always outputs **16kHz mono f32** to `tx`, resampling internally
/// when the device's native format differs.
///
/// Returns a stop function that the caller MUST call to stop capture and
/// release the audio stream.
pub fn start_mic_capture(
    tx: mpsc::Sender<Vec<f32>>,
    device_name: Option<&str>,
) -> anyhow::Result<Box<dyn FnOnce()>> {
    let host = cpal::default_host();

    let device = if let Some(name) = device_name {
        if name.is_empty() || name == "Default" {
            host.default_input_device()
                .ok_or_else(|| anyhow::anyhow!("No default input device found"))?
        } else {
            let lower = name.to_lowercase();
            let devices: Vec<cpal::Device> = host.input_devices()?.collect();
            let found = devices.into_iter().find(|d| {
                d.name().map(|n| n.to_lowercase().contains(&lower)).unwrap_or(false)
            });
            match found {
                Some(d) => d,
                None => {
                    eprintln!("[Audio] Device '{}' not found, using default", name);
                    host.default_input_device()
                        .ok_or_else(|| anyhow::anyhow!("No input device found"))?
                }
            }
        }
    } else {
        host.default_input_device()
            .ok_or_else(|| anyhow::anyhow!("No default input device found"))?
    };

    let dev_name = device.name().unwrap_or_else(|_| "unknown".into());
    eprintln!("[Audio] Device: {}", dev_name);

    // ── Determine stream config ──
    let target_rate = 16000u32;
    let (config, needs_resample, native_rate, native_ch) =
        choose_config(&device, target_rate)?;

    if needs_resample {
        eprintln!(
            "[Audio] Resampling {}ch {}Hz -> 1ch {}Hz",
            native_ch, native_rate.0, target_rate
        );
    }

    let (stop_tx, _stop_rx) = std::sync::mpsc::channel::<()>();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let sf_rs = Arc::clone(&stop_flag);
    let tx_rs = tx.clone();

    // Build the stream: either direct (16kHz native) or with resampling
    let stream = if needs_resample {
        device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if sf_rs.load(Ordering::Relaxed) {
                    return;
                }
                let resampled = downsample_to_mono_16k(data, native_rate.0, native_ch);
                if !resampled.is_empty() {
                    let _ = tx_rs.send(resampled);
                }
            },
            |err| eprintln!("[Audio] Error: {}", err),
            None,
        )?
    } else {
        device.build_input_stream(
            &config,
            {
                let stop_flag = Arc::clone(&stop_flag);
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if stop_flag.load(Ordering::Relaxed) {
                        return;
                    }
                    let _ = tx.send(data.to_vec());
                }
            },
            |err| eprintln!("[Audio] Error: {}", err),
            None,
        )?
    };

    stream.play()?;
    let mut stream_opt = Some(stream);

    Ok(Box::new(move || {
        stop_flag.store(true, Ordering::Relaxed);
        let _ = stop_tx.send(());
        drop(stream_opt.take());
    }))
}

/// Decide which cpal config to use, and whether resampling is needed.
fn choose_config(
    device: &cpal::Device,
    target_rate: u32,
) -> anyhow::Result<(cpal::StreamConfig, bool, cpal::SampleRate, u16)> {
    // Check supported configs for 16kHz
    let supports_16k = device
        .supported_input_configs()
        .map(|cfgs| {
            cfgs.into_iter().any(|c| {
                c.min_sample_rate() <= cpal::SampleRate(target_rate)
                    && c.max_sample_rate() >= cpal::SampleRate(target_rate)
                    && c.channels() >= 1
            })
        })
        .unwrap_or(false);

    if supports_16k {
        let config = cpal::StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(target_rate),
            buffer_size: cpal::BufferSize::Default,
        };
        return Ok((config, false, cpal::SampleRate(0), 0));
    }

    // Not supported — use native config with resampling
    let native = device.default_input_config()?;
    let native_rate = native.sample_rate();
    let native_ch = native.channels().min(2);
    let config = native.config();
    Ok((config, true, native_rate, native_ch))
}

/// Downsample multi-channel multi-rate audio to 16kHz mono f32.
///
/// Uses simple averaging: for each output sample at 16kHz,
/// averages the corresponding `factor` input samples.
fn downsample_to_mono_16k(data: &[f32], in_rate: u32, in_ch: u16) -> Vec<f32> {
    let factor = (in_rate as usize / 16000) * in_ch as usize;
    if factor == 1 {
        return data.to_vec(); // already 16kHz mono
    }
    let out_len = data.len() / factor;
    let mut out = Vec::with_capacity(out_len);
    for chunk in data.chunks_exact(factor) {
        let avg = chunk.iter().sum::<f32>() / factor as f32;
        out.push(avg);
    }
    out
}