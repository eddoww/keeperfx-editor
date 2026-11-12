//! The map dialogs: Level properties, Resize / transform, Regenerate and Symmetry.
use super::panels::{rgba_tex, texture_pack_names, texture_set_name, PropsDialog};
use super::*;
use crate::document::Regen;
use kfx_formats::levelinfo::{LevelInfo, LevelKind};
use kfx_map::transform::{Symmetry, SymmetryMode, Transform};
use kfx_prefab::i18n::{trd, trf};

impl EditorApp {
    pub fn open_properties(&mut self) {
        let Some(d) = &self.doc else { return };
        let install = self.install().map(Path::to_path_buf);
        let target = install.and_then(|i| document::find_pack_cfg(&i, &d.doc.folder, d.doc.level));
        let mut info = None;
        if let Some(cfg) = &target {
            info = kfx_formats::campaign::Campaign::load(cfg)
                .ok()
                .and_then(|c| c.map_info(d.doc.level));
        }
        let target = target.filter(|_| info.is_some());
        let info = info
            .or_else(|| {
                level_bin::find_file(&d.doc.folder, d.doc.level, "lof")
                    .and_then(|p| Lof::load(&p).ok())
                    .map(|l| l.info())
            })
            .unwrap_or_default();
        let names = texture_pack_names();
        let mut thumbs = vec![];
        if let Some(a) = &self.assets {
            let data = a.install.data_dir();
            for set in 0..64u32 {
                if !kfx_gfx::textures::set_files_exist(&data, set).0 {
                    if set > 16 {
                        break;
                    }
                    continue;
                }
                let tex = kfx_gfx::TextureSet::load_std(&a.install, set)
                    .ok()
                    .map(|ts| {
                        let ids: Vec<u16> = [
                            kfx_map::autotile::kinds::EARTH,
                            kfx_map::autotile::kinds::WALLDRAPE,
                            kfx_map::autotile::kinds::CLAIMED,
                            kfx_map::autotile::kinds::PATH,
                        ]
                        .iter()
                        .filter_map(|&k| {
                            assets::slab_preview_textures(&d.gd, &self.cubes, k as usize)[4]
                        })
                        .collect();
                        let mut img = kfx_gfx::Rgba::new(64, 64);
                        for (n, id) in ids.iter().enumerate() {
                            if let Some(t) = ts.id_rgba(*id, 0, &a.anim, &a.palette) {
                                img.blit(&t, (n % 2) as i32 * 32, (n / 2) as i32 * 32);
                            }
                        }
                        rgba_tex(&self.ctx, "set", &img)
                    })
                    .flatten();
                thumbs.push((set, texture_set_name(&names, set), tex));
            }
        }
        let texture = d.doc.texture_id;
        let id_text = info
            .name_id
            .as_deref()
            .and_then(|i| i.trim().parse::<i64>().ok())
            .and_then(|id| self.string_text(id));
        self.props_dialog = Some(PropsDialog {
            info,
            texture,
            target,
            thumbs,
            error: None,
            id_text,
            land: None,
        });
    }

    /// Write the level information (`.lof`, or the campaign `[mapNNNNN]` block) and set the
    /// texture set (an undoable change of the map).
    pub fn apply_properties(
        &mut self,
        info: &LevelInfo,
        texture: u8,
        target: Option<&Path>,
    ) -> Result<(), String> {
        let d = self.doc.as_ref().ok_or("no map")?;
        let (folder, level, dims) = (d.doc.folder.clone(), d.doc.level, d.dims());
        let mut info = info.clone();
        info.mapsize = if (dims.w, dims.h) != (85, 85) || info.mapsize.is_some() {
            Some((dims.w as i32, dims.h as i32))
        } else {
            None
        };
        let backups = self.session_backups.clone();
        let mut written = vec![];
        match target {
            Some(cfg) => {
                let mut c =
                    kfx_formats::campaign::Campaign::load(cfg).map_err(|e| e.to_string())?;
                c.set_map_info(level, &info);
                backups.ensure(cfg)?;
                c.save(cfg).map_err(|e| format!("{}: {e}", cfg.display()))?;
            }
            None => {
                std::fs::create_dir_all(&folder)
                    .map_err(|e| format!("{}: {e}", folder.display()))?;
                let p = level_bin::find_file(&folder, level, "lof")
                    .unwrap_or_else(|| folder.join(level_bin::file_name(level, "lof")));
                let mut l = match Lof::load(&p) {
                    Ok(l) => l,
                    Err(_) if !p.exists() => Lof::new(&LevelInfo::default()),
                    Err(e) => return Err(format!("{}: {e}", p.display())),
                };
                l.set_info(&info);
                backups.ensure(&p)?;
                l.save(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                written.push(p);
                // the original game's level name file: kept in step when the level has one, or
                // written for a map saved in the original format
                let lif = level_bin::find_file(&folder, level, "lif");
                if let Some(name) = info.name_text.as_deref().filter(|n| !n.trim().is_empty()) {
                    if lif.is_some() || self.save_format_legacy() {
                        let p =
                            lif.unwrap_or_else(|| folder.join(level_bin::file_name(level, "lif")));
                        let mut f = match kfx_formats::levelinfo::Lif::load(&p) {
                            Ok(f) => f,
                            Err(_) if !p.exists() => kfx_formats::levelinfo::Lif::parse(b""),
                            Err(e) => return Err(format!("{}: {e}", p.display())),
                        };
                        f.set_entry(level as i32, name.trim());
                        backups.ensure(&p)?;
                        f.save(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                        written.push(p);
                    }
                }
            }
        }
        self.refresh_fingerprint_for(&written);
        let d = self.doc.as_ref().ok_or("no map")?;
        if texture != d.doc.texture_id {
            let doc = self.doc.as_mut().unwrap();
            doc.end_stroke();
            let (_, rf) = doc.edit(&trd("kfx_editor.texture_set", "Texture set"), |m, _, _| {
                m.set_texture_id(texture)
            });
            self.after_change(rf);
        }
        self.status = trf(
            "kfx_editor.level_information_written_to_p",
            "Level information written to {p}",
            &[(
                "p",
                &target
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| trd("kfx_editor.the_lof", "the .lof")),
            )],
        );
        Ok(())
    }

    pub(super) fn properties_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.props_dialog else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let mut pick_land = false;
        let mut speech: Option<String> = None;
        egui::Modal::new(egui::Id::unique("props")).show(ctx, |ui| {
            ui.set_width(640.0);
            ui.heading(
                RichText::new(trd("kfx_editor.level_properties", "Level properties")).color(GOLD),
            );
            ui.label(
                RichText::new(match &dlg.target {
                    Some(p) => trf(
                        "kfx_editor.written_to_the_campaign_file_p_map_block",
                        "Written to the campaign file {p} [map block]",
                        &[("p", &p.display())],
                    ),
                    None => trd(
                        "kfx_editor.written_to_the_level_s_lof_file",
                        "Written to the level's .lof file",
                    ),
                })
                .small()
                .color(TEXT_DIM),
            );
            let i = &mut dlg.info;
            let text = |ui: &mut egui::Ui, label: &str, v: &mut Option<String>| {
                ui.label(label);
                let mut s = v.clone().unwrap_or_default();
                if ui
                    .add(egui::TextEdit::singleline(&mut s).desired_width(380.0))
                    .changed()
                {
                    *v = (!s.is_empty()).then_some(s);
                }
                ui.end_row();
            };
            egui::Grid::new("props-grid")
                .num_columns(2)
                .spacing([10.0, 5.0])
                .show(ui, |ui| {
                    text(ui, &trd("kfx_editor.name", "Name"), &mut i.name_text);
                    text(
                        ui,
                        &trd("kfx_editor.name_text_id", "Name text ID"),
                        &mut i.name_id,
                    );
                    if let Some(t) = &dlg.id_text {
                        ui.label("");
                        ui.label(
                            RichText::new(trf(
                                "kfx_editor.t_string_tables_of_the_level",
                                "= \u{201C}{t}\u{201D} (string tables of the level)",
                                &[("t", &t)],
                            ))
                            .small()
                            .color(TEXT_DIM),
                        );
                        ui.end_row();
                    }
                    text(ui, &trd("kfx_editor.author", "Author"), &mut i.author);
                    text(
                        ui,
                        &trd("kfx_editor.description", "Description"),
                        &mut i.description,
                    );
                    text(ui, &trd("kfx_editor.date", "Date"), &mut i.date);
                    text(ui, &trd("kfx_editor.ensign", "Ensign"), &mut i.ensign);
                    ui.label(trd("kfx_editor.kind", "Kind"));
                    ui.horizontal(|ui| {
                        for k in [
                            LevelKind::Single,
                            LevelKind::Multi,
                            LevelKind::Bonus,
                            LevelKind::Extra,
                            LevelKind::Free,
                        ] {
                            let mut on = i.kinds.contains(&k);
                            if ui.checkbox(&mut on, k.name()).changed() {
                                if on {
                                    i.kinds.push(k);
                                } else {
                                    i.kinds.retain(|x| *x != k);
                                }
                            }
                        }
                    });
                    ui.end_row();
                    ui.label(trd("kfx_editor.players", "Players"));
                    let mut p = i.players.unwrap_or(1);
                    if ui.add(egui::DragValue::new(&mut p).range(1..=8)).changed() {
                        i.players = Some(p);
                    }
                    ui.end_row();
                    ui.label(trd(
                        "kfx_editor.ensign_position_zoom",
                        "Ensign position / zoom",
                    ));
                    ui.horizontal(|ui| {
                        let (mut x, mut y) = i.ensign_pos.unwrap_or((0, 0));
                        let (mut zx, mut zy) = i.ensign_zoom.unwrap_or((0, 0));
                        let mut ch = ui.add(egui::DragValue::new(&mut x)).changed();
                        ch |= ui.add(egui::DragValue::new(&mut y)).changed();
                        if ch {
                            i.ensign_pos = Some((x, y));
                        }
                        let mut chz = ui.add(egui::DragValue::new(&mut zx)).changed();
                        chz |= ui.add(egui::DragValue::new(&mut zy)).changed();
                        if chz {
                            i.ensign_zoom = Some((zx, zy));
                        }
                    });
                    ui.end_row();
                    let pair =
                        |ui: &mut egui::Ui, label: &str, v: &mut Option<(String, String)>| {
                            ui.label(label);
                            let (mut a, mut b) = v.clone().unwrap_or_default();
                            ui.horizontal(|ui| {
                                let c1 = ui
                                    .add(egui::TextEdit::singleline(&mut a).desired_width(180.0))
                                    .changed();
                                let c2 = ui
                                    .add(egui::TextEdit::singleline(&mut b).desired_width(180.0))
                                    .changed();
                                if c1 || c2 {
                                    *v = (!a.is_empty() || !b.is_empty()).then_some((a, b));
                                }
                            });
                            ui.end_row();
                        };
                    pair(
                        ui,
                        &trd("kfx_editor.speech_intro_outro", "Speech (intro, outro)"),
                        &mut i.speech,
                    );
                    // listen to the level speech (campaign [speech] folder)
                    ui.label("");
                    ui.horizontal(|ui| {
                        let (a, b) = i.speech.clone().unwrap_or_default();
                        if ui
                            .add_enabled(
                                !a.is_empty(),
                                egui::Button::new(trd(
                                    "kfx_editor.preview_level_speech_intro",
                                    "Preview level speech (intro)",
                                )),
                            )
                            .on_hover_text(trd(
                                "kfx_editor.plays_the_file_from_the_campaign_s_speech_folder",
                                "Plays the file from the campaign's speech folder, like the \
                                land view does",
                            ))
                            .clicked()
                        {
                            speech = Some(a);
                        }
                        if ui
                            .add_enabled(
                                !b.is_empty(),
                                egui::Button::new(trd("kfx_editor.outro", "Outro")),
                            )
                            .clicked()
                        {
                            speech = Some(b);
                        }
                        if self.status.starts_with("Playing speech")
                            || self.status.starts_with("Speech")
                        {
                            ui.label(RichText::new(&self.status).small().color(TEXT_DIM));
                        }
                    });
                    ui.end_row();
                    pair(
                        ui,
                        &trd(
                            "kfx_editor.land_view_picture_window",
                            "Land view (picture, window)",
                        ),
                        &mut i.land_view,
                    );
                    ui.label("");
                    if ui
                        .button(trd(
                            "kfx_editor.pick_on_the_land_view",
                            "Pick on the land view…",
                        ))
                        .on_hover_text(trd(
                            "kfx_editor.choose_the_land_view_picture_and_click_on_it_to",
                            "Choose the land view picture and click on it to place the ensign",
                        ))
                        .clicked()
                    {
                        pick_land = true;
                    }
                    ui.end_row();
                    ui.label(trd("kfx_editor.map_size", "Map size"));
                    ui.label(
                        i.mapsize
                            .map(|(w, h)| {
                                trf(
                                    "kfx_editor.w_x_h_map_resize_to_change",
                                    "{w} x {h} (Map > Resize to change)",
                                    &[("w", &w), ("h", &h)],
                                )
                            })
                            .unwrap_or_else(|| "85 x 85".into()),
                    );
                    ui.end_row();
                });
            ui.separator();
            ui.label(RichText::new(trd("kfx_editor.texture_set", "Texture set")).color(GOLD));
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (set, name, tex) in &dlg.thumbs {
                        ui.vertical(|ui| {
                            let sel = dlg.texture as u32 == *set;
                            let (rect, resp) =
                                ui.allocate_exact_size(EVec2::splat(68.0), egui::Sense::click());
                            if let Some(t) = tex {
                                ui.painter().image(
                                    t.id(),
                                    rect.shrink(2.0),
                                    egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                            }
                            ui.painter().rect_stroke(
                                rect,
                                2.0,
                                egui::Stroke::new(
                                    if sel { 2.5 } else { 1.0 },
                                    if sel {
                                        Color32::from_rgb(220, 60, 60)
                                    } else {
                                        GOLD_DIM
                                    },
                                ),
                                egui::StrokeKind::Inside,
                            );
                            ui.label(RichText::new(format!("{set}: {name}")).small());
                            if resp.clicked() {
                                dlg.texture = *set as u8;
                            }
                        });
                    }
                });
            });
            if let Some(e) = &dlg.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.horizontal(|ui| {
                if ui.button(trd("kfx_editor.apply", "Apply")).clicked() {
                    ok = true;
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        self.land_picker_ui(ctx, pick_land);
        if let Some(s) = speech {
            self.preview_level_speech(&s);
        }
        if cancel {
            self.props_dialog = None;
        } else if ok {
            let d = self.props_dialog.take().unwrap();
            if let Err(e) = self.apply_properties(&d.info, d.texture, d.target.as_deref()) {
                self.props_dialog = Some(PropsDialog {
                    error: Some(e),
                    ..d
                });
            }
        }
    }

    /// Grow / shrink the map at each edge (one undo step).
    pub fn resize_map(
        &mut self,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        fill: &str,
    ) -> Result<(), String> {
        let doc = self.doc.as_mut().ok_or("no map")?;
        // terrain.cfg calls earth DIRT; the dialog offers the game's script name EARTH too
        let fill_kind = doc
            .gd
            .slab_id(fill)
            .or_else(|| {
                fill.eq_ignore_ascii_case("EARTH")
                    .then_some(kfx_map::autotile::kinds::EARTH as usize)
            })
            .ok_or_else(|| format!("no slab named {fill}"))? as u16;
        doc.end_stroke();
        let r = doc.doc.clone().resize(
            &doc.gd,
            left,
            top,
            right,
            bottom,
            fill_kind,
            kfx_map::autotile::PLAYER_NEUTRAL,
        );
        r.map_err(|e| e.to_string())?;
        doc.begin_stroke(&trd("kfx_editor.resize_map", "Resize map"));
        let (_, rf) = doc.op(
            &trd("kfx_editor.resize_map", "Resize map"),
            |d, g| match d.resize(
                g,
                left,
                top,
                right,
                bottom,
                fill_kind,
                kfx_map::autotile::PLAYER_NEUTRAL,
            ) {
                Ok(p) => ((), p),
                Err(_) => ((), kfx_map::patch::Patch::new()),
            },
        );
        // decorate the new / regenerated edge
        let dims = self.doc.as_ref().unwrap().dims();
        let doc = self.doc.as_mut().unwrap();
        let dec = doc.decorator.clone();
        let (_, rf2) = doc.edit(&trd("kfx_editor.resize_map", "Resize map"), |d, _, _| {
            d.decorate(
                &dec,
                SlabRect {
                    x0: 0,
                    y0: 0,
                    x1: dims.w as i32 - 1,
                    y1: dims.h as i32 - 1,
                },
            )
        });
        doc.end_stroke();
        self.selected.clear();
        self.selection = None;
        self.after_change(rf.merge(rf2));
        self.status = trf(
            "kfx_editor.map_resized_to_w_x_h",
            "Map resized to {w}x{h}",
            &[("w", &dims.w), ("h", &dims.h)],
        );
        Ok(())
    }

    /// Flip / rotate the whole map, or the slab selection when there is one.
    pub fn transform(&mut self, t: Transform, selection_only: bool) -> Result<(), String> {
        let sel = self.selection;
        let doc = self.doc.as_mut().ok_or("no map")?;
        doc.end_stroke();
        let name = format!("{t:?}");
        let rect = if selection_only {
            let ((x0, y0), (x1, y1)) = sel.ok_or(trd(
                "kfx_editor.select_slabs_first_select_tool",
                "select slabs first (Select tool)",
            ))?;
            Some(SlabRect { x0, y0, x1, y1 })
        } else {
            None
        };
        if let Some(r) = rect {
            if t.swaps_axes() && r.x1 - r.x0 != r.y1 - r.y0 {
                return Err(trd(
                    "kfx_editor.a_quarter_turn_of_a_selection_needs_a_square_sel",
                    "a quarter turn of a selection needs a square selection",
                ));
            }
        }
        doc.begin_stroke(&name);
        let (_, rf) = doc.op(&name, |d, g| match rect {
            Some(r) => ((), d.transform_rect(g, r, t).unwrap_or_default()),
            None => ((), d.transform_map(g, t)),
        });
        let dims = doc.dims();
        let dec = doc.decorator.clone();
        let area = rect.map(|r| r.grow(1)).unwrap_or(SlabRect {
            x0: 0,
            y0: 0,
            x1: dims.w as i32 - 1,
            y1: dims.h as i32 - 1,
        });
        let (_, rf2) = doc.edit(&name, |d, _, _| d.decorate(&dec, area));
        doc.end_stroke();
        self.selected.clear();
        if rect.is_none() {
            self.selection = None;
        }
        self.after_change(rf.merge(rf2));
        self.status = trf("kfx_editor.name_done", "{name} done", &[("name", &name)]);
        Ok(())
    }

    pub(super) fn resize_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.resize_dialog else {
            return;
        };
        let dims = self
            .doc
            .as_ref()
            .map(|d| d.dims())
            .unwrap_or(MapDims::new(85, 85));
        let mut ok = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("resize")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(RichText::new(trd("kfx_editor.resize_map", "Resize map")).color(GOLD));
            ui.label(trd(
                "kfx_editor.slabs_to_add_negative_remove_at_each_edge_things",
                "Slabs to add (negative: remove) at each edge; things move with the \
                map, things cut off are removed.",
            ));
            egui::Grid::new("resize-grid")
                .num_columns(2)
                .show(ui, |ui| {
                    for (l, v) in [
                        (trd("kfx_editor.west", "West"), &mut dlg.left),
                        (trd("kfx_editor.north", "North"), &mut dlg.top),
                        (trd("kfx_editor.east", "East"), &mut dlg.right),
                        (trd("kfx_editor.south", "South"), &mut dlg.bottom),
                    ] {
                        ui.label(l);
                        ui.add(egui::DragValue::new(v).range(-169..=169));
                        ui.end_row();
                    }
                    ui.label(trd("kfx_editor.fill_slab", "Fill slab"));
                    ui.text_edit_singleline(&mut dlg.fill);
                    ui.end_row();
                });
            let (nw, nh) = (
                dims.w as i32 + dlg.left + dlg.right,
                dims.h as i32 + dlg.top + dlg.bottom,
            );
            let valid = (1..=170).contains(&nw) && (1..=170).contains(&nh);
            ui.label(
                RichText::new(trf(
                    "kfx_editor.w_x_h_nw_x_nh",
                    "{w}x{h} -> {nw}x{nh}",
                    &[("w", &dims.w), ("h", &dims.h), ("nw", &nw), ("nh", &nh)],
                ))
                .color(if valid {
                    theme::TEXT
                } else {
                    Color32::LIGHT_RED
                }),
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(valid, egui::Button::new(trd("kfx_editor.resize", "Resize")))
                    .clicked()
                {
                    ok = true;
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.resize_dialog = None;
        } else if ok {
            let d = self.resize_dialog.take().unwrap();
            if let Err(e) = self.resize_map(d.left, d.top, d.right, d.bottom, &d.fill) {
                self.status = trf(
                    "kfx_editor.resize_failed_e",
                    "Resize failed: {e}",
                    &[("e", &e)],
                );
            }
        }
    }

    /// Regenerate the map or the selection.
    pub fn regenerate(&mut self, what: Regen, selection_only: bool) -> Result<(), String> {
        let sel = self.selection;
        let doc = self.doc.as_mut().ok_or("no map")?;
        let dims = doc.dims();
        let rect = if selection_only {
            let ((x0, y0), (x1, y1)) =
                sel.ok_or(trd("kfx_editor.select_slabs_first", "select slabs first"))?;
            SlabRect { x0, y0, x1, y1 }
        } else {
            SlabRect {
                x0: 0,
                y0: 0,
                x1: dims.w as i32 - 1,
                y1: dims.h as i32 - 1,
            }
        };
        let name = if selection_only {
            trd("kfx_editor.regenerate_selection", "Regenerate selection")
        } else {
            trd("kfx_editor.regenerate_all_slabs", "Regenerate all slabs")
        };
        let rf = doc.regenerate(&name, rect, what);
        self.prune_selection();
        self.after_change(rf);
        self.status = trf("kfx_editor.name_done", "{name} done", &[("name", &name)]);
        Ok(())
    }

    pub(super) fn regen_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.regen_dialog else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("regen")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading(
                RichText::new(if dlg.selection {
                    trd("kfx_editor.regenerate_selection", "Regenerate selection")
                } else {
                    trd("kfx_editor.regenerate_all_slabs", "Regenerate all slabs")
                })
                .color(GOLD),
            );
            ui.checkbox(
                &mut dlg.what.columns,
                trd(
                    "kfx_editor.columns_from_the_slab_sets_drops_hand_made_colum",
                    "Columns (from the slab sets; drops hand-made columns)",
                ),
            );
            ui.checkbox(
                &mut dlg.what.decoration,
                trd(
                    "kfx_editor.decoration_random_cube_variants",
                    "Decoration (random cube variants)",
                ),
            );
            ui.checkbox(
                &mut dlg.what.objects,
                trd(
                    "kfx_editor.slab_objects_torches_room_furniture_slab_lights",
                    "Slab objects (torches, room furniture, slab lights)",
                ),
            );
            ui.checkbox(
                &mut dlg.what.liquid,
                trd(
                    "kfx_editor.wibble_and_water_lava_wlb",
                    "Wibble and water / lava (.wlb)",
                ),
            );
            ui.checkbox(
                &mut dlg.what.doodads,
                trd(
                    "kfx_editor.random_doodads_map_random_doodads",
                    "Random doodads (Map > Random doodads)",
                ),
            );
            ui.colored_label(
                Color32::from_rgb(240, 180, 80),
                trd(
                    "kfx_editor.this_replaces_what_the_map_has_for_those_aspects",
                    "This replaces what the map has for those aspects. It can be undone.",
                ),
            );
            ui.horizontal(|ui| {
                if ui
                    .button(if dlg.confirm {
                        trd("kfx_editor.yes_regenerate", "Yes, regenerate")
                    } else {
                        trd("kfx_editor.regenerate", "Regenerate")
                    })
                    .clicked()
                {
                    if dlg.confirm {
                        ok = true;
                    } else {
                        dlg.confirm = true;
                    }
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.regen_dialog = None;
        } else if ok {
            let d = self.regen_dialog.take().unwrap();
            if let Err(e) = self.regenerate(d.what, d.selection) {
                self.status = e;
            }
        }
    }

    pub(super) fn apply_decor_prefs(&mut self) {
        let p = document::DecorPrefs {
            enabled: self.settings.decoration,
            families: self.settings.decor_families.clone(),
        };
        if let Some(d) = self.doc.as_mut() {
            d.set_decor_prefs(&p);
            d.rounding = self.settings.rounding;
        }
    }

    pub(super) fn symmetry_ui(&mut self, ctx: &egui::Context) {
        if !self.show_symmetry {
            return;
        }
        let mut open = true;
        let mut mode = self.symmetry.as_ref().map(|s| s.mode);
        let mut owners = self.symmetry.as_ref().map(|s| s.owners.clone());
        egui::Window::new(trd("kfx_editor.symmetry", "Symmetry"))
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut mode, None, trd("kfx_editor.off", "Off"));
                    for (m, l) in [
                        (SymmetryMode::NorthSouth, trd("kfx_editor.n_s", "N-S")),
                        (SymmetryMode::EastWest, trd("kfx_editor.e_w", "E-W")),
                        (SymmetryMode::FourWay, trd("kfx_editor.4_way", "4-way")),
                        (
                            SymmetryMode::Rot2,
                            trd("kfx_editor.rotational_2", "Rotational 2"),
                        ),
                        (
                            SymmetryMode::Rot4,
                            trd("kfx_editor.rotational_4", "Rotational 4"),
                        ),
                    ] {
                        ui.selectable_value(&mut mode, Some(m), l);
                    }
                });
                if let (Some(m), Some(o)) = (mode, owners.as_mut()) {
                    let fresh = Symmetry::new(m);
                    if o.len() != fresh.owners.len() {
                        *o = fresh.owners;
                    }
                    ui.label(trd(
                        "kfx_editor.owner_in_each_sector_for_an_owner_of_sector_0",
                        "Owner in each sector for an owner of sector 0:",
                    ));
                    egui::Grid::new("sym-owners").show(ui, |ui| {
                        ui.label("");
                        for s in 0..o.len() {
                            ui.label(trf("kfx_editor.sector_s", "sector {s}", &[("s", &s)]));
                        }
                        ui.end_row();
                        for p in [0usize, 1, 2, 3, 4, 6, 7, 8] {
                            ui.label(PLAYERS[p].0);
                            for row in o.iter_mut() {
                                egui::ComboBox::from_id_salt(("so", p, row.as_ptr() as usize))
                                    .selected_text(PLAYERS[row[p] as usize].0)
                                    .width(110.0)
                                    .show_ui(ui, |ui| {
                                        for q in 0..9u8 {
                                            ui.selectable_value(
                                                &mut row[p],
                                                q,
                                                PLAYERS[q as usize].0,
                                            );
                                        }
                                    });
                            }
                            ui.end_row();
                        }
                    });
                } else if let Some(m) = mode {
                    owners = Some(Symmetry::new(m).owners);
                }
            });
        self.show_symmetry = open;
        let new = mode.map(|m| Symmetry {
            mode: m,
            owners: owners.unwrap_or_else(|| Symmetry::new(m).owners),
        });
        if new != self.symmetry {
            self.symmetry = new;
            self.store_symmetry();
        }
    }

    pub(super) fn dialogs_extra(&mut self, ctx: &egui::Context) {
        self.properties_ui(ctx);
        self.resize_ui(ctx);
        self.regen_ui(ctx);
        self.symmetry_ui(ctx);
    }
}
