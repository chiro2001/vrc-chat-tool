//! Audio I/O helpers — microphone capture and WAV file reading.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;

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
/// Returns a stop function that the caller must call to stop capture.
pub fn start_mic_capture(tx: mpsc::Sender<Vec<f32>>) -> anyhow::Result<Box<dyn FnOnce()>> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No input device found"))?;

    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(16000),
        buffer_size: cpal::BufferSize::Fixed(1600), // 100ms at 16kHz
    };

    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if stop_rx.try_recv().is_ok() {
                return;
            }
            let _ = tx.send(data.to_vec());
        },
        |err| {
            eprintln!("[Audio] Capture error: {}", err);
        },
        None,
    )?;

    stream.play()?;

    // Return a stop function + keep stream alive
    let stream_handle = Box::leak(Box::new(stream)); // leak to keep alive in background
    Ok(Box::new(move || {
        let _ = stop_tx.send(());
        let _ = stream_handle;
    }))
}
