//! The editing session: the layer stack, the target layer, the open files with their unsaved
//! edits, saving through [`ConfigFs`], events for the host and the effective `GameData`.
use crate::cfgedit::{LayerDoc, LayeredCfg, Pending};
use crate::columnset::ColumnsetModel;
use crate::fs::{read_unpacked, ConfigFs, StdFs};
use crate::layers::{is_creature_file, LayerKind, LayerStack};
use crate::schema::Schema;
use crate::slabset::SlabsetModel;
use crate::tomledit::{parse_table, LayerToml, LayeredToml};
use kfx_formats::cfg::CfgDoc;
use kfx_map::gamedata::{ConfigLayer, GameData};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `MIN_CONFIG_FILE_SIZE` (src/config.h:37): shorter files are skipped by the game.
const MIN_CONFIG_FILE_SIZE: usize = 4;

/// `.cfg` files the game layers (`load_config` users).
pub const CFG_FILES: &[&str] = &[
    "rules.cfg",
    "terrain.cfg",
    "objects.cfg",
    "trapdoor.cfg",
    "magic.cfg",
    "creature.cfg",
    "crstates.cfg",
    "cubes.cfg",
    "lenses.cfg",
    "keepcompp.cfg",
    "sounds.cfg",
];
/// `.toml` files the game layers.
pub const TOML_FILES: &[&str] = &[
    "slabset.toml",
    "columnset.toml",
    "effects.toml",
    "textureanim.toml",
    "spritecolors.toml",
    "playerstates.toml",
    "powerhands.toml",
];

/// Something the host may want to react to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigEvent {
    /// An unsaved edit changed `file` at `layer` (reload `GameData` with
    /// [`ConfigSession::effective_gamedata`] and regenerate affected slabs).
    Changed { file: String, layer: LayerKind },
    /// `file` was written (or deleted) at `path`.
    Saved {
        file: String,
        path: PathBuf,
        deleted: bool,
    },
    /// The user asked to open a file in an external program.
    RequestOpenFile { path: PathBuf },
    /// The target layer changed; everything shown was re-read.
    TargetChanged { layer: LayerKind },
}

/// An open config file.
#[derive(Clone, Debug)]
pub enum OpenFile {
    Cfg(LayeredCfg),
    Toml(LayeredToml),
    Slabset(SlabsetModel),
    Columnset(ColumnsetModel),
}

impl OpenFile {
    pub fn pending(&self) -> Pending {
        match self {
            OpenFile::Cfg(c) => c.pending(),
            OpenFile::Toml(t) => t.pending(),
            OpenFile::Slabset(s) => s.pending(),
            OpenFile::Columnset(c) => c.pending(),
        }
    }
    pub fn target_path(&self) -> Option<&PathBuf> {
        match self {
            OpenFile::Cfg(c) => c.target_path.as_ref(),
            OpenFile::Toml(t) => t.target_path.as_ref(),
            OpenFile::Slabset(s) => s.target_path.as_ref(),
            OpenFile::Columnset(c) => c.target_path.as_ref(),
        }
    }
    fn mark_saved(&mut self) {
        match self {
            OpenFile::Cfg(c) => c.mark_saved(),
            OpenFile::Toml(t) => t.mark_saved(),
            OpenFile::Slabset(s) => s.mark_saved(),
            OpenFile::Columnset(c) => c.mark_saved(),
        }
    }
    /// The target layer's text as it would be saved (empty when the file would not exist).
    pub fn target_text(&self) -> String {
        match self {
            OpenFile::Cfg(c) => c.target.to_string(),
            OpenFile::Toml(t) => t.target.to_string(),
            OpenFile::Slabset(s) => s.target.to_string(),
            OpenFile::Columnset(c) => c.target.to_string(),
        }
    }
    /// The target file's text when it was opened / last saved.
    pub fn original_text(&self) -> String {
        let o = match self {
            OpenFile::Cfg(c) => &c.original,
            OpenFile::Toml(t) => &t.original,
            OpenFile::Slabset(s) => &s.original,
            OpenFile::Columnset(c) => &c.original,
        };
        o.as_ref()
            .map(|b| kfx_formats::cfg::latin1_decode(b))
            .unwrap_or_default()
    }
}

/// One config file of the file list.
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub key: String,
    /// Layers that have the file.
    pub present: Vec<usize>,
}

/// The editing session.
pub struct ConfigSession {
    pub layers: LayerStack,
    target: usize,
    /// Allows the base layer as target ("base game files (advanced)").
    pub allow_base: bool,
    pub fs: Arc<dyn ConfigFs + Send + Sync>,
    pub schema: &'static Schema,
    files: BTreeMap<String, OpenFile>,
    events: Vec<ConfigEvent>,
    /// Bumped on every edit (caches key on it).
    pub generation: u64,
    names_cache: Option<(u64, BTreeMap<String, Vec<(i64, String)>>)>,
    /// Errors of the last operations (shown by the UI).
    pub errors: Vec<String>,
}

impl ConfigSession {
    /// A session over `layers`, editing the top layer (the map when there is one).
    pub fn new(fs: Arc<dyn ConfigFs + Send + Sync>, layers: LayerStack) -> Self {
        let target = layers.layers.len().saturating_sub(1);
        ConfigSession {
            layers,
            target,
            allow_base: false,
            fs,
            schema: Schema::builtin(),
            files: BTreeMap::new(),
            events: vec![],
            generation: 0,
            names_cache: None,
            errors: vec![],
        }
    }

    pub fn target(&self) -> usize {
        self.target
    }
    pub fn target_kind(&self) -> &LayerKind {
        &self.layers.layers[self.target].kind
    }

    /// Change the target layer. Refused while there are unsaved edits (save or discard first)
    /// and for the base layer unless [`ConfigSession::allow_base`].
    pub fn set_target(&mut self, layer: usize) -> Result<(), String> {
        if layer >= self.layers.layers.len() {
            return Err("no such layer".into());
        }
        if layer == 0 && !self.allow_base {
            return Err("editing the base game files needs the advanced option".into());
        }
        if layer == self.target {
            return Ok(());
        }
        if self.is_dirty() {
            return Err("save or discard the unsaved edits first".into());
        }
        self.target = layer;
        self.files.clear();
        self.generation += 1;
        self.events.push(ConfigEvent::TargetChanged {
            layer: self.layers.layers[layer].kind.clone(),
        });
        Ok(())
    }

    /// Take the queued events.
    pub fn take_events(&mut self) -> Vec<ConfigEvent> {
        std::mem::take(&mut self.events)
    }
    pub fn push_event(&mut self, e: ConfigEvent) {
        self.events.push(e);
    }

    /// Record an edit of `key` (event + cache invalidation).
    pub fn touched(&mut self, key: &str) {
        self.generation += 1;
        let ev = ConfigEvent::Changed {
            file: key.to_string(),
            layer: self.target_kind().clone(),
        };
        if self.events.last() != Some(&ev) {
            self.events.push(ev);
        }
    }

    // ------------------------------------------------------------------ reading

    /// `(layer, path, bytes)` of every layer that has `key` (files under 4 bytes skipped).
    pub fn layer_files(&self, key: &str) -> Vec<(usize, PathBuf, Vec<u8>)> {
        let mut out = vec![];
        for (i, l) in self.layers.layers.iter().enumerate() {
            let Some(p) = l.existing(&*self.fs, key) else {
                continue;
            };
            match read_unpacked(&*self.fs, &p) {
                Ok(Some(b)) if b.len() >= MIN_CONFIG_FILE_SIZE || i == self.target => {
                    out.push((i, p, b))
                }
                Ok(_) => {}
                Err(e) => eprintln!("kfx-config-ui: {}: {e}", p.display()),
            }
        }
        out
    }

    fn whole_blocks(&self, key: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .schema
            .file(key)
            .map(|f| {
                f.sections
                    .iter()
                    .filter(|s| s.replaces_whole_block())
                    .map(|s| s.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        if key.eq_ignore_ascii_case("rules.cfg") {
            for s in ["research", "sacrifices"] {
                if !v.iter().any(|x| x.eq_ignore_ascii_case(s)) {
                    v.push(s.into());
                }
            }
        }
        v
    }

    fn open(&self, key: &str) -> OpenFile {
        let files = self.layer_files(key);
        let t = self.target;
        let tl = &self.layers.layers[t];
        let target_path = tl.write_path(&*self.fs, key);
        let original = files.iter().find(|f| f.0 == t).map(|f| f.2.clone());
        let lower_key = key.to_ascii_lowercase();
        if lower_key == "slabset.toml" || lower_key == "columnset.toml" {
            let tables: Vec<(usize, toml::Table)> = files
                .iter()
                .filter(|f| f.0 != t)
                .filter_map(|(l, p, b)| match parse_table(&String::from_utf8_lossy(b)) {
                    Ok(tb) => Some((*l, tb)),
                    Err(e) => {
                        eprintln!("kfx-config-ui: {}: {e}", p.display());
                        None
                    }
                })
                .collect();
            return if lower_key == "slabset.toml" {
                OpenFile::Slabset(SlabsetModel::new(tables, t, target_path, original))
            } else {
                OpenFile::Columnset(ColumnsetModel::new(tables, t, target_path, original))
            };
        }
        if lower_key.ends_with(".toml") {
            let layers: Vec<LayerToml> = files
                .iter()
                .filter(|f| f.0 != t)
                .filter_map(|(l, p, b)| {
                    parse_table(&String::from_utf8_lossy(b))
                        .ok()
                        .map(|table| LayerToml {
                            layer: *l,
                            path: p.clone(),
                            table,
                        })
                })
                .collect();
            return OpenFile::Toml(LayeredToml::new(key, layers, t, target_path, original));
        }
        let docs: Vec<LayerDoc> = files
            .iter()
            .map(|(l, p, b)| LayerDoc {
                layer: *l,
                path: p.clone(),
                doc: CfgDoc::from_bytes(b),
            })
            .collect();
        OpenFile::Cfg(LayeredCfg::new(
            key,
            docs,
            t,
            target_path,
            original,
            self.whole_blocks(key),
        ))
    }

    /// The open file `key`, opened on first use.
    pub fn file(&mut self, key: &str) -> &mut OpenFile {
        let k = key.to_ascii_lowercase();
        if !self.files.contains_key(&k) {
            let f = self.open(key);
            self.files.insert(k.clone(), f);
        }
        self.files.get_mut(&k).unwrap()
    }

    /// Already-open file, if any.
    pub fn file_if_open(&self, key: &str) -> Option<&OpenFile> {
        self.files.get(&key.to_ascii_lowercase())
    }

    pub fn cfg(&mut self, key: &str) -> Option<&mut LayeredCfg> {
        match self.file(key) {
            OpenFile::Cfg(c) => Some(c),
            _ => None,
        }
    }
    pub fn toml(&mut self, key: &str) -> Option<&mut LayeredToml> {
        match self.file(key) {
            OpenFile::Toml(c) => Some(c),
            _ => None,
        }
    }
    pub fn slabset(&mut self) -> &mut SlabsetModel {
        match self.file("slabset.toml") {
            OpenFile::Slabset(s) => s,
            _ => unreachable!(),
        }
    }
    pub fn columnset(&mut self) -> &mut ColumnsetModel {
        match self.file("columnset.toml") {
            OpenFile::Columnset(s) => s,
            _ => unreachable!(),
        }
    }

    /// Set a cfg key at the target layer (records an event when it changed).
    pub fn set_cfg(&mut self, key: &str, section: &str, k: &str, value: &str) -> bool {
        let changed = self.cfg(key).is_some_and(|c| c.set(section, k, value));
        if changed {
            self.touched(key);
        }
        changed
    }
    pub fn revert_cfg(&mut self, key: &str, section: &str, k: &str) -> bool {
        let changed = self.cfg(key).is_some_and(|c| c.revert(section, k));
        if changed {
            self.touched(key);
        }
        changed
    }

    /// Effective (as of the target layer) value of a cfg key.
    pub fn cfg_value(&mut self, key: &str, section: &str, k: &str) -> Option<String> {
        self.cfg(key)?.at_target(section, k).map(|p| p.value)
    }

    /// Creature names of `creature.cfg [common] Creatures` (as of the target layer).
    pub fn creature_names(&mut self) -> Vec<String> {
        self.cfg_value("creature.cfg", "common", "Creatures")
            .map(|v| v.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Every config file the editors know, with the layers that have it. Creature files are
    /// listed for every creature of `creature.cfg`.
    pub fn file_list(&mut self) -> Vec<FileEntry> {
        let mut keys: Vec<String> = CFG_FILES
            .iter()
            .chain(TOML_FILES)
            .map(|s| s.to_string())
            .collect();
        for f in &self.schema.files {
            if !f.file.contains('*') && !keys.iter().any(|k| k.eq_ignore_ascii_case(&f.file)) {
                keys.push(f.file.to_ascii_lowercase());
            }
        }
        for c in self.creature_names() {
            keys.push(format!("creatrs/{}.cfg", c.to_ascii_lowercase()));
        }
        keys.into_iter()
            .map(|k| {
                let present = self
                    .layers
                    .layers
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.existing(&*self.fs, &k).is_some())
                    .map(|(i, _)| i)
                    .collect();
                FileEntry { key: k, present }
            })
            .collect()
    }

    // ------------------------------------------------------------------ saving

    pub fn is_dirty(&self) -> bool {
        self.files
            .values()
            .any(|f| f.pending() != Pending::Unchanged)
    }

    /// `(key, path, pending)` of every file with unsaved edits.
    pub fn pending(&self) -> Vec<(String, PathBuf, Pending)> {
        self.files
            .iter()
            .filter_map(|(k, f)| {
                let p = f.pending();
                (p != Pending::Unchanged)
                    .then(|| (k.clone(), f.target_path().cloned().unwrap_or_default(), p))
            })
            .collect()
    }

    /// Write every pending file through the host's file system.
    pub fn save(&mut self) -> Result<Vec<PathBuf>, String> {
        let mut out = vec![];
        let keys: Vec<String> = self.files.keys().cloned().collect();
        for k in keys {
            if let Some(p) = self.save_file(&k)? {
                out.push(p);
            }
        }
        Ok(out)
    }

    /// Write one file; returns its path when something was written or deleted.
    pub fn save_file(&mut self, key: &str) -> Result<Option<PathBuf>, String> {
        if self.target == 0 && !self.allow_base {
            return Err("refusing to write base game files".into());
        }
        let k = key.to_ascii_lowercase();
        let Some(f) = self.files.get_mut(&k) else {
            return Ok(None);
        };
        let pending = f.pending();
        let Some(path) = f.target_path().cloned() else {
            return if pending == Pending::Unchanged {
                Ok(None)
            } else {
                Err(format!(
                    "{key}: the target layer has no folder for this file"
                ))
            };
        };
        let deleted = match pending {
            Pending::Unchanged => return Ok(None),
            Pending::Write(b) => {
                self.fs
                    .write(&path, &b)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                false
            }
            Pending::Delete => {
                self.fs
                    .remove(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                true
            }
        };
        f.mark_saved();
        self.events.push(ConfigEvent::Saved {
            file: k,
            path: path.clone(),
            deleted,
        });
        Ok(Some(path))
    }

    /// Throw away the unsaved edits of every file.
    pub fn discard_all(&mut self) {
        self.files.clear();
        self.generation += 1;
    }
    pub fn discard(&mut self, key: &str) {
        self.files.remove(&key.to_ascii_lowercase());
        self.generation += 1;
        self.touched(key);
    }

    // ------------------------------------------------------------------ game data

    /// The game's view of the configuration with the unsaved edits applied, for live previews.
    ///
    /// `kfx_map::gamedata` reads files itself, so the target layer's config files (saved state
    /// overlaid with the unsaved edits) are written to `scratch` (a private temporary folder of
    /// the caller, emptied first) and the target layer is replaced by it.
    pub fn effective_gamedata(&self, scratch: &Path) -> Result<GameData, String> {
        let _ = std::fs::remove_dir_all(scratch);
        std::fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
        let tl = &self.layers.layers[self.target];
        let disk = StdFs;
        let mut names: Vec<String> = CFG_FILES
            .iter()
            .chain(TOML_FILES)
            .map(|s| s.to_string())
            .collect();
        for k in self.files.keys() {
            if is_creature_file(k).is_none() && !names.contains(k) {
                names.push(k.clone());
            }
        }
        for n in &names {
            let content = match self.files.get(n) {
                Some(f) => {
                    let t = f.target_text();
                    (!t.is_empty()).then(|| kfx_formats::cfg::latin1_encode(&t))
                }
                None => tl
                    .existing(&*self.fs, n)
                    .and_then(|p| read_unpacked(&*self.fs, &p).ok().flatten()),
            };
            if let Some(b) = content {
                disk.write(&scratch.join(format!("{}{}", tl.prefix, n)), &b)
                    .map_err(|e| e.to_string())?;
            }
        }
        let mut cl = self.layers.clone();
        cl.layers[self.target].dir = Some(scratch.to_path_buf());
        let mut layers = cl.to_config_layers();
        if tl.dir.is_none() {
            // the target had no folder: put the scratch layer at its load position
            let pos = self.layers.layers[..self.target]
                .iter()
                .filter(|l| l.dir.is_some())
                .count();
            layers.layers.insert(
                pos,
                ConfigLayer {
                    dir: scratch.to_path_buf(),
                    prefix: tl.prefix.clone(),
                },
            );
        }
        GameData::load(&layers).map_err(|e| e.to_string())
    }

    // ------------------------------------------------------------------ names (for pickers)

    /// `(id, name)` lists for reference kinds: `creature`, `object`, `room`, `slab`, `spell`,
    /// `shot`, `power`, `special`, `trap`, `door`, `instance`, `job`, `angerjob`, `state`,
    /// `lens`, `cube`, `effect`, `effect_element`, `effect_generator`, `sound`, `process`,
    /// `check`, `event`, `computer`, `texture`. Values as of the target layer.
    pub fn names(&mut self, kind: &str) -> Vec<(i64, String)> {
        if self
            .names_cache
            .as_ref()
            .is_none_or(|c| c.0 != self.generation)
        {
            let all = self.compute_names();
            self.names_cache = Some((self.generation, all));
        }
        self.names_cache
            .as_ref()
            .unwrap()
            .1
            .get(kind)
            .cloned()
            .unwrap_or_default()
    }

    fn numbered_cfg(&mut self, file: &str, base: &str) -> Vec<(i64, String)> {
        let Some(c) = self.cfg(file) else {
            return vec![];
        };
        let mut out: Vec<(i64, String)> = c
            .section_names()
            .into_iter()
            .filter_map(|s| {
                let rest = s.get(base.len()..)?;
                if !s[..base.len()].eq_ignore_ascii_case(base)
                    || rest.is_empty()
                    || !rest.bytes().all(|b| b.is_ascii_digit())
                {
                    return None;
                }
                let n: i64 = rest.parse().ok()?;
                let name = c
                    .at_target(&s, "Name")
                    .map(|p| p.value.split_whitespace().next().unwrap_or("").to_string())
                    .unwrap_or_default();
                Some((n, name))
            })
            .collect();
        out.sort();
        out.dedup_by_key(|e| e.0);
        out
    }

    fn numbered_toml(&mut self, file: &str, base: &str, name_key: &str) -> Vec<(i64, String)> {
        let Some(t) = self.toml(file) else {
            return vec![];
        };
        let mut out: Vec<(i64, String)> = t
            .section_paths()
            .into_iter()
            .filter(|p| p.len() == 1)
            .filter_map(|p| {
                let rest = p[0].strip_prefix(base)?;
                let n: i64 = rest.parse().ok()?;
                if format!("{base}{n}") != p[0] {
                    return None;
                }
                let mut path = p.clone();
                path.push(name_key.to_string());
                let name = t
                    .at_target(&path)
                    .and_then(|v| v.value.as_str().map(str::to_string))
                    .unwrap_or_default();
                Some((n, name))
            })
            .collect();
        out.sort();
        out
    }

    fn compute_names(&mut self) -> BTreeMap<String, Vec<(i64, String)>> {
        let mut m = BTreeMap::new();
        let creatures: Vec<(i64, String)> = self
            .creature_names()
            .into_iter()
            .enumerate()
            .map(|(i, n)| (i as i64 + 1, n))
            .collect();
        m.insert("creature".into(), creatures);
        for (kind, file, base) in [
            ("object", "objects.cfg", "object"),
            ("room", "terrain.cfg", "room"),
            ("slab", "terrain.cfg", "slab"),
            ("spell", "magic.cfg", "spell"),
            ("shot", "magic.cfg", "shot"),
            ("power", "magic.cfg", "power"),
            ("special", "magic.cfg", "special"),
            ("trap", "trapdoor.cfg", "trap"),
            ("door", "trapdoor.cfg", "door"),
            ("instance", "creature.cfg", "instance"),
            ("job", "creature.cfg", "job"),
            ("angerjob", "creature.cfg", "angerjob"),
            ("attackpref", "creature.cfg", "attackpref"),
            ("state", "crstates.cfg", "state"),
            ("lens", "lenses.cfg", "lens"),
            ("cube", "cubes.cfg", "cube"),
            ("process", "keepcompp.cfg", "process"),
            ("check", "keepcompp.cfg", "check"),
            ("event", "keepcompp.cfg", "event"),
            ("computer", "keepcompp.cfg", "computer"),
        ] {
            let v = self.numbered_cfg(file, base);
            m.insert(kind.into(), v);
        }
        for (kind, base) in [
            ("effect", "effect"),
            ("effect_element", "effectElement"),
            ("effect_generator", "effectGenerator"),
        ] {
            let v = self.numbered_toml("effects.toml", base, "Name");
            m.insert(kind.into(), v);
        }
        let tex = self.numbered_toml("textureanim.toml", "texture", "name");
        m.insert("texture".into(), tex);
        let sounds: Vec<(i64, String)> = self
            .cfg("sounds.cfg")
            .map(|c| {
                c.keys("sounds")
                    .into_iter()
                    .map(|k| {
                        let id = c
                            .at_target("sounds", &k)
                            .and_then(|p| {
                                p.value
                                    .split_whitespace()
                                    .next()
                                    .and_then(|t| t.parse().ok())
                            })
                            .unwrap_or(-1);
                        (id, k)
                    })
                    .collect()
            })
            .unwrap_or_default();
        m.insert("sound".into(), sounds);
        m
    }
}
