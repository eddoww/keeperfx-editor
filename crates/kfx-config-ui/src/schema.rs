//! The config schema: every config file's sections and keys with types, ranges, enum tables and
//! docs, extracted from the game's loaders by `tools/extract_config_schema.py` into
//! `data/config_schema.json` (regenerate with `python3 -I tools/extract_config_schema.py`).
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

static BUILTIN_JSON: &str = include_str!("../../../data/config_schema.json");

/// One `NamedCommand` entry.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct EnumItem {
    pub name: String,
    #[serde(default)]
    pub value: Option<i64>,
}

/// Type of one argument of a key.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct ArgSpec {
    #[serde(rename = "type", default)]
    pub ty: String,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub min_symbol: Option<String>,
    #[serde(default)]
    pub max_symbol: Option<String>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    #[serde(rename = "enum", default)]
    pub enum_table: Option<String>,
    #[serde(rename = "ref", default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    /// The argument repeats to the end of the line.
    #[serde(default)]
    pub variadic: bool,
    /// Enum table chosen by the first argument's value (e.g. research `MAGIC` -> `power_desc`).
    #[serde(default)]
    pub enum_by_arg0: BTreeMap<String, String>,
}

impl ArgSpec {
    /// Reference kind of a lookup argument: the explicit `ref`, else the one of its (dynamic)
    /// enum table.
    pub fn ref_kind(&self) -> Option<String> {
        if let Some(r) = &self.reference {
            return Some(r.clone());
        }
        let t = self.enum_table.as_deref()?;
        let r = match t {
            "creature_desc" => "creature",
            "object_desc" => "object",
            "room_desc" => "room",
            "slab_desc" => "slab",
            "spell_desc" => "spell",
            "shot_desc" => "shot",
            "power_desc" => "power",
            "trap_desc" => "trap",
            "door_desc" => "door",
            "instance_desc" => "instance",
            "creaturejob_desc" => "job",
            "angerjob_desc" => "angerjob",
            "attackpref_desc" => "attackpref",
            "creatrstate_desc" => "state",
            "cube_desc" => "cube",
            "lenses_desc" => "lens",
            _ => return None,
        };
        Some(r.to_string())
    }
}

/// Reference kind of an enum table built from config names (`dynamic_enums`).
pub fn table_ref_kind(table: &str) -> Option<String> {
    ArgSpec {
        enum_table: Some(table.to_string()),
        ..Default::default()
    }
    .ref_kind()
}

impl ArgSpec {
    pub fn min_i64(&self) -> Option<i64> {
        self.min.map(|v| v as i64)
    }
    pub fn max_i64(&self) -> Option<i64> {
        self.max.map(|v| v as i64)
    }
}

/// One key of a section.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct KeySpec {
    pub name: String,
    #[serde(default)]
    pub whole_line: bool,
    #[serde(default)]
    pub repeat: bool,
    #[serde(default)]
    pub array: bool,
    #[serde(default)]
    pub args: Vec<ArgSpec>,
    #[serde(default)]
    pub fields: Vec<KeySpec>,
    #[serde(default)]
    pub parse: Option<String>,
    #[serde(default)]
    pub assign: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub doc: Option<String>,
    #[serde(default)]
    pub doc_source: Option<String>,
    #[serde(default)]
    pub confidence: Option<String>,
}

/// One section (or family of numbered sections `[baseN]`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct SectionSpec {
    pub name: String,
    #[serde(default)]
    pub numbered: bool,
    #[serde(default)]
    pub max: Option<i64>,
    #[serde(default)]
    pub max_symbol: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub doc: Option<String>,
    /// List semantics (e.g. `"replace"` when a later layer's block replaces the whole list).
    #[serde(default)]
    pub list: Option<serde_json::Value>,
    #[serde(default)]
    pub keys: Vec<KeySpec>,
}

impl SectionSpec {
    /// Whether a block of this section in an override replaces the whole block (research,
    /// sacrifices: `parse_rules_research_blocks` clears the list, src/config_rules.c:446).
    pub fn replaces_whole_block(&self) -> bool {
        match &self.list {
            Some(serde_json::Value::String(s)) => s.contains("replace"),
            Some(serde_json::Value::Object(o)) => o
                .values()
                .any(|v| v.as_str().is_some_and(|s| s.contains("replace"))),
            _ => false,
        }
    }
    /// Does section name `name` belong to this spec (`[slab12]` for `slab`, numbered)?
    pub fn matches(&self, name: &str) -> bool {
        if self.numbered {
            match name.get(..self.name.len()) {
                Some(p) if p.eq_ignore_ascii_case(&self.name) => {
                    let rest = &name[self.name.len()..];
                    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
                }
                _ => false,
            }
        } else {
            self.name.eq_ignore_ascii_case(name)
        }
    }
    pub fn key(&self, key: &str) -> Option<&KeySpec> {
        self.keys.iter().find(|k| k.name.eq_ignore_ascii_case(key))
    }
}

/// One config file.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct FileSpec {
    pub file: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub doc: Option<String>,
    #[serde(default)]
    pub sections: Vec<SectionSpec>,
}

impl FileSpec {
    /// The spec of section `name` (literal match first, then numbered families).
    pub fn section(&self, name: &str) -> Option<&SectionSpec> {
        self.sections
            .iter()
            .find(|s| !s.numbered && s.matches(name))
            .or_else(|| self.sections.iter().find(|s| s.numbered && s.matches(name)))
    }
    pub fn is_toml(&self) -> bool {
        self.format.eq_ignore_ascii_case("toml")
    }
}

/// Key counts by confidence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub full: usize,
    pub partial: usize,
    pub unknown: usize,
    pub documented: usize,
}

/// The whole schema.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Schema {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub enums: BTreeMap<String, Vec<EnumItem>>,
    #[serde(default)]
    pub files: Vec<FileSpec>,
}

impl Schema {
    pub fn from_json(text: &str) -> Result<Schema, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// The schema checked into `data/config_schema.json` (parsed once).
    pub fn builtin() -> &'static Schema {
        static S: OnceLock<Schema> = OnceLock::new();
        S.get_or_init(|| Schema::from_json(BUILTIN_JSON).unwrap_or_default())
    }

    /// Spec of config file `key` (`rules.cfg`, `creatrs/dragon.cfg` matches `creatrs/*.cfg`).
    pub fn file(&self, key: &str) -> Option<&FileSpec> {
        let key_l = key.to_ascii_lowercase();
        self.files
            .iter()
            .find(|f| f.file.eq_ignore_ascii_case(&key_l))
            .or_else(|| {
                if key_l.starts_with("creatrs/") {
                    self.files
                        .iter()
                        .find(|f| f.file.eq_ignore_ascii_case("creatrs/*.cfg"))
                } else {
                    None
                }
            })
    }

    pub fn enum_table(&self, name: &str) -> Option<&[EnumItem]> {
        self.enums.get(name).map(|v| v.as_slice())
    }

    /// Coverage of one file spec.
    pub fn coverage_of(f: &FileSpec) -> Coverage {
        let mut c = Coverage::default();
        for s in &f.sections {
            for k in &s.keys {
                match k.confidence.as_deref() {
                    Some("full") => c.full += 1,
                    Some("partial") => c.partial += 1,
                    _ => c.unknown += 1,
                }
                if k.doc.as_deref().is_some_and(|d| !d.trim().is_empty()) {
                    c.documented += 1;
                }
            }
        }
        c
    }

    /// Coverage over all files.
    pub fn coverage(&self) -> Coverage {
        let mut t = Coverage::default();
        for f in &self.files {
            let c = Self::coverage_of(f);
            t.full += c.full;
            t.partial += c.partial;
            t.unknown += c.unknown;
            t.documented += c.documented;
        }
        t
    }
}
