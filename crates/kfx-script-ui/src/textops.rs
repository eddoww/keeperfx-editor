//! Pure text-editing logic over Latin-1 byte buffers (one byte = one editor character).
//!
//! Nothing here touches egui, so all of it is unit tested.
use crate::lang::{CallParens, Completion, CompletionKind};
use kfx_script::Span;

/// Replace `[start, end)` by `text` (Latin-1 characters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl Edit {
    pub fn new(start: usize, end: usize, text: impl Into<String>) -> Edit {
        Edit {
            start,
            end,
            text: text.into(),
        }
    }
    pub fn insert(at: usize, text: impl Into<String>) -> Edit {
        Edit::new(at, at, text)
    }
}

/// Edits (non-overlapping, in pre-edit coordinates) plus the selection afterwards
/// (post-edit coordinates).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transaction {
    pub edits: Vec<Edit>,
    pub select: Option<(usize, usize)>,
}

impl Transaction {
    pub fn single(edit: Edit, select: Option<(usize, usize)>) -> Transaction {
        Transaction {
            edits: vec![edit],
            select,
        }
    }
}

/// Apply a transaction to a byte buffer (tests and the cache; the live buffer uses the same
/// order: highest start first).
pub fn apply(text: &mut Vec<u8>, tx: &Transaction) {
    let mut edits: Vec<&Edit> = tx.edits.iter().collect();
    edits.sort_by_key(|e| std::cmp::Reverse(e.start));
    for e in edits {
        let s = e.start.min(text.len());
        let en = e.end.clamp(s, text.len());
        let repl: Vec<u8> = e.text.chars().map(|c| c as u32 as u8).collect();
        text.splice(s..en, repl);
    }
}

// ---------------------------------------------------------------------------------------------
// Lines

/// Line table of a byte buffer. Works on raw file bytes too: a `\r\n` pair ends a line and the
/// `\r` is not part of the line's text range.
#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<usize>,
    len: usize,
    /// Byte before each line start (`\n`) was preceded by `\r`.
    crlf: Vec<bool>,
}

impl LineIndex {
    pub fn new(bytes: &[u8]) -> LineIndex {
        let mut starts = vec![0];
        let mut crlf = vec![false];
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
                crlf.push(i > 0 && bytes[i - 1] == b'\r');
            }
        }
        LineIndex {
            starts,
            len: bytes.len(),
            crlf,
        }
    }
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }
    /// 0-based line containing `off`. An offset inside a `\r\n` pair belongs to the line it ends.
    pub fn line_of(&self, off: usize) -> usize {
        match self.starts.binary_search(&off.min(self.len)) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    }
    /// Start of line `line` (clamped).
    pub fn line_start(&self, line: usize) -> usize {
        self.starts[line.min(self.starts.len() - 1)]
    }
    /// End of the line's text, before its line ending.
    pub fn line_end(&self, line: usize) -> usize {
        let line = line.min(self.starts.len() - 1);
        if line + 1 < self.starts.len() {
            self.starts[line + 1] - 1 - self.crlf[line + 1] as usize
        } else {
            self.len
        }
    }
    /// Start of the next line (or the end of the buffer for the last line).
    pub fn next_line_start(&self, line: usize) -> usize {
        self.starts.get(line + 1).copied().unwrap_or(self.len)
    }
    /// 0-based `(line, column)`; a column inside a `\r\n` pair is clamped to the line end.
    pub fn line_col(&self, off: usize) -> (usize, usize) {
        let l = self.line_of(off);
        (l, off.min(self.line_end(l)).saturating_sub(self.starts[l]))
    }
    /// Inverse of [`LineIndex::line_col`] (column clamped to the line).
    pub fn offset(&self, line: usize, col: usize) -> usize {
        let l = line.min(self.starts.len() - 1);
        (self.starts[l] + col).min(self.line_end(l))
    }
    pub fn line_range(&self, line: usize) -> Span {
        Span::new(self.line_start(line), self.line_end(line))
    }
}

fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Leading blanks of the line starting at `start`.
pub fn leading_ws(text: &[u8], start: usize) -> usize {
    text[start.min(text.len())..]
        .iter()
        .take_while(|&&b| is_blank(b))
        .count()
}

fn s(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

// ---------------------------------------------------------------------------------------------
// Comment toggle

/// True when `rest` (a line without leading blanks) starts with the comment marker as a word.
pub fn starts_with_marker(rest: &[u8], marker: &str) -> bool {
    let m = marker.trim().as_bytes();
    rest.len() >= m.len()
        && rest[..m.len()].eq_ignore_ascii_case(m)
        && rest
            .get(m.len())
            .map_or(true, |&b| !b.is_ascii_alphanumeric() && b != b'_')
}

/// Lines `[first, last]` covered by a selection (a selection ending at column 0 of a later line
/// does not include that line).
pub fn selected_lines(idx: &LineIndex, a: usize, b: usize) -> (usize, usize) {
    let (lo, hi) = (a.min(b), a.max(b));
    let first = idx.line_of(lo);
    let mut last = idx.line_of(hi);
    if hi > lo && last > first && hi == idx.line_start(last) {
        last -= 1;
    }
    (first, last)
}

/// Toggle line comments with `marker` (`"REM "`) over the selection.
pub fn toggle_comment(text: &[u8], sel: (usize, usize), marker: &str) -> Transaction {
    let idx = LineIndex::new(text);
    let (first, last) = selected_lines(&idx, sel.0, sel.1);
    let mut all_commented = true;
    let mut any = false;
    for l in first..=last {
        let st = idx.line_start(l);
        let en = idx.line_end(l);
        let ind = leading_ws(&text[..en], st);
        let rest = &text[st + ind..en];
        if rest.is_empty() {
            continue;
        }
        any = true;
        if !starts_with_marker(rest, marker) {
            all_commented = false;
        }
    }
    let region_start = idx.line_start(first);
    let region_end = idx.line_end(last);
    let mut out = String::new();
    let mut delta: isize = 0;
    let mut first_delta: isize = 0;
    for l in first..=last {
        let st = idx.line_start(l);
        let en = idx.line_end(l);
        let ind = leading_ws(&text[..en], st);
        let rest = &text[st + ind..en];
        let mut line = s(&text[st..st + ind]);
        let d: isize;
        if rest.is_empty() {
            line.push_str(&s(rest));
            d = 0;
        } else if any && all_commented {
            let m = marker.trim().len();
            let mut cut = m;
            if rest.get(m) == Some(&b' ') {
                cut += 1;
            }
            line.push_str(&s(&rest[cut..]));
            d = -(cut as isize);
        } else {
            line.push_str(marker);
            line.push_str(&s(rest));
            d = marker.len() as isize;
        }
        if l == first {
            first_delta = d;
        }
        delta += d;
        out.push_str(&line);
        if l != last {
            out.push('\n');
        }
    }
    let _ = region_start;
    // CR bytes (raw mode) are kept: lines were taken without them only when they are not part of
    // the text range, so re-add nothing here (CR-LF files are normalised by the editor).
    let (a, b) = sel;
    let (lo, hi) = (a.min(b), a.max(b));
    let new_lo = (lo as isize + first_delta).max(region_start as isize) as usize;
    let new_hi = (hi as isize + delta).max(new_lo as isize) as usize;
    let select = if a <= b {
        (new_lo, new_hi)
    } else {
        (new_hi, new_lo)
    };
    Transaction::single(Edit::new(region_start, region_end, out), Some(select))
}

// ---------------------------------------------------------------------------------------------
// Indentation

/// Indent (or outdent) every line of the selection by one `unit`.
pub fn indent_lines(text: &[u8], sel: (usize, usize), unit: &str, outdent: bool) -> Transaction {
    let idx = LineIndex::new(text);
    let (first, last) = selected_lines(&idx, sel.0, sel.1);
    let region_start = idx.line_start(first);
    let region_end = idx.line_end(last);
    let mut out = String::new();
    let mut deltas = vec![];
    for l in first..=last {
        let st = idx.line_start(l);
        let en = idx.line_end(l);
        let line = &text[st..en];
        if outdent {
            let cut = if line.starts_with(unit.as_bytes()) {
                unit.len()
            } else if line.first() == Some(&b'\t') {
                1
            } else {
                line.iter()
                    .take(unit.len().max(4))
                    .take_while(|&&b| b == b' ')
                    .count()
            };
            out.push_str(&s(&line[cut..]));
            deltas.push(-(cut as isize));
        } else if line.is_empty() {
            deltas.push(0);
        } else {
            out.push_str(unit);
            out.push_str(&s(line));
            deltas.push(unit.len() as isize);
        }
        if l != last {
            out.push('\n');
        }
    }
    let total: isize = deltas.iter().sum();
    let (a, b) = sel;
    let (lo, hi) = (a.min(b), a.max(b));
    let new_lo = (lo as isize + deltas[0]).max(region_start as isize) as usize;
    let new_hi = (hi as isize + total).max(new_lo as isize) as usize;
    let select = if a <= b {
        (new_lo, new_hi)
    } else {
        (new_hi, new_lo)
    };
    Transaction::single(Edit::new(region_start, region_end, out), Some(select))
}

/// Enter pressed with the selection `sel`. The new line copies the indentation of the current
/// line and adds one `unit` after a block opener (`block_delta > 0`). Typing Enter on a closing
/// line (`ENDIF`) first outdents it by one unit.
pub fn newline(text: &[u8], sel: (usize, usize), unit: &str, block_delta: i32) -> Transaction {
    let (lo, hi) = (sel.0.min(sel.1), sel.0.max(sel.1));
    let idx = LineIndex::new(text);
    let l = idx.line_of(lo);
    let st = idx.line_start(l);
    let en = idx.line_end(l);
    let ind = leading_ws(&text[..en], st).min(lo - st);
    let mut indent = s(&text[st..st + ind]);
    let at_end = text[hi.min(en)..en].iter().all(|&b| is_blank(b));
    if block_delta < 0 && at_end && lo >= st + leading_ws(&text[..en], st) {
        if let Some(p) = indent.strip_suffix(unit) {
            let dedented = p.to_string();
            // rewrite the current line's indentation, then break the line
            let full_ind = leading_ws(&text[..en], st);
            let new_line_start = format!("{}{}", dedented, s(&text[st + full_ind..hi]));
            let ins = format!("{}\n{}", new_line_start, dedented);
            let caret = st + ins.chars().count();
            return Transaction::single(Edit::new(st, hi, ins), Some((caret, caret)));
        }
    }
    if block_delta > 0 {
        indent.push_str(unit);
    }
    let ins = format!("\n{indent}");
    let caret = lo + ins.chars().count();
    Transaction::single(Edit::new(lo, hi, ins), Some((caret, caret)))
}

// ---------------------------------------------------------------------------------------------
// Completion and snippets

/// A snippet template after parsing: plain text plus where Tab stops are (character offsets).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedSnippet {
    pub text: String,
    /// `${n:default}` placeholders in ascending `n`.
    pub stops: Vec<(usize, usize)>,
    /// `$0`.
    pub end: Option<usize>,
}

/// Parse `${n:default}`, `${n}`, `$n` and `$0`. `\$` is a literal dollar sign. Placeholders must
/// not nest. Only the first `${n:..}` of a number is a Tab stop; repeats are plain text.
pub fn parse_snippet(template: &str) -> ParsedSnippet {
    let cs: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut n_out = 0usize;
    let mut stops: Vec<(u32, usize, usize)> = vec![];
    let mut end = None;
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' && cs.get(i + 1) == Some(&'$') {
            out.push('$');
            n_out += 1;
            i += 2;
        } else if c == '$' && cs.get(i + 1) == Some(&'{') {
            // ${n:default} or ${n}
            let mut j = i + 2;
            let mut num = String::new();
            while j < cs.len() && cs[j].is_ascii_digit() {
                num.push(cs[j]);
                j += 1;
            }
            let mut def = String::new();
            let mut ok = !num.is_empty();
            if ok && cs.get(j) == Some(&':') {
                j += 1;
                while j < cs.len() && cs[j] != '}' {
                    def.push(cs[j]);
                    j += 1;
                }
            }
            ok = ok && cs.get(j) == Some(&'}');
            if ok {
                let n: u32 = num.parse().unwrap_or(0);
                let st = n_out;
                out.push_str(&def);
                n_out += def.chars().count();
                if n == 0 {
                    end.get_or_insert(st);
                } else if !stops.iter().any(|s| s.0 == n) {
                    // a repeated number is plain text (no mirroring)
                    stops.push((n, st, n_out));
                }
                i = j + 1;
            } else {
                out.push(c);
                n_out += 1;
                i += 1;
            }
        } else if c == '$' && cs.get(i + 1).map_or(false, |d| d.is_ascii_digit()) {
            let n = cs[i + 1].to_digit(10).unwrap_or(0);
            if n == 0 {
                end.get_or_insert(n_out);
            } else {
                stops.push((n, n_out, n_out));
            }
            i += 2;
        } else {
            out.push(c);
            n_out += 1;
            i += 1;
        }
    }
    stops.sort_by_key(|s| s.0);
    ParsedSnippet {
        text: out,
        stops: stops.into_iter().map(|(_, a, b)| (a, b)).collect(),
        end,
    }
}

/// The template with every placeholder replaced by its default (what gets validated in tests).
pub fn snippet_defaults(template: &str) -> String {
    parse_snippet(template).text
}

/// Where the caret and the Tab stops end up after an edit.
#[derive(Clone, Debug, Default)]
pub struct Accepted {
    pub tx: Transaction,
    /// Remaining placeholders (absolute, after the edit), in Tab order.
    pub stops: Vec<(usize, usize)>,
    /// Where the caret goes after the last stop.
    pub end: Option<usize>,
}

fn with_indent(template: &str, indent: &str) -> String {
    if indent.is_empty() {
        template.to_string()
    } else {
        template.replace('\n', &format!("\n{indent}"))
    }
}

fn accepted_from(parsed: ParsedSnippet, edit: Edit, base: usize) -> Accepted {
    let stops: Vec<(usize, usize)> = parsed
        .stops
        .iter()
        .map(|&(a, b)| (base + a, base + b))
        .collect();
    let end = parsed.end.map(|e| base + e);
    let select = match (stops.first(), end) {
        (Some(&f), _) => f,
        (None, Some(e)) => (e, e),
        (None, None) => {
            let e = base + parsed.text.chars().count();
            (e, e)
        }
    };
    let (stops, end) = if stops.is_empty() {
        (vec![], None)
    } else {
        (stops, end.or(Some(base + parsed.text.chars().count())))
    };
    Accepted {
        tx: Transaction::single(edit, Some(select)),
        stops,
        end,
    }
}

/// Edit that accepts completion `c`: the partial word is replaced; a callable item gets its call
/// parentheses; snippets expand their placeholders.
pub fn accept_completion(text: &[u8], c: &Completion, parens: CallParens) -> Accepted {
    let idx = LineIndex::new(text);
    let l = idx.line_of(c.replace.start.min(text.len()));
    let st = idx.line_start(l);
    let ind = leading_ws(text, st);
    let indent = s(&text[st..st + ind]);
    let start = c.replace.start.min(text.len());
    let end = c.replace.end.min(text.len()).max(start);
    if c.kind == CompletionKind::Snippet {
        let parsed = parse_snippet(&with_indent(&c.insert, &indent));
        let edit = Edit::new(start, end, parsed.text.clone());
        return accepted_from(parsed, edit, start);
    }
    let mut ins = c.insert.clone();
    let mut caret = start + ins.chars().count();
    if let Some(has_args) = c.callable {
        if text.get(end) != Some(&b'(') {
            match parens {
                CallParens::Open => {
                    ins.push('(');
                    caret += 1;
                }
                CallParens::Pair => {
                    ins.push_str("()");
                    caret += if has_args { 1 } else { 2 };
                }
            }
        }
    }
    Accepted {
        tx: Transaction::single(Edit::new(start, end, ins), Some((caret, caret))),
        stops: vec![],
        end: None,
    }
}

/// Insert a snippet template at `caret`: on a blank line it replaces the blank, otherwise it goes
/// on a new line below with the same indentation.
pub fn insert_snippet_template(text: &[u8], caret: usize, template: &str) -> Accepted {
    let idx = LineIndex::new(text);
    let l = idx.line_of(caret.min(text.len()));
    let st = idx.line_start(l);
    let en = idx.line_end(l);
    let blank = text[st..en].iter().all(|&b| is_blank(b));
    if blank {
        let indent = s(&text[st..en]);
        let parsed = parse_snippet(&with_indent(template, &indent));
        let base = st + indent.chars().count();
        let edit = Edit::new(st, en, format!("{indent}{}", parsed.text));
        accepted_from(parsed, edit, base)
    } else {
        let ind = leading_ws(&text[..en], st);
        let indent = s(&text[st..st + ind]);
        let parsed = parse_snippet(&with_indent(template, &indent));
        let base = en + 1 + indent.chars().count();
        let edit = Edit::insert(en, format!("\n{indent}{}", parsed.text));
        accepted_from(parsed, edit, base)
    }
}

/// Re-map ranges through the single edit that turned `old` into `new` (the common prefix and
/// suffix delimit the changed region). Ends touching the region follow it, so typing at the end of
/// a placeholder grows it.
pub fn shift_ranges(old: &[u8], new: &[u8], ranges: &mut [(usize, usize)], extra: &mut [usize]) {
    let max_pre = old.len().min(new.len());
    let mut p = 0;
    while p < max_pre && old[p] == new[p] {
        p += 1;
    }
    let mut suf = 0;
    while suf < old.len() - p
        && suf < new.len() - p
        && old[old.len() - 1 - suf] == new[new.len() - 1 - suf]
    {
        suf += 1;
    }
    let old_end = old.len() - suf;
    let new_end = new.len() - suf;
    let delta = new_end as isize - old_end as isize;
    let start_of = |x: usize| -> usize {
        if x <= p {
            x
        } else if x >= old_end {
            (x as isize + delta) as usize
        } else {
            p
        }
    };
    let end_of = |x: usize| -> usize {
        if x < p {
            x
        } else if x >= old_end {
            (x as isize + delta) as usize
        } else {
            new_end
        }
    };
    for r in ranges.iter_mut() {
        *r = (start_of(r.0), end_of(r.1));
    }
    for e in extra.iter_mut() {
        *e = end_of(*e);
    }
}

/// Snippet text for inserting a command from the reference panel: `NAME(a, b)` with the
/// parameter names as placeholders. Returns the text and the first placeholder's span (relative).
pub fn command_snippet(name: &str, params: &[String]) -> (String, Option<(usize, usize)>) {
    if params.is_empty() {
        return (name.to_string(), None);
    }
    let mut t = format!("{name}(");
    let mut first = None;
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            t.push_str(", ");
        }
        let ph: String = p
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let st = t.chars().count();
        t.push_str(&ph);
        if i == 0 {
            first = Some((st, st + ph.chars().count()));
        }
    }
    t.push(')');
    (t, first)
}

/// Insert a snippet at the caret: on a blank line it replaces the blank, otherwise it goes on a
/// new line below, with the same indentation. The first placeholder ends up selected.
pub fn insert_snippet(
    text: &[u8],
    caret: usize,
    snippet: &str,
    first: Option<(usize, usize)>,
) -> Transaction {
    let idx = LineIndex::new(text);
    let l = idx.line_of(caret);
    let st = idx.line_start(l);
    let en = idx.line_end(l);
    let blank = text[st..en].iter().all(|&b| is_blank(b));
    let (edit, base) = if blank {
        let ind = en - st;
        let indent = s(&text[st..en]);
        let _ = ind;
        (
            Edit::new(st, en, format!("{indent}{snippet}")),
            st + indent.chars().count(),
        )
    } else {
        let ind = leading_ws(&text[..en], st);
        let indent = s(&text[st..st + ind]);
        let ins = format!("\n{indent}{snippet}");
        (Edit::insert(en, ins), en + 1 + indent.chars().count())
    };
    let sel = match first {
        Some((a, b)) => (base + a, base + b),
        None => {
            let e = base + snippet.chars().count();
            (e, e)
        }
    };
    Transaction::single(edit, Some(sel))
}

// ---------------------------------------------------------------------------------------------
// Auto-closing pairs and typing-time dedent

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A character typed at `sel`. `Some(transaction)` means the editor handles it (the typed text is
/// not inserted by the text field); `None` means plain insertion.
///
/// * an opener with a selection wraps the selection;
/// * a closer (or quote) typed in front of the same character steps over it;
/// * an opener is closed when the next character is blank, a closer or the end of the text; a
///   quote only after a non-word character (so apostrophes in words stay single).
pub fn type_char(
    text: &[u8],
    sel: (usize, usize),
    ch: char,
    pairs: &[(char, char)],
) -> Option<Transaction> {
    if (ch as u32) > 255 {
        return None;
    }
    let (lo, hi) = (sel.0.min(sel.1), sel.0.max(sel.1));
    let hi = hi.min(text.len());
    let lo = lo.min(hi);
    let next = text.get(hi).copied();
    let prev = if lo > 0 {
        text.get(lo - 1).copied()
    } else {
        None
    };
    let b = ch as u32 as u8;
    let opener = pairs.iter().find(|(o, _)| *o == ch);
    let closer = pairs.iter().find(|(_, c)| *c == ch);
    if let (Some(&(o, c)), true) = (opener, hi > lo) {
        let inner: String = text[lo..hi].iter().map(|&x| x as char).collect();
        let n = inner.chars().count();
        return Some(Transaction::single(
            Edit::new(lo, hi, format!("{o}{inner}{c}")),
            Some((lo + 1, lo + 1 + n)),
        ));
    }
    if hi > lo {
        return None;
    }
    if closer.is_some() && next == Some(b) {
        return Some(Transaction {
            edits: vec![],
            select: Some((lo + 1, lo + 1)),
        });
    }
    if let Some(&(o, c)) = opener {
        let quote = o == c;
        let next_ok = next.map_or(true, |n| n.is_ascii_whitespace() || b")]},;".contains(&n));
        let prev_ok = !quote || prev.map_or(true, |p| !is_word(p) && p != b);
        if next_ok && prev_ok {
            return Some(Transaction::single(
                Edit::insert(lo, format!("{o}{c}")),
                Some((lo + 1, lo + 1)),
            ));
        }
    }
    None
}

/// Backspace between an auto-closed pair (`(|)`) removes both characters.
pub fn backspace_pair(
    text: &[u8],
    sel: (usize, usize),
    pairs: &[(char, char)],
) -> Option<Transaction> {
    if sel.0 != sel.1 || sel.0 == 0 || sel.0 >= text.len() {
        return None;
    }
    let (p, n) = (text[sel.0 - 1] as char, text[sel.0] as char);
    pairs.iter().any(|&(o, c)| o == p && c == n).then(|| {
        Transaction::single(
            Edit::new(sel.0 - 1, sel.0 + 1, ""),
            Some((sel.0 - 1, sel.0 - 1)),
        )
    })
}

/// The line holding the caret has just become a lone closing word (`end`): move it one unit left
/// when it still sits at the indentation the previous block opener gave the body.
/// `prev_delta` is the block delta of the previous non-blank line.
pub fn dedent_closing_line(
    text: &[u8],
    caret: usize,
    unit: &str,
    prev_delta: i32,
) -> Option<Transaction> {
    let idx = LineIndex::new(text);
    let l = idx.line_of(caret.min(text.len()));
    if l == 0 {
        return None;
    }
    let st = idx.line_start(l);
    let en = idx.line_end(l);
    let ind = leading_ws(&text[..en], st);
    // previous non-blank line
    let mut pl = l;
    let prev = loop {
        if pl == 0 {
            return None;
        }
        pl -= 1;
        let (a, b) = (idx.line_start(pl), idx.line_end(pl));
        if text[a..b].iter().any(|&x| !is_blank(x)) {
            break (a, b);
        }
    };
    let prev_ind = &text[prev.0..prev.0 + leading_ws(&text[..prev.1], prev.0)];
    let expected_len = prev_ind.len() + if prev_delta > 0 { unit.len() } else { 0 };
    if ind != expected_len || ind < unit.len() || !text[st..st + ind].ends_with(unit.as_bytes()) {
        return None;
    }
    let new_caret = caret.saturating_sub(unit.len());
    Some(Transaction::single(
        Edit::new(st, st + ind, s(&text[st..st + ind - unit.len()])),
        Some((new_caret, new_caret)),
    ))
}

// ---------------------------------------------------------------------------------------------
// Find / replace

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub case_sensitive: bool,
}

fn matches_at(text: &[u8], needle: &[u8], i: usize, opts: FindOptions) -> bool {
    if i + needle.len() > text.len() {
        return false;
    }
    let w = &text[i..i + needle.len()];
    if opts.case_sensitive {
        w == needle
    } else {
        w.eq_ignore_ascii_case(needle)
    }
}

/// All non-overlapping matches.
pub fn find_all(text: &[u8], needle: &[u8], opts: FindOptions) -> Vec<Span> {
    let mut out = vec![];
    if needle.is_empty() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= text.len() {
        if matches_at(text, needle, i, opts) {
            out.push(Span::new(i, i + needle.len()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// First match starting at or after `from`, wrapping around.
pub fn find_next(text: &[u8], needle: &[u8], from: usize, opts: FindOptions) -> Option<Span> {
    let all = find_all(text, needle, opts);
    all.iter()
        .find(|m| m.start >= from)
        .or_else(|| all.first())
        .copied()
}

/// Last match that ends at or before `before`, wrapping around.
pub fn find_prev(text: &[u8], needle: &[u8], before: usize, opts: FindOptions) -> Option<Span> {
    let all = find_all(text, needle, opts);
    all.iter()
        .rev()
        .find(|m| m.end <= before)
        .or_else(|| all.last())
        .copied()
}

/// Replace every match; returns the transaction and the match count.
pub fn replace_all(
    text: &[u8],
    needle: &[u8],
    with: &str,
    opts: FindOptions,
) -> Option<(Transaction, usize)> {
    let all = find_all(text, needle, opts);
    if all.is_empty() {
        return None;
    }
    let edits = all
        .iter()
        .map(|m| Edit::new(m.start, m.end, with))
        .collect();
    Some((
        Transaction {
            edits,
            select: None,
        },
        all.len(),
    ))
}

/// Replace the match the selection sits on (if it is one), then return the transaction; the
/// caller advances to the next match.
pub fn replace_current(
    text: &[u8],
    needle: &[u8],
    with: &str,
    sel: (usize, usize),
    opts: FindOptions,
) -> Option<Transaction> {
    let (lo, hi) = (sel.0.min(sel.1), sel.0.max(sel.1));
    if needle.is_empty() || hi - lo != needle.len() || !matches_at(text, needle, lo, opts) {
        return None;
    }
    let end = lo + with.chars().count();
    Some(Transaction::single(
        Edit::new(lo, hi, with),
        Some((end, end)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, tx: &Transaction) -> String {
        let mut b: Vec<u8> = text.bytes().collect();
        apply(&mut b, tx);
        String::from_utf8(b).unwrap()
    }

    #[test]
    fn line_index_crlf() {
        let t = b"IF(A)\r\n\tFOO\r\nENDIF";
        let li = LineIndex::new(t);
        assert_eq!(li.line_count(), 3);
        assert_eq!(li.line_range(0), Span::new(0, 5));
        assert_eq!(li.line_range(1), Span::new(7, 11));
        assert_eq!(li.line_range(2), Span::new(13, 18));
        assert_eq!(li.line_col(8), (1, 1));
        assert_eq!(li.line_col(5), (0, 5));
        // inside the CR LF pair: clamped to the line end
        assert_eq!(li.line_col(6), (0, 5));
        assert_eq!(li.line_col(7), (1, 0));
        assert_eq!(li.offset(1, 99), 11);
        assert_eq!(li.offset(2, 2), 15);
        assert_eq!(&t[li.line_range(1).start..li.line_range(1).end], b"\tFOO");
    }

    #[test]
    fn line_index_lf_and_empty() {
        let li = LineIndex::new(b"");
        assert_eq!(li.line_count(), 1);
        assert_eq!(li.line_col(0), (0, 0));
        let li = LineIndex::new(b"a\n\nb\n");
        assert_eq!(li.line_count(), 4);
        assert_eq!(li.line_range(1), Span::new(2, 2));
        assert_eq!(li.line_range(3), Span::new(5, 5));
    }

    #[test]
    fn comment_toggle_single_and_block() {
        let t = "IF(A)\n\tFOO(1)\n\tBAR(2)\nENDIF";
        let tx = toggle_comment(t.as_bytes(), (7, 7), "REM ");
        assert_eq!(run(t, &tx), "IF(A)\n\tREM FOO(1)\n\tBAR(2)\nENDIF");
        let tx2 = toggle_comment(run(t, &tx).as_bytes(), (7, 7), "REM ");
        assert_eq!(run(&run(t, &tx), &tx2), t);
        // two lines
        let tx = toggle_comment(t.as_bytes(), (8, 16), "REM ");
        assert_eq!(run(t, &tx), "IF(A)\n\tREM FOO(1)\n\tREM BAR(2)\nENDIF");
        // selection ending at column 0 excludes that line
        let tx = toggle_comment(t.as_bytes(), (6, 13), "REM ");
        assert_eq!(run(t, &tx), "IF(A)\n\tREM FOO(1)\n\tBAR(2)\nENDIF");
        // blank lines are left alone, mixed selection comments everything
        let t2 = "REM a\n\nB";
        let tx = toggle_comment(t2.as_bytes(), (0, t2.len()), "REM ");
        assert_eq!(run(t2, &tx), "REM REM a\n\nREM B");
        // word boundary: REMOVE is not a comment
        let t3 = "REMOVE(1)";
        let tx = toggle_comment(t3.as_bytes(), (0, 0), "REM ");
        assert_eq!(run(t3, &tx), "REM REMOVE(1)");
    }

    #[test]
    fn comment_toggle_selection_follows() {
        let t = "FOO\nBAR";
        let tx = toggle_comment(t.as_bytes(), (1, 5), "REM ");
        assert_eq!(run(t, &tx), "REM FOO\nREM BAR");
        assert_eq!(tx.select, Some((5, 13)));
    }

    #[test]
    fn auto_indent_after_if() {
        let t = "IF(PLAYER0, FLAG0 == 1)";
        let tx = newline(t.as_bytes(), (t.len(), t.len()), "\t", 1);
        assert_eq!(run(t, &tx), "IF(PLAYER0, FLAG0 == 1)\n\t");
        assert_eq!(tx.select, Some((t.len() + 2, t.len() + 2)));
        // keeps the indentation of a normal line
        let t = "IF(A)\n\tFOO(1)";
        let tx = newline(t.as_bytes(), (t.len(), t.len()), "\t", 0);
        assert_eq!(run(t, &tx), "IF(A)\n\tFOO(1)\n\t");
        // splits a line in the middle
        let t = "\tFOO(1)";
        let tx = newline(t.as_bytes(), (4, 4), "\t", 0);
        assert_eq!(run(t, &tx), "\tFOO\n\t(1)");
    }

    #[test]
    fn auto_dedent_endif() {
        let t = "IF(A)\n\tENDIF";
        let tx = newline(t.as_bytes(), (t.len(), t.len()), "\t", -1);
        assert_eq!(run(t, &tx), "IF(A)\nENDIF\n");
        let c = tx.select.unwrap().0;
        assert_eq!(c, run(t, &tx).len());
        // nested
        let t = "IF(A)\n\tIF(B)\n\t\tX\n\t\tENDIF";
        let tx = newline(t.as_bytes(), (t.len(), t.len()), "\t", -1);
        assert_eq!(run(t, &tx), "IF(A)\n\tIF(B)\n\t\tX\n\tENDIF\n\t");
    }

    #[test]
    fn indent_outdent() {
        let t = "A\n\tB\nC";
        let tx = indent_lines(t.as_bytes(), (0, t.len()), "\t", false);
        assert_eq!(run(t, &tx), "\tA\n\t\tB\n\tC");
        let u = run(t, &tx);
        let tx = indent_lines(u.as_bytes(), (0, u.len()), "\t", true);
        assert_eq!(run(&u, &tx), t);
    }

    fn comp(text: &str, replace: (usize, usize), insert: &str, kind: CompletionKind) -> Completion {
        let _ = text;
        Completion::new(insert, kind, insert, Span::new(replace.0, replace.1))
    }
    fn callable(mut c: Completion) -> Completion {
        c.callable = Some(true);
        c
    }

    #[test]
    fn completion_replaces_partial_word() {
        let t = "ADD_CR";
        let c = callable(comp(
            t,
            (0, 6),
            "ADD_CREATURE_TO_LEVEL",
            CompletionKind::Command,
        ));
        let tx = accept_completion(t.as_bytes(), &c, CallParens::Open).tx;
        assert_eq!(run(t, &tx), "ADD_CREATURE_TO_LEVEL(");
        assert_eq!(tx.select, Some((22, 22)));
        // existing paren is not duplicated; word after the caret is replaced too
        let t = "ADD_CRx(1)";
        let c = callable(comp(
            t,
            (0, 7),
            "ADD_CREATURE_TO_LEVEL",
            CompletionKind::Command,
        ));
        let tx = accept_completion(t.as_bytes(), &c, CallParens::Open).tx;
        assert_eq!(run(t, &tx), "ADD_CREATURE_TO_LEVEL(1)");
        // command without arguments gets no paren
        let t = "WIN";
        let c = comp(t, (0, 3), "WIN_GAME", CompletionKind::Command);
        assert_eq!(
            run(t, &accept_completion(t.as_bytes(), &c, CallParens::Open).tx),
            "WIN_GAME"
        );
        // constant inside an argument list
        let t = "ADD_CREATURE_TO_LEVEL(PLAYER0, DRA";
        let c = comp(t, (31, 34), "DRAGON", CompletionKind::Constant);
        let tx = accept_completion(t.as_bytes(), &c, CallParens::Open).tx;
        assert_eq!(run(t, &tx), "ADD_CREATURE_TO_LEVEL(PLAYER0, DRAGON");
        assert_eq!(tx.select, Some((37, 37)));
    }

    #[test]
    fn snippet_completion_indents_and_places_caret() {
        let t = "\tIF";
        let c = comp(t, (1, 3), "IF(A)\n\t$0\nENDIF", CompletionKind::Snippet);
        let tx = accept_completion(t.as_bytes(), &c, CallParens::Open).tx;
        assert_eq!(run(t, &tx), "\tIF(A)\n\t\t\n\tENDIF");
        assert_eq!(tx.select, Some((9, 9)));
    }

    #[test]
    fn command_snippet_placeholders() {
        let (t, first) = command_snippet(
            "ADD_CREATURE_TO_LEVEL",
            &["player".into(), "creature".into(), "action point".into()],
        );
        assert_eq!(t, "ADD_CREATURE_TO_LEVEL(player, creature, action_point)");
        assert_eq!(first, Some((22, 28)));
        assert_eq!(command_snippet("ENDIF", &[]), ("ENDIF".to_string(), None));
    }

    #[test]
    fn snippet_insertion_positions() {
        let (sn, first) = command_snippet(
            "SET_FLAG",
            &["player".into(), "flag".into(), "value".into()],
        );
        // blank line: replaced in place
        let t = "A\n\t\nB";
        let tx = insert_snippet(t.as_bytes(), 3, &sn, first);
        assert_eq!(run(t, &tx), "A\n\tSET_FLAG(player, flag, value)\nB");
        let (a, b) = tx.select.unwrap();
        assert_eq!(&run(t, &tx)[a..b], "player");
        // non-blank line: new line below with the same indentation
        let t = "IF(X)\n\tFOO(1)\nENDIF";
        let tx = insert_snippet(t.as_bytes(), 8, &sn, first);
        assert_eq!(
            run(t, &tx),
            "IF(X)\n\tFOO(1)\n\tSET_FLAG(player, flag, value)\nENDIF"
        );
        let (a, b) = tx.select.unwrap();
        assert_eq!(&run(t, &tx)[a..b], "player");
    }

    #[test]
    fn find_replace() {
        let t = b"set_flag Set_Flag SET_FLAG xSET_FLAGx";
        assert_eq!(
            find_all(
                t,
                b"SET_FLAG",
                FindOptions {
                    case_sensitive: false
                }
            )
            .len(),
            4
        );
        assert_eq!(
            find_all(
                t,
                b"SET_FLAG",
                FindOptions {
                    case_sensitive: true
                }
            )
            .len(),
            2
        );
        let o = FindOptions::default();
        assert_eq!(find_next(t, b"set_flag", 1, o), Some(Span::new(9, 17)));
        assert_eq!(find_next(t, b"set_flag", 30, o), Some(Span::new(0, 8))); // wraps
        assert_eq!(find_prev(t, b"set_flag", 9, o), Some(Span::new(0, 8)));
        assert_eq!(find_prev(t, b"set_flag", 0, o), Some(Span::new(28, 36))); // wraps
        let (tx, n) = replace_all(t, b"set_flag", "X", o).unwrap();
        assert_eq!(n, 4);
        assert_eq!(run(std::str::from_utf8(t).unwrap(), &tx), "X X X xXx");
        assert!(replace_all(t, b"zzz", "X", o).is_none());
        assert!(find_all(t, b"", o).is_empty());
        let tx = replace_current(t, b"set_flag", "ab", (9, 17), o).unwrap();
        assert_eq!(
            run(std::str::from_utf8(t).unwrap(), &tx),
            "set_flag ab SET_FLAG xSET_FLAGx"
        );
        assert!(replace_current(t, b"set_flag", "ab", (0, 3), o).is_none());
    }

    const PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];

    fn typed(
        text: &str,
        sel: (usize, usize),
        ch: char,
    ) -> Option<(String, Option<(usize, usize)>)> {
        type_char(text.as_bytes(), sel, ch, PAIRS).map(|tx| (run(text, &tx), tx.select))
    }

    #[test]
    fn auto_close_rules() {
        // opener at the end / before blank / before a closer closes itself
        assert_eq!(
            typed("foo", (3, 3), '('),
            Some(("foo()".into(), Some((4, 4))))
        );
        assert_eq!(
            typed("a = ", (4, 4), '{'),
            Some(("a = {}".into(), Some((5, 5))))
        );
        assert_eq!(
            typed("f()", (2, 2), '['),
            Some(("f([])".into(), Some((3, 3))))
        );
        // not before a word character
        assert_eq!(typed("foo", (0, 0), '('), None);
        // closing character steps over an existing one
        assert_eq!(
            typed("f()", (2, 2), ')'),
            Some(("f()".into(), Some((3, 3))))
        );
        assert_eq!(typed("f()", (1, 1), ')'), None);
        // a quote: after a non-word character it pairs, after a word it does not (apostrophe)
        assert_eq!(
            typed("f(", (2, 2), '"'),
            Some(("f(\"\"".into(), Some((3, 3))))
        );
        assert_eq!(typed("don", (3, 3), '\''), None);
        // a second quote steps over the closing quote
        assert_eq!(
            typed("f(\"\")", (3, 3), '"'),
            Some(("f(\"\")".into(), Some((4, 4))))
        );
        // with a selection an opener wraps it
        assert_eq!(
            typed("abc", (0, 3), '"'),
            Some(("\"abc\"".into(), Some((1, 4))))
        );
        assert_eq!(
            typed("abc def", (4, 7), '('),
            Some(("abc (def)".into(), Some((5, 8))))
        );
        // other characters are not handled
        assert_eq!(typed("x", (1, 1), 'y'), None);
        assert_eq!(typed("x", (1, 1), '\u{20ac}'), None);
    }

    #[test]
    fn backspace_removes_an_empty_pair() {
        let t = "f()";
        let tx = backspace_pair(t.as_bytes(), (2, 2), PAIRS).unwrap();
        assert_eq!(run(t, &tx), "f");
        assert_eq!(tx.select, Some((1, 1)));
        assert!(backspace_pair(b"f(x)", (2, 2), PAIRS).is_none());
        assert!(backspace_pair(b"f()", (3, 3), PAIRS).is_none());
        let t = "\"\"";
        assert_eq!(
            run(t, &backspace_pair(t.as_bytes(), (1, 1), PAIRS).unwrap()),
            ""
        );
    }

    #[test]
    fn closing_word_dedents_once() {
        let u = "\t";
        // body of an `if` just opened: `end` moves out
        let t = "if x then\n\tend";
        let tx = dedent_closing_line(t.as_bytes(), t.len(), u, 1).unwrap();
        assert_eq!(run(t, &tx), "if x then\nend");
        assert_eq!(tx.select, Some((13, 13)));
        // after a body statement
        let t = "if x then\n\tfoo()\n\tend";
        assert_eq!(
            run(
                t,
                &dedent_closing_line(t.as_bytes(), t.len(), u, 0).unwrap()
            ),
            "if x then\n\tfoo()\nend"
        );
        // already at the outer level: nothing
        let t = "if x then\nend";
        assert!(dedent_closing_line(t.as_bytes(), t.len(), u, 1).is_none());
        // `elseif` after `else` keeps its dedent
        let t = "if x then\nelse";
        assert!(dedent_closing_line(t.as_bytes(), t.len(), u, 1).is_none());
    }

    #[test]
    fn snippet_parsing() {
        let p = parse_snippet("f(${1:a}, ${2:bb}) $0");
        assert_eq!(p.text, "f(a, bb) ");
        assert_eq!(p.stops, vec![(2, 3), (5, 7)]);
        assert_eq!(p.end, Some(9));
        // numbers decide the order, repeats are plain text, bare $n is an empty stop, \$ is literal
        let p = parse_snippet("${2:b} ${1:a} ${1:a} $3 \\$x");
        assert_eq!(p.text, "b a a  $x");
        assert_eq!(p.stops, vec![(2, 3), (0, 1), (6, 6)]);
        assert_eq!(p.end, None);
        assert_eq!(snippet_defaults("A(${1:x})\n\t$0"), "A(x)\n\t");
    }

    #[test]
    fn range_shifting_follows_a_single_edit() {
        // typing inside the first range grows it and moves the second
        let mut r = [(2, 5), (7, 9)];
        let mut end = [10];
        shift_ranges(b"f(abc, de)", b"f(abXYc, de)", &mut r, &mut end);
        assert_eq!(r, [(2, 7), (9, 11)]);
        assert_eq!(end, [12]);
        // typing at the end of a range extends it
        let mut r = [(2, 5)];
        shift_ranges(b"f(abc)", b"f(abcd)", &mut r, &mut []);
        assert_eq!(r, [(2, 6)]);
        // typing right before the second range does not grow the first
        let mut r = [(2, 3), (3, 4)];
        shift_ranges(b"f(ab)", b"f(aXb)", &mut r, &mut []);
        assert_eq!(r[0].0, 2);
        // replacing the whole content
        let mut r = [(2, 5)];
        shift_ranges(b"f(abc)", b"f(z)", &mut r, &mut []);
        assert_eq!(r, [(2, 3)]);
    }

    #[test]
    fn accept_callable_writes_parens() {
        let mut c = comp("Sta", (0, 3), "StartMoney", CompletionKind::Command);
        c.callable = Some(true);
        let a = accept_completion(b"Sta", &c, CallParens::Pair);
        assert_eq!(run("Sta", &a.tx), "StartMoney()");
        assert_eq!(a.tx.select, Some((11, 11)));
        c.callable = Some(false);
        let a = accept_completion(b"Sta", &c, CallParens::Pair);
        assert_eq!(a.tx.select, Some((12, 12)));
        // existing parentheses are kept
        let a = accept_completion(b"Sta(1)", &c, CallParens::Pair);
        assert_eq!(run("Sta(1)", &a.tx), "StartMoney(1)");
    }

    #[test]
    fn snippet_template_insertion_reports_stops() {
        let t = "function F()\n\t\nend";
        let a = insert_snippet_template(t.as_bytes(), 14, "if ${1:x} then\n\t${2:y}\nend");
        assert_eq!(
            run(t, &a.tx),
            "function F()\n\tif x then\n\t\ty\n\tend\nend"
        );
        assert_eq!(a.stops.len(), 2);
        assert_eq!(a.tx.select, Some(a.stops[0]));
        let out = run(t, &a.tx);
        assert_eq!(&out[a.stops[0].0..a.stops[0].1], "x");
        assert_eq!(&out[a.stops[1].0..a.stops[1].1], "y");
    }
}
