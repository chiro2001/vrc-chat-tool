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
/// The thread reads from cpal and sends f32 mono chunks via `tx`.
/// If `device_name` is provided AND not "Default", finds the matching input device.
/// Otherwise uses the system default input device.
///
/// Returns a stop function that the caller MUST call to stop capture and
/// release the audio stream. Dropping without calling leaks the stream.
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
            // Find device by name (case-insensitive substring match)
            let lower = name.to_lowercase();
            let devices: Vec<cpal::Device> = host.input_devices()?.collect();
            let found = devices.into_iter().find(|d| {
                d.name().map(|n| n.to_lowercase().contains(&lower)).unwrap_or(false)
            });
            match found {
                Some(d) => d,
                None => {
                    // Fall back to default if specified device not found
                    eprintln!(
                        "[Audio] Device '{}' not found, falling back to default",
                        name
                    );
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
    eprintln!("[Audio] Using device: {}", dev_name);

    // Build config: try 16000Hz mono with Fixed(1600) first.
    // If that fails, fall back to Default buffer size.
    let desired_config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(16000),
        buffer_size: cpal::BufferSize::Fixed(1600), // 100ms at 16kHz
    };

    let (stop_tx, _stop_rx) = std::sync::mpsc::channel::<()>();
    let stop_flag = Arc::new(AtomicBool::new(false));

    // Clones for multiple fallback attempts
    let tx_fb1 = tx.clone();
    let sf_fb1 = Arc::clone(&stop_flag);
    let tx_fb2 = tx.clone();
    let sf_fb2 = Arc::clone(&stop_flag);

    // Attempt 1: desired config (16kHz mono Fixed(1600))
    let stream_result = device.build_input_stream(
        &desired_config,
        {
            let stop_flag = Arc::clone(&stop_flag);
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if stop_flag.load(Ordering::Relaxed) {
                    return;
                }
                let _ = tx.send(data.to_vec());
            }
        },
        |err| {
            eprintln!("[Audio] Capture error: {}", err);
        },
        None,
    );

    let stream = match stream_result {
        Ok(s) => s,
        Err(e_fixed) => {
            // Attempt 2: Default buffer size (same sample rate, channels)
            eprintln!(
                "[Audio] Fixed(1600) not supported on this device, trying Default buffer: {}",
                e_fixed
            );
            let fallback_config = cpal::StreamConfig {
                channels: 1,
                sample_rate: cpal::SampleRate(16000),
                buffer_size: cpal::BufferSize::Default,
            };
            match device.build_input_stream(
                &fallback_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if sf_fb1.load(Ordering::Relaxed) {
                        return;
                    }
                    let _ = tx_fb1.send(data.to_vec());
                },
                |err| {
                    eprintln!("[Audio] Capture error: {}", err);
                },
                None,
            ) {
                Ok(s) => {
                    eprintln!("[Audio] Default buffer size OK");
                    s
                }
                Err(e_default) => {
                    // Attempt 3: use device's native default config
                    let native = device.default_input_config().map_err(|e| {
                        anyhow::anyhow!(
                            "Fixed(1600): {}, Default: {}, Native config query: {}",
                            e_fixed, e_default, e
                        )
                    })?;
                    let native_config = native.config();
                    eprintln!(
                        "[Audio] 16kHz not supported, using native: {:?} ch={:?} buffer={:?}",
                        native_config.sample_rate,
                        native.channels(),
                        native_config.buffer_size,
                    );
                    device.build_input_stream(
                        &native_config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if sf_fb2.load(Ordering::Relaxed) {
                                return;
                            }
                            let _ = tx_fb2.send(data.to_vec());
                        },
                        |err| {
                            eprintln!("[Audio] Capture error: {}", err);
                        },
                        None,
                    ).map_err(|e| {
                        anyhow::anyhow!(
                            "Fixed(1600): {}, Default: {}, Native: {}",
                            e_fixed, e_default, e
                        )
                    })?
                }
            }
        }
    };

    stream.play()?;
    let mut stream_opt = Some(stream);

    Ok(Box::new(move || {
        // Signal the callback to stop
        stop_flag.store(true, Ordering::Relaxed);
        let _ = stop_tx.send(());
        // Drop the stream — cpal stops capture on drop
        drop(stream_opt.take());
    }))
}