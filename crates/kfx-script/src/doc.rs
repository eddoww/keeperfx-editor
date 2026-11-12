//! Lossless lexer and per-line parser.
//!
//! The parser mirrors `get_next_token`, `script_scan_line` and `script_recognize_params`
//! of `src/lvl_script.c` token by token, but never fails: malformed lines yield diagnostics and
//! partial syntax nodes. Concatenating the token texts reproduces the file byte for byte.
use crate::diag::{Diagnostic, Severity, Span};
use crate::tables::{is_preloaded, CmdDesc, Tables, ARGS_COUNT};

/// Lexical token kinds (lossless: whitespace, newlines and comments are tokens too).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokKind {
    Whitespace,
    Newline,
    /// Letters, digits, `_`, `[`, `]` (the game calls all of these `TkCommand`, numbers included).
    Ident,
    /// Negative integer, `-12` (positive numbers are `Ident` in the game's tokeniser).
    Number,
    /// `"text"`, quotes included.
    Str,
    /// `"text` up to end of line: the game treats it as an invalid token.
    UnterminatedStr,
    /// `=`, `!`, `<`, `>`, optionally followed by `=`; or a lone `-`.
    Operator,
    Tilde,
    LParen,
    RParen,
    Comma,
    /// `REM ...` to end of line (the keyword is recognised by the tokeniser anywhere).
    LineComment,
    /// `/* ... */`, only recognised at the start of a line (process_multiline_comment).
    BlockComment,
    Invalid,
    /// Bytes >= 0x80: the game silently skips them between tokens.
    NonAscii,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokKind,
    pub span: Span,
}

impl TokKind {
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            TokKind::Whitespace | TokKind::Newline | TokKind::NonAscii | TokKind::BlockComment
        )
    }
}

/// How an argument was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgTok {
    Ident,
    Number,
    Str,
    Operator,
    Tilde,
}

/// One argument (the game's `tp[dst]`).
#[derive(Clone, Debug)]
pub struct Arg {
    /// Whole argument; for a function call it covers `NAME(...)`.
    pub span: Span,
    /// Text as the game sees it: identifiers upper-cased, strings without quotes.
    pub text: String,
    pub tok: ArgTok,
    /// Span of `text` in the source (inside the quotes for strings).
    pub text_span: Span,
    /// Index into the command's argument list (`dst`).
    pub slot: usize,
    /// Type letter the game used for the conversion (upper case; `+` repeats applied), `0` if none.
    pub letter: u8,
    pub extended: bool,
    /// `RANDOM(...)`, `DRAWFROM(...)`, `IMPORT(...)`.
    pub func: Option<Box<FuncCall>>,
}

#[derive(Clone, Debug)]
pub struct FuncCall {
    pub name: String,
    pub name_span: Span,
    pub open: Span,
    pub close: Option<Span>,
    pub args: Vec<Arg>,
    pub commas: Vec<Span>,
    /// False when the game reports an error and discards the function.
    pub ok: bool,
}

/// A parsed command line.
#[derive(Clone, Debug)]
pub struct Command {
    pub name: String,
    pub name_span: Span,
    /// Table row used (`None` for an unknown command).
    pub desc: Option<&'static CmdDesc>,
    /// Level version in effect for this command's table lookup.
    pub version: i64,
    pub preloaded: bool,
    pub open: Option<Span>,
    pub close: Option<Span>,
    pub args: Vec<Arg>,
    pub commas: Vec<Span>,
    /// `args_count` returned by `script_recognize_params` (empty trailing string not counted).
    pub arg_count: usize,
    /// False when the game would discard the line because of a syntax-level error.
    pub syntax_ok: bool,
    /// From the command name to the last consumed token.
    pub span: Span,
}

impl Command {
    pub fn arg(&self, i: usize) -> Option<&Arg> {
        self.args.get(i)
    }
    pub fn text(&self, i: usize) -> &str {
        self.args.get(i).map(|a| a.text.as_str()).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Nothing but whitespace.
    Blank,
    /// Only a comment (`REM ...` or `/* */`).
    Comment,
    Command,
    /// Syntax error before a command name could be read.
    Error,
}

/// One logical line: normally a physical line, but a leading multi-line `/* */` comment is part
/// of the line on which it ends (as in `parse_txt_data`/`load_script`).
#[derive(Clone, Debug)]
pub struct Line {
    /// Token index range `[first, last)`.
    pub toks: (usize, usize),
    /// Byte range of the whole logical line including the newline.
    pub span: Span,
    pub kind: LineKind,
    pub cmd: Option<Command>,
    /// `REM` comment to end of line, if any.
    pub rem: Option<Span>,
    /// Leading block comment, if any.
    pub block: Option<Span>,
}

/// Parsed script.
#[derive(Debug)]
pub struct Document {
    pub src: Vec<u8>,
    pub tokens: Vec<Token>,
    pub lines: Vec<Line>,
    line_starts: Vec<usize>,
    /// `level_file_version` after the preload pass (last `LEVEL_VERSION`, default 0).
    pub version: i64,
    /// Syntax-level diagnostics (the validator adds the rest).
    pub diagnostics: Vec<Diagnostic>,
}

pub(crate) fn strtol0(s: &str) -> (i64, bool) {
    // C strtol(s, &end, 0); returns (value, whole string consumed)
    let b = s.as_bytes();
    let mut i = 0;
    let mut neg = false;
    while i < b.len() && (b[i] == b' ' || (9..=13).contains(&b[i])) {
        i += 1;
    }
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut base = 10;
    if i + 1 < b.len()
        && b[i] == b'0'
        && (b[i + 1] == b'x' || b[i + 1] == b'X')
        && i + 2 < b.len()
        && b[i + 2].is_ascii_hexdigit()
    {
        base = 16;
        i += 2;
    } else if i < b.len() && b[i] == b'0' {
        base = 8;
    }
    let st = i;
    let mut v: i64 = 0;
    while i < b.len() {
        let d = match (b[i] as char).to_digit(base) {
            Some(d) => d as i64,
            None => break,
        };
        v = v.saturating_mul(base as i64).saturating_add(d);
        i += 1;
    }
    if i == st && base != 8 {
        return (0, false);
    }
    (if neg { -v } else { v }, i == b.len())
}

// ---------------------------------------------------------------------------------------------
// Lexer

fn is_alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

fn lex(src: &[u8]) -> (Vec<Token>, Vec<(usize, usize)>) {
    let len = src.len();
    let mut toks: Vec<Token> = Vec::new();
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let mut pos = 0;
    let push = |toks: &mut Vec<Token>, kind, s, e| {
        toks.push(Token {
            kind,
            span: Span::new(s, e),
        })
    };
    while pos < len {
        let first = toks.len();
        // process_multiline_comment (src/lvl_script.c:842)
        let mut p = pos;
        while p + 1 < len && (src[p] == b' ' || src[p] == b'\t') {
            p += 1;
        }
        if p + 1 < len && src[p] == b'/' && src[p + 1] == b'*' {
            let mut q = p + 2;
            let mut end = None;
            while q + 1 < len {
                if src[q] == b'*' && src[q + 1] == b'/' {
                    end = Some(q + 2);
                    break;
                }
                q += 1;
            }
            if let Some(e) = end {
                if p > pos {
                    push(&mut toks, TokKind::Whitespace, pos, p);
                }
                push(&mut toks, TokKind::BlockComment, p, e);
                pos = e;
            }
        }
        // rest of the line
        while pos < len {
            let b = src[pos];
            let start = pos;
            if b == b'\n' {
                push(&mut toks, TokKind::Newline, pos, pos + 1);
                pos += 1;
                break;
            }
            if b == b' ' || b == b'\t' || b == b'\r' {
                while pos < len && matches!(src[pos], b' ' | b'\t' | b'\r') {
                    pos += 1;
                }
                push(&mut toks, TokKind::Whitespace, start, pos);
            } else if b >= 0x80 {
                while pos < len && src[pos] >= 0x80 {
                    pos += 1;
                }
                push(&mut toks, TokKind::NonAscii, start, pos);
            } else if is_alnum(b) {
                while pos < len && (is_alnum(src[pos]) || matches!(src[pos], b'[' | b']' | b'_')) {
                    pos += 1;
                }
                if pos - start == 3 && src[start..pos].eq_ignore_ascii_case(b"REM") {
                    while pos < len && src[pos] != b'\n' {
                        pos += 1;
                    }
                    push(&mut toks, TokKind::LineComment, start, pos);
                } else {
                    push(&mut toks, TokKind::Ident, start, pos);
                }
            } else if b == b'-' {
                pos += 1;
                if pos < len && src[pos].is_ascii_digit() {
                    while pos < len && src[pos].is_ascii_digit() {
                        pos += 1;
                    }
                    push(&mut toks, TokKind::Number, start, pos);
                } else {
                    push(&mut toks, TokKind::Operator, start, pos);
                }
            } else if b == b'"' {
                pos += 1;
                while pos < len && src[pos] != b'"' && src[pos] != b'\n' {
                    pos += 1;
                }
                if pos < len && src[pos] == b'"' {
                    pos += 1;
                    push(&mut toks, TokKind::Str, start, pos);
                } else {
                    push(&mut toks, TokKind::UnterminatedStr, start, pos);
                }
            } else if matches!(b, b'=' | b'!' | b'<' | b'>') {
                pos += 1;
                if pos < len && src[pos] == b'=' {
                    pos += 1;
                }
                push(&mut toks, TokKind::Operator, start, pos);
            } else if b == b'~' {
                pos += 1;
                push(&mut toks, TokKind::Tilde, start, pos);
            } else if b == b'(' {
                pos += 1;
                push(&mut toks, TokKind::LParen, start, pos);
            } else if b == b')' {
                pos += 1;
                push(&mut toks, TokKind::RParen, start, pos);
            } else if b == b',' {
                pos += 1;
                push(&mut toks, TokKind::Comma, start, pos);
            } else {
                pos += 1;
                push(&mut toks, TokKind::Invalid, start, pos);
            }
        }
        lines.push((first, toks.len()));
    }
    (toks, lines)
}

// ---------------------------------------------------------------------------------------------
// Line parser

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CTy {
    Invalid,
    Command,
    Str,
    Operator,
    Number,
    Open,
    Close,
    Comma,
    End,
    Range,
}

#[derive(Clone, Copy, Debug)]
struct CTok {
    ty: CTy,
    span: Span,
    /// Inner text span (inside quotes for strings).
    text: Span,
    kind: TokKind,
}

const END_TOK: CTok = CTok {
    ty: CTy::End,
    span: Span { start: 0, end: 0 },
    text: Span { start: 0, end: 0 },
    kind: TokKind::Whitespace,
};

struct Sink<'a> {
    line_starts: &'a [usize],
    out: Vec<Diagnostic>,
}

impl Sink<'_> {
    fn add(&mut self, severity: Severity, span: Span, code: &'static str, message: String) {
        let line = match self.line_starts.binary_search(&span.start) {
            Ok(i) => i + 1,
            Err(i) => i,
        };
        self.out.push(Diagnostic {
            severity,
            span,
            line: line.max(1),
            message,
            code,
        });
    }
}

struct LineParser<'a> {
    src: &'a [u8],
    ct: Vec<CTok>,
    pos: usize,
    sink: RawSink,
}

#[derive(Default)]
struct RawSink {
    out: Vec<(Severity, Span, &'static str, String)>,
}

impl RawSink {
    fn add(&mut self, severity: Severity, span: Span, code: &'static str, message: String) {
        self.out.push((severity, span, code, message));
    }
}

fn ctoks_of(src: &[u8], toks: &[Token]) -> Vec<CTok> {
    let mut v = Vec::new();
    for t in toks {
        let (ty, text) = match t.kind {
            TokKind::Ident => (CTy::Command, t.span),
            TokKind::Number => (CTy::Number, t.span),
            TokKind::Str => (CTy::Str, Span::new(t.span.start + 1, t.span.end - 1)),
            TokKind::Operator => (CTy::Operator, t.span),
            TokKind::Tilde => (CTy::Range, t.span),
            TokKind::LParen => (CTy::Open, t.span),
            TokKind::RParen => (CTy::Close, t.span),
            TokKind::Comma => (CTy::Comma, t.span),
            TokKind::LineComment => (CTy::End, t.span),
            TokKind::UnterminatedStr | TokKind::Invalid => (CTy::Invalid, t.span),
            _ => continue,
        };
        v.push(CTok {
            ty,
            span: t.span,
            text,
            kind: t.kind,
        });
    }
    let _ = src;
    v
}

impl<'a> LineParser<'a> {
    fn next(&mut self) -> CTok {
        if self.pos < self.ct.len() {
            let t = self.ct[self.pos];
            self.pos += 1;
            if t.ty == CTy::End {
                self.pos = self.ct.len();
            }
            t
        } else {
            let end = self.ct.last().map(|t| t.span.end).unwrap_or(0);
            CTok {
                span: Span::new(end, end),
                text: Span::new(end, end),
                ..END_TOK
            }
        }
    }
    fn peek(&self) -> CTok {
        self.ct.get(self.pos).copied().unwrap_or(END_TOK)
    }
    fn text(&self, t: &CTok) -> String {
        let raw = &self.src[t.text.start..t.text.end];
        let s: String = raw.iter().map(|&b| b as char).collect();
        if t.ty == CTy::Command {
            s.to_ascii_uppercase()
        } else {
            s
        }
    }

    /// `script_recognize_params` (src/lvl_script.c:618). Returns `None` where the game returns -1.
    fn recognize(
        &mut self,
        desc: &CmdDesc,
        cmdname: &str,
        args: &mut Vec<Arg>,
        commas: &mut Vec<Span>,
        close: &mut Option<Span>,
    ) -> Option<usize> {
        let mut dst: i32 = 0;
        let mut src: i32 = 0;
        let mut reparse = false;
        let mut token = CTok {
            ty: CTy::Invalid,
            ..END_TOK
        };
        let letter = |i: i32| -> u8 {
            if i < 0 {
                0
            } else {
                desc.letter(i as usize)
            }
        };
        while dst <= ARGS_COUNT as i32 {
            let chr = letter(src);
            if token.ty == CTy::Close {
                break;
            }
            if chr == b'!' {
                src += 1;
                continue;
            }
            let slot = args.len();
            if reparse {
                reparse = false;
                let tk = match token.ty {
                    CTy::Range => ArgTok::Tilde,
                    _ => ArgTok::Operator,
                };
                args.push(Arg {
                    span: token.span,
                    text: self.text(&token),
                    tok: tk,
                    text_span: token.text,
                    slot,
                    letter: 0,
                    extended: false,
                    func: None,
                });
            } else {
                token = self.next();
                match token.ty {
                    CTy::Invalid | CTy::End | CTy::Comma => {
                        let (msg, code) = if token.kind == TokKind::UnterminatedStr {
                            ("Unterminated string".to_string(), "unterminated-string")
                        } else if token.ty == CTy::End && args.is_empty() {
                            (
                                format!(
                                    "Unfinished {cmdname}: the arguments and the closing \
                                parenthesis are missing"
                                ),
                                "unexpected-eol",
                            )
                        } else if token.ty == CTy::End {
                            (
                                format!(
                                    "Unfinished {cmdname}: an argument is expected after the last \
                                comma (and a closing parenthesis)"
                                ),
                                "unexpected-eol",
                            )
                        } else if token.ty == CTy::Comma {
                            ("Unexpected ',' (empty argument)".to_string(), "bad-token")
                        } else {
                            ("Invalid token".to_string(), "bad-token")
                        };
                        let sp = if token.span.is_empty() {
                            Span::new(token.span.start.saturating_sub(1), token.span.start)
                        } else {
                            token.span
                        };
                        self.sink.add(Severity::Error, sp, code, msg);
                        return None;
                    }
                    CTy::Close if chr == b' ' || chr == 0 => {
                        *close = Some(token.span);
                        break;
                    }
                    CTy::Number | CTy::Command | CTy::Str => {}
                    _ => {
                        let t =
                            String::from_utf8_lossy(&self.src[token.span.start..token.span.end])
                                .to_string();
                        self.sink.add(
                            Severity::Warning,
                            token.span,
                            "excess-token",
                            format!(
                                "Excessive parameter of command \"{cmdname}\", value \"{t}\"; \
                                ignored"
                            ),
                        );
                        if token.ty == CTy::Close {
                            *close = Some(token.span);
                        }
                        src += 1;
                        continue;
                    }
                }
                let tk = match token.ty {
                    CTy::Str => ArgTok::Str,
                    CTy::Number => ArgTok::Number,
                    _ => ArgTok::Ident,
                };
                let text = self.text(&token);
                let mut arg = Arg {
                    span: token.span,
                    text,
                    tok: tk,
                    text_span: token.text,
                    slot,
                    letter: 0,
                    extended: false,
                    func: None,
                };
                if let Some(fd) = Tables::get().find_sub(&arg.text) {
                    let f = self.subfunc(fd, cmdname, &arg);
                    if let Some(f) = f {
                        arg.span = Span::new(
                            arg.span.start,
                            f.close
                                .map(|c| c.end)
                                .unwrap_or(f.args.last().map(|a| a.span.end).unwrap_or(f.open.end)),
                        );
                        arg.func = Some(Box::new(f));
                    }
                }
                args.push(arg);
                token = self.next();
                if token.ty == CTy::Range {
                    reparse = true;
                } else if token.ty != CTy::Close && token.ty != CTy::Comma {
                    if letter(src + 1) != b'O' || token.ty != CTy::Operator {
                        let sp = if token.span.is_empty() {
                            args.last().map(|a| a.span).unwrap_or_default()
                        } else {
                            token.span
                        };
                        if token.ty == CTy::End {
                            // the game reports "Unexpected token after parameter N"
                            // (script_recognize_params, src/lvl_script.c:700): the line ended where `,`
                            // or `)` belongs. Same verdict, plain words.
                            self.sink.add(
                                Severity::Error,
                                sp,
                                "missing-close-paren",
                                format!(
                                    "Missing closing parenthesis: {cmdname} is not closed with \
                                    ')' after parameter {} (the game discards the command)",
                                    dst + 1
                                ),
                            );
                        } else {
                            self.sink.add(
                                Severity::Error,
                                sp,
                                "bad-arg-separator",
                                format!(
                                    "Unexpected token after parameter {} of \
                                    command \"{cmdname}\", discarding command",
                                    dst + 1
                                ),
                            );
                        }
                        return None;
                    } else {
                        reparse = true;
                    }
                }
                if token.ty == CTy::Close {
                    *close = Some(token.span);
                } else if token.ty == CTy::Comma {
                    commas.push(token.span);
                }
            }
            if args[slot].text.is_empty() {
                break;
            }
            let chr2 = letter(src);
            if letter(src + 1) == b'+' {
                src -= 1;
            }
            let extended = letter(src + 1) == b'!';
            args[slot].letter = chr2.to_ascii_uppercase();
            args[slot].extended = extended;
            dst += 1;
            src += 1;
        }
        Some(dst.max(0) as usize)
    }

    /// `process_subfunc` (src/lvl_script.c:426): syntax only. `None` when no `(` follows.
    fn subfunc(&mut self, fd: &'static CmdDesc, cmdname: &str, name_arg: &Arg) -> Option<FuncCall> {
        let nx = self.peek();
        if nx.ty != CTy::Open {
            self.sink.add(
                Severity::Error,
                name_arg.span,
                "expected-paren",
                format!("Expecting ( after function {}", fd.name),
            );
            return None;
        }
        self.pos += 1;
        let mut f = FuncCall {
            name: fd.name.clone(),
            name_span: name_arg.span,
            open: nx.span,
            close: None,
            args: vec![],
            commas: vec![],
            ok: true,
        };
        let n = self.recognize(fd, &fd.name, &mut f.args, &mut f.commas, &mut f.close);
        match n {
            None => f.ok = false,
            Some(n) => {
                if n < ARGS_COUNT && n < fd.required() {
                    let sp = f.args.last().map(|a| a.span).unwrap_or(name_arg.span);
                    self.sink.add(
                        Severity::Error,
                        sp,
                        "too-few-args",
                        format!("Not enough parameters for \"{}\", got only {n}", fd.name),
                    );
                    f.ok = false;
                }
            }
        }
        let _ = cmdname;
        Some(f)
    }
}

/// Message of the `unknown-command` diagnostic. The game would log "Invalid command"; the editor
/// only knows the commands of the KeeperFX revision its data was generated from
/// ([`crate::DataRevision`]), so it says that instead of calling the command invalid.
pub fn unknown_command_message(name: &str, version: i64) -> String {
    let rev = crate::DataRevision::get();
    if crate::Overlay::get().removed(name).is_some() {
        return format!(
            "Unknown command '{name}' (level version {version}): KeeperFX removed it before \
            {} (the revision this editor build's data was generated from); older KeeperFX \
            versions may still accept it",
            rev.keeperfx_day()
        );
    }
    format!(
        "Unknown command '{name}' (level version {version}): not known to this editor build's \
        data (generated from KeeperFX {}, {}); a newer editor build may know it",
        rev.keeperfx_short(),
        rev.keeperfx_day()
    )
}

fn did_you_mean(name: &str, table: &[CmdDesc]) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for c in table {
        let d = crate::util::levenshtein(name, &c.name);
        if best.map(|b| d < b.0).unwrap_or(true) {
            best = Some((d, &c.name));
        }
    }
    let (d, n) = best?;
    let limit = (name.len() / 3).max(2);
    if d <= limit {
        Some(n.to_string())
    } else {
        None
    }
}

impl Document {
    /// Parse a script. Never fails.
    pub fn parse(src: &[u8]) -> Document {
        let (tokens, line_toks) = lex(src);
        let mut line_starts = vec![0usize];
        for (i, &b) in src.iter().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        let tables = Tables::get();
        let mut sink = Sink {
            line_starts: &line_starts,
            out: vec![],
        };

        // Pass 1 (preload): only LEVEL_VERSION matters; remember the version in effect per line.
        let mut ver = 0i64;
        let mut pre_ver: Vec<i64> = Vec::with_capacity(line_toks.len());
        for &(a, b) in &line_toks {
            pre_ver.push(ver);
            let ct = ctoks_of(src, &tokens[a..b]);
            if let Some(t0) = ct.first() {
                if t0.ty == CTy::Command {
                    let mut p = LineParser {
                        src,
                        ct: ct.clone(),
                        pos: 1,
                        sink: RawSink::default(),
                    };
                    let name = p.text(t0);
                    if name == "LEVEL_VERSION" && tables.find(ver, &name).is_some() {
                        let nx = p.next();
                        if nx.ty == CTy::Open {
                            let (mut args, mut commas, mut close) = (vec![], vec![], None);
                            let desc = tables.find(ver, &name).unwrap();
                            if let Some(n) =
                                p.recognize(desc, &name, &mut args, &mut commas, &mut close)
                            {
                                if n >= desc.required() && n >= 1 {
                                    ver = strtol0(&args[0].text).0;
                                }
                            }
                        }
                    }
                }
            }
        }
        let final_version = ver;

        let mut lines: Vec<Line> = Vec::with_capacity(line_toks.len());
        for (li, &(a, b)) in line_toks.iter().enumerate() {
            let line_span = Span::new(
                tokens.get(a).map(|t| t.span.start).unwrap_or(0),
                tokens
                    .get(b.wrapping_sub(1))
                    .map(|t| t.span.end)
                    .unwrap_or(0),
            );
            let lt = &tokens[a..b];
            let block = lt
                .iter()
                .find(|t| t.kind == TokKind::BlockComment)
                .map(|t| t.span);
            let rem = lt
                .iter()
                .find(|t| t.kind == TokKind::LineComment)
                .map(|t| t.span);
            let ct = ctoks_of(src, lt);
            let mut line = Line {
                toks: (a, b),
                span: line_span,
                kind: LineKind::Blank,
                cmd: None,
                rem,
                block,
            };
            let first = ct.first().copied();
            match first {
                None => {
                    line.kind = if block.is_some() {
                        LineKind::Comment
                    } else {
                        LineKind::Blank
                    };
                }
                Some(t0) if t0.ty == CTy::End => {
                    line.kind = LineKind::Comment;
                }
                Some(t0) if t0.ty != CTy::Command => {
                    let (code, msg) = if t0.kind == TokKind::UnterminatedStr {
                        ("unterminated-string", "Unterminated string".to_string())
                    } else if lt
                        .iter()
                        .any(|t| t.kind == TokKind::Invalid && src[t.span.start] == b'/')
                        && t0.span.start < src.len()
                        && src[t0.span.start] == b'/'
                    {
                        (
                            "syntax",
                            "Syntax error: Script command expected (a /* comment must start \
                            the line and be closed with */)"
                                .to_string(),
                        )
                    } else {
                        (
                            "syntax",
                            "Syntax error: Script command expected".to_string(),
                        )
                    };
                    sink.add(Severity::Error, t0.span, code, msg);
                    line.kind = LineKind::Error;
                }
                Some(t0) => {
                    let mut p = LineParser {
                        src,
                        ct: ct.clone(),
                        pos: 1,
                        sink: RawSink::default(),
                    };
                    let name = p.text(&t0);
                    let preloaded = is_preloaded(&name);
                    let ver_here = if preloaded {
                        pre_ver[li]
                    } else {
                        final_version
                    };
                    let desc = tables.find(ver_here, &name);
                    let mut cmd = Command {
                        name: name.clone(),
                        name_span: t0.span,
                        desc: None,
                        version: ver_here,
                        preloaded,
                        open: None,
                        close: None,
                        args: vec![],
                        commas: vec![],
                        arg_count: 0,
                        syntax_ok: false,
                        span: t0.span,
                    };
                    // Pass 1 looks up every command in the table of the version at that point.
                    let pre_desc = tables.find(pre_ver[li], &name);
                    line.kind = LineKind::Command;
                    match desc {
                        None => {
                            let in_full = tables.find_full(&name).is_some();
                            let mut msg = unknown_command_message(&name, ver_here);
                            let code;
                            if in_full && ver_here <= 0 {
                                msg = format!(
                                    "Command {name} is not available in level version 0 \
                                    (the dk1 command set); put LEVEL_VERSION(1) before it"
                                );
                                code = "needs-level-version";
                            } else {
                                if let Some(s) =
                                    did_you_mean(&name, tables.table(ver_here.max(final_version)))
                                {
                                    msg.push_str(&format!("; did you mean {s}?"));
                                }
                                code = "unknown-command";
                            }
                            sink.add(Severity::Error, t0.span, code, msg);
                        }
                        Some(d) => {
                            cmd.desc = Some(d);
                            if pre_desc.is_none() && !preloaded {
                                sink.add(
                                    Severity::Warning,
                                    t0.span,
                                    "command-before-level-version",
                                    format!(
                                        "{name} comes before LEVEL_VERSION: the preload pass logs \
                                            it as an invalid command (it is still run by the main \
                                            pass)"
                                    ),
                                );
                            }
                            let nx = p.next();
                            let mut ok = true;
                            if nx.ty == CTy::End {
                                // no arguments
                            } else if nx.ty == CTy::Open {
                                cmd.open = Some(nx.span);
                                match p.recognize(
                                    d,
                                    &name,
                                    &mut cmd.args,
                                    &mut cmd.commas,
                                    &mut cmd.close,
                                ) {
                                    Some(n) => cmd.arg_count = n,
                                    None => ok = false,
                                }
                            } else {
                                p.sink.add(
                                    Severity::Error,
                                    nx.span,
                                    "expected-paren",
                                    format!("Syntax error: ( expected after {name}"),
                                );
                                ok = false;
                            }
                            if ok && cmd.arg_count < ARGS_COUNT && cmd.arg_count < d.required() {
                                let sp = cmd.args.last().map(|a| a.span).unwrap_or(t0.span);
                                p.sink.add(
                                    Severity::Error,
                                    Span::new(
                                        sp.start.min(cmd.name_span.start),
                                        sp.end.max(cmd.name_span.end),
                                    ),
                                    "too-few-args",
                                    format!(
                                        "Not enough parameters for \"{}\", got only {} (needs {})",
                                        d.name,
                                        cmd.arg_count,
                                        d.required()
                                    ),
                                );
                                ok = false;
                            }
                            if ok && nx.ty != CTy::End {
                                let t = p.next();
                                if t.ty != CTy::End {
                                    p.sink.add(
                                        Severity::Warning,
                                        t.span,
                                        "trailing-tokens",
                                        "Syntax error: unexpected end of line (extra \
                                        tokens after the command are ignored)"
                                            .into(),
                                    );
                                }
                            }
                            cmd.syntax_ok = ok;
                            let last = cmd
                                .close
                                .or_else(|| cmd.args.last().map(|a| a.span))
                                .unwrap_or(cmd.open.unwrap_or(t0.span));
                            cmd.span = Span::new(t0.span.start, last.end.max(t0.span.end));
                        }
                    }
                    for (sv, sp, code, msg) in std::mem::take(&mut p.sink.out) {
                        sink.add(sv, sp, code, msg);
                    }
                    line.cmd = Some(cmd);
                }
            }
            lines.push(line);
        }

        let diagnostics = sink.out;
        Document {
            src: src.to_vec(),
            tokens,
            lines,
            line_starts,
            version: final_version,
            diagnostics,
        }
    }

    pub fn text(&self, s: Span) -> &[u8] {
        &self.src[s.start.min(self.src.len())..s.end.min(self.src.len())]
    }
    /// Latin-1 decoded text of a span.
    pub fn text_lossy(&self, s: Span) -> String {
        self.text(s).iter().map(|&b| b as char).collect()
    }
    /// 1-based physical line of a byte offset.
    pub fn line_of(&self, off: usize) -> usize {
        match self.line_starts.binary_search(&off) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
        .max(1)
    }
    /// Byte offset where physical line `n` (1-based) starts.
    pub fn line_start(&self, n: usize) -> usize {
        self.line_starts
            .get(n.saturating_sub(1))
            .copied()
            .unwrap_or(self.src.len())
    }
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }
    /// Concatenation of all token texts (must equal the source).
    pub fn reassemble(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.src.len());
        for t in &self.tokens {
            v.extend_from_slice(&self.src[t.span.start..t.span.end]);
        }
        v
    }
    /// The logical line containing `off`.
    pub fn line_at(&self, off: usize) -> Option<usize> {
        let i = self.lines.partition_point(|l| l.span.end <= off);
        if i < self.lines.len() {
            Some(i)
        } else if !self.lines.is_empty() && off == self.src.len() {
            Some(self.lines.len() - 1)
        } else {
            None
        }
    }
    pub fn commands(&self) -> impl Iterator<Item = (usize, &Command)> {
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.cmd.as_ref().map(|c| (i, c)))
    }
}
