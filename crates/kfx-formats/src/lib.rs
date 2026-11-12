//! Readers and writers for the KeeperFX / Dungeon Keeper file formats a map editor needs. Pure
//! data: no rendering, no game logic.
//!
//! * [`level_bin`]: the binary per-level files (`.slb`, `.own`, `.clm`, `.dat`, `.tng`, ...),
//!   loaded together as [`level_bin::LevelBin`], with [`level_bin::MapDims`].
//! * [`level_fx`]: the TOML things / action points / lights files (`.tngfx`, `.aptfx`,
//!   `.lgtfx`: [`level_fx::TngFx`], [`level_fx::AptFx`], [`level_fx::LgtFx`]).
//! * [`levelinfo`]: `.lof` / `.lif` level information; [`campaign`]: campaign and map pack
//!   `.cfg` files ([`campaign::resolve`]).
//! * [`cfg`]: a lossless model of the game's `.cfg` dialect ([`cfg::CfgDoc`]).
//! * [`game_toml`]: a port of the game's own TOML reader; [`toml_emit`]: the writer that only
//!   produces TOML the game reads back as written.
//! * [`strings`]: string tables and the game's text encodings; [`translation`]:
//!   `fxdata/translation.toml`.
//! * [`rnc`]: RNC decompression ([`rnc::load_file`] reads packed or plain files).
//! * [`atomic`]: crash-safe writes ([`atomic::atomic_write`], [`atomic::SaveTransaction`]).
//! * [`testutil`]: helpers for tests and examples (test install, scratch folders).
pub mod atomic;
pub mod campaign;
pub mod cfg;
pub mod error;
pub mod game_toml;
pub mod level_bin;
pub mod level_fx;
pub mod levelinfo;
pub mod rnc;
pub mod strings;
pub mod testutil;
pub mod toml_emit;
pub mod translation;

pub use error::{Error, Result};
