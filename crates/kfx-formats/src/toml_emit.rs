//! The one TOML writer for files the game reads (`.tngfx` / `.aptfx` / `.lgtfx`, TOML config
//! overrides). It only produces forms the game's reader accepts and reads as written
//! ([`crate::game_toml`], a port of `deps/centitoml`):
//!
//! * Keys are bare when they match `[A-Za-z0-9_-]+` (`normalize_key`, toml_api.c:629), else a
//!   basic string. (For `key = value` the game keeps the quotes as part of the key,
//!   toml_api.c:768; quoting is only needed for keys the game does not know anyway.)
//! * Strings are always one-line basic strings: `"` and `\` escaped, control characters as
//!   `\b \t \n \f \r` or `\u00XX` with upper-case hex (the game rejects lower-case hex,
//!   toml_api.c:503), and every `'` as `'` when the text holds `'''` (rejected inside a basic
//!   string, toml_api.c:1648). Never `"""`: the game keeps the newline after the opening quotes.
//! * Integers in decimal; floats with a `.` or an exponent (`{:?}`), `nan` / `inf` as TOML
//!   spells them.
//! * Tables: a top-level table gets a `[a]` header (even when empty or holding only
//!   sub-tables: `[a.b]` needs `a` to exist, toml_api.c:1265); its sub-tables get `[a.b]`
//!   headers. Deeper tables are written as dotted keys inside the `[a.b]` section (`c.x = 1`,
//!   an empty one as `c = {}`): a third header level walks through `b`, which fails once `b` is
//!   explicit (toml_api.c:1294 / 683).
//! * Arrays of tables: `[[a]]` / `[[a.b]]` headers at the first two levels, inline `[{ ... }]`
//!   deeper; tables inside their elements as dotted keys (headers cannot reach into arrays).
//! * Values at the top of a document come before the first header.
use toml::{Table, Value};

/// Whether `k` can be written without quotes.
pub fn is_bare_key(k: &str) -> bool {
    !k.is_empty()
        && k.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// A key as written: bare, or a basic string.
pub fn key(k: &str) -> String {
    if is_bare_key(k) {
        k.to_string()
    } else {
        string(k)
    }
}

/// A one-line basic string the game reads back unchanged.
pub fn string(s: &str) -> String {
    let triple = s.contains("'''");
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\u{8}' => o.push_str("\\b"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            '\u{c}' => o.push_str("\\f"),
            '\r' => o.push_str("\\r"),
            '\'' if triple => o.push_str("\\u0027"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                o.push_str(&format!("\\u{:04X}", c as u32))
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn float(f: f64) -> String {
    if f.is_nan() {
        "nan".into()
    } else if f.is_infinite() {
        if f > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        }
    } else {
        format!("{f:?}")
    }
}

/// An inline value (tables as `{ k = v }`).
pub fn value(v: &Value) -> String {
    match v {
        Value::String(s) => string(s),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => float(*f),
        Value::Boolean(b) => b.to_string(),
        Value::Datetime(d) => d.to_string(),
        Value::Array(a) => format!("[{}]", a.iter().map(value).collect::<Vec<_>>().join(", ")),
        Value::Table(t) if t.is_empty() => "{}".into(),
        Value::Table(t) => format!(
            "{{ {} }}",
            t.iter()
                .map(|(k, v)| format!("{} = {}", key(k), value(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `key = value` and a newline.
pub fn keyval(out: &mut String, k: &str, v: &Value) {
    out.push_str(&key(k));
    out.push_str(" = ");
    out.push_str(&value(v));
    out.push('\n');
}

fn is_aot(v: &Value) -> bool {
    matches!(v, Value::Array(a) if !a.is_empty() && a.iter().all(Value::is_table))
}

fn header(path: &[&str]) -> String {
    path.iter().map(|k| key(k)).collect::<Vec<_>>().join(".")
}

/// Sub-tables of a section as dotted keys (`prefix.k = v`).
fn dotted(out: &mut String, prefix: &str, t: &Table) {
    if t.is_empty() {
        out.push_str(&format!("{prefix} = {{}}\n"));
        return;
    }
    for (k, v) in t {
        let p = format!("{prefix}.{}", key(k));
        match v {
            Value::Table(sub) => dotted(out, &p, sub),
            v => out.push_str(&format!("{p} = {}\n", value(v))),
        }
    }
}

/// The body of a section at `depth` (1 or 2 for headers, anything for array elements):
/// returns the children that get their own headers.
fn body<'a>(out: &mut String, t: &'a Table, headers_below: bool) -> Vec<(&'a str, &'a Value)> {
    let mut later = vec![];
    for (k, v) in t {
        match v {
            Value::Table(_) if headers_below => later.push((k.as_str(), v)),
            v if headers_below && is_aot(v) => later.push((k.as_str(), v)),
            Value::Table(_) => {}
            v => keyval(out, k, v),
        }
    }
    if !headers_below {
        for (k, v) in t {
            if let Value::Table(sub) = v {
                dotted(out, &key(k), sub);
            }
        }
    }
    later
}

fn section(out: &mut String, path: &[&str], t: &Table) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("[{}]\n", header(path)));
    let later = body(out, t, path.len() < 2);
    children(out, path, later);
}

fn aot_element(out: &mut String, path: &[&str], t: &Table) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("[[{}]]\n", header(path)));
    body(out, t, false);
}

fn children(out: &mut String, path: &[&str], later: Vec<(&str, &Value)>) {
    for (k, v) in later {
        let mut p = path.to_vec();
        p.push(k);
        match v {
            Value::Table(sub) => section(out, &p, sub),
            Value::Array(a) => {
                for e in a {
                    if let Value::Table(et) = e {
                        aot_element(out, &p, et);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The top-level `key = value` lines of `root` (everything that is neither a table nor an
/// array of tables). They must precede the first header.
pub fn top_values(out: &mut String, root: &Table) {
    for (k, v) in root {
        if !v.is_table() && !is_aot(v) {
            keyval(out, k, v);
        }
    }
}

/// The tables and arrays of tables of `root` as sections (see the module docs).
pub fn sections(out: &mut String, root: &Table) {
    let later: Vec<(&str, &Value)> = root
        .iter()
        .filter(|(_, v)| v.is_table() || is_aot(v))
        .map(|(k, v)| (k.as_str(), v))
        .collect();
    children(out, &[], later);
}

/// A whole document.
pub fn document(root: &Table) -> String {
    let mut out = String::new();
    top_values(&mut out, root);
    sections(&mut out, root);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_toml;

    fn t(s: &str) -> Table {
        s.parse().unwrap()
    }

    #[test]
    fn strings_survive_the_game_reader() {
        for s in [
            "Bob",
            "Bob \"the\" Imp",
            "back\\slash",
            "it's",
            "a'\"b",
            "two\nlines",
            "tab\there",
            "del\u{7f}x",
            "Bjørn",
            "nul\u{1}x",
            "x'''y\"z\nw",
            "",
        ] {
            let mut root = Table::new();
            root.insert("k".into(), Value::String(s.into()));
            let text = document(&root);
            assert_eq!(game_toml::same_as_toml(&text), Ok(()), "{s:?} -> {text}");
            assert_eq!(text.parse::<Table>().unwrap()["k"].as_str(), Some(s));
        }
    }

    #[test]
    fn nested_tables_use_accepted_headers() {
        for src in [
            "[foo]\n\n[foo.bar]\ny = 2\n",
            "[deep]\nx = 1\n[deep.a]\nz = 1\n[deep.a.b]\nw = 1\n[deep.a.c]\nv = 1\n[deep.a.d.e]\nu \
                = 1\n",
            "top = 1\n[[arr]]\nx = 1\n[[arr]]\nx = 2\n[arr2]\n[[arr2.list]]\ny = 1\n",
            "[a]\n\"needs quoting\" = 1\nempty = {}\n[a.\"b c\"]\nd = [1, 2]\n",
            "[x]\ny = [{a = 1}, {b = {c = 2}}]\n",
        ] {
            let root = t(src);
            let text = document(&root);
            assert_eq!(game_toml::same_as_toml(&text), Ok(()), "{src}\n---\n{text}");
            assert_eq!(text.parse::<Table>().unwrap(), root, "{text}");
        }
    }
}
