//! `slabset.toml` model with the game's merge rules, and the minimal override writer.
//!
//! `load_slabset_config_file` (`src/config_slabsets.c:87-160`), per `[slabN]` and style name:
//! * `slabN.<STYLE>.Columns` (array) sets the 9 column ids (missing entries read as 0);
//! * `slabN.<STYLE>_objects` (array of tables) non-empty replaces the style's objects;
//! * when the `slabN.<STYLE>` table exists and `<STYLE>_objects` is absent or empty, the style's
//!   objects are cleared;
//! * a partial file touches nothing else. The base load (`clear_slabsets`) starts from zero.
//!
//! (The shipped base file also has `Objects = []` inside each style table; the loader never reads
//! that key.)
//!
//! So a minimal override for a style whose columns changed writes `[slabN.STYLE] Columns` plus
//! the full `STYLE_objects` list when it is non-empty; a style whose objects alone changed
//! writes only `STYLE_objects` (or an empty `[slabN.STYLE]` table to clear them).
use crate::cfgedit::Pending;
use crate::tomledit::{parse_table, table_mut, to_edit_table};
use kfx_map::gamedata::{SLABSETS_PER_SLAB, SLAB_STYLE_NAMES};
use std::collections::BTreeMap;
use std::path::PathBuf;
use toml_edit::{ArrayOfTables, DocumentMut, Item};

/// One slabset entry (slab kind x style).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    /// Column ids as written in the file (positive: `columnset.toml` column; zero or negative:
    /// the game stores the negation, `ColumnIndex col_idx = -value`).
    pub columns: [i64; 9],
    /// Object tables (`IsLight`, `Subtile`, `RelativePosition`, `ThingType`, `Subtype`,
    /// `EffectRange`, `Orientation`), kept as written so unknown keys survive.
    pub objects: Vec<toml::Table>,
}

/// Where a style's parts were last set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleSource {
    pub columns: Option<usize>,
    pub objects: Option<usize>,
}

/// Key of a slabset entry: (slab kind, style 0..27).
pub type StyleKey = (usize, usize);

/// Apply one file to the state (`load_slabset_config_file`).
pub fn apply_file(
    state: &mut BTreeMap<StyleKey, (Style, StyleSource)>,
    root: &toml::Table,
    layer: usize,
) {
    for (k, v) in root {
        let Some(kind) = k.strip_prefix("slab").and_then(|n| n.parse::<usize>().ok()) else {
            continue;
        };
        if format!("slab{kind}") != *k {
            continue;
        }
        let Some(slb) = v.as_table() else { continue };
        for (style, sname) in SLAB_STYLE_NAMES.iter().enumerate() {
            let section = slb.get(*sname);
            let objs = slb
                .get(&format!("{sname}_objects"))
                .and_then(|o| o.as_array());
            let cols = section
                .and_then(|s| s.get("Columns"))
                .and_then(|c| c.as_array());
            let nobj = objs.map_or(0, |a| a.len());
            let touches = cols.is_some() || nobj > 0 || section.is_some_and(|s| s.is_table());
            if !touches {
                continue;
            }
            let e = state.entry((kind, style)).or_default();
            if let Some(cols) = cols {
                for i in 0..9 {
                    e.0.columns[i] = cols
                        .get(i)
                        .and_then(|x| x.as_integer())
                        .map_or(0, |x| x as i32 as i64);
                }
                e.1.columns = Some(layer);
            }
            if section.is_some_and(|s| s.is_table()) && nobj == 0 {
                e.0.objects.clear();
                e.1.objects = Some(layer);
            } else if let Some(arr) = objs.filter(|a| !a.is_empty()) {
                e.0.objects = arr
                    .iter()
                    .map(|o| o.as_table().cloned().unwrap_or_default())
                    .collect();
                e.1.objects = Some(layer);
            }
        }
    }
}

/// The slabset over the layers, edited at a target layer.
#[derive(Clone, Debug)]
pub struct SlabsetModel {
    pub target_layer: usize,
    pub target_path: Option<PathBuf>,
    /// State from the layers below the target.
    pub inherited: BTreeMap<StyleKey, (Style, StyleSource)>,
    /// State as of the target layer, including unsaved edits.
    pub current: BTreeMap<StyleKey, (Style, StyleSource)>,
    /// Upper layers (they override the target), raw.
    pub upper: Vec<(usize, toml::Table)>,
    pub target: DocumentMut,
    pub original: Option<Vec<u8>>,
    pub error: Option<String>,
}

impl SlabsetModel {
    /// `layers`: `(layer, table)` of every layer that has the file, in load order (the target's
    /// own file excluded; it is given as `original`).
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
        SlabsetModel {
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

    /// The style as of the target layer.
    pub fn style(&self, kind: usize, style: usize) -> Style {
        self.current
            .get(&(kind, style))
            .map(|e| e.0.clone())
            .unwrap_or_default()
    }
    pub fn source(&self, kind: usize, style: usize) -> StyleSource {
        self.current
            .get(&(kind, style))
            .map(|e| e.1)
            .unwrap_or_default()
    }
    pub fn inherited_style(&self, kind: usize, style: usize) -> Style {
        self.inherited
            .get(&(kind, style))
            .map(|e| e.0.clone())
            .unwrap_or_default()
    }
    /// Whether the target layer changes this style.
    pub fn is_overridden(&self, kind: usize, style: usize) -> bool {
        self.style(kind, style) != self.inherited_style(kind, style)
    }
    /// The style the game ends up with (upper layers applied).
    pub fn effective(&self) -> BTreeMap<StyleKey, (Style, StyleSource)> {
        let mut s = self.current.clone();
        for (l, t) in &self.upper {
            apply_file(&mut s, t, *l);
        }
        s
    }
    /// Highest slab kind present + 1.
    pub fn kind_count(&self) -> usize {
        self.current.keys().map(|k| k.0 + 1).max().unwrap_or(0)
    }

    /// Replace a style and rewrite the target document.
    pub fn set_style(&mut self, kind: usize, style: usize, s: Style) {
        let e = self.current.entry((kind, style)).or_default();
        if e.0.columns != s.columns {
            e.1.columns = Some(self.target_layer);
        }
        if e.0.objects != s.objects {
            e.1.objects = Some(self.target_layer);
        }
        e.0 = s;
        self.sync_key(kind, style);
    }

    /// Back to the inherited style.
    pub fn revert_style(&mut self, kind: usize, style: usize) {
        match self.inherited.get(&(kind, style)) {
            Some(e) => {
                self.current.insert((kind, style), e.clone());
            }
            None => {
                self.current.remove(&(kind, style));
            }
        }
        self.sync_key(kind, style);
    }

    /// Bring the target document in line with `current` for one style (minimal difference).
    fn sync_key(&mut self, kind: usize, style: usize) {
        if self.error.is_some() {
            return;
        }
        let want = self.style(kind, style);
        let inh = self.inherited_style(kind, style);
        let slab = format!("slab{kind}");
        let sname = SLAB_STYLE_NAMES[style].to_string();
        let okey = format!("{sname}_objects");
        let cols_diff = want.columns != inh.columns;
        let objs_diff = want.objects != inh.objects;
        let (want_table, want_cols, want_objs) = if !cols_diff && !objs_diff {
            (false, false, false)
        } else if cols_diff {
            (true, true, !want.objects.is_empty())
        } else if want.objects.is_empty() {
            (true, false, false)
        } else {
            (false, false, true)
        };
        // style table
        if want_table {
            let t = table_mut(&mut self.target, &[slab.clone(), sname.clone()]).expect("table");
            if want_cols {
                let mut arr = toml_edit::Array::new();
                for c in want.columns {
                    arr.push(c);
                }
                match t.get_mut("Columns") {
                    Some(Item::Value(v)) => {
                        let d = v.decor().clone();
                        *v = toml_edit::Value::Array(arr);
                        *v.decor_mut() = d;
                    }
                    _ => {
                        t.insert("Columns", Item::Value(toml_edit::Value::Array(arr)));
                    }
                }
            } else {
                t.remove("Columns");
            }
        } else if let Some(Item::Table(st)) = self.target.as_table_mut().get_mut(&slab) {
            st.remove(&sname);
        }
        // objects
        if want_objs {
            let mut aot = ArrayOfTables::new();
            for o in &want.objects {
                aot.push(to_edit_table(o));
            }
            let t = table_mut(&mut self.target, std::slice::from_ref(&slab)).expect("table");
            t.insert(&okey, Item::ArrayOfTables(aot));
        } else if let Some(Item::Table(st)) = self.target.as_table_mut().get_mut(&slab) {
            st.remove(&okey);
        }
        // drop an empty slab table
        let root = self.target.as_table_mut();
        if root
            .get(&slab)
            .and_then(|i| i.as_table())
            .is_some_and(|t| t.is_empty())
        {
            root.remove(&slab);
        }
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

    /// Every style the target layer changes.
    pub fn overridden_keys(&self) -> Vec<StyleKey> {
        let mut keys: Vec<StyleKey> = self
            .current
            .keys()
            .chain(self.inherited.keys())
            .copied()
            .collect();
        keys.sort();
        keys.dedup();
        keys.into_iter()
            .filter(|&(k, s)| self.is_overridden(k, s))
            .collect()
    }
}

// ------------------------------------------------------------------------- style operations

/// Subtile index (0..8, row-major, x right / y down) after rotating the 3x3 grid 90 degrees
/// clockwise (seen from above).
pub fn rotate_cw_index(i: usize) -> usize {
    let (x, y) = (i % 3, i / 3);
    let (nx, ny) = (2 - y, x);
    ny * 3 + nx
}

/// Mirror left/right.
pub fn mirror_x_index(i: usize) -> usize {
    let (x, y) = (i % 3, i / 3);
    y * 3 + (2 - x)
}

fn int(t: &toml::Table, k: &str) -> Option<i64> {
    match t.get(k)? {
        toml::Value::Integer(i) => Some(*i),
        toml::Value::Boolean(b) => Some(*b as i64),
        _ => None,
    }
}

/// Map objects through a subtile permutation and transform their relative position inside the
/// subtile (256 units per subtile; positions are relative to the subtile's corner).
fn map_objects(
    objs: &[toml::Table],
    idx: fn(usize) -> usize,
    pos: fn(i64, i64) -> (i64, i64),
) -> Vec<toml::Table> {
    objs.iter()
        .map(|o| {
            let mut o = o.clone();
            if let Some(s) = int(&o, "Subtile") {
                if (0..9).contains(&s) {
                    o.insert(
                        "Subtile".into(),
                        toml::Value::Integer(idx(s as usize) as i64),
                    );
                }
            }
            if let Some(toml::Value::Array(a)) = o.get("RelativePosition").cloned() {
                let x = a.first().and_then(|v| v.as_integer()).unwrap_or(0);
                let y = a.get(1).and_then(|v| v.as_integer()).unwrap_or(0);
                let (nx, ny) = pos(x, y);
                let mut na = a.clone();
                if na.len() >= 2 {
                    na[0] = toml::Value::Integer(nx);
                    na[1] = toml::Value::Integer(ny);
                }
                o.insert("RelativePosition".into(), toml::Value::Array(na));
            }
            o
        })
        .collect()
}

impl Style {
    /// Rotate 90 degrees clockwise (columns and object subtiles / positions).
    pub fn rotated_cw(&self) -> Style {
        let mut c = [0; 9];
        for (i, v) in self.columns.iter().enumerate() {
            c[rotate_cw_index(i)] = *v;
        }
        Style {
            columns: c,
            objects: map_objects(&self.objects, rotate_cw_index, |x, y| (256 - y, x)),
        }
    }
    /// Mirror left/right.
    pub fn mirrored_x(&self) -> Style {
        let mut c = [0; 9];
        for (i, v) in self.columns.iter().enumerate() {
            c[mirror_x_index(i)] = *v;
        }
        Style {
            columns: c,
            objects: map_objects(&self.objects, mirror_x_index, |x, y| (256 - x, y)),
        }
    }
}

/// Number of styles per slab kind.
pub const STYLES: usize = SLABSETS_PER_SLAB;

/// A slab object's typed fields for display (`struct SlabObj`, src/slab_data.h:125).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjView {
    pub is_light: bool,
    pub subtile: i64,
    pub rel: [i64; 3],
    pub thing_type: String,
    pub subtype: String,
    pub effect_range: i64,
    pub orientation: i64,
}

fn scalar_text(v: Option<&toml::Value>) -> String {
    match v {
        Some(toml::Value::String(s)) => s.clone(),
        Some(toml::Value::Integer(i)) => i.to_string(),
        Some(toml::Value::Boolean(b)) => (*b as i64).to_string(),
        Some(o) => o.to_string(),
        None => String::new(),
    }
}

/// Text to a TOML scalar: integers stay integers, anything else becomes a string.
pub fn scalar_value(s: &str) -> toml::Value {
    match s.trim().parse::<i64>() {
        Ok(i) => toml::Value::Integer(i),
        Err(_) => toml::Value::String(s.trim().to_string()),
    }
}

impl ObjView {
    pub fn of(t: &toml::Table) -> ObjView {
        let rel = t
            .get("RelativePosition")
            .and_then(|v| v.as_array())
            .map_or([0; 3], |a| {
                [0, 1, 2].map(|i| a.get(i).and_then(|v| v.as_integer()).unwrap_or(0))
            });
        ObjView {
            is_light: int(t, "IsLight").unwrap_or(0) != 0,
            subtile: int(t, "Subtile").unwrap_or(0),
            rel,
            thing_type: scalar_text(t.get("ThingType")),
            subtype: scalar_text(t.get("Subtype")),
            effect_range: int(t, "EffectRange").unwrap_or(0),
            orientation: int(t, "Orientation").unwrap_or(0),
        }
    }
    /// Write back into `t`, keeping keys the view does not know and the original
    /// representation of unchanged values.
    pub fn write(&self, t: &mut toml::Table) {
        let old = ObjView::of(t);
        if old.is_light != self.is_light || !t.contains_key("IsLight") {
            t.insert("IsLight".into(), toml::Value::Integer(self.is_light as i64));
        }
        if old.subtile != self.subtile || !t.contains_key("Subtile") {
            t.insert("Subtile".into(), toml::Value::Integer(self.subtile));
        }
        if old.rel != self.rel || !t.contains_key("RelativePosition") {
            t.insert(
                "RelativePosition".into(),
                toml::Value::Array(self.rel.iter().map(|v| toml::Value::Integer(*v)).collect()),
            );
        }
        if old.thing_type != self.thing_type || !t.contains_key("ThingType") {
            t.insert("ThingType".into(), scalar_value(&self.thing_type));
        }
        if old.subtype != self.subtype || !t.contains_key("Subtype") {
            t.insert("Subtype".into(), scalar_value(&self.subtype));
        }
        if old.effect_range != self.effect_range || !t.contains_key("EffectRange") {
            t.insert(
                "EffectRange".into(),
                toml::Value::Integer(self.effect_range),
            );
        }
        if old.orientation != self.orientation {
            t.insert("Orientation".into(), toml::Value::Integer(self.orientation));
        }
    }
}
