//! A TOML config file seen through the layers, edited at one target layer.
//!
//! Generic semantics (named-field TOML loaders such as `config_effects.c`, `config_textures.c`,
//! `config_objects.c` for TOML): a later layer replaces the keys it contains, tables merge.
//! Files with other semantics (slabset, columnset) have their own models
//! ([`crate::slabset`], [`crate::columnset`]) that reuse the document helpers here.
//!
//! The target file is a `toml_edit` document, so comments and layout of an existing override
//! survive; new tables are appended in the order they are created; tables whose last key is
//! reverted are removed.
use crate::cfgedit::Pending;
use std::path::PathBuf;
use toml_edit::{DocumentMut, Item, Table};

/// A layer's copy of the file.
#[derive(Clone, Debug)]
pub struct LayerToml {
    pub layer: usize,
    pub path: PathBuf,
    pub table: toml::Table,
}

/// Parse TOML text to a `toml::Table`.
pub fn parse_table(text: &str) -> Result<toml::Table, String> {
    text.parse::<toml::Table>().map_err(|e| e.to_string())
}

/// Convert a `toml::Value` into an editable `toml_edit` item (inline formatting).
pub fn to_edit_value(v: &toml::Value) -> toml_edit::Value {
    match v {
        toml::Value::String(s) => toml_edit::Value::from(s.as_str()),
        toml::Value::Integer(i) => toml_edit::Value::from(*i),
        toml::Value::Float(f) => toml_edit::Value::from(*f),
        toml::Value::Boolean(b) => toml_edit::Value::from(*b),
        toml::Value::Datetime(d) => d
            .to_string()
            .parse()
            .unwrap_or_else(|_| toml_edit::Value::from(d.to_string())),
        toml::Value::Array(a) => {
            let mut arr = toml_edit::Array::new();
            for x in a {
                arr.push(to_edit_value(x));
            }
            toml_edit::Value::Array(arr)
        }
        toml::Value::Table(t) => {
            let mut it = toml_edit::InlineTable::new();
            for (k, x) in t {
                it.insert(k, to_edit_value(x));
            }
            toml_edit::Value::InlineTable(it)
        }
    }
}

/// A `toml::Table` as a `toml_edit` table with `key = value` lines.
pub fn to_edit_table(t: &toml::Table) -> Table {
    let mut out = Table::new();
    for (k, v) in t {
        out.insert(k, Item::Value(to_edit_value(v)));
    }
    out
}

/// Value at `path` in a table.
pub fn get_path<'a>(t: &'a toml::Table, path: &[String]) -> Option<&'a toml::Value> {
    let (last, parents) = path.split_last()?;
    let mut cur = t;
    for p in parents {
        cur = cur.get(p)?.as_table()?;
    }
    cur.get(last)
}

/// Mutable table at `path` of a document, created when missing, in a form the game's TOML
/// reader (`deps/centitoml`, `kfx_formats::game_toml`) accepts: the first two levels get their
/// own headers, parents included (`[slab10]` before `[slab10.S]`: a sub-table header whose
/// parent does not exist yet fails with "out of memory", toml_api.c:702); deeper tables are
/// written as dotted keys inside the second-level section (a third header level walks through
/// the explicit middle table and fails with "duplicate key", toml_api.c:1294 / 683).
pub fn table_mut<'a>(doc: &'a mut DocumentMut, path: &[String]) -> Option<&'a mut Table> {
    let mut cur = doc.as_table_mut();
    for (depth, p) in path.iter().enumerate() {
        if !cur.contains_key(p) {
            let mut t = Table::new();
            if depth >= 2 {
                t.set_dotted(true);
            }
            cur.insert(p, Item::Table(t));
        }
        let item = cur.get_mut(p)?;
        if let Item::Table(t) = item {
            if depth < 2 {
                t.set_implicit(false);
            }
        }
        cur = item.as_table_mut()?;
    }
    Some(cur)
}

/// The text to write for `doc`: as laid out when the game's reader reads it as a TOML reader
/// does, else the same content from the shared writer (`kfx_formats::toml_emit`, which only
/// produces accepted forms; comments are lost then).
pub fn game_text(doc: &DocumentMut) -> String {
    let text = doc.to_string();
    if kfx_formats::game_toml::same_as_toml(&text).is_ok() {
        return text;
    }
    match parse_table(&text) {
        Ok(t) => kfx_formats::toml_emit::document(&t),
        Err(_) => text,
    }
}

/// What saving a target document means: unchanged, delete (empty), or the bytes to write
/// ([`game_text`]).
pub fn pending_of(target: &DocumentMut, original: &Option<Vec<u8>>) -> Pending {
    let text = target.to_string();
    let empty = target.as_table().is_empty() && !text.contains('#');
    match (original, empty) {
        (None, true) => Pending::Unchanged,
        (Some(_), true) => Pending::Delete,
        (Some(o), false) if *o == text.as_bytes() => Pending::Unchanged,
        _ => Pending::Write(game_text(target).into_bytes()),
    }
}

/// Remove `path` from a document, then every table on the way that became empty.
pub fn remove_path(doc: &mut DocumentMut, path: &[String]) -> bool {
    fn rec(t: &mut Table, path: &[String]) -> bool {
        match path {
            [] => false,
            [k] => t.remove(k).is_some(),
            [k, rest @ ..] => {
                let Some(Item::Table(sub)) = t.get_mut(k) else {
                    return false;
                };
                let r = rec(sub, rest);
                if r && sub.is_empty() {
                    t.remove(k);
                }
                r
            }
        }
    }
    rec(doc.as_table_mut(), path)
}

/// One TOML file over the layer stack.
#[derive(Clone, Debug)]
pub struct LayeredToml {
    pub key: String,
    pub lower: Vec<LayerToml>,
    pub upper: Vec<LayerToml>,
    pub target_layer: usize,
    pub target_path: Option<PathBuf>,
    pub target: DocumentMut,
    /// `target` as a plain table (kept in sync after each edit).
    target_table: toml::Table,
    pub original: Option<Vec<u8>>,
    /// Parse error of the target file (the editor then refuses edits).
    pub error: Option<String>,
}

/// Leaf provenance.
#[derive(Clone, Debug, PartialEq)]
pub struct TomlProvenance {
    pub value: toml::Value,
    pub layer: usize,
}

impl LayeredToml {
    pub fn new(
        key: &str,
        layers: Vec<LayerToml>,
        target_layer: usize,
        target_path: Option<PathBuf>,
        original: Option<Vec<u8>>,
    ) -> Self {
        let (mut lower, mut upper) = (vec![], vec![]);
        for l in layers {
            if l.layer < target_layer {
                lower.push(l);
            } else if l.layer > target_layer {
                upper.push(l);
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
        let mut s = LayeredToml {
            key: key.to_string(),
            lower,
            upper,
            target_layer,
            target_path,
            target,
            target_table: toml::Table::new(),
            original,
            error,
        };
        s.resync();
        s
    }

    /// Re-read the plain table from the document after an edit made directly on `target`.
    pub fn resync(&mut self) {
        self.target_table = parse_table(&self.target.to_string()).unwrap_or_default();
    }

    pub fn target_table(&self) -> &toml::Table {
        &self.target_table
    }

    fn last<'a>(
        it: impl Iterator<Item = (usize, &'a toml::Table)>,
        path: &[String],
    ) -> Option<TomlProvenance> {
        let mut out = None;
        for (layer, t) in it {
            if let Some(v) = get_path(t, path) {
                if !v.is_table() {
                    out = Some(TomlProvenance {
                        value: v.clone(),
                        layer,
                    });
                }
            }
        }
        out
    }

    pub fn inherited(&self, path: &[String]) -> Option<TomlProvenance> {
        Self::last(self.lower.iter().map(|l| (l.layer, &l.table)), path)
    }

    pub fn target_value(&self, path: &[String]) -> Option<&toml::Value> {
        get_path(&self.target_table, path).filter(|v| !v.is_table())
    }

    pub fn at_target(&self, path: &[String]) -> Option<TomlProvenance> {
        match self.target_value(path) {
            Some(v) => Some(TomlProvenance {
                value: v.clone(),
                layer: self.target_layer,
            }),
            None => self.inherited(path),
        }
    }

    pub fn effective(&self, path: &[String]) -> Option<TomlProvenance> {
        Self::last(self.upper.iter().map(|l| (l.layer, &l.table)), path)
            .or_else(|| self.at_target(path))
    }

    pub fn shadowed_by(&self, path: &[String]) -> Option<usize> {
        self.upper
            .iter()
            .rev()
            .find(|l| get_path(&l.table, path).is_some())
            .map(|l| l.layer)
    }

    fn all_tables(&self) -> impl Iterator<Item = &toml::Table> {
        self.lower
            .iter()
            .map(|l| &l.table)
            .chain(std::iter::once(&self.target_table))
            .chain(self.upper.iter().map(|l| &l.table))
    }

    /// Every table path holding at least one non-table value (or empty), over all layers, in
    /// first appearance order.
    pub fn section_paths(&self) -> Vec<Vec<String>> {
        fn rec(t: &toml::Table, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
            let has_values = t.values().any(|v| !v.is_table());
            if (has_values || (!path.is_empty() && t.is_empty())) && !out.contains(path) {
                out.push(path.clone());
            }
            for (k, v) in t {
                if let toml::Value::Table(sub) = v {
                    path.push(k.clone());
                    rec(sub, path, out);
                    path.pop();
                }
            }
        }
        let mut out = vec![];
        for t in self.all_tables() {
            rec(t, &mut vec![], &mut out);
        }
        out
    }

    /// Non-table keys of the table at `path`, over all layers.
    pub fn keys(&self, path: &[String]) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for t in self.all_tables() {
            let sub = if path.is_empty() {
                Some(t)
            } else {
                get_path(t, path).and_then(|v| v.as_table())
            };
            if let Some(sub) = sub {
                for (k, v) in sub {
                    if !v.is_table() && !out.contains(k) {
                        out.push(k.clone());
                    }
                }
            }
        }
        out
    }

    /// Set a leaf at the target layer; the inherited value removes it instead.
    pub fn set(&mut self, path: &[String], value: toml::Value) -> bool {
        if path.is_empty() || self.error.is_some() {
            return false;
        }
        if self.inherited(path).is_some_and(|p| p.value == value) {
            return self.revert(path);
        }
        if self.target_value(path) == Some(&value) {
            return false;
        }
        let (last, parents) = path.split_last().unwrap();
        let Some(t) = table_mut(&mut self.target, parents) else {
            return false;
        };
        match t.get_mut(last) {
            Some(Item::Value(v)) => {
                // keep the decor (comments before / after) of the existing line
                let decor = v.decor().clone();
                *v = to_edit_value(&value);
                *v.decor_mut() = decor;
            }
            _ => {
                t.insert(last, Item::Value(to_edit_value(&value)));
            }
        }
        self.resync();
        true
    }

    pub fn revert(&mut self, path: &[String]) -> bool {
        let r = remove_path(&mut self.target, path);
        self.resync();
        r
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

    pub fn discard(&mut self) {
        let text = self
            .original
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        self.target = text.parse().unwrap_or_default();
        self.resync();
    }
}

/// Dotted display of a path.
pub fn path_string(p: &[String]) -> String {
    p.join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_revert_keeps_comments() {
        let base = parse_table("[texture544]\nname = \"a\"\nframes = [1,2,3,4,5,6,7,8]\n").unwrap();
        let orig = "# mine\n[texture545]\nname = \"x\" # trailing\n";
        let mut t = LayeredToml::new(
            "textureanim.toml",
            vec![LayerToml {
                layer: 0,
                path: "b".into(),
                table: base,
            }],
            1,
            None,
            Some(orig.as_bytes().to_vec()),
        );
        let p = vec!["texture544".to_string(), "name".to_string()];
        assert!(t.set(&p, toml::Value::String("b".into())));
        assert_eq!(
            t.target.to_string(),
            "# mine\n[texture545]\nname = \"x\" # trailing\n\n[texture544]\nname = \"b\"\n"
        );
        assert!(t.set(&p, toml::Value::String("a".into())));
        assert_eq!(t.target.to_string(), orig);
        assert_eq!(t.pending(), Pending::Unchanged);
    }

    /// A value three tables deep must not be written as `[a]` `[a.b]` `[a.b.c]` (a header for
    /// every parent), which the game's reader rejects (the walk of `[a.b.c]` goes through the
    /// explicit `b`, toml_api.c:1294 / 683). What is written must be accepted and read as
    /// written, for 1, 2, 3 and 4 levels, also next to existing headers.
    #[test]
    fn written_tables_load_in_the_game() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        for orig in [
            "",
            "[a]\nkeep = 1\n",
            "[a]\n[a.b]\nkeep = 2\n",
            "[a]\n[a.b]\n[a.b.c]\nold = 3\n",
        ] {
            let mut t = LayeredToml::new(
                "x.toml",
                vec![],
                1,
                None,
                (!orig.is_empty()).then(|| orig.as_bytes().to_vec()),
            );
            assert!(t.set(&s(&["top"]), toml::Value::Integer(1)));
            assert!(t.set(&s(&["a", "one"]), toml::Value::Integer(1)));
            assert!(t.set(&s(&["a", "b", "two"]), toml::Value::Integer(2)));
            assert!(t.set(&s(&["a", "b", "c", "three"]), toml::Value::Integer(3)));
            assert!(t.set(
                &s(&["a", "b", "d", "e", "four"]),
                toml::Value::String("x\ny".into())
            ));
            let Pending::Write(bytes) = t.pending() else {
                panic!("nothing to write")
            };
            let text = String::from_utf8(bytes).unwrap();
            assert_eq!(
                kfx_formats::game_toml::same_as_toml(&text),
                Ok(()),
                "from {orig:?}:\n{text}"
            );
            assert_eq!(parse_table(&text).unwrap(), *t.target_table(), "{text}");
        }
    }
}
