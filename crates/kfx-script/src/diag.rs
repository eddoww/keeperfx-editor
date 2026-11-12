//! Diagnostics shared by the parser and the validator.

/// Byte range `[start, end)` into the script source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
    /// True when `off` is inside or at either edge of the span.
    pub fn touches(&self, off: usize) -> bool {
        off >= self.start && off <= self.end
    }
    pub fn contains(&self, off: usize) -> bool {
        off >= self.start && off < self.end
    }
    pub fn join(&self, o: Span) -> Span {
        Span {
            start: self.start.min(o.start),
            end: self.end.max(o.end),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// One problem found in a script.
///
/// `Error` = the game rejects the command/line (or logs an error and ignores part of it);
/// `Warning` = the game logs a warning, or accepts it but the result is unlikely to be intended;
/// `Info` = style or portability notes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub span: Span,
    /// 1-based physical line of `span.start`.
    pub line: usize,
    pub message: String,
    /// Stable machine readable class, e.g. `unknown-command`.
    pub code: &'static str,
}

/// Whether `d` is a complaint about a *value* (an unknown name, a bad number, a wrongly typed
/// argument, an unfinished argument) as opposed to the structure of the script. Those are noise
/// while the user is still typing the very token they are about.
pub fn is_value_diagnostic(d: &Diagnostic) -> bool {
    d.code.starts_with("unknown-")
        || matches!(
            d.code,
            "unfinished-arg"
                | "number-format"
                | "bad-location"
                | "missing-action-point"
                | "missing-hero-gate"
                | "lua-bad-literal"
                | "lua-arg-type"
                | "lua-undefined-global"
        )
}

/// Byte span of the word (letters, digits, `_`, `[`, `]`, `-`) the caret touches in `src`.
pub fn token_at(src: &[u8], caret: usize) -> Option<Span> {
    let w = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'[' | b']' | b'-');
    let caret = caret.min(src.len());
    let mut s = caret;
    while s > 0 && w(src[s - 1]) {
        s -= 1;
    }
    let mut e = caret;
    while e < src.len() && w(src[e]) {
        e += 1;
    }
    (e > s).then(|| Span::new(s, e))
}

/// Whether the UI should hide `d` while the caret is at `caret`: value diagnostics whose span the
/// caret touches (the token being typed). Structural problems always stay visible.
pub fn hidden_at_caret(d: &Diagnostic, caret: usize) -> bool {
    is_value_diagnostic(d) && d.span.touches(caret)
}

/// `diags` without the value diagnostics hidden by [`hidden_at_caret`]; spans and `caret` use the
/// same coordinates.
pub fn suppress_at_caret(diags: &[Diagnostic], caret: usize) -> Vec<Diagnostic> {
    diags
        .iter()
        .filter(|d| !hidden_at_caret(d, caret))
        .cloned()
        .collect()
}
