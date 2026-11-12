//! Configuration editors (`kfx-config-ui`) bound to the open level's layer stack, the
//! "Layers & mods" window (active layers with provenance, the mods of `load_order.cfg`) and the
//! "Level files" list.
//!
//! The editor writes through [`EditorFs`], which keeps a copy of every file it is about to
//! overwrite or delete in the editor's backups folder (once per file and session). Unsaved
//! edits are previewed live: the document's `GameData` is replaced by
//! `ConfigSession::effective_gamedata` and the view, palettes, sprites and script constants are
//! rebuilt. Slabs whose slab set, columns or terrain entry changed are listed and can be
//! regenerated per kind (one undo step each).
use super::*;
use kfx_config_ui::{ConfigEditor, ConfigEvent, ConfigFs, ConfigSession, Layer, LayerKind};
use kfx_map::gamedata::SLABSETS_PER_SLAB;
use kfx_map::mods::{LayerOrigin, Stage};
use kfx_prefab::i18n::{trd, trf, trn};
use std::io;

/// `ConfigFs` over `std::fs` that backs up a file before its first overwrite / removal in a
/// session (the editor's [`crate::files::SessionBackups`]). A backup that fails makes the write
/// fail: the file is not touched.
#[derive(Default)]
pub struct EditorFs {
    pub backups: crate::files::SessionBackups,
}

impl EditorFs {
    fn backup(&self, path: &Path) -> io::Result<()> {
        self.backups.ensure(path).map(|_| ()).map_err(|e| {
            io::Error::other(trf(
                "kfx_editor.not_written_e",
                "not written: {e}",
                &[("e", &e)],
            ))
        })
    }
}

impl ConfigFs for EditorFs {
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        kfx_config_ui::StdFs.read(path)
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.backup(path)?;
        kfx_config_ui::StdFs.write(path, bytes)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        self.backup(path)?;
        kfx_config_ui::StdFs.remove(path)
    }
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<String>> {
        kfx_config_ui::StdFs.list_dir(dir)
    }
}

/// [`EditorFs`] without backups (game tests).
#[cfg(all(test, unix))]
pub type EditorFsForTest = EditorFs;

/// The open configuration editor.
pub struct ConfigState {
    pub editor: ConfigEditor,
    pub open: bool,
    /// Docked beside the map (else its own window).
    pub docked: bool,
    /// The level the editor is bound to.
    pub folder: PathBuf,
    pub level: u32,
    /// Slab kinds on the map whose generation changed with the edits: (kind, slabs).
    pub affected: Vec<(u16, usize)>,
    /// Regenerate the affected slabs on every edit.
    pub live_regen: bool,
    pub message: Option<String>,
}

impl Drop for ConfigState {
    fn drop(&mut self) {
        // the private scratch folder of the live preview
        let _ = std::fs::remove_dir_all(&self.editor.gd.scratch);
    }
}

/// The `kfx-config-ui` layer stack for a document's stack: base, mods, campaign, map, in load
/// order (a mod with several folders gets one layer per folder).
pub fn ui_stack(st: &kfx_map::mods::LayerStack, level: u32) -> kfx_config_ui::LayerStack {
    let creature_dir = |o: &LayerOrigin| {
        st.creatures
            .iter()
            .find(|(c, l)| c == o && l.prefix.is_empty())
            .map(|(_, l)| l.dir.clone())
    };
    let mut out = vec![];
    let mut seen_mod: HashMap<String, usize> = HashMap::new();
    for (o, l) in &st.config {
        let layer = match (o.stage, &o.mod_name) {
            (Stage::Base, _) => Layer::base(l.dir.clone(), creature_dir(o)),
            (Stage::Campaign, _) => Layer::campaign(Some(l.dir.clone()), creature_dir(o)),
            (Stage::Map, _) => Layer::map(l.dir.clone(), level),
            (_, Some(m)) => {
                let n = seen_mod.entry(m.clone()).or_default();
                *n += 1;
                let name = if *n == 1 {
                    m.clone()
                } else {
                    format!("{m} #{n}")
                };
                let cr = if *n == 1 {
                    l.dir.parent().map(|r| r.join("creatrs"))
                } else {
                    None
                };
                Layer {
                    kind: LayerKind::Mod(name),
                    dir: Some(l.dir.clone()),
                    creature_dir: cr,
                    prefix: l.prefix.clone(),
                }
            }
            _ => continue,
        };
        out.push(layer);
    }
    kfx_config_ui::LayerStack { layers: out }
}

/// A per-kind signature of what slab generation reads: the terrain entry, the 28 slab sets
/// (resolved columns) and the slab objects.
fn kind_signature(gd: &GameData, k: usize) -> String {
    let sets = gd
        .slabset
        .get(k * SLABSETS_PER_SLAB..(k + 1) * SLABSETS_PER_SLAB)
        .unwrap_or(&[]);
    let objs = gd
        .slab_objects
        .get(k * SLABSETS_PER_SLAB..(k + 1) * SLABSETS_PER_SLAB)
        .unwrap_or(&[]);
    format!(
        "{:?}|{:?}|{:?}",
        gd.slabs.get(k).map(|s| (
            s.block_flags,
            s.wlb_type,
            s.category,
            s.animated,
            s.wibble,
            s.fill_style
        )),
        sets,
        objs
    )
}

/// Slab kinds on the map whose generation differs between `a` and `b`, with their counts.
pub fn affected_kinds(d: &MapDoc, a: &GameData, b: &GameData) -> Vec<(u16, usize)> {
    let mut count: std::collections::BTreeMap<u16, usize> = Default::default();
    for (i, &k) in d.kinds.iter().enumerate() {
        if d.custom_cells[i] == 0 {
            *count.entry(k).or_default() += 1;
        }
    }
    count
        .into_iter()
        .filter(|(k, _)| kind_signature(a, *k as usize) != kind_signature(b, *k as usize))
        .collect()
}

impl EditorApp {
    /// The mods that apply: none when ignored in the preferences.
    pub fn active_mods(&self) -> kfx_map::mods::ModsConfig {
        if self.settings.ignore_mods {
            return Default::default();
        }
        self.mods
            .as_ref()
            .map(|m| m.for_game(false))
            .unwrap_or_default()
    }

    /// Game data for a layer stack (cached by its layers).
    pub(super) fn gamedata_stack(
        &mut self,
        st: &kfx_map::mods::LayerStack,
    ) -> Result<Arc<GameData>, String> {
        let key = st.to_config_layers();
        if let Some(g) = self.gd_cache.get(&key) {
            return Ok(g.clone());
        }
        let g = Arc::new(GameData::load_stack(st).map_err(|e| {
            trf(
                "kfx_editor.game_configuration_e",
                "game configuration: {e}",
                &[("e", &e)],
            )
        })?);
        self.gd_cache.insert(key, g.clone());
        Ok(g)
    }

    /// Open the configuration editor on the open level (on `tab` when given).
    pub fn open_config_editor(&mut self, tab: Option<kfx_config_ui::ui::Tab>) {
        let Some(d) = &self.doc else {
            self.status = trd(
                "kfx_editor.open_a_map_first_the_configuration_editor_works",
                "Open a map first: the configuration editor works on its layers",
            );
            return;
        };
        let (folder, level) = (d.doc.folder.clone(), d.doc.level);
        let rebind = self
            .config
            .as_ref()
            .is_none_or(|c| c.folder != folder || c.level != level);
        if rebind {
            if self
                .config
                .as_ref()
                .is_some_and(|c| c.editor.session.is_dirty())
            {
                self.status = trd(
                    "kfx_editor.the_configuration_editor_has_unsaved_edits_of_an",
                    "The configuration editor has unsaved edits of another level: \
                    save or discard them first",
                );
                if let Some(c) = self.config.as_mut() {
                    c.open = true;
                }
                return;
            }
            let layers = ui_stack(&d.stack, level);
            let fs = EditorFs {
                backups: if self.persist {
                    self.session_backups.clone()
                } else {
                    crate::files::SessionBackups::disabled()
                },
            };
            let mut session = ConfigSession::new(Arc::new(fs), layers);
            if let Some(i) = session.layers.index_of(&LayerKind::Map(level)) {
                let _ = session.set_target(i);
            }
            let _ = session.take_events();
            let install = self.install().map(Path::to_path_buf);
            let assets = install.and_then(|i| {
                let search = TextureSearch {
                    level_dir: Some(folder.clone()),
                    ..TextureSearch::new(self.assets.as_ref()?.install.data_dir())
                };
                kfx_config_ui::assets::Assets::load_install(&i, Some(search), Some(level), &[], &[])
                    .ok()
            });
            let scratch = std::env::temp_dir().join(format!(
                "kfx-editor-cfg-{}-{}",
                std::process::id(),
                level
            ));
            self.config = Some(ConfigState {
                editor: ConfigEditor::new(session, assets, scratch),
                open: true,
                docked: false,
                folder,
                level,
                affected: vec![],
                live_regen: false,
                message: None,
            });
        }
        if let Some(c) = self.config.as_mut() {
            c.open = true;
            if let Some(t) = tab {
                c.editor.tab = t;
            }
        }
    }

    /// Make a layer of the config editor the target (`map`, `campaign`, `base`, or a mod name).
    pub fn set_config_target(&mut self, which: &str) -> Result<(), String> {
        let c = self
            .config
            .as_mut()
            .ok_or("the configuration editor is not open")?;
        let s = &mut c.editor.session;
        let i = s
            .layers
            .layers
            .iter()
            .position(|l| match (&l.kind, which) {
                (LayerKind::Map(_), "map")
                | (LayerKind::Campaign, "campaign")
                | (LayerKind::Base, "base") => true,
                (LayerKind::Mod(n), w) => n.eq_ignore_ascii_case(w),
                _ => false,
            })
            .ok_or_else(|| format!("no layer {which}"))?;
        if i == 0 {
            s.allow_base = true;
        }
        s.set_target(i)
    }

    /// Add a mod folder (`<install>/mods/<name>`) to the editor's layers as a target, even when
    /// it is not in `load_order.cfg` (or mods are ignored).
    pub fn edit_mod_folder(&mut self, name: &str) -> Result<(), String> {
        let install = self.install().map(Path::to_path_buf).ok_or("no install")?;
        let root = kfx_formats::testutil::resolve_ci(&install, &format!("mods/{name}"))
            .filter(|p| p.is_dir())
            .ok_or_else(|| format!("no mod folder {name}"))?;
        self.open_config_editor(None);
        let c = self.config.as_mut().ok_or("no map")?;
        let s = &mut c.editor.session;
        if s.layers
            .index_of(&LayerKind::Mod(name.to_string()))
            .is_none()
        {
            if s.is_dirty() {
                return Err(trd(
                    "kfx_editor.save_or_discard_the_unsaved_configuration_edits",
                    "save or discard the unsaved configuration edits first",
                ));
            }
            s.layers.insert_mod(1, name, &root);
        }
        let i = s
            .layers
            .index_of(&LayerKind::Mod(name.to_string()))
            .ok_or("mod layer")?;
        s.set_target(i)
    }

    /// Per-frame: the configuration editor window / panel and its events.
    pub(super) fn config_ui(&mut self, ctx: &egui::Context) {
        let Some(c) = self.config.as_mut() else {
            return;
        };
        if !c.open || c.docked {
            return;
        }
        let mut open = true;
        let mut events = vec![];
        let title = trf(
            "kfx_editor.configuration_map_level",
            "Configuration - map{level}",
            &[("level", &format!("{:05}", c.level))],
        );
        egui::Window::new(title)
            .id(egui::Id::unique("config-editor"))
            .open(&mut open)
            .default_size([1100.0, 760.0])
            .show(ctx, |ui| {
                events = Self::config_body(c, ui);
            });
        c.open = open;
        self.handle_config_events(events);
    }

    /// The docked configuration panel (before the central panel).
    pub(super) fn config_dock_ui(&mut self, ui: &mut egui::Ui) {
        let Some(c) = self.config.as_mut() else {
            return;
        };
        if !c.open || !c.docked {
            return;
        }
        let mut events = vec![];
        let max = super::side_panel_max(ui);
        let w = self.settings.layout.config_dock_w.min(max);
        let r = egui::Panel::right("config-dock")
            .resizable(true)
            .default_size(w)
            .size_range(320f32.min(max)..=max)
            .show(ui, |ui| {
                events = Self::config_body(c, ui);
            });
        self.settings.layout.config_dock_w = r.response.rect.width();
        self.handle_config_events(events);
    }

    /// Per-frame: the creature stats comparison window (Configuration > Creature stats).
    pub(super) fn creature_stats_ui(&mut self, ctx: &egui::Context) {
        let Some(c) = self.config.as_mut() else {
            return;
        };
        let f = c.editor.creature_stats_window(ctx);
        if f.show_editor {
            c.open = true;
        }
        self.handle_config_events(f.events);
    }

    /// Open the creature stats comparison window (binds the configuration editor to the level
    /// without showing it).
    pub fn open_creature_stats(&mut self) {
        let was_open = self.config.as_ref().is_some_and(|c| c.open);
        self.open_config_editor(None);
        if let Some(c) = self.config.as_mut() {
            c.editor.stats_open = true;
            c.open = was_open;
        }
    }

    fn config_body(c: &mut ConfigState, ui: &mut egui::Ui) -> Vec<ConfigEvent> {
        ui.horizontal(|ui| {
            if ui
                .small_button(if c.docked {
                    trd("kfx_editor.window_e36e", "🗗 Window")
                } else {
                    trd("kfx_editor.dock", "⬅ Dock")
                })
                .on_hover_text(trd(
                    "kfx_editor.own_window_docked_beside_the_map",
                    "Own window / docked beside the map",
                ))
                .clicked()
            {
                c.docked = !c.docked;
            }
            if c.docked
                && ui
                    .small_button("✖")
                    .on_hover_text(trd("kfx_editor.close", "Close"))
                    .clicked()
            {
                c.open = false;
            }
            ui.checkbox(
                &mut c.live_regen,
                trd(
                    "kfx_editor.live_regenerate_affected_slabs",
                    "Live: regenerate affected slabs",
                ),
            )
            .on_hover_text(trd(
                "kfx_editor.regenerate_the_slabs_whose_definition_changed_af",
                "Regenerate the slabs whose definition changed after every edit (one \
                undo step per edit)",
            ));
            if let Some(m) = &c.message {
                ui.label(RichText::new(m).small().color(GOLD));
            }
        });
        if !c.affected.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.slabs_on_the_map_use_changed_definitions",
                        "Slabs on the map use changed definitions:",
                    ))
                    .color(Color32::from_rgb(240, 190, 80)),
                );
                for &(k, n) in &c.affected.clone() {
                    if ui
                        .button(trf(
                            "kfx_editor.regenerate_all_x_n",
                            "Regenerate all {x} ({n})",
                            &[
                                (
                                    "x",
                                    &c.editor
                                        .session
                                        .names("slab")
                                        .iter()
                                        .find(|x| x.0 == k as i64)
                                        .map(|x| x.1.clone())
                                        .unwrap_or_else(|| format!("slab {k}")),
                                ),
                                ("n", &n),
                            ],
                        ))
                        .clicked()
                    {
                        c.message = Some(format!("regen:{k}"));
                    }
                }
                if c.affected.len() > 1
                    && ui
                        .button(trd("kfx_editor.regenerate_them_all", "Regenerate them all"))
                        .clicked()
                {
                    c.message = Some("regen:all".into());
                }
            });
        }
        ui.separator();
        c.editor.show(ui)
    }

    fn handle_config_events(&mut self, events: Vec<ConfigEvent>) {
        // a regenerate request from the offer bar
        let req = self
            .config
            .as_mut()
            .and_then(|c| c.message.take_if(|m| m.starts_with("regen:")));
        if let Some(r) = req {
            let kinds: Vec<u16> = match r.trim_start_matches("regen:") {
                "all" => self
                    .config
                    .as_ref()
                    .map(|c| c.affected.iter().map(|x| x.0).collect())
                    .unwrap_or_default(),
                k => k.parse().ok().into_iter().collect(),
            };
            self.regenerate_kinds(&kinds);
        }
        let mut reload = false;
        for e in events {
            match e {
                ConfigEvent::Changed { .. } | ConfigEvent::TargetChanged { .. } => reload = true,
                ConfigEvent::Saved { path, deleted, .. } => {
                    self.gd_cache.clear();
                    reload = true;
                    // our own write is not a change by another program (`check_external_changes`)
                    self.refresh_fingerprint_for(std::slice::from_ref(&path));
                    self.status = if deleted {
                        trf(
                            "kfx_editor.removed_path",
                            "Removed {path}",
                            &[("path", &path.display())],
                        )
                    } else {
                        trf(
                            "kfx_editor.saved_path",
                            "Saved {path}",
                            &[("path", &path.display())],
                        )
                    };
                }
                ConfigEvent::RequestOpenFile { path } => {
                    if let Err(e) = crate::files::open_path(&path) {
                        self.status = e;
                    }
                }
            }
        }
        if reload {
            self.apply_config_preview();
        }
    }

    /// Put the configuration editor's effective game data (with its unsaved edits) into the
    /// document and rebuild what depends on it.
    pub fn apply_config_preview(&mut self) {
        let Some(c) = self.config.as_mut() else {
            return;
        };
        let gd = match c.editor.gamedata() {
            Ok(g) => Arc::new(g.clone()),
            Err(e) => {
                c.message = Some(trf("kfx_editor.preview_e", "preview: {e}", &[("e", &e)]));
                return;
            }
        };
        let live = c.live_regen;
        let Some(d) = self.doc.as_mut() else { return };
        if d.doc.folder != c.folder || d.doc.level != c.level {
            return;
        }
        let old = d.gd.clone();
        let affected = affected_kinds(&d.doc, &old, &gd);
        // keep earlier offers that still differ from what is on disk
        let mut all = c.affected.clone();
        for a in affected {
            if !all.iter().any(|x| x.0 == a.0) {
                all.push(a);
            }
        }
        c.affected = all;
        self.set_gamedata(gd);
        if live {
            let kinds: Vec<u16> = self
                .config
                .as_ref()
                .map(|c| c.affected.iter().map(|x| x.0).collect())
                .unwrap_or_default();
            if !kinds.is_empty() {
                self.regenerate_kinds(&kinds);
            }
        }
    }

    /// Replace the document's game data and rebuild cubes, view, palettes, sprites, script
    /// constants and the gold rules.
    pub fn set_gamedata(&mut self, gd: Arc<GameData>) {
        let Some(d) = self.doc.as_mut() else { return };
        d.gd = gd;
        let p = document::DecorPrefs {
            enabled: self.settings.decoration,
            families: self.settings.decor_families.clone(),
        };
        d.set_decor_prefs(&p);
        self.cubes = CubeFaces::from_gamedata(&self.doc.as_ref().unwrap().gd);
        self.rebuild_view();
        self.load_sprites();
        self.build_thing_palette();
        self.gold_rules = self
            .doc
            .as_ref()
            .map(|d| things::GoldRules::load(&d.layers))
            .unwrap_or_default();
        self.attach_scripts();
        self.revalidate();
    }

    /// Regenerate every slab of `kinds` (columns, decoration, slab objects) as one undo step.
    pub fn regenerate_kinds(&mut self, kinds: &[u16]) {
        let Some(d) = self.doc.as_mut() else { return };
        if kinds.is_empty() {
            return;
        }
        let gd = d.gd.clone();
        let names: Vec<String> = kinds
            .iter()
            .map(|&k| gd.slab_name(k as usize).unwrap_or("?").to_string())
            .collect();
        let name = trf(
            "kfx_editor.regenerate_names",
            "Regenerate {names}",
            &[("names", &names.join(", "))],
        );
        d.end_stroke();
        let dims = d.dims();
        let set: HashSet<u16> = kinds.iter().copied().collect();
        let what = document::Regen {
            columns: true,
            decoration: d.decorator.enabled,
            objects: true,
            liquid: false,
            doodads: false,
        };
        let mut n = 0;
        let (_, rf) = d.edit(&name, |m, g, dec| {
            let cells: Vec<(i32, i32)> = (0..dims.slabs())
                .filter(|&i| set.contains(&m.kinds[i]) && m.custom_cells[i] == 0)
                .map(|i| ((i % dims.w as usize) as i32, (i / dims.w as usize) as i32))
                .collect();
            n = cells.len();
            for (x, y) in cells {
                document::regenerate(m, g, dec, SlabRect::single(x, y), what);
            }
        });
        self.after_change(rf);
        if let Some(c) = self.config.as_mut() {
            c.affected.retain(|a| !set.contains(&a.0));
        }
        self.status = trf(
            "kfx_editor.name_n_slabs_one_undo_step",
            "{name}: {n} slabs (one undo step)",
            &[("name", &name), ("n", &n)],
        );
    }

    // ------------------------------------------------------------------ layers & mods

    /// Status bar summary of the active layers, e.g. `base + 2 mods + campaign + map`.
    pub fn layers_summary(&self) -> Option<String> {
        let d = self.doc.as_ref()?;
        let mut parts: Vec<String> = vec![];
        let mut mods: Vec<String> = vec![];
        for (o, _) in &d.stack.config {
            match (&o.mod_name, o.stage) {
                (Some(m), _) => {
                    if !mods.contains(m) {
                        mods.push(m.clone());
                    }
                }
                (None, Stage::Base) => parts.push("base".into()),
                (None, Stage::Campaign) => parts.push("campaign".into()),
                (None, _) => parts.push("map".into()),
            }
        }
        if !mods.is_empty() {
            parts.insert(
                1.min(parts.len()),
                trn(
                    "kfx_editor.n_mods",
                    mods.len() as i64,
                    "{n} mod",
                    "{n} mods",
                    &[],
                ),
            );
        }
        let ignored =
            self.settings.ignore_mods && self.mods.as_ref().is_some_and(|m| !m.is_empty());
        Some(trf(
            "kfx_editor.layers_parts_ignored",
            "layers: {parts}{ignored}",
            &[
                ("parts", &parts.join(" + ")),
                (
                    "ignored",
                    &(if ignored {
                        trd("kfx_editor.mods_ignored", " (mods ignored)")
                    } else {
                        String::new()
                    }),
                ),
            ],
        ))
    }

    /// Mod folders of the install (`mods/*/`).
    pub fn mod_folders(&self) -> Vec<String> {
        let Some(i) = self.install() else {
            return vec![];
        };
        let Some(d) = kfx_formats::testutil::resolve_ci(i, "mods").filter(|p| p.is_dir()) else {
            return vec![];
        };
        let mut v: Vec<String> = std::fs::read_dir(d)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    /// Reload `mods/load_order.cfg` and reopen the map with the new layers.
    pub fn reload_mods(&mut self) {
        self.mods = self.install().and_then(kfx_map::mods::ModsConfig::load);
        self.gd_cache.clear();
        if let Some(p) = self
            .doc
            .as_ref()
            .filter(|_| self.doc_on_disk)
            .map(|d| d.slb_path())
        {
            if self.anything_dirty() {
                self.status = trd(
                    "kfx_editor.mods_changed_save_or_reload_the_map_to_apply_the",
                    "Mods changed: save or reload the map to apply them",
                );
            } else if let Err(e) = self.open_map(&p) {
                self.status = trf(
                    "kfx_editor.could_not_reload_e",
                    "Could not reload: {e}",
                    &[("e", &e)],
                );
            }
        }
    }

    pub(super) fn layers_window(&mut self, ctx: &egui::Context) {
        if !self.show_layers {
            return;
        }
        let mut open = true;
        let mut act: Option<(&'static str, String)> = None;
        let unset_hint = trd(
            "kfx_editor.only_values_set_above_the_base_are_listed_type_a",
            "Only values set above the base are listed; type a \
            filter to see others.",
        );
        egui::Window::new(trd("kfx_editor.layers_mods", "Layers & mods"))
            .open(&mut open)
            .default_size([760.0, 560.0])
            .show(ctx, |ui| {
                let mut ignore = self.settings.ignore_mods;
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut ignore, trd("kfx_editor.ignore_mods", "Ignore mods"))
                        .on_hover_text(trd(
                            "kfx_editor.edit_as_the_game_plays_without_mods_it_ignores_t",
                            "Edit as the game plays without mods (it ignores them in \
                            multiplayer games)",
                        ))
                        .changed()
                    {
                        act = Some(("ignore", ignore.to_string()));
                    }
                    if ui
                        .button(trd(
                            "kfx_editor.reload_load_order_cfg",
                            "Reload load_order.cfg",
                        ))
                        .clicked()
                    {
                        act = Some(("reload", String::new()));
                    }
                });
                if let Some(d) = &self.doc {
                    if d.pack_cfg.as_ref().is_some_and(|(_, k)| {
                        *k == kfx_formats::campaign::PackKind::MultiplayerMappack
                    }) && !self.settings.ignore_mods
                    {
                        ui.colored_label(
                            Color32::from_rgb(240, 190, 80),
                            trd(
                                "kfx_editor.a_multiplayer_map_the_game_ignores_mods_in_multi",
                                "A multiplayer map: the game ignores mods in multiplayer \
                                games (only single-player tests use them).",
                            ),
                        );
                    }
                }
                match &self.mods {
                    None => {
                        ui.label(
                            RichText::new(trd(
                                "kfx_editor.no_mods_load_order_cfg_in_the_install_no_mods_ar",
                                "No mods/load_order.cfg in the install: no mods are loaded.",
                            ))
                            .color(TEXT_DIM),
                        );
                    }
                    Some(m) => {
                        egui::Grid::new("mods-grid")
                            .striped(true)
                            .num_columns(2)
                            .show(ui, |ui| {
                                for (b, list) in [
                                    ("after_base", &m.after_base),
                                    ("after_campaign", &m.after_campaign),
                                    ("after_map", &m.after_map),
                                ] {
                                    ui.label(RichText::new(b).strong());
                                    ui.label(if list.is_empty() {
                                        "-".to_string()
                                    } else {
                                        list.join(", ")
                                    });
                                    ui.end_row();
                                }
                            });
                        for w in &m.warnings {
                            ui.colored_label(Color32::from_rgb(240, 190, 80), w);
                        }
                    }
                }
                ui.separator();
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.active_configuration_layers_load_order_later_win",
                        "Active configuration layers (load order, later wins)",
                    ))
                    .color(GOLD),
                );
                if let Some(d) = &self.doc {
                    egui::ScrollArea::vertical()
                        .id_salt("layers-scroll")
                        .max_height(200.0)
                        .show(ui, |ui| {
                            egui::Grid::new("layers-grid")
                                .striped(true)
                                .num_columns(3)
                                .show(ui, |ui| {
                                    for (o, l) in &d.stack.config {
                                        ui.label(RichText::new(o.to_string()).color(
                                            match (&o.mod_name, o.stage) {
                                                (Some(_), _) => Color32::from_rgb(200, 140, 255),
                                                (None, Stage::Base) => TEXT_DIM,
                                                (None, Stage::Campaign) => {
                                                    Color32::from_rgb(120, 200, 255)
                                                }
                                                _ => Color32::from_rgb(140, 230, 140),
                                            },
                                        ));
                                        ui.label(
                                            RichText::new(format!(
                                                "{}{}*",
                                                l.dir.display(),
                                                if l.prefix.is_empty() {
                                                    "/".to_string()
                                                } else {
                                                    format!("/{}", l.prefix)
                                                }
                                            ))
                                            .monospace()
                                            .small(),
                                        );
                                        let n = [
                                            "rules.cfg",
                                            "terrain.cfg",
                                            "objects.cfg",
                                            "trapdoor.cfg",
                                            "magic.cfg",
                                            "creature.cfg",
                                            "cubes.cfg",
                                            "slabset.toml",
                                            "columnset.toml",
                                        ]
                                        .iter()
                                        .filter(|f| l.file(f).is_some())
                                        .count();
                                        ui.label(
                                            RichText::new(trf(
                                                "kfx_editor.n_config_files",
                                                "{n} config files",
                                                &[("n", &n)],
                                            ))
                                            .small(),
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(trd("kfx_editor.provenance_of", "Provenance of"))
                                .color(GOLD),
                        );
                        egui::ComboBox::from_id_salt("prov-file")
                            .selected_text(self.prov_file.clone())
                            .show_ui(ui, |ui| {
                                for f in kfx_config_ui::session::CFG_FILES
                                    .iter()
                                    .chain(kfx_config_ui::session::TOML_FILES)
                                {
                                    ui.selectable_value(&mut self.prov_file, f.to_string(), *f);
                                }
                            });
                        ui.add(
                            egui::TextEdit::singleline(&mut self.prov_filter)
                                .hint_text(trd("kfx_editor.filter_keys", "filter keys"))
                                .desired_width(160.0),
                        );
                    });
                    let lc = if self.prov_file.ends_with(".toml") {
                        d.gd.layered_toml(&self.prov_file)
                    } else {
                        d.gd.layered_cfg(&self.prov_file)
                    };
                    match lc {
                        Ok(lc) => {
                            ui.label(
                                RichText::new(
                                    lc.layers
                                        .iter()
                                        .map(|l| format!("{} ({})", l.origin, l.path.display()))
                                        .collect::<Vec<_>>()
                                        .join("  ->  "),
                                )
                                .small()
                                .color(TEXT_DIM),
                            );
                            let q = self.prov_filter.to_ascii_lowercase();
                            egui::ScrollArea::vertical()
                                .id_salt("prov-scroll")
                                .max_height(240.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    egui::Grid::new("prov-grid")
                                        .striped(true)
                                        .num_columns(3)
                                        .show(ui, |ui| {
                                            let mut shown = 0;
                                            for s in &lc.sections {
                                                for (k, v) in &s.entries {
                                                    let overridden = v.layer > 0;
                                                    if !q.is_empty()
                                                        && !format!("{} {k}", s.name)
                                                            .to_ascii_lowercase()
                                                            .contains(&q)
                                                    {
                                                        continue;
                                                    }
                                                    if q.is_empty() && !overridden {
                                                        continue;
                                                    }
                                                    shown += 1;
                                                    if shown > 400 {
                                                        break;
                                                    }
                                                    ui.label(
                                                        RichText::new(format!("[{}] {k}", s.name))
                                                            .monospace()
                                                            .small(),
                                                    );
                                                    ui.label(
                                                        RichText::new(&v.value).monospace().small(),
                                                    );
                                                    let from = lc
                                                        .layers
                                                        .get(v.layer)
                                                        .map(|l| l.origin.to_string())
                                                        .unwrap_or_default();
                                                    let hist: Vec<String> = v
                                                        .overridden
                                                        .iter()
                                                        .map(|(i, val)| {
                                                            format!(
                                                                "{}: {val}",
                                                                lc.layers
                                                                    .get(*i)
                                                                    .map(|l| l.origin.to_string())
                                                                    .unwrap_or_default()
                                                            )
                                                        })
                                                        .collect();
                                                    ui.label(
                                                        RichText::new(from).small().color(GOLD),
                                                    )
                                                    .on_hover_text(if hist.is_empty() {
                                                        trd(
                                                            "kfx_editor.only_set_here",
                                                            "only set here",
                                                        )
                                                    } else {
                                                        trf(
                                                            "kfx_editor.overrides_hist",
                                                            "overrides\n{hist}",
                                                            &[("hist", &hist.join("\n"))],
                                                        )
                                                    });
                                                    ui.end_row();
                                                }
                                            }
                                        });
                                });
                            if q.is_empty() {
                                ui.label(
                                    RichText::new(unset_hint.as_str()).small().color(TEXT_DIM),
                                );
                            }
                        }
                        Err(e) => {
                            ui.colored_label(Color32::LIGHT_RED, e.to_string());
                        }
                    }
                } else {
                    ui.label(
                        RichText::new(trd("kfx_editor.no_map_open", "No map open")).color(TEXT_DIM),
                    );
                }
                ui.separator();
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.mod_folders_of_the_install",
                        "Mod folders of the install",
                    ))
                    .color(GOLD),
                );
                ui.horizontal_wrapped(|ui| {
                    let folders = self.mod_folders();
                    if folders.is_empty() {
                        ui.label(
                            RichText::new(trd("kfx_editor.none_557b", "none")).color(TEXT_DIM),
                        );
                    }
                    for m in folders {
                        if ui
                            .button(trf(
                                "kfx_editor.edit_configs_of_m",
                                "Edit configs of {m}",
                                &[("m", &m)],
                            ))
                            .on_hover_text(trd(
                                "kfx_editor.open_the_configuration_editor_with_this_mod_fold",
                                "Open the configuration editor with this mod folder as the \
                                target layer",
                            ))
                            .clicked()
                        {
                            act = Some(("edit-mod", m));
                        }
                    }
                });
            });
        self.show_layers = open;
        match act {
            Some(("ignore", v)) => {
                self.settings.ignore_mods = v == "true";
                self.save_settings();
                self.reload_mods();
            }
            Some(("reload", _)) => self.reload_mods(),
            Some(("edit-mod", m)) => {
                if let Err(e) = self.edit_mod_folder(&m) {
                    self.status = e;
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ level files

    /// Every file of the open level: `(path, size, role)`.
    pub fn level_file_list(&self) -> Vec<(PathBuf, u64, &'static str)> {
        let Some(d) = &self.doc else { return vec![] };
        let mut v: Vec<(PathBuf, u64, &'static str)> =
            document::level_files(&d.doc.folder, d.doc.level)
                .into_iter()
                .map(|p| {
                    let name = p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let ext = document::level_file_ext(&name, d.doc.level).unwrap_or_default();
                    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    (p, size, document::level_file_role(&ext))
                })
                .collect();
        if let Some((cfg, _)) = &d.pack_cfg {
            v.push((
                cfg.clone(),
                std::fs::metadata(cfg).map(|m| m.len()).unwrap_or(0),
                "campaign / map pack file",
            ));
        }
        v
    }

    pub(super) fn level_files_ui(&mut self, ui: &mut egui::Ui) {
        if self.doc.is_none() {
            ui.label(RichText::new(trd("kfx_editor.no_map_open", "No map open")).color(TEXT_DIM));
            return;
        }
        let list = self.level_file_list();
        ui.label(
            RichText::new(trf(
                "kfx_editor.list_files_belong_to_this_level",
                "{list} files belong to this level",
                &[("list", &list.len())],
            ))
            .small()
            .color(TEXT_DIM),
        );
        let mut act: Option<(bool, PathBuf)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("level-files")
                    .striped(true)
                    .num_columns(4)
                    .show(ui, |ui| {
                        for (p, size, role) in &list {
                            ui.label(
                                RichText::new(p.file_name().unwrap_or_default().to_string_lossy())
                                    .monospace()
                                    .small(),
                            )
                            .on_hover_text(p.display().to_string());
                            ui.label(
                                RichText::new(if *size >= 10240 {
                                    trf(
                                        "kfx_editor.size_kib",
                                        "{size} KiB",
                                        &[("size", &(size / 1024))],
                                    )
                                } else {
                                    trf("kfx_editor.size_b", "{size} B", &[("size", &size)])
                                })
                                .small(),
                            );
                            ui.label(RichText::new(*role).small().color(TEXT_DIM));
                            ui.horizontal(|ui| {
                                if ui
                                    .small_button(trd("kfx_editor.open_70e9", "Open"))
                                    .on_hover_text(trd(
                                        "kfx_editor.open_with_the_system_s_program",
                                        "Open with the system's program",
                                    ))
                                    .clicked()
                                {
                                    act = Some((true, p.clone()));
                                }
                                if ui
                                    .small_button(trd("kfx_editor.reveal", "Reveal"))
                                    .on_hover_text(trd(
                                        "kfx_editor.show_the_folder",
                                        "Show the folder",
                                    ))
                                    .clicked()
                                {
                                    act = Some((false, p.clone()));
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
        if let Some((open, p)) = act {
            let r = if open {
                crate::files::open_path(&p)
            } else {
                crate::files::open_folder(p.parent().unwrap_or(Path::new(".")))
            };
            if let Err(e) = r {
                self.status = e;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config override is not written when its backup fails.
    #[test]
    fn failed_backup_stops_the_write() {
        let d = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/kfx-editorfs-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("map00001.rules.cfg");
        std::fs::write(&f, b"[game]\nGoldPerHoard = 1\n").unwrap();
        std::fs::write(d.join("blocker"), b"x").unwrap();
        let fs = EditorFs {
            backups: crate::files::SessionBackups::new(Some(d.join("blocker").join("backups"))),
        };
        assert!(fs.write(&f, b"[game]\nGoldPerHoard = 2\n").is_err());
        assert!(fs.remove(&f).is_err());
        assert_eq!(std::fs::read(&f).unwrap(), b"[game]\nGoldPerHoard = 1\n");
        let ok = EditorFs {
            backups: crate::files::SessionBackups::new(Some(d.join("backups"))),
        };
        ok.write(&f, b"[game]\nGoldPerHoard = 3\n").unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }
}
