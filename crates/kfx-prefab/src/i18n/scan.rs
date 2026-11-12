//! Source scanner behind the `extract_strings` example and the key tests: finds the
//! translated calls ([`super::tr`], [`super::trd`], [`super::trf`], [`super::trn`],
//! [`super::tx`]) with their
//! keys, English texts and argument names, and drafts the user-visible string literals that are
//! still passed to egui calls directly (`ui.label("..")`, `.on_hover_text("..")`, ...).
//!
//! It is a small tokenizer, not a Rust parser: it sees string literals, identifiers and
//! punctuation. What it cannot see is listed in the docs of `extract_strings`.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// (callee name, required qualifier, index of the text argument) of the egui calls whose
/// literal text argument is drafted.
pub const CALLS: &[(&str, Option<&str>, usize)] = &[
    ("label", None, 0),
    ("button", None, 0),
    ("small_button", None, 0),
    ("heading", None, 0),
    ("strong", None, 0),
    ("weak", None, 0),
    ("monospace", None, 0),
    ("colored_label", None, 1),
    ("selectable_label", None, 1),
    ("selectable_value", None, 2),
    ("radio_value", None, 2),
    ("radio", None, 1),
    ("checkbox", None, 1),
    ("toggle_value", None, 1),
    ("menu_button", None, 0),
    ("collapsing", None, 0),
    ("on_hover_text", None, 0),
    ("on_disabled_hover_text", None, 0),
    ("on_hover_text_at_pointer", None, 0),
    ("hint_text", None, 0),
    ("hyperlink_to", None, 0),
    ("link", None, 0),
    ("selected_text", None, 0),
    ("set_title", None, 0),
    ("add_filter", None, 0),
    ("new", Some("Button"), 0),
    ("selectable", Some("Button"), 1),
    ("new", Some("Window"), 0),
    ("new", Some("RichText"), 0),
    ("new", Some("Label"), 0),
    ("new", Some("CollapsingHeader"), 0),
    ("new", Some("Checkbox"), 1),
    ("new", Some("Hyperlink"), 0),
    ("new", Some("SelectableLabel"), 1),
    ("from_label", Some("ComboBox"), 0),
    ("title", None, 0),
];

/// Tables whose rows carry a stable id and an English label shown through a key built from
/// the id at display time: (path suffix of the file, row function, key prefix, index of the id
/// argument, index of the text argument). When both indices are equal the key is the
/// [`slug`] of the text.
pub const TABLES: &[(&str, &str, &str, usize, usize)] = &[
    (
        "kfx-editor/src/shortcuts.rs",
        "a",
        "kfx_editor.action.",
        0,
        1,
    ),
    (
        "kfx-editor/src/shortcuts.rs",
        "fixed",
        "kfx_editor.action.",
        0,
        1,
    ),
    (
        "kfx-editor/src/shortcuts.rs",
        "a",
        "kfx_editor.action_group.",
        2,
        2,
    ),
    (
        "kfx-editor/src/shortcuts.rs",
        "fixed",
        "kfx_editor.action_group.",
        2,
        2,
    ),
];

/// One translated call found in the code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use {
    pub key: String,
    /// `tr`, `trd`, `trf`, `trn`, `tx` or `table`.
    pub form: &'static str,
    /// The English text written in the code (`None` for `tr(key)`, which has none).
    pub text: Option<String>,
    /// `trn` only.
    pub one: Option<String>,
    pub other: Option<String>,
    /// The `{name}` arguments passed (`trf`, `trn`); `None` when not a literal list.
    pub args: Option<Vec<String>>,
    pub file: String,
    pub line: usize,
}

/// A literal still passed to an egui call directly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
    pub file: String,
    pub line: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub uses: Vec<Use>,
    pub drafts: Vec<Draft>,
    /// egui text arguments built with `format!` (not drafted).
    pub formatted: usize,
    /// egui text arguments that are variables or other expressions (data or texts made
    /// elsewhere).
    pub dynamic: usize,
    /// Translated calls whose key is not a string literal.
    pub unkeyed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Punct(char),
    /// Numbers, char literals, lifetimes.
    Other,
}

struct Lexed {
    toks: Vec<(Tok, usize)>,
    lines: Vec<usize>,
}

impl Lexed {
    fn line(&self, off: usize) -> usize {
        self.lines.partition_point(|&s| s <= off)
    }
}

fn lex(src: &str) -> Lexed {
    let b = src.as_bytes();
    let mut lines = vec![0usize];
    for (i, c) in b.iter().enumerate() {
        if *c == b'\n' {
            lines.push(i + 1);
        }
    }
    let mut toks = vec![];
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if c == b'"'
            || (c == b'r'
                && matches!(b.get(i + 1), Some(b'"') | Some(b'#'))
                && raw_start(b, i).is_some())
            || (c == b'b'
                && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'))
                && (b.get(i + 1) == Some(&b'"')
                    || (b.get(i + 1) == Some(&b'r')
                        && matches!(b.get(i + 2), Some(b'"') | Some(b'#')))))
        {
            let start = i;
            if c == b'b' {
                i += 1;
            }
            if b[i] == b'r' {
                let (hashes, open) = raw_start(b, i).unwrap_or((0, i + 1));
                let body = open + 1;
                let mut k = body;
                while k < b.len()
                    && !(b[k] == b'"'
                        && b[k + 1..]
                            .iter()
                            .take(hashes)
                            .filter(|&&x| x == b'#')
                            .count()
                            == hashes)
                {
                    k += 1;
                }
                toks.push((
                    Tok::Str(
                        String::from_utf8_lossy(&b[body.min(b.len())..k.min(b.len())]).to_string(),
                    ),
                    start,
                ));
                i = (k + 1 + hashes).min(b.len());
            } else {
                let mut out: Vec<u8> = vec![];
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                        match b.get(i) {
                            Some(b'n') => out.push(b'\n'),
                            Some(b't') => out.push(b'\t'),
                            Some(b'r') => out.push(b'\r'),
                            Some(b'0') => out.push(0),
                            Some(b'\n') => {
                                while b.get(i + 1).is_some_and(|c| c.is_ascii_whitespace()) {
                                    i += 1;
                                }
                            }
                            Some(b'u') => {
                                if let Some(end) = b[i..].iter().position(|&x| x == b'}') {
                                    let hex = String::from_utf8_lossy(
                                        b.get(i + 2..i + end).unwrap_or_default(),
                                    )
                                    .to_string();
                                    if let Some(ch) =
                                        u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                                    {
                                        let mut buf = [0u8; 4];
                                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                                    }
                                    i += end;
                                }
                            }
                            Some(&x) => out.push(x),
                            None => {}
                        }
                        i += 1;
                    } else {
                        out.push(b[i]);
                        i += 1;
                    }
                }
                i += 1;
                toks.push((Tok::Str(String::from_utf8_lossy(&out).to_string()), start));
            }
        } else if c == b'\'' {
            // char literal or lifetime
            let start = i;
            if b.get(i + 1) == Some(&b'\\') {
                i += 3;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
                i += 1;
            } else {
                let mut j = i + 1;
                if j < b.len() {
                    j += 1;
                    while j < b.len() && (b[j] & 0xC0) == 0x80 {
                        j += 1;
                    }
                }
                if b.get(j) == Some(&b'\'') {
                    i = j + 1;
                } else {
                    i += 1;
                    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                        i += 1;
                    }
                }
            }
            toks.push((Tok::Other, start));
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            toks.push((Tok::Ident(String::from_utf8_lossy(&b[s..i]).to_string()), s));
        } else if c.is_ascii_digit() {
            let s = i;
            while i < b.len()
                && (b[i].is_ascii_alphanumeric()
                    || b[i] == b'_'
                    || (b[i] == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())))
            {
                i += 1;
            }
            toks.push((Tok::Other, s));
        } else {
            // one token per character; multi-byte characters become one Other token
            let mut j = i + 1;
            while j < b.len() && (b[j] & 0xC0) == 0x80 {
                j += 1;
            }
            toks.push((
                if c.is_ascii() {
                    Tok::Punct(c as char)
                } else {
                    Tok::Other
                },
                i,
            ));
            i = j;
        }
    }
    Lexed { toks, lines }
}

/// `r"`, `r#"`: (number of hashes, index of the opening quote).
fn raw_start(b: &[u8], i: usize) -> Option<(usize, usize)> {
    if i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') && b[i - 1] != b'b' {
        return None;
    }
    let mut j = i + 1;
    let mut h = 0;
    while b.get(j) == Some(&b'#') {
        h += 1;
        j += 1;
    }
    (b.get(j) == Some(&b'"')).then_some((h, j))
}

/// Index of the token closing the bracket opened at `open`.
fn close_of(toks: &[(Tok, usize)], open: usize) -> usize {
    let mut depth = 0i32;
    for (k, (t, _)) in toks.iter().enumerate().skip(open) {
        match t {
            Tok::Punct('(') | Tok::Punct('[') | Tok::Punct('{') => depth += 1,
            Tok::Punct(')') | Tok::Punct(']') | Tok::Punct('}') => {
                depth -= 1;
                if depth == 0 {
                    return k;
                }
            }
            _ => {}
        }
    }
    toks.len()
}

/// The arguments of the call whose `(` is token `open`: token ranges split at depth-1 commas.
fn call_args(toks: &[(Tok, usize)], open: usize) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut depth = 0;
    let mut start = open + 1;
    for (k, (t, _)) in toks.iter().enumerate().skip(open) {
        match t {
            Tok::Punct('(') | Tok::Punct('[') | Tok::Punct('{') => depth += 1,
            Tok::Punct(')') | Tok::Punct(']') | Tok::Punct('}') => {
                depth -= 1;
                if depth == 0 {
                    if k > start {
                        out.push((start, k));
                    }
                    return out;
                }
            }
            Tok::Punct(',') if depth == 1 => {
                out.push((start, k));
                start = k + 1;
            }
            _ => {}
        }
    }
    out
}

/// Token ranges of `#[cfg(test)]` items (test modules are not scanned).
fn test_regions(toks: &[(Tok, usize)]) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let id = |k: usize, s: &str| matches!(toks.get(k), Some((Tok::Ident(x), _)) if x == s);
    let p = |k: usize, c: char| matches!(toks.get(k), Some((Tok::Punct(x), _)) if *x == c);
    for k in 0..toks.len() {
        if p(k, '#')
            && p(k + 1, '[')
            && id(k + 2, "cfg")
            && p(k + 3, '(')
            && id(k + 4, "test")
            && p(k + 5, ')')
            && p(k + 6, ']')
        {
            let mut j = k + 7;
            while j < toks.len() && !p(j, '{') && !p(j, ';') {
                j += 1;
            }
            if p(j, '{') {
                out.push((k, close_of(toks, j)));
            }
        }
    }
    out
}

enum Arg {
    Literal(String),
    Keyed,
    Formatted,
    Dynamic,
}

fn classify(toks: &[(Tok, usize)]) -> Arg {
    let t: Vec<&Tok> = toks.iter().map(|(t, _)| t).collect();
    // "literal" or &"literal"
    let t: Vec<&Tok> = if t.first() == Some(&&Tok::Punct('&')) {
        t[1..].to_vec()
    } else {
        t
    };
    if let [Tok::Str(s)] = t.as_slice() {
        return Arg::Literal(s.clone());
    }
    if let Some(Tok::Ident(n)) = t.first() {
        if n == "format" && t.get(1) == Some(&&Tok::Punct('!')) {
            return Arg::Formatted;
        }
        if matches!(n.as_str(), "tr" | "trd" | "trf" | "trn" | "tx")
            && t.get(1) == Some(&&Tok::Punct('('))
        {
            return Arg::Keyed;
        }
        // RichText::new(tr..(..)) and similar wrappers of a translated call
        if t.iter()
            .any(|x| matches!(x, Tok::Ident(n) if matches!(n.as_str(), "trd" | "trf" | "trn")))
        {
            return Arg::Keyed;
        }
    }
    Arg::Dynamic
}

fn lit(toks: &[(Tok, usize)], r: Option<&(usize, usize)>) -> Option<String> {
    let &(a, b) = r?;
    match &toks[a..b] {
        [(Tok::Str(s), _)] => Some(s.clone()),
        _ => None,
    }
}

/// Names of a literal argument list `&[("name", &value), ...]`.
fn arg_names(toks: &[(Tok, usize)], r: Option<&(usize, usize)>) -> Option<Vec<String>> {
    let &(a, b) = r?;
    let mut k = a;
    if toks.get(k).map(|t| &t.0) == Some(&Tok::Punct('&')) {
        k += 1;
    }
    if toks.get(k).map(|t| &t.0) != Some(&Tok::Punct('[')) || close_of(toks, k) + 1 != b {
        return None;
    }
    let mut out = vec![];
    let end = close_of(toks, k);
    let mut j = k + 1;
    while j < end {
        if toks[j].0 == Tok::Punct('(') {
            let args = call_args(toks, j);
            out.push(lit(toks, args.first())?);
            j = close_of(toks, j) + 1;
        } else {
            j += 1;
        }
    }
    Some(out)
}

/// Scan one source file. `file` is the path recorded in the results (also used to match
/// [`TABLES`]).
pub fn scan_source(src: &str, file: &str, out: &mut Scan) {
    let lx = lex(src);
    let toks = &lx.toks;
    let tests = test_regions(toks);
    let table_rules: Vec<_> = TABLES
        .iter()
        .filter(|t| file.replace('\\', "/").ends_with(t.0))
        .collect();
    for k in 1..toks.len() {
        if toks[k].0 != Tok::Punct('(') || tests.iter().any(|&(a, b)| (a..=b).contains(&k)) {
            continue;
        }
        let Tok::Ident(name) = &toks[k - 1].0 else {
            continue;
        };
        // a definition (`fn tr(`) is not a use
        if k >= 2 && toks[k - 2].0 == Tok::Ident("fn".into()) {
            continue;
        }
        let line = lx.line(toks[k].1);
        let args = call_args(toks, k);
        if matches!(name.as_str(), "tr" | "trd" | "trf" | "trn" | "tx")
            && !(k >= 2 && toks[k - 2].0 == Tok::Punct('.'))
        {
            let Some(key) = lit(toks, args.first()) else {
                out.unkeyed.push(format!("{file}:{line}"));
                continue;
            };
            let mut u = Use {
                key,
                form: "tr",
                text: None,
                one: None,
                other: None,
                args: None,
                file: file.to_string(),
                line,
            };
            match name.as_str() {
                "trd" | "tx" => {
                    u.form = if name == "tx" { "tx" } else { "trd" };
                    u.text = lit(toks, args.get(1));
                }
                "trf" => {
                    u.form = "trf";
                    u.text = lit(toks, args.get(1));
                    u.args = arg_names(toks, args.get(2));
                }
                "trn" => {
                    u.form = "trn";
                    u.one = lit(toks, args.get(2));
                    u.other = lit(toks, args.get(3));
                    u.args = arg_names(toks, args.get(4));
                }
                _ => {}
            }
            out.uses.push(u);
            continue;
        }
        for rule in &table_rules {
            if rule.1 == name && !(k >= 2 && toks[k - 2].0 == Tok::Punct('.')) {
                let (Some(id), Some(text)) =
                    (lit(toks, args.get(rule.3)), lit(toks, args.get(rule.4)))
                else {
                    continue;
                };
                let key = if rule.3 == rule.4 {
                    format!("{}{}", rule.2, slug(&id))
                } else {
                    format!("{}{}", rule.2, id)
                };
                out.uses.push(Use {
                    key,
                    form: "table",
                    text: Some(text),
                    one: None,
                    other: None,
                    args: None,
                    file: file.to_string(),
                    line,
                });
            }
        }
        let qual = if k >= 4 && toks[k - 2].0 == Tok::Punct(':') && toks[k - 3].0 == Tok::Punct(':')
        {
            match &toks[k - 4].0 {
                Tok::Ident(q) => Some(q.as_str()),
                _ => None,
            }
        } else {
            None
        };
        let Some(&(_, _, idx)) = CALLS
            .iter()
            .find(|(c, need, _)| c == name && need.is_none_or(|n| Some(n) == qual))
        else {
            continue;
        };
        let Some(&(a, b)) = args.get(idx) else {
            continue;
        };
        match classify(&toks[a..b]) {
            Arg::Literal(t) => {
                if is_text(&t) {
                    out.drafts.push(Draft {
                        text: t,
                        file: file.to_string(),
                        line: lx.line(toks[a].1),
                    });
                }
            }
            Arg::Keyed => {}
            Arg::Formatted => out.formatted += 1,
            Arg::Dynamic => out.dynamic += 1,
        }
    }
}

/// Does a literal look like a text rather than an identifier? It needs two letters and must not
/// be a dotted id (`file.open`), a command line flag (`-nomods`) or a file extension (`.cfg`).
pub fn is_text(t: &str) -> bool {
    let ident = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    let dotted = t.split('.').count() > 1 && t.split('.').all(ident);
    let flag = t.strip_prefix('-').is_some_and(ident);
    let ext = t.strip_prefix('.').is_some_and(ident);
    t.chars().filter(|c| c.is_alphabetic()).count() >= 2 && !dotted && !flag && !ext
}

/// Is this a file of tests (not scanned)?
pub fn is_test_file(p: &Path) -> bool {
    let n = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    n.contains("test") || n.starts_with("sweep_")
}

/// The `.rs` files below `dir`, sorted, without test files.
pub fn rs_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        for p in v {
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") && !is_test_file(&p) {
                out.push(p);
            }
        }
    }
    let mut out = vec![];
    walk(dir, &mut out);
    out
}

/// Scan every non-test `.rs` file below `dir`; paths are recorded relative to `root`.
pub fn scan_dir(dir: &Path, root: &Path) -> Scan {
    let mut out = Scan::default();
    for f in rs_files(dir) {
        let Ok(src) = std::fs::read_to_string(&f) else {
            continue;
        };
        let rel = f
            .strip_prefix(root)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        scan_source(&src, &rel, &mut out);
    }
    out
}

/// Key part made from a text: lower-case ASCII letters and digits, other runs become `_`, at
/// most 48 characters.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        "text".into()
    } else {
        out
    }
}

/// The `{name}` placeholders of a text.
pub fn placeholders(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(o) = rest.find('{') {
        rest = &rest[o + 1..];
        if let Some(c) = rest.find('}') {
            let name = &rest[..c];
            if !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
                out.insert(name.to_string());
                rest = &rest[c + 1..];
            }
        }
    }
    out
}

/// Problems between the code's translated calls and an English catalogue: missing keys, texts
/// that differ from the catalogue (run the extractor), one key with two texts, `trf` / `trn`
/// arguments that do not match the placeholders of the text.
pub fn check_uses(uses: &[Use], english: &super::Catalog) -> Vec<String> {
    let mut out = vec![];
    let mut seen: std::collections::BTreeMap<&str, &Use> = Default::default();
    for u in uses {
        let at = format!("{}:{}", u.file, u.line);
        if let Some(prev) = seen.get(u.key.as_str()) {
            if (&prev.text, &prev.one, &prev.other) != (&u.text, &u.one, &u.other)
                && u.text.is_some() == prev.text.is_some()
                && u.form != "tr"
            {
                out.push(format!(
                    "{at}: key {} has another text at {}:{}",
                    u.key, prev.file, prev.line
                ));
            }
        } else {
            seen.insert(&u.key, u);
        }
        let Some(e) = english.entries.get(&u.key) else {
            out.push(format!(
                "{at}: key {} is not in the English catalogue",
                u.key
            ));
            continue;
        };
        if let Some(t) = &u.text {
            if &e.text != t {
                out.push(format!(
                    "{at}: key {}: the catalogue has {:?}, the code {:?}",
                    u.key, e.text, t
                ));
            }
        }
        if u.form == "trn" && (e.one != u.one || e.other != u.other) {
            out.push(format!(
                "{at}: key {}: plural forms differ from the catalogue",
                u.key
            ));
        }
        match (u.form, &u.args) {
            ("trf", Some(args)) => {
                let want = placeholders(u.text.as_deref().unwrap_or_default());
                let have: BTreeSet<String> = args.iter().cloned().collect();
                if want != have {
                    out.push(format!(
                        "{at}: key {}: placeholders {want:?} but arguments {have:?}",
                        u.key
                    ));
                }
            }
            ("trn", Some(args)) => {
                let mut want = placeholders(u.one.as_deref().unwrap_or_default());
                want.extend(placeholders(u.other.as_deref().unwrap_or_default()));
                let mut have: BTreeSet<String> = args.iter().cloned().collect();
                have.insert("n".into());
                if !want.is_subset(&have) {
                    out.push(format!(
                        "{at}: key {}: placeholders {want:?} but arguments {have:?}",
                        u.key
                    ));
                }
            }
            ("trf" | "trn", None) => out.push(format!(
                "{at}: key {}: the arguments are not a literal list",
                u.key
            )),
            _ => {}
        }
    }
    out
}

/// Problems of a translation against the English catalogue: placeholders that differ (a
/// translation may drop none and add none), plural keys translated as plain text and back.
pub fn check_translation(english: &super::Catalog, other: &super::Catalog) -> Vec<String> {
    let mut out = vec![];
    for (k, t) in &other.entries {
        let Some(e) = english.entries.get(k) else {
            out.push(format!("{}: {k} is not an English key", other.code));
            continue;
        };
        let all = |e: &super::Entry| {
            let mut s = placeholders(&e.text);
            for x in [&e.one, &e.other].into_iter().flatten() {
                s.extend(placeholders(x));
            }
            s
        };
        let (pe, pt) = (all(e), all(t));
        if !pt.is_subset(&pe) || (e.one.is_none() && pe != pt) {
            out.push(format!(
                "{}: {k}: placeholders {pt:?}, English has {pe:?}",
                other.code
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_calls_and_drafts() {
        let src = r#"
fn ui() {
    ui.label(trd("x.hello", "Hello"));
    ui.label(trf("x.n_things", "{n} things in {where}", &[("n", &n), ("where", &w)]));
    let s = trn("x.files", k, "{n} file", "{n} files", &[]);
    ui.button("Draft me");
    ui.label(format!("{a}"));
    ui.label(name);
    let k = tr(&key);
}
#[cfg(test)]
mod tests { fn t() { ui.label("not me"); } }
"#;
        let mut s = Scan::default();
        scan_source(src, "a.rs", &mut s);
        assert_eq!(s.uses.len(), 3);
        assert_eq!(s.uses[0].text.as_deref(), Some("Hello"));
        assert_eq!(
            s.uses[1].args.as_deref(),
            Some(&["n".to_string(), "where".to_string()][..])
        );
        assert_eq!(s.uses[2].other.as_deref(), Some("{n} files"));
        assert_eq!(s.drafts.len(), 1);
        assert_eq!(s.drafts[0].text, "Draft me");
        assert_eq!((s.formatted, s.dynamic), (1, 1));
        assert_eq!(s.unkeyed.len(), 1);
        assert_eq!(
            placeholders("{a} and {b_2} {{x"),
            ["a", "b_2"].iter().map(|s| s.to_string()).collect()
        );
    }
}
