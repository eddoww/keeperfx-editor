//! Translation workflow for the texts of a level or a campaign.
//!
//! # What is collected
//! Every string id a level refers to: the message arguments of its DK script
//! (`kfx_script::string_args`) and Lua script (`kfx_script::lua::string_args`), the level's
//! name and description ids (`.lif` `#id` names, `.lof` / campaign block `NAME_ID`,
//! `INTRO_DESC_ID`). Only ids `0..2000` are level / campaign strings; ids of the translation
//! table (`2000..4000`) and the GUI (`4000..`) are counted in [`TextProject::ignored`] and left
//! alone.
//!
//! # Where an id is read from and written to
//! The game answers an id below 2000 from the level's `mapNNNNN.<lang>.dat` when that string is
//! not empty, else from the campaign strings file of the language, else from the GUI string of
//! the same index (`get_string`, src/config_strings.c:420; `kfx_formats::strings`). Writes follow it:
//!
//! * a row whose id is present in the source language's level file belongs to that level
//!   ([`Scope::Level`]) and is written to `mapNNNNN.<lang>.dat`;
//! * a row whose id is in the source language's campaign file (`[strings]` of the campaign
//!   cfg) belongs to the campaign ([`Scope::Campaign`]) and is written to the campaign's file of
//!   the language, shared by every level of the campaign. Name / description ids belong to the
//!   campaign file too (the land view reads them before a level loads);
//! * any other id is a new level text: `mapNNNNN.<lang>.dat`.
//!
//! The files of the game install (`fxdata/gtext_*.dat`, also used as campaign strings by
//! campaigns that have none of their own) are never written; a campaign row of a language that
//! has no campaign file of its own cannot be edited until [`TextProject::create_campaign_file`]
//! made one. Every file is written through `kfx_formats::atomic`.
//!
//! # Outdated tracking
//! Next to the strings a small sidecar (`mapNNNNN.kfxtexts.toml`, `<campaign>.kfxtexts.toml`)
//! records, for each text a tool translated, a hash of the source text it was translated from.
//! When the pinned source language text changes, the translation shows as *outdated*.
pub mod csv;
pub mod po;
pub mod project;
pub mod widget;

pub use project::*;

/// FNV-1a 64 bit of a text, as 16 hex digits (the "source hash" of the sidecar).
pub fn text_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}
