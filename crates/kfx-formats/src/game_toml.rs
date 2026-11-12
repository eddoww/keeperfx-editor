//! The game's TOML reader, ported: `toml_parse` of `deps/centitoml/toml_api.c` (CentiTOML, a
//! cut-down tomlc99). It is what the game uses for `.tngfx` / `.aptfx` / `.lgtfx`
//! (`load_kfx_toml_file`, src/lvl_filesdk1.c:787) and its TOML configs, and it differs from the TOML
//! specification (and from the `toml` crate) in ways that matter for files the editors write:
//!
//! * A table header `[a.b]` needs `a` to exist already: the walk from the root does not create
//!   missing root-level tables (`walk_tabpath`, toml_api.c:1250, case `'r'`), so the
//!   `create_table_in_table` that follows fails with "out of memory" (toml_api.c:702).
//! * A middle table of a deeper header is created implicitly (toml_api.c:1294) and turns
//!   explicit the next time a header walks through it (toml_api.c:683); a header walking
//!   through an explicit middle table fails with "duplicate key". So `[a]` `[a.b.c]` works, `[a]`
//!   `[a.b]` `[a.b.c]` does not, and neither does a third `[a.b.x]` after `[a.b.c]` `[a.b.d]`.
//! * Headers cannot reach into arrays of tables (`[[a]]` then `[a.b]` or `[[a.b]]` fails).
//! * `key = value` stores the key *as written*, quotes included (`create_value_in_table` uses
//!   the raw token, toml_api.c:768); headers, dotted keys, arrays and inline tables use the
//!   unquoted key. So `"ThingType" = 1` is not the key `ThingType` for the game.
//! * Duplicate keys fail, except a duplicate whose value is `[]` (the failed array is parsed into
//!   nothing and no error is returned, toml_api.c:1162).
//! * Basic strings: `\u` / `\U` take upper-case hex digits only (toml_api.c:503); `'''` cannot
//!   appear in a one-line basic string (toml_api.c:1648); a multi-line string keeps the newline
//!   right after the opening quotes (no trimming in `norm_basic_str`).
//! * Values: `true`/`false` match by prefix (toml_conv.c:57); integers follow `toml_is_i`
//!   (`-0` and `+0` are rejected, toml_conv.c:96); floats need digits on both sides of `.`;
//!   dates and times are rejected ("Unexpected value").
//! * Numbers are stored by `init_number` from the raw token (toml_conv.c:216), which only
//!   understands plain JSON numbers: `0x10`, `1_000` or `+5` are accepted by the parser but read as
//!   nothing by the game ([`GValue::Unreadable`]).
//! * The text ends at the first NUL byte (`strlen`, toml_api.c:1432).
use std::fmt;

/// A value as the game holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum GValue {
    Str(String),
    Int(i128),
    Float(f64),
    Bool(bool),
    Array(Vec<GValue>),
    Table(GTable),
    /// A number the parser accepts but `init_number` cannot store (the game reads null).
    Unreadable(String),
}

/// A table in file order; value keys as written (see the module docs).
pub type GTable = Vec<(String, GValue)>;

/// The game's parse error: `line N: message`, as in `keeperfx.log`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameTomlError {
    pub line: usize,
    pub msg: String,
}

impl fmt::Display for GameTomlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

impl std::error::Error for GameTomlError {}

type R<T> = std::result::Result<T, GameTomlError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tk {
    Dot,
    Comma,
    Equal,
    LBrace,
    RBrace,
    Newline,
    LBracket,
    RBracket,
    Str,
}

#[derive(Clone, Copy, Debug)]
struct Token {
    tok: Tk,
    line: usize,
    ptr: usize,
    len: usize,
    eof: bool,
}

/// A table in file order with a key index (files hold thousands of `[thingN]` tables).
#[derive(Debug, Default)]
struct Tab {
    entries: Vec<(String, Node)>,
    index: std::collections::HashMap<String, usize>,
}

impl Tab {
    fn find(&self, key: &str) -> Option<usize> {
        self.index.get(key).copied()
    }
    fn push(&mut self, key: String, node: Node) -> usize {
        let i = self.entries.len();
        self.index.insert(key.clone(), i);
        self.entries.push((key, node));
        i
    }
}

#[derive(Debug)]
enum NVal {
    Table(Tab),
    Array(Vec<Node>),
    Scalar(GValue),
}

#[derive(Debug)]
struct Node {
    /// `IDX_KIND`: `'r'` root, 0 tables, the value type of scalars, the element kind of arrays.
    kind: u8,
    implicit: bool,
    val: NVal,
}

impl Node {
    fn table(kind: u8) -> Node {
        Node {
            kind,
            implicit: false,
            val: NVal::Table(Tab::default()),
        }
    }
    fn get(&self, key: &str) -> Option<usize> {
        match &self.val {
            NVal::Table(t) => t.find(key),
            _ => None,
        }
    }
    fn to_value(&self) -> GValue {
        match &self.val {
            NVal::Table(t) => GValue::Table(
                t.entries
                    .iter()
                    .map(|(k, n)| (k.clone(), n.to_value()))
                    .collect(),
            ),
            NVal::Array(a) => GValue::Array(a.iter().map(Node::to_value).collect()),
            NVal::Scalar(v) => v.clone(),
        }
    }
}

/// A step from the root to a node.
#[derive(Clone, Debug)]
enum Step {
    Key(usize),
    Index(usize),
}

struct Ctx<'a> {
    s: &'a [u8],
    tok: Token,
    root: Node,
    /// Path of the current table; `None` is the C's NULL table pointer.
    cur: Option<Vec<Step>>,
}

fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}

fn err<T>(line: usize, msg: impl Into<String>) -> R<T> {
    Err(GameTomlError {
        line,
        msg: msg.into(),
    })
}

impl<'a> Ctx<'a> {
    fn b(&self, i: usize) -> u8 {
        self.s.get(i).copied().unwrap_or(0)
    }

    fn node(&mut self, path: &Option<Vec<Step>>) -> Option<&mut Node> {
        let path = path.as_ref()?;
        let mut n = &mut self.root;
        for st in path {
            n = match (st, &mut n.val) {
                (Step::Key(i), NVal::Table(t)) => &mut t.entries.get_mut(*i)?.1,
                (Step::Index(i), NVal::Array(a)) => a.get_mut(*i)?,
                _ => return None,
            };
        }
        Some(n)
    }

    fn kind(&mut self, path: &Option<Vec<Step>>) -> u8 {
        self.node(path).map_or(0, |n| n.kind)
    }

    // ------------------------------------------------------------------ tokenizer

    fn set(&mut self, tok: Tk, line: usize, ptr: usize, len: usize) {
        self.tok = Token {
            tok,
            line,
            ptr,
            len,
            eof: false,
        };
    }

    fn next_token(&mut self, dotspecial: bool) -> R<()> {
        let mut line = self.tok.line;
        let mut p = self.tok.ptr;
        for _ in 0..self.tok.len {
            if self.b(p) == b'\n' {
                line += 1;
            }
            p += 1;
        }
        while p < self.s.len() {
            let c = self.s[p];
            if c == b'#' {
                p += 1;
                while p < self.s.len() && self.s[p] != b'\n' {
                    p += 1;
                }
                continue;
            }
            if dotspecial && c == b'.' {
                self.set(Tk::Dot, line, p, 1);
                return Ok(());
            }
            let t = match c {
                b',' => Some(Tk::Comma),
                b'=' => Some(Tk::Equal),
                b'{' => Some(Tk::LBrace),
                b'}' => Some(Tk::RBrace),
                b'[' => Some(Tk::LBracket),
                b']' => Some(Tk::RBracket),
                b'\n' => Some(Tk::Newline),
                b'\r' | b' ' | b'\t' => {
                    p += 1;
                    continue;
                }
                _ => None,
            };
            if let Some(t) = t {
                self.set(t, line, p, 1);
                return Ok(());
            }
            return self.scan_string(p, line, dotspecial);
        }
        self.tok = Token {
            tok: Tk::Newline,
            line,
            ptr: self.s.len(),
            len: 0,
            eof: true,
        };
        Ok(())
    }

    fn find(&self, from: usize, pat: &[u8]) -> Option<usize> {
        if from > self.s.len() {
            return None;
        }
        self.s[from..]
            .windows(pat.len())
            .position(|w| w == pat)
            .map(|i| i + from)
    }

    fn scan_date(&self, p: usize) -> bool {
        let digits = |at: usize, n: usize| (0..n).all(|k| is_digit(self.b(at + k)));
        digits(p, 4)
            && self.b(p + 4) == b'-'
            && digits(p + 5, 2)
            && self.b(p + 7) == b'-'
            && digits(p + 8, 2)
    }

    fn scan_time(&self, p: usize) -> bool {
        let digits = |at: usize, n: usize| (0..n).all(|k| is_digit(self.b(at + k)));
        digits(p, 2)
            && self.b(p + 2) == b':'
            && digits(p + 3, 2)
            && self.b(p + 5) == b':'
            && digits(p + 6, 2)
    }

    fn scan_string(&mut self, orig: usize, line: usize, dotspecial: bool) -> R<()> {
        let s3 = |c: u8| [c, c, c];
        if self.b(orig) == b'\'' && self.b(orig + 1) == b'\'' && self.b(orig + 2) == b'\'' {
            let Some(mut q) = self.find(orig + 3, &s3(b'\'')) else {
                return err(line, "unterminated triple-s-quote");
            };
            while self.b(q + 3) == b'\'' {
                q += 1;
            }
            self.set(Tk::Str, line, orig, q + 3 - orig);
            return Ok(());
        }
        if self.b(orig) == b'"' && self.b(orig + 1) == b'"' && self.b(orig + 2) == b'"' {
            let mut q = orig + 3;
            loop {
                let Some(f) = self.find(q, &s3(b'"')) else {
                    return err(line, "unterminated triple-d-quote");
                };
                q = f;
                if self.b(q - 1) == b'\\' {
                    q += 1;
                    continue;
                }
                while self.b(q + 3) == b'"' {
                    q += 1;
                }
                break;
            }
            let (mut hexreq, mut escape) = (0, false);
            let mut p = orig + 3;
            while p < q {
                let c = self.b(p);
                if escape {
                    escape = false;
                    if b"btnfr\"\\".contains(&c) {
                    } else if c == b'u' {
                        hexreq = 4;
                    } else if c == b'U' {
                        hexreq = 8;
                    } else if self.b(self.skip_set(p, b" \t\r")) == b'\n' {
                    } else {
                        return err(line, "bad escape char");
                    }
                } else if hexreq > 0 {
                    hexreq -= 1;
                    if !b"0123456789ABCDEF".contains(&c) {
                        return err(line, "expect hex char");
                    }
                } else if c == b'\\' {
                    escape = true;
                }
                p += 1;
            }
            if escape {
                return err(line, "expect an escape char");
            }
            if hexreq > 0 {
                return err(line, "expected more hex char");
            }
            self.set(Tk::Str, line, orig, q + 3 - orig);
            return Ok(());
        }
        if self.b(orig) == b'\'' {
            let mut p = orig + 1;
            while self.b(p) != 0 && self.b(p) != b'\n' && self.b(p) != b'\'' {
                p += 1;
            }
            if self.b(p) != b'\'' {
                return err(line, "unterminated s-quote");
            }
            self.set(Tk::Str, line, orig, p + 1 - orig);
            return Ok(());
        }
        if self.b(orig) == b'"' {
            let (mut hexreq, mut escape) = (0, false);
            let mut p = orig + 1;
            while self.b(p) != 0 {
                let c = self.b(p);
                if escape {
                    escape = false;
                    if b"btnfr\"\\".contains(&c) {
                    } else if c == b'u' {
                        hexreq = 4;
                    } else if c == b'U' {
                        hexreq = 8;
                    } else {
                        return err(line, "bad escape char");
                    }
                } else if hexreq > 0 {
                    hexreq -= 1;
                    if !b"0123456789ABCDEF".contains(&c) {
                        return err(line, "expect hex char");
                    }
                } else if c == b'\\' {
                    escape = true;
                } else if c == b'\'' {
                    if self.b(p + 1) == b'\'' && self.b(p + 2) == b'\'' {
                        return err(line, "triple-s-quote inside string lit");
                    }
                } else if c == b'\n' || c == b'"' {
                    break;
                }
                p += 1;
            }
            if self.b(p) != b'"' {
                return err(line, "unterminated quote");
            }
            self.set(Tk::Str, line, orig, p + 1 - orig);
            return Ok(());
        }
        if self.scan_date(orig) || self.scan_time(orig) {
            let mut p = self.skip_set(orig, b"0123456789.:+-Tt Zz");
            while p > orig && self.b(p - 1) == b' ' {
                p -= 1;
            }
            self.set(Tk::Str, line, orig, p - orig);
            return Ok(());
        }
        let mut p = orig;
        while p < self.s.len() && self.s[p] != b'\n' {
            let c = self.s[p];
            if c == b'.' && dotspecial {
                break;
            }
            if c.is_ascii_alphabetic() || b"0123456789+-_.".contains(&c) {
                p += 1;
                continue;
            }
            break;
        }
        self.set(Tk::Str, line, orig, p - orig);
        Ok(())
    }

    fn skip_set(&self, mut p: usize, set: &[u8]) -> usize {
        while p < self.s.len() && set.contains(&self.s[p]) {
            p += 1;
        }
        p
    }

    fn eat(&mut self, t: Tk, dotspecial: bool) -> R<()> {
        if self.tok.tok != t {
            return err(self.tok.line, "internal error");
        }
        self.next_token(dotspecial)
    }

    // ------------------------------------------------------------------ strings and keys

    /// `norm_basic_str` (toml_api.c:426) over `s[from..to]`.
    fn norm_basic(
        &self,
        from: usize,
        to: usize,
        multiline: bool,
    ) -> std::result::Result<Vec<u8>, String> {
        let mut out = vec![];
        let mut p = from;
        while p < to {
            let c = self.b(p);
            p += 1;
            if c != b'\\' {
                if (c <= 0x08 || (0x0a..=0x1f).contains(&c) || c == 0x7f)
                    && !(multiline && (c == b'\r' || c == b'\n'))
                {
                    return Err(format!("invalid char U+{c:04x}"));
                }
                out.push(c);
                continue;
            }
            if p >= to {
                return Err("last backslash is invalid".into());
            }
            if multiline && self.b(self.skip_set(p, b" \t\r")) == b'\n' {
                p = self.skip_set(p, b" \t\r\n");
                continue;
            }
            let e = self.b(p);
            p += 1;
            let ch = match e {
                b'u' | b'U' => {
                    let n = if e == b'u' { 4 } else { 8 };
                    let mut ucs: u32 = 0;
                    for _ in 0..n {
                        if p >= to {
                            return Err(format!("\\{} expects {n} hex chars", e as char));
                        }
                        let h = self.b(p);
                        p += 1;
                        let v = match h {
                            b'0'..=b'9' => h - b'0',
                            b'A'..=b'F' => h - b'A' + 10,
                            _ => return Err("invalid hex chars for \\u or \\U".into()),
                        };
                        ucs = ucs.wrapping_mul(16).wrapping_add(v as u32);
                    }
                    let Some(ch) = char::from_u32(ucs) else {
                        return Err("illegal ucs code in \\u or \\U".into());
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    continue;
                }
                b'b' => 8,
                b't' => b'\t',
                b'n' => b'\n',
                b'f' => 12,
                b'r' => b'\r',
                b'"' => b'"',
                b'\\' => b'\\',
                _ => return Err(format!("illegal escape char \\{}", e as char)),
            };
            out.push(ch);
        }
        Ok(out)
    }

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap_or_else(|e| crate::cfg::latin1_decode(e.as_bytes()))
    }

    fn raw(&self, t: Token) -> String {
        Self::text(self.s[t.ptr..t.ptr + t.len].to_vec())
    }

    /// `normalize_key` (toml_api.c:575).
    fn normalize_key(&self, t: Token) -> R<String> {
        let (sp, sq) = (t.ptr, t.ptr + t.len);
        let ch = self.b(sp);
        if ch == b'\'' || ch == b'"' {
            let multiline = self.b(sp + 1) == ch && self.b(sp + 2) == ch;
            let (a, z) = if multiline {
                (sp + 3, sq.saturating_sub(3))
            } else {
                (sp + 1, sq.saturating_sub(1))
            };
            let z = z.max(a);
            let key = if ch == b'\'' {
                self.s[a..z].to_vec()
            } else {
                match self.norm_basic(a, z, multiline) {
                    Ok(k) => k,
                    Err(m) => return err(t.line, m),
                }
            };
            if key.contains(&b'\n') {
                return err(t.line, "bad key");
            }
            return Ok(Self::text(key));
        }
        for &c in &self.s[sp..sq] {
            if !(c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
                return err(t.line, "bad key");
            }
        }
        Ok(Self::text(self.s[sp..sq].to_vec()))
    }

    // ------------------------------------------------------------------ tree

    /// `create_table_in_table`: returns the path of the table.
    fn create_table_in_table(&mut self, tab: &Option<Vec<Step>>, keytok: Token) -> R<Vec<Step>> {
        self.create_table(tab, keytok).map(|(p, _)| p)
    }

    /// [`Ctx::create_table_in_table`], also telling whether the table is new.
    fn create_table(&mut self, tab: &Option<Vec<Step>>, keytok: Token) -> R<(Vec<Step>, bool)> {
        let key = self.normalize_key(keytok)?;
        let line = keytok.line;
        let Some(n) = self.node(tab) else {
            return err(line, "out of memory");
        };
        let NVal::Table(t) = &mut n.val else {
            return err(line, "out of memory");
        };
        let mut path = tab.clone().unwrap_or_default();
        if let Some(i) = t.find(&key) {
            if t.entries[i].1.implicit {
                t.entries[i].1.implicit = false;
                path.push(Step::Key(i));
                return Ok((path, false));
            }
            return err(line, "duplicate key");
        }
        let i = t.push(key, Node::table(0));
        path.push(Step::Key(i));
        Ok((path, true))
    }

    fn create_keyarray_in_table(
        &mut self,
        tab: &Option<Vec<Step>>,
        keytok: Token,
        kind: u8,
    ) -> R<Vec<Step>> {
        let key = self.normalize_key(keytok)?;
        let line = keytok.line;
        let Some(n) = self.node(tab) else {
            return err(line, "out of memory");
        };
        let NVal::Table(t) = &mut n.val else {
            return err(line, "out of memory");
        };
        if t.find(&key).is_some() {
            return err(line, "duplicate key");
        }
        let i = t.push(
            key,
            Node {
                kind,
                implicit: false,
                val: NVal::Array(vec![]),
            },
        );
        let mut path = tab.clone().unwrap_or_default();
        path.push(Step::Key(i));
        Ok(path)
    }

    /// `create_value_in_table` (raw key, toml_api.c:768).
    fn create_value_in_table(
        &mut self,
        tab: &Option<Vec<Step>>,
        keytok: Token,
        value: Node,
    ) -> R<()> {
        let key = self.raw(keytok);
        let line = keytok.line;
        let Some(n) = self.node(tab) else {
            return err(line, "out of memory");
        };
        let NVal::Table(t) = &mut n.val else {
            return err(line, "out of memory");
        };
        if t.find(&key).is_some() {
            return err(line, "out of memory");
        }
        t.push(key, value);
        Ok(())
    }

    fn append(&mut self, arr: &Option<Vec<Step>>, node: Node) -> R<Vec<Step>> {
        let line = self.tok.line;
        let Some(n) = self.node(arr) else {
            return err(line, "out of memory");
        };
        let NVal::Array(a) = &mut n.val else {
            return err(line, "out of memory");
        };
        a.push(node);
        let mut path = arr.clone().unwrap_or_default();
        path.push(Step::Index(a.len() - 1));
        Ok(path)
    }

    // ------------------------------------------------------------------ values

    fn parse_simple(&mut self, t: Token) -> R<Node> {
        let (start, len) = (t.ptr, t.len);
        let c0 = self.b(start);
        if c0 == b'\'' || c0 == b'"' {
            let c = self.b(start);
            let multiline = len >= 6
                && self.b(start + 1) == c
                && self.b(start + 2) == c
                && self.b(start + len - 1) == c
                && self.b(start + len - 2) == c
                && self.b(start + len - 3) == c;
            let (a, z) = if multiline {
                (start + 3, start + len - 3)
            } else {
                (start + 1, start + len.max(2) - 1)
            };
            let s = if c == b'"' {
                match self.norm_basic(a, z, multiline) {
                    Ok(s) => s,
                    Err(m) => return err(t.line, m),
                }
            } else {
                self.s[a..z].to_vec()
            };
            return Ok(Node {
                kind: b's',
                implicit: false,
                val: NVal::Scalar(GValue::Str(Self::text(s))),
            });
        }
        let tok = &self.s[start..start + len];
        if tok.starts_with(b"true") || tok.starts_with(b"false") {
            return Ok(Node {
                kind: b'b',
                implicit: false,
                val: NVal::Scalar(GValue::Bool(tok.starts_with(b"true"))),
            });
        }
        if let Some(i) = self.is_int(start, len) {
            let v = if json_number(tok) {
                GValue::Int(i)
            } else {
                GValue::Unreadable(Self::text(tok.to_vec()))
            };
            return Ok(Node {
                kind: b'i',
                implicit: false,
                val: NVal::Scalar(v),
            });
        }
        if let Some(f) = self.is_double(start, len) {
            let v = if json_number(tok) {
                GValue::Float(f)
            } else {
                GValue::Unreadable(Self::text(tok.to_vec()))
            };
            return Ok(Node {
                kind: b'd',
                implicit: false,
                val: NVal::Scalar(v),
            });
        }
        err(self.tok.line, "Unexpected value")
    }

    /// `toml_is_i` (toml_conv.c:73).
    fn is_int(&self, start: usize, len: usize) -> Option<i128> {
        let e = start + len;
        let mut s = start;
        let mut buf: Vec<u8> = vec![];
        if self.b(s) == b'+' || self.b(s) == b'-' {
            buf.push(self.b(s));
            s += 1;
        }
        if self.b(s) == b'_' {
            return None;
        }
        let mut base = 10;
        if self.b(s) == b'0' {
            if len == 1 {
                return Some(0);
            }
            match self.b(s + 1) {
                b'x' => {
                    base = 16;
                    s += 2;
                }
                b'o' => {
                    base = 8;
                    s += 2;
                }
                b'b' => {
                    base = 2;
                    s += 2;
                }
                0 => return Some(0),
                _ => return None,
            }
        }
        while s < e && buf.len() < 100 {
            let c = self.b(s);
            s += 1;
            if c == b'_' {
                if self.b(s) == b'_' || self.b(s) == 0 {
                    return None;
                }
                continue;
            }
            buf.push(c);
        }
        if buf.len() >= 100 {
            return None;
        }
        strtoll(&buf, base)
    }

    /// `toml_is_d` (toml_conv.c:136).
    fn is_double(&self, start: usize, len: usize) -> Option<f64> {
        let e = start + len;
        let mut s = start;
        let mut buf: Vec<u8> = vec![];
        if self.b(s) == b'+' || self.b(s) == b'-' {
            buf.push(self.b(s));
            s += 1;
        }
        if self.b(s) == b'_' {
            return None;
        }
        if let Some(dot) = self.find(s, b".") {
            if dot < e && (dot == s || !is_digit(self.b(dot - 1)) || !is_digit(self.b(dot + 1))) {
                return None;
            }
        }
        if self.b(s) == b'0' && self.b(s + 1) != 0 && !b"eE.".contains(&self.b(s + 1)) {
            return None;
        }
        while s < e && buf.len() < 100 {
            let c = self.b(s);
            s += 1;
            if c == b'_' {
                if self.b(s) == b'_' || self.b(s) == 0 {
                    return None;
                }
                continue;
            }
            buf.push(c);
        }
        if s < e || buf.len() >= 100 {
            return None;
        }
        strtod(&buf)
    }

    // ------------------------------------------------------------------ grammar

    fn skip_newlines(&mut self, dotspecial: bool) -> R<()> {
        while self.tok.tok == Tk::Newline {
            self.next_token(dotspecial)?;
            if self.tok.eof {
                break;
            }
        }
        Ok(())
    }

    fn parse_keyval(&mut self, tab: Option<Vec<Step>>) -> R<()> {
        let key = self.tok;
        self.eat(Tk::Str, true)?;
        if self.tok.tok == Tk::Dot {
            let name = self.normalize_key(key)?;
            let found = self.node(&tab).and_then(|n| n.get(&name));
            let sub = match found {
                Some(i) => {
                    let mut p = tab.clone().unwrap_or_default();
                    p.push(Step::Key(i));
                    p
                }
                None => self.create_table_in_table(&tab, key)?,
            };
            self.next_token(true)?;
            return self.parse_keyval(Some(sub));
        }
        if self.tok.tok != Tk::Equal {
            return err(self.tok.line, "missing =");
        }
        self.next_token(false)?;
        match self.tok.tok {
            Tk::Str => {
                let v = self.parse_simple(self.tok)?;
                self.create_value_in_table(&tab, key, v)?;
                self.next_token(true)
            }
            Tk::LBracket => match self.create_keyarray_in_table(&tab, key, 0) {
                Ok(arr) => self.parse_array(Some(arr)),
                // the C parses into a NULL array: only `[]` gets through (see the module docs)
                Err(e) => self.parse_array(None).map_err(|_| e),
            },
            Tk::LBrace => {
                let t = self.create_table_in_table(&tab, key)?;
                self.parse_inline_table(Some(t))
            }
            _ => err(self.tok.line, "syntax error"),
        }
    }

    fn parse_inline_table(&mut self, tab: Option<Vec<Step>>) -> R<()> {
        self.eat(Tk::LBrace, true)?;
        loop {
            if self.tok.tok == Tk::Newline {
                return err(self.tok.line, "newline not allowed in inline table");
            }
            if self.tok.tok == Tk::RBrace {
                break;
            }
            if self.tok.tok != Tk::Str {
                return err(self.tok.line, "expect a string");
            }
            self.parse_keyval(tab.clone())?;
            if self.tok.tok == Tk::Newline {
                return err(self.tok.line, "newline not allowed in inline table");
            }
            if self.tok.tok == Tk::Comma {
                self.eat(Tk::Comma, true)?;
                continue;
            }
            break;
        }
        self.eat(Tk::RBrace, true)
    }

    fn set_kind(&mut self, arr: &Option<Vec<Step>>, k: u8) {
        if let Some(n) = self.node(arr) {
            n.kind = k;
        }
    }

    fn arr_len(&mut self, arr: &Option<Vec<Step>>) -> usize {
        match self.node(arr).map(|n| &n.val) {
            Some(NVal::Array(a)) => a.len(),
            _ => 0,
        }
    }

    fn parse_array(&mut self, arr: Option<Vec<Step>>) -> R<()> {
        self.eat(Tk::LBracket, false)?;
        loop {
            self.skip_newlines(false)?;
            if self.tok.tok == Tk::RBracket {
                break;
            }
            match self.tok.tok {
                Tk::Str => {
                    let kind = self.kind(&arr);
                    let v = self.parse_simple(self.tok)?;
                    let vt = v.kind;
                    self.append(&arr, v)?;
                    if self.arr_len(&arr) == 1 {
                        self.set_kind(&arr, vt);
                    } else if kind != vt {
                        self.set_kind(&arr, b'm');
                    }
                    self.eat(Tk::Str, false)?;
                }
                Tk::LBracket => {
                    let kind = self.kind(&arr);
                    if kind == 0 {
                        self.set_kind(&arr, b'a');
                    } else if kind != b'a' {
                        self.set_kind(&arr, b'm');
                    }
                    let sub = self.append(
                        &arr,
                        Node {
                            kind: 0,
                            implicit: false,
                            val: NVal::Array(vec![]),
                        },
                    )?;
                    self.parse_array(Some(sub))?;
                }
                Tk::LBrace => {
                    let kind = self.kind(&arr);
                    if kind == 0 {
                        self.set_kind(&arr, b'{');
                    } else if kind != b'{' {
                        self.set_kind(&arr, b'm');
                    }
                    let sub = self.append(&arr, Node::table(0))?;
                    self.parse_inline_table(Some(sub))?;
                }
                _ => return err(self.tok.line, "syntax error"),
            }
            self.skip_newlines(false)?;
            if self.tok.tok == Tk::Comma {
                self.eat(Tk::Comma, false)?;
                continue;
            }
            break;
        }
        self.eat(Tk::RBracket, true)
    }

    fn parse_select(&mut self) -> R<()> {
        let line = self.tok.line;
        let llb = self.tok.ptr + 1 < self.s.len() && self.s[self.tok.ptr + 1] == b'[';
        self.eat(Tk::LBracket, true)?;
        if llb {
            self.eat(Tk::LBracket, true)?;
        }
        // fill_tabpath
        let mut path: Vec<Token> = vec![];
        loop {
            if path.len() >= 10 {
                return err(line, "table path is too deep; max allowed is 10.");
            }
            if self.tok.tok != Tk::Str {
                return err(line, "invalid or missing key");
            }
            self.normalize_key(self.tok)?;
            path.push(self.tok);
            self.next_token(true)?;
            if self.tok.tok == Tk::RBracket {
                break;
            }
            if self.tok.tok != Tk::Dot {
                return err(line, "invalid key");
            }
            self.next_token(true)?;
        }
        let z = path.pop().expect("one key at least");
        // walk_tabpath
        let mut cur: Option<Vec<Step>> = Some(vec![]);
        for t in &path {
            let kind = self.kind(&cur);
            cur = match kind {
                b'r' | b'{' => {
                    let key = self.normalize_key(*t)?;
                    let i = self.node(&cur).and_then(|n| n.get(&key));
                    match (cur, i) {
                        (Some(mut p), Some(i)) => {
                            p.push(Step::Key(i));
                            Some(p)
                        }
                        _ => None,
                    }
                }
                b'a' => return err(t.line, "internal error"),
                _ => {
                    // tables created by the walk are implicit; an existing implicit one was
                    // just made explicit by create_table_in_table (toml_api.c:683)
                    let (p, created) = self.create_table(&cur, *t)?;
                    if created {
                        if let Some(n) = self.node(&Some(p.clone())) {
                            n.implicit = true;
                        }
                    }
                    Some(p)
                }
            };
        }
        if !llb {
            let t = self.create_table_in_table(&cur, z)?;
            self.cur = Some(t);
        } else {
            let key = self.normalize_key(z)?;
            let found = self.node(&cur).and_then(|n| n.get(&key));
            let arr = match found {
                Some(i) => {
                    let mut p = cur.clone().unwrap_or_default();
                    p.push(Step::Key(i));
                    p
                }
                None => self.create_keyarray_in_table(&cur, z, b'{')?,
            };
            if self.kind(&Some(arr.clone())) != b'{' {
                return err(z.line, "array mismatch");
            }
            let t = self.append(&Some(arr), Node::table(0))?;
            self.cur = Some(t);
        }
        if self.tok.tok != Tk::RBracket {
            return err(self.tok.line, "expects ]");
        }
        if llb {
            if !(self.tok.ptr + 1 < self.s.len() && self.s[self.tok.ptr + 1] == b']') {
                return err(self.tok.line, "expects ]]");
            }
            self.eat(Tk::RBracket, true)?;
        }
        self.eat(Tk::RBracket, true)?;
        if self.tok.tok != Tk::Newline {
            return err(self.tok.line, "extra chars after ] or ]]");
        }
        Ok(())
    }
}

/// Plain JSON number grammar (what `init_number` / `json_analyze_number` read).
fn json_number(t: &[u8]) -> bool {
    let mut i = 0;
    let n = t.len();
    if i < n && t[i] == b'-' {
        i += 1;
    }
    if i >= n {
        return false;
    }
    if t[i] == b'0' {
        i += 1;
    } else if t[i].is_ascii_digit() {
        while i < n && t[i].is_ascii_digit() {
            i += 1;
        }
    } else {
        return false;
    }
    if i < n && t[i] == b'.' {
        i += 1;
        let s = i;
        while i < n && t[i].is_ascii_digit() {
            i += 1;
        }
        if i == s {
            return false;
        }
    }
    if i < n && (t[i] == b'e' || t[i] == b'E') {
        i += 1;
        if i < n && (t[i] == b'+' || t[i] == b'-') {
            i += 1;
        }
        let s = i;
        while i < n && t[i].is_ascii_digit() {
            i += 1;
        }
        if i == s {
            return false;
        }
    }
    i == n
}

/// `strtoll(buf, &end, base)` with base 0 meaning decimal here (leading zeros were rejected
/// before), requiring the whole buffer to be consumed without overflow.
fn strtoll(buf: &[u8], base: u32) -> Option<i128> {
    let mut i = 0;
    let neg = match buf.first() {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    if base == 16
        && buf.len() > i + 2
        && buf[i] == b'0'
        && (buf[i + 1] | 0x20) == b'x'
        && (buf[i + 2] as char).is_ascii_hexdigit()
    {
        i += 2;
    }
    if i >= buf.len() {
        // nothing converted: strtoll returns 0 with the end at the start, which is the NUL
        return if buf.is_empty() { Some(0) } else { None };
    }
    let mut v: i128 = 0;
    for &c in &buf[i..] {
        let d = (c as char).to_digit(base)?;
        v = v * base as i128 + d as i128;
        if v > i64::MAX as i128 + 1 {
            return None;
        }
    }
    let v = if neg { -v } else { v };
    if v > i64::MAX as i128 || v < i64::MIN as i128 {
        return None;
    }
    Some(v)
}

/// `strtod` requiring the whole buffer and no range error.
fn strtod(buf: &[u8]) -> Option<f64> {
    let s = std::str::from_utf8(buf).ok()?;
    let f: f64 = s.parse().ok()?;
    let lower = s.to_ascii_lowercase();
    let literal_inf = lower.contains("inf");
    if f.is_infinite() && !literal_inf {
        return None;
    }
    if f == 0.0 && lower.bytes().any(|c| (b'1'..=b'9').contains(&c)) && !lower.contains('e') {
        return Some(f);
    }
    if f == 0.0
        && lower
            .split(['e'])
            .next()
            .is_some_and(|m| m.bytes().any(|c| (b'1'..=b'9').contains(&c)))
    {
        return None; // underflow
    }
    Some(f)
}

/// Parse like the game. The text ends at the first NUL.
pub fn parse(text: &str) -> std::result::Result<GTable, GameTomlError> {
    let bytes = text.as_bytes();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let mut c = Ctx {
        s: &bytes[..end],
        tok: Token {
            tok: Tk::Newline,
            line: 1,
            ptr: 0,
            len: 0,
            eof: false,
        },
        root: Node::table(b'r'),
        cur: Some(vec![]),
    };
    loop {
        let t = c.tok;
        if t.eof {
            break;
        }
        match t.tok {
            Tk::Newline => c.next_token(true)?,
            Tk::Str => {
                let cur = c.cur.clone();
                c.parse_keyval(cur)?;
                if c.tok.tok != Tk::Newline {
                    return err(c.tok.line, "extra chars after value");
                }
                c.eat(Tk::Newline, true)?;
            }
            Tk::LBracket => c.parse_select()?,
            _ => return err(t.line, "syntax error"),
        }
    }
    match c.root.to_value() {
        GValue::Table(t) => Ok(t),
        _ => unreachable!("the root is a table"),
    }
}

/// Whether the game's reader accepts `text`.
pub fn check(text: &str) -> std::result::Result<(), GameTomlError> {
    parse(text).map(|_| ())
}

/// The key a raw `key = value` key stands for in TOML (quotes removed, escapes resolved).
pub fn unquote_key(raw: &str) -> String {
    let b = raw.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') {
        if let Ok(t) = format!("{raw} = 0").parse::<toml::Table>() {
            if let Some(k) = t.keys().next() {
                return k.clone();
            }
        }
    }
    raw.to_string()
}

/// Convert to a `toml::Table` with every quoted key unquoted (for comparisons with a TOML
/// reader). Fails on integers beyond `i64` and on [`GValue::Unreadable`] numbers.
pub fn to_toml(t: &GTable) -> std::result::Result<toml::Table, String> {
    convert(t, false)
}

/// Convert keeping what the game sees: a quoted `key = value` key whose unquoted form is a bare
/// key stays quoted (`"ThingType"` is not `ThingType` for the game); other quoted keys are
/// unquoted, so writing them back with [`crate::toml_emit::key`] gives the same key.
pub fn to_toml_like_game(t: &GTable) -> std::result::Result<toml::Table, String> {
    convert(t, true)
}

fn convert(t: &GTable, keep_raw: bool) -> std::result::Result<toml::Table, String> {
    fn conv(v: &GValue, keep_raw: bool) -> std::result::Result<toml::Value, String> {
        Ok(match v {
            GValue::Str(s) => toml::Value::String(s.clone()),
            GValue::Int(i) => toml::Value::Integer(
                i64::try_from(*i).map_err(|_| format!("integer {i} out of range"))?,
            ),
            GValue::Float(f) => toml::Value::Float(*f),
            GValue::Bool(b) => toml::Value::Boolean(*b),
            GValue::Array(a) => toml::Value::Array(
                a.iter()
                    .map(|x| conv(x, keep_raw))
                    .collect::<std::result::Result<_, _>>()?,
            ),
            GValue::Table(t) => toml::Value::Table(convert(t, keep_raw)?),
            GValue::Unreadable(s) => return Err(format!("the game cannot read the number {s}")),
        })
    }
    let mut out = toml::Table::new();
    for (k, v) in t {
        let u = unquote_key(k);
        let key = if keep_raw && u != *k && crate::toml_emit::is_bare_key(&u) {
            k.clone()
        } else {
            u
        };
        out.insert(key, conv(v, keep_raw)?);
    }
    Ok(out)
}

/// Whether the game reads `text` as the same tree as a TOML reader does (`toml` crate parse,
/// keys compared unquoted). `Err` holds the first difference.
pub fn same_as_toml(text: &str) -> std::result::Result<(), String> {
    let g = parse(text).map_err(|e| format!("the game rejects it: {e}"))?;
    let t: toml::Table = text.parse().map_err(|e| format!("not TOML: {e}"))?;
    let g = to_toml(&g)?;
    if g == t {
        Ok(())
    } else {
        Err(format!("the game reads it differently: {g:?} vs {t:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> bool {
        parse(s).is_ok()
    }

    /// Header sequences, from the walk in `walk_tabpath` / `create_table_in_table`.
    #[test]
    fn table_headers_like_the_game() {
        assert!(ok("[a]\nx = 1\n[a.b]\ny = 2\n"));
        assert!(!ok("[a.b]\ny = 2\n"), "parent missing");
        assert!(ok("[a]\n[a.b.c]\nw = 1\n"), "middle created implicitly");
        assert!(!ok("[a]\n[a.b]\n[a.b.c]\nw = 1\n"), "explicit middle");
        assert!(
            ok("[a]\n[a.b.c]\n[a.b]\nx = 1\n"),
            "implicit middle made explicit later"
        );
        assert!(
            ok("[a]\n[a.b.c]\n[a.b.d]\n"),
            "second walk makes b explicit"
        );
        assert!(
            !ok("[a]\n[a.b.c]\n[a.b.d]\n[a.b.e]\n"),
            "third walk through explicit b"
        );
        assert!(!ok("[a]\n[a]\n"), "duplicate table");
        assert!(ok("[[a]]\nx = 1\n[[a]]\nx = 2\n"));
        assert!(
            !ok("[[a]]\nx = 1\n[a.b]\n"),
            "no headers into arrays of tables"
        );
        assert!(ok("[a]\n[[a.b]]\nx = 1\n[[a.b]]\nx = 2\n"));
        assert!(ok("[a]\nb.c.d = 1\nb.c.e = 2\n"), "dotted keys");
    }

    #[test]
    fn values_like_the_game() {
        assert!(ok(
            "a = 1\nb = -5\nc = 1.5\nd = true\ne = \"x\"\nf = 'y'\ng = [1, 2]\nh = { i = 1 }\n"
        ));
        assert!(!ok("a = -0\n"), "toml_is_i / toml_is_d reject -0");
        assert!(!ok("a = 1979-05-27\n"), "dates");
        assert!(!ok("a = \"\\u00e9\"\n"), "lower-case hex");
        assert!(ok("a = \"\\u00E9\"\n"));
        assert!(
            !ok("a = \"x'''y\"\n"),
            "triple single quote inside a basic string"
        );
        assert!(!ok("a = 1\na = 2\n"), "duplicate key");
        assert!(
            ok("a = 1\na = []\n"),
            "duplicate key with an empty array slips through"
        );
        assert!(
            ok("\"a\" = 1\na = 2\n"),
            "a quoted value key keeps its quotes"
        );
        assert_eq!(parse("\"a b\" = 1\n").unwrap()[0].0, "\"a b\"");
        assert_eq!(
            parse("s = \"\"\"\nx\"\"\"\n").unwrap()[0].1,
            GValue::Str("\nx".into()),
            "leading newline kept"
        );
        assert_eq!(
            parse("v = 0x10\n").unwrap()[0].1,
            GValue::Unreadable("0x10".into())
        );
        assert!(ok("t = {a = 1,}\n"), "trailing comma in an inline table");
        assert!(ok("a = 1 # comment\n# c\n[t] # c\n"));
        assert_eq!(
            parse("a = 1\0garbage").unwrap().len(),
            1,
            "text ends at NUL"
        );
    }
}
