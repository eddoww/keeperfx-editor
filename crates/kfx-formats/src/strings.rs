//! The game's string tables: `fxdata/gtext_<lang>.dat` (GUI strings), campaign `[strings]` files
//! and per-map `mapNNNNN.<lang>.dat`.
//!
//! # File format
//! A file is a sequence of NUL-terminated strings; string `i` is the `i`-th chunk
//! (`fill_strings_list`, `src/config_strings.c:56-84`). The file is read through RNC
//! (`LbFileLengthRnc` / `LbFileLoadAt`), files shorter than 16 bytes are rejected
//! (src/config_strings.c:111, 303; src/lvl_filesdk1.c:1383), then converted to UTF-8 with
//! `convert_codepage_to_utf8_buffer(.., lang_id)` (`src/bflib_text.c:119`):
//! * every language except Japanese, Chinese (simplified / traditional) and Korean uses the
//!   game's own 256-entry byte table `internal_codepage_map` (src/bflib_text.c:36-67, the original
//!   DK font layout: Latin accents, Polish, Cyrillic ...). A byte that has no entry (0xB8-0xC1,
//!   0xFE, 0xFF) decodes to `'?'`; byte 0 stays NUL.
//! * Japanese is CP932 (Shift-JIS), Chinese (simplified and traditional alike) GBK (CP936),
//!   Korean EUC-KR (CP949 on Windows) (src/bflib_text.c:177-241: `iconv` on Linux,
//!   `MultiByteToWideChar` on Windows). They are implemented with the `encoding_rs` crate
//!   (`SHIFT_JIS` is Windows-31J, `GBK`, `EUC_KR` is the CP949 superset). Bytes that are not a
//!   valid sequence decode to U+FFFD (the game's `iconv` would drop the whole file; the editor must
//!   still show it); characters the code page lacks cannot be encoded.
//!
//! The raw bytes of every string are kept anyway: a string that is not edited is never
//! re-encoded, so malformed or ambiguous byte sequences (CP932 has duplicate code points) and
//! the bytes the internal table has no entry for survive any edit of the other strings. Setting
//! a string to the text it already decodes to keeps its raw bytes ([`StringTable::set`]).
//!
//! [`StringTable`] keeps the raw bytes of every string plus whether the last string was
//! NUL-terminated, so `parse` + `to_bytes` reproduces any file exactly (also the bytes the table
//! cannot represent). Text is decoded on demand and edits re-encode only the edited string.
//!
//! # Which table answers a string id (`get_string`, src/config_strings.c:420)
//! * `0..2000` ([`STRINGS_MAX`]): the level's `mapNNNNN.<lang>.dat` string when non-empty, else
//!   the campaign string (`cmpgn_string`, :396: the campaign string when non-empty, else the GUI
//!   string of the same index).
//! * `2000..4000` ([`TRANSLATION_STRINGS_START`]..[`GUI_STRINGS_START`]): `translation.toml`
//!   aliases ([`crate::translation`]).
//! * `4000..`: the GUI string `id - 4000` (`gtext_<lang>.dat`).
//! Tables stack: a later file only replaces the slots where it has a non-empty string ("Compatible
//! with mods. Do not replace empty string", src/config_strings.c:67). GUI: base file, then the mods'
//! `fxdata/gtext_<lang>.dat` (english when the mod has no file for the language;
//! src/config_strings.c:183-205). Campaign: the `[strings]` file for the language, else the English
//! entry, then the mods the same way. Map: `mapNNNNN.<lang>.dat` in the levels folder, else the
//! campaign's default language (the first `[strings]` entry), else none (src/lvl_filesdk1.c:1345).
use crate::{rnc, Error, Result};
use std::path::{Path, PathBuf};

/// `STRINGS_MAX` (src/config_strings.h:29): campaign / level strings hold ids 0..=2000.
pub const STRINGS_MAX: usize = 2000;
/// `MAX_TOML_TRANSLATION_ENTRIES` (src/config_strings.h:30).
pub const MAX_TOML_TRANSLATION_ENTRIES: usize = 2000;
/// `TRANSLATION_STRINGS_START` (src/config_strings.h:32).
pub const TRANSLATION_STRINGS_START: usize = STRINGS_MAX;
/// `GUI_STRINGS_START` (src/config_strings.h:33).
pub const GUI_STRINGS_START: usize = STRINGS_MAX + MAX_TOML_TRANSLATION_ENTRIES;
/// Files shorter than this are not loaded (src/config_strings.c:111).
pub const MIN_STRINGS_FILE_SIZE: usize = 16;

/// `enum TbLanguage` (src/config_keeperfx.h:62) with the codes of `lang_type` (src/config_keeperfx.c:82).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lang {
    English = 1,
    French,
    German,
    Italian,
    Spanish,
    Swedish,
    Polish,
    Dutch,
    Hungarian,
    Korean,
    Danish,
    Norwegian,
    Czech,
    Arabic,
    Russian,
    Japanese,
    ChineseInt,
    ChineseTra,
    Portuguese,
    Hindi,
    Bengali,
    Javanese,
    Latin,
    Ukrainian,
}

impl Lang {
    pub const ALL: [Lang; 24] = [
        Lang::English,
        Lang::French,
        Lang::German,
        Lang::Italian,
        Lang::Spanish,
        Lang::Swedish,
        Lang::Polish,
        Lang::Dutch,
        Lang::Hungarian,
        Lang::Korean,
        Lang::Danish,
        Lang::Norwegian,
        Lang::Czech,
        Lang::Arabic,
        Lang::Russian,
        Lang::Japanese,
        Lang::ChineseInt,
        Lang::ChineseTra,
        Lang::Portuguese,
        Lang::Hindi,
        Lang::Bengali,
        Lang::Javanese,
        Lang::Latin,
        Lang::Ukrainian,
    ];
    /// The upper-case three letter code (`ENG`, `CHT`, ...).
    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "ENG",
            Lang::French => "FRE",
            Lang::German => "GER",
            Lang::Italian => "ITA",
            Lang::Spanish => "SPA",
            Lang::Swedish => "SWE",
            Lang::Polish => "POL",
            Lang::Dutch => "DUT",
            Lang::Hungarian => "HUN",
            Lang::Korean => "KOR",
            Lang::Danish => "DAN",
            Lang::Norwegian => "NOR",
            Lang::Czech => "CZE",
            Lang::Arabic => "ARA",
            Lang::Russian => "RUS",
            Lang::Japanese => "JPN",
            Lang::ChineseInt => "CHI",
            Lang::ChineseTra => "CHT",
            Lang::Portuguese => "POR",
            Lang::Hindi => "HIN",
            Lang::Bengali => "BEN",
            Lang::Javanese => "JAV",
            Lang::Latin => "LAT",
            Lang::Ukrainian => "UKR",
        }
    }
    /// The lower-case code used in file names (`get_language_lwrstr`, src/config_keeperfx.c:364).
    pub fn lower(self) -> String {
        self.code().to_ascii_lowercase()
    }
    pub fn from_code(s: &str) -> Option<Lang> {
        Lang::ALL
            .into_iter()
            .find(|l| l.code().eq_ignore_ascii_case(s.trim()))
    }
    /// The numeric id (`install_info.lang_id`).
    pub fn id(self) -> u8 {
        self as u8
    }
    /// Whether the game converts this language with a double-byte code page instead of its own
    /// byte table (src/bflib_text.c:136).
    pub fn is_double_byte(self) -> bool {
        matches!(
            self,
            Lang::Japanese | Lang::ChineseInt | Lang::ChineseTra | Lang::Korean
        )
    }
    /// The code page name the game uses for the double-byte languages.
    pub fn codepage_name(self) -> &'static str {
        match self {
            Lang::Japanese => "CP932",
            Lang::ChineseInt | Lang::ChineseTra => "CP936",
            Lang::Korean => "EUC-KR (CP949)",
            _ => "internal",
        }
    }
    /// `gtext_<lang>.dat`.
    pub fn gtext_file(self) -> String {
        format!("gtext_{}.dat", self.lower())
    }
    /// `mapNNNNN.<lang>.dat`.
    pub fn map_strings_file(self, level: u32) -> String {
        format!("map{level:05}.{}.dat", self.lower())
    }
}

/// `internal_codepage_map` (src/bflib_text.c:36-67): byte -> Unicode code point, 0 = no entry.
const INTERNAL: [u32; 256] = [
    0, 0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007, 0x0008, 0x0009, 0x000A, 0x000B,
    0x000C, 0x000D, 0x000E, 0x0404, 0x0454, 0x0490, 0x0491, 0x0013, 0x0014, 0x0015, 0x0016, 0x0017,
    0x0018, 0x0019, 0x001A, 0x001B, 0x001C, 0x001D, 0x001E, 0x001F, 0x20, 0x21, 0x22, 0x23, 0x24,
    0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34,
    0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44,
    0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x52, 0x53, 0x54,
    0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x5D, 0x5E, 0x5F, 0x60, 0x61, 0x62, 0x63, 0x64,
    0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x70, 0x71, 0x72, 0x73, 0x74,
    0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x7B, 0x7C, 0x7D, 0x7E, 0x041B, 0x00C7, 0x00FC, 0x00E9,
    0x00E2, 0x00E4, 0x00E0, 0x00E5, 0x00E7, 0x00EA, 0x00EB, 0x00E8, 0x00EF, 0x00EE, 0x00EC, 0x00C4,
    0x00C5, 0x00C9, 0x00E6, 0x041F, 0x00F4, 0x00F6, 0x00F2, 0x00FB, 0x00F9, 0x00FF, 0x00D6, 0x00DC,
    0x042D, 0x042E, 0x042F, 0x043A, 0x043B, 0x00E1, 0x00ED, 0x00F3, 0x00FA, 0x00F1, 0x00D1, 0x043C,
    0x043D, 0x00BF, 0x043F, 0x044A, 0x044B, 0x044C, 0x00A1, 0x044D, 0x044E, 0x044F, 0x0411, 0x0413,
    0x0414, 0x0416, 0x00C1, 0x00C2, 0x00C0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0417, 0x0418, 0x0419,
    0x0423, 0x00E3, 0x00C3, 0x0424, 0x0426, 0x0427, 0x0428, 0x0429, 0x0431, 0x0432, 0x0433, 0x0434,
    0x0436, 0x00CA, 0x00CB, 0x00C8, 0x0437, 0x00CD, 0x00CE, 0x00CF, 0x0438, 0x0439, 0x0442, 0x0444,
    0x0446, 0x00CC, 0x0447, 0x00D3, 0x00DF, 0x00D4, 0x00D2, 0x00F5, 0x00D5, 0x0448, 0x0449, 0x042B,
    0x00DA, 0x00DB, 0x00D9, 0x00FD, 0x00DD, 0x0105, 0x0104, 0x0107, 0x0106, 0x0119, 0x0118, 0x0142,
    0x0141, 0x0144, 0x0143, 0x015B, 0x015A, 0x017A, 0x0179, 0x017C, 0x017B, 0, 0,
];

/// A character that has no byte in the game's code page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unencodable {
    pub ch: char,
    /// Character index in the text.
    pub at: usize,
}

impl std::fmt::Display for Unencodable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} (U+{:04X}) at character {} has no byte in this language's code page",
            self.ch, self.ch as u32, self.at
        )
    }
}

/// Decode with the game's byte table: unknown bytes become `?` like the game.
pub fn decode_internal(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match INTERNAL[b as usize] {
            0 if b == 0 => '\0',
            0 => '?',
            c => char::from_u32(c).unwrap_or('?'),
        })
        .collect()
}

/// The byte of a character in the game's table (first entry wins; the table has no duplicates).
pub fn encode_internal_char(c: char) -> Option<u8> {
    let cp = c as u32;
    (1..256)
        .find(|&b| INTERNAL[b] == cp)
        .map(|b| b as u8)
        .or(if cp == 0 { Some(0) } else { None })
}

/// Encode text to the game's byte table.
pub fn encode_internal(text: &str) -> std::result::Result<Vec<u8>, Unencodable> {
    text.chars()
        .enumerate()
        .map(|(at, ch)| encode_internal_char(ch).ok_or(Unencodable { ch, at }))
        .collect()
}

/// The `encoding_rs` codec of a double-byte language (see the module doc).
fn codec(lang: Lang) -> Option<&'static encoding_rs::Encoding> {
    match lang {
        Lang::Japanese => Some(encoding_rs::SHIFT_JIS),
        Lang::ChineseInt | Lang::ChineseTra => Some(encoding_rs::GBK),
        Lang::Korean => Some(encoding_rs::EUC_KR),
        _ => None,
    }
}

/// Lossless stand-in for a byte the code page cannot decode: `U+F700 + byte` (private use area).
/// [`decode`] produces it and [`encode`] turns it back into the byte, so every byte sequence
/// survives a text round trip. The game itself shows `?` (internal table) or whatever its converter
/// yields for such bytes, see [`decode_game`].
pub const ESCAPE_BASE: u32 = 0xF700;

fn escape(b: u8) -> char {
    char::from_u32(ESCAPE_BASE + b as u32).unwrap_or('?')
}

fn unescape(c: char) -> Option<u8> {
    let cp = c as u32;
    (ESCAPE_BASE..ESCAPE_BASE + 256)
        .contains(&cp)
        .then(|| (cp - ESCAPE_BASE) as u8)
}

/// Characters a code page decodes but the encoder has no mapping for (CP932 and GBK user-defined
/// areas decode to private-use characters; the WHATWG encoders cannot produce them): char -> its
/// two-byte sequence. Built once per codec by decoding every lead/trail pair.
fn decode_only_chars(
    enc: &'static encoding_rs::Encoding,
) -> &'static std::collections::HashMap<char, [u8; 2]> {
    use std::sync::OnceLock;
    static MAPS: OnceLock<Vec<(&'static str, std::collections::HashMap<char, [u8; 2]>)>> =
        OnceLock::new();
    let maps = MAPS.get_or_init(|| {
        [
            encoding_rs::SHIFT_JIS,
            encoding_rs::GBK,
            encoding_rs::EUC_KR,
        ]
        .into_iter()
        .map(|e| {
            let mut m = std::collections::HashMap::new();
            for a in 0x81u8..=0xFE {
                for b in 0x40u8..=0xFE {
                    let pair = [a, b];
                    let (t, bad) = e.decode_without_bom_handling(&pair);
                    let mut it = t.chars();
                    if let (false, Some(c), None) = (bad, it.next(), it.next()) {
                        let mut buf = [0u8; 4];
                        if e.encode(c.encode_utf8(&mut buf)).2 {
                            m.entry(c).or_insert([a, b]);
                        }
                    }
                }
            }
            (e.name(), m)
        })
        .collect()
    });
    &maps
        .iter()
        .find(|(n, _)| *n == enc.name())
        .expect("codec table")
        .1
}

/// Decode bytes of `lang` without losing anything: the internal table, or for the double-byte
/// languages their code page; bytes without a character become [`ESCAPE_BASE`] escapes.
pub fn decode(lang: Lang, bytes: &[u8]) -> String {
    match codec(lang) {
        Some(enc) => decode_dbcs(enc, bytes),
        None => bytes
            .iter()
            .map(|&b| match INTERNAL[b as usize] {
                0 if b == 0 => '\0',
                0 => escape(b),
                c => char::from_u32(c).unwrap_or_else(|| escape(b)),
            })
            .collect(),
    }
}

fn decode_dbcs(enc: &'static encoding_rs::Encoding, bytes: &[u8]) -> String {
    let mut dec = enc.new_decoder_without_bom_handling();
    let mut out = String::with_capacity(bytes.len() + 8);
    let mut pos = 0;
    loop {
        let (res, read) = dec.decode_to_string_without_replacement(&bytes[pos..], &mut out, true);
        pos += read;
        match res {
            encoding_rs::DecoderResult::InputEmpty => break,
            encoding_rs::DecoderResult::OutputFull => out.reserve(16 + bytes.len() - pos),
            encoding_rs::DecoderResult::Malformed(bad, extra) => {
                // the `extra` bytes after the bad ones are decoded again by the decoder itself
                let end = pos.saturating_sub(extra as usize);
                let start = end.saturating_sub(bad as usize);
                for &b in &bytes[start..end] {
                    out.push(escape(b));
                }
            }
        }
    }
    out
}

/// What the game shows for `bytes`: unmapped bytes of the internal table are `?`
/// (`internal_byte_to_unicode`, src/bflib_text.c:92), invalid double-byte sequences U+FFFD (the Windows
/// converter; `iconv` would drop the whole file).
pub fn decode_game(lang: Lang, bytes: &[u8]) -> String {
    match codec(lang) {
        Some(enc) => enc.decode_without_bom_handling(bytes).0.into_owned(),
        None => decode_internal(bytes),
    }
}

/// Whether `bytes` decode without any invalid sequence in `lang` (always true for the internal
/// table, whose unmapped bytes decode to `?` like the game).
pub fn decodes_cleanly(lang: Lang, bytes: &[u8]) -> bool {
    match codec(lang) {
        Some(enc) => !enc.decode_without_bom_handling(bytes).1,
        None => true,
    }
}

/// Encode text for `lang`; fails on the first character the code page has no byte sequence for.
/// [`ESCAPE_BASE`] escapes become their byte.
pub fn encode(lang: Lang, text: &str) -> std::result::Result<Vec<u8>, Unencodable> {
    let Some(enc) = codec(lang) else {
        return text
            .chars()
            .enumerate()
            .map(|(at, ch)| {
                encode_internal_char(ch)
                    .or_else(|| unescape(ch).filter(|&b| INTERNAL[b as usize] == 0))
                    .ok_or(Unencodable { ch, at })
            })
            .collect();
    };
    let mut out = Vec::with_capacity(text.len());
    let mut run = String::new();
    let mut run_start = 0usize;
    let flush = |run: &mut String,
                 run_start: usize,
                 out: &mut Vec<u8>|
     -> std::result::Result<(), Unencodable> {
        if run.is_empty() {
            return Ok(());
        }
        let (bytes, _, unmappable) = enc.encode(run);
        if unmappable {
            // locate the first character without a mapping
            let mut buf = [0u8; 4];
            for (i, ch) in run.chars().enumerate() {
                if enc.encode(ch.encode_utf8(&mut buf)).2 {
                    return Err(Unencodable {
                        ch,
                        at: run_start + i,
                    });
                }
            }
            return Err(Unencodable {
                ch: '\u{FFFD}',
                at: run_start,
            });
        }
        out.extend_from_slice(&bytes);
        run.clear();
        Ok(())
    };
    let only = decode_only_chars(enc);
    for (at, ch) in text.chars().enumerate() {
        match unescape(ch) {
            Some(b) => {
                flush(&mut run, run_start, &mut out)?;
                out.push(b);
                run_start = at + 1;
            }
            None if only.contains_key(&ch) => {
                flush(&mut run, run_start, &mut out)?;
                out.extend_from_slice(&only[&ch]);
                run_start = at + 1;
            }
            None => {
                if run.is_empty() {
                    run_start = at;
                }
                run.push(ch);
            }
        }
    }
    flush(&mut run, run_start, &mut out)?;
    Ok(out)
}

/// One strings file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringTable {
    pub lang: Lang,
    raw: Vec<Vec<u8>>,
    /// The last string is followed by a NUL byte.
    terminated: bool,
}

impl StringTable {
    pub fn new(lang: Lang) -> StringTable {
        StringTable {
            lang,
            raw: vec![],
            terminated: true,
        }
    }

    /// Split file content into strings. Every byte sequence is accepted.
    pub fn parse(data: &[u8], lang: Lang) -> StringTable {
        if data.is_empty() {
            return StringTable {
                lang,
                raw: vec![],
                terminated: false,
            };
        }
        let mut raw: Vec<Vec<u8>> = data.split(|&b| b == 0).map(|c| c.to_vec()).collect();
        let terminated = data.last() == Some(&0);
        if terminated {
            raw.pop();
        }
        StringTable {
            lang,
            raw,
            terminated,
        }
    }

    /// Load through RNC. Files shorter than 16 bytes are an error (the game refuses them).
    pub fn load(path: &Path, lang: Lang) -> Result<StringTable> {
        let b = rnc::load_file(path)?;
        if b.len() < MIN_STRINGS_FILE_SIZE {
            return Err(Error::format(
                "strings",
                format!(
                    "{}: {} bytes, the game needs at least {MIN_STRINGS_FILE_SIZE}",
                    path.display(),
                    b.len()
                ),
            ));
        }
        Ok(StringTable::parse(&b, lang))
    }

    /// The exact file content ([`StringTable::parse`] of it is `self`).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.raw.iter().map(|s| s.len() + 1).sum());
        for (i, s) in self.raw.iter().enumerate() {
            out.extend_from_slice(s);
            if i + 1 < self.raw.len() || self.terminated {
                out.push(0);
            }
        }
        out
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::atomic::atomic_write(path, &self.to_bytes()).map_err(|e| Error::io(path, e))
    }

    pub fn len(&self) -> usize {
        self.raw.len()
    }
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }
    /// The raw bytes of string `i`.
    pub fn raw(&self, i: usize) -> Option<&[u8]> {
        self.raw.get(i).map(Vec::as_slice)
    }
    /// String `i` as the game shows it (decoded), `None` past the end.
    pub fn get(&self, i: usize) -> Option<String> {
        self.raw.get(i).map(|b| decode(self.lang, b))
    }
    /// Every string decoded.
    pub fn strings(&self) -> Vec<String> {
        self.raw.iter().map(|b| decode(self.lang, b)).collect()
    }

    /// Replace string `i` (the table grows with empty strings when `i` is past the end). The
    /// text must be encodable and contain no NUL.
    pub fn set(&mut self, i: usize, text: &str) -> Result<()> {
        if text.contains('\0') {
            return Err(Error::format("strings", "a string cannot contain NUL"));
        }
        // an unchanged text keeps its exact bytes (ambiguous or malformed sequences survive)
        if self.get(i).as_deref() == Some(text) {
            return Ok(());
        }
        let bytes = encode(self.lang, text).map_err(|e| Error::format("strings", e.to_string()))?;
        self.set_raw(i, bytes);
        Ok(())
    }

    /// Replace string `i` with raw bytes (NUL bytes cut the string).
    pub fn set_raw(&mut self, i: usize, mut bytes: Vec<u8>) {
        if let Some(p) = bytes.iter().position(|&b| b == 0) {
            bytes.truncate(p);
        }
        if i >= self.raw.len() {
            // padding strings are empty; the last string is terminated from now on
            self.raw.resize(i + 1, vec![]);
            self.terminated = true;
        }
        self.raw[i] = bytes;
    }

    /// Append a string, returning its id.
    pub fn add(&mut self, text: &str) -> Result<usize> {
        let i = self.raw.len();
        self.set(i, text)?;
        Ok(i)
    }

    /// The first id whose string is empty (a free slot), or the end of the table.
    pub fn first_free(&self) -> usize {
        self.raw
            .iter()
            .position(|s| s.is_empty())
            .unwrap_or(self.raw.len())
    }
}

/// The slots of one game string array (`gui_strings`, `campaign.strings`, `level_strings`),
/// filled the way the game fills them: later tables replace only non-empty strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slots {
    pub slots: Vec<String>,
}

impl Slots {
    /// `size` empty slots (`reset_strings`, src/config_strings.c:42; the game resets `max + 1`).
    pub fn new(size: usize) -> Slots {
        Slots {
            slots: vec![String::new(); size],
        }
    }
    /// `fill_strings_list` (src/config_strings.c:56) with the table of the file.
    pub fn apply(&mut self, t: &StringTable) {
        for i in 0..t.len().min(self.slots.len()) {
            if t.raw[i].first().is_some() {
                self.slots[i] = decode_game(t.lang, &t.raw[i]);
            }
        }
    }
    pub fn get(&self, i: usize) -> &str {
        self.slots.get(i).map_or("", String::as_str)
    }
}

/// `parse_campaign_strings_blocks` (src/config_campaigns.c:786): which file of a campaign's
/// `[strings]` block serves `lang`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CampaignStringsFiles {
    /// `strings_fname`: the entry of `lang`, else the first entry.
    pub fname: Option<String>,
    /// Language the file is decoded with (`strings_lang`).
    pub fname_lang: Option<Lang>,
    /// `strings_fname_eng`: the English entry, else the first entry.
    pub fname_eng: Option<String>,
    /// `default_language`: the language of the first entry (used for map strings).
    pub default_language: Option<Lang>,
}

/// Pick the campaign strings files for `lang` from the `(code, file)` entries of `[strings]`
/// (`kfx_formats::campaign::Campaign::strings`).
pub fn campaign_strings_files(entries: &[(String, String)], lang: Lang) -> CampaignStringsFiles {
    let mut out = CampaignStringsFiles::default();
    let mut n = 0;
    for (code, file) in entries {
        let l = Lang::from_code(code);
        if n == 0 {
            out.default_language = Some(l.unwrap_or(Lang::English));
        }
        let Some(l) = l else { continue };
        if l == Lang::English || l == lang || n == 0 {
            let file = file.trim();
            if l == Lang::English || n == 0 {
                if !file.is_empty() {
                    out.fname_eng = Some(file.to_string());
                }
            }
            if l == lang || n == 0 {
                if !file.is_empty() {
                    out.fname = Some(file.to_string());
                    out.fname_lang = Some(l);
                    n += 1;
                }
            }
        }
    }
    out
}

fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    crate::testutil::resolve_ci(dir, &name.replace('\\', "/")).filter(|p| p.is_file())
}

/// Files that make up the strings of one level, resolved with the game's fallback rules.
#[derive(Clone, Debug, Default)]
pub struct StringSources {
    pub lang: Option<Lang>,
    /// `gtext_<lang>.dat` of the base then of each mod (each with its decoding language).
    pub gui: Vec<(PathBuf, Lang)>,
    /// Campaign strings: base file then the mods' files.
    pub campaign: Vec<(PathBuf, Lang)>,
    /// `mapNNNNN.<lang>.dat` (or of the campaign's default language).
    pub level: Option<(PathBuf, Lang)>,
    /// `translation.toml` files in layer order.
    pub translation: Vec<PathBuf>,
}

fn usable(p: &Path) -> bool {
    rnc::load_file(p)
        .map(|b| b.len() >= MIN_STRINGS_FILE_SIZE)
        .unwrap_or(false)
}

impl StringSources {
    /// Resolve the files for `lang`:
    /// * `install`: the install folder (`fxdata/`, campaign paths are relative to it);
    /// * `mod_dirs`: the mods in load order (each a folder under `mods/`);
    /// * `campaign_strings`: the `[strings]` entries of the campaign cfg;
    /// * `levels_dir` + `level`: where `mapNNNNN.<lang>.dat` lives.
    pub fn resolve(
        install: &Path,
        mod_dirs: &[PathBuf],
        campaign_strings: &[(String, String)],
        levels_dir: Option<&Path>,
        level: Option<u32>,
        lang: Lang,
    ) -> Result<StringSources> {
        let mut s = StringSources {
            lang: Some(lang),
            ..Default::default()
        };
        // GUI: the base file must exist (setup_gui_strings_data returns false otherwise)
        let fx = crate::testutil::resolve_ci(install, "fxdata")
            .unwrap_or_else(|| install.join("fxdata"));
        let base = find_ci(&fx, &lang.gtext_file()).filter(|p| usable(p));
        let Some(base) = base else {
            return Err(Error::format(
                "strings",
                format!(
                    "{}: GUI strings file for {} missing",
                    fx.display(),
                    lang.code()
                ),
            ));
        };
        s.gui.push((base, lang));
        for m in mod_dirs {
            let mfx = m.join("fxdata");
            if let Some(p) = find_ci(&mfx, &lang.gtext_file()).filter(|p| usable(p)) {
                s.gui.push((p, lang));
            } else if lang != Lang::English {
                if let Some(p) = find_ci(&mfx, &Lang::English.gtext_file()).filter(|p| usable(p)) {
                    s.gui.push((p, Lang::English));
                }
            }
        }
        // campaign strings
        let cf = campaign_strings_files(campaign_strings, lang);
        let mut campaign_files = vec![];
        if let (Some(f), Some(fl)) = (&cf.fname, cf.fname_lang) {
            if let Some(p) = find_ci(install, f).filter(|p| usable(p)) {
                campaign_files.push((p, fl));
            } else if let Some(e) = cf
                .fname_eng
                .as_ref()
                .filter(|e| Some(*e) != cf.fname.as_ref())
            {
                if let Some(p) = find_ci(install, e).filter(|p| usable(p)) {
                    campaign_files.push((p, Lang::English));
                }
            }
        }
        if !campaign_files.is_empty() {
            for m in mod_dirs {
                let try_one =
                    |f: &str, l: Lang| find_ci(m, f).filter(|p| usable(p)).map(|p| (p, l));
                let got = cf
                    .fname
                    .as_deref()
                    .and_then(|f| try_one(f, lang))
                    .or_else(|| {
                        cf.fname_eng
                            .as_deref()
                            .filter(|e| Some(*e) != cf.fname.as_deref())
                            .and_then(|e| try_one(e, Lang::English))
                    });
                campaign_files.extend(got);
            }
        }
        s.campaign = campaign_files;
        // level strings
        if let (Some(dir), Some(n)) = (levels_dir, level) {
            let first = lang.map_strings_file(n);
            let mut pick = find_ci(dir, &first).map(|p| (p, lang));
            if pick.is_none() {
                if let Some(def) = cf.default_language {
                    let second = def.map_strings_file(n);
                    if !second.eq_ignore_ascii_case(&first) {
                        pick = find_ci(dir, &second).map(|p| (p, def));
                    }
                }
            }
            s.level = pick.filter(|(p, _)| usable(p));
        }
        Ok(s)
    }
}

/// All string tables of a level, answering ids like the game's `get_string`.
#[derive(Clone, Debug)]
pub struct StringResolver {
    pub lang: Lang,
    pub gui: Slots,
    pub campaign: Slots,
    pub level: Slots,
    /// The game's translation table (index = id - 2000), see [`crate::translation`].
    pub translation: crate::translation::TranslationTable,
}

impl StringResolver {
    /// Load every table named by `sources` (a missing / unreadable file is skipped like the game
    /// does for optional ones; the GUI base file is required by [`StringSources::resolve`]).
    pub fn load(sources: &StringSources) -> Result<StringResolver> {
        let lang = sources.lang.unwrap_or(Lang::English);
        // the GUI array is as long as its biggest file; the game sizes it by an enum
        let mut gui = Slots::new(0);
        for (p, l) in &sources.gui {
            let t = StringTable::load(p, *l)?;
            if gui.slots.len() < t.len() {
                gui.slots.resize(t.len(), String::new());
            }
            gui.apply(&t);
        }
        let mut campaign = Slots::new(STRINGS_MAX + 1);
        for (p, l) in &sources.campaign {
            if let Ok(t) = StringTable::load(p, *l) {
                campaign.apply(&t);
            }
        }
        let mut level = Slots::new(STRINGS_MAX + 1);
        if let Some((p, l)) = &sources.level {
            if let Ok(t) = StringTable::load(p, *l) {
                level.apply(&t);
            }
        }
        let translation =
            crate::translation::TranslationTable::load(&sources.translation, lang.code())?;
        Ok(StringResolver {
            lang,
            gui,
            campaign,
            level,
            translation,
        })
    }

    fn gui_string(&self, index: usize) -> String {
        match self.gui.slots.get(index) {
            Some(s) => s.clone(),
            None => format!("untranslated <{index}>"),
        }
    }

    fn campaign_string(&self, index: usize) -> String {
        if index >= STRINGS_MAX {
            return self.gui_string(index - STRINGS_MAX);
        }
        if !self.campaign.get(index).is_empty() {
            return self.campaign.get(index).to_string();
        }
        self.gui_string(index)
    }

    /// `get_string` (src/config_strings.c:420) for a string id of a script / cfg.
    pub fn get(&self, id: i64) -> String {
        if id < 0 {
            return "invalid string id".into();
        }
        let id = id as usize;
        if id < TRANSLATION_STRINGS_START {
            let l = self.level.get(id);
            if !l.is_empty() {
                return l.to_string();
            }
            return self.campaign_string(id);
        }
        if id < GUI_STRINGS_START {
            return self.translation.get(id).to_string();
        }
        self.gui_string(id - GUI_STRINGS_START)
    }

    /// Which table answers `id`.
    pub fn source_of(&self, id: i64) -> StringSource {
        if id < 0 {
            return StringSource::Invalid;
        }
        let id = id as usize;
        if id < TRANSLATION_STRINGS_START {
            if !self.level.get(id).is_empty() {
                StringSource::Level
            } else if !self.campaign.get(id).is_empty() {
                StringSource::Campaign
            } else {
                StringSource::Gui
            }
        } else if id < GUI_STRINGS_START {
            StringSource::Translation
        } else {
            StringSource::Gui
        }
    }
}

/// The table a string id was answered from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringSource {
    Level,
    Campaign,
    Translation,
    Gui,
    Invalid,
}
