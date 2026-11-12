//! Bytes-preserving Latin-1 text buffer.
//!
//! Scripts are not always UTF-8, so the editor holds each byte as the `char` with the same
//! code point (U+0000..=U+00FF). A character index of the buffer is therefore also the byte
//! offset in the (LF-normalised) file, which keeps every span of `kfx_script` valid as is.
use egui::text::CharIndex;
use egui::TextBuffer;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub fn decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Encode the buffer back. Characters above U+00FF cannot occur (the buffer rejects them);
/// they would become `?`.
pub fn encode(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
        .collect()
}

/// Line ending convention of a loaded file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    Lf,
    /// Every line ends with CR LF: the editor buffer holds LF only and CRs are restored on save.
    CrLf,
    /// Anything else (lone CRs, mixed endings): the buffer keeps the raw CR characters.
    Raw,
}

pub fn detect_eol(bytes: &[u8]) -> Eol {
    let mut cr = 0usize;
    let mut crlf = 0usize;
    let mut lf = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\r' => {
                cr += 1;
                if bytes.get(i + 1) == Some(&b'\n') {
                    crlf += 1;
                }
            }
            b'\n' => lf += 1,
            _ => {}
        }
    }
    if cr == 0 {
        Eol::Lf
    } else if cr == crlf && crlf == lf {
        Eol::CrLf
    } else {
        Eol::Raw
    }
}

/// Convert file bytes to the editor buffer text and back.
pub fn to_buffer_text(bytes: &[u8], eol: Eol) -> String {
    match eol {
        Eol::CrLf => bytes
            .iter()
            .filter(|&&b| b != b'\r')
            .map(|&b| b as char)
            .collect(),
        _ => decode(bytes),
    }
}

pub fn from_buffer_text(text: &str, eol: Eol) -> Vec<u8> {
    let mut out = encode(text);
    if eol == Eol::CrLf {
        let mut v = Vec::with_capacity(out.len() + out.len() / 30);
        for b in out.drain(..) {
            if b == b'\n' {
                v.push(b'\r');
            }
            v.push(b);
        }
        out = v;
    }
    out
}

pub fn byte_of_char(s: &str, ci: usize) -> usize {
    if s.is_ascii() {
        return ci.min(s.len());
    }
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

fn sanitize(text: &str, strip_cr: bool) -> String {
    text.chars()
        .filter(|&c| !(strip_cr && c == '\r'))
        .map(|c| if (c as u32) < 256 { c } else { '?' })
        .collect()
}

/// The `String` an egui `TextEdit` edits. Counts every change in a shared generation counter.
pub struct LatinBuffer {
    text: String,
    generation: Arc<AtomicU64>,
    strip_cr: bool,
    read_only: bool,
}

impl LatinBuffer {
    pub fn new(generation: Arc<AtomicU64>) -> LatinBuffer {
        LatinBuffer {
            text: String::new(),
            generation,
            strip_cr: true,
            read_only: false,
        }
    }
    pub fn set_read_only(&mut self, v: bool) {
        self.read_only = v;
    }
    pub fn read_only(&self) -> bool {
        self.read_only
    }
    pub fn set_strip_cr(&mut self, v: bool) {
        self.strip_cr = v;
    }
    fn bump(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
    pub fn set(&mut self, text: String) {
        self.text = text;
        self.bump();
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn char_len(&self) -> usize {
        if self.text.is_ascii() {
            self.text.len()
        } else {
            self.text.chars().count()
        }
    }
    /// Replace chars `[start, end)` by `with` (Latin-1 sanitised).
    pub fn replace(&mut self, start: usize, end: usize, with: &str) {
        let with = sanitize(with, self.strip_cr);
        let bs = byte_of_char(&self.text, start);
        let be = byte_of_char(&self.text, end.max(start));
        self.text.replace_range(bs..be, &with);
        self.bump();
    }
}

impl TextBuffer for LatinBuffer {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<LatinBuffer>()
    }
    fn is_mutable(&self) -> bool {
        !self.read_only
    }
    fn as_str(&self) -> &str {
        &self.text
    }
    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        let clean = sanitize(text, self.strip_cr);
        if clean.is_empty() {
            return 0;
        }
        let b = byte_of_char(&self.text, char_index.0);
        self.text.insert_str(b, &clean);
        self.bump();
        clean.chars().count()
    }
    fn delete_char_range(&mut self, r: Range<CharIndex>) {
        if r.end.0 <= r.start.0 {
            return;
        }
        let bs = byte_of_char(&self.text, r.start.0);
        let be = byte_of_char(&self.text, r.end.0);
        self.text.replace_range(bs..be, "");
        self.bump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_bytes() {
        let all: Vec<u8> = (0..=255u8).collect();
        assert_eq!(encode(&decode(&all)), all);
    }

    #[test]
    fn eol_detection() {
        assert_eq!(detect_eol(b"a\nb\n"), Eol::Lf);
        assert_eq!(detect_eol(b"a\r\nb\r\n"), Eol::CrLf);
        assert_eq!(detect_eol(b"a\r\nb\n"), Eol::Raw);
        assert_eq!(detect_eol(b"a\rb"), Eol::Raw);
        assert_eq!(detect_eol(b""), Eol::Lf);
    }

    #[test]
    fn eol_roundtrip() {
        for src in [
            &b"a\r\nb\r\nc"[..],
            b"a\nb",
            b"a\r\nb\n",
            b"\xe9\xff\r\n\x80",
            b"x\r",
        ] {
            let e = detect_eol(src);
            assert_eq!(from_buffer_text(&to_buffer_text(src, e), e), src);
        }
    }

    #[test]
    fn buffer_rejects_wide_chars_and_counts_generations() {
        let g = Arc::new(AtomicU64::new(0));
        let mut b = LatinBuffer::new(g.clone());
        b.insert_text("a\u{20ac}\u{e9}", CharIndex(0));
        assert_eq!(b.text(), "a?\u{e9}");
        assert!(g.load(Ordering::Relaxed) > 0);
        b.replace(1, 2, "\r\n");
        assert_eq!(b.text(), "a\n\u{e9}");
    }
}
