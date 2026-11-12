//! LuaLS annotation parsing (`---@class`, `---@param`, `---@alias`, ...) and the type grammar.
//!
//! Annotation blocks are runs of consecutive `---` comment lines. The tags understood are the ones
//! the shipped KeeperFX library uses plus the common LuaLS ones; unknown tags are kept as
//! [`Tag::Other`] so nothing is lost.
use super::lexer::{TokKind, Token};
use crate::diag::Span;
use std::fmt;

/// A LuaLS type expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Name(String),
    /// `"LITERAL"`
    Str(String),
    Num(String),
    Bool(bool),
    Union(Vec<Ty>),
    Array(Box<Ty>),
    Optional(Box<Ty>),
    Generic(String, Vec<Ty>),
    Fun(Box<FunTy>),
    Table(Vec<(String, Ty)>),
    /// `T...`
    Vararg(Box<Ty>),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct FunTy {
    /// (name, type, optional)
    pub params: Vec<(String, Ty, bool)>,
    pub returns: Vec<Ty>,
}

impl Ty {
    pub fn any() -> Ty {
        Ty::Name("any".into())
    }
    /// Flatten nested unions and optionals into the member list (`T?` gives `T`, `nil`).
    pub fn members(&self) -> Vec<Ty> {
        match self {
            Ty::Union(v) => v.iter().flat_map(|t| t.members()).collect(),
            Ty::Optional(t) => {
                let mut m = t.members();
                m.push(Ty::Name("nil".into()));
                m
            }
            t => vec![t.clone()],
        }
    }
    pub fn accepts_nil(&self) -> bool {
        self.members()
            .iter()
            .any(|m| matches!(m, Ty::Name(n) if n == "nil" || n == "any" || n == "unknown"))
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Name(n) => write!(f, "{n}"),
            Ty::Str(s) => write!(f, "\"{s}\""),
            Ty::Num(n) => write!(f, "{n}"),
            Ty::Bool(b) => write!(f, "{b}"),
            Ty::Union(v) => {
                for (i, t) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, "|")?;
                    }
                    write!(f, "{t}")?;
                }
                Ok(())
            }
            Ty::Array(t) => match **t {
                Ty::Union(_) | Ty::Fun(_) => write!(f, "({t})[]"),
                _ => write!(f, "{t}[]"),
            },
            Ty::Optional(t) => match **t {
                Ty::Union(_) | Ty::Fun(_) => write!(f, "({t})?"),
                _ => write!(f, "{t}?"),
            },
            Ty::Generic(n, a) => {
                write!(f, "{n}<")?;
                for (i, t) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, ">")
            }
            Ty::Fun(ft) => {
                write!(f, "fun(")?;
                for (i, (n, t, o)) in ft.params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{n}{}: {t}", if *o { "?" } else { "" })?;
                }
                write!(f, ")")?;
                if !ft.returns.is_empty() {
                    write!(f, ": ")?;
                    for (i, t) in ft.returns.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{t}")?;
                    }
                }
                Ok(())
            }
            Ty::Table(fields) => {
                write!(f, "{{ ")?;
                for (i, (n, t)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{n}: {t}")?;
                }
                write!(f, " }}")
            }
            Ty::Vararg(t) => write!(f, "{t}..."),
        }
    }
}

// ---- type parser --------------------------------------------------------------------------

struct TyParser<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
    depth: usize,
}

fn name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'$' | b'-' | b'*') || c >= 0x80
}

impl<'a> TyParser<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] == b' ' || self.s[self.i] == b'\t') {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn union(&mut self) -> Option<Ty> {
        if self.depth > 24 {
            return None;
        }
        self.depth += 1;
        let r = self.union_inner();
        self.depth -= 1;
        r
    }
    fn union_inner(&mut self) -> Option<Ty> {
        let first = self.postfix()?;
        let mut v = vec![first];
        loop {
            let save = self.i;
            self.ws();
            if self.peek() == Some(b'|') {
                self.i += 1;
                self.ws();
                match self.postfix() {
                    Some(t) => v.push(t),
                    None => {
                        self.i = save;
                        break;
                    }
                }
            } else {
                self.i = save;
                break;
            }
        }
        Some(if v.len() == 1 {
            v.pop().unwrap()
        } else {
            Ty::Union(v)
        })
    }
    fn postfix(&mut self) -> Option<Ty> {
        let mut t = self.atom()?;
        loop {
            if self.s[self.i..].starts_with(b"[]") {
                self.i += 2;
                t = Ty::Array(Box::new(t));
            } else if self.peek() == Some(b'?') {
                self.i += 1;
                t = Ty::Optional(Box::new(t));
            } else if self.s[self.i..].starts_with(b"...") {
                self.i += 3;
                t = Ty::Vararg(Box::new(t));
            } else {
                break;
            }
        }
        Some(t)
    }
    fn atom(&mut self) -> Option<Ty> {
        self.ws();
        let c = self.peek()?;
        match c {
            b'"' | b'\'' => {
                let q = c;
                let st = self.i + 1;
                let mut j = st;
                while j < self.s.len() && self.s[j] != q {
                    j += 1;
                }
                if j >= self.s.len() {
                    return None;
                }
                self.i = j + 1;
                Some(Ty::Str(self.src[st..j].to_string()))
            }
            b'`' => {
                let st = self.i + 1;
                let j = self.src[st..].find('`')? + st;
                self.i = j + 1;
                Some(Ty::Name(self.src[st..j].to_string()))
            }
            b'(' => {
                self.i += 1;
                let t = self.union()?;
                self.ws();
                if self.peek() == Some(b')') {
                    self.i += 1;
                    Some(t)
                } else {
                    None
                }
            }
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                loop {
                    self.ws();
                    match self.peek()? {
                        b'}' => {
                            self.i += 1;
                            break;
                        }
                        b',' | b';' => self.i += 1,
                        _ => {
                            let st = self.i;
                            while self.peek().map_or(false, name_char) {
                                self.i += 1;
                            }
                            let key = self.src[st..self.i].to_string();
                            if key.is_empty() {
                                return None;
                            }
                            if self.peek() == Some(b'?') {
                                self.i += 1;
                            }
                            self.ws();
                            if self.peek() != Some(b':') {
                                return None;
                            }
                            self.i += 1;
                            let t = self.union()?;
                            fields.push((key, t));
                        }
                    }
                }
                Some(Ty::Table(fields))
            }
            b'0'..=b'9' | b'-'
                if c != b'-' || self.s.get(self.i + 1).map_or(false, |d| d.is_ascii_digit()) =>
            {
                let st = self.i;
                self.i += 1;
                while self
                    .peek()
                    .map_or(false, |d| d.is_ascii_alphanumeric() || d == b'.')
                {
                    self.i += 1;
                }
                Some(Ty::Num(self.src[st..self.i].to_string()))
            }
            _ if name_char(c) => {
                let st = self.i;
                while self.peek().map_or(false, name_char) {
                    // stop before `...` vararg suffix
                    if self.s[self.i..].starts_with(b"...") {
                        break;
                    }
                    self.i += 1;
                }
                let n = &self.src[st..self.i];
                if n.is_empty() {
                    return None;
                }
                match n {
                    "true" => return Some(Ty::Bool(true)),
                    "false" => return Some(Ty::Bool(false)),
                    _ => {}
                }
                if n == "fun" && self.peek() == Some(b'(') {
                    return self.fun();
                }
                if self.peek() == Some(b'<') {
                    self.i += 1;
                    let mut args = Vec::new();
                    loop {
                        args.push(self.union()?);
                        self.ws();
                        match self.peek()? {
                            b',' => self.i += 1,
                            b'>' => {
                                self.i += 1;
                                break;
                            }
                            _ => return None,
                        }
                    }
                    return Some(Ty::Generic(n.to_string(), args));
                }
                Some(Ty::Name(n.to_string()))
            }
            _ => None,
        }
    }
    fn fun(&mut self) -> Option<Ty> {
        self.i += 1; // (
        let mut ft = FunTy::default();
        loop {
            self.ws();
            match self.peek()? {
                b')' => {
                    self.i += 1;
                    break;
                }
                b',' => self.i += 1,
                _ => {
                    // `name: Type`, `name?: Type`, `...: Type` or a bare type.
                    let save = self.i;
                    let st = self.i;
                    if self.s[self.i..].starts_with(b"...") {
                        self.i += 3;
                    } else {
                        while self
                            .peek()
                            .map_or(false, |c| c.is_ascii_alphanumeric() || c == b'_')
                        {
                            self.i += 1;
                        }
                    }
                    let nm = self.src[st..self.i].to_string();
                    let mut opt = false;
                    if self.peek() == Some(b'?') {
                        opt = true;
                        self.i += 1;
                    }
                    self.ws();
                    if !nm.is_empty() && self.peek() == Some(b':') {
                        self.i += 1;
                        let t = self.union()?;
                        ft.params.push((nm, t, opt));
                    } else {
                        self.i = save;
                        let t = self.union()?;
                        ft.params
                            .push((format!("arg{}", ft.params.len() + 1), t, false));
                    }
                }
            }
        }
        let save = self.i;
        self.ws();
        if self.peek() == Some(b':') {
            self.i += 1;
            loop {
                match self.union() {
                    Some(t) => ft.returns.push(t),
                    None => break,
                }
                let sv = self.i;
                self.ws();
                if self.peek() == Some(b',') {
                    self.i += 1;
                } else {
                    self.i = sv;
                    break;
                }
            }
        } else {
            self.i = save;
        }
        Some(Ty::Fun(Box::new(ft)))
    }
}

/// Parse a type at the start of `s`; returns the type and the unconsumed rest (description).
pub fn parse_ty_prefix(s: &str) -> Option<(Ty, &str)> {
    let mut p = TyParser {
        s: s.as_bytes(),
        src: s,
        i: 0,
        depth: 0,
    };
    let t = p.union()?;
    Some((t, &s[p.i..]))
}

/// Parse a complete type (rest ignored); falls back to `any` on garbage.
pub fn parse_ty(s: &str) -> Ty {
    parse_ty_prefix(s.trim())
        .map(|x| x.0)
        .unwrap_or_else(Ty::any)
}

// ---- annotation blocks ----------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Class {
        name: String,
        parents: Vec<String>,
        exact: bool,
    },
    Field {
        name: String,
        ty: String,
        optional: bool,
        doc: String,
        visibility: Option<String>,
    },
    Type(Vec<String>),
    Param {
        name: String,
        ty: String,
        optional: bool,
        doc: String,
    },
    Return {
        ty: String,
        doc: String,
    },
    Alias {
        name: String,
        ty: Option<String>,
        variants: Vec<(String, String)>,
    },
    Enum {
        name: String,
    },
    Overload(String),
    Deprecated(String),
    Generic(String),
    Vararg(String),
    Meta(String),
    Nodiscard,
    Async,
    See(String),
    /// `---@diagnostic disable-next-line: undefined-global`
    Diagnostic {
        action: String,
        codes: Vec<String>,
    },
    Other {
        name: String,
        rest: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DocBlock {
    /// Free text lines (before/between tags), joined with newlines.
    pub description: String,
    pub tags: Vec<(Tag, Span)>,
    pub span: Span,
    /// 1-based line numbers of the first and last line.
    pub first_line: usize,
    pub last_line: usize,
}

impl DocBlock {
    pub fn is_meta(&self) -> bool {
        self.tags.iter().any(|(t, _)| matches!(t, Tag::Meta(_)))
    }
}

fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

fn clean_doc(s: &str) -> String {
    s.trim().trim_start_matches('#').trim().to_string()
}

/// Parse one `@tag rest` line.
fn parse_tag(name: &str, rest: &str) -> Tag {
    match name {
        "class" => {
            let mut r = rest.trim();
            let mut exact = false;
            if let Some(x) = r.strip_prefix("(exact)") {
                exact = true;
                r = x.trim_start();
            }
            let (w, after) = split_word(r);
            let (nm, tail) = if let Some(i) = w.find(':') {
                let rest = &w[i + 1..];
                (
                    &w[..i],
                    if rest.is_empty() {
                        after
                    } else {
                        r[i + 1..].trim()
                    },
                )
            } else if let Some(x) = after.strip_prefix(':') {
                (w, x.trim())
            } else {
                (w, "")
            };
            let parents = tail
                .split(',')
                .map(|p| p.split_whitespace().next().unwrap_or("").to_string())
                .filter(|p| !p.is_empty() && !p.starts_with('#'))
                .collect();
            Tag::Class {
                name: nm.to_string(),
                parents,
                exact,
            }
        }
        "field" => {
            let mut r = rest.trim();
            let mut vis = None;
            for v in ["private", "protected", "public", "package"] {
                if let Some(x) = r.strip_prefix(v) {
                    if x.starts_with(char::is_whitespace) {
                        vis = Some(v.to_string());
                        r = x.trim_start();
                    }
                }
            }
            let (nm, tail) = if r.starts_with('[') {
                match r.find(']') {
                    Some(i) => (&r[..=i], r[i + 1..].trim_start()),
                    None => split_word(r),
                }
            } else {
                split_word(r)
            };
            let (optional, nm) = match nm.strip_suffix('?') {
                Some(n) => (true, n),
                None => (false, nm),
            };
            let (ty, doc) = match parse_ty_prefix(tail) {
                Some((t, rest)) => (t.to_string(), clean_doc(rest)),
                None => ("any".to_string(), clean_doc(tail)),
            };
            Tag::Field {
                name: nm.to_string(),
                ty,
                optional,
                doc,
                visibility: vis,
            }
        }
        "type" => {
            let mut v = Vec::new();
            let mut r = rest.trim();
            while let Some((t, tail)) = parse_ty_prefix(r) {
                v.push(t.to_string());
                let tail = tail.trim_start();
                match tail.strip_prefix(',') {
                    Some(x) => r = x.trim_start(),
                    None => break,
                }
            }
            Tag::Type(v)
        }
        "param" => {
            let (nm, tail) = split_word(rest);
            let (optional, nm) = match nm.strip_suffix('?') {
                Some(n) => (true, n),
                None => (false, nm),
            };
            let (ty, doc) = match parse_ty_prefix(tail) {
                Some((t, rest)) => (t.to_string(), clean_doc(rest)),
                None => ("any".to_string(), clean_doc(tail)),
            };
            Tag::Param {
                name: nm.to_string(),
                ty,
                optional,
                doc,
            }
        }
        "return" => match parse_ty_prefix(rest) {
            Some((t, tail)) => {
                // `@return A, B` declares two returns; keep the first and fold the rest into a
                // union-free list
                Tag::Return {
                    ty: t.to_string(),
                    doc: clean_doc(tail),
                }
            }
            None => Tag::Return {
                ty: "any".into(),
                doc: clean_doc(rest),
            },
        },
        "alias" => {
            let (nm, tail) = split_word(rest);
            let ty = if tail.trim().is_empty() {
                None
            } else {
                Some(
                    parse_ty_prefix(tail)
                        .map(|x| x.0.to_string())
                        .unwrap_or_else(|| tail.trim().to_string()),
                )
            };
            Tag::Alias {
                name: nm.to_string(),
                ty,
                variants: vec![],
            }
        }
        "enum" => Tag::Enum {
            name: split_word(rest).0.to_string(),
        },
        "overload" => Tag::Overload(parse_ty(rest).to_string()),
        "deprecated" => Tag::Deprecated(rest.trim().to_string()),
        "generic" => Tag::Generic(rest.trim().to_string()),
        "vararg" => Tag::Vararg(parse_ty(rest).to_string()),
        "meta" => Tag::Meta(rest.trim().to_string()),
        "nodiscard" => Tag::Nodiscard,
        "async" => Tag::Async,
        "see" => Tag::See(rest.trim().to_string()),
        "diagnostic" => {
            let (action, codes) = match rest.find(':') {
                Some(i) => (
                    rest[..i].trim().to_string(),
                    rest[i + 1..]
                        .split(',')
                        .map(|c| c.trim().to_string())
                        .filter(|c| !c.is_empty())
                        .collect(),
                ),
                None => (rest.trim().to_string(), vec![]),
            };
            Tag::Diagnostic { action, codes }
        }
        other => Tag::Other {
            name: other.to_string(),
            rest: rest.trim().to_string(),
        },
    }
}

/// Parse the `---` lines of one block. `lines` are `(span of the whole comment, text after ---)`.
pub fn parse_block(lines: &[(Span, &str)], first_line: usize) -> DocBlock {
    let mut desc: Vec<String> = Vec::new();
    let mut tags: Vec<(Tag, Span)> = Vec::new();
    for (span, text) in lines {
        let t = text.trim();
        if let Some(rest) = t.strip_prefix('@') {
            let (name, tail) = split_word(rest);
            tags.push((parse_tag(name, tail), *span));
        } else if let Some(v) = t.strip_prefix('|') {
            // alias variant: `| "VALUE" # doc` or `|> "VALUE" # doc`
            let v = v.strip_prefix('>').unwrap_or(v).trim();
            if let Some((Tag::Alias { variants, .. }, _)) = tags.last_mut() {
                let (ty, doc) = match parse_ty_prefix(v) {
                    Some((t, rest)) => (t.to_string(), clean_doc(rest)),
                    None => (v.to_string(), String::new()),
                };
                variants.push((ty, doc));
            }
        } else if tags.is_empty() {
            desc.push(t.to_string());
        } else if let Some((
            Tag::Param { doc, .. } | Tag::Return { doc, .. } | Tag::Field { doc, .. },
            _,
        )) = tags.last_mut()
        {
            // continuation line of a multi-line tag description
            if !t.is_empty() {
                if !doc.is_empty() {
                    doc.push(' ');
                }
                doc.push_str(t);
            }
        } else if !t.is_empty() {
            desc.push(t.to_string());
        }
    }
    let span = Span::new(
        lines.first().map_or(0, |l| l.0.start),
        lines.last().map_or(0, |l| l.0.end),
    );
    DocBlock {
        description: desc.join("\n").trim().to_string(),
        tags,
        span,
        first_line,
        last_line: first_line + lines.len().saturating_sub(1),
    }
}

/// Find all annotation blocks (runs of `---` line comments) of a file.
///
/// Plain `--` comment lines directly adjacent to a run (explanations between the tags, as in
/// `bindings/parties.lua`) belong to the run but contribute no tags.
pub fn extract_blocks(src: &str, tokens: &[Token]) -> Vec<DocBlock> {
    let mut blocks = Vec::new();
    let mut cur: Vec<(Span, &str)> = Vec::new();
    let mut cur_first_line = 0usize;
    let mut run_end = 0usize;
    let mut line = 1usize;
    let mut code_on_line = false;
    // Newlines seen since the last comment of the current run (the run continues if exactly 1).
    let mut nl_since = 0usize;
    let flush =
        |blocks: &mut Vec<DocBlock>, cur: &mut Vec<(Span, &str)>, first: usize, end: usize| {
            if !cur.is_empty() {
                let mut b = parse_block(cur, first);
                b.span.end = b.span.end.max(end);
                blocks.push(b);
                cur.clear();
            }
        };
    for t in tokens {
        let text = t.text(src);
        match t.kind {
            TokKind::Whitespace => {
                let n = text.bytes().filter(|&b| b == b'\n').count();
                if n > 0 {
                    code_on_line = false;
                }
                nl_since += n;
                line += n;
            }
            TokKind::LineComment => {
                let doc = text.starts_with("---") && !text.starts_with("----");
                if code_on_line {
                    continue;
                }
                if doc {
                    if !cur.is_empty() && nl_since != 1 {
                        flush(&mut blocks, &mut cur, cur_first_line, run_end);
                    }
                    if cur.is_empty() {
                        cur_first_line = line;
                    }
                    cur.push((t.span, &text[3..]));
                    run_end = t.span.end;
                    nl_since = 0;
                } else if !cur.is_empty() && nl_since == 1 {
                    run_end = t.span.end;
                    nl_since = 0;
                } else {
                    flush(&mut blocks, &mut cur, cur_first_line, run_end);
                }
            }
            TokKind::Eof => {}
            _ => {
                flush(&mut blocks, &mut cur, cur_first_line, run_end);
                code_on_line = true;
                line += text.bytes().filter(|&b| b == b'\n').count();
            }
        }
    }
    flush(&mut blocks, &mut cur, cur_first_line, run_end);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::lexer::lex;

    #[test]
    fn types() {
        assert_eq!(parse_ty("integer").to_string(), "integer");
        assert_eq!(
            parse_ty("Thing[] | Creature[]").to_string(),
            "Thing[]|Creature[]"
        );
        assert_eq!(parse_ty("string|nil").members().len(), 2);
        assert_eq!(parse_ty("\"A\"|\"B\"").to_string(), "\"A\"|\"B\"");
        assert_eq!(
            parse_ty("fun(a: integer, b?: string): boolean").to_string(),
            "fun(a: integer, b?: string): boolean"
        );
        assert_eq!(
            parse_ty("table<string, integer>").to_string(),
            "table<string,integer>"
        );
        assert_eq!(parse_ty("integer?").to_string(), "integer?");
        assert!(parse_ty("integer?").accepts_nil());
        let (t, rest) = parse_ty_prefix("creature_type The type of creature").unwrap();
        assert_eq!(t.to_string(), "creature_type");
        assert_eq!(rest.trim(), "The type of creature");
        assert_eq!(
            parse_ty("Player | \"PLAYER0\" | \"PLAYER1\"")
                .members()
                .len(),
            3
        );
    }

    #[test]
    fn blocks() {
        let src =
            "---@meta\n\n---Does a thing.\n---More.\n---@param a integer the a\n---@param b? \
            string\n---@return boolean ok\nfunction F(a,b) end\n---@alias color\n---| \"RED\" # \
            red one\n---|> \"BLUE\"\nx = 1 ---@type string\n---@class Creature: Thing, \
            Other\n---@field level integer the level\n";
        let l = lex(src);
        let b = extract_blocks(src, &l.tokens);
        assert_eq!(b.len(), 4, "{b:#?}");
        assert!(b[0].is_meta());
        assert_eq!(b[1].description, "Does a thing.\nMore.");
        assert!(matches!(
            &b[1].tags[0].0,
            Tag::Param { name, ty, optional: false, doc }
                if name == "a" && ty == "integer" && doc == "the a"
        ));
        assert!(matches!(&b[1].tags[1].0, Tag::Param { name, optional: true, .. } if name == "b"));
        match &b[2].tags[0].0 {
            Tag::Alias { name, variants, .. } => {
                assert_eq!(name, "color");
                assert_eq!(variants.len(), 2);
                assert_eq!(variants[0], ("\"RED\"".to_string(), "red one".to_string()));
            }
            t => panic!("{t:?}"),
        }
        assert!(matches!(
            &b[3].tags[0].0,
            Tag::Class { name, parents, .. }
                if name == "Creature"
                    && parents == &["Thing".to_string(), "Other".to_string()]
        ));
    }
}
