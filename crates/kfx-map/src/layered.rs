//! Layered configuration with provenance: for every `section` / `key` the effective value, the
//! layer it came from and the values it replaced. This is what a config editor needs to show
//! "overridden by mod X / campaign / this map".
//!
//! Layering rule of the game (`load_config`, src/config.c:2326): files are applied in layer order, a
//! later file assigning a key replaces the earlier value (inside one file the last assignment of a
//! key wins too). Sections are matched case-insensitively. Only the value text is tracked; a
//! numbered section such as `[slab3]` is a section like any other.
use crate::gamedata::GenericCfg;
use crate::mods::LayerOrigin;
use crate::{Error, Result};
use kfx_formats::cfg::CfgDoc;
use kfx_formats::rnc;
use std::path::PathBuf;

/// `MIN_CONFIG_FILE_SIZE`: shorter files are not read by the game.
const MIN_CONFIG_FILE_SIZE: usize = 4;

/// One layer that contributed a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerSource {
    pub origin: LayerOrigin,
    pub path: PathBuf,
}

/// The effective value of a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayeredValue {
    pub value: String,
    /// Index into [`LayeredCfg::layers`] of the layer that set the effective value.
    pub layer: usize,
    /// Earlier values that were replaced, oldest first, with the layer that set them.
    pub overridden: Vec<(usize, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayeredSection {
    pub name: String,
    /// In order of first appearance.
    pub entries: Vec<(String, LayeredValue)>,
}

impl LayeredSection {
    pub fn get(&self, key: &str) -> Option<&LayeredValue> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }
}

/// A config file loaded through its layers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayeredCfg {
    /// The layers that had the file, in load order.
    pub layers: Vec<LayerSource>,
    pub sections: Vec<LayeredSection>,
}

impl LayeredCfg {
    fn set(&mut self, section: &str, key: &str, value: String, layer: usize) {
        let si = match self
            .sections
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(section))
        {
            Some(i) => i,
            None => {
                self.sections.push(LayeredSection {
                    name: section.to_string(),
                    entries: vec![],
                });
                self.sections.len() - 1
            }
        };
        let sec = &mut self.sections[si];
        match sec
            .entries
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
        {
            Some((_, v)) => {
                let old = std::mem::replace(&mut v.value, value);
                let old_layer = std::mem::replace(&mut v.layer, layer);
                v.overridden.push((old_layer, old));
            }
            None => sec.entries.push((
                key.to_string(),
                LayeredValue {
                    value,
                    layer,
                    overridden: vec![],
                },
            )),
        }
    }

    /// Load `.cfg` files given as `(origin, path)` in load order.
    pub fn from_cfg_files(files: Vec<(LayerOrigin, PathBuf)>) -> Result<LayeredCfg> {
        let mut out = LayeredCfg::default();
        for (origin, path) in files {
            let b = rnc::load_file(&path)?;
            if b.len() < MIN_CONFIG_FILE_SIZE {
                continue;
            }
            let doc = CfgDoc::from_bytes(&b);
            let layer = out.layers.len();
            out.layers.push(LayerSource { origin, path });
            let mut seen: Vec<String> = vec![];
            for name in doc.section_names() {
                if seen.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
                    continue;
                }
                seen.push(name.clone());
                for e in doc.entries_all_blocks(&name) {
                    out.set(&name, &e.key, e.value.clone(), layer);
                }
            }
        }
        Ok(out)
    }

    /// Load `.toml` files (`slabset.toml`, `effects.toml`, ...): every leaf value becomes a key
    /// of the section named by the dotted path of its parent tables (top level: `""`); arrays are
    /// one value. Later files replace leaves; tables merge.
    pub fn from_toml_files(files: Vec<(LayerOrigin, PathBuf)>) -> Result<LayeredCfg> {
        fn walk(out: &mut LayeredCfg, path: &str, t: &toml::Table, layer: usize) {
            for (k, v) in t {
                match v {
                    toml::Value::Table(sub) => {
                        let p = if path.is_empty() {
                            k.clone()
                        } else {
                            format!("{path}.{k}")
                        };
                        walk(out, &p, sub, layer);
                    }
                    other => out.set(path, k, other.to_string(), layer),
                }
            }
        }
        let mut out = LayeredCfg::default();
        for (origin, path) in files {
            let b = rnc::load_file(&path)?;
            if b.len() < MIN_CONFIG_FILE_SIZE {
                continue;
            }
            let t: toml::Table =
                String::from_utf8_lossy(&b)
                    .parse()
                    .map_err(|e: toml::de::Error| Error::Toml {
                        path: path.clone(),
                        msg: e.to_string(),
                    })?;
            let layer = out.layers.len();
            out.layers.push(LayerSource { origin, path });
            walk(&mut out, "", &t, layer);
        }
        Ok(out)
    }

    pub fn section(&self, name: &str) -> Option<&LayeredSection> {
        self.sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }
    pub fn get(&self, section: &str, key: &str) -> Option<&LayeredValue> {
        self.section(section)?.get(key)
    }
    /// The effective value text.
    pub fn value(&self, section: &str, key: &str) -> Option<&str> {
        self.get(section, key).map(|v| v.value.as_str())
    }
    /// The layer the effective value came from.
    pub fn origin(&self, section: &str, key: &str) -> Option<&LayerOrigin> {
        self.get(section, key).map(|v| &self.layers[v.layer].origin)
    }
    /// Whether a later layer replaced a value of an earlier one.
    pub fn is_overridden(&self, section: &str, key: &str) -> bool {
        self.get(section, key)
            .is_some_and(|v| !v.overridden.is_empty())
    }
    /// The layers that set `key`, oldest first, with their values (the effective one last).
    pub fn history(&self, section: &str, key: &str) -> Vec<(&LayerOrigin, &str)> {
        let Some(v) = self.get(section, key) else {
            return vec![];
        };
        v.overridden
            .iter()
            .map(|(l, s)| (&self.layers[*l].origin, s.as_str()))
            .chain(std::iter::once((
                &self.layers[v.layer].origin,
                v.value.as_str(),
            )))
            .collect()
    }
    /// The same data without provenance.
    pub fn to_generic(&self) -> GenericCfg {
        GenericCfg {
            sections: self
                .sections
                .iter()
                .map(|s| {
                    (
                        s.name.clone(),
                        crate::gamedata::RawFields(
                            s.entries
                                .iter()
                                .map(|(k, v)| (k.clone(), v.value.clone()))
                                .collect(),
                        ),
                    )
                })
                .collect(),
        }
    }
}
