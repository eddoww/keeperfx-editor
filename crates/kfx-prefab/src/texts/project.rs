//! The model of the translation grid: rows (string ids), columns (languages), cell states,
//! editing with the encodability check, outdated tracking, statistics and saving.
use super::text_hash;
use kfx_formats::atomic;
use kfx_formats::campaign::Campaign;
use kfx_formats::levelinfo::{Lif, Lof};
use kfx_formats::rnc;
use kfx_formats::strings::{self, Lang, StringTable, MIN_STRINGS_FILE_SIZE, STRINGS_MAX};
use kfx_formats::testutil::resolve_ci;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Whose strings file a row belongs to (see the module docs of [`crate::texts`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    /// The campaign's strings (shared by every level), also level names and descriptions.
    Campaign,
    /// The level's own strings (`mapNNNNN.<lang>.dat`).
    Level(u32),
}

impl Scope {
    /// `campaign` / `level:N`.
    pub fn key(self) -> String {
        match self {
            Scope::Campaign => "campaign".into(),
            Scope::Level(n) => format!("level:{n}"),
        }
    }
    pub fn parse(s: &str) -> Option<Scope> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("campaign") {
            return Some(Scope::Campaign);
        }
        s.strip_prefix("level:")
            .and_then(|n| n.trim().parse().ok())
            .map(Scope::Level)
    }
}

/// What kind of place refers to a string id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Script,
    Lua,
    LevelName,
    LevelDescription,
}

/// A place that uses a string id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    pub level: u32,
    pub file: String,
    /// 1-based line (0 when not a script).
    pub line: usize,
    pub command: String,
    pub kind: RefKind,
}

/// One row of the grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub scope: Scope,
    pub id: i64,
    pub refs: Vec<Ref>,
}

/// What to open.
#[derive(Clone, Debug)]
pub struct ProjectSpec {
    /// The install (read-only; `fxdata/gtext_*.dat` come from here).
    pub install: PathBuf,
    pub campaign_cfg: Option<PathBuf>,
    /// `[strings]` of the campaign: `(code, file relative to the install)`.
    pub campaign_strings: Vec<(String, String)>,
    pub levels_dir: PathBuf,
    pub levels: Vec<u32>,
    /// Columns shown first (the source language is added when missing).
    pub langs: Vec<Lang>,
    pub source: Lang,
}

impl ProjectSpec {
    /// A campaign: every level of its cfg (or only `levels`), the languages of its `[strings]`.
    pub fn for_campaign(
        install: &Path,
        cfg: &Path,
        levels: Option<Vec<u32>>,
    ) -> crate::Result<ProjectSpec> {
        let rc = kfx_formats::campaign::resolve(install, cfg)?;
        let camp = Campaign::load(cfg)?;
        let strings = camp.strings();
        let levels_dir = rc
            .levels_dir
            .clone()
            .ok_or_else(|| crate::Error::Invalid("the campaign has no levels folder".into()))?;
        let all: Vec<u32> = rc
            .levels
            .iter()
            .filter(|l| {
                l.slb.is_some() || levels_dir.join(format!("map{:05}.txt", l.number)).exists()
            })
            .map(|l| l.number)
            .collect();
        let mut langs: Vec<Lang> = strings
            .iter()
            .filter_map(|(c, _)| Lang::from_code(c))
            .collect();
        if !langs.contains(&Lang::English) {
            langs.insert(0, Lang::English);
        }
        Ok(ProjectSpec {
            install: install.to_path_buf(),
            campaign_cfg: Some(cfg.to_path_buf()),
            campaign_strings: strings,
            levels_dir,
            levels: levels.unwrap_or(all),
            langs,
            source: Lang::English,
        })
    }
}

/// Where a cell's text comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Level,
    Campaign,
    /// A file of the game install (never written).
    Shared,
    /// The GUI string of the same index (the game's last fallback).
    Gui,
    /// The text of the campaign's default language, which the game shows when the language has
    /// no table of its own (`campaign_strings_files`, `StringSources::resolve`).
    Fallback,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellState {
    /// The text of the pinned source language.
    Source,
    /// Translated (or untracked: no record says otherwise).
    Ok,
    /// A copy of the source text put there by "copy source to missing".
    Copied,
    /// The source text changed after this was translated.
    Outdated,
    /// No text in any table of the language.
    Missing,
    /// No text of its own, but a shared / GUI fallback exists.
    Inherited,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellInfo {
    pub text: String,
    pub state: CellState,
    pub origin: Origin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetError {
    UnknownRow,
    /// The language's code page cannot encode these characters (position in chars, char).
    Unencodable(Vec<(usize, char)>),
    ContainsNul,
    /// No file the text could be written to (message says why).
    NoTarget(String),
}

impl std::fmt::Display for SetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetError::UnknownRow => write!(f, "no such row"),
            SetError::Unencodable(v) => {
                let chars: String = v
                    .iter()
                    .map(|(_, c)| *c)
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                write!(
                    f,
                    "characters the language's code page cannot encode: {chars}"
                )
            }
            SetError::ContainsNul => write!(f, "a string cannot contain NUL"),
            SetError::NoTarget(m) => f.write_str(m),
        }
    }
}

/// Per-language statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LangStats {
    pub total: usize,
    pub translated: usize,
    pub copied: usize,
    pub outdated: usize,
    pub missing: usize,
    pub inherited: usize,
    /// Texts the language's code page cannot encode (cannot be saved as they are).
    pub unencodable: usize,
}

/// Result of an import.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub updated: Vec<(Scope, i64, Lang)>,
    pub unchanged: usize,
    /// Cells that were empty (nothing supplied).
    pub empty: usize,
    pub unknown: Vec<String>,
    pub errors: Vec<String>,
}

/// Result of "copy source to missing".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CopyReport {
    pub copied: usize,
    pub unencodable: Vec<(Scope, i64, Lang)>,
    pub no_target: Vec<(Scope, i64, Lang)>,
}

// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct TableFile {
    path: PathBuf,
    table: StringTable,
    exists: bool,
    dirty: bool,
}

impl TableFile {
    fn load(path: Option<PathBuf>, fallback: PathBuf, lang: Lang) -> TableFile {
        if let Some(p) = path {
            // too short for the game (< 16 bytes) is read as empty, kept as it is until written
            if let Ok(b) = rnc::load_file(&p) {
                return TableFile {
                    table: StringTable::parse(&b, lang),
                    path: p,
                    exists: true,
                    dirty: false,
                };
            }
        }
        TableFile {
            path: fallback,
            table: StringTable::new(lang),
            exists: false,
            dirty: false,
        }
    }
}

#[derive(Clone, Debug)]
struct CampaignTable {
    file: TableFile,
    /// In the campaign's own folder (false: a shared game file, read-only).
    own: bool,
}

#[derive(Clone, Debug, Default)]
struct LangData {
    levels: BTreeMap<u32, TableFile>,
    campaign: Option<CampaignTable>,
    /// What the game reads as campaign strings for a language with no entry of its own.
    campaign_fallback: Option<StringTable>,
    gui: Option<StringTable>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SideEntry {
    hash: String,
    copied: bool,
}

#[derive(Clone, Debug, Default)]
struct Sidecar {
    source: Option<Lang>,
    entries: BTreeMap<(i64, Lang), SideEntry>,
    dirty: bool,
}

/// The translation project of a level or campaign.
pub struct TextProject {
    pub spec: ProjectSpec,
    pub rows: Vec<Row>,
    /// Languages of the grid, source first.
    pub langs: Vec<Lang>,
    source: Lang,
    /// Ids that are not level / campaign strings (translation table, GUI): `(level, id)`.
    pub ignored: Vec<(u32, i64)>,
    data: BTreeMap<Lang, LangData>,
    sidecars: BTreeMap<Scope, Sidecar>,
    /// Language of the first `[strings]` entry: the game's fallback for level strings.
    default_lang: Option<Lang>,
}

fn is_under(path: &Path, dir: &Path) -> bool {
    let (p, d) = (
        path.to_string_lossy().to_lowercase(),
        dir.to_string_lossy().to_lowercase(),
    );
    p.starts_with(&d)
}

impl TextProject {
    /// Open the project: load the string files of every language, collect the references of
    /// every level, build the rows.
    pub fn open(spec: ProjectSpec) -> crate::Result<TextProject> {
        let mut langs = spec.langs.clone();
        langs.retain(|l| *l != spec.source);
        langs.sort();
        langs.dedup();
        langs.insert(0, spec.source);
        let default_lang =
            strings::campaign_strings_files(&spec.campaign_strings, spec.source).default_language;
        let mut p = TextProject {
            source: spec.source,
            rows: vec![],
            langs: vec![],
            ignored: vec![],
            data: BTreeMap::new(),
            sidecars: BTreeMap::new(),
            default_lang,
            spec,
        };
        for &l in &langs {
            p.ensure_lang(l);
        }
        p.langs = langs;
        let camp = p
            .spec
            .campaign_cfg
            .as_ref()
            .and_then(|c| Campaign::load(c).ok());
        let mut found: BTreeMap<(Scope, i64), Vec<Ref>> = BTreeMap::new();
        let levels = p.spec.levels.clone();
        for &n in &levels {
            for (id, r) in collect_level_refs(&p.spec.levels_dir, n, camp.as_ref()) {
                if !(0..STRINGS_MAX as i64).contains(&id) {
                    p.ignored.push((n, id));
                    continue;
                }
                let scope = match r.kind {
                    RefKind::LevelName | RefKind::LevelDescription => Scope::Campaign,
                    _ => p.scope_for(n, id),
                };
                found.entry((scope, id)).or_default().push(r);
            }
        }
        p.rows = found
            .into_iter()
            .map(|((scope, id), refs)| Row { scope, id, refs })
            .collect();
        p.rows.sort_by_key(|r| (r.scope, r.id));
        for s in p
            .rows
            .iter()
            .map(|r| r.scope)
            .collect::<std::collections::BTreeSet<_>>()
        {
            let sc = p.load_sidecar(s);
            p.sidecars.insert(s, sc);
        }
        Ok(p)
    }

    fn lang_dir_name(l: Lang) -> String {
        l.lower()
    }

    fn ensure_lang(&mut self, l: Lang) {
        if self.data.contains_key(&l) {
            return;
        }
        let mut d = LangData::default();
        for &n in &self.spec.levels {
            let name = l.map_strings_file(n);
            d.levels.insert(
                n,
                TableFile::load(
                    resolve_ci(&self.spec.levels_dir, &name),
                    self.spec.levels_dir.join(name.to_lowercase()),
                    l,
                ),
            );
        }
        // the campaign's own entry for exactly this language (the fallback to another language's
        // file is not "the language's table")
        if let Some((_, rel)) = self
            .spec
            .campaign_strings
            .iter()
            .find(|(c, _)| Lang::from_code(c) == Some(l))
        {
            let rel = rel.trim().replace('\\', "/");
            let found = resolve_ci(&self.spec.install, &rel);
            let path = found
                .clone()
                .unwrap_or_else(|| self.spec.install.join(&rel));
            let own = !is_under(
                &path,
                &resolve_ci(&self.spec.install, "fxdata")
                    .unwrap_or_else(|| self.spec.install.join("fxdata")),
            );
            d.campaign = Some(CampaignTable {
                file: TableFile::load(found, path, l),
                own,
            });
        } else {
            let cf = strings::campaign_strings_files(&self.spec.campaign_strings, l);
            if let (Some(f), Some(fl)) = (cf.fname, cf.fname_lang) {
                d.campaign_fallback = resolve_ci(&self.spec.install, &f.trim().replace('\\', "/"))
                    .and_then(|p| rnc::load_file(&p).ok())
                    .map(|b| StringTable::parse(&b, fl));
            }
        }
        let fx = resolve_ci(&self.spec.install, "fxdata")
            .unwrap_or_else(|| self.spec.install.join("fxdata"));
        d.gui = resolve_ci(&fx, &l.gtext_file())
            .and_then(|p| rnc::load_file(&p).ok())
            .map(|b| StringTable::parse(&b, l));
        self.data.insert(l, d);
        let _ = Self::lang_dir_name;
    }

    /// Add a language column.
    pub fn add_language(&mut self, l: Lang) {
        self.ensure_lang(l);
        if !self.langs.contains(&l) {
            self.langs.push(l);
        }
    }

    pub fn source(&self) -> Lang {
        self.source
    }

    /// Pin another source language (rows stay; outdated marks are relative to the source they
    /// were recorded for, so records of another source count as untracked).
    pub fn set_source(&mut self, l: Lang) {
        self.add_language(l);
        self.source = l;
        self.langs.retain(|x| *x != l);
        self.langs.insert(0, l);
    }

    fn scope_for(&self, level: u32, id: i64) -> Scope {
        let Some(d) = self.data.get(&self.source) else {
            return Scope::Level(level);
        };
        let get = |t: &StringTable| t.get(id as usize).filter(|s| !s.is_empty());
        if d.levels
            .get(&level)
            .is_some_and(|f| get(&f.table).is_some())
        {
            return Scope::Level(level);
        }
        if d.campaign
            .as_ref()
            .is_some_and(|c| c.own && get(&c.file.table).is_some())
        {
            return Scope::Campaign;
        }
        Scope::Level(level)
    }

    pub fn row_index(&self, scope: Scope, id: i64) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| r.scope == scope && r.id == id)
    }

    // ---------------------------------------------------------------------------- cells

    fn raw_cell(&self, row: &Row, lang: Lang) -> (String, Origin) {
        let Some(d) = self.data.get(&lang) else {
            return (String::new(), Origin::None);
        };
        let id = row.id as usize;
        let get = |t: &StringTable| t.get(id).filter(|s| !s.is_empty());
        // an edited file is written (padded to the game's minimum size) by the next save, so its
        // texts count as the file's already
        let usable = |f: &TableFile| {
            f.dirty || (f.exists && f.table.to_bytes().len() >= MIN_STRINGS_FILE_SIZE)
        };
        if let Scope::Level(n) = row.scope {
            // the game reads the language's level file when it is usable, else the default
            // language's (StringSources::resolve)
            match d.levels.get(&n).filter(|f| usable(f)) {
                Some(f) => {
                    if let Some(t) = get(&f.table) {
                        return (t, Origin::Level);
                    }
                }
                None => {
                    let fb = self
                        .default_lang
                        .filter(|dl| *dl != lang)
                        .and_then(|dl| self.data.get(&dl))
                        .and_then(|dd| dd.levels.get(&n))
                        .filter(|f| usable(f));
                    if let Some(t) = fb.and_then(|f| get(&f.table)) {
                        return (t, Origin::Fallback);
                    }
                }
            }
        }
        if let Some(c) = &d.campaign {
            if let Some(t) = get(&c.file.table) {
                return (
                    t,
                    if c.own {
                        Origin::Campaign
                    } else {
                        Origin::Shared
                    },
                );
            }
        } else if let Some(t) = d.campaign_fallback.as_ref().and_then(get) {
            return (t, Origin::Fallback);
        }
        if let Some(t) = d.gui.as_ref().and_then(get) {
            return (t, Origin::Gui);
        }
        (String::new(), Origin::None)
    }

    /// The text and state of cell (`row`, `lang`).
    pub fn cell(&self, row: usize, lang: Lang) -> CellInfo {
        let Some(r) = self.rows.get(row) else {
            return CellInfo {
                text: String::new(),
                state: CellState::Missing,
                origin: Origin::None,
            };
        };
        let (text, origin) = self.raw_cell(r, lang);
        if lang != self.source {
            // The game shows another language's text (the default language's file), or the GUI
            // string that happens to have the same index, where the author gave the source text
            // in the level / campaign file: neither is a translation of this text.
            let authored = matches!(
                self.raw_cell(r, self.source).1,
                Origin::Level | Origin::Campaign
            );
            if origin == Origin::Fallback
                || (authored && matches!(origin, Origin::Gui | Origin::Shared))
            {
                return CellInfo {
                    text: String::new(),
                    state: CellState::Missing,
                    origin,
                };
            }
        }
        let own = match r.scope {
            Scope::Level(_) => matches!(origin, Origin::Level | Origin::Campaign),
            Scope::Campaign => origin == Origin::Campaign,
        };
        let state = if lang == self.source {
            if text.is_empty() {
                CellState::Missing
            } else {
                CellState::Source
            }
        } else if origin == Origin::None {
            CellState::Missing
        } else if !own {
            CellState::Inherited
        } else {
            let src = self.raw_cell(r, self.source).0;
            match self
                .sidecars
                .get(&r.scope)
                .filter(|s| s.source == Some(self.source))
                .and_then(|s| s.entries.get(&(r.id, lang)))
            {
                Some(e) if e.hash != text_hash(&src) => CellState::Outdated,
                Some(e) if e.copied => CellState::Copied,
                _ => CellState::Ok,
            }
        };
        CellInfo {
            text,
            state,
            origin,
        }
    }

    /// Source text of a row.
    pub fn source_text(&self, row: usize) -> String {
        self.cell(row, self.source).text
    }

    /// Whether the language's code page can encode `text`; the characters it cannot.
    pub fn check_encodable(lang: Lang, text: &str) -> Result<(), Vec<(usize, char)>> {
        let bad: Vec<(usize, char)> = text
            .chars()
            .enumerate()
            .filter(|(_, c)| strings::encode(lang, &c.to_string()).is_err())
            .collect();
        if bad.is_empty() {
            Ok(())
        } else {
            Err(bad)
        }
    }

    // ---------------------------------------------------------------------------- targets

    /// The file a text of (`row`, `lang`) is written to, or why there is none.
    pub fn write_target(&self, row: usize, lang: Lang) -> Result<PathBuf, String> {
        let r = self.rows.get(row).ok_or("no such row")?;
        match r.scope {
            Scope::Level(n) => Ok(self
                .data
                .get(&lang)
                .and_then(|d| d.levels.get(&n))
                .map(|f| f.path.clone())
                .unwrap_or_else(|| {
                    self.spec
                        .levels_dir
                        .join(lang.map_strings_file(n).to_lowercase())
                })),
            Scope::Campaign => match self.data.get(&lang).and_then(|d| d.campaign.as_ref()) {
                Some(c) if c.own => Ok(c.file.path.clone()),
                Some(_) => Err(format!(
                    "the campaign uses the shared game file for {}",
                    lang.code()
                )),
                None => Err(format!(
                    "the campaign has no strings file for {}: create one first",
                    lang.code()
                )),
            },
        }
    }

    fn table_mut(&mut self, scope: Scope, lang: Lang) -> Result<&mut TableFile, String> {
        self.ensure_lang(lang);
        let levels_dir = self.spec.levels_dir.clone();
        let d = self.data.get_mut(&lang).ok_or("language not loaded")?;
        match scope {
            Scope::Level(n) => Ok(d.levels.entry(n).or_insert_with(|| TableFile {
                path: levels_dir.join(lang.map_strings_file(n).to_lowercase()),
                table: StringTable::new(lang),
                exists: false,
                dirty: false,
            })),
            Scope::Campaign => match &mut d.campaign {
                Some(c) if c.own => Ok(&mut c.file),
                Some(_) => Err(format!(
                    "the campaign uses the shared game file for {}",
                    lang.code()
                )),
                None => Err(format!(
                    "the campaign has no strings file for {}: create one first",
                    lang.code()
                )),
            },
        }
    }

    /// Make a campaign strings file for `lang` (an empty one in the campaign's folder, listed in
    /// the `[strings]` of the campaign cfg). Returns its path.
    pub fn create_campaign_file(&mut self, lang: Lang) -> crate::Result<PathBuf> {
        let Some(cfg) = self.spec.campaign_cfg.clone() else {
            return Err(crate::Error::Refused(
                "this project has no campaign cfg".into(),
            ));
        };
        self.ensure_lang(lang);
        if self
            .data
            .get(&lang)
            .and_then(|d| d.campaign.as_ref())
            .is_some()
        {
            return Err(crate::Error::Refused(format!(
                "the campaign already lists a strings file for {}",
                lang.code()
            )));
        }
        let rel_dir = self
            .spec
            .levels_dir
            .strip_prefix(&self.spec.install)
            .map_err(|_| {
                crate::Error::Invalid("the levels folder is not inside the install".into())
            })?;
        let name = format!("text_{}.dat", lang.lower());
        let rel = format!("{}/{}", rel_dir.to_string_lossy().replace('\\', "/"), name);
        let path = self.spec.levels_dir.join(&name);
        let mut t = StringTable::new(lang);
        pad_table(&mut t);
        atomic::atomic_write(&path, &t.to_bytes())
            .map_err(|e| crate::Error::Io(format!("{}: {e}", path.display())))?;
        let mut camp = Campaign::load(&cfg)?;
        camp.doc.add("strings", lang.code(), &rel);
        camp.save(&cfg)?;
        self.spec
            .campaign_strings
            .push((lang.code().to_string(), rel));
        if let Some(d) = self.data.get_mut(&lang) {
            d.campaign = Some(CampaignTable {
                file: TableFile {
                    path: path.clone(),
                    table: t,
                    exists: true,
                    dirty: false,
                },
                own: true,
            });
        }
        Ok(path)
    }

    // ---------------------------------------------------------------------------- editing

    /// Set the text of a cell. Checks the code page first; the text goes to the file the game
    /// reads that string from (see the module docs).
    pub fn set_text(&mut self, row: usize, lang: Lang, text: &str) -> Result<(), SetError> {
        let r = self.rows.get(row).cloned().ok_or(SetError::UnknownRow)?;
        if text.contains('\0') {
            return Err(SetError::ContainsNul);
        }
        if let Err(bad) = Self::check_encodable(lang, text) {
            return Err(SetError::Unencodable(bad));
        }
        let before = self.cell(row, lang).text;
        let src_hash = text_hash(&self.source_text(row));
        {
            let f = self.table_mut(r.scope, lang).map_err(SetError::NoTarget)?;
            if before != text || !f.exists {
                f.table
                    .set(r.id as usize, text)
                    .map_err(|e| SetError::NoTarget(e.to_string()))?;
                f.dirty = true;
            }
        }
        let source = self.source;
        let sc = self.sidecars.entry(r.scope).or_default();
        if lang != source {
            if sc.source != Some(source) {
                // records for another source language no longer apply
                sc.entries.clear();
                sc.source = Some(source);
            }
            if text.is_empty() {
                sc.entries.remove(&(r.id, lang));
            } else {
                sc.entries.insert(
                    (r.id, lang),
                    SideEntry {
                        hash: src_hash,
                        copied: false,
                    },
                );
            }
            sc.dirty = true;
        }
        Ok(())
    }

    /// Record that the translation of a cell is up to date with the current source text.
    pub fn mark_current(&mut self, row: usize, lang: Lang) {
        let Some(r) = self.rows.get(row).cloned() else {
            return;
        };
        let h = text_hash(&self.source_text(row));
        let source = self.source;
        let sc = self.sidecars.entry(r.scope).or_default();
        if sc.source != Some(source) {
            sc.entries.clear();
            sc.source = Some(source);
        }
        sc.entries.insert(
            (r.id, lang),
            SideEntry {
                hash: h,
                copied: false,
            },
        );
        sc.dirty = true;
    }

    /// Put the source text into every missing cell of `langs` (default: all but the source) that
    /// can take it. The cells show as [`CellState::Copied`] until someone edits them.
    pub fn copy_source_to_missing(&mut self, langs: Option<&[Lang]>) -> CopyReport {
        let mut rep = CopyReport::default();
        let langs: Vec<Lang> = langs.map(|l| l.to_vec()).unwrap_or_else(|| {
            self.langs
                .iter()
                .copied()
                .filter(|l| *l != self.source)
                .collect()
        });
        for ri in 0..self.rows.len() {
            let src = self.source_text(ri);
            if src.is_empty() {
                continue;
            }
            for &l in &langs {
                if l == self.source || self.cell(ri, l).state != CellState::Missing {
                    continue;
                }
                let (scope, id) = (self.rows[ri].scope, self.rows[ri].id);
                match self.set_text(ri, l, &src) {
                    Ok(()) => {
                        if let Some(e) = self
                            .sidecars
                            .get_mut(&scope)
                            .and_then(|s| s.entries.get_mut(&(id, l)))
                        {
                            e.copied = true;
                        }
                        rep.copied += 1;
                    }
                    Err(SetError::Unencodable(_)) => rep.unencodable.push((scope, id, l)),
                    Err(_) => rep.no_target.push((scope, id, l)),
                }
            }
        }
        rep
    }

    /// Shared by CSV and `.po` import: apply one supplied text.
    pub(crate) fn import_cell(
        &mut self,
        rep: &mut ImportReport,
        ri: usize,
        lang: Lang,
        text: &str,
        fuzzy: bool,
    ) {
        if text.is_empty() {
            rep.empty += 1;
            return;
        }
        if self.cell(ri, lang).text == text {
            rep.unchanged += 1;
            return;
        }
        let (scope, id) = (self.rows[ri].scope, self.rows[ri].id);
        match self.set_text(ri, lang, text) {
            Ok(()) => {
                if fuzzy && lang != self.source {
                    if let Some(e) = self
                        .sidecars
                        .get_mut(&scope)
                        .and_then(|s| s.entries.get_mut(&(id, lang)))
                    {
                        e.hash = "fuzzy".into();
                    }
                }
                rep.updated.push((scope, id, lang));
            }
            Err(e) => rep
                .errors
                .push(format!("{} id {id} {}: {e}", scope.key(), lang.code())),
        }
    }

    // ---------------------------------------------------------------------------- statistics

    pub fn stats(&self, lang: Lang) -> LangStats {
        let mut s = LangStats {
            total: self.rows.len(),
            ..Default::default()
        };
        for ri in 0..self.rows.len() {
            let c = self.cell(ri, lang);
            match c.state {
                CellState::Source | CellState::Ok => s.translated += 1,
                CellState::Copied => s.copied += 1,
                CellState::Outdated => s.outdated += 1,
                CellState::Missing => s.missing += 1,
                CellState::Inherited => s.inherited += 1,
            }
            if !c.text.is_empty() && strings::encode(lang, &c.text).is_err() {
                s.unencodable += 1;
            }
        }
        s
    }

    // ---------------------------------------------------------------------------- saving

    fn sidecar_path(&self, scope: Scope) -> PathBuf {
        match scope {
            Scope::Level(n) => self
                .spec
                .levels_dir
                .join(format!("map{n:05}.kfxtexts.toml")),
            Scope::Campaign => match &self.spec.campaign_cfg {
                Some(c) => c.with_extension("kfxtexts.toml"),
                None => self.spec.levels_dir.join("campaign.kfxtexts.toml"),
            },
        }
    }

    fn load_sidecar(&self, scope: Scope) -> Sidecar {
        let Ok(text) = std::fs::read_to_string(self.sidecar_path(scope)) else {
            return Sidecar::default();
        };
        let Ok(t) = text.parse::<toml::Table>() else {
            return Sidecar::default();
        };
        let mut sc = Sidecar {
            source: t
                .get("source")
                .and_then(|v| v.as_str())
                .and_then(Lang::from_code),
            ..Default::default()
        };
        if let Some(toml::Value::Array(a)) = t.get("text") {
            for e in a.iter().filter_map(|v| v.as_table()) {
                let (Some(id), Some(lang), Some(hash)) = (
                    e.get("id").and_then(|v| v.as_integer()),
                    e.get("lang")
                        .and_then(|v| v.as_str())
                        .and_then(Lang::from_code),
                    e.get("hash").and_then(|v| v.as_str()),
                ) else {
                    continue;
                };
                sc.entries.insert(
                    (id, lang),
                    SideEntry {
                        hash: hash.to_string(),
                        copied: e.get("copied").and_then(|v| v.as_bool()).unwrap_or(false),
                    },
                );
            }
        }
        sc
    }

    fn sidecar_text(sc: &Sidecar) -> String {
        let mut out = String::from(
            "# Translation tracking of the KeeperFX editor: hash of the \
                source text each translation was made from.\nversion = 1\n",
        );
        if let Some(s) = sc.source {
            out.push_str(&format!("source = \"{}\"\n", s.code()));
        }
        for ((id, lang), e) in &sc.entries {
            out.push_str(&format!(
                "\n[[text]]\nid = {id}\nlang = \"{}\"\nhash = \"{}\"\n",
                lang.code(),
                e.hash
            ));
            if e.copied {
                out.push_str("copied = true\n");
            }
        }
        out
    }

    /// Whether anything is unsaved.
    pub fn is_dirty(&self) -> bool {
        self.data.values().any(|d| {
            d.levels.values().any(|f| f.dirty) || d.campaign.as_ref().is_some_and(|c| c.file.dirty)
        }) || self.sidecars.values().any(|s| s.dirty)
    }

    /// The files [`TextProject::save`] would write now (strings files and sidecars), so a host
    /// can back them up first.
    pub fn dirty_files(&self) -> Vec<PathBuf> {
        let mut out = vec![];
        for d in self.data.values() {
            out.extend(
                d.levels
                    .values()
                    .filter(|f| f.dirty)
                    .map(|f| f.path.clone()),
            );
            if let Some(c) = d.campaign.as_ref().filter(|c| c.own && c.file.dirty) {
                out.push(c.file.path.clone());
            }
        }
        out.extend(
            self.sidecars
                .iter()
                .filter(|(_, s)| s.dirty)
                .map(|(k, _)| self.sidecar_path(*k)),
        );
        out
    }

    /// Write every changed strings file and sidecar. Returns the files written.
    pub fn save(&mut self) -> crate::Result<Vec<PathBuf>> {
        let mut written = vec![];
        let io = |p: &Path, e: std::io::Error| crate::Error::Io(format!("{}: {e}", p.display()));
        for d in self.data.values_mut() {
            let mut files: Vec<&mut TableFile> = d.levels.values_mut().collect();
            if let Some(c) = d.campaign.as_mut().filter(|c| c.own) {
                files.push(&mut c.file);
            }
            for f in files.into_iter().filter(|f| f.dirty) {
                pad_table(&mut f.table);
                atomic::atomic_write(&f.path, &f.table.to_bytes()).map_err(|e| io(&f.path, e))?;
                f.dirty = false;
                f.exists = true;
                written.push(f.path.clone());
            }
        }
        let scopes: Vec<Scope> = self
            .sidecars
            .iter()
            .filter(|(_, s)| s.dirty)
            .map(|(k, _)| *k)
            .collect();
        for s in scopes {
            let path = self.sidecar_path(s);
            let sc = self
                .sidecars
                .get_mut(&s)
                .ok_or_else(|| crate::Error::Invalid("sidecar".into()))?;
            if sc.entries.is_empty() {
                atomic::atomic_remove(&path).map_err(|e| io(&path, e))?;
            } else {
                atomic::atomic_write(&path, Self::sidecar_text(sc).as_bytes())
                    .map_err(|e| io(&path, e))?;
                written.push(path);
            }
            sc.dirty = false;
        }
        Ok(written)
    }
}

/// The game rejects strings files shorter than 16 bytes: pad with empty strings.
fn pad_table(t: &mut StringTable) {
    while t.to_bytes().len() < MIN_STRINGS_FILE_SIZE {
        let n = t.len();
        t.set_raw(n, vec![]);
    }
}

/// Every string id reference of level `n`: `(id, where)`.
pub fn collect_level_refs(levels_dir: &Path, n: u32, camp: Option<&Campaign>) -> Vec<(i64, Ref)> {
    let mut out = vec![];
    let name = |ext: &str| format!("map{n:05}.{ext}");
    if let Some(p) = resolve_ci(levels_dir, &name("txt")) {
        if let Ok(bytes) = rnc::load_file(&p) {
            let doc = kfx_script::Document::parse(&bytes);
            for a in kfx_script::string_args(&doc) {
                if let Some(id) = a.id {
                    out.push((
                        id,
                        Ref {
                            level: n,
                            file: name("txt"),
                            line: doc.line_of(a.span.start) + 1,
                            command: a.command,
                            kind: RefKind::Script,
                        },
                    ));
                }
            }
        }
    }
    if let Some(p) = resolve_ci(levels_dir, &name("lua")) {
        if let Ok(bytes) = std::fs::read(&p) {
            let src = String::from_utf8_lossy(&bytes).to_string();
            let doc = kfx_script::lua::LuaDoc::parse(&src);
            for a in kfx_script::lua::string_args(&doc, &kfx_script::lua::ApiIndex::stock()) {
                if let Some(id) = a.id {
                    out.push((
                        id,
                        Ref {
                            level: n,
                            file: name("lua"),
                            line: doc.line_of(a.span.start) + 1,
                            command: a.command,
                            kind: RefKind::Lua,
                        },
                    ));
                }
            }
        }
    }
    let name_ref = |file: &str, kind: RefKind, id: i64| {
        (
            id,
            Ref {
                level: n,
                file: file.to_string(),
                line: 0,
                command: if kind == RefKind::LevelName {
                    "NAME_ID".into()
                } else {
                    "INTRO_DESC_ID".into()
                },
                kind,
            },
        )
    };
    if let Some(p) = resolve_ci(levels_dir, &name("lif")) {
        if let Ok(lif) = Lif::load(&p) {
            for e in lif.entries().into_iter().filter(|e| e.number == n as i32) {
                if let Some(id) = e.string_id {
                    out.push(name_ref(&name("lif"), RefKind::LevelName, id as i64));
                }
            }
        }
    }
    let mut add_info = |file: &str, info: kfx_formats::levelinfo::LevelInfo| {
        if let Some(id) = info
            .name_id
            .as_deref()
            .and_then(|s| s.trim().parse::<i64>().ok())
        {
            out.push(name_ref(file, RefKind::LevelName, id));
        }
        if let Some(id) = info
            .intro_desc_id
            .as_deref()
            .and_then(|s| s.trim().parse::<i64>().ok())
        {
            out.push(name_ref(file, RefKind::LevelDescription, id));
        }
    };
    if let Some(p) = resolve_ci(levels_dir, &name("lof")) {
        if let Ok(lof) = Lof::load(&p) {
            add_info(&name("lof"), lof.info());
        }
    }
    if let Some(info) = camp.and_then(|c| c.map_info(n)) {
        add_info(&format!("[map{n:05}]"), info);
    }
    out
}
