//! Decoded audio: interleaved `f32` samples in `-1.0..=1.0`.

/// Decoded audio.
#[derive(Clone, Debug, PartialEq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved, `frames() * channels` values.
    pub samples: Vec<f32>,
}

/// Numbers describing a clip (used by tests and the UI).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub duration_secs: f64,
    pub peak: f32,
    pub rms: f32,
    /// Samples whose magnitude is at or above full scale (0.999).
    pub clipped: usize,
    /// All samples are (nearly) zero.
    pub silent: bool,
}

impl Pcm {
    pub fn new(sample_rate: u32, channels: u16, samples: Vec<f32>) -> Pcm {
        Pcm {
            sample_rate,
            channels: channels.max(1),
            samples,
        }
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames() as f64 / self.sample_rate as f64
        }
    }

    pub fn stats(&self) -> Stats {
        let mut peak = 0f32;
        let mut sum = 0f64;
        let mut clipped = 0;
        for &s in &self.samples {
            let a = s.abs();
            peak = peak.max(a);
            sum += (s as f64) * (s as f64);
            if a >= 0.999 {
                clipped += 1;
            }
        }
        let rms = if self.samples.is_empty() {
            0.0
        } else {
            (sum / self.samples.len() as f64).sqrt() as f32
        };
        Stats {
            duration_secs: self.duration_secs(),
            peak,
            rms,
            clipped,
            silent: peak < 1e-4,
        }
    }

    /// Mono mix (average of the channels).
    pub fn to_mono(&self) -> Vec<f32> {
        let ch = self.channels.max(1) as usize;
        if ch == 1 {
            return self.samples.clone();
        }
        self.samples
            .chunks_exact(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect()
    }

    /// `n` (min, max) pairs over the mono mix, for a waveform thumbnail.
    pub fn waveform(&self, n: usize) -> Vec<(f32, f32)> {
        let mono = self.to_mono();
        if n == 0 || mono.is_empty() {
            return vec![];
        }
        (0..n)
            .map(|i| {
                let a = i * mono.len() / n;
                let b = (((i + 1) * mono.len()) / n).max(a + 1).min(mono.len());
                let w = &mono[a.min(mono.len() - 1)..b];
                (
                    w.iter().cloned().fold(f32::MAX, f32::min),
                    w.iter().cloned().fold(f32::MIN, f32::max),
                )
            })
            .collect()
    }

    /// A canonical 16-bit PCM WAV file (what the tests write to inspect a decode).
    pub fn to_wav_bytes(&self) -> Vec<u8> {
        let ch = self.channels.max(1);
        let data_len = (self.samples.len() * 2) as u32;
        let mut v = Vec::with_capacity(44 + data_len as usize);
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&ch.to_le_bytes());
        v.extend_from_slice(&self.sample_rate.to_le_bytes());
        v.extend_from_slice(&(self.sample_rate * ch as u32 * 2).to_le_bytes());
        v.extend_from_slice(&(ch * 2).to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        for &s in &self.samples {
            v.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
        v
    }
}
