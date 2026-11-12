//! Level information files: `.lof` (Level Overview File) and `.lif` (Level Index File), and the
//! shared typed [`LevelInfo`] record that also describes a `[mapNNNNN]` block of a campaign cfg.
//!
//! # LOF
//! Reference: `level_lof_file_parse` (`src/lvl_filesdk1.c:344`), key table `cmpgn_map_commands`
//! (`src/config_campaigns.c:92`). A LOF is a `.cfg`-dialect file ([`crate::cfg`]) without sections:
//! parsing stops at the first line starting with `[` (`ccr_endOfBlock`, 379). The file name must be
//! `mapNNNNN.lof` (`get_level_number_from_file_name`, 195); the level number comes from the name,
//! not from the content.
//!
//! Keys and how the game reads them (lvl_filesdk1.c line of the case):
//!
//! | key | value | game |
//! |-----|-------|------|
//! | `NAME_TEXT` | whole line | 382, `LINEMSG_SIZE` |
//! | `NAME_ID` | one number (`atoi`, must be > 0) in LOF; in campaign `[mapNNNNN]` it is a string *alias* (`get_string_id_by_alias`, config_campaigns.c:~948), so kept as raw token here | 392 |
//! | `ENSIGN_POS` | two numbers x y, each > 0 | 405 |
//! | `ENSIGN_ZOOM` | two numbers, each > 0 | 431 |
//! | `PLAYERS` | one number > 0 | 457 |
//! | `ENSIGN` / `OPTIONS` (legacy alias, same command 6) | one token: DEFAULT TUTORIAL SINGLE BONUS FULL_MOON NEW_MOON COOP or a custom ensign name | 472 |
//! | `SPEECH` | two file names (before, after) | 493 |
//! | `LAND_VIEW` | two file names (view, window) | 510 |
//! | `KIND` | any of SINGLE MULTI BONUS EXTRA FREE (LOF only) | 527 |
//! | `AUTHOR` `DESCRIPTION` `DATE` `MAP_FORMAT_VERSION` | ignored by the game (whole line text) | 590 |
//! | `MAPSIZE` | two numbers x y, each > 0, default 85 85 (`DEFAULT_MAP_SIZE`, src/map_data.h:55) | 596 |
//! | `INTRO_DESC_ID` | whole line (campaign block only, config_campaigns.c:~1100) | - |
//! | `DESCRIPTION_GEO` | four numbers (campaign block only) | - |
//!
//! Real-file observations are in the tests (`tests/levelinfo.rs`).
//!
//! # LIF
//! Reference: `level_lif_entry_parse` (`src/lvl_filesdk1.c:182`). Each entry is
//! `<number><sep><name>` where `<sep>` is any run of whitespace, `,`, `;`, `:`; the name runs to
//! the end of the line. A `;` before the number comments out the rest of the line. If the name
//! starts with `#` the digits after it are a string id (`set_level_info_string_index`) and the
//! level name stays the literal text `#NNN` (the C code steps the pointer back onto the `#`). A
//! line that does not start with a number (after separators) ends the whole parse in the game only
//! through `Can't read level number` (returns 0 -> stops); the model here keeps such lines as
//! opaque text instead.
use crate::cfg::{latin1_decode, latin1_encode, CfgDoc};
use crate::{rnc, Error, Result};
use std::fmt;
use std::path::Path;

/// `DEFAULT_MAP_SIZE` (src/map_data.h:55).
pub const DEFAULT_MAP_SIZE: i32 = 85;

/// Level list membership (`cmpgn_map_cmnds_kind`, config_campaigns.c:~127).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LevelKind {
    Single,
    Multi,
    Bonus,
    Extra,
    Free,
}

impl LevelKind {
    pub const ALL: [LevelKind; 5] = [
        LevelKind::Single,
        LevelKind::Multi,
        LevelKind::Bonus,
        LevelKind::Extra,
        LevelKind::Free,
    ];
    pub fn name(self) -> &'static str {
        match self {
            LevelKind::Single => "SINGLE",
            LevelKind::Multi => "MULTI",
            LevelKind::Bonus => "BONUS",
            LevelKind::Extra => "EXTRA",
            LevelKind::Free => "FREE",
        }
    }
    pub fn from_name(s: &str) -> Option<LevelKind> {
        LevelKind::ALL
            .into_iter()
            .find(|k| k.name().eq_ignore_ascii_case(s))
    }
}

/// C `atoi` on a token: optional sign and leading digits, 0 when none.
pub fn atoi(s: &str) -> i32 {
    let t = s.trim_start();
    let (neg, rest) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let mut v: i64 = 0;
    for c in rest.chars() {
        match c.to_digit(10) {
            Some(d) => v = (v * 10 + d as i64).min(i32::MAX as i64),
            None => break,
        }
    }
    (if neg { -v } else { v }) as i32
}

fn pos_pair(v: &[String]) -> Option<(i32, i32)> {
    if v.len() < 2 {
        return None;
    }
    let (a, b) = (atoi(&v[0]), atoi(&v[1]));
    (a > 0 && b > 0).then_some((a, b))
}

/// Set `keys[0]` (or whichever alias key already exists) in `section` when `changed`.
pub(crate) fn put(
    doc: &mut CfgDoc,
    section: &str,
    keys: &[&str],
    new: Option<String>,
    changed: bool,
) {
    if !changed {
        return;
    }
    // reuse whichever of the alias keys exists (last occurrence), else the first name
    let existing = doc
        .entries(section)
        .into_iter()
        .rev()
        .find(|e| keys.iter().any(|k| e.key.eq_ignore_ascii_case(k)))
        .map(|e| e.key);
    let key = existing.clone().unwrap_or_else(|| keys[0].to_string());
    match new {
        Some(v) => doc.set(section, &key, &v),
        None if existing.is_some() => doc.set(section, &key, ""),
        None => {}
    }
}

/// Typed content of a LOF file or of a `[mapNNNNN]` campaign block. `None` = key absent or empty
/// or not accepted by the game's parser.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelInfo {
    pub name_text: Option<String>,
    /// Raw token: a number in LOF files, a number or string alias in campaign blocks.
    pub name_id: Option<String>,
    pub ensign_pos: Option<(i32, i32)>,
    pub ensign_zoom: Option<(i32, i32)>,
    pub players: Option<i32>,
    /// DEFAULT / TUTORIAL / ... or a custom ensign name, as written.
    pub ensign: Option<String>,
    /// (before, after) speech file names.
    pub speech: Option<(String, String)>,
    /// (land view, land window) file names.
    pub land_view: Option<(String, String)>,
    pub kinds: Vec<LevelKind>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub date: Option<String>,
    pub map_format_version: Option<String>,
    pub mapsize: Option<(i32, i32)>,
    pub intro_desc_id: Option<String>,
    /// x y width height (campaign block / `[common]` only).
    pub description_geo: Option<[i32; 4]>,
}

impl LevelInfo {
    /// Map size with the game default applied.
    pub fn mapsize_or_default(&self) -> (i32, i32) {
        self.mapsize.unwrap_or((DEFAULT_MAP_SIZE, DEFAULT_MAP_SIZE))
    }

    /// Read from `section` of `doc` (`""` for a LOF). The last occurrence of each key wins.
    pub fn read(doc: &CfgDoc, section: &str) -> LevelInfo {
        let ents = doc.entries(section);
        let last = |keys: &[&str]| -> Option<&crate::cfg::Entry> {
            ents.iter()
                .rev()
                .find(|e| keys.iter().any(|k| e.key.eq_ignore_ascii_case(k)))
        };
        let text = |k: &str| {
            last(&[k])
                .map(|e| e.value.clone())
                .filter(|v| !v.is_empty())
        };
        let tok = |k: &str| last(&[k]).and_then(|e| e.tokens.first().cloned());
        let mut kinds = Vec::new();
        for e in ents.iter().filter(|e| e.key.eq_ignore_ascii_case("KIND")) {
            for t in &e.tokens {
                match LevelKind::from_name(t) {
                    Some(k) => {
                        if !kinds.contains(&k) {
                            kinds.push(k)
                        }
                    }
                    None => break,
                }
            }
        }
        let pair_str = |k: &str| {
            last(&[k]).and_then(|e| {
                (e.tokens.len() >= 2).then(|| (e.tokens[0].clone(), e.tokens[1].clone()))
            })
        };
        LevelInfo {
            name_text: text("NAME_TEXT"),
            name_id: tok("NAME_ID"),
            ensign_pos: last(&["ENSIGN_POS"]).and_then(|e| pos_pair(&e.tokens)),
            ensign_zoom: last(&["ENSIGN_ZOOM"]).and_then(|e| pos_pair(&e.tokens)),
            players: last(&["PLAYERS"])
                .and_then(|e| e.tokens.first().map(|t| atoi(t)))
                .filter(|&k| k > 0),
            ensign: last(&["ENSIGN", "OPTIONS"]).and_then(|e| e.tokens.first().cloned()),
            speech: pair_str("SPEECH"),
            land_view: pair_str("LAND_VIEW"),
            kinds,
            author: text("AUTHOR"),
            description: text("DESCRIPTION"),
            date: text("DATE"),
            map_format_version: text("MAP_FORMAT_VERSION"),
            mapsize: last(&["MAPSIZE"]).and_then(|e| pos_pair(&e.tokens)),
            intro_desc_id: text("INTRO_DESC_ID"),
            description_geo: last(&["DESCRIPTION_GEO"]).and_then(|e| {
                if e.tokens.len() < 4 {
                    return None;
                }
                let v: Vec<i32> = e.tokens.iter().take(4).map(|t| atoi(t)).collect();
                Some([v[0], v[1], v[2], v[3]])
            }),
        }
    }

    /// Write changed fields into `section` of `doc`. Only fields that differ from what
    /// [`LevelInfo::read`] currently returns are touched, so unrelated lines, comments, unknown
    /// keys and original spelling survive. A field that becomes `None` keeps its key with an empty
    /// value (that is how the game's own exporter writes it).
    pub fn write(&self, doc: &mut CfgDoc, section: &str) {
        let old = LevelInfo::read(doc, section);
        let pair = |p: Option<(i32, i32)>| p.map(|(a, b)| format!("{a} {b}"));
        let spair = |p: &Option<(String, String)>| p.as_ref().map(|(a, b)| format!("{a} {b}"));
        put(
            doc,
            section,
            &["NAME_TEXT"],
            self.name_text.clone(),
            self.name_text != old.name_text,
        );
        put(
            doc,
            section,
            &["NAME_ID"],
            self.name_id.clone(),
            self.name_id != old.name_id,
        );
        put(
            doc,
            section,
            &["ENSIGN_POS"],
            pair(self.ensign_pos),
            self.ensign_pos != old.ensign_pos,
        );
        put(
            doc,
            section,
            &["ENSIGN_ZOOM"],
            pair(self.ensign_zoom),
            self.ensign_zoom != old.ensign_zoom,
        );
        put(
            doc,
            section,
            &["PLAYERS"],
            self.players.map(|p| p.to_string()),
            self.players != old.players,
        );
        put(
            doc,
            section,
            &["ENSIGN", "OPTIONS"],
            self.ensign.clone(),
            self.ensign != old.ensign,
        );
        put(
            doc,
            section,
            &["SPEECH"],
            spair(&self.speech),
            self.speech != old.speech,
        );
        put(
            doc,
            section,
            &["LAND_VIEW"],
            spair(&self.land_view),
            self.land_view != old.land_view,
        );
        let kinds = (!self.kinds.is_empty()).then(|| {
            self.kinds
                .iter()
                .map(|k| k.name())
                .collect::<Vec<_>>()
                .join(" ")
        });
        put(doc, section, &["KIND"], kinds, self.kinds != old.kinds);
        put(
            doc,
            section,
            &["AUTHOR"],
            self.author.clone(),
            self.author != old.author,
        );
        put(
            doc,
            section,
            &["DESCRIPTION"],
            self.description.clone(),
            self.description != old.description,
        );
        put(
            doc,
            section,
            &["DATE"],
            self.date.clone(),
            self.date != old.date,
        );
        put(
            doc,
            section,
            &["MAP_FORMAT_VERSION"],
            self.map_format_version.clone(),
            self.map_format_version != old.map_format_version,
        );
        put(
            doc,
            section,
            &["MAPSIZE"],
            pair(self.mapsize),
            self.mapsize != old.mapsize,
        );
        put(
            doc,
            section,
            &["INTRO_DESC_ID"],
            self.intro_desc_id.clone(),
            self.intro_desc_id != old.intro_desc_id,
        );
        put(
            doc,
            section,
            &["DESCRIPTION_GEO"],
            self.description_geo
                .map(|g| format!("{} {} {} {}", g[0], g[1], g[2], g[3])),
            self.description_geo != old.description_geo,
        );
    }
}

/// A `.lof` file: a [`CfgDoc`] (so comments, unknown keys and spelling survive) with a typed
/// view.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Lof {
    pub doc: CfgDoc,
}

impl Lof {
    pub fn parse(bytes: &[u8]) -> Lof {
        Lof {
            doc: CfgDoc::from_bytes(bytes),
        }
    }
    pub fn load(path: &Path) -> Result<Lof> {
        Ok(Lof::parse(&rnc::load_file(path)?))
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.doc.save(path)
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        self.doc.to_bytes()
    }
    /// Typed view of the preamble (the only part the game reads).
    pub fn info(&self) -> LevelInfo {
        LevelInfo::read(&self.doc, "")
    }
    /// Apply changed fields (see [`LevelInfo::write`]).
    pub fn set_info(&mut self, info: &LevelInfo) {
        info.write(&mut self.doc, "");
    }
    /// A new LOF in the shape KeeperFX tools write (header comment, version, all keys present).
    pub fn new(info: &LevelInfo) -> Lof {
        let mut s = String::from("; KeeperFX Level Overview File (LOF)\n");
        let v = info
            .map_format_version
            .clone()
            .unwrap_or_else(|| "1.06".into());
        s.push_str(&format!("MAP_FORMAT_VERSION = {v}\n"));
        for k in [
            "NAME_TEXT",
            "NAME_ID",
            "KIND",
            "ENSIGN_POS",
            "ENSIGN_ZOOM",
            "PLAYERS",
            "OPTIONS",
            "SPEECH",
            "LAND_VIEW",
            "AUTHOR",
            "DESCRIPTION",
            "DATE",
            "MAPSIZE",
        ] {
            s.push_str(&format!("{k} = \n"));
        }
        let mut lof = Lof {
            doc: CfgDoc::parse(&s),
        };
        let mut i = info.clone();
        i.map_format_version = Some(v);
        lof.set_info(&i);
        lof
    }
}

/// Level number from a `mapNNNNN.xxx` file name (`get_level_number_from_file_name`).
pub fn level_number_from_file_name(name: &str) -> Option<u32> {
    let n = name.get(..3)?;
    if !n.eq_ignore_ascii_case("map") {
        return None;
    }
    let v = atoi(&name[3..]);
    (v > 0).then_some(v as u32)
}

// ---------------------------------------------------------------- LIF

/// One level entry of a LIF file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifEntry {
    pub number: i32,
    /// Name text to end of line, exactly (e.g. `Red Carpet` or `#222`).
    pub name: String,
    /// Set when the name starts with `#`: the string id following it.
    pub string_id: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LifLine {
    text: String,
    eol: String,
    entry: Option<LifEntry>,
}

/// A `.lif` file, lossless (comments and unparsable lines are kept verbatim).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Lif {
    lines: Vec<LifLine>,
}

fn is_lif_skip(c: char) -> bool {
    c.is_ascii_whitespace() || matches!(c, ',' | ':' | ';')
}

fn parse_lif_line(text: &str) -> Option<LifEntry> {
    let t = text.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',' || c == ':');
    if t.is_empty() || t.starts_with(';') {
        return None;
    }
    let digits_end = {
        let mut i = 0;
        let b = t.as_bytes();
        if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
            i += 1;
        }
        let s = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == s {
            return None;
        }
        i
    };
    let number = atoi(&t[..digits_end]);
    let name = t[digits_end..].trim_start_matches(is_lif_skip);
    if name.is_empty() {
        return None; // "Can't read level name"
    }
    let string_id = name.strip_prefix('#').map(atoi);
    Some(LifEntry {
        number,
        name: name.to_string(),
        string_id,
    })
}

impl Lif {
    pub fn parse(bytes: &[u8]) -> Lif {
        let s = latin1_decode(bytes);
        let chars: Vec<char> = s.chars().collect();
        let mut lines = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let st = i;
            while i < chars.len() && chars[i] != '\n' && chars[i] != '\r' {
                i += 1;
            }
            let text: String = chars[st..i].iter().collect();
            let mut eol = String::new();
            if i < chars.len() {
                if chars[i] == '\r' && chars.get(i + 1) == Some(&'\n') {
                    eol.push_str("\r\n");
                    i += 2;
                } else {
                    eol.push(chars[i]);
                    i += 1;
                }
            }
            let entry = parse_lif_line(&text);
            lines.push(LifLine { text, eol, entry });
        }
        Lif { lines }
    }
    pub fn load(path: &Path) -> Result<Lif> {
        Ok(Lif::parse(&rnc::load_file(path)?))
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        crate::atomic::atomic_write(path, &self.to_bytes()).map_err(|e| Error::io(path, e))
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        latin1_encode(&self.to_string())
    }
    pub fn entries(&self) -> Vec<&LifEntry> {
        self.lines.iter().filter_map(|l| l.entry.as_ref()).collect()
    }
    /// Lines that are neither blank nor an entry nor a `;` comment (what the game would stumble
    /// on).
    pub fn odd_lines(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter(|l| l.entry.is_none())
            .map(|l| l.text.as_str())
            .filter(|t| {
                let t = t.trim_matches(|c: char| is_lif_skip(c) && c != ';');
                !t.is_empty() && !t.starts_with(';')
            })
            .collect()
    }
    fn dominant_eol(&self) -> String {
        let crlf = self.lines.iter().filter(|l| l.eol == "\r\n").count();
        let lf = self.lines.iter().filter(|l| l.eol == "\n").count();
        if crlf > lf {
            "\r\n".into()
        } else {
            "\n".into()
        }
    }
    /// Set (or add) the name of a level. Rewrites that line as `NN, name`.
    pub fn set_entry(&mut self, number: i32, name: &str) {
        let text = format!("{number}, {name}");
        let entry = parse_lif_line(&text);
        if let Some(l) = self
            .lines
            .iter_mut()
            .find(|l| l.entry.as_ref().is_some_and(|e| e.number == number))
        {
            l.text = text;
            l.entry = entry;
            return;
        }
        let eol = self.dominant_eol();
        if let Some(last) = self.lines.last_mut() {
            if last.eol.is_empty() {
                last.eol = eol.clone();
            }
        }
        self.lines.push(LifLine { text, eol, entry });
    }
    /// Remove all entries for a level number. Returns how many lines were dropped.
    pub fn remove_entry(&mut self, number: i32) -> usize {
        let before = self.lines.len();
        self.lines
            .retain(|l| !l.entry.as_ref().is_some_and(|e| e.number == number));
        before - self.lines.len()
    }
}

impl fmt::Display for Lif {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for l in &self.lines {
            f.write_str(&l.text)?;
            f.write_str(&l.eol)?;
        }
        Ok(())
    }
}
