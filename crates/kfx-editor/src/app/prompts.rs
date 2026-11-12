//! File dialogs and prompts: Open, Save as, the first-run install choice, the unsaved-changes
//! prompt, the save format and the report shown before a save in the original format.
use super::*;

impl EditorApp {
    pub(super) fn open_dialog(&mut self) {
        let mut dlg = rfd::FileDialog::new().add_filter(
            trd("kfx_editor.keeperfx_map_map_slb", "KeeperFX map (map*.slb)"),
            &["slb", "SLB"],
        );
        if let Some(d) = self
            .doc
            .as_ref()
            .map(|l| l.doc.folder.clone())
            .or_else(|| self.install().map(Path::to_path_buf))
        {
            dlg = dlg.set_directory(d);
        }
        if let Some(p) = dlg.pick_file() {
            self.request(Pending::Open(p));
        }
    }

    pub(super) fn open_save_as(&mut self) {
        if let Some(d) = &self.doc {
            self.save_as_dialog = Some(SaveAsDialog {
                folder: d.doc.folder.display().to_string(),
                level: d.doc.level,
                confirm_overwrite: false,
                error: None,
            });
        }
    }

    pub(super) fn do_save(&mut self) {
        if !self.doc_on_disk
            && self
                .doc
                .as_ref()
                .is_some_and(|d| !document::level_files(&d.doc.folder, d.doc.level).is_empty())
        {
            // a new map whose target already exists: let the user confirm in Save As
            self.open_save_as();
            return;
        }
        if let Err(e) = self.save() {
            self.status = trf("kfx_editor.save_failed_e", "Save failed: {e}", &[("e", &e)]);
        }
    }

    pub(super) fn first_run_ui(&mut self, ctx: &egui::Context) {
        if !self.first_run.open {
            return;
        }
        let mut chosen = None;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("first-run")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(
                RichText::new(trd("kfx_editor.keeperfx_folder", "KeeperFX folder")).color(GOLD),
            );
            ui.label(trd(
                "kfx_editor.choose_the_folder_of_your_keeperfx_install_the_o",
                "Choose the folder of your KeeperFX install (the one that holds \
                fxdata/ and data/).",
            ));
            if let Some(e) = &self.data_error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.first_run.text).desired_width(380.0));
                if ui.button(trd("kfx_editor.browse", "Browse…")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.first_run.text = p.display().to_string();
                    }
                }
            });
            let p = PathBuf::from(self.first_run.text.trim());
            let valid = validate_install(&p);
            match &valid {
                Ok(()) => {
                    ui.colored_label(
                        Color32::LIGHT_GREEN,
                        trd(
                            "kfx_editor.looks_like_a_keeperfx_install",
                            "Looks like a KeeperFX install.",
                        ),
                    );
                    let c = extras::install_checks(&p);
                    ui.label(RichText::new(c.summary()).small().color(if c.writable {
                        TEXT_DIM
                    } else {
                        Color32::from_rgb(240, 160, 80)
                    }));
                }
                Err(e) if !self.first_run.text.trim().is_empty() => {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
                Err(_) => {}
            }
            if let Some(e) = &self.first_run.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        valid.is_ok(),
                        egui::Button::new(trd("kfx_editor.use_this_folder", "Use this folder")),
                    )
                    .clicked()
                {
                    chosen = Some(p.clone());
                }
                if self.assets.is_some() && ui.button(trd("kfx_editor.cancel", "Cancel")).clicked()
                {
                    cancel = true;
                }
            });
        });
        if let Some(p) = chosen {
            self.first_run.open = false;
            self.set_install(p);
        }
        if cancel {
            self.first_run.open = false;
        }
    }

    pub(super) fn unsaved_prompt(&mut self, ctx: &egui::Context) {
        let Some(p) = self.pending.clone() else {
            return;
        };
        let mut what = vec![];
        if self.doc.as_ref().is_some_and(|d| d.is_dirty()) {
            what.push(
                self.doc
                    .as_ref()
                    .map(|d| format!("map{:05}", d.doc.level))
                    .unwrap_or_default(),
            );
        }
        if let Some(s) = &self.scripts {
            what.extend(s.ws.dirty_tabs());
        }
        let name = what.join(", ");
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("unsaved")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(
                RichText::new(trd("kfx_editor.unsaved_changes", "Unsaved changes")).color(GOLD),
            );
            ui.label(trf(
                "kfx_editor.unsaved_changes_in_name_save_them_first",
                "Unsaved changes in {name}. Save them first?",
                &[("name", &name)],
            ));
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(trd("kfx_editor.save", "💾 Save")).clicked() {
                    answer = Some(0);
                }
                if ui
                    .button(trd("kfx_editor.discard_changes", "Discard changes"))
                    .clicked()
                {
                    answer = Some(1);
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    answer = Some(2);
                }
            });
        });
        match answer {
            Some(0) => {
                self.pending = None;
                self.do_save();
                if !self.anything_dirty() {
                    self.perform(p);
                }
            }
            Some(1) => {
                self.pending = None;
                self.drop_snapshot();
                if let Some(s) = self.scripts.as_mut() {
                    // discarded: reload the scripts from disk (also when the map is reopened)
                    s.ws.load(&s.files);
                }
                self.perform(p);
            }
            Some(2) => self.pending = None,
            _ => {}
        }
    }

    pub(super) fn save_as_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.save_as_dialog else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let exists = !dlg.folder.trim().is_empty()
            && !document::level_files(Path::new(dlg.folder.trim()), dlg.level).is_empty();
        let install = self.assets.as_ref().map(|a| a.install.root.clone());
        let multi = self.doc.as_ref().is_some_and(|d| {
            let lof = level_bin::find_file(&d.doc.folder, d.doc.level, "lof")
                .and_then(|p| Lof::load(&p).ok())
                .map(|l| l.info());
            lof.or_else(|| d.pending_info.clone())
                .is_some_and(|i| i.kinds.contains(&kfx_formats::levelinfo::LevelKind::Multi))
        });
        egui::Modal::new(egui::Id::unique("save-as")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(RichText::new(trd("kfx_editor.save_map_as", "Save map as")).color(GOLD));
            egui::Grid::new("save-as-grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label(trd("kfx_editor.folder", "Folder"));
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::TextEdit::singleline(&mut dlg.folder).desired_width(320.0))
                            .changed()
                        {
                            dlg.confirm_overwrite = false;
                        }
                        if ui.button(trd("kfx_editor.browse", "Browse…")).clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                dlg.folder = p.display().to_string();
                                dlg.confirm_overwrite = false;
                            }
                        }
                    });
                    ui.end_row();
                    ui.label(trd("kfx_editor.level_number", "Level number"));
                    if ui
                        .add(egui::DragValue::new(&mut dlg.level).range(1..=99999))
                        .changed()
                    {
                        dlg.confirm_overwrite = false;
                    }
                    ui.end_row();
                });
            if ui
                .small_button(trd("kfx_editor.next_free_number", "Next free number"))
                .on_hover_text(trd(
                    "kfx_editor.the_lowest_level_number_with_no_files_in_this_fo",
                    "The lowest level number with no files in this folder",
                ))
                .clicked()
            {
                let f = Path::new(dlg.folder.trim());
                if let Some(n) = (1..=32767u32).find(|&n| document::level_files(f, n).is_empty()) {
                    dlg.level = n;
                    dlg.confirm_overwrite = false;
                }
            }
            match extras::playable_from(install.as_deref(), Path::new(dlg.folder.trim())) {
                Ok(why) => ui.label(
                    RichText::new(trf(
                        "kfx_editor.playable_from_this_folder_why",
                        "✔ Playable from this folder ({why})",
                        &[("why", &why)],
                    ))
                    .small()
                    .color(Color32::from_rgb(140, 210, 120)),
                ),
                Err(why) => ui.label(
                    RichText::new(format!("⚠ {why}"))
                        .small()
                        .color(Color32::from_rgb(240, 180, 80)),
                ),
            };
            if multi && !extras::in_multiplayer_folder(Path::new(dlg.folder.trim())) {
                ui.colored_label(
                    Color32::from_rgb(240, 180, 80),
                    trd(
                        "kfx_editor.this_is_a_multi_multiplayer_level_the_game_lists",
                        "This is a MULTI (multiplayer) level: the game lists multiplayer \
                        maps from the multiplayer folder.",
                    ),
                );
            }
            if exists {
                ui.colored_label(
                    Color32::from_rgb(240, 180, 80),
                    trf(
                        "kfx_editor.map_level_already_exists_there_and_will_be_overw",
                        "map{level} already exists there and will be overwritten (a \
                        backup is made first).",
                        &[("level", &format!("{:05}", dlg.level))],
                    ),
                );
            }
            if let Some(e) = &dlg.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let label = if exists && !dlg.confirm_overwrite {
                    trd("kfx_editor.save_f008", "Save")
                } else if exists {
                    trd("kfx_editor.overwrite", "Overwrite")
                } else {
                    trd("kfx_editor.save_f008", "Save")
                };
                if ui
                    .add_enabled(!dlg.folder.trim().is_empty(), egui::Button::new(label))
                    .clicked()
                {
                    if exists && !dlg.confirm_overwrite {
                        dlg.confirm_overwrite = true;
                        dlg.error = Some(
                            trd(
                                "kfx_editor.click_overwrite_to_replace_the_existing_map",
                                "Click Overwrite to replace the existing map.",
                            )
                            .into(),
                        );
                    } else {
                        ok = true;
                    }
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.save_as_dialog = None;
        } else if ok {
            let d = self.save_as_dialog.take().unwrap();
            let folder = PathBuf::from(d.folder.trim());
            if let Err(e) = self.save_as(&folder, d.level) {
                self.save_as_dialog = Some(SaveAsDialog {
                    error: Some(e),
                    confirm_overwrite: false,
                    ..d
                });
            }
        }
    }

    /// The open map saves in the original DK format (`.tng/.apt/.lgt`): the map's sidecar
    /// (`save_format`), else the per-map list older versions kept in the editor settings.
    pub fn save_format_legacy(&self) -> bool {
        let Some(d) = &self.doc else { return false };
        match d.doc.sidecar.save_format.as_deref() {
            Some(f) => f == "legacy",
            None => {
                let p = d.slb_path();
                self.settings
                    .save_formats
                    .iter()
                    .find(|(x, _)| *x == p)
                    .map(|(_, f)| f == "legacy")
                    .unwrap_or(false)
            }
        }
    }

    /// Choose the save format of the open map. Stored in the map's sidecar (an undoable change
    /// that is saved with the map); the old settings entry for the map is dropped.
    pub fn set_save_format(&mut self, legacy: bool) {
        let Some(p) = self.doc.as_ref().map(|d| d.slb_path()) else {
            return;
        };
        if self.settings.save_formats.iter().any(|(x, _)| *x == p) {
            self.settings.save_formats.retain(|(x, _)| *x != p);
            self.save_settings();
        }
        let want = if legacy { "legacy" } else { "fx" };
        if let Some(d) = self.doc.as_mut() {
            if d.doc.sidecar.save_format.as_deref() != Some(want) {
                let rf = d.edit(&trd("kfx_editor.save_format", "Save format"), |m, _, _| {
                    m.touch_props();
                    m.sidecar.save_format = Some(want.to_string());
                });
                self.after_change(rf.1);
            }
        }
        if legacy {
            if let Some(d) = &self.doc {
                let rep = d.doc.legacy_report(&d.gd);
                if !rep.errors.is_empty() || !rep.warnings.is_empty() {
                    self.legacy_prompt = Some((d.doc.folder.clone(), d.doc.level, rep));
                }
            }
        }
        self.status = trf(
            "kfx_editor.map_level_saves_in_the_legacy_format",
            "map{level} saves in the {legacy} format",
            &[
                (
                    "level",
                    &format!("{:05}", self.doc.as_ref().map(|d| d.doc.level).unwrap_or(0)),
                ),
                (
                    "legacy",
                    &(if legacy {
                        trd("kfx_editor.original_dk", "original DK")
                    } else {
                        trd("kfx_editor.keeperfx_format_name", "KeeperFX")
                    }),
                ),
            ],
        );
    }

    /// What a legacy save loses (warnings) or cannot do (errors, the save is refused).
    pub(super) fn legacy_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some((_, level, rep)) = self.legacy_prompt.clone() else {
            return;
        };
        let mut close = false;
        let mut back_to_fx = false;
        egui::Modal::new(egui::Id::unique("legacy-report")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(
                RichText::new(trf(
                    "kfx_editor.map_level_in_the_original_dk_format",
                    "map{level} in the original DK format",
                    &[("level", &format!("{:05}", level))],
                ))
                .color(GOLD),
            );
            if rep.errors.is_empty() {
                ui.label(trd(
                    "kfx_editor.the_map_can_be_saved_in_the_tng_apt_lgt_files_th",
                    "The map can be saved in the .tng / .apt / .lgt files; these are lost:",
                ));
            } else {
                ui.colored_label(
                    Color32::from_rgb(240, 90, 80),
                    trd(
                        "kfx_editor.the_map_cannot_be_saved_in_the_original_format",
                        "The map cannot be saved in the original format:",
                    ),
                );
                for e in &rep.errors {
                    ui.label(
                        RichText::new(format!("✖ {e}")).color(Color32::from_rgb(240, 120, 100)),
                    );
                }
            }
            for w in &rep.warnings {
                ui.label(RichText::new(format!("⚠ {w}")).color(Color32::from_rgb(240, 190, 80)));
            }
            if rep.errors.is_empty() && rep.warnings.is_empty() {
                ui.label(trd("kfx_editor.nothing_is_lost", "Nothing is lost."));
            }
            ui.horizontal(|ui| {
                if ui.button(trd("kfx_editor.ok", "OK")).clicked() {
                    close = true;
                }
                if ui
                    .button(trd(
                        "kfx_editor.save_in_the_keeperfx_format_instead",
                        "Save in the KeeperFX format instead",
                    ))
                    .clicked()
                {
                    back_to_fx = true;
                }
            });
        });
        if close || back_to_fx {
            self.legacy_prompt = None;
        }
        if back_to_fx {
            self.set_save_format(false);
        }
    }
}
