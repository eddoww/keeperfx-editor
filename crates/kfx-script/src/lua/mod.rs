//! Lua (LuaJIT) level-script tooling for KeeperFX.
//!
//! KeeperFX runs LuaJIT (Lua 5.1 plus extensions and the `bit` library): `fxdata/lua/init.lua`,
//! then the campaign's `<CONFIGS_LOCATION>/lua/init.lua`, then `<LEVELS_LOCATION>/mapNNNNN.lua`
//! (`src/lua_base.c:461-550`). Hooks are global functions called by name (`src/lua_triggers.c`, see
//! [`hooks`]).
//!
//! # Pipeline
//! * [`lexer`]: lossless tokens with byte spans (LuaJIT numbers, long brackets, labels, escapes).
//! * [`parser`] / [`ast`]: tolerant recursive descent, never panics, reports errors and continues.
//!   Written in-tree instead of using `full_moon` (evaluated with its `luajit` feature: it parses
//!   all shipped files, but its error recovery drops whole constructs, e.g. `if x then foo(` leaves
//!   one statement, reports errors on tokens instead of the construct, offers no scopes and no
//!   argument/comma positions, and is a dependency under another licence).
//! * [`analyze`]: locals, parameters, upvalues, global writes, function table.
//! * [`annot`]: LuaLS annotation blocks and the type grammar.
//! * [`index`]: [`ApiIndex`], the annotation index (functions, classes, aliases, enums, globals);
//!   [`doc::LuaDoc`] bundles all of the above for one file and exposes the file's own definitions.
//! * [`infer`]: [`infer::Env`], name resolution and shallow type inference over file + index.
//! * Services: [`diagnostics()`], [`completions()`], [`signature_help()`], [`hover()`],
//!   [`highlight()`], [`outline()`], [`definition()`], [`references()`] / [`positions()`] (map
//!   markers), [`format()`], [`new_script()`] (a new script from a [`template`]).
//! * [`convert`]: the DK script to Lua assistant (a starting point, not a full translation).
//!
//! # Regenerating the shipped data
//! * `data/lua_api.json` (embedded by [`ApiIndex::stock`]):
//!   `KFX_LUA_REGEN=1 cargo test -p kfx-script --test lua_index_gen -- --nocapture`
//! * `data/lua_c_api.json` (what the C code registers):
//!   `python3 -I tools/extract_lua_api.py [KEEPERFX_SRC_DIR] [OUTPUT_JSON]`; the report comparing
//!   both is printed by `cargo test -p kfx-script --test lua_c_crosscheck -- --nocapture`.
pub mod analyze;
pub mod annot;
pub mod ast;
pub mod convert;
pub mod diagnostics;
pub mod doc;
pub mod hooks;
pub mod index;
pub mod infer;
pub mod lexer;
pub mod nav;
pub mod parser;
pub mod services;
pub mod stdlib;
pub mod template;

pub use diagnostics::{diagnostics, LuaDiagnostic};
pub use doc::LuaDoc;
pub use index::ApiIndex;
pub use nav::{
    definition, format, outline, positions, references, LuaFormatOptions, LuaLocation,
    LuaOutlineItem, LuaOutlineKind,
};
pub use services::{
    completions, highlight, hover, signature_help, string_arg_at, string_args, LuaCompletion,
    LuaCompletionKind, LuaHover, LuaSignatureHelp, LuaTokenClass,
};
pub use template::{new_script, TemplateOptions};
