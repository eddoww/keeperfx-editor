//! `fxdata/translation.toml`: text aliases usable "in cfgs anywhere a text id is used".
//!
//! Source: `src/config_translation.c`, and the header of the shipped `fxdata/translation.toml`.
//!
//! # File
//! Top-level tables, one per alias (`[BEHOLDER_NAME]`), each with one string per language code
//! (`ENG = "Beholder"`). A key spelled `_XXX` marks a machine translation ("fuzzy"): it is used
//! like `XXX` when `XXX` is absent (`get_language_value`, src/config_translation.c:51-65). Aliases must
//! be unique, they are at most 63 bytes (`MAX_ALIAS_LEN` 64, :32). Comments in the file are not
//! data.
//!
//! # Which text an alias gets (`translation_section_visitor`, :107-133)
//! The current language (`XXX`, then `_XXX`); Traditional Chinese (`CHT`) falls back to `CHI`;
//! then `ENG`; then the alias text itself.
//!
//! # Layering (`load_config`, src/config.c:2326-2370; the file header)
//! `fxdata/translation.toml` first; more files may sit in the `CONFIGS_LOCATION` folder of
//! campaigns, maps and mods, or in `LEVELS_LOCATION` named `map0####.translation.toml`; mods
//! contribute theirs at their place of the load order. A later file replaces the text of an alias
//! that already exists and appends new aliases. Inside one file aliases are visited in sorted order
//! (`value_dict_walk_sorted`), and an alias's id is its position in the table:
//! `TRANSLATION_STRINGS_START + index` (:158-175). The table holds at most 2000 entries; when it is
//! full the rest of the current file is skipped (:98-99). A numeric alias is a plain string id up
//! to `STRINGS_MAX` (:155).
//!
//! The layered files are given as paths in load order (this crate does not know the mod layers;
//! `kfx_map::mods::LayerStack::files("translation.toml")` lists them).
use crate::strings::{MAX_TOML_TRANSLATION_ENTRIES, STRINGS_MAX, TRANSLATION_STRINGS_START};
use crate::{rnc, Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `MAX_ALIAS_LEN` (src/config_translation.c:32): aliases are cut to 63 bytes.
pub const MAX_ALIAS_LEN: usize = 64;

/// One language's text of an alias.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LangText {
    pub text: String,
    /// Written as `_XXX` (machine translated).
    pub fuzzy: bool,
}

/// One `[ALIAS]` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranslationEntry {
    pub alias: String,
    /// Upper-case language code (`ENG`) to text. When a file has both `XXX` and `_XXX` the plain
    /// one wins in the game, so only it is kept.
    pub texts: BTreeMap<String, LangText>,
}

impl TranslationEntry {
    fn value(&self, code: &str) -> Option<&str> {
        self.texts.get(code).map(|t| t.text.as_str())
    }

    /// The text for language `code` with the game's fallbacks, `None` when even English is
    /// missing (the game then shows the alias).
    pub fn text_for(&self, code: &str) -> Option<&str> {
        let code = code.to_ascii_uppercase();
        self.value(&code)
            .or_else(|| (code == "CHT").then(|| self.value("CHI")).flatten())
            .or_else(|| self.value("ENG"))
    }

    /// The text the game shows for `code`: [`TranslationEntry::text_for`], else the alias.
    pub fn resolve(&self, code: &str) -> &str {
        self.text_for(code).unwrap_or(&self.alias)
    }

    /// Whether the text shown for `code` is a fuzzy one or a fallback to another language.
    pub fn fallback_of(&self, code: &str) -> Option<&'static str> {
        let code = code.to_ascii_uppercase();
        if self.texts.contains_key(&code) {
            return None;
        }
        if code == "CHT" && self.texts.contains_key("CHI") {
            return Some("CHI");
        }
        if self.texts.contains_key("ENG") {
            return Some("ENG");
        }
        Some("alias")
    }
}

/// A parsed `translation.toml` (aliases in sorted order, like the game visits them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TranslationFile {
    pub entries: Vec<TranslationEntry>,
}

impl TranslationFile {
    pub fn parse(text: &str, path: &Path) -> Result<TranslationFile> {
        let t: toml::Table = text.parse().map_err(|e: toml::de::Error| {
            Error::format("translation", format!("{}: {e}", path.display()))
        })?;
        let mut entries = vec![];
        for (alias, v) in t {
            let toml::Value::Table(tab) = v else { continue }; // non-table entries are skipped
            let mut texts = BTreeMap::new();
            // plain codes first so they win over `_XXX`
            for (k, v) in &tab {
                if let (toml::Value::String(s), false) = (v, k.starts_with('_')) {
                    texts.insert(
                        k.clone(),
                        LangText {
                            text: s.clone(),
                            fuzzy: false,
                        },
                    );
                }
            }
            for (k, v) in &tab {
                if let (toml::Value::String(s), Some(code)) = (v, k.strip_prefix('_')) {
                    texts.entry(code.to_string()).or_insert(LangText {
                        text: s.clone(),
                        fuzzy: true,
                    });
                }
            }
            entries.push(TranslationEntry { alias, texts });
        }
        Ok(TranslationFile { entries })
    }

    pub fn load(path: &Path) -> Result<TranslationFile> {
        let b = rnc::load_file(path)?;
        Self::parse(&String::from_utf8_lossy(&b), path)
    }

    pub fn get(&self, alias: &str) -> Option<&TranslationEntry> {
        self.entries.iter().find(|e| e.alias == alias)
    }

    /// Set the text of `alias` in language `code`, creating the entry (kept in sorted order).
    pub fn set(&mut self, alias: &str, code: &str, text: &str, fuzzy: bool) {
        let code = code.to_ascii_uppercase();
        let i = match self.entries.iter().position(|e| e.alias == alias) {
            Some(i) => i,
            None => {
                let at = self.entries.partition_point(|e| e.alias.as_str() < alias);
                self.entries.insert(
                    at,
                    TranslationEntry {
                        alias: alias.to_string(),
                        texts: BTreeMap::new(),
                    },
                );
                at
            }
        };
        self.entries[i].texts.insert(
            code,
            LangText {
                text: text.to_string(),
                fuzzy,
            },
        );
    }

    pub fn remove(&mut self, alias: &str) -> bool {
        let n = self.entries.len();
        self.entries.retain(|e| e.alias != alias);
        n != self.entries.len()
    }

    /// Canonical TOML text (comments of an original file are not kept).
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            // `crate::toml_emit`: only forms the game's TOML reader takes as written
            out.push_str(&format!("[{}]\n", crate::toml_emit::key(&e.alias)));
            for (code, t) in &e.texts {
                let k = if t.fuzzy {
                    format!("_{code}")
                } else {
                    code.clone()
                };
                out.push_str(&format!(
                    "{} = {}\n",
                    crate::toml_emit::key(&k),
                    crate::toml_emit::string(&t.text)
                ));
            }
            out.push('\n');
        }
        out
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::atomic::atomic_write(path, self.to_toml().as_bytes()).map_err(|e| Error::io(path, e))
    }
}

/// One alias of the layered table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableEntry {
    pub alias: String,
    /// The text for the language the table was built for.
    pub text: String,
    /// Index into [`TranslationTable::files`] of the file that set the effective text.
    pub file: usize,
    /// Files that defined this alias before, oldest first (overridden).
    pub overridden_from: Vec<usize>,
}

/// The game's translation table for one language.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TranslationTable {
    pub lang_code: String,
    pub files: Vec<PathBuf>,
    pub entries: Vec<TableEntry>,
}

impl TranslationTable {
    /// Build the table from `files` in layer order for language `lang_code` (`ENG`, ...). Files
    /// that do not exist or are shorter than 4 bytes are skipped (`MIN_CONFIG_FILE_SIZE`); a file
    /// that does not parse is an error.
    pub fn load(files: &[PathBuf], lang_code: &str) -> Result<TranslationTable> {
        let mut t = TranslationTable {
            lang_code: lang_code.to_ascii_uppercase(),
            ..Default::default()
        };
        for p in files {
            let Ok(b) = rnc::load_file(p) else { continue };
            if b.len() < 4 {
                continue;
            }
            let f = TranslationFile::parse(&String::from_utf8_lossy(&b), p)?;
            t.apply(&f, p.clone());
        }
        Ok(t)
    }

    /// Apply one more file on top (what the game does for every layer).
    pub fn apply(&mut self, f: &TranslationFile, path: PathBuf) {
        let file = self.files.len();
        self.files.push(path);
        for e in &f.entries {
            if self.entries.len() >= MAX_TOML_TRANSLATION_ENTRIES {
                break;
            }
            let alias: String = e.alias.chars().take(MAX_ALIAS_LEN - 1).collect();
            let text = e.resolve(&self.lang_code).to_string();
            match self.entries.iter_mut().find(|x| x.alias == alias) {
                Some(x) => {
                    x.overridden_from.push(x.file);
                    x.file = file;
                    x.text = text;
                }
                None => self.entries.push(TableEntry {
                    alias,
                    text,
                    file,
                    overridden_from: vec![],
                }),
            }
        }
    }

    /// `get_string_id_by_alias` (src/config_translation.c:155): a number is a plain id up to
    /// `STRINGS_MAX`, an alias is `TRANSLATION_STRINGS_START + index`.
    pub fn id_of_alias(&self, alias: &str) -> Option<usize> {
        if let Ok(n) = alias.trim().parse::<i64>() {
            return (0..=STRINGS_MAX as i64).contains(&n).then_some(n as usize);
        }
        self.entries
            .iter()
            .position(|e| e.alias == alias)
            .map(|i| TRANSLATION_STRINGS_START + i)
    }

    /// `get_translation_file_string` (src/config_translation.c:172) for a string id.
    pub fn get(&self, id: usize) -> &str {
        match id
            .checked_sub(TRANSLATION_STRINGS_START)
            .and_then(|i| self.entries.get(i))
        {
            Some(e) => &e.text,
            None => "oh_crap_invalid_string_id",
        }
    }

    pub fn entry(&self, alias: &str) -> Option<&TableEntry> {
        self.entries.iter().find(|e| e.alias == alias)
    }
}
