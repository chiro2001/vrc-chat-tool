//! Standalone Voice Activity Detection (VAD) filter.
//!
//! Energy-based VAD with configurable silence/speech thresholds,
//! extracted from hybrid.rs for reuse across all ASR backends.
//! Zero model dependency — pure signal processing.

/// Compute RMS energy of a float32 sample buffer.
pub fn rms_energy(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Compute RMS energy from i16 PCM bytes.
pub fn rms_energy_i16(pcm: &[u8]) -> f64 {
    if pcm.len() < 2 {
        return 0.0;
    }
    let count = pcm.len() / 2;
    let sum_sq: f64 = pcm
        .chunks_exact(2)
        .map(|pair| {
            let sample = i16::from_le_bytes([pair[0], pair[1]]) as f64;
            sample * sample
        })
        .sum();
    (sum_sq / count as f64).sqrt()
}

/// Decision from a VAD processing step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadDecision {
    /// Audio chunk contains speech — should be forwarded.
    Speech,
    /// Audio chunk is silence — should be dropped.
    Silence,
}

/// Energy-based VAD filter with hysteresis.
///
/// Uses configurable energy threshold and minimum silence duration
/// to avoid cutting off speech too early (hangover).
pub struct VadFilter {
    /// Energy threshold (RMS normalized to [-1,1] range).
    /// Typical: 0.005 for sensitive, 0.01 for conservative.
    energy_threshold: f64,
    /// Minimum consecutive silence samples before deciding "silence".
    /// Provides hangover to avoid chopping words.
    min_silence_samples: usize,
    /// Current consecutive silence sample count.
    silence_samples: usize,
    /// Whether speech has been detected in the current utterance.
    had_speech: bool,
}

impl VadFilter {
    /// Create a new VAD filter with default thresholds.
    ///
    /// - `energy_threshold`: RMS energy below which audio is considered silence.
    /// - `min_silence_ms`: minimum silence duration (ms) before switching to Silence state.
    /// - `sample_rate`: audio sample rate (Hz), used to convert ms to samples.
    pub fn new(energy_threshold: f64, min_silence_ms: u64, sample_rate: i32) -> Self {
        let min_silence_samples =
            (min_silence_ms as usize * sample_rate as usize) / 1000;
        Self {
            energy_threshold,
            min_silence_samples,
            silence_samples: 0,
            had_speech: false,
        }
    }

    /// Create with default settings (0.005 threshold, 300ms hangover, 16000 Hz).
    pub fn default_16000() -> Self {
        Self::new(0.005, 300, 16000)
    }

    /// Process an audio chunk (i16 PCM bytes) and return whether it contains speech.
    ///
    /// Maintains internal state for hangover — silence is only reported after
    /// `min_silence_samples` consecutive silent samples have been observed
    /// following speech.
    pub fn process_i16(&mut self, pcm: &[u8]) -> VadDecision {
        let energy = rms_energy_i16(pcm);
        let normalized = energy / 32767.0;
        let has_speech = normalized >= self.energy_threshold;

        if has_speech {
            self.had_speech = true;
            self.silence_samples = 0;
            VadDecision::Speech
        } else {
            let chunk_samples = pcm.len() / 2;
            self.silence_samples += chunk_samples;

            // Hangover: keep reporting Speech until enough silence accumulates
            if self.had_speech && self.silence_samples < self.min_silence_samples {
                VadDecision::Speech
            } else {
                // Enough silence — reset utterance state
                self.had_speech = false;
                self.silence_samples = 0;
                VadDecision::Silence
            }
        }
    }

    /// Reset internal state for a new utterance/session.
    pub fn reset(&mut self) {
        self.silence_samples = 0;
        self.had_speech = false;
    }

    /// Check if the filter is currently in a speech segment.
    pub fn is_speech(&self) -> bool {
        self.had_speech
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rms_energy_silence() {
        let silence = vec![0.0f32; 1600];
        assert!(rms_energy(&silence) < 0.0001);
    }

    #[test]
    fn test_rms_energy_speech() {
        let speech: Vec<f32> = (0..1600).map(|i| (i as f32 / 1600.0) * 0.1).collect();
        let energy = rms_energy(&speech);
        assert!(energy > 0.0);
    }

    #[test]
    fn test_rms_energy_i16_silence() {
        let silence_i16: Vec<u8> = vec![0u8; 3200]; // 1600 samples × 2 bytes
        assert!(rms_energy_i16(&silence_i16) < 0.0001);
    }

    #[test]
    fn test_vad_filter_speech_then_silence() {
        let mut vad = VadFilter::new(0.005, 300, 16000);

        // Simulate 200ms speech (3200 samples × 2 bytes = 6400 bytes)
        let speech: Vec<u8> = (0..6400).map(|i| ((i as f32 * 0.1).sin() * 16000.0) as i16 as u8).collect();
        // Actually this creates garbled data. Let's use actual i16 values.
        let speech_i16: Vec<i16> = (0..3200).map(|i| ((i as f64 * 0.1).sin() * 16000.0) as i16).collect();
        let speech_bytes: Vec<u8> = speech_i16.iter().flat_map(|s| s.to_le_bytes()).collect();

        let result = vad.process_i16(&speech_bytes);
        assert_eq!(result, VadDecision::Speech);
        assert!(vad.is_speech());
    }

    #[test]
    fn test_vad_filter_silence() {
        let mut vad = VadFilter::new(0.005, 300, 16000);
        let silence = vec![0u8; 6400]; // ~200ms silence
        let result = vad.process_i16(&silence);
        assert_eq!(result, VadDecision::Silence);
    }

    #[test]
    fn test_vad_filter_hangover() {
        let mut vad = VadFilter::new(0.005, 300, 16000);

        // Speech chunk
        let speech_i16: Vec<i16> = (0..1600).map(|i| ((i as f64 * 0.1).sin() * 16000.0) as i16).collect();
        let speech_bytes: Vec<u8> = speech_i16.iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(vad.process_i16(&speech_bytes), VadDecision::Speech);

        // Short silence (100ms < 300ms hangover) — should still be Speech
        let short_silence = vec![0u8; 3200]; // 100ms
        assert_eq!(vad.process_i16(&short_silence), VadDecision::Speech);

        // Long silence (exceeds hangover) — should become Silence
        let long_silence = vec![0u8; 12800]; // 400ms
        assert_eq!(vad.process_i16(&long_silence), VadDecision::Silence);
    }

    #[test]
    fn test_vad_filter_reset() {
        let mut vad = VadFilter::new(0.005, 300, 16000);
        // Generate speech
        let speech_i16: Vec<i16> = (0..1600).map(|i| ((i as f64 * 0.1).sin() * 16000.0) as i16).collect();
        let speech_bytes: Vec<u8> = speech_i16.iter().flat_map(|s| s.to_le_bytes()).collect();
        vad.process_i16(&speech_bytes);
        assert!(vad.is_speech());

        vad.reset();
        assert!(!vad.is_speech());
    }
}
