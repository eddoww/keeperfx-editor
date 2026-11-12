//! A `.cfg` file seen through the layers, edited at one target layer.
//!
//! Partial loads change only the fields a file contains (`parse_named_field_block`,
//! `src/config.c:984`): a key's effective value is the last occurrence in the first block of
//! that name (`find_conf_block`, src/config.c:186) of the last layer that has it. Some blocks are
//! lists whose presence in a later file replaces the whole list (`[research]`, `[sacrifices]`:
//! `clear_research_for_all_players` / `clear_sacrifice_recipes`, src/config_rules.c:446/542); those
//! are written as whole blocks.
//!
//! The target layer's file is a lossless [`CfgDoc`]: editing one key rewrites only that line,
//! new keys are appended to their section, comments and formatting of an existing override file
//! are kept. A value equal to the inherited one is removed instead of written, so the file stays
//! the minimal difference.
use kfx_formats::cfg::{split_tokens, CfgDoc};
use std::path::PathBuf;

/// A layer's copy of the file.
#[derive(Clone, Debug)]
pub struct LayerDoc {
    pub layer: usize,
    pub path: PathBuf,
    pub doc: CfgDoc,
}

/// A value and the layer it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub value: String,
    pub layer: usize,
}

/// What saving the target file would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    Unchanged,
    Write(Vec<u8>),
    Delete,
}

/// One cfg file over the layer stack.
#[derive(Clone, Debug)]
pub struct LayeredCfg {
    pub key: String,
    /// Layers below the target that have the file, in load order.
    pub lower: Vec<LayerDoc>,
    /// Layers above the target that have the file (they override the target).
    pub upper: Vec<LayerDoc>,
    pub target_layer: usize,
    /// Where the target file is or would be written (`None`: the layer has no folder for it).
    pub target_path: Option<PathBuf>,
    pub target: CfgDoc,
    /// Bytes of the target file when opened (`None`: there was no file).
    pub original: Option<Vec<u8>>,
    /// Lower-case names of whole-block sections.
    pub whole_blocks: Vec<String>,
}

fn same_value(a: &str, b: &str) -> bool {
    split_tokens(a) == split_tokens(b)
}

/// Does the body of the first `[name]` block hold a comment line?
pub fn section_has_comment(doc: &CfgDoc, name: &str) -> bool {
    let text = doc.to_string();
    let mut inside = false;
    for line in text.split(['\n', '\r']) {
        let t = line.trim_start_matches([' ', '\t']);
        if let Some(rest) = t.strip_prefix('[') {
            if inside {
                return false;
            }
            inside = rest
                .split(']')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case(name);
        } else if inside && t.starts_with(';') {
            return true;
        }
    }
    false
}

/// The `;` comment lines right above `key` in the first `[section]` block (the explanations the
/// install's fxdata files carry), joined by newlines.
pub fn key_comment(doc: &CfgDoc, section: &str, key: &str) -> Option<String> {
    let text = doc.to_string();
    let mut inside = false;
    let mut seen = false;
    let mut pending: Vec<&str> = vec![];
    for line in text.lines() {
        let t = line.trim_matches([' ', '\t', '\r']);
        if let Some(rest) = t.strip_prefix('[') {
            if inside {
                return None;
            }
            inside = !seen
                && rest
                    .split(']')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case(section);
            seen |= inside;
            pending.clear();
        } else if let Some(c) = t.strip_prefix(';') {
            pending.push(c.trim());
        } else if t.is_empty() {
            pending.clear();
        } else {
            if inside
                && t.split('=')
                    .next()
                    .is_some_and(|k| k.trim().eq_ignore_ascii_case(key))
            {
                return (!pending.is_empty()).then(|| pending.join("\n"));
            }
            pending.clear();
        }
    }
    None
}

/// Whether a document has anything that changes the game: an entry, a section header, or a
/// comment worth keeping.
pub fn doc_is_empty(doc: &CfgDoc) -> bool {
    doc.section_names().is_empty()
        && doc.sections().iter().all(|s| s.entries.is_empty())
        && !doc
            .to_string()
            .lines()
            .any(|l| l.trim_start().starts_with(';'))
}

impl LayeredCfg {
    /// Build from the per-layer documents. `target` is the target layer's current document
    /// (`CfgDoc::default()` when it has no file).
    pub fn new(
        key: &str,
        docs: Vec<LayerDoc>,
        target_layer: usize,
        target_path: Option<PathBuf>,
        original: Option<Vec<u8>>,
        whole_blocks: Vec<String>,
    ) -> Self {
        let (mut lower, mut upper) = (vec![], vec![]);
        let mut target = CfgDoc::default();
        for d in docs {
            if d.layer < target_layer {
                lower.push(d);
            } else if d.layer > target_layer {
                upper.push(d);
            } else {
                target = d.doc;
            }
        }
        LayeredCfg {
            key: key.to_string(),
            lower,
            upper,
            target_layer,
            target_path,
            target,
            original,
            whole_blocks: whole_blocks
                .into_iter()
                .map(|s| s.to_ascii_lowercase())
                .collect(),
        }
    }

    pub fn is_whole_block(&self, section: &str) -> bool {
        self.whole_blocks
            .iter()
            .any(|s| s.eq_ignore_ascii_case(section))
    }

    fn last_value<'a>(
        docs: impl Iterator<Item = (usize, &'a CfgDoc)>,
        section: &str,
        key: &str,
    ) -> Option<Provenance> {
        let mut out = None;
        for (layer, doc) in docs {
            if let Some(v) = doc.get(section, key) {
                out = Some(Provenance { value: v, layer });
            }
        }
        out
    }

    fn lower_docs(&self) -> impl Iterator<Item = (usize, &CfgDoc)> {
        self.lower.iter().map(|d| (d.layer, &d.doc))
    }

    fn all_docs(&self) -> impl Iterator<Item = (usize, &CfgDoc)> {
        self.lower
            .iter()
            .map(|d| (d.layer, &d.doc))
            .chain(std::iter::once((self.target_layer, &self.target)))
            .chain(self.upper.iter().map(|d| (d.layer, &d.doc)))
    }

    /// Value the target layer inherits (layers below it).
    pub fn inherited(&self, section: &str, key: &str) -> Option<Provenance> {
        if self.is_whole_block(section) {
            let (layer, lines) = self.inherited_block(section)?;
            return lines
                .iter()
                .rev()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| Provenance {
                    value: v.clone(),
                    layer,
                });
        }
        Self::last_value(self.lower_docs(), section, key)
    }

    /// Value written in the target layer, if any.
    pub fn target_value(&self, section: &str, key: &str) -> Option<String> {
        self.target.get(section, key)
    }

    /// The value the game ends up with (all layers, including those above the target).
    pub fn effective(&self, section: &str, key: &str) -> Option<Provenance> {
        if self.is_whole_block(section) {
            let (layer, lines) = self.effective_block(section)?;
            return lines
                .iter()
                .rev()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| Provenance {
                    value: v.clone(),
                    layer,
                });
        }
        Self::last_value(self.all_docs(), section, key)
    }

    /// The value as seen up to and including the target layer (what the editor edits).
    pub fn at_target(&self, section: &str, key: &str) -> Option<Provenance> {
        match self.target_value(section, key) {
            Some(v) if !self.is_whole_block(section) => Some(Provenance {
                value: v,
                layer: self.target_layer,
            }),
            _ if self.is_whole_block(section) => {
                let (layer, lines) = self.block_at_target(section)?;
                lines
                    .iter()
                    .rev()
                    .find(|(k, _)| k.eq_ignore_ascii_case(key))
                    .map(|(_, v)| Provenance {
                        value: v.clone(),
                        layer,
                    })
            }
            _ => self.inherited(section, key),
        }
    }

    /// A layer above the target that sets this key (the target's value is then not what the
    /// game uses).
    pub fn shadowed_by(&self, section: &str, key: &str) -> Option<usize> {
        if self.is_whole_block(section) {
            return self
                .upper
                .iter()
                .rev()
                .find(|d| d.doc.has_section(section))
                .map(|d| d.layer);
        }
        self.upper
            .iter()
            .rev()
            .find(|d| d.doc.get(section, key).is_some())
            .map(|d| d.layer)
    }

    /// Section names over all layers, first appearance order, case-insensitively unique.
    pub fn section_names(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for (_, doc) in self.all_docs() {
            for n in doc.section_names() {
                if !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
                    out.push(n);
                }
            }
        }
        out
    }

    /// Keys of a section over all layers, first appearance order.
    pub fn keys(&self, section: &str) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for (_, doc) in self.all_docs() {
            for e in doc.entries(section) {
                if !out.iter().any(|o| o.eq_ignore_ascii_case(&e.key)) {
                    out.push(e.key);
                }
            }
        }
        out
    }

    /// Does the target layer touch this section?
    pub fn target_has_section(&self, section: &str) -> bool {
        self.target.has_section(section)
    }

    // ---------------------------------------------------------------- edits

    /// Set a key at the target layer. Writing the inherited value removes the key instead.
    /// Returns whether the document changed.
    pub fn set(&mut self, section: &str, key: &str, value: &str) -> bool {
        let value = value.trim_end();
        let value: String = value.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        if self.is_whole_block(section) {
            let mut lines = self
                .block_at_target(section)
                .map(|(_, l)| l)
                .unwrap_or_default();
            match lines
                .iter_mut()
                .rev()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
            {
                Some(e) => e.1 = value,
                None => lines.push((key.to_string(), value)),
            }
            return self.set_block(section, &lines);
        }
        let before = self.target.to_string();
        match self.inherited(section, key) {
            Some(p) if same_value(&p.value, &value) => {
                self.revert(section, key);
            }
            _ => {
                if self.target_value(section, key).is_some_and(|v| v == value) {
                    return false;
                }
                self.target.set(section, key, &value);
            }
        }
        before != self.target.to_string()
    }

    /// Remove a key from the target layer (the inherited value applies again).
    pub fn revert(&mut self, section: &str, key: &str) -> bool {
        if self.is_whole_block(section) {
            let mut lines = self
                .block_at_target(section)
                .map(|(_, l)| l)
                .unwrap_or_default();
            let inh = self
                .inherited_block(section)
                .map(|(_, l)| l)
                .unwrap_or_default();
            let iv = inh
                .iter()
                .rev()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .cloned();
            lines.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
            if let Some(e) = iv {
                lines.push(e);
            }
            return self.set_block(section, &lines);
        }
        let n = self.target.remove(section, key);
        self.cleanup_section(section);
        n > 0
    }

    /// Remove a whole section from the target layer.
    pub fn revert_section(&mut self, section: &str) -> bool {
        let mut any = false;
        while self.target.remove_section(section) {
            any = true;
        }
        any
    }

    /// Drop an empty section header (no entries, no comments) of a field section.
    fn cleanup_section(&mut self, section: &str) {
        if self.is_whole_block(section) || !self.target.has_section(section) {
            return;
        }
        if self.target.entries(section).is_empty() && !section_has_comment(&self.target, section) {
            self.target.remove_section(section);
        }
    }

    // ---------------------------------------------------------------- whole blocks

    fn block_of(doc: &CfgDoc, section: &str) -> Vec<(String, String)> {
        doc.entries(section)
            .into_iter()
            .map(|e| (e.key, e.value))
            .collect()
    }

    /// Block of the last lower layer that has the section.
    pub fn inherited_block(&self, section: &str) -> Option<(usize, Vec<(String, String)>)> {
        self.lower
            .iter()
            .rev()
            .find(|d| d.doc.has_section(section))
            .map(|d| (d.layer, Self::block_of(&d.doc, section)))
    }

    /// Block as of the target layer.
    pub fn block_at_target(&self, section: &str) -> Option<(usize, Vec<(String, String)>)> {
        if self.target.has_section(section) {
            return Some((self.target_layer, Self::block_of(&self.target, section)));
        }
        self.inherited_block(section)
    }

    /// Block the game ends up with.
    pub fn effective_block(&self, section: &str) -> Option<(usize, Vec<(String, String)>)> {
        if let Some(d) = self.upper.iter().rev().find(|d| d.doc.has_section(section)) {
            return Some((d.layer, Self::block_of(&d.doc, section)));
        }
        self.block_at_target(section)
    }

    /// Replace a whole block at the target layer; equal to the inherited block removes it.
    pub fn set_block(&mut self, section: &str, lines: &[(String, String)]) -> bool {
        let before = self.target.to_string();
        let inh = self.inherited_block(section).map(|(_, l)| l);
        let equal = inh.as_ref().is_some_and(|i| {
            i.len() == lines.len()
                && i.iter()
                    .zip(lines)
                    .all(|(a, b)| a.0.eq_ignore_ascii_case(&b.0) && same_value(&a.1, &b.1))
        });
        if equal {
            self.revert_section(section);
        } else {
            if self.target.has_section(section) {
                let keys: Vec<String> = self
                    .target
                    .entries(section)
                    .into_iter()
                    .map(|e| e.key)
                    .collect();
                for k in keys {
                    self.target.remove(section, &k);
                }
            } else {
                self.target.add_section(section);
            }
            for (k, v) in lines {
                self.target.add(section, k, v);
            }
        }
        before != self.target.to_string()
    }

    // ---------------------------------------------------------------- saving

    /// What saving would do.
    pub fn pending(&self) -> Pending {
        let empty = doc_is_empty(&self.target);
        let bytes = self.target.to_bytes();
        match (&self.original, empty) {
            (None, true) => Pending::Unchanged,
            (Some(_), true) => Pending::Delete,
            (Some(o), false) if *o == bytes => Pending::Unchanged,
            _ => Pending::Write(bytes),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.pending() != Pending::Unchanged
    }

    /// Mark the current document as saved.
    pub fn mark_saved(&mut self) {
        self.original = if doc_is_empty(&self.target) {
            None
        } else {
            Some(self.target.to_bytes())
        };
    }

    /// Discard unsaved edits.
    pub fn discard(&mut self) {
        self.target = match &self.original {
            Some(b) => CfgDoc::from_bytes(b),
            None => CfgDoc::default(),
        };
    }

    /// The base layer's comment above `key` ([`key_comment`]), if the base file is open here.
    pub fn base_comment(&self, section: &str, key: &str) -> Option<String> {
        let base = if self.target_layer == 0 {
            Some(&self.target)
        } else {
            self.lower.iter().find(|d| d.layer == 0).map(|d| &d.doc)
        };
        key_comment(base?, section, key)
    }

    /// `(section, key, inherited, target)` of every key the target layer sets.
    pub fn overrides(&self) -> Vec<(String, String, Option<String>, String)> {
        let mut out = vec![];
        for s in self.target.sections().into_iter().skip(1) {
            for e in s.entries {
                let inh = self.inherited(&s.name, &e.key).map(|p| p.value);
                out.push((s.name.clone(), e.key, inh, e.value));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lc(base: &str, target: Option<&str>) -> LayeredCfg {
        let mut docs = vec![LayerDoc {
            layer: 0,
            path: "base".into(),
            doc: CfgDoc::parse(base),
        }];
        if let Some(t) = target {
            docs.push(LayerDoc {
                layer: 1,
                path: "t".into(),
                doc: CfgDoc::parse(t),
            });
        }
        LayeredCfg::new(
            "rules.cfg",
            docs,
            1,
            Some("t".into()),
            target.map(|t| t.as_bytes().to_vec()),
            vec!["research".into()],
        )
    }

    #[test]
    fn set_and_revert_minimal() {
        let mut c = lc(
            "; base\n[game]\nGoldPerHoard = 2000\nPayDayGap = 5000\n",
            None,
        );
        assert!(c.set("game", "GOLDPERHOARD", "2500"));
        assert_eq!(c.target.to_string(), "[game]\nGOLDPERHOARD = 2500\n");
        assert_eq!(
            c.effective("game", "GoldPerHoard").unwrap(),
            Provenance {
                value: "2500".into(),
                layer: 1
            }
        );
        assert!(c.set("game", "GoldPerHoard", "2000"));
        assert_eq!(c.target.to_string(), "");
        assert_eq!(c.pending(), Pending::Unchanged);
    }

    #[test]
    fn comments_survive() {
        let t = "; my override\n[game]\n; tuned\nPayDayGap = 100\n";
        let mut c = lc("[game]\nGoldPerHoard = 2000\nPayDayGap = 5000\n", Some(t));
        c.set("game", "GoldPerHoard", "3000");
        assert_eq!(
            c.target.to_string(),
            "; my override\n[game]\n; tuned\nPayDayGap = 100\nGoldPerHoard = 3000\n"
        );
        c.revert("game", "PayDayGap");
        c.revert("game", "GoldPerHoard");
        // the section keeps its comment
        assert_eq!(c.target.to_string(), "; my override\n[game]\n; tuned\n");
    }

    #[test]
    fn comments_above_keys() {
        let d = CfgDoc::parse(
            "[game]\n; Gold per hoard.\n; Second line.\nGoldPerHoard = \
                2000\nPayDayGap = 5000\n\n; other\n[x]\nPayDayGap = 1\n",
        );
        assert_eq!(
            key_comment(&d, "game", "GoldPerHoard").as_deref(),
            Some("Gold per hoard.\nSecond line.")
        );
        assert_eq!(key_comment(&d, "game", "PayDayGap"), None);
        assert_eq!(key_comment(&d, "x", "PayDayGap"), None);
    }

    #[test]
    fn whole_block() {
        let mut c = lc(
            "[research]\nResearch = MAGIC POWER_HAND 10\nResearch = ROOM TREASURE 20\n",
            None,
        );
        let mut l = c.block_at_target("research").unwrap().1;
        l.swap(0, 1);
        assert!(c.set_block("research", &l));
        assert_eq!(
            c.target.to_string(),
            "[research]\nResearch = ROOM TREASURE 20\nResearch = MAGIC POWER_HAND 10\n"
        );
        l.swap(0, 1);
        c.set_block("research", &l);
        assert_eq!(c.target.to_string(), "");
        // an empty list is meaningful (clears research)
        c.set_block("research", &[]);
        assert_eq!(c.target.to_string(), "[research]\n");
        assert!(matches!(c.pending(), Pending::Write(_)));
    }
}
