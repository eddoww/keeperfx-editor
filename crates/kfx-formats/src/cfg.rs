//! Lossless model of the KeeperFX `.cfg` dialect (fxdata/*.cfg, creatrs/*.cfg, campaign cfgs).
//!
//! # The dialect (confirmed from the game's parser)
//!
//! Reference: `src/config.c` `skip_conf_to_next_line` (152), `find_conf_block` (186),
//! `iterate_conf_blocks` (253), `recognize_conf_command` (315), `get_conf_parameter_whole` (1091),
//! `get_conf_parameter_single` (1113), `parse_named_field_block` (984).
//!
//! * Line oriented. A line ends at `\n` or `\r` (CRLF, LF and lone CR all work).
//! * Leading blanks are skipped: space, TAB, `\r`, `\n`, byte 26 (Ctrl-Z) and any byte `< 7`.
//! * A line whose first non-blank byte is `;` is a comment. There are **no inline comments**:
//!   `KEY = 1 ; x` hands `;` and `x` to the value parser as ordinary tokens.
//! * A line whose first non-blank byte is `[` starts a section: `[` blanks name blanks `]`.
//!   Section names are matched case-insensitively (`strncasecmp`). Anything after `]` on the line
//!   is ignored. Section names for numbered blocks are letters+digits (`conf_get_block_name`). The
//!   same section name may appear several times (`iterate_conf_blocks` sees every one,
//!   `find_conf_block` only the first).
//! * Any other line is `KEY<sep>value`: the key runs up to a space, TAB, `=` or a byte `< 7`;
//!   the separator is any run of spaces, TABs, `=` and bytes `< 7` (so `A==B` and `A = B` and
//!   `A B` are equivalent). Keys are matched case-insensitively and must be followed by one of
//!   those separator bytes (or end of line).
//! * No quoting and no escapes. The value is either split on blanks into tokens
//!   (`get_conf_parameter_single`: space/TAB separated) or taken whole to end of line
//!   (`get_conf_parameter_whole`, used for names / paths / text). Trailing blanks are not part of
//!   the value in practice (whole-line values may contain them in the game, here they are kept
//!   apart as `trail` so they survive edits unchanged).
//! * A key may repeat; for named-field loaders each occurrence is assigned in turn, so the last
//!   one wins for scalars, and list-like keys (e.g. `LEVELS_LOCATION` is scalar, but
//!   `Spell`-style repeated keys) accumulate. [`CfgDoc::get`] returns the last occurrence,
//!   [`CfgDoc::get_all`] all of them.
//! * Text before the first `[section]` belongs to the "preamble" (section name `""`). The game
//!   scans for a given block with `find_conf_block`, so preamble keys are ignored by block loaders,
//!   but `cmpgn_common_commands` style files (campaign cfg) are read with a
//!   `recognize_conf_command` loop starting at the file start, hence the preamble is exposed.
//!
//! # Encoding choice
//!
//! Files are decoded as **Latin-1** (every byte maps to the same code point), so any byte
//! sequence (CP1252 / UTF-8 / garbage) round-trips exactly through [`CfgDoc::to_bytes`].
//! [`CfgDoc::to_string`] (via `Display`) returns that Latin-1 view. The game itself treats the
//! bytes as single-byte characters, so no UTF-8 interpretation is attempted.
use crate::{rnc, Error, Result};
use std::fmt;
use std::path::Path;

/// Decode bytes as Latin-1 (lossless).
pub fn latin1_decode(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// Encode a string produced by [`latin1_decode`] back to bytes. Code points above U+00FF (only
/// possible when the caller put them in) are written as `?`.
pub fn latin1_encode(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| {
            if (c as u32) < 256 {
                c as u32 as u8
            } else {
                b'?'
            }
        })
        .collect()
}

fn is_lead_blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n') || (c as u32) == 26 || (c as u32) < 7
}
fn is_sep(c: char) -> bool {
    matches!(c, ' ' | '\t' | '=') || (c as u32) < 7
}
fn is_trail_blank(c: char) -> bool {
    matches!(c, ' ' | '\t') || (c as u32) == 26
}

/// What a physical line is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Blank,
    Comment,
    /// `name` is the trimmed text between `[` and `]` (or to end of line when unterminated).
    Section {
        name: String,
    },
    /// `text = indent + key + sep + value + trail`.
    Entry {
        indent: usize,
        key_len: usize,
        sep_len: usize,
        trail_len: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    /// Line text without the line terminator.
    text: String,
    /// Line terminator as found ("", "\n", "\r\n", "\r").
    eol: String,
    kind: Kind,
}

impl Line {
    fn classify(text: String, eol: String) -> Line {
        let chars: Vec<char> = text.chars().collect();
        let kind = classify(&chars);
        Line { text, eol, kind }
    }
}

fn classify(chars: &[char]) -> Kind {
    let mut i = 0;
    while i < chars.len() && is_lead_blank(chars[i]) {
        i += 1;
    }
    if i >= chars.len() {
        return Kind::Blank;
    }
    match chars[i] {
        ';' => Kind::Comment,
        '[' => {
            let rest: String = chars[i + 1..].iter().collect();
            let inner = match rest.find(']') {
                Some(p) => &rest[..p],
                None => &rest[..],
            };
            Kind::Section {
                name: inner.trim_matches(|c: char| is_lead_blank(c)).to_string(),
            }
        }
        _ => {
            let mut k = i;
            while k < chars.len() && !is_sep(chars[k]) {
                k += 1;
            }
            let mut s = k;
            while s < chars.len() && is_sep(chars[s]) {
                s += 1;
            }
            let mut end = chars.len();
            while end > s && is_trail_blank(chars[end - 1]) {
                end -= 1;
            }
            Kind::Entry {
                indent: i,
                key_len: k - i,
                sep_len: s - k,
                trail_len: chars.len() - end,
            }
        }
    }
}

/// Read-only view of one `KEY value` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Key spelling as written.
    pub key: String,
    /// Everything after the separator, without trailing blanks. Whole-line value.
    pub value: String,
    /// `value` split on spaces and TABs (the game's `get_conf_parameter_single`).
    pub tokens: Vec<String>,
}

/// Read-only view of one section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// Name as written (`""` for the preamble before the first section).
    pub name: String,
    pub entries: Vec<Entry>,
}

/// Split a value into blank separated tokens like `get_conf_parameter_single`.
pub fn split_tokens(v: &str) -> Vec<String> {
    v.split(|c: char| c == ' ' || c == '\t' || (c as u32) < 7)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// A lossless `.cfg` document. `to_bytes()` of an unmodified parse is byte-identical to the input.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CfgDoc {
    lines: Vec<Line>,
}

impl fmt::Display for CfgDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for l in &self.lines {
            f.write_str(&l.text)?;
            f.write_str(&l.eol)?;
        }
        Ok(())
    }
}

fn entry_of(l: &Line) -> Option<Entry> {
    if let Kind::Entry {
        indent,
        key_len,
        sep_len,
        trail_len,
    } = l.kind
    {
        let chars: Vec<char> = l.text.chars().collect();
        let key: String = chars[indent..indent + key_len].iter().collect();
        let vs = indent + key_len + sep_len;
        let value: String = chars[vs..chars.len() - trail_len].iter().collect();
        let tokens = split_tokens(&value);
        Some(Entry { key, value, tokens })
    } else {
        None
    }
}

impl CfgDoc {
    /// Parse Latin-1 text (see [`latin1_decode`]). Never fails.
    pub fn parse(text: &str) -> CfgDoc {
        let mut lines = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let start = i;
            while i < chars.len() && chars[i] != '\n' && chars[i] != '\r' {
                i += 1;
            }
            let body: String = chars[start..i].iter().collect();
            let mut eol = String::new();
            if i < chars.len() {
                if chars[i] == '\r' && i + 1 < chars.len() && chars[i + 1] == '\n' {
                    eol.push_str("\r\n");
                    i += 2;
                } else {
                    eol.push(chars[i]);
                    i += 1;
                }
            }
            lines.push(Line::classify(body, eol));
        }
        CfgDoc { lines }
    }

    /// Parse raw file bytes (decoded as Latin-1).
    pub fn from_bytes(b: &[u8]) -> CfgDoc {
        CfgDoc::parse(&latin1_decode(b))
    }

    /// Load through [`rnc::load_file`] (handles RNC-packed files).
    pub fn load(path: &Path) -> Result<CfgDoc> {
        Ok(CfgDoc::from_bytes(&rnc::load_file(path)?))
    }

    /// Write back to disk (bytes exactly as the document renders).
    pub fn save(&self, path: &Path) -> Result<()> {
        crate::atomic::atomic_write(path, &self.to_bytes()).map_err(|e| Error::io(path, e))
    }

    /// Exact bytes of the document.
    pub fn to_bytes(&self) -> Vec<u8> {
        latin1_encode(&self.to_string())
    }

    /// Number of physical lines.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Dominant line terminator, used for newly inserted lines ("\n" when the file has none).
    fn dominant_eol(&self) -> String {
        let (mut crlf, mut lf) = (0, 0);
        for l in &self.lines {
            match l.eol.as_str() {
                "\r\n" => crlf += 1,
                "\n" => lf += 1,
                _ => {}
            }
        }
        if crlf > lf {
            "\r\n".into()
        } else {
            "\n".into()
        }
    }

    /// Line range `[start, end)` of the body of the first section called `name`
    /// (case-insensitive; `""` = preamble). `start` is the line after the header.
    fn body_range(&self, name: &str) -> Option<(usize, usize)> {
        self.body_ranges(name).into_iter().next()
    }

    fn body_ranges(&self, name: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut cur: Option<(usize, String)> = Some((0, String::new()));
        let finish =
            |cur: &mut Option<(usize, String)>, end: usize, out: &mut Vec<(usize, usize)>| {
                if let Some((s, n)) = cur.take() {
                    if n.eq_ignore_ascii_case(name) {
                        out.push((s, end));
                    }
                }
            };
        for (i, l) in self.lines.iter().enumerate() {
            if let Kind::Section { name: n } = &l.kind {
                finish(&mut cur, i, &mut out);
                cur = Some((i + 1, n.clone()));
            }
        }
        finish(&mut cur, self.lines.len(), &mut out);
        out
    }

    /// All section names in file order (duplicates included, preamble excluded).
    pub fn section_names(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter_map(|l| {
                if let Kind::Section { name } = &l.kind {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn has_section(&self, name: &str) -> bool {
        !name.is_empty() && self.body_range(name).is_some()
    }

    /// Every section (the preamble first, even if empty) with its entries.
    pub fn sections(&self) -> Vec<Section> {
        let mut out = vec![Section {
            name: String::new(),
            entries: vec![],
        }];
        for l in &self.lines {
            match &l.kind {
                Kind::Section { name } => out.push(Section {
                    name: name.clone(),
                    entries: vec![],
                }),
                Kind::Entry { .. } => {
                    if let Some(e) = entry_of(l) {
                        out.last_mut().unwrap().entries.push(e);
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Entries of the first section named `section` (`""` = preamble).
    pub fn entries(&self, section: &str) -> Vec<Entry> {
        match self.body_range(section) {
            Some((s, e)) => self.lines[s..e].iter().filter_map(entry_of).collect(),
            None => vec![],
        }
    }

    /// Entries of every section called `section` (duplicate blocks), concatenated in order.
    pub fn entries_all_blocks(&self, section: &str) -> Vec<Entry> {
        self.body_ranges(section)
            .into_iter()
            .flat_map(|(s, e)| {
                self.lines[s..e]
                    .iter()
                    .filter_map(entry_of)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// All occurrences of `key` (case-insensitive) in the first `section`.
    pub fn get_all(&self, section: &str, key: &str) -> Vec<Entry> {
        self.entries(section)
            .into_iter()
            .filter(|e| e.key.eq_ignore_ascii_case(key))
            .collect()
    }

    /// Raw value (whole line after the separator) of the **last** occurrence, like the game.
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        self.get_all(section, key).pop().map(|e| e.value)
    }

    /// Blank-split tokens of the last occurrence.
    pub fn get_tokens(&self, section: &str, key: &str) -> Option<Vec<String>> {
        self.get_all(section, key).pop().map(|e| e.tokens)
    }

    fn key_line_idx(&self, section: &str, key: &str) -> Vec<usize> {
        match self.body_range(section) {
            None => vec![],
            Some((s, e)) => (s..e)
                .filter(|&i| {
                    entry_of(&self.lines[i]).is_some_and(|en| en.key.eq_ignore_ascii_case(key))
                })
                .collect(),
        }
    }

    fn rebuild_value(line: &mut Line, new_value: &str) {
        if let Kind::Entry {
            indent,
            key_len,
            sep_len,
            trail_len,
        } = line.kind
        {
            let chars: Vec<char> = line.text.chars().collect();
            let vs = indent + key_len + sep_len;
            let head: String = chars[..vs].iter().collect();
            let trail: String = chars[chars.len() - trail_len..].iter().collect();
            let mut sep_fix = String::new();
            // A key with no separator (`KEY` alone) needs one before a value.
            if sep_len == 0 && !new_value.is_empty() {
                sep_fix.push_str(" = ");
            }
            line.text = format!("{head}{sep_fix}{new_value}{trail}");
            let chars: Vec<char> = line.text.chars().collect();
            line.kind = classify(&chars);
        }
    }

    /// Replace the value of the last occurrence of `key` in `section` keeping indentation,
    /// key spelling and separator; otherwise append `KEY = value` to the section (creating the
    /// section at the end of the file when missing). Values must not contain line breaks.
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        debug_assert!(!value.contains(['\n', '\r']));
        if let Some(&i) = self.key_line_idx(section, key).last() {
            Self::rebuild_value(&mut self.lines[i], value);
        } else {
            self.add(section, key, value);
        }
    }

    /// Set from tokens (joined with single spaces).
    pub fn set_tokens<S: AsRef<str>>(&mut self, section: &str, key: &str, tokens: &[S]) {
        let v: Vec<&str> = tokens.iter().map(|t| t.as_ref()).collect();
        self.set(section, key, &v.join(" "));
    }

    /// Append `KEY = value` after the last non-blank line of the first matching section
    /// (before trailing blank lines). Creates the section if it does not exist.
    pub fn add(&mut self, section: &str, key: &str, value: &str) {
        let eol = self.dominant_eol();
        let text = if value.is_empty() {
            key.to_string()
        } else {
            format!("{key} = {value}")
        };
        if self.body_range(section).is_none() {
            self.add_section(section);
        }
        let (s, e) = self.body_range(section).unwrap();
        let mut at = e;
        while at > s && matches!(self.lines[at - 1].kind, Kind::Blank) {
            at -= 1;
        }
        // The line before the insertion point may be the file's final line without terminator.
        if at > 0 && self.lines[at - 1].eol.is_empty() {
            self.lines[at - 1].eol = eol.clone();
        }
        self.lines.insert(at, Line::classify(text, eol));
    }

    /// Append a new empty `[name]` section at the end of the file, separated by a blank line
    /// when the file is not empty. No-op for `""`.
    pub fn add_section(&mut self, name: &str) {
        if name.is_empty() {
            return;
        }
        let eol = self.dominant_eol();
        if let Some(last) = self.lines.last_mut() {
            if last.eol.is_empty() {
                last.eol = eol.clone();
            }
            if !matches!(last.kind, Kind::Blank) {
                self.lines.push(Line::classify(String::new(), eol.clone()));
            }
        }
        self.lines.push(Line::classify(format!("[{name}]"), eol));
    }

    /// Remove every occurrence of `key` in the first `section`; returns how many lines went.
    pub fn remove(&mut self, section: &str, key: &str) -> usize {
        let idx = self.key_line_idx(section, key);
        for &i in idx.iter().rev() {
            self.lines.remove(i);
        }
        idx.len()
    }

    /// Remove the first section called `name` including its header and body. Returns whether
    /// it existed.
    pub fn remove_section(&mut self, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        match self.body_range(name) {
            None => false,
            Some((s, e)) => {
                self.lines.drain(s - 1..e);
                true
            }
        }
    }
}
