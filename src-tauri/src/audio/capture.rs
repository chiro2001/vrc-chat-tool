use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub index: usize,
}

pub struct AudioCapture {
    device: cpal::Device,
    config: cpal::StreamConfig,
}

/// Find the best supported input config for the device, preferring 16kHz mono.
fn find_best_config(device: &cpal::Device) -> anyhow::Result<cpal::StreamConfig> {
    let target_rate = 16000u32;

    // Try to find a config that supports exactly 16kHz with at least 1 channel
    let supported = device.supported_input_configs()?;
    let mut best_config: Option<(cpal::StreamConfig, u32)> = None;

    for range in supported {
        let min = range.min_sample_rate().0;
        let max = range.max_sample_rate().0;
        let ch = range.channels();

        // If 16kHz is in this range, use it immediately
        if target_rate >= min && target_rate <= max {
            return Ok(cpal::StreamConfig {
                sample_rate: cpal::SampleRate(target_rate),
                channels: ch.min(2), // prefer mono/stereo
                buffer_size: cpal::BufferSize::Default,
            });
        }

        // Otherwise find the closest supported rate
        let diff = if target_rate < min {
            min - target_rate
        } else {
            target_rate - max
        };

        if best_config.is_none() || diff < best_config.as_ref().unwrap().1 {
            let actual_rate = if target_rate < min { min } else { max };
            best_config = Some((
                cpal::StreamConfig {
                    sample_rate: cpal::SampleRate(actual_rate),
                    channels: ch.min(2),
                    buffer_size: cpal::BufferSize::Default,
                },
                diff,
            ));
        }
    }

    best_config
        .map(|(cfg, _)| cfg)
        .ok_or_else(|| anyhow::anyhow!("No supported input configuration found for device"))
}

impl AudioCapture {
    pub fn list_devices() -> anyhow::Result<Vec<AudioDeviceInfo>> {
        let host = cpal::default_host();
        let mut devices: Vec<AudioDeviceInfo> = host
            .input_devices()?
            .enumerate()
            .map(|(index, device)| {
                let name = device
                    .name()
                    .unwrap_or_else(|_| format!("Device {}", index));
                AudioDeviceInfo { name, index }
            })
            .collect();
        devices.sort_by_key(|d| d.index);
        Ok(devices)
    }

    pub fn new() -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("No input device found"))?;
        let config = find_best_config(&device)?;
        Ok(Self { device, config })
    }

    pub fn new_by_device(device: cpal::Device, config: cpal::StreamConfig) -> Self {
        Self { device, config }
    }

    pub fn new_by_index(index: usize) -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host
            .input_devices()?
            .nth(index)
            .ok_or_else(|| anyhow::anyhow!("Device index {} not found", index))?;
        let config = find_best_config(&device)?;
        Ok(Self { device, config })
    }

    pub fn capture_streaming<F>(
        &self,
        on_chunk: F,
        stop_signal: Arc<AtomicBool>,
    ) -> anyhow::Result<()>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let input_sample_rate = self.config.sample_rate.0;
        let channels = self.config.channels;
        let target_sample_rate = 16000u32;
        let chunk_size = 6400;

        // Use shared buffer so we can flush remaining data after stream stops
        let buffer = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let buffer_clone = buffer.clone();
        let stop_signal_clone = stop_signal.clone();

        let stream = self.device.build_input_stream(
            &self.config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // g. Check stop signal at start of each callback
                if stop_signal_clone.load(Ordering::Relaxed) {
                    // Flush any remaining buffered data before returning
                    let mut buf = buffer_clone.lock().unwrap();
                    if !buf.is_empty() {
                        let remaining = std::mem::take(&mut *buf);
                        drop(buf);
                        on_chunk(remaining);
                    }
                    return;
                }

                // a. Extract mono from samples
                let mut mono = Vec::with_capacity(data.len() / channels as usize);
                for (i, &sample) in data.iter().enumerate() {
                    if i % channels as usize == 0 {
                        mono.push(sample.clamp(-1.0, 1.0));
                    }
                }

                // b. Resample to 16kHz if needed
                let resampled = if input_sample_rate != target_sample_rate {
                    let ratio = input_sample_rate as f32 / target_sample_rate as f32;
                    let output_len = (mono.len() as f32 / ratio).round() as usize;
                    let mut out = Vec::with_capacity(output_len.max(1));
                    if output_len > 0 {
                        for i in 0..output_len {
                            let pos = (i as f32) * ratio;
                            let idx0 = pos.floor() as usize;
                            let idx1 = (idx0 + 1).min(mono.len().saturating_sub(1));
                            let frac = pos - idx0 as f32;
                            let s0 = mono[idx0.min(mono.len().saturating_sub(1))];
                            let s1 = mono[idx1.min(mono.len().saturating_sub(1))];
                            out.push(s0 * (1.0 - frac) + s1 * frac);
                        }
                    }
                    out
                } else {
                    mono
                };

                // Skip if resampled is empty (no audio in this callback)
                if resampled.is_empty() {
                    return;
                }

                // c/d. Convert f32 to i16 PCM bytes
                let mut buf = buffer_clone.lock().unwrap();
                for &sample in &resampled {
                    let s16 = (sample * 32767.0) as i16;
                    buf.extend_from_slice(&s16.to_le_bytes());
                }

                // e-f. Send complete chunks
                while buf.len() >= chunk_size {
                    let chunk = buf[..chunk_size].to_vec();
                    buf.drain(..chunk_size);
                    drop(buf);
                    on_chunk(chunk);
                    buf = buffer_clone.lock().unwrap();
                }
            },
            |err| eprintln!("Audio stream error: {}", err),
            None,
        )?;

        stream.play()?;

        // Block until stop signal is received
        while !stop_signal.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        drop(stream);
        Ok(())
    }
}

/// Convert f32 PCM samples (possibly multi-channel) to 16-bit mono PCM bytes at target sample rate.
pub fn samples_to_pcm16(
    samples: &[f32],
    input_sample_rate: u32,
    target_sample_rate: u32,
    channels: u16,
) -> Vec<u8> {
    // Extract mono (first channel)
    let mut mono = Vec::with_capacity(samples.len() / channels as usize);
    for (i, &sample) in samples.iter().enumerate() {
        if i % channels as usize == 0 {
            mono.push(sample.clamp(-1.0, 1.0));
        }
    }

    // Resample using linear interpolation
    let resampled = if input_sample_rate != target_sample_rate {
        let ratio = input_sample_rate as f32 / target_sample_rate as f32;
        let output_len = (mono.len() as f32 / ratio).round() as usize;
        let mut out = Vec::with_capacity(output_len);
        for i in 0..output_len {
            let pos = (i as f32) * ratio;
            let idx0 = pos.floor() as usize;
            let idx1 = (idx0 + 1).min(mono.len() - 1);
            let frac = pos - idx0 as f32;
            let s0 = mono[idx0];
            let s1 = mono[idx1];
            out.push(s0 * (1.0 - frac) + s1 * frac);
        }
        out
    } else {
        mono
    };

    // Convert f32 to i16 PCM bytes (little-endian)
    let mut pcm = Vec::with_capacity(resampled.len() * 2);
    for &sample in &resampled {
        let s16 = (sample * 32767.0) as i16;
        pcm.extend_from_slice(&s16.to_le_bytes());
    }
    pcm
}

/// Build a valid 44-byte RIFF WAV header.
pub fn create_wav_header(
    data_len: u32,
    sample_rate: u32,
    bits_per_sample: u16,
    channels: u16,
) -> Vec<u8> {
    let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
    let block_align = channels * bits_per_sample / 8;
    let file_size = 36 + data_len;

    let mut header = Vec::with_capacity(44);

    // RIFF chunk
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&file_size.to_le_bytes());
    header.extend_from_slice(b"WAVE");

    // fmt subchunk
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16u32.to_le_bytes()); // subchunk size (PCM)
    header.extend_from_slice(&1u16.to_le_bytes()); // audio format (linear PCM = 1)
    header.extend_from_slice(&channels.to_le_bytes());
    header.extend_from_slice(&sample_rate.to_le_bytes());
    header.extend_from_slice(&byte_rate.to_le_bytes());
    header.extend_from_slice(&block_align.to_le_bytes());
    header.extend_from_slice(&bits_per_sample.to_le_bytes());

    // data subchunk
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_len.to_le_bytes());

    header
}

/// Calculate RMS of f32 samples and return a normalized 0-1 dB-like value.
pub fn calculate_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    let rms = (sum_sq / samples.len() as f32).sqrt();
    if rms <= 0.0 {
        return 0.0;
    }
    // dB relative to noise floor (0.0001), normalized to 0-1
    // 0dB (rms=0.0001) -> 0, 80dB (rms=1.0) -> 1.0
    let db = 20.0 * (rms / 0.0001).log10();
    (db / 80.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcm_conversion_mono() {
        // Sine wave: 440Hz, 1 sec at 48kHz -> down to 16kHz
        let freq = 440.0;
        let input_rate = 48000;
        let _duration_secs = 1.0;
        let samples: Vec<f32> = (0..(input_rate as usize))
            .map(|i| {
                (2.0 * std::f32::consts::PI * freq * i as f32 / input_rate as f32).sin() * 0.5
            })
            .collect();
        let pcm = samples_to_pcm16(&samples, input_rate, 16000, 1);
        // Should produce ~16000 samples * 2 bytes = 32000 bytes
        assert!(pcm.len() >= 30000 && pcm.len() <= 34000);
        // First few bytes should not all be zero
        assert!(pcm[0..4].iter().any(|&b| b != 0));
    }

    #[test]
    fn test_pcm_conversion_stereo_to_mono() {
        let input_rate = 44100;
        let samples: Vec<f32> = (0..(input_rate as usize * 2))
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.3 })
            .collect();
        let pcm = samples_to_pcm16(&samples, input_rate, 16000, 2);
        // Stereo->mono (left only), then 44.1k->16k resample
        assert!(pcm.len() > 100);
    }

    #[test]
    fn test_create_wav_header() {
        let header = create_wav_header(64000, 16000, 16, 1);
        assert_eq!(&header[0..4], b"RIFF");
        assert_eq!(&header[8..12], b"WAVE");
        assert_eq!(&header[12..16], b"fmt ");
        // bits_per_sample at offset 34
        assert_eq!(header[34], 16);
    }

    #[test]
    fn test_calculate_rms_silence() {
        let samples = vec![0.0f32; 1000];
        let rms = calculate_rms(&samples);
        assert!(rms < 0.001);
    }

    #[test]
    fn test_calculate_rms_sine() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.01).sin()).collect();
        let rms = calculate_rms(&samples);
        assert!(rms > 0.0 && rms < 1.0);
    }
}
