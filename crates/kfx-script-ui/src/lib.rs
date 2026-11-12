//! egui script editor widget for KeeperFX level scripts.
//!
//! [`ScriptEditor`] is a reusable widget plus state object with no dependency on the main app:
//! the host owns it, calls [`ScriptEditor::show`] every frame with a
//! [`kfx_script::ConstantProvider`], and reacts to the returned [`ScriptEvent`]s.
//!
//! * Language plumbing: [`lang::Language`] / [`lang::Analysis`]; [`dk::DkLanguage`] is the DK
//!   level script implementation, [`lua::LuaLanguage`] the Lua one (on [`kfx_script::lua`]).
//! * [`workspace::ScriptWorkspace`]: tabs for a level's DK and Lua scripts (and read-only library
//!   files), dirty state, creation from templates, combined problems, external-change handling.
//!   File access goes through the host's [`workspace::ScriptFiles`].
//! * [`snippets`]: the snippet library shared by both languages.
//! * Text is held as bytes-preserving Latin-1 ([`latin1`]); [`ScriptEditor::text_bytes`] returns
//!   the file bytes.
//! * [`wizard`]: the script wizard (start gold, availability, computer players, alliances,
//!   win/lose) writing one delimited block into the DK or Lua script; [`wizard::ScriptWizard`] is
//!   its form, [`workspace::ScriptWorkspace::open_wizard`] hosts it.
//! * Pure editing logic (comments, indentation, find/replace, snippets, completion edits) lives
//!   in [`textops`] and is unit tested without a UI.
pub mod dk;
pub mod docs;
mod editor;
pub mod lang;
pub mod latin1;
pub mod lua;
mod panels;
pub mod snippets;
pub mod sounds;
pub mod strings;
pub mod textops;
pub mod theme;
mod ui;
pub mod wizard;
mod wizard_ui;
pub mod workspace;
#[doc(hidden)]
pub use ui::build_job;

pub use dk::DkLanguage;
pub use editor::{EditorOptions, ScriptEditor, ScriptEvent, SidePanel};
pub use lang::{
    Analysis, ArgNav, CallParens, Completion, CompletionAction, CompletionKind, Definition,
    HlClass, HoverInfo, Language, MapReference, MapTarget, Meter, MeterLevel, OutlineItem,
    PickKind, PickValue, RefEntry, SigParam, SignatureHelp, StatusItem, Tone, Validation,
};
pub use latin1::Eol;
pub use lua::{LibraryRoot, LuaLanguage};
pub use snippets::{LuaPlace, Snippet};
pub use sounds::{SoundArg, SoundArgKind, SoundLookup};
pub use strings::StringLookup;
pub use textops::LineIndex;
pub use theme::Theme;
