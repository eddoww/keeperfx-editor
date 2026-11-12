//! The language tables of a pack: `[strings]` (campaign string files, one per language) and
//! `[speech]` (one speech folder per language).
//!
//! Reference: `parse_campaign_strings_blocks` / `parse_campaign_speech_blocks`
//! (src/config_campaigns.c:792-890) and `get_string` (src/config_strings.c:420). A string id below 2000
//! is looked up in the level's own table, then in the campaign table of the player's language
//! (the `[strings]` entry of that language, else the first entry; `strings_fname`), and when that
//! slot is empty in the GUI table `fxdata/gtext_<lang>.dat` (`cmpgn_string`:402). Ids from 2000
//! are `translation.toml` aliases, from 4000 GUI strings.
//!
//! Tables are edited in memory and written with [`Texts::save`]. A table whose file is not one of
//! the pack's own files (for example the shipped `fxdata/gtext_eng.dat` that the original
//! campaign points at) is marked `shared` and refuses edits.
use crate::pack::{is_own_location, Pack};
use kfx_formats::atomic::SaveTransaction;
use kfx_formats::strings::{Lang, StringTable, STRINGS_MAX};
use kfx_formats::testutil::resolve_ci;
use kfx_formats::{Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Number of (empty) strings in a table created by the editor. The game refuses strings files
/// shorter than 16 bytes (`MIN_STRINGS_FILE_SIZE`, setup_campaign_strings_data).
pub const NEW_TABLE_STRINGS: usize = 128;

/// One `[strings]` entry with its loaded table.
#[derive(Clone, Debug)]
pub struct LangTable {
    /// Language code as written (`ENG`).
    pub code: String,
    pub lang: Option<Lang>,
    /// File as written in the cfg (install relative).
    pub file: String,
    pub path: PathBuf,
    /// The file exists on disk (or was created by [`Texts::add_language`] and not saved yet).
    pub exists: bool,
    /// The file is not owned by the pack: editing is refused.
    pub shared: bool,
    pub table: Option<StringTable>,
    pub dirty: bool,
    pub error: Option<String>,
}

/// What a cell of the text grid is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellState {
    /// Has text.
    Ok,
    /// The language has no text while the first language does.
    Missing,
    /// No language has text for the id.
    Empty,
}

/// A string id a pack uses, with the place it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsedId {
    pub id: u32,
    pub label: String,
}

/// The language tables of a pack.
#[derive(Clone, Debug)]
pub struct Texts {
    pub install: PathBuf,
    pub langs: Vec<LangTable>,
    /// `[speech]`: (code, folder).
    pub speech: Vec<(String, String)>,
    gui: BTreeMap<u8, Option<StringTable>>,
}

fn err(msg: impl Into<String>) -> Error {
    Error::format("strings", msg)
}

impl Texts {
    /// Load every table named by the pack's `[strings]` block.
    pub fn load(pack: &Pack) -> Texts {
        let stem = pack.stem();
        let mut langs = vec![];
        for (code, file) in pack.cfg.strings() {
            let lang = Lang::from_code(&code);
            let path = pack.loc(&file);
            let exists = path.is_file();
            let shared = !is_own_location(&stem, &file);
            let (table, error) = match (exists, lang) {
                (true, Some(l)) => match StringTable::load(&path, l) {
                    Ok(t) => (Some(t), None),
                    Err(e) => (None, Some(e.to_string())),
                },
                (true, None) => (None, Some(format!("unknown language code {code}"))),
                (false, _) => (None, Some("file not found".to_string())),
            };
            langs.push(LangTable {
                code,
                lang,
                file,
                path,
                exists,
                shared,
                table,
                dirty: false,
                error,
            });
        }
        Texts {
            install: pack.install.clone(),
            langs,
            speech: pack.cfg.speech(),
            gui: BTreeMap::new(),
        }
    }

    /// Index of a language code.
    pub fn index_of(&self, code: &str) -> Option<usize> {
        self.langs
            .iter()
            .position(|l| l.code.eq_ignore_ascii_case(code))
    }

    /// The text of string `id` in the campaign table of language `i` (empty slots are `None`).
    pub fn get(&self, i: usize, id: u32) -> Option<String> {
        let t = self.langs.get(i)?.table.as_ref()?;
        t.get(id as usize).filter(|s| !s.is_empty())
    }

    fn gui_table(&mut self, lang: Lang) -> Option<&StringTable> {
        let install = self.install.clone();
        self.gui
            .entry(lang.id())
            .or_insert_with(|| {
                let fx = resolve_ci(&install, "fxdata")?;
                let p = resolve_ci(&fx, &lang.gtext_file())
                    .or_else(|| resolve_ci(&fx, &Lang::English.gtext_file()))?;
                StringTable::load(&p, lang).ok()
            })
            .as_ref()
    }

    /// Text of GUI string `id` (`fxdata/gtext_<lang>.dat`) in a language.
    pub fn gui_text(&mut self, lang: Lang, id: u32) -> Option<String> {
        self.gui_table(lang)?
            .get(id as usize)
            .filter(|s| !s.is_empty())
    }

    /// What the game shows for `id` in language `i`: the campaign text, else the GUI text
    /// (`cmpgn_string`, src/config_strings.c:402). `None` when both are empty.
    pub fn effective(&mut self, i: usize, id: u32) -> Option<String> {
        if let Some(s) = self.get(i, id) {
            return Some(s);
        }
        let lang = self.langs.get(i)?.lang?;
        self.gui_table(lang)?
            .get(id as usize)
            .filter(|s| !s.is_empty())
    }

    /// Edit a string. Refused for shared tables and for ids at or over `STRINGS_MAX`.
    pub fn set(&mut self, i: usize, id: u32, text: &str) -> Result<()> {
        if id as usize >= STRINGS_MAX {
            return Err(err(format!("string id {id} is not below {STRINGS_MAX}")));
        }
        let l = self
            .langs
            .get_mut(i)
            .ok_or_else(|| err("no such language"))?;
        if l.shared {
            return Err(err(format!(
                "{} is shared with the game ({}): not edited",
                l.code, l.file
            )));
        }
        let t = l
            .table
            .as_mut()
            .ok_or_else(|| err(format!("{} has no loaded table", l.code)))?;
        t.set(id as usize, text)?;
        l.dirty = true;
        Ok(())
    }

    /// State of a cell for the text grid (compared with the first language).
    pub fn cell(&self, i: usize, id: u32) -> CellState {
        if self.get(i, id).is_some() {
            return CellState::Ok;
        }
        let other = (0..self.langs.len()).any(|j| j != i && self.get(j, id).is_some());
        if other {
            CellState::Missing
        } else {
            CellState::Empty
        }
    }

    /// The smallest id from `from` whose slot is empty in every table.
    pub fn free_id(&self, from: u32) -> u32 {
        let mut id = from.max(1);
        while (0..self.langs.len()).any(|i| self.get(i, id).is_some()) {
            id += 1;
        }
        id
    }

    /// Add a language: a new empty table file `text_<lang>.dat` in the levels folder (kept in
    /// memory until [`Texts::save`]) and its `[strings]` entry in the cfg. The speech folder is
    /// added too when `speech_folder` is given.
    pub fn add_language(
        &mut self,
        pack: &mut Pack,
        lang: Lang,
        speech_folder: Option<&str>,
    ) -> Result<usize> {
        let code = lang.code().to_string();
        if self.index_of(&code).is_some() {
            return Err(err(format!("{code} is already in [strings]")));
        }
        let levels = pack
            .common()
            .levels_location
            .ok_or_else(|| err("the pack has no LEVELS_LOCATION"))?;
        let file = format!("{}/text_{}.dat", levels.trim_end_matches('/'), lang.lower());
        let mut table = StringTable::new(lang);
        table.set_raw(NEW_TABLE_STRINGS - 1, vec![]);
        pack.cfg.doc.add("strings", &code, &file);
        if let Some(f) = speech_folder {
            pack.cfg.doc.add("speech", &code, f);
            self.speech.push((code.clone(), f.to_string()));
        }
        let path = pack.loc(&file);
        self.langs.push(LangTable {
            code,
            lang: Some(lang),
            file,
            path,
            exists: true,
            shared: false,
            table: Some(table),
            dirty: true,
            error: None,
        });
        Ok(self.langs.len() - 1)
    }

    /// Remove a language from `[strings]` (and `[speech]`); the files stay on disk.
    pub fn remove_language(&mut self, pack: &mut Pack, i: usize) -> Result<()> {
        let l = self
            .langs
            .get(i)
            .ok_or_else(|| err("no such language"))?
            .clone();
        pack.cfg.doc.remove("strings", &l.code);
        pack.cfg.doc.remove("speech", &l.code);
        self.speech
            .retain(|(c, _)| !c.eq_ignore_ascii_case(&l.code));
        self.langs.remove(i);
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.langs.iter().any(|l| l.dirty)
    }

    /// Write every edited table (one transaction per folder).
    pub fn save(&mut self) -> Result<Vec<PathBuf>> {
        let mut by_dir: BTreeMap<PathBuf, SaveTransaction> = BTreeMap::new();
        for l in self.langs.iter().filter(|l| l.dirty && !l.shared) {
            let Some(t) = &l.table else { continue };
            let dir = l.path.parent().map(Path::to_path_buf).unwrap_or_default();
            let name = l
                .path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            by_dir
                .entry(dir.clone())
                .or_insert_with(|| SaveTransaction::new(&dir))
                .write(name, t.to_bytes());
        }
        let mut written = vec![];
        for (_, tx) in by_dir {
            written.extend(tx.commit()?.written);
        }
        for l in &mut self.langs {
            if !l.shared {
                l.dirty = false;
            }
        }
        Ok(written)
    }

    /// The string ids the pack uses: `NAME_ID` and `INTRO_DESC_ID` of its levels (numbers below
    /// `STRINGS_MAX`; aliases are `translation.toml` ids and not edited here) and the campaign
    /// description.
    pub fn used_ids(pack: &Pack) -> Vec<UsedId> {
        let mut out: Vec<UsedId> = vec![];
        let mut add = |id: u32, label: String| {
            if !out.iter().any(|u| u.id == id) {
                out.push(UsedId { id, label });
            }
        };
        for e in pack.entries() {
            if let Some(id) = e
                .info
                .name_id
                .as_deref()
                .and_then(|t| t.parse::<u32>().ok())
                .filter(|&i| i > 0 && (i as usize) < STRINGS_MAX)
            {
                add(id, format!("level {} name", e.number));
            }
            if let Some(id) = e
                .info
                .intro_desc_id
                .as_deref()
                .and_then(|t| t.trim().parse::<u32>().ok())
                .filter(|&i| i > 0 && (i as usize) < STRINGS_MAX)
            {
                add(id, format!("level {} description", e.number));
            }
        }
        out.sort_by_key(|u| u.id);
        out
    }
}
