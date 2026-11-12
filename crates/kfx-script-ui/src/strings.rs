//! String-id arguments: hover with the text of the string, completion listing ids with a preview.
//!
//! The host supplies a [`StringLookup`] (`fn(id) -> Option<String>`, resolving an id the way the
//! game's `get_string` does for the open level). Ids are probed from `0` to
//! [`MAX_SCAN_ID`]; the level and campaign tables hold ids up to 2000 (`STRINGS_MAX`,
//! `src/config_strings.h:29`).
use crate::lang::{Completion, CompletionAction, CompletionKind, HoverInfo};
use kfx_script::{Span, StringArg};
use std::sync::Arc;

/// Resolves a string id to its text; `None` (or an empty text) when the id has no string.
pub type StringLookup = Arc<dyn Fn(i64) -> Option<String> + Send + Sync>;

/// Highest id offered by completion.
pub const MAX_SCAN_ID: i64 = 2000;

/// Label of the item that asks the host to create a string.
pub const NEW_STRING_LABEL: &str = "New string…";

/// One line of at most `max` characters.
pub fn preview(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > max {
        let cut: String = line.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else if text.trim().lines().count() > 1 {
        format!("{line} …")
    } else {
        line.to_string()
    }
}

/// Hover card for the id under the caret.
pub fn hover(arg: &StringArg, lookup: &StringLookup) -> Option<HoverInfo> {
    let id = arg.id?;
    let text = lookup(id).filter(|t| !t.is_empty());
    let (summary, title) = match text {
        Some(t) => (t, format!("{}: string {id}", arg.param)),
        None => (
            format!("No text for string {id} in this level, its campaign or the GUI strings."),
            format!("{}: string {id}", arg.param),
        ),
    };
    Some(HoverInfo {
        span: arg.span,
        title: Some(title),
        summary,
        ..Default::default()
    })
}

/// Completion items for a string-id argument: every id with a text (whose digits start with what is
/// typed in `typed`), each with a preview, after the "New string…" item.
pub fn completions(arg: &StringArg, typed: &str, lookup: &StringLookup) -> Vec<Completion> {
    let mut out = vec![];
    let mut new = Completion::new(NEW_STRING_LABEL, CompletionKind::Snippet, "", arg.span);
    new.detail = "create a string and insert its id".into();
    new.doc = "Creates a new string of the level and writes its ID here.".into();
    new.action = Some(CompletionAction::NewString);
    out.push(new);
    for id in 0..=MAX_SCAN_ID {
        let label = id.to_string();
        if !label.starts_with(typed) {
            continue;
        }
        let Some(text) = lookup(id).filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let mut c = Completion::new(label.clone(), CompletionKind::Constant, label, arg.span);
        c.detail = preview(&text, 60);
        c.doc = text;
        out.push(c);
    }
    out
}

/// Span of the argument text typed up to `offset`, for filtering.
pub fn typed_prefix<'a>(src: &'a [u8], span: Span, offset: usize) -> &'a str {
    let end = offset
        .clamp(span.start, span.end.max(span.start))
        .min(src.len());
    let start = span.start.min(end);
    std::str::from_utf8(&src[start..end]).unwrap_or("")
}
