//! kfx-script: Dungeon Keeper / KeeperFX level script (`mapNNNNN.txt`) tooling.
//!
//! * [`Document::parse`]: lossless lexer + per-line parser that never fails.
//! * [`validate`]: diagnostics (`Diagnostic { severity, span, line, message, code }`).
//! * [`completions`], [`signature_help`], [`hover`], [`highlight`], [`outline`], [`references`],
//!   [`positions`], [`format`]: editor services over a parsed document.
//! * Config-driven constants come through [`ConstantProvider`].
//! * [`lua`]: the same services for Lua level scripts, and a DK script to Lua assistant.
pub mod consts;
pub mod diag;
pub mod doc;
pub mod overlay;
pub mod provider;
pub mod schema;
pub mod services;
pub mod tables;
pub mod util;
pub mod validate;

pub use diag::{
    hidden_at_caret, is_value_diagnostic, suppress_at_caret, token_at, Diagnostic, Severity, Span,
};
pub use doc::{Arg, ArgTok, Command, Document, FuncCall, Line, LineKind, TokKind, Token};
pub use overlay::{ArgKind, Overlay};
pub use provider::{
    ConstKind, ConstantProvider, InstallProvider, NullProvider, StaticProvider, EXTRA_CONST_KINDS,
};
pub use schema::{ConfigTarget, Schema};
pub use services::{
    completions, format, highlight, hover, outline, positions, references, signature_help,
    signature_of, string_arg_at, string_args, Completion, CompletionKind, FormatOptions, Hover,
    OutlineItem, OutlineKind, RefKind, ScriptPos, ScriptRef, SignatureHelp, StringArg, TokenClass,
};
pub use tables::Tables;
pub use validate::{validate, Report, Stats};
pub mod lua;
