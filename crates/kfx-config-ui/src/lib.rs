//! Configuration editors for KeeperFX levels, campaigns and mods (egui widgets, host
//! independent).
//!
//! KeeperFX loads every config file in layers: the install's `fxdata` (base), mods, the
//! campaign's `CONFIGS_LOCATION`, then `mapNNNNN.<file>` next to the level (see
//! [`layers`]). The editors work on one target layer and write only what differs from the
//! layers below, as a minimal partial file that keeps the comments and layout of an existing
//! override ([`cfgedit`], [`tomledit`], [`slabset`], [`columnset`]).
//!
//! The host provides:
//! * a [`fs::ConfigFs`] (read / write / remove / list) — the only file access of the crate;
//! * the [`layers::LayerStack`] of the level being edited and the target layer;
//! * the display art ([`assets::Assets`]).
//!
//! and calls [`ui::ConfigEditor::show`] every frame, then reacts to
//! [`session::ConfigEvent`]s (reload `GameData` with
//! [`session::ConfigSession::effective_gamedata`], regenerate slabs, open files). The creature
//! stats comparison window is [`ui::ConfigEditor::creature_stats_window`] (one call per frame,
//! see [`ui::stats`]).
pub mod assets;
pub mod cfgedit;
pub mod columnset;
pub mod creature;
pub mod diff;
pub mod fptd;
pub mod fs;
pub mod layers;
pub mod raster;
pub mod schema;
pub mod session;
pub mod slabset;
pub mod spritezip;
pub mod tiles;
pub mod tomledit;
pub mod ui;
pub mod validate;

pub use fs::{ConfigFs, MemFs, StdFs};
pub use layers::{Layer, LayerKind, LayerStack};
pub use session::{ConfigEvent, ConfigSession};
pub use ui::stats::{CreatureStats, StatsAction};
pub use ui::ConfigEditor;
