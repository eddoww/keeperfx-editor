//! The game's sound banks: `sound/sound.dat` (effects) and `sound/speech_<lang>.dat` (speech).
//!
//! # Container format
//!
//! Reference: `load_sound_bank` and the structs above it, `src/bflib_sndlib.cpp:386-450`
//! (the original Bullfrog "sound bank" container that `tools/sndbanker` builds; the game ships
//! the same `.dat` files as Dungeon Keeper 1). All integers little-endian, `#pragma pack(1)`.
//!
//! ```text
//! file:   [sample data ............................][sample directory][bank header][u32 header_offset]
//!                                                                                  ^ last 4 bytes
//! header (at header_offset):   u8 signature[14]; u32 version;          (18 bytes, ignored by the game)
//!                              BankEntry entries[9];                   (9 * 16 bytes)
//! BankEntry (16 bytes):        u32 first_sample_offset  absolute file offset of the directory
//!                              u32 first_data_offset    absolute file offset the sample data starts at
//!                              u32 total_samples_size   bytes of directory (count * 32)
//!                              u32 entries_count        (not used by the game; equals the size here)
//! ```
//!
//! The game reads only `entries[2]` (`directory_index = 2`, `bflib_sndlib.cpp:416`); the other
//! eight are `0xFFFFFFFF` in every bank of the install. The directory is `total_samples_size / 32`
//! records:
//!
//! ```text
//! SampleRecord (32 bytes):  char filename[18]   NUL padded, 18 chars max (long names are cut)
//!                           u32  data_offset    relative to entries[2].first_data_offset
//!                           u32  sample_rate    0 in the shipped banks (the WAV header is used)
//!                           u32  data_size      bytes of the embedded WAV file (may include 1-16
//!                                               padding / trailing bytes after the RIFF)
//!                           u8   sfx_id         0, or 255 on many effects (`get_sample_sfxid`, only
//!                                               copied into the mixer's sample struct)
//!                           u8   format_flags   0 in the shipped banks
//! ```
//!
//! Every sample's data is a complete RIFF/WAVE file (`bflib_sndlib.cpp:445`, parsed with the
//! class `wave_file`). Record 0 is a dummy (empty name, `data_size` = the whole data area): the
//! game never plays sample 0 (`smptbl_id <= 0` is rejected, `play_sample`), so ids start at 1.
//!
//! # Id space
//!
//! The mixer uses one "unified" sample id (`SoundSmplTblID`, a `short`): `1..effects` are the
//! `sound.dat` samples, then `speech_offset = effects.len()` (`g_speech_offset`,
//! `bflib_sndlib.cpp:519`) starts the speech bank, and `custom_offset = speech_offset +
//! speech.len()` starts the custom (file) bank that `sounds.cfg` fills at runtime. Speech messages
//! (`SMsg_*`, `src/gui_soundmsgs.h:47`) are plain indices into the speech bank
//! (`play_speech_sample`, `src/bflib_sound.c:843`: `unified = speech_offset + smptbl_id`).
//!
//! # Which speech file
//!
//! `speech_<lang>.dat` with the install language's three letter code (`eng`, `fre`, ...), else
//! `speech.dat`, else `speech_eng.dat` (`load_sound_banks`, `bflib_sndlib.cpp:502-514`).
//!
//! `sound/*.sbk` (`atmos1.sbk`, ...) and `sound/music.dat` (an XMIDI file, `FORM....XDIRINFO`) are
//! original-game leftovers that KeeperFX never opens (no reference in `src/`).
use crate::codec::{self, Codec};
use crate::error::{Error, Result};
use crate::pcm::Pcm;
use std::path::{Path, PathBuf};

/// Which bank a sample belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BankKind {
    Effects,
    Speech,
}

/// One directory record plus what the WAV header says.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleInfo {
    pub index: u16,
    /// File name stored in the directory (`foot1a.wav`); empty for the reserved record 0.
    pub name: String,
    /// Absolute offset of the embedded WAV in the bank file, and its size.
    pub offset: usize,
    pub size: usize,
    pub dir_sample_rate: u32,
    pub sfx_id: u8,
    pub format_flags: u8,
    pub codec: Codec,
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: usize,
    /// Record 0: never played by the game.
    pub reserved: bool,
    /// The WAV header could not be read (the game would refuse the whole bank).
    pub broken: bool,
}

impl SampleInfo {
    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames as f64 / self.sample_rate as f64
        }
    }
}

/// A loaded bank file.
pub struct SoundBank {
    pub kind: BankKind,
    pub path: PathBuf,
    data: Vec<u8>,
    pub version: u32,
    pub samples: Vec<SampleInfo>,
}

fn rd32(d: &[u8], o: usize) -> Result<u32> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| Error::format("sound bank", "truncated"))
}

impl SoundBank {
    /// Load through [`kfx_formats::rnc::load_file`] (the file may be RNC packed).
    pub fn load(kind: BankKind, path: &Path) -> Result<SoundBank> {
        let data = kfx_formats::rnc::load_file(path)?;
        Self::from_bytes(kind, path, data)
    }

    pub fn from_bytes(kind: BankKind, path: &Path, data: Vec<u8>) -> Result<SoundBank> {
        let fail = |m: String| Error::format("sound bank", format!("{}: {m}", path.display()));
        if data.len() < 4 + 18 + 9 * 16 {
            return Err(fail("file too small".into()));
        }
        let head = rd32(&data, data.len() - 4)? as usize;
        if head + 18 + 9 * 16 > data.len() {
            return Err(fail(format!("header offset {head} outside the file")));
        }
        let version = rd32(&data, head + 14)?;
        let e = head + 18 + 2 * 16;
        let first_sample = rd32(&data, e)? as usize;
        let first_data = rd32(&data, e + 4)? as usize;
        let total = rd32(&data, e + 8)? as usize;
        if first_sample == 0 || first_sample == u32::MAX as usize {
            return Err(fail("invalid sample offset".into()));
        }
        if total < 32 {
            return Err(fail("invalid samples size".into()));
        }
        let count = total / 32;
        let dir_end = first_sample
            .checked_add(count * 32)
            .ok_or_else(|| fail("overflow".into()))?;
        if dir_end > data.len() {
            return Err(fail("directory outside the file".into()));
        }
        let mut samples = Vec::with_capacity(count);
        for i in 0..count {
            let o = first_sample + i * 32;
            let raw = &data[o..o + 18];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(18);
            let name = kfx_formats::cfg::latin1_decode(&raw[..end]);
            let data_off = rd32(&data, o + 18)? as usize;
            let rate = rd32(&data, o + 22)?;
            let size = rd32(&data, o + 26)? as usize;
            let sfx_id = data[o + 30];
            let flags = data[o + 31];
            let offset = first_data.saturating_add(data_off);
            let reserved = i == 0;
            let mut info = SampleInfo {
                index: i as u16,
                name,
                offset,
                size,
                dir_sample_rate: rate,
                sfx_id,
                format_flags: flags,
                codec: Codec::Unknown,
                channels: 0,
                sample_rate: 0,
                frames: 0,
                reserved,
                broken: false,
            };
            if !reserved {
                match data.get(offset..offset.saturating_add(size).min(data.len())) {
                    Some(wav) if !wav.is_empty() => match codec::wav_info(wav) {
                        Ok((c, ch, sr, fr)) => {
                            info.codec = c;
                            info.channels = ch;
                            info.sample_rate = sr;
                            info.frames = fr;
                        }
                        Err(_) => info.broken = true,
                    },
                    _ => info.broken = true,
                }
            }
            samples.push(info);
        }
        Ok(SoundBank {
            kind,
            path: path.to_path_buf(),
            data,
            version,
            samples,
        })
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&SampleInfo> {
        self.samples.get(index)
    }

    /// The embedded WAV file of sample `index`.
    pub fn wav_bytes(&self, index: usize) -> Result<&[u8]> {
        let s = self
            .samples
            .get(index)
            .ok_or_else(|| Error::NotFound(format!("sample {index}")))?;
        if s.reserved {
            return Err(Error::NotFound("sample 0 is reserved".into()));
        }
        self.data
            .get(s.offset..s.offset.saturating_add(s.size).min(self.data.len()))
            .ok_or_else(|| Error::format("sound bank", format!("sample {index} outside the file")))
    }

    /// Decode a sample to PCM.
    pub fn decode(&self, index: usize) -> Result<Pcm> {
        Ok(codec::decode_wav(self.wav_bytes(index)?)?.0)
    }

    /// Index of the sample with this file name (case-insensitive), e.g. `foot1a.wav`.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.samples
            .iter()
            .position(|s| !s.reserved && s.name.eq_ignore_ascii_case(name))
    }
}

/// Speech file name for a language (`eng`) like `load_sound_banks`.
pub fn speech_candidates(language: &str) -> Vec<String> {
    let mut v = vec![
        format!("speech_{}.dat", language.to_ascii_lowercase()),
        "speech.dat".to_string(),
        "speech_eng.dat".to_string(),
    ];
    v.dedup();
    v
}

/// Open both banks of an install (`<install>/sound`). Either may be absent.
pub fn open_banks(install: &Path, language: &str) -> (Option<SoundBank>, Option<SoundBank>) {
    use kfx_formats::testutil::resolve_ci;
    let effects = resolve_ci(install, "sound/sound.dat")
        .filter(|p| p.is_file())
        .and_then(|p| SoundBank::load(BankKind::Effects, &p).ok());
    let speech = speech_candidates(language)
        .iter()
        .find_map(|n| resolve_ci(install, &format!("sound/{n}")).filter(|p| p.is_file()))
        .and_then(|p| SoundBank::load(BankKind::Speech, &p).ok());
    (effects, speech)
}
