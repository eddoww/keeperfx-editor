//! Map document model and slab autotile for KeeperFX maps.
//!
//! * [`gamedata`]: the layered game configuration the slab logic needs (terrain, cubes,
//!   slabset/columnset, sprite colours) plus generic name lists (creatures, objects, traps, doors,
//!   effect generators).
//! * [`column`]: column helpers (the game's `create_column` bitfield rules) and the
//!   [`column::ColumnPool`] that builds `.clm` + `.dat`.
//! * [`autotile`]: port of the game's slab placement (`src/map_blocks.c`).
//! * [`decorate`]: optional, deterministic decoration randomisation (cube variant families
//!   derived from `cubes.cfg`).
//! * [`mapdoc`]: the editable map document (load / save).
//! * [`things`]: things, action points, lights: queries and reversible edits (doors, hero
//!   gates, ownership).
//! * [`transform`]: resize, flip, rotate (map or rectangle) and symmetry helpers.
//! * [`validate`]: checks against the game's limits and load rules.
//! * [`patch`]: reversible document changes ([`patch::Patch`]) for undo / redo.
//! * [`legacy`]: saving in the original `.tng` / `.apt` / `.lgt` formats.
//! * [`creature`]: typed per-creature configuration (`creatrs/<name>.cfg`).
//! * [`mods`], [`layered`]: the mod / campaign / map configuration layers and per-key
//!   provenance.
//! * [`sidecar`]: editor-only data saved next to a map (custom slabs, editor settings);
//!   [`unearth`]: the `.une` custom slab files of the Unearth editor.
pub mod autotile;
pub mod column;
pub mod creature;
pub mod decorate;
pub mod error;
pub mod gamedata;
pub mod layered;
pub mod legacy;
pub mod mapdoc;
pub mod mods;
pub mod patch;
pub mod sidecar;
pub mod things;
pub mod transform;
pub mod unearth;
pub mod validate;

pub use error::{Error, Result};
