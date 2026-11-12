//! Documents: opening a map, installing a loaded document in the editor, refreshing the view
//! after changes, the pending actions behind the unsaved-changes prompt, saving (with backups
//! and the external-change fingerprint) and the prompt for a file that cannot be read.
use super::*;
use kfx_prefab::i18n::trn;

impl EditorApp {
    /// Load a map (`mapNNNNN.slb` or any file of the set). Does not ask about unsaved changes.
    pub fn open_map(&mut self, path: &Path) -> Result<(), String> {
        let Some(install) = self.install().map(Path::to_path_buf) else {
            self.pending_open = Some(path.to_path_buf());
            return Err(trd(
                "kfx_editor.no_install_folder_yet",
                "no install folder yet",
            ));
        };
        let level = document::map_number(path).ok_or_else(|| {
            trf(
                "kfx_editor.path_not_a_mapnnnnn_file",
                "{path}: not a mapNNNNN file",
                &[("path", &path.display())],
            )
        })?;
        let folder = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let loc = document::locate_with(&install, &folder, level, &self.active_mods())?;
        let gd = self.gamedata_stack(&loc.stack)?;
        let doc = Document::load(&loc, gd)?;
        let warn = doc.doc.warnings.clone();
        let unreadable = doc.doc.unreadable.clone();
        if let Some(w) = warn.iter().find(|w| w.contains("custom slab cells")) {
            self.notice = Some(trf(
                "kfx_editor.this_map_has_custom_slabs_from_its_une_file",
                "This map has custom slabs from its .une file: {w}. They are editor \
                custom slabs now (Custom tab of the palette), saved in map{level}.kfxed.toml \
                next to the map.",
                &[("w", &w), ("level", &format!("{:05}", level))],
            ));
        }
        self.set_document(doc, true);
        self.from_backup = false;
        self.unreadable_prompt = (!unreadable.is_empty()).then_some(unreadable);
        let packed = extras::rnc_packed_files(&folder, level);
        if packed > 0 && self.settings.rnc_notice && self.notice.is_none() {
            self.notice = Some(trn(
                "kfx_editor.packed_file_s_of_map_level_are_rnc_compressed_th",
                packed as i64,
                "{n} file of map{level} is RNC-compressed (the original \
                game's packer). It is read normally; saving writes it uncompressed, which \
                the game reads as well. (Preferences > Files turns this notice off.)",
                "{n} files of map{level} are RNC-compressed (the original \
                game's packer). They are read normally; saving writes them uncompressed, which \
                the game reads as well. (Preferences > Files turns this notice off.)",
                &[("level", &format!("{:05}", level))],
            ));
        }
        self.settings.push_recent(path);
        self.remember_name(path);
        self.save_settings();
        let d = self.doc.as_ref().unwrap();
        let pack = d
            .pack
            .clone()
            .unwrap_or_else(|| trd("kfx_editor.no_campaign", "no campaign").into());
        let layers: Vec<String> = d
            .stack
            .config
            .iter()
            .map(|(o, l)| {
                format!(
                    "{}{}*{}",
                    l.dir.file_name().unwrap_or_default().to_string_lossy(),
                    if l.prefix.is_empty() {
                        "/".to_string()
                    } else {
                        format!("/{}", l.prefix)
                    },
                    if o.is_mod() {
                        format!(" ({o})")
                    } else {
                        String::new()
                    }
                )
            })
            .collect();
        self.status = trf(
            "kfx_editor.opened_path_pack_config_layers_texture_sets_text",
            "Opened {path} [{pack}; config {layers}; texture sets {texture_sets}]{warn}",
            &[
                ("path", &path.display()),
                ("pack", &pack),
                ("layers", &layers.join(" + ")),
                ("texture_sets", &self.texture_sets.join(", ")),
                (
                    "warn",
                    &(if warn.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", warn.join("; "))
                    }),
                ),
            ],
        );
        Ok(())
    }

    /// Show a document: terrain, atlas, meshes, palette, minimap.
    pub(super) fn set_document(&mut self, doc: Document, on_disk: bool) {
        let Some(assets) = &self.assets else { return };
        self.drag = None;
        self.pasting = false;
        self.selection = None;
        self.cubes = CubeFaces::from_gamedata(&doc.gd);
        self.terrain = Terrain::from_doc(&doc.doc);
        let t0 = Instant::now();
        let search = TextureSearch {
            level_dir: Some(doc.doc.folder.clone()),
            ..TextureSearch::new(assets.install.data_dir())
        };
        let atlas = atlas::build(
            &search,
            Some(doc.doc.level),
            doc.doc.texture_id as u32,
            &self.terrain.used_variations(),
            &assets.palette,
            &assets.anim,
        );
        let atlas_ms = t0.elapsed().as_secs_f32() * 1000.0;
        let t1 = Instant::now();
        let (cw, ch) = mesh::chunk_counts(&self.terrain);
        let meshes: Vec<mesh::ChunkMesh> = (0..cw * ch)
            .map(|i| mesh::build_chunk(&self.terrain, &self.cubes, i % cw, i / cw))
            .collect();
        let mesh_ms = t1.elapsed().as_secs_f32() * 1000.0;
        let t2 = Instant::now();
        self.view
            .set_atlas(&self.gpu.device, &self.gpu.queue, &atlas);
        self.view.set_chunks(&self.gpu.device, &meshes);
        let upload_ms = t2.elapsed().as_secs_f32() * 1000.0;
        self.perf = Perf {
            atlas_ms,
            mesh_ms,
            upload_ms,
            chunks: meshes.len(),
            ..Default::default()
        };
        self.build_palette(&doc.gd, &atlas);
        self.texture_sets = atlas.layer_sets.iter().map(|s| s.to_string()).collect();
        self.atlas = Some(Arc::new(atlas));
        let atlas = self.atlas.clone().unwrap();
        if let Some(b) = self.brush {
            if b as usize >= doc.gd.slab_count() || assets::is_door(&doc.gd, b as usize) {
                self.brush = None;
            }
        }
        if !atlas.warnings.is_empty() {
            self.status = atlas.warnings.join("; ");
        }
        self.symmetry = doc.doc.sidecar.symmetry.clone();
        let mut doc = doc;
        doc.backups = self.session_backups.clone();
        self.doc = Some(doc);
        self.doc_on_disk = on_disk;
        self.minimap_dirty = true;
        self.camera_pending_home = true;
        self.camera2_home = true;
        self.strings = None;
        self.custom_brush = None;
        self.build_custom_palette();
        self.hover = None;
        self.selected.clear();
        self.thing_drag = None;
        self.rect_sel = None;
        self.apply_decor_prefs();
        self.load_sprites();
        self.build_thing_palette();
        self.gold_rules = self
            .doc
            .as_ref()
            .map(|d| things::GoldRules::load(&d.layers))
            .unwrap_or_default();
        self.attach_scripts();
        self.take_fingerprint();
        self.last_autosave = Instant::now();
        if let Some(p) = &self.place {
            let gd = self.doc.as_ref().unwrap().gd.clone();
            if PlaceKind::parse(
                &format!(
                    "{}:{}",
                    match p {
                        PlaceKind::Creature(_) => "creature",
                        PlaceKind::Object(_) => "object",
                        PlaceKind::Trap(_) => "trap",
                        PlaceKind::Door(_) => "door",
                        PlaceKind::EffectGen(_) => "effect",
                        _ => "",
                    },
                    p.model()
                ),
                &gd,
            )
            .is_none()
                && p.class().is_some()
            {
                self.place = None;
            }
        }
        self.revalidate();
    }

    /// Rebuild terrain, texture atlas and meshes from the document (size / texture changes).
    pub(super) fn rebuild_view(&mut self) {
        let (Some(assets), Some(doc)) = (&self.assets, &self.doc) else {
            return;
        };
        self.terrain = Terrain::from_doc(&doc.doc);
        let search = TextureSearch {
            level_dir: Some(doc.doc.folder.clone()),
            ..TextureSearch::new(assets.install.data_dir())
        };
        let atlas = atlas::build(
            &search,
            Some(doc.doc.level),
            doc.doc.texture_id as u32,
            &self.terrain.used_variations(),
            &assets.palette,
            &assets.anim,
        );
        let (cw, ch) = mesh::chunk_counts(&self.terrain);
        let meshes: Vec<mesh::ChunkMesh> = (0..cw * ch)
            .map(|i| mesh::build_chunk(&self.terrain, &self.cubes, i % cw, i / cw))
            .collect();
        self.view
            .set_atlas(&self.gpu.device, &self.gpu.queue, &atlas);
        self.view.set_chunks(&self.gpu.device, &meshes);
        self.perf.chunks = meshes.len();
        self.texture_sets = atlas.layer_sets.iter().map(|s| s.to_string()).collect();
        self.minimap_dirty = true;
        self.grid_cols = None;
        let gd = doc.gd.clone();
        self.build_palette(&gd, &atlas);
        self.atlas = Some(Arc::new(atlas));
        self.build_custom_palette();
    }

    /// Redraw what a change needs.
    pub(crate) fn after_change(&mut self, r: Refresh) {
        self.prefab_sync_scripts();
        if r.items || r.whole {
            self.check_numbers();
        }
        if r.whole {
            self.rebuild_view();
            self.prune_selection();
            return;
        }
        if r.items {
            self.prune_selection();
        }
        // a .slx style outside the atlas' loaded sets needs a new atlas
        if let (Some(d), Some(_)) = (&self.doc, r.slabs) {
            let loaded = self.terrain.used_variations();
            if d.doc.slx.iter().any(|v| !loaded.contains(&(v & 0x1F))) {
                self.rebuild_view();
                return;
            }
        }
        self.refresh(r.slabs);
    }

    /// Redraw after the slabs of `r` changed.
    pub(super) fn refresh(&mut self, r: Option<SlabRect>) {
        let (Some(r), Some(doc)) = (r, &self.doc) else {
            return;
        };
        if r.is_empty() {
            return;
        }
        self.terrain.update(&doc.doc, r);
        let (cw, _) = mesh::chunk_counts(&self.terrain);
        let t = Instant::now();
        for i in mesh::chunks_for_slabs(&self.terrain, r) {
            let m = mesh::build_chunk(&self.terrain, &self.cubes, i % cw, i / cw);
            self.view.update_chunk(&self.gpu.device, i, &m);
        }
        self.perf.mesh_ms = t.elapsed().as_secs_f32() * 1000.0;
        self.minimap_dirty = true;
        self.grid_cols = None;
    }

    /// Run `p` now, or ask first when the document has unsaved changes.
    pub(super) fn request(&mut self, p: Pending) {
        if self.anything_dirty() {
            self.pending = Some(p);
        } else {
            self.perform(p);
        }
    }

    pub(super) fn perform(&mut self, p: Pending) {
        match p {
            Pending::Open(path) => {
                if let Err(e) = self.open_map(&path) {
                    self.status = trf(
                        "kfx_editor.could_not_open_path_e",
                        "Could not open {path}: {e}",
                        &[("path", &path.display()), ("e", &e)],
                    );
                }
            }
            Pending::New => self.open_new_dialog(),
            Pending::Reload => {
                if let Some(path) = self
                    .doc
                    .as_ref()
                    .filter(|_| self.doc_on_disk)
                    .map(|d| d.slb_path())
                {
                    if let Err(e) = self.open_map(&path) {
                        self.status = trf(
                            "kfx_editor.could_not_reload_e",
                            "Could not reload: {e}",
                            &[("e", &e)],
                        );
                    } else {
                        self.status = trf(
                            "kfx_editor.reloaded_path",
                            "Reloaded {path}",
                            &[("path", &path.display())],
                        );
                    }
                }
            }
            Pending::Exit => {
                self.allow_close = true;
                self.quit_requested = true;
            }
            Pending::Restart(what) => self.restart_for(what),
        }
    }

    /// Save to the document's own folder and level number.
    pub fn save(&mut self) -> Result<(), String> {
        let Some(d) = &self.doc else {
            return Err(trd("kfx_editor.no_map", "no map"));
        };
        if self.from_backup {
            self.open_save_as();
            return Err(trd(
                "kfx_editor.this_map_was_opened_from_a_backup_save_it_with_f",
                "This map was opened from a backup: save it with File > Save as (or \
                restore the backup from File > Backups)",
            ));
        }
        let (folder, level) = (d.doc.folder.clone(), d.doc.level);
        self.write_doc(&folder, level)
    }

    /// Save under another folder / level.
    pub fn save_as(&mut self, folder: &Path, level: u32) -> Result<(), String> {
        if self.from_backup
            && self
                .doc
                .as_ref()
                .is_some_and(|d| d.doc.level == level && extras::same_dir(&d.doc.folder, folder))
        {
            return Err(trd(
                "kfx_editor.this_map_was_opened_from_a_backup_choose_another",
                "This map was opened from a backup: choose another folder (the \
                backups folder is not a place to save maps)",
            ));
        }
        self.write_doc(folder, level)?;
        self.from_backup = false;
        Ok(())
    }

    pub(super) fn write_doc(&mut self, folder: &Path, level: u32) -> Result<(), String> {
        self.cancel_tool();
        if self.doc.is_none() {
            return Err(trd("kfx_editor.no_map", "no map"));
        }
        if let Some(why) = self.doc.as_ref().and_then(|d| d.doc.read_only_reason()) {
            self.status = trf(
                "kfx_editor.not_saved_the_map_is_why",
                "Not saved: the map is {why}",
                &[("why", &why)],
            );
            return Err(trf(
                "kfx_editor.not_saved_the_map_is_why",
                "Not saved: the map is {why}",
                &[("why", &why)],
            ));
        }
        // back up an existing map (with its scripts) once per session before overwriting it
        let key = (
            folder
                .canonicalize()
                .unwrap_or_else(|_| folder.to_path_buf()),
            level,
        );
        let mut note = String::new();
        self.backup_level_once(folder, level, &mut note)?;
        let same = self
            .doc
            .as_ref()
            .is_some_and(|d| d.doc.folder == folder && d.doc.level == level);
        if same
            && self.doc_on_disk
            && !self.doc.as_ref().unwrap().is_dirty()
            && self.scripts_dirty()
        {
            // only the scripts changed
            let r = self.save_scripts();
            self.status = match r {
                Ok(n) => {
                    // the crash-recovery snapshot goes only once its content is on disk
                    self.drop_snapshot();
                    trn(
                        "kfx_editor.saved_n_script_s_note",
                        n as i64,
                        "Saved {n} script{note}",
                        "Saved {n} scripts{note}",
                        &[("note", &note)],
                    )
                }
                Err(e) => trf(
                    "kfx_editor.script_save_failed_e",
                    "Script save failed: {e}",
                    &[("e", &e)],
                ),
            };
            return Ok(());
        }
        if let Some(dir) = crate::settings::Settings::backup_dir() {
            let removed = crate::files::prune_backups(&dir, self.settings.backup_limit_mb);
            if !removed.is_empty() {
                note.push_str(&format!(" ({} old backups removed)", removed.len()));
            }
        }
        let legacy = self.save_format_legacy();
        let (from_folder, from_level) = self
            .doc
            .as_ref()
            .map(|d| (d.doc.folder.clone(), d.doc.level))
            .unwrap();
        let d = self.doc.as_mut().unwrap();
        if legacy {
            // the report first: errors block the save, warnings are shown in the status
            let rep = d.doc.legacy_report(&d.gd);
            if !rep.errors.is_empty() {
                let msg = trf(
                    "kfx_editor.not_saved_the_original_dk_format_cannot_hold_thi",
                    "Not saved: the original DK format cannot hold this map: {errors}",
                    &[("errors", &rep.errors.join("; "))],
                );
                self.legacy_prompt = Some((folder.to_path_buf(), level, rep));
                return Err(msg);
            }
            if !rep.warnings.is_empty() {
                note.push_str(&format!(" (legacy format: {})", rep.warnings.join("; ")));
            }
        }
        if legacy && d.doc.sidecar.save_format.is_none() {
            // a choice from the old settings list moves into the map's sidecar
            d.doc.sidecar.save_format = Some("legacy".into());
        }
        let mut complete = true;
        if let Err(e) = d.save_to_format(folder, level, legacy) {
            if !e.starts_with("the map was saved") {
                self.status = trf("kfx_editor.not_saved_e", "Not saved: {e}", &[("e", &e)]);
                return Err(e);
            }
            complete = false;
            note.push_str(&format!(" ({e})"));
        }
        if !same {
            // config overrides, string tables and the rest of the level's own files go along
            match document::copy_level_extras(&from_folder, from_level, folder, level) {
                Ok(v) if !v.is_empty() => note.push_str(&format!(
                    " ({} level files copied: {})",
                    v.len(),
                    v.iter()
                        .map(|p| p
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Ok(_) => {}
                Err(e) => {
                    complete = false;
                    note.push_str(&format!(" (level files not copied: {e})"));
                }
            }
        }
        if let Err(e) = self.save_strings() {
            complete = false;
            note.push_str(&format!(" (strings not saved: {e})"));
        }
        let d = self.doc.as_mut().unwrap();
        self.backed_up.insert(key);
        self.doc_on_disk = true;
        let p = d.slb_path();
        let scripts = self.move_scripts(folder, level);
        self.settings.push_recent(&p);
        self.remember_name(&p);
        self.save_settings();
        self.take_fingerprint();
        if complete && scripts.is_ok() {
            // the crash-recovery snapshot goes only once everything it holds is on disk
            self.drop_snapshot();
        }
        self.status = match scripts {
            Ok(()) => trf(
                "kfx_editor.saved_p_note",
                "Saved {p}{note}",
                &[("p", &p.display()), ("note", &note)],
            ),
            Err(e) => trf(
                "kfx_editor.saved_p_but_the_scripts_failed_e_note",
                "Saved {p} but the scripts failed: {e}{note}",
                &[("p", &p.display()), ("e", &e), ("note", &note)],
            ),
        };
        Ok(())
    }

    /// Back up the files of level `level` in `folder` once per session, before the first write
    /// (a failed backup stops the save). The copies are also known to the session backups.
    pub(crate) fn backup_level_once(
        &mut self,
        folder: &Path,
        level: u32,
        note: &mut String,
    ) -> Result<(), String> {
        let key = (
            folder
                .canonicalize()
                .unwrap_or_else(|_| folder.to_path_buf()),
            level,
        );
        let files = document::level_files(folder, level);
        if self.backed_up.contains(&key) || files.is_empty() {
            return Ok(());
        }
        let dir = crate::settings::Settings::backup_dir().ok_or(trd(
            "kfx_editor.no_config_directory_for_backups_not_saved",
            "no config directory for backups: not saved",
        ))?;
        if let Some(b) = document::backup_level(folder, level, &dir).map_err(|e| {
            trf(
                "kfx_editor.the_backup_failed_so_nothing_was_written_e",
                "the backup failed, so nothing was written: {e}",
                &[("e", &e)],
            )
        })? {
            note.push_str(&format!(" (previous files backed up to {})", b.display()));
        }
        for f in files {
            self.session_backups.mark(&f);
        }
        self.backed_up.insert(key);
        Ok(())
    }

    /// Re-read the fingerprint entries of files the editor itself just wrote (so they are not
    /// taken for a change by another program); other entries stay as they were.
    pub(crate) fn refresh_fingerprint_for(&mut self, written: &[PathBuf]) {
        let Some(d) = &self.doc else { return };
        if !self.doc_on_disk || self.map_fp.is_empty() {
            return;
        }
        let names: Vec<String> = written
            .iter()
            .filter_map(|p| {
                p.file_name()
                    .map(|f| f.to_string_lossy().to_ascii_lowercase())
            })
            .collect();
        let now = crate::files::fingerprint(&d.doc.folder, d.doc.level, false);
        self.map_fp.retain(|(n, _, _)| !names.contains(n));
        self.map_fp
            .extend(now.into_iter().filter(|(n, _, _)| names.contains(n)));
        self.map_fp.sort_by(|a, b| a.0.cmp(&b.0));
    }

    /// The map opened with unreadable files: open read-only, or discard them (after a backup).
    pub(crate) fn discard_unreadable(&mut self) -> Result<(), String> {
        let Some(d) = self.doc.as_ref() else {
            return Err(trd("kfx_editor.no_map", "no map"));
        };
        let (folder, level) = (d.doc.folder.clone(), d.doc.level);
        // always a fresh copy of the level as it is now (the unreadable files may have changed
        // since an earlier backup of this session)
        let dir = crate::settings::Settings::backup_dir().ok_or(trd(
            "kfx_editor.no_config_directory_for_backups_the_files_are_ke",
            "no config directory for backups: the files are kept",
        ))?;
        let b = document::backup_level(&folder, level, &dir).map_err(|e| {
            trf(
                "kfx_editor.the_backup_failed_so_the_files_are_kept_e",
                "the backup failed, so the files are kept: {e}",
                &[("e", &e)],
            )
        })?;
        let note = b
            .map(|b| {
                trf(
                    "kfx_editor.the_current_files_are_backed_up_in_b",
                    " (the current files are backed up in {b})",
                    &[("b", &b.display())],
                )
            })
            .unwrap_or_default();
        for f in document::level_files(&folder, level) {
            self.session_backups.mark(&f);
        }
        self.backed_up.insert((
            folder.canonicalize().unwrap_or_else(|_| folder.clone()),
            level,
        ));
        if let Some(d) = self.doc.as_mut() {
            d.doc.discard_unreadable();
        }
        self.unreadable_prompt = None;
        self.status = trf(
            "kfx_editor.the_unreadable_files_will_be_replaced_when_the_m",
            "The unreadable files will be replaced when the map is saved{note}",
            &[("note", &note)],
        );
        Ok(())
    }

    pub(super) fn unreadable_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(parts) = self.unreadable_prompt.clone() else {
            return;
        };
        let mut read_only = false;
        let mut discard = false;
        egui::Modal::new(egui::Id::unique("unreadable-files")).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.heading(
                RichText::new(trd(
                    "kfx_editor.some_files_of_this_map_could_not_be_read",
                    "Some files of this map could not be read",
                ))
                .color(GOLD),
            );
            ui.label(trd(
                "kfx_editor.the_editor_shows_the_map_without_what_these_file",
                "The editor shows the map without what these files hold. Saving \
                would replace them, so the map is open read-only.",
            ));
            for p in &parts {
                ui.separator();
                ui.label(RichText::new(format!("{} ({})", p.path.display(), p.what)).strong());
                ui.label(
                    RichText::new(&p.error)
                        .color(Color32::from_rgb(240, 120, 100))
                        .monospace(),
                );
            }
            ui.separator();
            ui.label(
                RichText::new(trd(
                    "kfx_editor.fix_the_file_in_a_text_editor_and_reopen_the_map",
                    "Fix the file in a text editor and reopen the map to keep its \
                    content (Troubleshooting in the manual).",
                ))
                .small()
                .color(TEXT_DIM),
            );
            ui.horizontal(|ui| {
                if ui
                    .button(trd("kfx_editor.open_read_only", "Open read-only"))
                    .clicked()
                {
                    read_only = true;
                }
                if ui
                    .button(trd(
                        "kfx_editor.open_and_discard_the_unreadable_things_a_backup",
                        "Open and discard the unreadable things (a backup is kept)",
                    ))
                    .clicked()
                {
                    discard = true;
                }
            });
        });
        if read_only {
            self.unreadable_prompt = None;
            self.status = trd(
                "kfx_editor.read_only_the_map_has_files_the_editor_could_not",
                "Read-only: the map has files the editor could not read (File > Save is refused)",
            );
        }
        if discard {
            if let Err(e) = self.discard_unreadable() {
                self.unreadable_prompt = None;
                self.status = trf(
                    "kfx_editor.still_read_only_e",
                    "Still read-only: {e}",
                    &[("e", &e)],
                );
            }
        }
    }
}
