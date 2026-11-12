//! kfx-logic-ui: visual level logic editor for KeeperFX DK level scripts.
//!
//! * [`model`]: the logic document ([`LogicDoc`]): events (trigger conditions + ordered
//!   actions, nested events for AND), parties, variable aliases. Independent of script syntax,
//!   but every node keeps its source line so an unedited script is written back byte for byte.
//! * [`import()`] / [`export()`]: DK script <-> model. [`compare()`] checks two scripts command by
//!   command per condition context; [`import()`] uses it to mark scripts it cannot represent
//!   faithfully as view-only ([`LogicDoc::read_only`]).
//! * [`analysis`]: readable summaries, map locations, variable usage, time gates, validation
//!   and resource meters of the generated script.
//! * [`lua`]: export to a Lua level script on the shipped trigger library, flagging what has no
//!   Lua equivalent.
//! * [`waves`]: party strength and wave templates.
//! * [`LogicEditor`]: the egui widget (event cards, node graph, timeline, party / wave designer,
//!   variables table, Lua export) with undo / redo, driven by a host through [`LogicHost`] and
//!   [`LogicEvent`].
pub mod analysis;
pub mod compare;
pub mod editor;
pub mod export;
pub mod import;
pub mod lua;
pub mod model;
pub mod syntax;
mod ui_args;
mod ui_cards;
mod ui_graph;
mod ui_parties;
mod ui_timeline;
pub mod waves;

pub use compare::{compare, compare_per_context, semantic_sequence};
pub use editor::{
    default_cmd, LogicEditor, LogicEvent, LogicHost, PickKind, PickRequest, PickValue, View,
};
pub use export::{export, export_bytes};
pub use import::import;
pub use model::*;
pub use ui_parties::lua_text;
