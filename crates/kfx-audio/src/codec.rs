//! Audio containers and codecs: the game's own WAV reader (`src/bflib_sndlib.cpp:211-300`,
//! class `wave_file`) and the external formats of custom sounds (`decode_audio_buffer_and_store`,
//! `src/bflib_sndlib.cpp:1376-1470`).
//!
//! What the game does, and what this module does differently:
//!
//! * The game accepts WAVE format tags 1 (PCM) and 2 (MS ADPCM), 1 or 2 channels, 4/8/16 bits
//!   (`bflib_sndlib.cpp:258-282`). MS ADPCM is approximated, not decoded (`(b >> 4) * 2`, `(b &
//!   7) * 2` as 8-bit samples, `bflib_sndlib.cpp:324-331`, written for `heart6a.wav`); stereo ADPCM
//!   throws. This crate decodes MS ADPCM properly instead, so the preview sounds like the sound was
//!   meant to, not like what the engine plays.
//! * Chunks other than `fmt ` and `data` (`fact`, `LIST`, `id3 `) are skipped; trailing bytes after
//!   the RIFF are ignored. Sizes larger than the file are clamped (the game would read garbage).
//! * External files are sniffed by content, not by extension: `BMU V1.0` + MP3, MP3 (ID3 tag or
//!   frame sync), RIFF/WAVE, everything else goes to the generic decoder (OGG Vorbis, FLAC).
use crate::error::{Error, Result};
use crate::pcm::Pcm;
use std::io::Cursor;

/// Codec of one clip, for reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Codec {
    Pcm8,
    Pcm16,
    Pcm24,
    Pcm32,
    Float,
    MsAdpcm,
    /// Other WAVE format tag.
    WavOther(u16),
    Mp3,
    Vorbis,
    Flac,
    Unknown,
}

impl Codec {
    pub fn name(self) -> String {
        match self {
            Codec::Pcm8 => "PCM 8-bit".into(),
            Codec::Pcm16 => "PCM 16-bit".into(),
            Codec::Pcm24 => "PCM 24-bit".into(),
            Codec::Pcm32 => "PCM 32-bit".into(),
            Codec::Float => "PCM float".into(),
            Codec::MsAdpcm => "MS ADPCM".into(),
            Codec::WavOther(t) => format!("WAVE tag {t:#06x}"),
            Codec::Mp3 => "MP3".into(),
            Codec::Vorbis => "Ogg Vorbis".into(),
            Codec::Flac => "FLAC".into(),
            Codec::Unknown => "unknown".into(),
        }
    }
}

/// Header of a WAVE file's `fmt ` chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WavFormat {
    pub tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub avg_bytes: u32,
    pub block_align: u16,
    pub bits: u16,
    /// Bytes of the `fmt ` chunk after the 16 base bytes (`cbSize` and what it announces).
    pub extra: Vec<u8>,
}

impl WavFormat {
    /// The effective format tag (`WAVE_FORMAT_EXTENSIBLE` is resolved to its sub format).
    pub fn effective_tag(&self) -> u16 {
        if self.tag == 0xFFFE && self.extra.len() >= 10 {
            u16::from_le_bytes([self.extra[8], self.extra[9]])
        } else {
            self.tag
        }
    }

    pub fn codec(&self) -> Codec {
        match (self.effective_tag(), self.bits) {
            (1, 8) => Codec::Pcm8,
            (1, 16) => Codec::Pcm16,
            (1, 24) => Codec::Pcm24,
            (1, 32) => Codec::Pcm32,
            (3, _) => Codec::Float,
            (2, _) => Codec::MsAdpcm,
            (t, _) => Codec::WavOther(t),
        }
    }
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Walk the chunks of a RIFF/WAVE file; returns the format and the (clamped) data chunk.
pub fn parse_wav(bytes: &[u8]) -> Result<(WavFormat, &[u8])> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(Error::format("wav", "not a RIFF/WAVE file"));
    }
    let mut pos = 12usize;
    let mut fmt: Option<WavFormat> = None;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() && !(fmt.is_some() && data.is_some()) {
        let id = &bytes[pos..pos + 4];
        let size = u32le(bytes, pos + 4) as usize;
        let body_start = pos + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " => {
                if body_end - body_start < 16 {
                    return Err(Error::format("wav", "fmt chunk smaller than WAVEFORMAT"));
                }
                let b = &bytes[body_start..body_end];
                fmt = Some(WavFormat {
                    tag: u16le(b, 0),
                    channels: u16le(b, 2),
                    sample_rate: u32le(b, 4),
                    avg_bytes: u32le(b, 8),
                    block_align: u16le(b, 12),
                    bits: u16le(b, 14),
                    extra: b[16..].to_vec(),
                });
            }
            b"data" => data = Some(&bytes[body_start..body_end]),
            _ => {}
        }
        pos = body_start.saturating_add(size).saturating_add(size & 1);
    }
    match (fmt, data) {
        (Some(f), Some(d)) => Ok((f, d)),
        (None, _) => Err(Error::format("wav", "no fmt chunk")),
        (_, None) => Err(Error::format("wav", "no data chunk")),
    }
}

const ADAPT: [i32; 16] = [
    230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
];
const STD_COEF: [(i16, i16); 7] = [
    (256, 0),
    (512, -256),
    (0, 0),
    (192, 64),
    (240, 0),
    (460, -208),
    (392, -232),
];

struct AdpcmCh {
    c1: i32,
    c2: i32,
    delta: i32,
    s1: i32,
    s2: i32,
}

impl AdpcmCh {
    fn nibble(&mut self, n: u8) -> i32 {
        // C division, truncates toward zero like the Microsoft codec
        let pred = (self.s1 * self.c1 + self.s2 * self.c2) / 256;
        let signed = ((n ^ 8) as i32) - 8;
        let v = (pred + signed * self.delta).clamp(-32768, 32767);
        self.s2 = self.s1;
        self.s1 = v;
        self.delta = ((ADAPT[n as usize] * self.delta) >> 8).max(16);
        v
    }
}

/// Microsoft ADPCM (`WAVE_FORMAT_ADPCM`, tag 2), mono or stereo.
pub fn decode_ms_adpcm(fmt: &WavFormat, data: &[u8]) -> Result<Pcm> {
    let ch = fmt.channels as usize;
    if !(1..=2).contains(&ch) {
        return Err(Error::Unsupported(format!("MS ADPCM with {ch} channels")));
    }
    let block = fmt.block_align as usize;
    if block < 7 * ch {
        return Err(Error::format(
            "adpcm",
            format!("block align {block} too small"),
        ));
    }
    let mut coefs: Vec<(i16, i16)> = vec![];
    if fmt.extra.len() >= 6 {
        let n = u16le(&fmt.extra, 4) as usize;
        for i in 0..n {
            let o = 6 + 4 * i;
            if o + 4 > fmt.extra.len() {
                break;
            }
            coefs.push((u16le(&fmt.extra, o) as i16, u16le(&fmt.extra, o + 2) as i16));
        }
    }
    if coefs.is_empty() {
        coefs = STD_COEF.to_vec();
    }
    let mut out: Vec<f32> = Vec::with_capacity(data.len() * 2);
    for blk in data.chunks(block) {
        let hdr = 7 * ch;
        if blk.len() < hdr {
            break;
        }
        let mut st: Vec<AdpcmCh> = Vec::with_capacity(ch);
        let mut o = 0;
        let mut idx = vec![];
        for _ in 0..ch {
            idx.push(blk[o] as usize);
            o += 1;
        }
        let mut deltas = vec![];
        for _ in 0..ch {
            deltas.push(u16le(blk, o) as i16 as i32);
            o += 2;
        }
        let mut s1 = vec![];
        for _ in 0..ch {
            s1.push(u16le(blk, o) as i16 as i32);
            o += 2;
        }
        let mut s2 = vec![];
        for _ in 0..ch {
            s2.push(u16le(blk, o) as i16 as i32);
            o += 2;
        }
        for c in 0..ch {
            let (c1, c2) = coefs[idx[c].min(coefs.len() - 1)];
            st.push(AdpcmCh {
                c1: c1 as i32,
                c2: c2 as i32,
                delta: deltas[c],
                s1: s1[c],
                s2: s2[c],
            });
        }
        // the two header samples come first, oldest (sample 2) first
        for c in 0..ch {
            out.push(st[c].s2 as f32 / 32768.0);
        }
        for c in 0..ch {
            out.push(st[c].s1 as f32 / 32768.0);
        }
        for &b in &blk[hdr..] {
            let (hi, lo) = (b >> 4, b & 15);
            if ch == 1 {
                out.push(st[0].nibble(hi) as f32 / 32768.0);
                out.push(st[0].nibble(lo) as f32 / 32768.0);
            } else {
                out.push(st[0].nibble(hi) as f32 / 32768.0);
                out.push(st[1].nibble(lo) as f32 / 32768.0);
            }
        }
    }
    Ok(Pcm::new(fmt.sample_rate, fmt.channels, out))
}

/// Decode a WAVE file: PCM (8/16/24/32 bit), float and MS ADPCM natively.
pub fn decode_wav(bytes: &[u8]) -> Result<(Pcm, Codec)> {
    let (fmt, data) = parse_wav(bytes)?;
    let codec = fmt.codec();
    let ch = fmt.channels.max(1) as usize;
    let tag = fmt.effective_tag();
    let samples: Vec<f32> = match (tag, fmt.bits) {
        (1, 8) => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
        (1, 16) => data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect(),
        (1, 24) => data
            .chunks_exact(3)
            .map(|c| {
                (((c[0] as i32) << 8 | (c[1] as i32) << 16 | (c[2] as i32) << 24) >> 8) as f32
                    / 8388608.0
            })
            .collect(),
        (1, 32) => data
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2147483648.0)
            .collect(),
        (3, 32) => data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        (3, 64) => data
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32)
            .collect(),
        (2, _) => return Ok((decode_ms_adpcm(&fmt, data)?, codec)),
        _ => {
            return Err(Error::Unsupported(format!(
                "WAVE tag {tag:#06x}, {} bits",
                fmt.bits
            )))
        }
    };
    let whole = samples.len() / ch * ch;
    let mut samples = samples;
    samples.truncate(whole);
    Ok((Pcm::new(fmt.sample_rate, fmt.channels, samples), codec))
}

/// Header-only description of a WAVE clip (no decode): codec, channels, rate, frame count.
pub fn wav_info(bytes: &[u8]) -> Result<(Codec, u16, u32, usize)> {
    let (fmt, data) = parse_wav(bytes)?;
    let ch = fmt.channels.max(1) as usize;
    let frames = match fmt.effective_tag() {
        2 => {
            let spb = if fmt.extra.len() >= 4 {
                u16le(&fmt.extra, 2) as usize
            } else {
                0
            };
            let spb = if spb > 0 {
                spb
            } else {
                (fmt.block_align as usize).saturating_sub(7 * ch) * 2 / ch + 2
            };
            let full = data.len() / fmt.block_align.max(1) as usize;
            let rest = data.len() % fmt.block_align.max(1) as usize;
            let tail = if rest > 7 * ch {
                (rest - 7 * ch) * 2 / ch + 2
            } else {
                0
            };
            full * spb + tail
        }
        _ => {
            let bytes_per_frame = (fmt.bits as usize).div_ceil(8) * ch;
            if bytes_per_frame == 0 {
                0
            } else {
                data.len() / bytes_per_frame
            }
        }
    };
    Ok((fmt.codec(), fmt.channels, fmt.sample_rate, frames))
}

/// What an audio buffer looks like from its first bytes.
pub fn sniff(bytes: &[u8]) -> Codec {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return parse_wav(bytes)
            .map(|(f, _)| f.codec())
            .unwrap_or(Codec::Unknown);
    }
    if bytes.starts_with(b"BMU V1.0") {
        return Codec::Mp3;
    }
    if bytes.starts_with(b"OggS") {
        return Codec::Vorbis;
    }
    if bytes.starts_with(b"fLaC") {
        return Codec::Flac;
    }
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
    {
        return Codec::Mp3;
    }
    Codec::Unknown
}

/// Decode with the generic decoder (MP3, Ogg Vorbis, FLAC and unusual WAVE).
pub fn decode_generic(bytes: &[u8], ext_hint: Option<&str>) -> Result<Pcm> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error as SymErr;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let bad = |e: SymErr| Error::format("audio", e.to_string());
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = ext_hint {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(bad)?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| Error::format("audio", "no audio track"))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(bad)?;
    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(0);
    let mut samples: Vec<f32> = vec![];
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymErr::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymErr::ResetRequired) => break,
            Err(e) => {
                if samples.is_empty() {
                    return Err(bad(e));
                }
                break;
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                rate = spec.rate;
                channels = spec.channels.count() as u16;
                let sb = buf.get_or_insert_with(|| {
                    SampleBuffer::<f32>::new(decoded.capacity() as u64, spec)
                });
                sb.copy_interleaved_ref(decoded);
                samples.extend_from_slice(sb.samples());
            }
            Err(SymErr::DecodeError(_)) => continue,
            Err(e) => {
                if samples.is_empty() {
                    return Err(bad(e));
                }
                break;
            }
        }
    }
    if samples.is_empty() {
        return Err(Error::format("audio", "no samples decoded"));
    }
    Ok(Pcm::new(rate, channels, samples))
}

/// Decode a custom sound file the way the game routes it: `BMU V1.0` wrapper, MP3, WAVE, then
/// anything else. `name` only supplies an extension hint.
pub fn decode_audio(bytes: &[u8], name: &str) -> Result<(Pcm, Codec)> {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if bytes.len() > 8 && bytes.starts_with(b"BMU V1.0") {
        return Ok((decode_generic(&bytes[8..], Some("mp3"))?, Codec::Mp3));
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        match decode_wav(bytes) {
            Ok(r) => return Ok(r),
            Err(Error::Unsupported(_)) => {
                let codec = sniff(bytes);
                return Ok((decode_generic(bytes, Some("wav"))?, codec));
            }
            Err(e) => return Err(e),
        }
    }
    let codec = sniff(bytes);
    let hint = match codec {
        Codec::Mp3 => Some("mp3".to_string()),
        Codec::Vorbis => Some("ogg".to_string()),
        Codec::Flac => Some("flac".to_string()),
        _ => ext,
    };
    Ok((decode_generic(bytes, hint.as_deref())?, codec))
}
