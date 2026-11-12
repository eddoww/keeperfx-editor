//! `columnset.toml` model and minimal override writer.
//!
//! `load_columns_config_file` (`src/config_slabsets.c:166-233`): for every `[columnN]` of a file
//! the bitfields restart at "permanent" only, then `Lintel` and `Height` are OR-ed in when
//! present; `SolidMask`, `FloorTexture`, `Orientation`, `Cubes` change only when present. A
//! partial override of a column that omits `Lintel`/`Height` therefore resets them to 0, so the
//! writer always writes a changed column whole (all six keys).
//!
//! The game recomputes solid mask and heights when it creates a slabset's columns
//! (`create_columns_from_list` -> `create_column`, `src/map_columns.c:293`), but finds the
//! created column again with the columnset entry *as loaded* (`update_slabset_column_indices`),
//! including `SolidMask`: a `SolidMask` that disagrees with the cubes leaves the slabset entry
//! unresolved. [`Col::auto`] gives the values `create_column` computes.
use crate::cfgedit::Pending;
use crate::tomledit::{parse_table, table_mut};
use kfx_map::column::{create_column_bitfields, solidmask_of};
use std::collections::BTreeMap;
use std::path::PathBuf;
use toml_edit::{DocumentMut, Item};

/// One `[columnN]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Col {
    pub lintel: i64,
    pub height: i64,
    pub solid_mask: i64,
    pub floor_texture: i64,
    pub orientation: i64,
    pub cubes: [i64; 8],
}

/// Values `create_column` derives from the cubes (src/map_columns.c:293-358).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoValues {
    pub height: i64,
    pub lintel: i64,
    pub solid_mask: i64,
}

impl Col {
    pub fn cubes_u16(&self) -> [u16; 8] {
        self.cubes.map(|c| c.clamp(0, u16::MAX as i64) as u16)
    }
    pub fn auto(&self) -> AutoValues {
        let cubes = self.cubes_u16();
        let bf = create_column_bitfields(&cubes, 1);
        AutoValues {
            height: ((bf >> 4) & 0x0F) as i64,
            lintel: ((bf >> 1) & 0x07) as i64,
            solid_mask: solidmask_of(&cubes) as i64,
        }
    }
    /// Same content for the game's `column_is_equivalent` (cubes, floor, orientation, mask).
    pub fn equivalent(&self, o: &Col) -> bool {
        self.cubes == o.cubes
            && self.floor_texture == o.floor_texture
            && self.orientation == o.orientation
            && self.solid_mask == o.solid_mask
    }
    pub fn to_table(&self) -> toml::Table {
        let mut t = toml::Table::new();
        t.insert("Lintel".into(), self.lintel.into());
        t.insert("Height".into(), self.height.into());
        t.insert("SolidMask".into(), self.solid_mask.into());
        t.insert("FloorTexture".into(), self.floor_texture.into());
        t.insert("Orientation".into(), self.orientation.into());
        t.insert(
            "Cubes".into(),
            toml::Value::Array(
                self.cubes
                    .iter()
                    .map(|c| toml::Value::Integer(*c))
                    .collect(),
            ),
        );
        t
    }
    /// A column as the editor builds it: auto solid mask / height / lintel.
    pub fn with_auto(mut self) -> Col {
        let a = self.auto();
        self.solid_mask = a.solid_mask;
        self.height = a.height;
        self.lintel = a.lintel;
        self
    }
}

/// Apply one file (`load_columns_config_file`).
pub fn apply_file(state: &mut BTreeMap<usize, (Col, usize)>, root: &toml::Table, layer: usize) {
    for (k, v) in root {
        let Some(n) = k
            .strip_prefix("column")
            .and_then(|s| s.parse::<usize>().ok())
        else {
            continue;
        };
        if format!("column{n}") != *k || n >= kfx_map::gamedata::COLUMNS_COUNT {
            continue;
        }
        let Some(t) = v.as_table() else { continue };
        let e = state.entry(n).or_default();
        let g = |key: &str| t.get(key).and_then(|x| x.as_integer());
        e.0.lintel = g("Lintel").unwrap_or(0);
        e.0.height = g("Height").unwrap_or(0);
        if let Some(x) = g("SolidMask") {
            e.0.solid_mask = x;
        }
        if let Some(x) = g("FloorTexture") {
            e.0.floor_texture = x;
        }
        if let Some(x) = g("Orientation") {
            e.0.orientation = x;
        }
        if let Some(a) = t.get("Cubes").and_then(|x| x.as_array()) {
            for i in 0..8 {
                e.0.cubes[i] = a
                    .get(i)
                    .and_then(|x| x.as_integer())
                    .map_or(0, |x| x as i32 as i64);
            }
        }
        e.1 = layer;
    }
}

/// The columnset over the layers, edited at a target layer.
#[derive(Clone, Debug)]
pub struct ColumnsetModel {
    pub target_layer: usize,
    pub target_path: Option<PathBuf>,
    pub inherited: BTreeMap<usize, (Col, usize)>,
    pub current: BTreeMap<usize, (Col, usize)>,
    pub upper: Vec<(usize, toml::Table)>,
    pub target: DocumentMut,
    pub original: Option<Vec<u8>>,
    pub error: Option<String>,
}

impl ColumnsetModel {
    pub fn new(
        layers: Vec<(usize, toml::Table)>,
        target_layer: usize,
        target_path: Option<PathBuf>,
        original: Option<Vec<u8>>,
    ) -> Self {
        let mut inherited = BTreeMap::new();
        let mut upper = vec![];
        for (layer, t) in layers {
            if layer < target_layer {
                apply_file(&mut inherited, &t, layer);
            } else if layer > target_layer {
                upper.push((layer, t));
            }
        }
        let text = original
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        let (target, error) = match text.parse::<DocumentMut>() {
            Ok(d) => (d, None),
            Err(e) => (DocumentMut::new(), Some(e.to_string())),
        };
        let mut current = inherited.clone();
        if let Ok(t) = parse_table(&target.to_string()) {
            apply_file(&mut current, &t, target_layer);
        }
        ColumnsetModel {
            target_layer,
            target_path,
            inherited,
            current,
            upper,
            target,
            original,
            error,
        }
    }

    pub fn count(&self) -> usize {
        self.current.keys().next_back().map_or(0, |k| k + 1)
    }
    pub fn col(&self, n: usize) -> Col {
        self.current.get(&n).map(|e| e.0).unwrap_or_default()
    }
    pub fn source(&self, n: usize) -> Option<usize> {
        self.current.get(&n).map(|e| e.1)
    }
    pub fn inherited_col(&self, n: usize) -> Option<Col> {
        self.inherited.get(&n).map(|e| e.0)
    }
    pub fn is_overridden(&self, n: usize) -> bool {
        self.current.get(&n).map(|e| e.0) != self.inherited.get(&n).map(|e| e.0)
    }

    pub fn set_col(&mut self, n: usize, c: Col) {
        self.current.insert(n, (c, self.target_layer));
        self.sync(n);
    }

    pub fn revert_col(&mut self, n: usize) {
        match self.inherited.get(&n) {
            Some(e) => {
                self.current.insert(n, *e);
            }
            None => {
                self.current.remove(&n);
            }
        }
        self.sync(n);
    }

    fn sync(&mut self, n: usize) {
        if self.error.is_some() {
            return;
        }
        let key = format!("column{n}");
        if !self.is_overridden(n) {
            self.target.as_table_mut().remove(&key);
            return;
        }
        let c = self.col(n);
        let t = table_mut(&mut self.target, std::slice::from_ref(&key)).expect("table");
        for (k, v) in c.to_table() {
            let nv = crate::tomledit::to_edit_value(&v);
            match t.get_mut(&k) {
                Some(Item::Value(old)) => {
                    let d = old.decor().clone();
                    *old = nv;
                    *old.decor_mut() = d;
                }
                _ => {
                    t.insert(&k, Item::Value(nv));
                }
            }
        }
    }

    /// Ids of columns whose content equals `c` (find identical).
    pub fn find_identical(&self, c: &Col) -> Vec<usize> {
        self.current
            .iter()
            .filter(|(_, e)| e.0.equivalent(c))
            .map(|(k, _)| *k)
            .collect()
    }

    /// First id that is unused (all-zero column absent from the slabset) after column 0.
    pub fn first_free(&self, usage: &BTreeMap<usize, usize>) -> usize {
        (1..kfx_map::gamedata::COLUMNS_COUNT)
            .find(|n| {
                usage.get(n).copied().unwrap_or(0) == 0
                    && self.current.get(n).is_none_or(|e| e.0 == Col::default())
            })
            .unwrap_or(self.count())
    }

    /// Drop the target layer's overrides of columns no slabset entry uses.
    pub fn clear_unused_overrides(&mut self, usage: &BTreeMap<usize, usize>) -> Vec<usize> {
        let ids: Vec<usize> = self
            .current
            .keys()
            .copied()
            .filter(|n| self.is_overridden(*n) && usage.get(n).copied().unwrap_or(0) == 0)
            .collect();
        for n in &ids {
            self.revert_col(*n);
        }
        ids
    }

    pub fn pending(&self) -> Pending {
        crate::tomledit::pending_of(&self.target, &self.original)
    }

    pub fn mark_saved(&mut self) {
        let text = self.target.to_string();
        self.original = if self.target.as_table().is_empty() && !text.contains('#') {
            None
        } else {
            Some(text.into_bytes())
        };
    }
}

/// Use count of every columnset id over a slabset (positive column ids).
pub fn usage_counts(
    slabset: &BTreeMap<
        crate::slabset::StyleKey,
        (crate::slabset::Style, crate::slabset::StyleSource),
    >,
) -> BTreeMap<usize, usize> {
    let mut out = BTreeMap::new();
    for (s, _) in slabset.values() {
        for &c in &s.columns {
            if c > 0 {
                *out.entry(c as usize).or_insert(0) += 1;
            }
        }
    }
    out
}
