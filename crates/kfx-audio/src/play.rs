//! Preview playback: a software [`Mixer`] behind the [`AudioBackend`] trait.
//!
//! * [`NullBackend`]: renders into nothing (no device is opened). It advances the mixer by wall
//!   clock time on [`AudioBackend::poll`] (or by an explicit [`NullBackend::advance`] in tests), so
//!   voices end, the peak meter moves and the UI behaves exactly as with a real device.
//! * `CpalBackend` (feature `playback`): the same mixer driven by the default output device.
//!   Nothing in this crate starts it on its own; the UI only creates it when the user presses a
//!   play button for the first time.
//!
//! The mixer works at the output rate with linear interpolation, plays any number of voices up to
//! a limit (the oldest voice is stopped when a new one needs the slot), supports volume, pan and
//! looping, and keeps a peak meter plus the last samples as a scope.
#[cfg(feature = "playback")]
use crate::error::Error;
use crate::error::Result;
use crate::pcm::Pcm;
use std::collections::VecDeque;
use std::sync::Arc;
#[cfg(feature = "playback")]
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VoiceId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayOptions {
    /// Linear gain, 1.0 = unchanged.
    pub volume: f32,
    /// -1.0 (left) ..= 1.0 (right).
    pub pan: f32,
    pub looping: bool,
}

impl Default for PlayOptions {
    fn default() -> Self {
        PlayOptions {
            volume: 1.0,
            pan: 0.0,
            looping: false,
        }
    }
}

/// What the UI shows while sound plays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meter {
    /// Recent peak per output channel (decays).
    pub peak: [f32; 2],
    /// RMS of the last block.
    pub rms: f32,
    /// The last 512 mono output samples.
    pub scope: Vec<f32>,
}

pub trait AudioBackend {
    fn name(&self) -> &str;
    /// Start a voice. When the voice limit is reached the oldest voice is stopped.
    fn play(&mut self, pcm: Arc<Pcm>, opts: PlayOptions) -> Result<VoiceId>;
    fn stop(&mut self, id: VoiceId);
    fn stop_all(&mut self);
    fn is_playing(&self, id: VoiceId) -> bool;
    fn active_voices(&self) -> usize;
    fn set_master_volume(&mut self, volume: f32);
    fn master_volume(&self) -> f32;
    fn set_max_voices(&mut self, n: usize);
    fn meter(&self) -> Meter;
    /// Housekeeping to call every UI frame (the null backend advances its clock here).
    fn poll(&mut self) {}
    /// Playback position of a voice in seconds of the source (None when it ended).
    fn position_secs(&self, id: VoiceId) -> Option<f64>;
}

struct Voice {
    id: VoiceId,
    pcm: Arc<Pcm>,
    /// Position in source frames.
    pos: f64,
    opts: PlayOptions,
}

/// Device independent mixer.
pub struct Mixer {
    voices: Vec<Voice>,
    next_id: u64,
    pub max_voices: usize,
    pub master: f32,
    peak: [f32; 2],
    rms: f32,
    scope: VecDeque<f32>,
}

const SCOPE_LEN: usize = 512;

impl Default for Mixer {
    fn default() -> Self {
        Mixer {
            voices: vec![],
            next_id: 1,
            max_voices: 16,
            master: 1.0,
            peak: [0.0; 2],
            rms: 0.0,
            scope: VecDeque::new(),
        }
    }
}

impl Mixer {
    pub fn play(&mut self, pcm: Arc<Pcm>, opts: PlayOptions) -> VoiceId {
        let id = VoiceId(self.next_id);
        self.next_id += 1;
        while self.voices.len() >= self.max_voices.max(1) {
            self.voices.remove(0);
        }
        if pcm.frames() > 0 {
            self.voices.push(Voice {
                id,
                pcm,
                pos: 0.0,
                opts,
            });
        }
        id
    }

    pub fn stop(&mut self, id: VoiceId) {
        self.voices.retain(|v| v.id != id);
    }

    pub fn stop_all(&mut self) {
        self.voices.clear();
    }

    pub fn is_playing(&self, id: VoiceId) -> bool {
        self.voices.iter().any(|v| v.id == id)
    }

    pub fn active(&self) -> usize {
        self.voices.len()
    }

    pub fn position_secs(&self, id: VoiceId) -> Option<f64> {
        self.voices
            .iter()
            .find(|v| v.id == id)
            .map(|v| v.pos / v.pcm.sample_rate.max(1) as f64)
    }

    pub fn meter(&self) -> Meter {
        Meter {
            peak: self.peak,
            rms: self.rms,
            scope: self.scope.iter().cloned().collect(),
        }
    }

    /// Mix `out.len() / 2` stereo frames at `rate` Hz into `out` (interleaved L R), overwriting it.
    pub fn render(&mut self, out: &mut [f32], rate: u32) {
        out.fill(0.0);
        let frames = out.len() / 2;
        for v in &mut self.voices {
            let ch = v.pcm.channels.max(1) as usize;
            let n = v.pcm.frames();
            let step = v.pcm.sample_rate.max(1) as f64 / rate.max(1) as f64;
            let (pl, pr) = {
                let p = v.opts.pan.clamp(-1.0, 1.0);
                (
                    ((1.0 - p) * 0.5 + 0.5f32).min(1.0),
                    ((1.0 + p) * 0.5 + 0.5f32).min(1.0),
                )
            };
            let gain = v.opts.volume;
            for f in 0..frames {
                if v.pos >= n as f64 {
                    if v.opts.looping {
                        v.pos -= n as f64;
                    } else {
                        break;
                    }
                }
                let i0 = v.pos as usize;
                let frac = (v.pos - i0 as f64) as f32;
                let i1 = if i0 + 1 < n {
                    i0 + 1
                } else if v.opts.looping {
                    0
                } else {
                    i0
                };
                let s = |i: usize, c: usize| v.pcm.samples[i * ch + c.min(ch - 1)];
                let l = s(i0, 0) * (1.0 - frac) + s(i1, 0) * frac;
                let r = if ch > 1 {
                    s(i0, 1) * (1.0 - frac) + s(i1, 1) * frac
                } else {
                    l
                };
                out[f * 2] += l * gain * pl;
                out[f * 2 + 1] += r * gain * pr;
                v.pos += step;
            }
        }
        self.voices
            .retain(|v| v.opts.looping || v.pos < v.pcm.frames() as f64);
        let mut sq = 0f64;
        let mut pk = [0f32; 2];
        for (i, s) in out.iter_mut().enumerate() {
            *s = (*s * self.master).clamp(-1.0, 1.0);
            pk[i & 1] = pk[i & 1].max(s.abs());
            sq += (*s as f64) * (*s as f64);
        }
        for c in 0..2 {
            // decay about 12 dB per 100 ms of render time worth of frames
            let decay = (-(frames as f32) / rate.max(1) as f32 * 20.0).exp();
            self.peak[c] = (self.peak[c] * decay).max(pk[c]);
        }
        self.rms = if out.is_empty() {
            0.0
        } else {
            (sq / out.len() as f64).sqrt() as f32
        };
        for f in 0..frames {
            self.scope.push_back((out[f * 2] + out[f * 2 + 1]) * 0.5);
        }
        while self.scope.len() > SCOPE_LEN {
            self.scope.pop_front();
        }
    }
}

/// A backend that opens no device. See the module docs.
pub struct NullBackend {
    mixer: Mixer,
    rate: u32,
    last_poll: Option<std::time::Instant>,
    /// Render by wall clock in [`AudioBackend::poll`].
    pub realtime: bool,
}

impl NullBackend {
    pub fn new() -> NullBackend {
        NullBackend {
            mixer: Mixer::default(),
            rate: 44100,
            last_poll: None,
            realtime: true,
        }
    }

    /// Advance the virtual device by `secs` (renders and discards the audio).
    pub fn advance(&mut self, secs: f64) {
        let mut left = (secs * self.rate as f64).round() as usize;
        let mut buf = vec![0f32; 2 * 1024];
        while left > 0 {
            let n = left.min(1024);
            self.mixer.render(&mut buf[..n * 2], self.rate);
            left -= n;
        }
    }
}

impl Default for NullBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for NullBackend {
    fn name(&self) -> &str {
        "null (no audio device)"
    }
    fn play(&mut self, pcm: Arc<Pcm>, opts: PlayOptions) -> Result<VoiceId> {
        Ok(self.mixer.play(pcm, opts))
    }
    fn stop(&mut self, id: VoiceId) {
        self.mixer.stop(id)
    }
    fn stop_all(&mut self) {
        self.mixer.stop_all()
    }
    fn is_playing(&self, id: VoiceId) -> bool {
        self.mixer.is_playing(id)
    }
    fn active_voices(&self) -> usize {
        self.mixer.active()
    }
    fn set_master_volume(&mut self, volume: f32) {
        self.mixer.master = volume.clamp(0.0, 4.0);
    }
    fn master_volume(&self) -> f32 {
        self.mixer.master
    }
    fn set_max_voices(&mut self, n: usize) {
        self.mixer.max_voices = n.max(1);
    }
    fn meter(&self) -> Meter {
        self.mixer.meter()
    }
    fn poll(&mut self) {
        let now = std::time::Instant::now();
        if self.realtime {
            if let Some(t) = self.last_poll {
                let dt = now.duration_since(t).as_secs_f64().min(0.25);
                if self.mixer.active() > 0 || dt > 0.0 {
                    self.advance(dt);
                }
            }
        }
        self.last_poll = Some(now);
    }
    fn position_secs(&self, id: VoiceId) -> Option<f64> {
        self.mixer.position_secs(id)
    }
}

#[cfg(feature = "playback")]
pub use cpal_backend::CpalBackend;

#[cfg(feature = "playback")]
mod cpal_backend {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    /// Real output through the default cpal device.
    pub struct CpalBackend {
        mixer: Arc<Mutex<Mixer>>,
        _stream: cpal::Stream,
        device_name: String,
    }

    impl CpalBackend {
        /// Open the default output device and start the (silent until a voice plays) stream.
        pub fn new() -> Result<CpalBackend> {
            let host = cpal::default_host();
            let device = host
                .default_output_device()
                .ok_or_else(|| Error::Playback("no default output device".into()))?;
            let device_name = device.name().unwrap_or_else(|_| "unknown".into());
            let supported = device
                .default_output_config()
                .map_err(|e| Error::Playback(e.to_string()))?;
            let format = supported.sample_format();
            let config: cpal::StreamConfig = supported.into();
            let mixer = Arc::new(Mutex::new(Mixer::default()));
            let stream = match format {
                cpal::SampleFormat::F32 => Self::build::<f32>(&device, &config, mixer.clone())?,
                cpal::SampleFormat::I16 => Self::build::<i16>(&device, &config, mixer.clone())?,
                cpal::SampleFormat::U16 => Self::build::<u16>(&device, &config, mixer.clone())?,
                other => {
                    return Err(Error::Playback(format!(
                        "unsupported output sample format {other:?}"
                    )))
                }
            };
            stream.play().map_err(|e| Error::Playback(e.to_string()))?;
            Ok(CpalBackend {
                mixer,
                _stream: stream,
                device_name,
            })
        }

        fn build<T>(
            device: &cpal::Device,
            config: &cpal::StreamConfig,
            mixer: Arc<Mutex<Mixer>>,
        ) -> Result<cpal::Stream>
        where
            T: cpal::SizedSample + cpal::FromSample<f32>,
        {
            let channels = config.channels as usize;
            let rate = config.sample_rate.0;
            let mut scratch: Vec<f32> = vec![];
            device
                .build_output_stream(
                    config,
                    move |data: &mut [T], _| {
                        let frames = data.len() / channels.max(1);
                        scratch.resize(frames * 2, 0.0);
                        match mixer.try_lock() {
                            Ok(mut m) => m.render(&mut scratch, rate),
                            Err(_) => scratch.fill(0.0),
                        }
                        for f in 0..frames {
                            for c in 0..channels {
                                let s = if channels == 1 {
                                    (scratch[f * 2] + scratch[f * 2 + 1]) * 0.5
                                } else {
                                    scratch[f * 2 + (c & 1)]
                                };
                                data[f * channels + c] = T::from_sample(s);
                            }
                        }
                    },
                    |e| eprintln!("kfx-audio: output stream error: {e}"),
                    None,
                )
                .map_err(|e| Error::Playback(e.to_string()))
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, Mixer> {
            self.mixer.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    impl AudioBackend for CpalBackend {
        fn name(&self) -> &str {
            &self.device_name
        }
        fn play(&mut self, pcm: Arc<Pcm>, opts: PlayOptions) -> Result<VoiceId> {
            Ok(self.lock().play(pcm, opts))
        }
        fn stop(&mut self, id: VoiceId) {
            self.lock().stop(id)
        }
        fn stop_all(&mut self) {
            self.lock().stop_all()
        }
        fn is_playing(&self, id: VoiceId) -> bool {
            self.lock().is_playing(id)
        }
        fn active_voices(&self) -> usize {
            self.lock().active()
        }
        fn set_master_volume(&mut self, volume: f32) {
            self.lock().master = volume.clamp(0.0, 4.0);
        }
        fn master_volume(&self) -> f32 {
            self.lock().master
        }
        fn set_max_voices(&mut self, n: usize) {
            self.lock().max_voices = n.max(1);
        }
        fn meter(&self) -> Meter {
            self.lock().meter()
        }
        fn position_secs(&self, id: VoiceId) -> Option<f64> {
            self.lock().position_secs(id)
        }
    }
}

/// Caches decoded clips and drives a backend: what the browser uses.
pub struct Player {
    pub backend: Box<dyn AudioBackend>,
    cache: std::collections::HashMap<String, Arc<Pcm>>,
    /// Voices started by [`Player::play_clip`], newest last.
    pub current: Option<VoiceId>,
}

impl Player {
    pub fn new(backend: Box<dyn AudioBackend>) -> Player {
        Player {
            backend,
            cache: Default::default(),
            current: None,
        }
    }

    /// A player on the null backend.
    pub fn null() -> Player {
        Player::new(Box::new(NullBackend::new()))
    }

    /// Cached decode through `decode`.
    pub fn clip(&mut self, key: &str, decode: impl FnOnce() -> Result<Pcm>) -> Result<Arc<Pcm>> {
        if let Some(c) = self.cache.get(key) {
            return Ok(c.clone());
        }
        let pcm = Arc::new(decode()?);
        if self.cache.len() > 256 {
            self.cache.clear();
        }
        self.cache.insert(key.to_string(), pcm.clone());
        Ok(pcm)
    }

    /// Play a clip, replacing the previous preview voice.
    pub fn play_clip(&mut self, pcm: Arc<Pcm>, opts: PlayOptions) -> Result<VoiceId> {
        if let Some(v) = self.current.take() {
            self.backend.stop(v);
        }
        let id = self.backend.play(pcm, opts)?;
        self.current = Some(id);
        Ok(id)
    }

    pub fn stop(&mut self) {
        if let Some(v) = self.current.take() {
            self.backend.stop(v);
        }
    }

    pub fn is_playing(&self) -> bool {
        self.current.is_some_and(|v| self.backend.is_playing(v))
    }
}
