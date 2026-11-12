//! Crash-recovery snapshots, external changes, the Backups window, recent map names.
use super::*;
use crate::files::{self, Backup, Snapshot, SnapshotInfo};
use crate::settings::Settings as S;
use kfx_prefab::i18n::{trd, trf};
use kfx_script::Span;

/// The Backups window.
pub struct BackupsWindow {
    pub list: Vec<Backup>,
    pub only_current: bool,
    pub confirm: Option<usize>,
    pub message: Option<String>,
}

impl EditorApp {
    // ------------------------------------------------------------------- snapshots

    /// Every `autosave_secs` while something is unsaved: write a crash-recovery snapshot.
    pub(super) fn autosave_tick(&mut self) {
        let secs = self.settings.autosave_secs;
        if secs == 0 || !self.persist || self.doc.is_none() || !self.anything_dirty() {
            return;
        }
        if self.last_autosave.elapsed() < Duration::from_secs(secs as u64) {
            self.ctx.request_repaint_after(
                Duration::from_secs(secs as u64) - self.last_autosave.elapsed(),
            );
            return;
        }
        self.last_autosave = Instant::now();
        if let Err(e) = self.write_snapshot() {
            self.status = trf(
                "kfx_editor.autosave_failed_e",
                "Autosave failed: {e}",
                &[("e", &e)],
            );
        }
    }

    /// Write the snapshot of the open map now.
    pub fn write_snapshot(&mut self) -> Result<PathBuf, String> {
        let root = S::autosave_dir().ok_or("no config directory")?;
        self.doc.as_ref().ok_or("no map")?;
        if let Some(d) = self.doc.as_mut() {
            d.end_stroke();
        }
        let d = self.doc.as_ref().unwrap();
        let scripts: Vec<(String, Vec<u8>)> = self
            .scripts
            .as_ref()
            .map(|s| {
                s.ws.dirty_tabs()
                    .into_iter()
                    .filter_map(|n| s.ws.editor(&n).map(|e| (n.clone(), e.text_bytes())))
                    .collect()
            })
            .unwrap_or_default();
        let info = SnapshotInfo {
            folder: d.doc.folder.clone(),
            level: d.doc.level,
            time: files::now(),
            map_dirty: d.is_dirty() || !self.doc_on_disk,
            scripts: scripts.iter().map(|s| s.0.clone()).collect(),
            new_map: !self.doc_on_disk,
        };
        let (gd, mut m) = (d.gd.clone(), d.doc.clone());
        let dims = d.dims();
        files::write_snapshot(
            &root,
            &info,
            |dir| {
                m.folder = dir.to_path_buf();
                m.save(&gd, dir, kfx_map::mapdoc::SaveColumns::Current)
                    .map_err(|e| e.to_string())?;
                let li = kfx_formats::levelinfo::LevelInfo {
                    mapsize: Some((dims.w as i32, dims.h as i32)),
                    ..Default::default()
                };
                Lof::new(&li)
                    .save(&dir.join(level_bin::file_name(info.level, "lof")))
                    .map_err(|e| e.to_string())
            },
            &scripts,
        )
    }

    /// Forget the snapshot of the open map (saved, or changes discarded).
    pub(super) fn drop_snapshot(&mut self) {
        if let (Some(root), Some(d)) = (S::autosave_dir(), &self.doc) {
            if self.persist {
                files::remove_snapshot(&root, &d.doc.folder, d.doc.level);
            }
        }
    }

    /// At start: offer the snapshots left by a crash.
    pub(super) fn find_snapshots(&mut self) {
        if !self.persist {
            return;
        }
        if let Some(root) = S::autosave_dir() {
            self.recovery = files::list_snapshots(&root);
        }
    }

    /// List the snapshots for the start-up prompt even when settings are not persisted
    /// (headless).
    pub fn show_recovery_prompt(&mut self) {
        if let Some(root) = S::autosave_dir() {
            self.recovery = files::list_snapshots(&root);
        }
    }

    /// Recover a snapshot as the open, unsaved document.
    pub fn recover(&mut self, s: &Snapshot) -> Result<(), String> {
        let install = self.install().map(Path::to_path_buf).ok_or("no install")?;
        let (folder, level) = (s.info.folder.clone(), s.info.level);
        let on_disk = level_bin::find_file(&folder, level, "slb").is_some();
        let doc = if s.info.map_dirty {
            let snap = document::locate(&install, &s.dir, level)?;
            let c = document::level_config(&install, &folder, level, &self.active_mods());
            let gd = self.gamedata_stack(&c.stack)?;
            let mut m = MapDoc::load(&s.dir, level, snap.dims, &gd).map_err(|e| e.to_string())?;
            m.folder = folder.clone();
            let mut d = Document::new(m, gd, c.stack, c.pack, false);
            d.layers = c.layers;
            d.pack_cfg = c.pack_cfg;
            d
        } else {
            let loc = document::locate_with(&install, &folder, level, &self.active_mods())?;
            let gd = self.gamedata_stack(&loc.stack)?;
            Document::load(&loc, gd)?
        };
        self.scripts = None;
        self.set_document(doc, on_disk);
        for name in &s.info.scripts {
            if let Ok(b) = std::fs::read(s.dir.join(name)) {
                self.set_script_text_dirty(name, &b);
            }
        }
        self.status = trf(
            "kfx_editor.recovered_map_level_from_time_not_saved_yet",
            "Recovered map{level} from {time} (not saved yet)",
            &[
                ("level", &format!("{:05}", level)),
                ("time", &files::pretty_time(&s.info.time)),
            ],
        );
        Ok(())
    }

    /// Give a script tab new text as an unsaved edit (recovery).
    pub fn set_script_text_dirty(&mut self, name: &str, bytes: &[u8]) {
        let Some(s) = self.scripts.as_mut() else {
            return;
        };
        let exists = s.ws.tab_infos().iter().any(|t| t.name == name && t.exists);
        if !exists {
            if name == s.ws.lua_name() {
                s.ws.create_lua_script(false, false);
            } else {
                s.ws.create_dk_script();
            }
        }
        let Some(e) = s.ws.editor_mut(name) else {
            return;
        };
        let len = e.text_bytes().len();
        let text: String = kfx_formats::cfg::latin1_decode(bytes).replace("\r\n", "\n");
        e.select_span(Span::new(0, len));
        e.insert_at_caret(&text);
        e.apply_pending();
        s.ws.invalidate();
    }

    pub(super) fn recovery_ui(&mut self, ctx: &egui::Context) {
        if self.recovery.is_empty() || self.assets.is_none() || self.first_run.open {
            return;
        }
        let mut act: Option<(usize, bool)> = None;
        let mut later = false;
        egui::Modal::new(egui::Id::unique("recovery")).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.heading(
                RichText::new(trd(
                    "kfx_editor.recover_unsaved_work",
                    "Recover unsaved work",
                ))
                .color(GOLD),
            );
            ui.label(trd(
                "kfx_editor.the_editor_did_not_close_normally_these_maps_had",
                "The editor did not close normally. These maps had unsaved changes:",
            ));
            for (i, s) in self.recovery.iter().enumerate() {
                ui.separator();
                ui.label(
                    RichText::new(trf(
                        "kfx_editor.map_level_in_folder",
                        "map{level} in {folder}",
                        &[
                            ("level", &format!("{:05}", s.info.level)),
                            ("folder", &s.info.folder.display()),
                        ],
                    ))
                    .strong(),
                );
                let what = match (s.info.map_dirty, s.info.scripts.is_empty()) {
                    (true, true) => "map".to_string(),
                    (true, false) => trf(
                        "kfx_editor.map_and_scripts",
                        "map and {scripts}",
                        &[("scripts", &s.info.scripts.join(", "))],
                    ),
                    (false, _) => s.info.scripts.join(", "),
                };
                ui.label(
                    RichText::new(trf(
                        "kfx_editor.what_saved_time",
                        "{what}, saved {time}",
                        &[("what", &what), ("time", &files::pretty_time(&s.info.time))],
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
                ui.horizontal(|ui| {
                    if ui.button(trd("kfx_editor.recover", "Recover")).clicked() {
                        act = Some((i, true));
                    }
                    if ui.button(trd("kfx_editor.discard", "Discard")).clicked() {
                        act = Some((i, false));
                    }
                });
            }
            ui.separator();
            if ui
                .button(trd("kfx_editor.decide_later", "Decide later"))
                .clicked()
            {
                later = true;
            }
        });
        if later {
            self.recovery.clear();
        }
        if let Some((i, recover)) = act {
            let s = self.recovery.remove(i);
            if recover {
                if self.anything_dirty() {
                    self.status = trd(
                        "kfx_editor.save_or_discard_the_open_map_first",
                        "Save or discard the open map first",
                    );
                    self.recovery.insert(i, s);
                    return;
                }
                if let Err(e) = self.recover(&s) {
                    self.status = trf(
                        "kfx_editor.recovery_failed_e",
                        "Recovery failed: {e}",
                        &[("e", &e)],
                    );
                }
            } else {
                let _ = std::fs::remove_dir_all(&s.dir);
            }
        }
    }

    // ------------------------------------------------------------------- external changes

    /// Remember the files of the open map as they are now.
    pub(super) fn take_fingerprint(&mut self) {
        self.map_fp = match &self.doc {
            Some(d) if self.doc_on_disk => files::fingerprint(&d.doc.folder, d.doc.level, false),
            _ => vec![],
        };
    }

    /// The window got the focus back: reload what changed on disk (ask when edited).
    pub fn check_external_changes(&mut self) {
        let Some(d) = &self.doc else { return };
        if !self.doc_on_disk {
            return;
        }
        let now = files::fingerprint(&d.doc.folder, d.doc.level, false);
        if !self.map_fp.is_empty() && now != self.map_fp {
            if d.is_dirty() {
                self.external_prompt = true;
            } else {
                let p = d.slb_path();
                let cam = self.camera;
                match self.reload_map(&p) {
                    Ok(()) => {
                        self.camera = cam;
                        self.camera_pending_home = false;
                        self.status = trf(
                            "kfx_editor.p_changed_on_disk_reloaded",
                            "{p} changed on disk: reloaded",
                            &[("p", &p.display())],
                        );
                    }
                    Err(e) => {
                        self.status = trf(
                            "kfx_editor.p_changed_on_disk_but_could_not_be_reloaded_e",
                            "{p} changed on disk but could not be reloaded: {e}",
                            &[("p", &p.display()), ("e", &e)],
                        )
                    }
                }
            }
        }
        self.map_fp = now;
        if let Some(s) = self.scripts.as_mut() {
            for name in [s.ws.dk_name(), s.ws.lua_name()] {
                use kfx_script_ui::workspace::ExternalChange as X;
                match s.ws.file_changed_on_disk(&name, &s.files) {
                    X::Reloaded => {
                        self.status = trf(
                            "kfx_editor.name_changed_on_disk_reloaded",
                            "{name} changed on disk: reloaded",
                            &[("name", &name)],
                        )
                    }
                    X::Conflict => {
                        self.status = trf(
                            "kfx_editor.name_changed_on_disk_and_has_unsaved_edits_see_t",
                            "{name} changed on disk and has unsaved edits: see the script panel",
                            &[("name", &name)],
                        );
                        if self.settings.script_dock == crate::settings::ScriptDock::Hidden {
                            self.settings.script_dock = crate::settings::ScriptDock::Docked;
                        }
                        s.ws.activate(&name);
                    }
                    X::Unchanged => {}
                }
            }
        }
    }

    /// Reload the map files only (scripts stay).
    fn reload_map(&mut self, p: &Path) -> Result<(), String> {
        self.open_map(p)
    }

    pub(super) fn external_prompt_ui(&mut self, ctx: &egui::Context) {
        if !self.external_prompt {
            return;
        }
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("external")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading(
                RichText::new(trd("kfx_editor.map_changed_on_disk", "Map changed on disk"))
                    .color(GOLD),
            );
            ui.label(trd(
                "kfx_editor.the_map_files_were_changed_by_another_program_an",
                "The map files were changed by another program, and the editor has \
                unsaved changes to the map.",
            ));
            ui.horizontal(|ui| {
                if ui
                    .button(trd(
                        "kfx_editor.reload_from_disk_lose_my_changes",
                        "Reload from disk (lose my changes)",
                    ))
                    .clicked()
                {
                    answer = Some(true);
                }
                if ui
                    .button(trd("kfx_editor.keep_my_version", "Keep my version"))
                    .clicked()
                {
                    answer = Some(false);
                }
            });
        });
        if let Some(a) = answer {
            self.external_prompt = false;
            if a {
                if let Some(p) = self.doc.as_ref().map(|d| d.slb_path()) {
                    let cam = self.camera;
                    if let Err(e) = self.reload_map(&p) {
                        self.status = trf(
                            "kfx_editor.reload_failed_e",
                            "Reload failed: {e}",
                            &[("e", &e)],
                        );
                    } else {
                        self.camera = cam;
                        self.camera_pending_home = false;
                    }
                }
            }
        }
    }

    /// Focus tracking (called every frame).
    pub(super) fn focus_tick(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(i.focused));
        if focused && !self.was_focused {
            self.check_external_changes();
        }
        self.was_focused = focused;
    }

    // ------------------------------------------------------------------- backups

    pub fn open_backups(&mut self) {
        let list = S::backup_dir()
            .map(|d| files::list_backups(&d))
            .unwrap_or_default();
        self.backups = Some(BackupsWindow {
            list,
            only_current: self.doc.is_some(),
            confirm: None,
            message: None,
        });
    }

    /// Restore backup `b` into its source folder (the files there are backed up first).
    pub fn restore(&mut self, b: &Backup) -> Result<String, String> {
        let dest = b
            .source
            .clone()
            .or_else(|| self.doc.as_ref().map(|d| d.doc.folder.clone()))
            .ok_or(trd(
                "kfx_editor.the_backup_does_not_say_where_it_came_from",
                "the backup does not say where it came from",
            ))?;
        let dir =
            S::backup_dir().ok_or(trd("kfx_editor.no_backups_folder", "no backups folder"))?;
        let safety = document::backup_level(&dest, b.level, &dir)?;
        let n = files::restore_backup(b, &dest)?;
        let open = self
            .doc
            .as_ref()
            .is_some_and(|d| d.doc.level == b.level && same_folder(&d.doc.folder, &dest));
        if open {
            if let Some(p) = level_bin::find_file(&dest, b.level, "slb") {
                self.scripts = None;
                self.open_map(&p)?;
            }
        }
        Ok(trf(
            "kfx_editor.restored_n_files_of_map_level_into_dest_s",
            "Restored {n} files of map{level} into {dest}{s}",
            &[
                ("n", &n),
                ("level", &format!("{:05}", b.level)),
                ("dest", &dest.display()),
                (
                    "s",
                    &safety
                        .map(|s| format!(" (the previous files are in {})", s.display()))
                        .unwrap_or_default(),
                ),
            ],
        ))
    }

    pub(super) fn backups_ui(&mut self, ctx: &egui::Context) {
        let Some(w) = &mut self.backups else { return };
        let mut open = true;
        let mut restore: Option<usize> = None;
        let mut open_copy: Option<usize> = None;
        let mut folder: Option<PathBuf> = None;
        let cur = self
            .doc
            .as_ref()
            .map(|d| (d.doc.folder.clone(), d.doc.level));
        let restore_tip = trd(
            "kfx_editor.put_these_files_back_the_current_files_are_backe",
            "Put these files back (the current files are backed \
            up first)",
        );
        let open_tip = trd(
            "kfx_editor.open_this_backup_without_restoring_it_save_as_on",
            "Open this backup without restoring it (Save as only)",
        );
        egui::Window::new(trd("kfx_editor.backups", "Backups"))
            .open(&mut open)
            .default_size([760.0, 440.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut w.only_current,
                        trd("kfx_editor.only_the_open_map", "Only the open map"),
                    );
                    let total: u64 = w.list.iter().map(|b| b.bytes).sum();
                    ui.label(
                        RichText::new(trf(
                            "kfx_editor.list_backups_total_mib_limit_backup_limit_mb_mib",
                            "{list} backups, {total} MiB (limit {backup_limit_mb} MiB in \
                            Preferences)",
                            &[
                                ("list", &w.list.len()),
                                ("total", &format!("{:.1}", total as f64 / 1048576.0)),
                                ("backup_limit_mb", &self.settings.backup_limit_mb),
                            ],
                        ))
                        .small()
                        .color(TEXT_DIM),
                    );
                    if ui
                        .button(trd("kfx_editor.open_backups_folder", "Open backups folder"))
                        .clicked()
                    {
                        folder = S::backup_dir();
                    }
                });
                if let Some(m) = &w.message {
                    ui.colored_label(GOLD, m);
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("backups")
                            .striped(true)
                            .num_columns(5)
                            .show(ui, |ui| {
                                ui.label(RichText::new(trd("kfx_editor.when", "When")).strong());
                                ui.label(RichText::new(trd("kfx_editor.map", "Map")).strong());
                                ui.label(RichText::new(trd("kfx_editor.from", "From")).strong());
                                ui.label(RichText::new(trd("kfx_editor.size", "Size")).strong());
                                ui.label("");
                                ui.end_row();
                                for (i, b) in w.list.iter().enumerate() {
                                    let mine = cur.as_ref().is_some_and(|(f, l)| {
                                        *l == b.level
                                            && b.source
                                                .as_deref()
                                                .is_some_and(|s| same_folder(s, f))
                                    });
                                    if w.only_current && !mine {
                                        continue;
                                    }
                                    ui.label(files::pretty_time(&b.time));
                                    ui.label(trf(
                                        "kfx_editor.map_level",
                                        "map{level}",
                                        &[("level", &format!("{:05}", b.level))],
                                    ))
                                    .on_hover_text(b.files.join("\n"));
                                    ui.label(
                                        RichText::new(
                                            b.source
                                                .as_ref()
                                                .map(|s| s.display().to_string())
                                                .unwrap_or_else(|| "?".into()),
                                        )
                                        .small(),
                                    );
                                    ui.label(trf(
                                        "kfx_editor.bytes_kib",
                                        "{bytes} KiB",
                                        &[("bytes", &(b.bytes / 1024))],
                                    ));
                                    ui.horizontal(|ui| {
                                        if w.confirm == Some(i) {
                                            if ui
                                                .button(
                                                    RichText::new(trd(
                                                        "kfx_editor.really_restore",
                                                        "Really restore",
                                                    ))
                                                    .color(Color32::from_rgb(240, 180, 80)),
                                                )
                                                .clicked()
                                            {
                                                restore = Some(i);
                                            }
                                            if ui.small_button(trd("kfx_editor.no", "No")).clicked()
                                            {
                                                w.confirm = None;
                                            }
                                        } else if ui
                                            .button(trd("kfx_editor.restore", "Restore"))
                                            .on_hover_text(restore_tip.as_str())
                                            .clicked()
                                        {
                                            w.confirm = Some(i);
                                        }
                                        if ui
                                            .small_button(trd("kfx_editor.open", "Open"))
                                            .on_hover_text(open_tip.as_str())
                                            .clicked()
                                        {
                                            open_copy = Some(i);
                                        }
                                        if ui
                                            .small_button("📁")
                                            .on_hover_text(trd(
                                                "kfx_editor.open_the_backup_folder",
                                                "Open the backup folder",
                                            ))
                                            .clicked()
                                        {
                                            folder = Some(b.dir.clone());
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                    });
            });
        if !open {
            self.backups = None;
            return;
        }
        if let Some(f) = folder {
            if let Err(e) = files::open_folder(&f) {
                self.status = e;
            }
        }
        if let Some(i) = open_copy {
            let b = self.backups.as_ref().unwrap().list[i].clone();
            if self.anything_dirty() {
                if let Some(w) = self.backups.as_mut() {
                    w.message = Some(
                        trd(
                            "kfx_editor.the_open_map_has_unsaved_changes_save_or_discard",
                            "The open map has unsaved changes: save or discard them first",
                        )
                        .into(),
                    );
                }
                return;
            }
            if let Err(e) = self.open_backup(&b.dir, b.level) {
                if let Some(w) = self.backups.as_mut() {
                    w.message = Some(e);
                }
            }
        }
        if let Some(i) = restore {
            let b = self.backups.as_ref().unwrap().list[i].clone();
            if self.anything_dirty() && cur.as_ref().is_some_and(|(_, l)| *l == b.level) {
                if let Some(w) = self.backups.as_mut() {
                    w.message = Some(
                        trd(
                            "kfx_editor.the_open_map_has_unsaved_changes_save_or_discard",
                            "The open map has unsaved changes: save or discard them first",
                        )
                        .into(),
                    );
                    w.confirm = None;
                }
                return;
            }
            let r = self.restore(&b);
            if let Some(w) = self.backups.as_mut() {
                w.message = Some(r.unwrap_or_else(|e| {
                    trf(
                        "kfx_editor.restore_failed_e",
                        "Restore failed: {e}",
                        &[("e", &e)],
                    )
                }));
                w.confirm = None;
                w.list = S::backup_dir()
                    .map(|d| files::list_backups(&d))
                    .unwrap_or_default();
            }
        }
    }

    // ------------------------------------------------------------------- recent names

    pub(super) fn remember_name(&mut self, path: &Path) {
        let Some(d) = &self.doc else { return };
        let install = self.install().map(Path::to_path_buf);
        let name = install.and_then(|i| files::level_name(&i, &d.doc.folder, d.doc.level));
        let pack = d.pack.clone();
        let label = match (name, pack) {
            (Some(n), Some(p)) => format!("{n} ({p})"),
            (Some(n), None) => n,
            (None, Some(p)) => format!("map{:05} ({p})", d.doc.level),
            (None, None) => String::new(),
        };
        self.settings.set_recent_name(path, &label);
    }

    /// Open the folder of the open map in the file manager.
    pub fn open_containing_folder(&mut self) {
        if let Some(d) = &self.doc {
            if let Err(e) = files::open_folder(&d.doc.folder) {
                self.status = e;
            }
        }
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}
