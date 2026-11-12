//! Semantic comparison of two DK scripts.
//!
//! Both scripts are parsed with [`kfx_script::Document`]; each command line becomes
//! `context :: [R] COMMAND(args)` where `context` is the chain of enclosing `IF*` lines (as the
//! game reads them) and `[R]` marks a command preceded by `NEXT_COMMAND_REUSABLE`. Comments,
//! blank lines, whitespace and identifier case are ignored. Two scripts are semantically the
//! same when these sequences are equal.
use crate::syntax::{command_key, is_if};
use kfx_script::{Document, LineKind};

/// The command sequence of a script, one entry per command (see the module docs).
pub fn semantic_sequence(src: &[u8]) -> Vec<String> {
    let doc = Document::parse(src);
    let mut out = vec![];
    let mut ctx: Vec<String> = vec![];
    let mut reusable = 0u8;
    for l in &doc.lines {
        if reusable > 0 {
            reusable -= 1;
        }
        match l.kind {
            LineKind::Blank | LineKind::Comment => continue,
            LineKind::Error => {
                let t = doc
                    .text_lossy(l.span)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push(format!("{} :: !error {t}", ctx.join(" & ")));
                continue;
            }
            LineKind::Command => {}
        }
        let c = l.cmd.as_ref().unwrap();
        let valid = c.desc.is_some() && c.syntax_ok;
        if valid && c.name == "NEXT_COMMAND_REUSABLE" {
            reusable = 2;
            continue;
        }
        let key = command_key(&doc, c);
        if valid && is_if(&c.name) {
            ctx.push(key);
            continue;
        }
        if valid && c.name == "ENDIF" && !ctx.is_empty() {
            ctx.pop();
            continue;
        }
        let r = if reusable > 0 { "[R] " } else { "" };
        out.push(format!("{} :: {r}{key}", ctx.join(" & ")));
    }
    if !ctx.is_empty() {
        out.push(format!("unclosed: {}", ctx.join(" & ")));
    }
    out
}

/// `Ok` when both scripts have the same command sequence; else the first difference.
pub fn compare(a: &[u8], b: &[u8]) -> Result<(), String> {
    let sa = semantic_sequence(a);
    let sb = semantic_sequence(b);
    if sa == sb {
        return Ok(());
    }
    let i = sa
        .iter()
        .zip(sb.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(sa.len().min(sb.len()));
    Err(format!(
        "command {} differs: before `{}`, after `{}` ({} vs {} commands)",
        i + 1,
        sa.get(i).map(|s| s.as_str()).unwrap_or("<end>"),
        sb.get(i).map(|s| s.as_str()).unwrap_or("<end>"),
        sa.len(),
        sb.len()
    ))
}

/// Per condition context comparison (order within each context kept, contexts may move):
/// `Ok` when every context holds the same command list in both scripts.
pub fn compare_per_context(a: &[u8], b: &[u8]) -> Result<(), String> {
    use std::collections::BTreeMap;
    fn group(s: Vec<String>) -> BTreeMap<String, Vec<String>> {
        let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for e in s {
            let (c, k) = e
                .split_once(" :: ")
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .unwrap_or((String::new(), e));
            m.entry(c).or_default().push(k);
        }
        m
    }
    let ga = group(semantic_sequence(a));
    let gb = group(semantic_sequence(b));
    if ga == gb {
        return Ok(());
    }
    for (k, v) in &ga {
        if gb.get(k) != Some(v) {
            return Err(format!("context `{k}` differs"));
        }
    }
    Err("a context exists only after export".into())
}
