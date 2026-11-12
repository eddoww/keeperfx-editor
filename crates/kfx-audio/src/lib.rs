//! Game sounds for the KeeperFX map editor: sound banks, resolution like the game, preview
//! playback.
//!
//! * [`bank`]: the `sound.dat` / `speech_<lang>.dat` container (format documented there).
//! * [`codec`]: WAV (PCM, MS ADPCM) and external formats (MP3, Ogg Vorbis, FLAC, `BMU V1.0`),
//!   decoded to [`Pcm`].
//! * [`context`], [`audio`]: `sounds.cfg` layering, custom file search, speech overrides, creature
//!   sound sets, level speech, music tracks; `Audio::resolve` and `Audio::decode`.
//! * [`play`]: playback engine behind a trait (cpal backend with feature `playback`, null backend).
//! * [`ui`] (feature `ui`): egui sound browser.
pub mod audio;
pub mod bank;
pub mod codec;
pub mod context;
pub mod error;
pub mod pcm;
pub mod play;
pub mod speech_names;
#[cfg(feature = "ui")]
pub mod ui;

pub use audio::{
    Audio, CatalogEntry, Category, CreatureSet, CreatureSound, Layer, LayerKind, MusicTrack,
    Resolved, Source, Variant,
};
pub use bank::{BankKind, SampleInfo, SoundBank};
pub use codec::Codec;
pub use context::{CampaignDirs, Context, ModTiers};
pub use error::{Error, Result};
pub use pcm::Pcm;
