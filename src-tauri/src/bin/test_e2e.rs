//! End-to-end audio capture test using VB-Cable
//! Usage: cargo run --bin test_e2e -- <duration_secs> <output_wav_path>
//! Example: cargo run --bin test_e2e -- 3 tmp/e2e_capture.wav

use std::sync::{Arc, atomic::{AtomicBool, Ordering}, Mutex};
use std::time::Duration;
use std::thread;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use vrc_chat_tool::config::AppConfig;
use vrc_chat_tool::speech::streaming::StreamingRecognizer;
use vrc_chat_tool::osc::sender::OscSender;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let duration_secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let output_path = args.get(2).cloned().unwrap_or_else(|| "tmp/e2e_capture.wav".to_string());

    println!("E2E Audio Capture Test");
    println!("Duration: {}s, Output: {}", duration_secs, output_path);

    // Find VB-Cable input device
    let host = cpal::default_host();
    let devices: Vec<_> = host.input_devices()?.collect();
    
    let device = devices.iter().find(|d| {
        d.name().map(|n| n.contains("CABLE")).unwrap_or(false)
    }).ok_or("VB-Cable capture device not found. Is VB-Cable installed?")?;
    
    println!("Using device: {}", device.name()?);

    // Try to find supported config (prefer 16kHz mono)
    let config = find_supported_config(device)?;
    println!("Config: {}Hz, {}ch", config.sample_rate.0, config.channels);

    // Shared PCM buffer
    let pcm_buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
    let pcm_buffer_clone = pcm_buffer.clone();
    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_signal_clone = stop_signal.clone();
    let sample_rate = config.sample_rate.0;
    let channels = config.channels;

    // Build input stream
    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if stop_signal_clone.load(Ordering::Relaxed) {
                return;
            }
            // Extract mono, convert to i16 PCM
            let mut mono = Vec::with_capacity(data.len() / channels as usize);
            for (i, &sample) in data.iter().enumerate() {
                if i % channels as usize == 0 {
                    mono.push(sample.clamp(-1.0, 1.0));
                }
            }
            for sample in &mono {
                let s16 = (*sample * 32767.0) as i16;
                pcm_buffer_clone.lock().unwrap().extend_from_slice(&s16.to_le_bytes());
            }
        },
        |err| eprintln!("Stream error: {}", err),
        None,
    )?;

    stream.play()?;
    println!("Recording for {} seconds...", duration_secs);
    thread::sleep(Duration::from_secs(duration_secs));

    stop_signal.store(true, Ordering::SeqCst);
    drop(stream);

    let pcm_data = pcm_buffer.lock().unwrap().clone();
    println!("Captured {} bytes of PCM data", pcm_data.len());

    if pcm_data.is_empty() {
        eprintln!("ERROR: No audio data captured!");
        std::process::exit(1);
    }

    // Write WAV file
    let header = create_wav_header(pcm_data.len() as u32, sample_rate, 16, 1);
    let mut wav_data = header;
    wav_data.extend_from_slice(&pcm_data);
    std::fs::write(&output_path, &wav_data)?;
    println!("WAV saved to: {}", output_path);

    // --- ASR Pipeline (--asr flag) ---
    let run_asr = args.iter().any(|a| a == "--asr");
    let send_osc = args.iter().any(|a| a == "--osc");

    if run_asr {
        let config = AppConfig::load().unwrap_or_default();

        if config.tencent_app_id.is_empty() {
            eprintln!("ERROR: No Tencent credentials configured");
            std::process::exit(1);
        }

        let recognizer = StreamingRecognizer::new(
            config.tencent_app_id.clone(),
            config.tencent_secret_id.clone(),
            config.tencent_secret_key.clone(),
            true,
        );

        let pcm_len = pcm_data.len();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let recognized_text = match rt.block_on(async {
            recognizer.recognize_pcm(pcm_data, sample_rate).await
        }) {
            Ok(text) => text,
            Err(e) => {
                let result = serde_json::json!({
                    "status": "error",
                    "message": format!("{}", e),
                    "pcm_bytes": pcm_len,
                });
                println!("{}", serde_json::to_string(&result).unwrap());
                std::process::exit(1);
            }
        };

        if send_osc {
            let osc = OscSender::new(config.osc_host.clone(), config.osc_port);
            let _ = osc.send_typing(true);
            let _ = osc.send_chatbox(&recognized_text);
            let _ = osc.send_typing(false);
        }

        let result = serde_json::json!({
            "status": "ok",
            "text": recognized_text,
            "pcm_bytes": pcm_len,
            "sample_rate": sample_rate,
            "duration_secs": duration_secs,
            "osc_sent": send_osc,
        });
        println!("{}", serde_json::to_string(&result).unwrap());

        return Ok(());
    }

    // Quick validation: check for non-silent audio
    let has_signal = pcm_data.chunks(2).any(|pair| {
        let sample = i16::from_le_bytes([pair[0], pair[1]]);
        sample.abs() > 100 // signal above noise floor
    });
    
    if has_signal {
        println!("PASS: Audio signal detected in capture");
        Ok(())
    } else {
        eprintln!("FAIL: Captured audio appears to be silent");
        std::process::exit(1);
    }
}

fn find_supported_config(device: &cpal::Device) -> Result<cpal::StreamConfig, Box<dyn std::error::Error>> {
    let target_rate = 16000u32;
    let supported = device.supported_input_configs()?;
    
    for range in supported {
        let min = range.min_sample_rate().0;
        let max = range.max_sample_rate().0;
        if target_rate >= min && target_rate <= max {
            return Ok(cpal::StreamConfig {
                sample_rate: cpal::SampleRate(target_rate),
                channels: range.channels().min(2),
                buffer_size: cpal::BufferSize::Default,
            });
        }
    }
    
    // Fallback to first supported config
    if let Some(first) = device.supported_input_configs()?.next() {
        let rate = first.max_sample_rate().0;
        return Ok(cpal::StreamConfig {
            sample_rate: cpal::SampleRate(rate),
            channels: first.channels().min(2),
            buffer_size: cpal::BufferSize::Default,
        });
    }
    
    Err("No supported config found".into())
}

fn create_wav_header(data_len: u32, sample_rate: u32, bits_per_sample: u16, channels: u16) -> Vec<u8> {
    let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
    let block_align = channels * bits_per_sample / 8;
    let file_size = 36 + data_len;
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&file_size.to_le_bytes());
    header.extend_from_slice(b"WAVE");
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&channels.to_le_bytes());
    header.extend_from_slice(&sample_rate.to_le_bytes());
    header.extend_from_slice(&byte_rate.to_le_bytes());
    header.extend_from_slice(&block_align.to_le_bytes());
    header.extend_from_slice(&bits_per_sample.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_len.to_le_bytes());
    header
}
