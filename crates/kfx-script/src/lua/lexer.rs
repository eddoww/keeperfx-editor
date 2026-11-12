//! Lossless LuaJIT lexer.
//!
//! Every byte of the input belongs to exactly one token (whitespace and comments are tokens), so
//! concatenating the token texts reproduces the source byte for byte. Malformed input never stops
//! the lexer: the offending token is still emitted and a [`LexError`] is recorded.
//!
//! LuaJIT specifics handled here (as in `lj_lex.c`): long strings and comments with `=` levels,
//! hex floats with `p` exponents, `LL`/`ULL`/`i` number suffixes, `::label::`, `goto` as a
//! keyword, the escapes `\x`, `\z`, `\u{...}` and `\ddd`, and non-ASCII bytes inside identifiers.
use crate::diag::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokKind {
    Whitespace,
    LineComment,
    BlockComment,
    Name,
    Keyword,
    Number,
    String,
    LongString,
    Punct,
    /// `#!...` first line.
    Shebang,
    /// A byte that starts no valid token (`$`, `@`, `` ` ``, `!`, `?`, `\`).
    Unknown,
    /// Zero-width end marker (never part of the reassembly).
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokKind,
    pub span: Span,
}

impl Token {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.span.start..self.span.end]
    }
    pub fn is_trivia(&self) -> bool {
        matches!(
            self.kind,
            TokKind::Whitespace | TokKind::LineComment | TokKind::BlockComment | TokKind::Shebang
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexError {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct Lexed {
    /// Tokens covering the source completely, followed by one zero-width `Eof` token.
    pub tokens: Vec<Token>,
    pub errors: Vec<LexError>,
}

pub const KEYWORDS: [&str; 22] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

pub fn is_keyword(s: &str) -> bool {
    KEYWORDS.binary_search(&s).is_ok()
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}
fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// If a long bracket opens at `i` (`[`, `=`*, `[`), return `(level, index after the opener)`.
fn long_open(b: &[u8], i: usize) -> Option<(usize, usize)> {
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while b.get(j) == Some(&b'=') {
        j += 1;
    }
    if b.get(j) == Some(&b'[') {
        Some((j - i - 1, j + 1))
    } else {
        None
    }
}

/// Scan to the end of a long bracket of `level` starting at `i` (after the opener).
/// Returns `(end index, terminated)`.
fn long_close(b: &[u8], mut i: usize, level: usize) -> (usize, bool) {
    while i < b.len() {
        if b[i] == b']' {
            let mut j = i + 1;
            let mut n = 0;
            while b.get(j) == Some(&b'=') {
                n += 1;
                j += 1;
            }
            if n == level && b.get(j) == Some(&b']') {
                return (j + 1, true);
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    (b.len(), false)
}

/// Validate a LuaJIT numeric literal; returns an error message when malformed.
pub fn check_number(s: &str) -> Option<&'static str> {
    let b = s.as_bytes();
    let (mut i, hex) = if b.len() > 1 && b[0] == b'0' && (b[1] == b'x' || b[1] == b'X') {
        (2, true)
    } else {
        (0, false)
    };
    let mut digits = 0;
    let isd = |c: u8| {
        if hex {
            c.is_ascii_hexdigit()
        } else {
            c.is_ascii_digit()
        }
    };
    while i < b.len() && isd(b[i]) {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && isd(b[i]) {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return Some("malformed number");
    }
    let exp = |c: u8| {
        if hex {
            c == b'p' || c == b'P'
        } else {
            c == b'e' || c == b'E'
        }
    };
    if i < b.len() && exp(b[i]) {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let st = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if st == i {
            return Some("malformed number: missing exponent digits");
        }
    }
    let rest = &s[i..];
    match rest {
        "" | "LL" | "ll" | "ULL" | "uLL" | "Ull" | "ull" | "uLl" | "ULl" | "UlL" | "i" | "I" => {
            None
        }
        _ => Some("malformed number"),
    }
}

pub fn lex(src: &str) -> Lexed {
    let b = src.as_bytes();
    let mut out = Lexed::default();
    let mut i = 0;
    let n = b.len();
    let push = |out: &mut Lexed, kind, s: usize, e: usize| {
        out.tokens.push(Token {
            kind,
            span: Span::new(s, e),
        })
    };
    if b.starts_with(b"#") {
        let e = b.iter().position(|&c| c == b'\n').unwrap_or(n);
        push(&mut out, TokKind::Shebang, 0, e);
        i = e;
    }
    while i < n {
        let c = b[i];
        let s = i;
        match c {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c => {
                while i < n && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c) {
                    i += 1;
                }
                push(&mut out, TokKind::Whitespace, s, i);
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                if let Some((level, body)) = long_open(b, i + 2) {
                    let (e, ok) = long_close(b, body, level);
                    if !ok {
                        out.errors.push(LexError {
                            span: Span::new(s, e),
                            message: "unfinished long comment".into(),
                        });
                    }
                    push(&mut out, TokKind::BlockComment, s, e);
                    i = e;
                } else {
                    while i < n && b[i] != b'\n' {
                        i += 1;
                    }
                    push(&mut out, TokKind::LineComment, s, i);
                }
            }
            b'[' if long_open(b, i).is_some() => {
                let (level, body) = long_open(b, i).unwrap();
                let (e, ok) = long_close(b, body, level);
                if !ok {
                    out.errors.push(LexError {
                        span: Span::new(s, e),
                        message: "unfinished long string".into(),
                    });
                }
                push(&mut out, TokKind::LongString, s, e);
                i = e;
            }
            b'[' if b.get(i + 1) == Some(&b'=') => {
                // `[=` without a second `[`: LuaJIT: invalid long string delimiter.
                let mut j = i + 1;
                while b.get(j) == Some(&b'=') {
                    j += 1;
                }
                out.errors.push(LexError {
                    span: Span::new(i, j),
                    message: "invalid long string delimiter".into(),
                });
                push(&mut out, TokKind::Punct, i, i + 1);
                i += 1;
            }
            b'"' | b'\'' => {
                i += 1;
                let mut closed = false;
                while i < n {
                    let d = b[i];
                    if d == c {
                        i += 1;
                        closed = true;
                        break;
                    }
                    if d == b'\n' || d == b'\r' {
                        break;
                    }
                    if d == b'\\' {
                        let es = i;
                        i += 1;
                        match b.get(i) {
                            None => break,
                            Some(
                                b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'\\' | b'"'
                                | b'\'',
                            ) => i += 1,
                            Some(b'\n') => {
                                i += 1;
                                if b.get(i) == Some(&b'\r') {
                                    i += 1;
                                }
                            }
                            Some(b'\r') => {
                                i += 1;
                                if b.get(i) == Some(&b'\n') {
                                    i += 1;
                                }
                            }
                            Some(b'z') => {
                                i += 1;
                                while i < n
                                    && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)
                                {
                                    i += 1;
                                }
                            }
                            Some(b'x') => {
                                i += 1;
                                let st = i;
                                while i < n && i < st + 2 && b[i].is_ascii_hexdigit() {
                                    i += 1;
                                }
                                if i - st != 2 {
                                    out.errors.push(LexError {
                                        span: Span::new(es, i),
                                        message: "invalid escape sequence \\x".into(),
                                    });
                                }
                            }
                            Some(b'u') => {
                                i += 1;
                                let st = i;
                                let mut ok = false;
                                if b.get(i) == Some(&b'{') {
                                    i += 1;
                                    let hs = i;
                                    while i < n && b[i].is_ascii_hexdigit() {
                                        i += 1;
                                    }
                                    if i > hs && b.get(i) == Some(&b'}') {
                                        i += 1;
                                        ok = true;
                                    }
                                }
                                if !ok {
                                    i = st;
                                    out.errors.push(LexError {
                                        span: Span::new(es, i),
                                        message: "invalid escape sequence \\u".into(),
                                    });
                                }
                            }
                            Some(d) if d.is_ascii_digit() => {
                                let st = i;
                                let mut v = 0u32;
                                while i < n && i < st + 3 && b[i].is_ascii_digit() {
                                    v = v * 10 + (b[i] - b'0') as u32;
                                    i += 1;
                                }
                                if v > 255 {
                                    out.errors.push(LexError {
                                        span: Span::new(es, i),
                                        message: "decimal escape too large".into(),
                                    });
                                }
                            }
                            Some(_) => {
                                // consume one whole char so the span stays on a boundary
                                let mut j = i + 1;
                                while j < n && (b[j] & 0xC0) == 0x80 {
                                    j += 1;
                                }
                                out.errors.push(LexError {
                                    span: Span::new(es, j),
                                    message: "invalid escape sequence".into(),
                                });
                                i = j;
                            }
                        }
                        continue;
                    }
                    i += 1;
                }
                if !closed {
                    out.errors.push(LexError {
                        span: Span::new(s, i),
                        message: "unfinished string".into(),
                    });
                }
                push(&mut out, TokKind::String, s, i);
            }
            _ if c.is_ascii_digit()
                || (c == b'.' && b.get(i + 1).map_or(false, |d| d.is_ascii_digit())) =>
            {
                let hex = c == b'0' && matches!(b.get(i + 1), Some(b'x' | b'X'));
                i += 1;
                while i < n {
                    let d = b[i];
                    let expo = if hex {
                        matches!(d, b'p' | b'P')
                    } else {
                        matches!(d, b'e' | b'E')
                    };
                    if is_ident(d) || d == b'.' {
                        i += 1;
                    } else if (d == b'+' || d == b'-') && i > s && {
                        let p = b[i - 1];
                        if hex {
                            matches!(p, b'p' | b'P')
                        } else {
                            matches!(p, b'e' | b'E')
                        }
                    } {
                        i += 1;
                    } else {
                        let _ = expo;
                        break;
                    }
                }
                if let Some(m) = check_number(&src[s..i]) {
                    out.errors.push(LexError {
                        span: Span::new(s, i),
                        message: m.into(),
                    });
                }
                push(&mut out, TokKind::Number, s, i);
            }
            _ if is_ident_start(c) => {
                while i < n && is_ident(b[i]) {
                    i += 1;
                }
                let kind = if is_keyword(&src[s..i]) {
                    TokKind::Keyword
                } else {
                    TokKind::Name
                };
                push(&mut out, kind, s, i);
            }
            _ => {
                let two: &[u8] = &b[i..(i + 2).min(n)];
                let len = if b[i..].starts_with(b"...") {
                    3
                } else if matches!(two, b".." | b"==" | b"~=" | b"<=" | b">=" | b"::") {
                    2
                } else {
                    1
                };
                let kind = if len == 1 && !b"+-*/%^#<>=(){}[];:,.&|~".contains(&c) {
                    TokKind::Unknown
                } else {
                    TokKind::Punct
                };
                // Unknown non-ASCII cannot happen (they are identifier bytes); ASCII is one byte.
                i += len;
                push(&mut out, kind, s, i);
            }
        }
    }
    out.tokens.push(Token {
        kind: TokKind::Eof,
        span: Span::new(n, n),
    });
    out
}

/// Reassemble the source from tokens (test helper and sanity check).
pub fn reassemble(src: &str, tokens: &[Token]) -> String {
    let mut s = String::with_capacity(src.len());
    for t in tokens {
        s.push_str(t.text(src));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    fn kinds(s: &str) -> Vec<(TokKind, &str)> {
        let l = lex(s);
        l.tokens
            .iter()
            .filter(|t| t.kind != TokKind::Eof)
            .map(|t| (t.kind, t.text(s)))
            .collect()
    }
    #[test]
    fn long_forms() {
        let s = "x = [==[a]]b]==] --[[c]] --[=[\nd]=]\n-- e";
        assert_eq!(reassemble(s, &lex(s).tokens), s);
        let k: Vec<_> = kinds(s)
            .into_iter()
            .filter(|t| t.0 != TokKind::Whitespace)
            .collect();
        assert_eq!(k[2], (TokKind::LongString, "[==[a]]b]==]"));
        assert_eq!(k[3].0, TokKind::BlockComment);
        assert_eq!(k[4].0, TokKind::BlockComment);
        assert_eq!(k[5].0, TokKind::LineComment);
        assert!(lex(s).errors.is_empty());
    }
    #[test]
    fn numbers() {
        for ok in [
            "1", "1.5", ".5", "0x1F", "0xA.8p3", "1e-3", "3LL", "0xFFull", "2i", "1e+5",
        ] {
            assert!(lex(ok).errors.is_empty(), "{ok}");
            assert_eq!(kinds(ok).len(), 1, "{ok}");
        }
        for bad in ["1e", "0x", "3abc", "1..2"] {
            assert!(!lex(bad).errors.is_empty(), "{bad}");
        }
    }
    #[test]
    fn unfinished() {
        assert!(!lex("\"abc").errors.is_empty());
        assert!(!lex("--[[ abc").errors.is_empty());
        assert!(!lex("'a\\qb'").errors.is_empty());
        assert!(lex("'a\\x41\\u{20AC}\\065\\z  b'").errors.is_empty());
    }
    #[test]
    fn labels_and_ops() {
        let k = kinds("::top:: a.b:c(...) goto top");
        assert!(k.contains(&(TokKind::Punct, "::")));
        assert!(k.contains(&(TokKind::Punct, "...")));
        assert!(k.contains(&(TokKind::Keyword, "goto")));
    }
}
