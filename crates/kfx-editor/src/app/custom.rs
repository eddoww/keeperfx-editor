//! Sidecar-backed editor features (`kfx_map::sidecar`, saved as `mapNNNNN.kfxed.toml`):
//! custom slabs (a "Custom" palette tab, created from a slab of the map or from nine columns,
//! placed and painted like normal slabs), per-subtile column editing stored as a custom slab,
//! locked regions, notes pinned to slabs, and the map's own symmetry / decoration settings.
//! Every change of the sidecar is an undo step of the map.
use super::*;
use kfx_map::sidecar::{CustomSlab, LockedRegion, Note};
use kfx_prefab::i18n::{trd, trf, trn};

/// A palette entry of the Custom tab.
pub struct CustomTile {
    pub id: u16,
    pub name: String,
    pub tooltip: String,
    pub tex: Option<egui::TextureHandle>,
}

/// "New custom slab" from nine columnset columns.
#[derive(Clone, Debug)]
pub struct CustomDialog {
    pub name: String,
    pub recognized_as: u16,
    /// Columnset ids, row-major.
    pub columns: [usize; 9],
    pub error: Option<String>,
}

/// "Edit this column" of a subtile.
#[derive(Clone, Debug)]
pub struct ColumnDialog {
    pub stl: (usize, usize),
    pub cubes: [u16; 8],
    pub floor: u16,
    pub error: Option<String>,
}

/// "Add note" at a slab.
#[derive(Clone, Debug)]
pub struct NoteDialog {
    pub at: (i32, i32),
    pub text: String,
}

/// The map's decoration settings being edited.
#[derive(Clone, Debug)]
pub struct DecorDialog {
    pub seed: Option<u64>,
    pub enabled: bool,
    pub disabled: Vec<String>,
}

impl EditorApp {
    // ------------------------------------------------------------------ previews

    /// 32x32 RGBA of an atlas texture id (level texture set, frame 0).
    pub(super) fn atlas_tile(&self, id: u16) -> Option<Vec<u8>> {
        let atlas = self.atlas.as_ref()?;
        let phys = *atlas.lookup.get(id as usize)?;
        let (layer, local) = ((phys / LAYER_TILES) as usize, phys % LAYER_TILES);
        let px = atlas.layers.get(layer)?.first()?;
        let (tx, ty) = ((local % TILES_PER_ROW) * 32, (local / TILES_PER_ROW) * 32);
        let mut out = vec![0u8; 32 * 32 * 4];
        for y in 0..32 {
            let o = (((ty + y) * LAYER_W + tx) * 4) as usize;
            out[(y * 32 * 4) as usize..((y + 1) * 32 * 4) as usize]
                .copy_from_slice(&px[o..o + 32 * 4]);
        }
        Some(out)
    }

    /// 3x3 top textures of nine columns (96x96).
    pub(super) fn columns_preview(&self, cols: &[level_bin::Column; 9]) -> kfx_gfx::Rgba {
        let mut img = kfx_gfx::Rgba::new(96, 96);
        for (i, c) in cols.iter().enumerate() {
            let id =
                assets::column_top_texture(&self.cubes, c.solidmask, c.floor_texture, &c.cubes);
            let Some(px) = self.atlas_tile(id) else {
                continue;
            };
            let (ox, oy) = ((i % 3) * 32, (i / 3) * 32);
            for y in 0..32 {
                let o = ((oy + y) * 96 + ox) * 4;
                img.data[o..o + 32 * 4].copy_from_slice(&px[y * 32 * 4..(y + 1) * 32 * 4]);
            }
            // height cue: a darker rim for columns taller than the floor
            let h = assets::floor_height(c.solidmask);
            if h >= 4 {
                for k in 0..32 {
                    for (x, y) in [(ox + k, oy), (ox, oy + k)] {
                        let o = (y * 96 + x) * 4;
                        img.data[o] /= 2;
                        img.data[o + 1] /= 2;
                        img.data[o + 2] /= 2;
                    }
                }
            }
        }
        img
    }

    /// The Custom palette tab from the sidecar definitions.
    pub(super) fn build_custom_palette(&mut self) {
        let Some(d) = &self.doc else {
            self.custom_palette.clear();
            return;
        };
        let defs = d.doc.sidecar.custom_slabs.clone();
        let gd = d.gd.clone();
        let used: HashMap<u16, usize> =
            d.doc
                .custom_cells
                .iter()
                .filter(|&&c| c != 0)
                .fold(HashMap::new(), |mut m, &c| {
                    *m.entry(c).or_default() += 1;
                    m
                });
        self.custom_palette = defs
            .iter()
            .map(|c| {
                let img = self.columns_preview(&c.columns);
                CustomTile {
                    id: c.id,
                    name: c.name.clone(),
                    tooltip: trf(
                        "kfx_editor.name_custom_slab_id_seen_by_the_game_as_recogniz",
                        "{name} (custom slab {id}, seen by the game as \
                        {recognized_as}; {id2} on the map)",
                        &[
                            ("name", &c.name),
                            ("id", &c.id),
                            (
                                "recognized_as",
                                &gd.slab_name(c.recognized_as as usize).unwrap_or("?"),
                            ),
                            ("id2", &used.get(&c.id).copied().unwrap_or(0)),
                        ],
                    ),
                    tex: Some(self.ctx.load_texture(
                        "custom",
                        img_to_color(&img),
                        egui::TextureOptions::NEAREST,
                    )),
                }
            })
            .collect();
        if self
            .custom_brush
            .is_some_and(|b| !defs.iter().any(|c| c.id == b))
        {
            self.custom_brush = None;
        }
    }

    pub(super) fn custom_palette_ui(&mut self, ui: &mut egui::Ui) {
        let has_doc = self.doc.is_some();
        let mut act: Option<&'static str> = None;
        let mut chosen = None;
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let sel = self.selection.map(|(a, _)| a).or(self
                    .hover
                    .as_ref()
                    .map(|h| (h.slab.0 as i32, h.slab.1 as i32)));
                if ui
                    .add_enabled(
                        has_doc && self.selection.is_some(),
                        egui::Button::new(trd(
                            "kfx_editor.from_selected_slab",
                            "➕ From selected slab",
                        )),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.a_custom_slab_with_the_nine_columns_of_the_first",
                        "A custom slab with the nine columns of the first selected slab",
                    ))
                    .clicked()
                {
                    act = Some("from-slab");
                }
                let _ = sel;
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(trd("kfx_editor.from_columns", "➕ From columns…")),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.pick_nine_columns_of_the_column_set",
                        "Pick nine columns of the column set",
                    ))
                    .clicked()
                {
                    act = Some("from-scratch");
                }
                if ui
                    .add_enabled(
                        self.custom_brush.is_some(),
                        egui::Button::new(trd(
                            "kfx_editor.delete_definition",
                            "🗑 Delete definition",
                        )),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.slabs_using_it_keep_their_columns_until_regenera",
                        "Slabs using it keep their columns until regenerated",
                    ))
                    .clicked()
                {
                    act = Some("delete");
                }
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.custom_slabs_keep_their_nine_columns_when_the_ma",
                        "Custom slabs keep their nine columns when the map around them \
                        is regenerated; the game sees their \"recognized as\" kind.",
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
            });
            if self.custom_palette.is_empty() {
                ui.label(
                    RichText::new(if has_doc {
                        trd(
                            "kfx_editor.no_custom_slabs_yet_select_a_slab_select_tool_an",
                            "No custom slabs yet: select a slab (Select tool) and click \
                            \"From selected slab\"",
                        )
                    } else {
                        trd("kfx_editor.open_a_map", "Open a map")
                    })
                    .color(TEXT_DIM),
                );
                return;
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = EVec2::new(4.0, 4.0);
                        for t in &self.custom_palette {
                            let sel = self.custom_brush == Some(t.id);
                            let (rect, resp) = ui
                                .allocate_exact_size(EVec2::new(64.0, 64.0), egui::Sense::click());
                            let painter = ui.painter_at(rect.expand(2.0));
                            painter.rect_filled(rect, 2.0, Color32::from_rgb(12, 11, 10));
                            if let Some(tex) = &t.tex {
                                let r = egui::Rect::from_min_max(
                                    rect.min + EVec2::new(4.0, 3.0),
                                    rect.max - EVec2::new(4.0, 13.0),
                                );
                                let s = r.width().min(r.height());
                                painter.image(
                                    tex.id(),
                                    egui::Rect::from_center_size(r.center(), EVec2::splat(s)),
                                    egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                            }
                            let label: String = t.name.chars().take(11).collect();
                            painter.text(
                                egui::pos2(rect.center().x, rect.max.y - 6.0),
                                egui::Align2::CENTER_CENTER,
                                label,
                                egui::FontId::proportional(9.0),
                                theme::TEXT,
                            );
                            let stroke = if sel {
                                egui::Stroke::new(2.0, Color32::from_rgb(200, 50, 60))
                            } else if resp.hovered() {
                                egui::Stroke::new(1.0, GOLD)
                            } else {
                                egui::Stroke::new(1.0, Color32::from_rgb(60, 52, 44))
                            };
                            painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Outside);
                            resp.widget_info(|| {
                                egui::WidgetInfo::selected(egui::Role::Button, true, sel, &t.name)
                            });
                            if resp.on_hover_text(&t.tooltip).clicked() {
                                chosen = Some(t.id);
                            }
                        }
                    });
                });
        });
        if let Some(id) = chosen {
            self.select_custom_brush(id);
        }
        match act {
            Some("from-slab") => {
                if let Some(((x, y), _)) = self.selection {
                    if let Err(e) = self.create_custom_from_slab(x, y, None) {
                        self.status = e;
                    }
                }
            }
            Some("from-scratch") => self.open_custom_dialog(),
            Some("delete") => {
                if let Some(id) = self.custom_brush {
                    self.delete_custom(id);
                }
            }
            _ => {}
        }
    }

    /// Paint with custom slab `id`.
    pub fn select_custom_brush(&mut self, id: u16) {
        self.custom_brush = Some(id);
        self.style_brush = None;
        self.owner_only = false;
        self.category = Category::Custom;
        if matches!(
            self.tool,
            Tool::Eyedropper | Tool::Select | Tool::Place | Tool::Things
        ) {
            self.set_tool(Tool::Paint);
        }
    }

    /// Define a custom slab with the nine current columns of slab (x, y); it becomes the brush.
    pub fn create_custom_from_slab(
        &mut self,
        x: i32,
        y: i32,
        name: Option<&str>,
    ) -> Result<u16, String> {
        let d = self.doc.as_mut().ok_or("no map")?;
        if !d.in_map(x, y) {
            return Err(trd("kfx_editor.outside_the_map", "outside the map"));
        }
        let dm = d.dims();
        let i = dm.slab_index(x as usize, y as usize);
        let kind = d.doc.kinds[i];
        let cols: [level_bin::Column; 9] = std::array::from_fn(|k| {
            d.doc.columns[dm.stl_index(3 * x as usize + k % 3, 3 * y as usize + k / 3)]
        });
        let wib: [u8; 9] = std::array::from_fn(|k| {
            d.doc.wibble[dm.stl_index(3 * x as usize + k % 3, 3 * y as usize + k / 3)]
        });
        let name = name.map(str::to_string).unwrap_or_else(|| {
            format!(
                "{} {x},{y}",
                d.gd.slab_name(kind as usize).unwrap_or("slab")
            )
        });
        let mut def = CustomSlab::new(&name, kind, cols);
        def.wibble = wib;
        let wlb = d.doc.wlb[i];
        if wlb != d.gd.slab(kind as usize).wlb_type {
            def.liquid = Some(wlb);
        }
        d.end_stroke();
        let (id, rf) = d.edit(
            &trd("kfx_editor.new_custom_slab", "New custom slab"),
            |m, _, _| m.edit_sidecar(|s| s.add_custom_slab(def)),
        );
        self.after_change(rf);
        self.build_custom_palette();
        self.select_custom_brush(id);
        self.status = trf(
            "kfx_editor.custom_slab_id_name_defined_from_slab_x_y_paint",
            "Custom slab {id} \"{name}\" defined from slab ({x},{y}); paint it from the Custom tab",
            &[("id", &id), ("name", &name), ("x", &x), ("y", &y)],
        );
        Ok(id)
    }

    /// Define a custom slab from nine columnset columns.
    pub fn create_custom_from_columns(
        &mut self,
        name: &str,
        recognized_as: u16,
        cols: [usize; 9],
    ) -> Result<u16, String> {
        let d = self.doc.as_mut().ok_or("no map")?;
        let gd = d.gd.clone();
        let mut out = [level_bin::Column::default(); 9];
        for (k, &c) in cols.iter().enumerate() {
            out[k] = *gd.columnset.get(c).ok_or_else(|| {
                trf(
                    "kfx_editor.no_column_c_in_the_column_set_columnset_columns",
                    "no column {c} in the column set ({columnset} columns)",
                    &[("c", &c), ("columnset", &gd.columnset.len())],
                )
            })?;
        }
        if recognized_as as usize >= gd.slab_count() {
            return Err(trf(
                "kfx_editor.no_slab_kind_recognized_as",
                "no slab kind {recognized_as}",
                &[("recognized_as", &recognized_as)],
            ));
        }
        let def = CustomSlab::new(name, recognized_as, out);
        d.end_stroke();
        let (id, rf) = d.edit(
            &trd("kfx_editor.new_custom_slab", "New custom slab"),
            |m, _, _| m.edit_sidecar(|s| s.add_custom_slab(def)),
        );
        self.after_change(rf);
        self.build_custom_palette();
        self.select_custom_brush(id);
        Ok(id)
    }

    pub fn delete_custom(&mut self, id: u16) {
        let Some(d) = self.doc.as_mut() else { return };
        d.end_stroke();
        let (_, rf) = d.edit(
            &trd("kfx_editor.delete_custom_slab", "Delete custom slab"),
            |m, _, _| {
                m.edit_sidecar(|s| s.remove_custom_slab(id));
                for i in 0..m.custom_cells.len() {
                    if m.custom_cells[i] == id {
                        m.custom_cells[i] = 0;
                    }
                }
            },
        );
        self.after_change(rf);
        self.build_custom_palette();
        self.status = trf(
            "kfx_editor.custom_slab_id_deleted_its_slabs_are_normal_slab",
            "Custom slab {id} deleted (its slabs are normal slabs again; regenerate \
            to rebuild their columns)",
            &[("id", &id)],
        );
    }

    pub fn open_custom_dialog(&mut self) {
        let Some(d) = &self.doc else { return };
        // start from the slab set of the selected palette slab
        let kind = self.brush.unwrap_or(kfx_map::autotile::kinds::EARTH as u16);
        let base = kind as usize * kfx_map::gamedata::SLABSETS_PER_SLAB + assets::STYLE_ALL;
        let cols: [usize; 9] = std::array::from_fn(|k| {
            d.gd.slabset_raw
                .get(base)
                .map(|r| r[k].unsigned_abs() as usize)
                .unwrap_or(1)
        });
        self.custom_dialog = Some(CustomDialog {
            name: format!("Custom {}", d.doc.sidecar.free_custom_id()),
            recognized_as: kind,
            columns: cols,
            error: None,
        });
    }

    pub(super) fn custom_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = self.custom_dialog.clone() else {
            return;
        };
        let Some(gd) = self.gd().map(|g| g.clone()) else {
            return;
        };
        let mut dlg = dlg;
        let mut ok = false;
        let mut cancel = false;
        let previews: Vec<Option<egui::TextureHandle>> = dlg
            .columns
            .iter()
            .map(|&c| {
                let col = gd.columnset.get(c)?;
                let id = assets::column_top_texture(
                    &self.cubes,
                    col.solidmask,
                    col.floor_texture,
                    &col.cubes,
                );
                let px = self.atlas_tile(id)?;
                Some(self.ctx.load_texture(
                    "col",
                    egui::ColorImage::from_rgba_unmultiplied([32, 32], &px),
                    egui::TextureOptions::NEAREST,
                ))
            })
            .collect();
        egui::Modal::new(egui::Id::unique("custom-dialog")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(
                RichText::new(trd("kfx_editor.new_custom_slab", "New custom slab")).color(GOLD),
            );
            egui::Grid::new("custom-grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label(trd("kfx_editor.name", "Name"));
                    ui.text_edit_singleline(&mut dlg.name);
                    ui.end_row();
                    ui.label(trd("kfx_editor.recognized_as", "Recognized as"));
                    egui::ComboBox::from_id_salt("recog")
                        .selected_text(
                            gd.slab_name(dlg.recognized_as as usize)
                                .unwrap_or("?")
                                .to_string(),
                        )
                        .height(300.0)
                        .show_ui(ui, |ui| {
                            for k in 0..gd.slab_count() {
                                ui.selectable_value(
                                    &mut dlg.recognized_as,
                                    k as u16,
                                    format!("{k} {}", gd.slab_name(k).unwrap_or("?")),
                                );
                            }
                        });
                    ui.end_row();
                });
            ui.label(
                RichText::new(trd(
                    "kfx_editor.columns_column_set_ids_north_west_first",
                    "Columns (column set IDs, north-west first)",
                ))
                .small()
                .color(TEXT_DIM),
            );
            egui::Grid::new("cols9")
                .num_columns(3)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    for k in 0..9 {
                        ui.vertical(|ui| {
                            let (r, _) =
                                ui.allocate_exact_size(EVec2::splat(48.0), egui::Sense::hover());
                            match &previews[k] {
                                Some(t) => {
                                    ui.painter().image(
                                        t.id(),
                                        r,
                                        egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        Color32::WHITE,
                                    );
                                }
                                None => {
                                    ui.painter()
                                        .rect_filled(r, 0.0, Color32::from_rgb(40, 0, 0));
                                }
                            }
                            ui.add(
                                egui::DragValue::new(&mut dlg.columns[k])
                                    .range(0..=gd.columnset.len().saturating_sub(1))
                                    .prefix(trd("kfx_editor.col", "col ")),
                            )
                            .on_hover_text(trd(
                                "kfx_editor.column_of_the_column_set",
                                "Column of the column set",
                            ));
                        });
                        if k % 3 == 2 {
                            ui.end_row();
                        }
                    }
                });
            if let Some(e) = &dlg.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.horizontal(|ui| {
                if ui.button(trd("kfx_editor.create", "Create")).clicked() {
                    ok = true;
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.custom_dialog = None;
        } else if ok {
            match self.create_custom_from_columns(&dlg.name.clone(), dlg.recognized_as, dlg.columns)
            {
                Ok(_) => self.custom_dialog = None,
                Err(e) => {
                    dlg.error = Some(e);
                    self.custom_dialog = Some(dlg);
                }
            }
        } else {
            self.custom_dialog = Some(dlg);
        }
    }

    // ------------------------------------------------------------------ per-subtile columns

    /// Open "Edit this column" for subtile `stl`.
    pub fn open_column_dialog(&mut self, stl: (usize, usize)) {
        let Some(d) = &self.doc else { return };
        let dm = d.dims();
        if stl.0 >= dm.stl_w() || stl.1 >= dm.stl_h() {
            return;
        }
        let c = d.doc.columns[dm.stl_index(stl.0, stl.1)];
        self.column_dialog = Some(ColumnDialog {
            stl,
            cubes: c.cubes,
            floor: c.floor_texture,
            error: None,
        });
    }

    /// Replace the column of subtile `stl` (cubes bottom to top, floor texture). The slab becomes
    /// a custom slab (a new definition with this column) so that regeneration keeps it. One undo
    /// step.
    pub fn edit_column(
        &mut self,
        stl: (usize, usize),
        cubes: [u16; 8],
        floor: u16,
    ) -> Result<u16, String> {
        let d = self.doc.as_mut().ok_or("no map")?;
        let dm = d.dims();
        let (x, y) = ((stl.0 / 3) as i32, (stl.1 / 3) as i32);
        if !d.in_map(x, y) || stl.0 >= dm.stl_w() || stl.1 >= dm.stl_h() {
            return Err(trd("kfx_editor.outside_the_map", "outside the map"));
        }
        let i = dm.slab_index(x as usize, y as usize);
        let k = (stl.0 % 3) + 3 * (stl.1 % 3);
        let owner = d.doc.owners[i];
        let old_id = d.doc.custom_cells[i];
        let kind = d.doc.kinds[i];
        let mut def = match d.doc.custom_slab_at(x, y) {
            Some(c) => {
                let mut c = c.clone();
                if !c.name.ends_with("(edited)") {
                    c.name = format!("{} (edited)", c.name);
                }
                c
            }
            None => {
                let cols: [level_bin::Column; 9] = std::array::from_fn(|n| {
                    d.doc.columns[dm.stl_index(3 * x as usize + n % 3, 3 * y as usize + n / 3)]
                });
                let mut c = CustomSlab::new(
                    &format!(
                        "{} {x},{y} (edited)",
                        d.gd.slab_name(kind as usize).unwrap_or("slab")
                    ),
                    kind,
                    cols,
                );
                c.wibble = std::array::from_fn(|n| {
                    d.doc.wibble[dm.stl_index(3 * x as usize + n % 3, 3 * y as usize + n / 3)]
                });
                c
            }
        };
        let mut col = def.columns[k];
        col.cubes = cubes;
        col.floor_texture = floor;
        def.columns[k] = kfx_map::column::canonical_column(&col, true);
        def.id = 0;
        let still_used = d
            .doc
            .custom_cells
            .iter()
            .enumerate()
            .any(|(j, &c)| c == old_id && j != i);
        d.end_stroke();
        d.begin_stroke(&trd("kfx_editor.edit_column_1709", "Edit column"));
        let (id, rf) = d.edit(
            &trd("kfx_editor.edit_column_1709", "Edit column"),
            |m, _, _| {
                m.edit_sidecar(|s| {
                    if old_id != 0 && !still_used {
                        s.remove_custom_slab(old_id);
                    }
                    s.add_custom_slab(def)
                })
            },
        );
        let rf2 = d
            .apply(&[(x, y, Edit::Custom { id, owner })])
            .unwrap_or_default();
        d.end_stroke();
        self.after_change(rf.merge(rf2));
        self.build_custom_palette();
        self.status = trf(
            "kfx_editor.column_at_subtile_stl_stl2_changed_slab_x_y_is_n",
            "Column at subtile ({stl},{stl2}) changed; slab ({x},{y}) is now custom slab {id}",
            &[
                ("stl", &stl.0),
                ("stl2", &stl.1),
                ("x", &x),
                ("y", &y),
                ("id", &id),
            ],
        );
        Ok(id)
    }

    pub(super) fn column_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.column_dialog.clone() else {
            return;
        };
        let Some(gd) = self.gd().map(|g| g.clone()) else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("column-dialog")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(
                RichText::new(trf(
                    "kfx_editor.column_of_subtile_stl_stl2",
                    "Column of subtile ({stl}, {stl2})",
                    &[("stl", &dlg.stl.0), ("stl2", &dlg.stl.1)],
                ))
                .color(GOLD),
            );
            ui.label(
                RichText::new(trd(
                    "kfx_editor.cubes_from_the_bottom_0_empty_the_slab_becomes_a",
                    "Cubes from the bottom (0 = empty). The slab becomes a custom slab \
                    so regeneration keeps it.",
                ))
                .small()
                .color(TEXT_DIM),
            );
            egui::Grid::new("col-cubes").num_columns(2).show(ui, |ui| {
                for n in (0..8).rev() {
                    ui.label(trf("kfx_editor.cube_n", "cube {n}", &[("n", &(n + 1))]));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut dlg.cubes[n])
                                .range(0..=gd.cubes.len().saturating_sub(1) as u16),
                        );
                        ui.label(
                            RichText::new(
                                gd.cubes
                                    .get(dlg.cubes[n] as usize)
                                    .map(|c| c.name.clone())
                                    .unwrap_or_default(),
                            )
                            .small()
                            .color(TEXT_DIM),
                        );
                    });
                    ui.end_row();
                }
                ui.label(trd("kfx_editor.floor_texture", "Floor texture"));
                ui.add(egui::DragValue::new(&mut dlg.floor).range(0..=999));
                ui.end_row();
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
        if cancel {
            self.column_dialog = None;
        } else if ok {
            match self.edit_column(dlg.stl, dlg.cubes, dlg.floor) {
                Ok(_) => self.column_dialog = None,
                Err(e) => {
                    dlg.error = Some(e);
                    self.column_dialog = Some(dlg);
                }
            }
        } else {
            self.column_dialog = Some(dlg);
        }
    }

    // ------------------------------------------------------------------ locked regions

    /// Slab (x, y) is inside a locked region.
    pub fn is_locked(&self, x: i32, y: i32) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|d| d.doc.sidecar.locked_at(x, y).is_some())
    }

    /// Lock the slab selection (tools refuse to change it).
    pub fn lock_selection(&mut self, note: &str) -> Result<(), String> {
        let ((x0, y0), (x1, y1)) = self.selection.ok_or(trd(
            "kfx_editor.select_slabs_first_select_tool",
            "select slabs first (Select tool)",
        ))?;
        let d = self.doc.as_mut().ok_or("no map")?;
        d.end_stroke();
        let r = LockedRegion {
            x0,
            y0,
            x1,
            y1,
            note: note.to_string(),
        };
        let (_, rf) = d.edit(&trd("kfx_editor.lock_region", "Lock region"), |m, _, _| {
            m.edit_sidecar(|s| s.locked.push(r))
        });
        self.after_change(rf);
        self.status = trf(
            "kfx_editor.slabs_x0_y0_x1_y1_locked",
            "Slabs ({x0},{y0})-({x1},{y1}) locked",
            &[("x0", &x0), ("y0", &y0), ("x1", &x1), ("y1", &y1)],
        );
        Ok(())
    }

    /// Remove the locked regions that overlap the selection (or contain the hovered slab).
    pub fn unlock_selection(&mut self) -> Result<usize, String> {
        let ((x0, y0), (x1, y1)) = self
            .selection
            .or(self.hover.as_ref().map(|h| {
                (
                    (h.slab.0 as i32, h.slab.1 as i32),
                    (h.slab.0 as i32, h.slab.1 as i32),
                )
            }))
            .ok_or(trd("kfx_editor.select_slabs_first", "select slabs first"))?;
        let d = self.doc.as_mut().ok_or("no map")?;
        let hit = |l: &LockedRegion| l.x0 <= x1 && l.x1 >= x0 && l.y0 <= y1 && l.y1 >= y0;
        let n = d.doc.sidecar.locked.iter().filter(|l| hit(l)).count();
        if n == 0 {
            return Ok(0);
        }
        d.end_stroke();
        let (_, rf) = d.edit(
            &trd("kfx_editor.unlock_region", "Unlock region"),
            |m, _, _| m.edit_sidecar(|s| s.locked.retain(|l| !hit(l))),
        );
        self.after_change(rf);
        self.status = trn(
            "kfx_editor.n_locked_region_s_removed",
            n as i64,
            "{n} locked region removed",
            "{n} locked regions removed",
            &[],
        );
        Ok(n)
    }

    // ------------------------------------------------------------------ notes

    pub fn add_note(&mut self, at: (i32, i32), text: &str) -> Result<(), String> {
        let d = self.doc.as_mut().ok_or("no map")?;
        if !d.in_map(at.0, at.1) {
            return Err(trd("kfx_editor.outside_the_map", "outside the map"));
        }
        d.end_stroke();
        let n = Note {
            x: at.0,
            y: at.1,
            text: text.to_string(),
            color: None,
        };
        let (_, rf) = d.edit(&trd("kfx_editor.add_note_c868", "Add note"), |m, _, _| {
            m.edit_sidecar(|s| s.notes.push(n))
        });
        self.after_change(rf);
        Ok(())
    }

    pub fn remove_note(&mut self, i: usize) {
        let Some(d) = self.doc.as_mut() else { return };
        if i >= d.doc.sidecar.notes.len() {
            return;
        }
        d.end_stroke();
        let (_, rf) = d.edit(&trd("kfx_editor.remove_note", "Remove note"), |m, _, _| {
            m.edit_sidecar(|s| s.notes.remove(i))
        });
        self.after_change(rf);
    }

    pub fn open_note_dialog(&mut self) {
        let at = self.selection.map(|(a, _)| a).or(self
            .hover
            .as_ref()
            .map(|h| (h.slab.0 as i32, h.slab.1 as i32)));
        match at {
            Some(at) => {
                self.note_dialog = Some(NoteDialog {
                    at,
                    text: String::new(),
                })
            }
            None => {
                self.status = trd(
                    "kfx_editor.select_a_slab_or_hover_it_to_pin_a_note_to_it",
                    "Select a slab (or hover it) to pin a note to it",
                )
            }
        }
    }

    pub(super) fn note_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.note_dialog.clone() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("note-dialog")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(
                RichText::new(trf(
                    "kfx_editor.note_on_slab_at_at2",
                    "Note on slab ({at}, {at2})",
                    &[("at", &dlg.at.0), ("at2", &dlg.at.1)],
                ))
                .color(GOLD),
            );
            let r = ui.add(
                egui::TextEdit::multiline(&mut dlg.text)
                    .desired_rows(4)
                    .desired_width(400.0)
                    .hint_text(trd("kfx_editor.text", "text")),
            );
            if !r.has_focus() && dlg.text.is_empty() {
                r.request_focus();
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !dlg.text.trim().is_empty(),
                        egui::Button::new(trd("kfx_editor.add", "Add")),
                    )
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
            self.note_dialog = None;
        } else if ok {
            self.note_dialog = None;
            if let Err(e) = self.add_note(dlg.at, dlg.text.trim()) {
                self.status = e;
            }
        } else {
            self.note_dialog = Some(dlg);
        }
    }

    pub(super) fn notes_ui(&mut self, ui: &mut egui::Ui) {
        let Some(d) = &self.doc else {
            ui.label(RichText::new(trd("kfx_editor.no_map_open", "No map open")).color(TEXT_DIM));
            return;
        };
        let notes = d.doc.sidecar.notes.clone();
        let locked = d.doc.sidecar.locked.clone();
        ui.horizontal(|ui| {
            if ui
                .button(trd("kfx_editor.note_at_selection", "➕ Note at selection"))
                .clicked()
            {
                self.open_note_dialog();
            }
        });
        let mut go = None;
        let mut del = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if notes.is_empty() {
                    ui.label(
                        RichText::new(trd(
                            "kfx_editor.no_notes_map_add_note_pins_one_to_the_selected_s",
                            "No notes. Map > Add note pins one to the selected slab.",
                        ))
                        .color(TEXT_DIM),
                    );
                }
                for (i, n) in notes.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("🗑")
                            .on_hover_text(trd("kfx_editor.remove", "Remove"))
                            .clicked()
                        {
                            del = Some(i);
                        }
                        if ui
                            .add(
                                egui::Button::selectable(
                                    false,
                                    RichText::new(format!("({},{}) {}", n.x, n.y, n.text)),
                                )
                                .wrap(),
                            )
                            .on_hover_text(trd("kfx_editor.click_to_go_there", "Click to go there"))
                            .clicked()
                        {
                            go = Some((n.x, n.y));
                        }
                    });
                }
                if !locked.is_empty() {
                    ui.separator();
                    ui.label(
                        RichText::new(trd("kfx_editor.locked_regions", "Locked regions"))
                            .color(GOLD),
                    );
                    for l in &locked {
                        if ui
                            .add(egui::Button::selectable(
                                false,
                                format!("🔒 ({},{})-({},{}) {}", l.x0, l.y0, l.x1, l.y1, l.note),
                            ))
                            .clicked()
                        {
                            go = Some(((l.x0 + l.x1) / 2, (l.y0 + l.y1) / 2));
                        }
                    }
                }
            });
        if let Some(i) = del {
            self.remove_note(i);
        }
        if let Some((x, y)) = go {
            self.camera.target = Vec3::new(
                x as f32 * 3.0 + 1.5,
                self.camera.target.y,
                y as f32 * 3.0 + 1.5,
            );
            self.flash = Some((
                Vec3::new(
                    x as f32 * 3.0 + 1.5,
                    self.terrain.slab_top(x as usize, y as usize) as f32,
                    y as f32 * 3.0 + 1.5,
                ),
                Instant::now(),
            ));
        }
    }

    /// Note markers on the viewport; returns the text of the note under the pointer.
    pub(super) fn draw_notes(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        vp: Vec2,
        ppp: f32,
        pointer: Option<egui::Pos2>,
    ) -> Option<String> {
        let d = self.doc.as_ref()?;
        let to_ui = |p: Vec2| rect.min + EVec2::new(p.x / ppp, p.y / ppp);
        let mut hover = None;
        for n in &d.doc.sidecar.notes {
            let h = self
                .terrain
                .slab_top(n.x.max(0) as usize, n.y.max(0) as usize) as f32;
            let Some(p) = self.camera.project(
                Vec3::new(n.x as f32 * 3.0 + 1.5, h + 0.2, n.y as f32 * 3.0 + 1.5),
                vp,
            ) else {
                continue;
            };
            let c = to_ui(p);
            if !rect.contains(c) {
                continue;
            }
            let r = egui::Rect::from_center_size(c - EVec2::new(0.0, 9.0), EVec2::splat(16.0));
            let hot = pointer.is_some_and(|p| r.expand(3.0).contains(p));
            painter.rect(
                r,
                3.0,
                if hot {
                    Color32::from_rgb(255, 230, 120)
                } else {
                    Color32::from_rgb(240, 200, 80)
                },
                egui::Stroke::new(1.0, Color32::BLACK),
                egui::StrokeKind::Inside,
            );
            painter.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "📝",
                egui::FontId::proportional(11.0),
                Color32::BLACK,
            );
            painter.line_segment(
                [egui::pos2(c.x, r.max.y), c],
                egui::Stroke::new(1.0, Color32::from_rgb(240, 200, 80)),
            );
            if hot {
                hover = Some(trf(
                    "kfx_editor.note_on_slab_x_y_text",
                    "Note on slab ({x},{y}):\n{text}",
                    &[("x", &n.x), ("y", &n.y), ("text", &n.text)],
                ));
            }
        }
        hover
    }

    /// Hatching lines over the locked regions (overlay, drawn on top).
    pub(super) fn locked_overlay(&self, top: &mut Vec<LineVertex>) {
        let Some(d) = &self.doc else { return };
        let c = [255, 90, 70, 200];
        let lv = |p: Vec3| LineVertex {
            pos: p.to_array(),
            color: c,
        };
        for l in &d.doc.sidecar.locked {
            let (x0, z0, x1, z1) = (
                l.x0 as f32 * 3.0,
                l.y0 as f32 * 3.0,
                (l.x1 + 1) as f32 * 3.0,
                (l.y1 + 1) as f32 * 3.0,
            );
            let y = 5.1;
            for (a, b) in [
                ((x0, z0), (x1, z0)),
                ((x1, z0), (x1, z1)),
                ((x1, z1), (x0, z1)),
                ((x0, z1), (x0, z0)),
            ] {
                top.push(lv(Vec3::new(a.0, y, a.1)));
                top.push(lv(Vec3::new(b.0, y, b.1)));
            }
            // diagonal hatching every 1.5 subtiles: lines x - z = k
            let step = 1.5;
            let mut k = (x0 - z1) + step;
            while k < x1 - z0 {
                // segment of x - z = k inside the rectangle
                let (ax, bx) = (x0.max(k + z0), x1.min(k + z1));
                if ax < bx {
                    top.push(lv(Vec3::new(ax, y, ax - k)));
                    top.push(lv(Vec3::new(bx, y, bx - k)));
                }
                k += step;
            }
        }
    }

    // ------------------------------------------------------------------ map settings

    /// Persist the symmetry setting in the map's sidecar (an undo step).
    pub(super) fn store_symmetry(&mut self) {
        let s = self.symmetry.clone();
        let Some(d) = self.doc.as_mut() else { return };
        if d.doc.sidecar.symmetry == s {
            return;
        }
        d.end_stroke();
        let (_, rf) = d.edit(
            &trd("kfx_editor.symmetry_settings", "Symmetry settings"),
            |m, _, _| m.edit_sidecar(|sc| sc.symmetry = s),
        );
        self.after_change(rf);
    }

    pub fn open_decor_dialog(&mut self) {
        let Some(d) = &self.doc else { return };
        let ds = &d.doc.sidecar.decoration;
        self.decor_dialog = Some(DecorDialog {
            seed: ds.seed,
            enabled: ds.enabled,
            disabled: ds.disabled_families.clone(),
        });
    }

    /// Store the map's decoration settings (an undo step) and apply them.
    pub fn set_map_decoration(&mut self, seed: Option<u64>, enabled: bool, disabled: Vec<String>) {
        let Some(d) = self.doc.as_mut() else { return };
        d.end_stroke();
        let (_, rf) = d.edit(
            &trd("kfx_editor.decoration_settings", "Decoration settings"),
            |m, _, _| {
                m.edit_sidecar(|s| {
                    s.decoration.seed = seed;
                    s.decoration.enabled = enabled;
                    s.decoration.disabled_families = disabled;
                })
            },
        );
        self.after_change(rf);
        self.apply_decor_prefs();
    }

    pub(super) fn decor_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.decor_dialog.clone() else {
            return;
        };
        let Some(d) = &self.doc else { return };
        let fams: Vec<String> = d
            .decorator
            .families
            .cubes
            .iter()
            .map(|f| f.name.clone())
            .collect();
        let level_seed = kfx_map::decorate::Decorator::seed_for_level(d.doc.level);
        let mut ok = false;
        let mut cancel = false;
        let mut open = true;
        egui::Window::new(trd(
            "kfx_editor.decoration_of_this_map",
            "Decoration of this map",
        ))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(
                RichText::new(trd(
                    "kfx_editor.stored_with_the_map_mapnnnnn_kfxed_toml_the_pref",
                    "Stored with the map (mapNNNNN.kfxed.toml); the Preferences hold the \
                    editor-wide defaults.",
                ))
                .small()
                .color(TEXT_DIM),
            );
            ui.checkbox(
                &mut dlg.enabled,
                trd(
                    "kfx_editor.decorate_placed_regenerated_slabs",
                    "Decorate placed / regenerated slabs",
                ),
            );
            ui.horizontal(|ui| {
                let mut own = dlg.seed.is_some();
                if ui
                    .checkbox(&mut own, trd("kfx_editor.own_seed", "Own seed"))
                    .changed()
                {
                    dlg.seed = own.then_some(level_seed);
                }
                if let Some(s) = dlg.seed.as_mut() {
                    ui.add(egui::DragValue::new(s));
                } else {
                    ui.label(
                        RichText::new(trf(
                            "kfx_editor.from_the_level_number_level_seed",
                            "from the level number ({level_seed})",
                            &[("level_seed", &level_seed)],
                        ))
                        .small()
                        .color(TEXT_DIM),
                    );
                }
            });
            ui.label(
                RichText::new(trd(
                    "kfx_editor.families_switched_off_for_this_map",
                    "Families switched off for this map",
                ))
                .small()
                .color(TEXT_DIM),
            );
            egui::Grid::new("map-fams").num_columns(3).show(ui, |ui| {
                for (n, f) in fams.iter().enumerate() {
                    let mut off = dlg.disabled.iter().any(|x| x.eq_ignore_ascii_case(f));
                    if ui.checkbox(&mut off, f).changed() {
                        dlg.disabled.retain(|x| !x.eq_ignore_ascii_case(f));
                        if off {
                            dlg.disabled.push(f.clone());
                        }
                    }
                    if n % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.button(trd("kfx_editor.apply", "Apply")).clicked() {
                    ok = true;
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
        if cancel || !open {
            self.decor_dialog = None;
        } else if ok {
            self.decor_dialog = None;
            self.set_map_decoration(dlg.seed, dlg.enabled, dlg.disabled);
        } else {
            self.decor_dialog = Some(dlg);
        }
    }

    /// The `.une` import notice of the open map, if any.
    pub(super) fn notice_ui(&mut self, ctx: &egui::Context) {
        let Some(n) = self.notice.clone() else { return };
        let mut close = false;
        egui::Window::new(trd("kfx_editor.notice", "Notice"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, EVec2::new(0.0, 80.0))
            .show(ctx, |ui| {
                ui.set_max_width(520.0);
                ui.label(n);
                if ui.button(trd("kfx_editor.ok", "OK")).clicked() {
                    close = true;
                }
            });
        if close {
            self.notice = None;
        }
    }
}
