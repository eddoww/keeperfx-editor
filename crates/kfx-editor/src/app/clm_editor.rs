//! Map > Columns of this map: the distinct columns of the map as the save writes them to the
//! `.clm` (`kfx_map::column::ColumnPool`, find-or-create by content like `find_column` /
//! `create_column`, src/map_columns.c:280-358), with their use counts. A column is edited once and
//! the change replaces it in every subtile that uses it (one undo step). Height, lintel and solid
//! mask are derived from the cubes (`canonical_column`).
//!
//! There is no "clear unused": the editor keeps a column per subtile and the save interns only
//! the columns some subtile uses, so an unused column never reaches the saved `.clm`.
use super::*;
use kfx_map::column::{canonical_column, ColumnKey, CLF_ACTIVE};
use kfx_map::gamedata::COLUMNS_COUNT;
use kfx_prefab::i18n::{trd, trf};

/// One distinct column of the map.
#[derive(Clone, Copy, Debug)]
pub struct PoolRow {
    /// Index in the saved `.clm` (first use in `.dat` order, like `ColumnPool::build`).
    pub index: u16,
    pub col: level_bin::Column,
    pub uses: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnSort {
    Index,
    Uses,
    Height,
}

/// The column being edited (a copy; nothing changes until Apply).
#[derive(Clone, Copy, Debug)]
pub struct ColumnEdit {
    /// What the subtiles to change hold now.
    pub key: ColumnKey,
    pub index: u16,
    pub cubes: [u16; 8],
    pub floor: u16,
    pub orient: u8,
}

pub struct ColumnsWindow {
    pub rows: Vec<PoolRow>,
    /// (document revision, level, grid size) the rows were built from.
    built: Option<(u64, u32, usize)>,
    /// Columns the game adds when it loads the level, and the limit check.
    pub game_added: usize,
    pub limit: Result<(), String>,
    pub sort: ColumnSort,
    pub desc: bool,
    pub filter: String,
    pub edit: Option<ColumnEdit>,
    /// Copied cubes and floor texture.
    pub clipboard: Option<([u16; 8], u16)>,
    /// Which use the next "jump" goes to.
    jump_next: usize,
    /// Atlas tiles as textures, by texture id (for the atlas `tex_atlas`).
    tex: HashMap<u16, Option<egui::TextureHandle>>,
    tex_atlas: usize,
    pub error: Option<String>,
}

/// Distinct columns of a grid in `.clm` order, with use counts.
pub fn pool_rows(columns: &[level_bin::Column], dims: MapDims) -> Vec<PoolRow> {
    let mut index: HashMap<ColumnKey, usize> = HashMap::new();
    // index 0 is the all-zero column every `.clm` starts with (ColumnPool::new)
    let zero = level_bin::Column::default();
    index.insert(ColumnKey::from(&zero), 0);
    let mut rows = vec![PoolRow {
        index: 0,
        col: zero,
        uses: 0,
    }];
    for c in columns.iter().take(dims.stl_grid()) {
        let k = ColumnKey::from(c);
        let i = *index.entry(k).or_insert_with(|| {
            rows.push(PoolRow {
                index: rows.len() as u16,
                col: *c,
                uses: 0,
            });
            rows.len() - 1
        });
        rows[i].uses += 1;
    }
    rows
}

/// Floor height (cubes from the bottom), lintel (ceiling bits) and active flag of a column's
/// bitfields (`ColumnFlags`, src/map_columns.h:52).
pub fn column_bits(c: &level_bin::Column) -> (u8, u8, bool) {
    (
        c.bitfields >> 4,
        (c.bitfields >> 1) & 7,
        c.bitfields & CLF_ACTIVE != 0,
    )
}

impl ColumnsWindow {
    fn new() -> Self {
        ColumnsWindow {
            rows: vec![],
            built: None,
            game_added: 0,
            limit: Ok(()),
            sort: ColumnSort::Index,
            desc: false,
            filter: String::new(),
            edit: None,
            clipboard: None,
            jump_next: 0,
            tex: HashMap::new(),
            tex_atlas: 0,
            error: None,
        }
    }

    fn select(&mut self, r: &PoolRow) {
        self.edit = Some(ColumnEdit {
            key: ColumnKey::from(&r.col),
            index: r.index,
            cubes: r.col.cubes,
            floor: r.col.floor_texture,
            orient: r.col.orient,
        });
        self.jump_next = 0;
        self.error = None;
    }

    /// Rows that pass the filter, in the chosen order.
    pub fn visible(&self, gd: &GameData) -> Vec<usize> {
        let tokens: Vec<String> = self
            .filter
            .split_whitespace()
            .map(|t| t.to_ascii_lowercase())
            .collect();
        let mut v: Vec<usize> = (0..self.rows.len())
            .filter(|&i| tokens.iter().all(|t| row_matches(&self.rows[i], t, gd)))
            .collect();
        match self.sort {
            ColumnSort::Index => {}
            ColumnSort::Uses => {
                v.sort_by_key(|&i| (self.rows[i].uses, std::cmp::Reverse(self.rows[i].index)))
            }
            ColumnSort::Height => v.sort_by_key(|&i| {
                (
                    assets::floor_height(self.rows[i].col.solidmask),
                    std::cmp::Reverse(self.rows[i].index),
                )
            }),
        }
        if self.desc != (self.sort != ColumnSort::Index) {
            v.reverse();
        }
        v
    }
}

/// Filter token: `#N` the index, `f:N` the floor texture, a number the index or a cube id, a
/// word a part of a cube name.
fn row_matches(r: &PoolRow, t: &str, gd: &GameData) -> bool {
    if let Some(n) = t.strip_prefix('#').and_then(|s| s.parse::<u16>().ok()) {
        return r.index == n;
    }
    if let Some(n) = t.strip_prefix("f:").and_then(|s| s.parse::<u16>().ok()) {
        return r.col.floor_texture == n;
    }
    if let Ok(n) = t.parse::<u16>() {
        return r.index == n || r.col.cubes.contains(&n);
    }
    r.col.cubes.iter().any(|&c| {
        c != 0
            && gd
                .cubes
                .get(c as usize)
                .is_some_and(|k| k.name.to_ascii_lowercase().contains(t))
    })
}

impl EditorApp {
    /// Open Map > Columns of this map (the first row in the current order selected).
    pub fn open_columns_window(&mut self) {
        if self.doc.is_none() {
            return;
        }
        let mut w = self.columns_win.take().unwrap_or_else(ColumnsWindow::new);
        w.built = None;
        self.refresh_columns(&mut w);
        if w.edit.is_none() {
            let gd = self.doc.as_ref().unwrap().gd.clone();
            if let Some(&i) = w.visible(&gd).iter().find(|&&i| w.rows[i].index != 0) {
                let r = w.rows[i];
                w.select(&r);
            }
        }
        self.columns_win = Some(w);
    }

    /// Rebuild the rows when the map changed.
    fn refresh_columns(&self, w: &mut ColumnsWindow) {
        let Some(d) = &self.doc else {
            w.rows.clear();
            return;
        };
        let key = (d.revision, d.doc.level, d.dims().stl_grid());
        if w.built == Some(key) {
            return;
        }
        w.built = Some(key);
        w.rows = pool_rows(&d.doc.columns, d.dims());
        // the game's own columns (unrevealed, slabset, static) also take pool slots
        let mut pool = kfx_map::column::ColumnPool::new();
        let mut over = false;
        for r in w.rows.iter().skip(1) {
            if pool.intern(&r.col).is_err() {
                over = true;
                break;
            }
        }
        w.game_added = pool.game_added_columns(&d.gd);
        w.limit = if over {
            Err(trf(
                "kfx_editor.rows_columns_more_than_the_v_a_clm_can_hold",
                "{rows} columns: more than the {v} a .clm can hold",
                &[("rows", &w.rows.len()), ("v", &COLUMNS_COUNT)],
            ))
        } else {
            pool.check_game_limit(&d.gd).map_err(|e| e.to_string())
        };
        // keep the selection on the same content
        if let Some(e) = w.edit {
            match w.rows.iter().find(|r| ColumnKey::from(&r.col) == e.key) {
                Some(r) => {
                    w.edit = Some(ColumnEdit {
                        index: r.index,
                        ..e
                    })
                }
                None => w.edit = None,
            }
        }
    }

    /// Replace the column with content `old` in every subtile that holds it by one with these
    /// cubes (bottom to top), floor texture and orientation; height, lintel and solid mask are
    /// recomputed (`canonical_column`). One undo step. Returns the number of subtiles changed.
    pub fn replace_pool_column(
        &mut self,
        old: ColumnKey,
        cubes: [u16; 8],
        floor: u16,
        orient: u8,
    ) -> Result<usize, String> {
        let d = self.doc.as_mut().ok_or("no map")?;
        let dm = d.dims();
        let n = dm.stl_grid().min(d.doc.columns.len());
        let cells: Vec<usize> = (0..n)
            .filter(|&i| ColumnKey::from(&d.doc.columns[i]) == old)
            .collect();
        if cells.is_empty() {
            return Err(trd(
                "kfx_editor.no_subtile_holds_this_column_any_more",
                "no subtile holds this column any more",
            ));
        }
        let first = d.doc.columns[cells[0]];
        let index = pool_rows(&d.doc.columns, dm)
            .iter()
            .find(|r| ColumnKey::from(&r.col) == old)
            .map_or(0, |r| r.index);
        let make = |c: &level_bin::Column| {
            let mut c = *c;
            c.cubes = cubes;
            c.floor_texture = floor;
            c.orient = orient;
            canonical_column(&c, c.bitfields & CLF_ACTIVE != 0)
        };
        if ColumnKey::from(&make(&first)) == old {
            return Ok(0);
        }
        let w = dm.stl_w() + 1;
        d.end_stroke();
        let (_, rf) = d.edit_tracked(
            &trf(
                "kfx_editor.edit_column_index_cells_subtiles",
                "Edit column {index} ({cells} subtiles)",
                &[("index", &index), ("cells", &cells.len())],
            ),
            |m, _, _| {
                for &i in &cells {
                    m.touch_subtile(i % w, i / w);
                    m.columns[i] = make(&m.columns[i]);
                }
            },
        );
        self.after_change(rf);
        self.status = trf(
            "kfx_editor.column_index_changed_in_cells_subtiles",
            "Column {index} changed in {cells} subtiles",
            &[("index", &index), ("cells", &cells.len())],
        );
        Ok(cells.len())
    }

    /// Centre the view on the `n`-th subtile (wrapping) holding column `key`; returns it.
    pub fn jump_to_column_use(&mut self, key: ColumnKey, n: usize) -> Option<(usize, usize)> {
        let d = self.doc.as_ref()?;
        let dm = d.dims();
        let w = dm.stl_w() + 1;
        let cells: Vec<usize> = (0..dm.stl_grid().min(d.doc.columns.len()))
            .filter(|&i| ColumnKey::from(&d.doc.columns[i]) == key)
            .collect();
        let i = *cells.get(n % cells.len().max(1))?;
        let (x, y) = (i % w, i / w);
        let h = d.doc.floor_height(x as i32, y as i32) as f32;
        let p = Vec3::new(x as f32 + 0.5, h, y as f32 + 0.5);
        self.camera.target = Vec3::new(p.x, self.camera.target.y, p.z);
        self.camera2.target = Vec3::new(p.x, self.camera2.target.y, p.z);
        self.flash = Some((p, Instant::now()));
        self.status = trf(
            "kfx_editor.subtile_x_y_uses_this_column_cells_of_cells2",
            "Subtile ({x},{y}) uses this column ({cells} of {cells2})",
            &[
                ("x", &x),
                ("y", &y),
                ("cells", &(n % cells.len() + 1)),
                ("cells2", &cells.len()),
            ],
        );
        Some((x, y))
    }

    /// Atlas tile `id` as a texture (cached per atlas).
    fn column_tex(&self, w: &mut ColumnsWindow, id: u16) -> Option<egui::TextureHandle> {
        let atlas = self.atlas.as_ref().map_or(0, |a| Arc::as_ptr(a) as usize);
        if w.tex_atlas != atlas {
            w.tex.clear();
            w.tex_atlas = atlas;
        }
        w.tex
            .entry(id)
            .or_insert_with(|| {
                let px = self.atlas_tile(id)?;
                Some(self.ctx.load_texture(
                    format!("clm-{id}"),
                    egui::ColorImage::from_rgba_unmultiplied([32, 32], &px),
                    egui::TextureOptions::NEAREST,
                ))
            })
            .clone()
    }

    /// Side view of a cube stack (south faces, bottom up) over its floor texture, and the top
    /// texture the game shows from above.
    fn draw_column_stack(
        &self,
        w: &mut ColumnsWindow,
        ui: &mut egui::Ui,
        cubes: &[u16; 8],
        floor: u16,
        cell: f32,
    ) {
        let solid = kfx_map::column::solidmask_of(cubes);
        let top = assets::column_top_texture(&self.cubes, solid, floor, cubes);
        let side: Vec<Option<egui::TextureHandle>> = cubes
            .iter()
            .map(|&c| {
                if c == 0 {
                    None
                } else {
                    self.column_tex(w, self.cubes.face(c, assets::FACE_S))
                }
            })
            .collect();
        let floor_tex = self.column_tex(w, floor);
        let top_tex = self.column_tex(w, top);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.horizontal(|ui| {
            let (rect, resp) = ui.allocate_exact_size(
                EVec2::new(cell + 8.0, cell * 8.0 + cell * 0.5 + 8.0),
                egui::Sense::hover(),
            );
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Image,
                    true,
                    trd("kfx_editor.column_side_view", "column side view"),
                )
            });
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 2.0, Color32::from_rgb(12, 11, 10));
            let x0 = rect.min.x + 4.0;
            let base = rect.max.y - 4.0 - cell * 0.5;
            // floor strip (the floor texture is what an empty bottom shows)
            let fr = egui::Rect::from_min_size(egui::pos2(x0, base), EVec2::new(cell, cell * 0.5));
            match &floor_tex {
                Some(t) => p.image(
                    t.id(),
                    fr,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.5)),
                    Color32::from_gray(150),
                ),
                None => p.rect_filled(fr, 0.0, Color32::from_gray(40)),
            };
            for (k, t) in side.iter().enumerate() {
                let r = egui::Rect::from_min_size(
                    egui::pos2(x0, base - cell * (k + 1) as f32),
                    EVec2::splat(cell),
                );
                match t {
                    Some(t) => {
                        p.image(t.id(), r, uv, Color32::WHITE);
                    }
                    None if cubes[k] != 0 => {
                        p.rect_filled(r, 0.0, Color32::from_rgb(90, 30, 90));
                    }
                    None => {
                        p.rect_stroke(
                            r.shrink(1.0),
                            0.0,
                            egui::Stroke::new(1.0, Color32::from_rgb(45, 40, 34)),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            }
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(trd("kfx_editor.top", "top"))
                        .small()
                        .color(TEXT_DIM),
                );
                let (r, _) = ui.allocate_exact_size(EVec2::splat(cell * 1.5), egui::Sense::hover());
                if let Some(t) = &top_tex {
                    ui.painter().image(t.id(), r, uv, Color32::WHITE);
                }
                ui.label(
                    RichText::new(trf(
                        "kfx_editor.texture_top",
                        "texture {top}",
                        &[("top", &top)],
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
            });
        });
    }

    pub(super) fn columns_window_ui(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.columns_win.take() else {
            return;
        };
        let Some(gd) = self.doc.as_ref().map(|d| d.gd.clone()) else {
            return;
        };
        self.refresh_columns(&mut w);
        let mut open = true;
        let mut apply = None;
        let mut jump = None;
        egui::Window::new(trd("kfx_editor.columns_of_this_map", "Columns of this map"))
            .open(&mut open)
            .default_size([1060.0, 640.0])
            .max_height(ctx.content_rect().height() * 0.85)
            .show(ctx, |ui| {
                let used = w.rows.len() - 1;
                let limit = COLUMNS_COUNT - 1;
                let text = trf(
                    "kfx_editor.used_columns_in_the_saved_clm_game_added_the_gam",
                    "{used} columns in the saved .clm (+ {game_added} the game adds on \
                    load) of {limit}",
                    &[
                        ("used", &used),
                        ("game_added", &w.game_added),
                        ("limit", &limit),
                    ],
                );
                match &w.limit {
                    Ok(()) => ui.label(RichText::new(text).color(GOLD)),
                    Err(e) => ui.colored_label(Color32::LIGHT_RED, format!("{text}: {e}")),
                };
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.one_row_per_distinct_column_as_the_save_writes_i",
                        "One row per distinct column as the save writes it (index 0 is \
                        the empty column every .clm starts with). There is no \"clear unused\": \
                        unused columns never reach the saved .clm. An edit replaces the column in \
                        every subtile using it; placing or regenerating a slab rebuilds its \
                        columns \
                        from the slab set (use Map > Edit column to keep a change through that).",
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
                ui.horizontal(|ui| {
                    ui.label(trd("kfx_editor.filter", "Filter"));
                    ui.add(
                        egui::TextEdit::singleline(&mut w.filter)
                            .hint_text(trd(
                                "kfx_editor.12_12_f_5_cube_id_or_name",
                                "12, #12, f:5, cube ID or name",
                            ))
                            .desired_width(220.0),
                    );
                    ui.separator();
                    ui.label(trd("kfx_editor.sort", "Sort"));
                    for (s, l) in [
                        (ColumnSort::Index, trd("kfx_editor.by_index", "by index")),
                        (ColumnSort::Uses, trd("kfx_editor.by_uses", "by uses")),
                        (ColumnSort::Height, trd("kfx_editor.by_height", "by height")),
                    ] {
                        let arrow = if w.sort == s {
                            if w.desc {
                                " ⏷"
                            } else {
                                " ⏶"
                            }
                        } else {
                            ""
                        };
                        if ui
                            .add(egui::Button::selectable(w.sort == s, format!("{l}{arrow}")))
                            .clicked()
                        {
                            if w.sort == s {
                                w.desc = !w.desc;
                            } else {
                                w.sort = s;
                                w.desc = false;
                            }
                        }
                    }
                });
                ui.separator();
                let vis = w.visible(&gd);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(640.0);
                        let widths = [52.0, 50.0, 132.0, 168.0, 40.0, 36.0, 26.0, 36.0, 44.0];
                        ui.horizontal(|ui| {
                            for ((t, tip), wd) in [
                                (trd("kfx_editor.index", "index"), None),
                                (trd("kfx_editor.uses", "uses"), None),
                                (trd("kfx_editor.cubes_bottom_up", "cubes (bottom up)"), None),
                                (trd("kfx_editor.cube_ids", "cube IDs"), None),
                                (trd("kfx_editor.floor", "floor"), None),
                                (
                                    trd("kfx_editor.orient", "orient"),
                                    Some(trd("kfx_editor.orientation", "Orientation")),
                                ),
                                ("h".to_string(), Some(trd("kfx_editor.height", "Height"))),
                                (trd("kfx_editor.lintel", "lintel"), None),
                                (trd("kfx_editor.mask", "mask"), None),
                            ]
                            .iter()
                            .zip(widths)
                            {
                                let r = ui.add_sized(
                                    [wd, 16.0],
                                    egui::Label::new(
                                        RichText::new(t.as_str()).small().color(GOLD_DIM),
                                    ),
                                );
                                if let Some(tip) = tip {
                                    r.on_hover_text(tip.as_str());
                                }
                            }
                        });
                        let row_h = 20.0;
                        let sel = w.edit.map(|e| e.index);
                        let mut picked = None;
                        let rows = w.rows.clone();
                        egui::ScrollArea::vertical()
                            .id_salt("clm-rows")
                            .auto_shrink([false, false])
                            .max_height(ui.available_height().max(200.0))
                            .show_rows(ui, row_h, vis.len(), |ui, range| {
                                for &i in &vis[range] {
                                    let r = rows[i];
                                    let (h, lintel, _) = column_bits(&r.col);
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 4.0;
                                        if ui
                                            .add_sized(
                                                [widths[0], row_h],
                                                egui::Button::selectable(
                                                    sel == Some(r.index),
                                                    format!("#{}", r.index),
                                                ),
                                            )
                                            .clicked()
                                        {
                                            picked = Some(r);
                                        }
                                        ui.add_sized(
                                            [widths[1], row_h],
                                            egui::Label::new(r.uses.to_string()),
                                        );
                                        // mini side view: 8 cube faces bottom to top
                                        let (rect, resp) = ui.allocate_exact_size(
                                            EVec2::new(widths[2], row_h),
                                            egui::Sense::hover(),
                                        );
                                        let p = ui.painter_at(rect);
                                        for (k, &c) in r.col.cubes.iter().enumerate() {
                                            let cr = egui::Rect::from_min_size(
                                                rect.min + EVec2::new(k as f32 * 16.0, 2.0),
                                                EVec2::splat(15.0),
                                            );
                                            if c == 0 {
                                                p.rect_stroke(
                                                    cr,
                                                    0.0,
                                                    egui::Stroke::new(
                                                        1.0,
                                                        Color32::from_rgb(45, 40, 34),
                                                    ),
                                                    egui::StrokeKind::Inside,
                                                );
                                            } else if let Some(t) = self.column_tex(
                                                &mut w,
                                                self.cubes.face(c, assets::FACE_S),
                                            ) {
                                                p.image(
                                                    t.id(),
                                                    cr,
                                                    egui::Rect::from_min_max(
                                                        egui::pos2(0.0, 0.0),
                                                        egui::pos2(1.0, 1.0),
                                                    ),
                                                    Color32::WHITE,
                                                );
                                            }
                                        }
                                        let names: Vec<String> = r
                                            .col
                                            .cubes
                                            .iter()
                                            .enumerate()
                                            .filter(|(_, &c)| c != 0)
                                            .map(|(k, &c)| {
                                                format!(
                                                    "{}: {} {}",
                                                    k + 1,
                                                    c,
                                                    gd.cubes
                                                        .get(c as usize)
                                                        .map_or("?", |x| x.name.as_str())
                                                )
                                            })
                                            .collect();
                                        resp.on_hover_text(if names.is_empty() {
                                            trd("kfx_editor.no_cubes", "no cubes")
                                        } else {
                                            names.join("\n")
                                        });
                                        let ids: Vec<String> =
                                            r.col
                                                .cubes
                                                .iter()
                                                .map(|&c| {
                                                    if c == 0 {
                                                        "·".into()
                                                    } else {
                                                        c.to_string()
                                                    }
                                                })
                                                .collect();
                                        ui.add_sized(
                                            [widths[3], row_h],
                                            egui::Label::new(RichText::new(ids.join(" ")).small())
                                                .truncate(),
                                        );
                                        ui.add_sized(
                                            [widths[4], row_h],
                                            egui::Label::new(r.col.floor_texture.to_string()),
                                        );
                                        ui.add_sized(
                                            [widths[5], row_h],
                                            egui::Label::new(r.col.orient.to_string()),
                                        );
                                        ui.add_sized(
                                            [widths[6], row_h],
                                            egui::Label::new(h.to_string()),
                                        );
                                        ui.add_sized(
                                            [widths[7], row_h],
                                            egui::Label::new(lintel.to_string()),
                                        );
                                        ui.add_sized(
                                            [widths[8], row_h],
                                            egui::Label::new(format!("{:02x}", r.col.solidmask)),
                                        );
                                    });
                                }
                            });
                        if let Some(r) = picked {
                            w.select(&r);
                        }
                        if vis.is_empty() {
                            ui.label(
                                RichText::new(trd(
                                    "kfx_editor.no_column_matches_the_filter",
                                    "No column matches the filter",
                                ))
                                .color(TEXT_DIM),
                            );
                        }
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        let Some(mut e) = w.edit else {
                            ui.label(
                                RichText::new(trd("kfx_editor.select_a_column", "Select a column"))
                                    .color(TEXT_DIM),
                            );
                            return;
                        };
                        let uses = w
                            .rows
                            .iter()
                            .find(|r| r.index == e.index)
                            .map_or(0, |r| r.uses);
                        ui.heading(
                            RichText::new(trf(
                                "kfx_editor.column_index",
                                "Column {index}",
                                &[("index", &e.index)],
                            ))
                            .color(GOLD),
                        );
                        ui.label(
                            RichText::new(trf(
                                "kfx_editor.used_by_uses_subtiles",
                                "used by {uses} subtiles",
                                &[("uses", &uses)],
                            ))
                            .small()
                            .color(TEXT_DIM),
                        );
                        self.draw_column_stack(&mut w, ui, &e.cubes, e.floor, 24.0);
                        egui::Grid::new("clm-edit")
                            .num_columns(2)
                            .spacing([8.0, 3.0])
                            .show(ui, |ui| {
                                for n in (0..8).rev() {
                                    ui.label(trf(
                                        "kfx_editor.cube_n",
                                        "cube {n}",
                                        &[("n", &(n + 1))],
                                    ));
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::DragValue::new(&mut e.cubes[n])
                                                .range(0..=gd.cubes.len().saturating_sub(1) as u16),
                                        );
                                        ui.label(
                                            RichText::new(
                                                gd.cubes
                                                    .get(e.cubes[n] as usize)
                                                    .filter(|_| e.cubes[n] != 0)
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
                                ui.add(egui::DragValue::new(&mut e.floor).range(0..=999));
                                ui.end_row();
                                ui.label(trd("kfx_editor.orientation", "Orientation"));
                                ui.add(egui::DragValue::new(&mut e.orient).range(0..=255));
                                ui.end_row();
                                // derived like create_column (src/map_columns.c:293)
                                let probe = canonical_column(
                                    &level_bin::Column {
                                        cubes: e.cubes,
                                        floor_texture: e.floor,
                                        orient: e.orient,
                                        bitfields: CLF_ACTIVE,
                                        ..Default::default()
                                    },
                                    true,
                                );
                                let (h, lintel, _) = column_bits(&probe);
                                ui.label(
                                    RichText::new(trd("kfx_editor.derived", "Derived"))
                                        .color(TEXT_DIM),
                                );
                                ui.label(
                                    RichText::new(trf(
                                        "kfx_editor.height_h_lintel_lintel_solid_mask_solidmask",
                                        "height {h}, lintel {lintel}, solid mask {solidmask}",
                                        &[
                                            ("h", &h),
                                            ("lintel", &lintel),
                                            ("solidmask", &format!("{:02x}", probe.solidmask)),
                                        ],
                                    ))
                                    .color(TEXT_DIM),
                                );
                                ui.end_row();
                            });
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .button(trd("kfx_editor.copy_cubes", "Copy cubes"))
                                .on_hover_text(trd(
                                    "kfx_editor.copy_the_cubes_and_floor_texture",
                                    "Copy the cubes and floor texture",
                                ))
                                .clicked()
                            {
                                w.clipboard = Some((e.cubes, e.floor));
                            }
                            if ui
                                .add_enabled(
                                    w.clipboard.is_some(),
                                    egui::Button::new(trd("kfx_editor.paste_cubes", "Paste cubes")),
                                )
                                .on_hover_text(trd(
                                    "kfx_editor.paste_copied_cubes_and_floor_texture_into_this_c",
                                    "Paste copied cubes and floor texture into this column \
                                    (Apply to keep)",
                                ))
                                .clicked()
                            {
                                if let Some((c, f)) = w.clipboard {
                                    e.cubes = c;
                                    e.floor = f;
                                }
                            }
                            if let Some(r) = w.rows.iter().find(|r| r.index == e.index) {
                                if ui.button(trd("kfx_editor.revert", "Revert")).clicked() {
                                    e.cubes = r.col.cubes;
                                    e.floor = r.col.floor_texture;
                                    e.orient = r.col.orient;
                                }
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add_enabled(
                                    e.index != 0,
                                    egui::Button::new(trd(
                                        "kfx_editor.apply_to_all_uses",
                                        "Apply to all uses",
                                    )),
                                )
                                .on_hover_text(trf(
                                    "kfx_editor.replace_this_column_in_all_uses_subtiles_one_und",
                                    "Replace this column in all {uses} subtiles (one undo \
                                    step)",
                                    &[("uses", &uses)],
                                ))
                                .clicked()
                            {
                                apply = Some(e);
                            }
                            if ui
                                .add_enabled(
                                    uses > 0,
                                    egui::Button::new(trd(
                                        "kfx_editor.jump_to_a_use",
                                        "Jump to a use",
                                    )),
                                )
                                .on_hover_text(trd(
                                    "kfx_editor.centre_the_view_on_a_subtile_using_this_column_a",
                                    "Centre the view on a subtile using this column (again \
                                    for the next one)",
                                ))
                                .clicked()
                            {
                                jump = Some(e.key);
                            }
                        });
                        if e.index == 0 {
                            ui.label(
                                RichText::new(trd(
                                    "kfx_editor.column_0_stays_empty_the_game_s_default_column",
                                    "Column 0 stays empty (the game's default column).",
                                ))
                                .small()
                                .color(TEXT_DIM),
                            );
                        }
                        if let Some(err) = &w.error {
                            ui.colored_label(Color32::LIGHT_RED, err);
                        }
                        w.edit = Some(e);
                    });
                });
            });
        if let Some(e) = apply {
            match self.replace_pool_column(e.key, e.cubes, e.floor, e.orient) {
                Ok(_) => {
                    w.error = None;
                    let new = canonical_column(
                        &level_bin::Column {
                            cubes: e.cubes,
                            floor_texture: e.floor,
                            orient: e.orient,
                            ..Default::default()
                        },
                        true,
                    );
                    w.edit = Some(ColumnEdit {
                        key: ColumnKey::from(&new),
                        ..e
                    });
                    w.built = None;
                    self.refresh_columns(&mut w);
                }
                Err(err) => w.error = Some(err),
            }
        }
        if let Some(k) = jump {
            self.jump_to_column_use(k, w.jump_next);
            w.jump_next += 1;
        }
        if open {
            self.columns_win = Some(w);
        }
    }
}

#[cfg(test)]
impl EditorApp {
    pub fn columns_window_state(&self) -> Option<&ColumnsWindow> {
        self.columns_win.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_rows_match_the_saved_pool() {
        let dims = MapDims::new(2, 2);
        let mut cols = vec![level_bin::Column::default(); dims.stl_grid()];
        let mut a = level_bin::Column::default();
        a.cubes[0] = 5;
        let a = canonical_column(&a, true);
        cols[3] = a;
        cols[9] = a;
        let rows = pool_rows(&cols, dims);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].uses as usize, dims.stl_grid() - 2);
        assert_eq!((rows[1].index, rows[1].uses), (1, 2));
        // same order and indices as the save's pool
        let mut pool = kfx_map::column::ColumnPool::new();
        for c in &cols {
            pool.intern(c).unwrap();
        }
        assert_eq!(pool.len(), rows.len());
        let (h, lintel, active) = column_bits(&rows[1].col);
        assert_eq!((h, lintel, active), (1, 0, true));
    }
}
