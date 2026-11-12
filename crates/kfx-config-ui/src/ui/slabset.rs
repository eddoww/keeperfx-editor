//! Slabset editor: for a slab kind, the 28 styles with their 9 columns as isometric previews,
//! subtile picking, copy / paste / rotate / mirror / revert, per-style slab objects, and a live
//! preview of a 5x5 patch of the kind inside a ring of a chosen neighbour, generated with
//! `kfx_map::autotile` from the edited data.
use super::common::{badge, image_fit, layer_color, picker, weak};
use super::Cx;
use crate::raster::{draw_columns, Canvas, ColumnDraw, Iso};
use crate::slabset::{ObjView, Style, STYLES};
use egui::{Color32, RichText, Ui};
use kfx_formats::level_bin::{Column, MapDims};
use kfx_map::autotile::{Layout, MapGen};
use kfx_map::gamedata::{GameData, SLAB_STYLE_NAMES};

pub struct SlabsetState {
    pub kind: usize,
    pub style: usize,
    pub subtile: usize,
    pub neighbour: usize,
    pub clipboard: Option<Style>,
    pub animate: bool,
    pub owner: u8,
    last_gen: u64,
}

impl Default for SlabsetState {
    fn default() -> Self {
        SlabsetState {
            kind: 10,
            style: 8,
            subtile: 4,
            neighbour: 2,
            clipboard: None,
            animate: false,
            owner: 0,
            last_gen: u64::MAX,
        }
    }
}

/// Columns of a slabset entry (resolved like the game; unresolved ones drawn empty).
fn style_columns(gd: &GameData, kind: usize, style: usize) -> [Option<Column>; 9] {
    let id = GameData::slabset_index(kind, style);
    std::array::from_fn(|i| gd.slabset_column(id, i).copied())
}

fn faces_of(gd: &GameData) -> impl Fn(u16) -> [u16; 6] + '_ {
    move |c| {
        gd.cubes
            .get(c as usize)
            .map(|c| c.textures)
            .unwrap_or([0; 6])
    }
}

/// Render a 3x3 slabset entry; ids 1..9 = subtile + 1.
pub fn render_style(
    assets: &mut Option<crate::assets::Assets>,
    gd: &GameData,
    kind: usize,
    style: usize,
    size: f32,
    selected: Option<usize>,
    frame: usize,
) -> Option<Canvas> {
    let tiles = assets.as_mut()?.tiles.as_mut()?;
    let cols = style_columns(gd, kind, style);
    let draws: Vec<ColumnDraw> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let c = c.unwrap_or_default();
            ColumnDraw {
                x: (i % 3) as i32,
                y: (i / 3) as i32,
                cubes: c.cubes,
                floor: c.floor_texture,
                tint: (selected == Some(i)).then_some([255, 220, 60, 90]),
                id: i as u32 + 1,
            }
        })
        .collect();
    let h = (size * 1.15) as u32;
    let mut cv = Canvas::new(size as u32, h);
    let iso = Iso::fit(
        3.0,
        3.0,
        crate::raster::max_height(&draws).max(3.0),
        size,
        h as f32,
    );
    draw_columns(&mut cv, &iso, tiles, frame, &draws, &faces_of(gd));
    Some(cv)
}

/// Render generated columns of a map region (subtile grid `w x h` from `gen`).
fn render_gen(
    assets: &mut Option<crate::assets::Assets>,
    gd: &GameData,
    gen: &MapGen,
    size: f32,
    frame: usize,
) -> Option<Canvas> {
    let tiles = assets.as_mut()?.tiles.as_mut()?;
    let d = gen.dims;
    let (w, h) = (d.stl_w() as i32, d.stl_h() as i32);
    let mut draws = vec![];
    for y in 0..h {
        for x in 0..w {
            let c = gen.column_at(x, y).copied().unwrap_or_default();
            draws.push(ColumnDraw {
                x,
                y,
                cubes: c.cubes,
                floor: c.floor_texture,
                tint: None,
                id: (y * w + x) as u32 + 1,
            });
        }
    }
    let ch = (size * 0.85) as u32;
    let mut cv = Canvas::new(size as u32, ch);
    let iso = Iso::fit(
        w as f32,
        h as f32,
        crate::raster::max_height(&draws).max(2.0),
        size,
        ch as f32,
    );
    draw_columns(&mut cv, &iso, tiles, frame, &draws, &faces_of(gd));
    Some(cv)
}

pub fn show(ui: &mut Ui, cx: &mut Cx, st: &mut SlabsetState) {
    let ctx = ui.ctx().clone();
    if st.last_gen != cx.s.generation {
        cx.tex.forget_prefix("iso:");
        st.last_gen = cx.s.generation;
    }
    let frame = if st.animate {
        (cx.time * 10.0) as usize
    } else {
        0
    };
    // ---- phase 1: render everything that needs GameData
    let gd = match cx.gd.get(cx.s) {
        Ok(g) => g,
        Err(e) => {
            ui.label(
                RichText::new(format!("cannot build the game data: {e}"))
                    .color(Color32::from_rgb(230, 90, 80)),
            );
            return;
        }
    };
    let slab_names: Vec<String> = gd.slabs.iter().map(|s| s.name.clone()).collect();
    let nkinds = slab_names.len();
    st.kind = st.kind.min(nkinds.saturating_sub(1));
    let mut grid_tex = vec![];
    for s in 0..STYLES {
        let key = format!("iso:style:{}:{s}:{frame}", st.kind);
        let t = match cx.tex.peek(&key) {
            Some(t) => Some(t),
            None => render_style(cx.assets, gd, st.kind, s, 72.0, None, frame)
                .map(|cv| cx.tex.put(&ctx, &key, &cv.img)),
        };
        grid_tex.push(t);
    }
    let big = render_style(
        cx.assets,
        gd,
        st.kind,
        st.style,
        300.0,
        Some(st.subtile),
        frame,
    );
    let big_tex = big.as_ref().map(|cv| cx.tex.put(&ctx, "iso:big", &cv.img));
    // live 7x7 patch
    let dims = MapDims::new(7, 7);
    let mut lay = Layout::new(
        dims,
        st.neighbour.min(nkinds.saturating_sub(1)) as u8,
        st.owner,
    );
    for y in 1..6 {
        for x in 1..6 {
            lay.set(x, y, st.kind as u8, st.owner);
        }
    }
    let gen = MapGen::regenerate_all(&lay, gd);
    let unresolved = gen.unresolved;
    let patch = render_gen(cx.assets, gd, &gen, 420.0, frame);
    let patch_tex = patch
        .as_ref()
        .map(|cv| cx.tex.put(&ctx, "iso:patch", &cv.img));
    let resolved_cols = style_columns(gd, st.kind, st.style);

    // ---- phase 2: UI and edits
    let model = cx.s.slabset();
    let cur = model.style(st.kind, st.style);
    let src = model.source(st.kind, st.style);
    let overridden = model.is_overridden(st.kind, st.style);
    let mut new_style: Option<Style> = None;
    let mut revert = false;

    egui::Panel::left("slab-kinds")
        .resizable(true)
        .default_size(190.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("kinds")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (i, n) in slab_names.iter().enumerate() {
                        let ov = (0..STYLES).any(|s| cx.s.slabset().is_overridden(i, s));
                        let mut t = RichText::new(format!("{i:3} {n}")).monospace();
                        if ov {
                            t = t.color(layer_color(cx.s.target_kind()));
                        }
                        if ui.selectable_label(st.kind == i, t).clicked() {
                            st.kind = i;
                        }
                    }
                });
        });
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("slabset-main")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(format!(
                        "slab{} {}",
                        st.kind,
                        slab_names.get(st.kind).cloned().unwrap_or_default()
                    ));
                    ui.checkbox(&mut st.animate, "Animate textures");
                });
                // style grid: 3 fill styles x 9 + centre
                egui::Grid::new("styles")
                    .spacing([4.0, 2.0])
                    .show(ui, |ui| {
                        for row in 0..3 {
                            for col in 0..9 {
                                let s = row * 9 + col;
                                style_cell(ui, cx, st, s, grid_tex[s].as_ref());
                            }
                            if row == 0 {
                                style_cell(ui, cx, st, 27, grid_tex[27].as_ref());
                            }
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(SLAB_STYLE_NAMES[st.style]).strong());
                            if let Some(l) = src.columns {
                                let k = cx.s.layers.layers[l].kind.clone();
                                badge(ui, &format!("columns: {}", k.badge()), layer_color(&k));
                            }
                            if let Some(l) = src.objects {
                                let k = cx.s.layers.layers[l].kind.clone();
                                badge(ui, &format!("objects: {}", k.badge()), layer_color(&k));
                            }
                            if overridden {
                                ui.label(
                                    RichText::new("overridden here")
                                        .small()
                                        .color(layer_color(cx.s.target_kind())),
                                );
                            }
                        });
                        if let Some(t) = &big_tex {
                            let r =
                                image_fit(ui, t, egui::vec2(300.0, 345.0), egui::Sense::click());
                            if r.clicked() {
                                if let (Some(p), Some(cv)) =
                                    (r.interact_pointer_pos(), big.as_ref())
                                {
                                    let k = cv.img.w as f32 / r.rect.width();
                                    let id = cv.id_at(
                                        ((p.x - r.rect.min.x) * k) as i32,
                                        ((p.y - r.rect.min.y) * k) as i32,
                                    );
                                    if id > 0 {
                                        st.subtile = id as usize - 1;
                                    }
                                }
                            }
                            r.on_hover_text("Click a subtile to pick its column");
                        } else {
                            ui.label(weak("No texture data for previews"));
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Copy").clicked() {
                                st.clipboard = Some(cur.clone());
                            }
                            if ui
                                .add_enabled(st.clipboard.is_some(), egui::Button::new("Paste"))
                                .clicked()
                            {
                                new_style = st.clipboard.clone();
                            }
                            if ui.button("Rotate").clicked() {
                                new_style = Some(cur.rotated_cw());
                            }
                            if ui.button("Mirror").clicked() {
                                new_style = Some(cur.mirrored_x());
                            }
                            if ui
                                .add_enabled(overridden, egui::Button::new("Revert"))
                                .clicked()
                            {
                                revert = true;
                            }
                        });
                    });
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(
                                "Columns (column set IDs; 0 or negative = floor texture only)",
                            )
                            .small(),
                        );
                        egui::Grid::new("cols3x3").show(ui, |ui| {
                            for y in 0..3 {
                                for x in 0..3 {
                                    let i = y * 3 + x;
                                    let mut v = cur.columns[i];
                                    let sel = st.subtile == i;
                                    ui.vertical(|ui| {
                                        if ui.selectable_label(sel, format!("stl {i}")).clicked() {
                                            st.subtile = i;
                                        }
                                        if ui
                                            .add(egui::DragValue::new(&mut v).range(-2047..=16383))
                                            .changed()
                                        {
                                            let mut s = cur.clone();
                                            s.columns[i] = v;
                                            new_style = Some(s);
                                        }
                                        if resolved_cols[i].is_none() {
                                            ui.label(
                                                RichText::new("unresolved")
                                                    .small()
                                                    .color(Color32::from_rgb(230, 90, 80)),
                                            );
                                        }
                                    });
                                }
                                ui.end_row();
                            }
                        });
                        if let Some(c) = resolved_cols[st.subtile] {
                            ui.label(weak(format!(
                                "subtile {}: cubes {:?} floor {}",
                                st.subtile, c.cubes, c.floor_texture
                            )));
                        }
                        ui.separator();
                        ui.label(
                            RichText::new("live preview: 5x5 of this kind in a ring of").small(),
                        );
                        ui.horizontal(|ui| {
                            let opts: Vec<(i64, String)> = slab_names
                                .iter()
                                .enumerate()
                                .map(|(i, n)| (i as i64, n.clone()))
                                .collect();
                            let mut nb = slab_names.get(st.neighbour).cloned().unwrap_or_default();
                            if picker(ui, "neighbour", &mut nb, &opts, 150.0, |_, _, _| None) {
                                if let Some(i) = slab_names.iter().position(|n| *n == nb) {
                                    st.neighbour = i;
                                }
                            }
                            egui::ComboBox::from_id_salt("owner")
                                .selected_text(kfx_map::validate::owner_script_name(st.owner))
                                .show_ui(ui, |ui| {
                                    for o in 0..9u8 {
                                        ui.selectable_value(
                                            &mut st.owner,
                                            o,
                                            kfx_map::validate::owner_script_name(o),
                                        );
                                    }
                                });
                        });
                        if let Some(t) = &patch_tex {
                            image_fit(ui, t, egui::vec2(420.0, 360.0), egui::Sense::hover());
                        }
                        if unresolved > 0 {
                            ui.label(
                                RichText::new(format!(
                                    "{unresolved} subtiles with unresolved columns"
                                ))
                                .small()
                                .color(Color32::from_rgb(220, 180, 80)),
                            );
                        }
                    });
                });
                ui.separator();
                if let Some(s) = objects_table(ui, cx, st, &cur) {
                    new_style = Some(s);
                }
            });
    });
    if let Some(s) = new_style {
        cx.s.slabset().set_style(st.kind, st.style, s);
        cx.s.touched("slabset.toml");
    }
    if revert {
        cx.s.slabset().revert_style(st.kind, st.style);
        cx.s.touched("slabset.toml");
    }
}

fn style_cell(
    ui: &mut Ui,
    cx: &mut Cx,
    st: &mut SlabsetState,
    s: usize,
    tex: Option<&egui::TextureHandle>,
) {
    let ov = cx.s.slabset().is_overridden(st.kind, s);
    ui.vertical(|ui| {
        let (rect, r) = ui.allocate_exact_size(egui::vec2(72.0, 82.0), egui::Sense::click());
        if st.style == s {
            ui.painter()
                .rect_filled(rect, 3.0, Color32::from_rgb(60, 60, 90));
        }
        if let Some(t) = tex {
            ui.painter().image(
                t.id(),
                egui::Rect::from_min_size(rect.min, egui::vec2(72.0, 72.0 * 1.15).min(rect.size())),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if ov {
            ui.painter().circle_filled(
                rect.right_top() + egui::vec2(-6.0, 6.0),
                4.0,
                layer_color(cx.s.target_kind()),
            );
        }
        if r.clicked() {
            st.style = s;
        }
        ui.label(RichText::new(SLAB_STYLE_NAMES[s]).small());
    });
}

/// The style's slab objects; returns a new style when edited.
fn objects_table(ui: &mut Ui, cx: &mut Cx, st: &SlabsetState, cur: &Style) -> Option<Style> {
    ui.label(
        RichText::new(format!(
            "slab objects of {} ({})",
            SLAB_STYLE_NAMES[st.style],
            cur.objects.len()
        ))
        .strong(),
    );
    let mut out = cur.clone();
    let mut changed = false;
    let mut remove = None;
    let classes: Vec<(i64, String)> = kfx_formats::level_fx::CLASS_NAMES
        .iter()
        .map(|(n, i)| (*i as i64, n.to_string()))
        .collect();
    egui::Grid::new("slabobjs").striped(true).show(ui, |ui| {
        for h in [
            "type",
            "subtype",
            "subtile",
            "relative x y z",
            "range",
            "light",
            "orientation",
            "",
        ] {
            ui.label(weak(h));
        }
        ui.end_row();
        for (i, t) in out.objects.iter_mut().enumerate() {
            let mut v = ObjView::of(t);
            let before = v.clone();
            // thing type by name or number
            let mut tt = match v.thing_type.parse::<i64>() {
                Ok(n) => classes
                    .iter()
                    .find(|(c, _)| *c == n)
                    .map(|(_, s)| s.clone())
                    .unwrap_or(v.thing_type.clone()),
                Err(_) => v.thing_type.clone(),
            };
            if picker(ui, ("otype", i), &mut tt, &classes, 100.0, |_, _, _| None) {
                v.thing_type = tt.clone();
            }
            let cls = classes
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(&tt))
                .map(|(c, _)| *c)
                .unwrap_or(0);
            let opts = match cls {
                1 => cx.s.names("object"),
                7 => cx.s.names("effect_generator"),
                _ => vec![],
            };
            if opts.is_empty() {
                ui.add(egui::TextEdit::singleline(&mut v.subtype).desired_width(110.0));
            } else {
                let mut sub = match v.subtype.parse::<i64>() {
                    Ok(n) => opts
                        .iter()
                        .find(|(c, _)| *c == n)
                        .map(|(_, s)| s.clone())
                        .unwrap_or(v.subtype.clone()),
                    Err(_) => v.subtype.clone(),
                };
                if picker(ui, ("osub", i), &mut sub, &opts, 150.0, |_, _, _| None) {
                    v.subtype = sub;
                }
            }
            ui.add(egui::DragValue::new(&mut v.subtile).range(0..=8));
            ui.horizontal(|ui| {
                for c in v.rel.iter_mut() {
                    ui.add(egui::DragValue::new(c).speed(4.0));
                }
            });
            ui.add(egui::DragValue::new(&mut v.effect_range).range(0..=255));
            ui.checkbox(&mut v.is_light, "");
            ui.add(egui::DragValue::new(&mut v.orientation));
            if ui.small_button("x").clicked() {
                remove = Some(i);
            }
            if v != before {
                v.write(t);
                changed = true;
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        out.objects.remove(i);
        changed = true;
    }
    if ui.button("Add object").clicked() {
        let mut t = toml::Table::new();
        ObjView {
            is_light: false,
            subtile: st.subtile as i64,
            rel: [128, 128, 0],
            thing_type: "1".into(),
            subtype: "1".into(),
            effect_range: 0,
            orientation: 0,
        }
        .write(&mut t);
        out.objects.push(t);
        changed = true;
    }
    changed.then_some(out)
}

/// Terrain tab preview: the kind's `ALL` (or first) style.
pub fn terrain_preview(ui: &mut Ui, cx: &mut Cx, sec: &str) {
    let Some(kind) = sec
        .strip_prefix("slab")
        .and_then(|n| n.parse::<usize>().ok())
    else {
        return;
    };
    let ctx = ui.ctx().clone();
    let gen = cx.s.generation;
    let Ok(gd) = cx.gd.get(cx.s) else { return };
    let style = if gd.slab(kind).animated != 0 { 0 } else { 8 };
    let key = format!("iso:terrain:{kind}:{gen}");
    let tex = match cx.tex.peek(&key) {
        Some(t) => Some(t),
        None => render_style(cx.assets, gd, kind, style, 160.0, None, 0)
            .map(|cv| cx.tex.put(&ctx, &key, &cv.img)),
    };
    ui.horizontal(|ui| {
        if let Some(t) = tex {
            image_fit(ui, &t, egui::vec2(160.0, 184.0), egui::Sense::hover());
        }
        ui.label(weak(format!(
            "slabset style {} of slab{kind} (live, with unsaved edits)",
            SLAB_STYLE_NAMES[style]
        )));
    });
    ui.separator();
}
