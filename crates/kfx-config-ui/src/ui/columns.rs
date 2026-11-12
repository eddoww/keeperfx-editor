//! Column editor (`columnset.toml`) and cube editor (`cubes.cfg`).
use super::common::{badge, image_box, image_fit, layer_color, picker, weak};
use super::Cx;
use crate::columnset::{usage_counts, Col};
use crate::raster::{draw_columns, draw_cube, Canvas, ColumnDraw, Faces, Iso};
use egui::{Color32, RichText, Ui};
use kfx_formats::cfg::split_tokens;
use kfx_map::gamedata::{cube_flags, GameData, PLAYER_NAMES};

#[derive(Default)]
pub struct ColumnState {
    pub id: usize,
    pub filter: ColFilter,
    pub manual: bool,
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColFilter {
    #[default]
    All,
    Used,
    Unused,
    Overridden,
}

/// Render one column (big voxel preview).
pub fn render_column(
    assets: &mut Option<crate::assets::Assets>,
    gd: &GameData,
    c: &Col,
    size: f32,
    frame: usize,
) -> Option<Canvas> {
    let tiles = assets.as_mut()?.tiles.as_mut()?;
    let d = ColumnDraw {
        x: 0,
        y: 0,
        cubes: c.cubes_u16(),
        floor: c.floor_texture.clamp(0, u16::MAX as i64) as u16,
        tint: None,
        id: 1,
    };
    let h = (size * 1.6) as u32;
    let mut cv = Canvas::new(size as u32, h);
    let iso = Iso::fit(
        1.0,
        1.0,
        crate::raster::max_height(std::slice::from_ref(&d)).max(4.0),
        size,
        h as f32,
    );
    draw_columns(&mut cv, &iso, tiles, frame, &[d], &|cube| {
        gd.cubes
            .get(cube as usize)
            .map(|c| c.textures)
            .unwrap_or([0; 6])
    });
    Some(cv)
}

pub fn show_columns(ui: &mut Ui, cx: &mut Cx, st: &mut ColumnState) {
    let ctx = ui.ctx().clone();
    let frame = (cx.time * 10.0) as usize;
    let usage = usage_counts(&cx.s.slabset().current);
    let cs = cx.s.columnset();
    let count = cs.count();
    st.id = st.id.min(count.saturating_sub(1));
    let col = cs.col(st.id);
    let src = cs.source(st.id);
    let overridden = cs.is_overridden(st.id);
    let inherited = cs.inherited_col(st.id);
    let identical: Vec<usize> = cs
        .find_identical(&col)
        .into_iter()
        .filter(|&i| i != st.id)
        .collect();
    let ids: Vec<(usize, bool)> = (0..count).map(|i| (i, cs.is_overridden(i))).collect();
    // render
    let (gd_err, big_tex) = match cx.gd.get(cx.s) {
        Ok(gd) => (
            None,
            render_column(cx.assets, gd, &col, 220.0, frame)
                .map(|cv| cx.tex.put(&ctx, "iso:column", &cv.img)),
        ),
        Err(e) => (Some(e), None),
    };
    let mut new_col: Option<Col> = None;
    let mut revert = false;
    egui::Panel::left("col-list")
        .resizable(true)
        .default_size(200.0)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (f, l) in [
                    (ColFilter::All, "all"),
                    (ColFilter::Used, "used"),
                    (ColFilter::Unused, "unused"),
                    (ColFilter::Overridden, "overridden"),
                ] {
                    ui.selectable_value(&mut st.filter, f, l);
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("cols")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (i, ov) in &ids {
                        let u = usage.get(i).copied().unwrap_or(0);
                        let show = match st.filter {
                            ColFilter::All => true,
                            ColFilter::Used => u > 0,
                            ColFilter::Unused => u == 0,
                            ColFilter::Overridden => *ov,
                        };
                        if !show {
                            continue;
                        }
                        let mut t = RichText::new(format!("column{i:<5} used {u}")).monospace();
                        if *ov {
                            t = t.color(layer_color(cx.s.target_kind()));
                        }
                        if ui.selectable_label(st.id == *i, t).clicked() {
                            st.id = *i;
                        }
                    }
                });
        });
    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(format!("column{}", st.id));
            let mut id = st.id;
            if ui
                .add(egui::DragValue::new(&mut id).range(0..=count.saturating_sub(1)))
                .changed()
            {
                st.id = id;
            }
            if let Some(l) = src {
                let k = cx.s.layers.layers[l].kind.clone();
                badge(ui, &k.badge(), layer_color(&k));
            }
            if overridden {
                ui.label(
                    RichText::new("overridden here")
                        .small()
                        .color(layer_color(cx.s.target_kind())),
                );
                if ui.small_button("Revert").clicked() {
                    revert = true;
                }
            }
            ui.label(weak(format!(
                "used by {} slabset subtiles",
                usage.get(&st.id).copied().unwrap_or(0)
            )));
        });
        if let Some(e) = gd_err {
            ui.label(RichText::new(e).color(Color32::from_rgb(230, 90, 80)));
        }
        ui.horizontal(|ui| {
            if let Some(t) = &big_tex {
                image_fit(ui, t, egui::vec2(220.0, 352.0), egui::Sense::hover());
            }
            ui.vertical(|ui| {
                let mut c = col;
                let cube_opts = cx.s.names("cube");
                ui.label(RichText::new("Cubes, top to bottom").strong());
                for z in (0..8).rev() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{z}"));
                        let mut v = c.cubes[z];
                        let mut name = cube_opts
                            .iter()
                            .find(|(i, _)| *i == v)
                            .map(|(_, n)| n.clone())
                            .unwrap_or_else(|| v.to_string());
                        if v == 0 {
                            name = "(empty)".into();
                        }
                        let mut opts = vec![(0i64, "(empty)".to_string())];
                        opts.extend(cube_opts.iter().cloned());
                        let ctx2 = ctx.clone();
                        if picker(ui, ("cube", z), &mut name, &opts, 200.0, |_, _, id| {
                            cube_icon(&ctx2, cx, id)
                        }) {
                            v = opts
                                .iter()
                                .find(|(_, n)| *n == name)
                                .map(|(i, _)| *i)
                                .unwrap_or(0);
                        }
                        if ui
                            .add(egui::DragValue::new(&mut v).range(0..=1023))
                            .changed()
                            || v != c.cubes[z]
                        {
                            c.cubes[z] = v;
                        }
                        let t = cube_icon(&ctx, cx, v);
                        image_box(ui, t.as_ref(), egui::vec2(20.0, 20.0));
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Floor texture");
                    ui.add(egui::DragValue::new(&mut c.floor_texture).range(0..=1543));
                    let t = cx.tex.tile(&ctx, cx.assets, c.floor_texture as u16, frame);
                    image_box(ui, t.as_ref(), egui::vec2(24.0, 24.0));
                    ui.label("Orientation");
                    ui.add(egui::DragValue::new(&mut c.orientation).range(0..=255));
                });
                let auto = c.auto();
                ui.checkbox(&mut st.manual, "Set height / lintel / solid mask by hand")
                    .on_hover_text(
                        // create_column, src/map_columns.c:293
                        "The game recomputes them from the cubes but matches the slabset to the \
                        column with the SolidMask as written: a mask that disagrees with the cubes \
                        leaves the slabset entry unresolved.",
                    );
                egui::Grid::new("auto").show(ui, |ui| {
                    for (label, val, a) in [
                        ("height", &mut c.height, auto.height),
                        ("lintel", &mut c.lintel, auto.lintel),
                        ("solid mask", &mut c.solid_mask, auto.solid_mask),
                    ] {
                        ui.label(label);
                        if st.manual {
                            ui.add(egui::DragValue::new(val));
                        } else {
                            *val = a;
                            ui.label(RichText::new(a.to_string()).monospace());
                        }
                        ui.label(weak(format!("auto {a}")));
                        if *val != a {
                            ui.label(
                                RichText::new("differs from auto")
                                    .small()
                                    .color(Color32::from_rgb(220, 180, 80)),
                            );
                        }
                        ui.end_row();
                    }
                });
                if c != col {
                    new_col = Some(c);
                }
            });
        });
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "identical columns: {}",
                if identical.is_empty() {
                    "none".into()
                } else {
                    identical
                        .iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ));
            if let Some(i) = inherited {
                if i != col {
                    ui.label(weak(format!(
                        "inherited: cubes {:?} floor {}",
                        i.cubes, i.floor_texture
                    )));
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Copy to first free ID").clicked() {
                let n = cx.s.columnset().first_free(&usage);
                cx.s.columnset().set_col(n, col);
                cx.s.touched("columnset.toml");
                st.message = Some(format!("copied to column{n}"));
                st.id = n;
            }
            if ui
                .button("Clear unused overrides")
                .on_hover_text("Drop this layer's overrides of columns no slabset entry uses")
                .clicked()
            {
                let v = cx.s.columnset().clear_unused_overrides(&usage);
                cx.s.touched("columnset.toml");
                st.message = Some(format!("dropped {} unused column overrides", v.len()));
            }
            let unused = (1..count)
                .filter(|i| usage.get(i).copied().unwrap_or(0) == 0)
                .count();
            ui.label(weak(format!(
                "{unused} of {count} columns unused by the slabset"
            )));
            if let Some(m) = &st.message {
                ui.label(weak(m.clone()));
            }
        });
    });
    if let Some(c) = new_col {
        cx.s.columnset().set_col(st.id, c);
        cx.s.touched("columnset.toml");
    }
    if revert {
        cx.s.columnset().revert_col(st.id);
        cx.s.touched("columnset.toml");
    }
}

/// Top face of a cube as a thumbnail.
fn cube_icon(ctx: &egui::Context, cx: &mut Cx, cube: i64) -> Option<egui::TextureHandle> {
    if cube <= 0 {
        return None;
    }
    let tex = cube_textures(cx, cube as usize)?;
    cx.tex.tile(ctx, cx.assets, tex[4], 0)
}

/// Face textures of a cube as of the target layer.
fn cube_textures(cx: &mut Cx, cube: usize) -> Option<[u16; 6]> {
    let v =
        cx.s.cfg("cubes.cfg")?
            .at_target(&format!("cube{cube}"), "Textures")?
            .value;
    let t = split_tokens(&v);
    Some(std::array::from_fn(|i| {
        t.get(i).and_then(|x| x.parse().ok()).unwrap_or(0)
    }))
}

// ------------------------------------------------------------------------------ cubes

#[derive(Default)]
pub struct CubeState {
    pub id: usize,
    pub filter: String,
    /// Face whose texture is being picked.
    pub picking: Option<usize>,
    pub atlas_page: usize,
}

const FACE_NAMES: [&str; 6] = ["north", "east", "south", "west", "top", "bottom"];

pub fn show_cubes(ui: &mut Ui, cx: &mut Cx, st: &mut CubeState) {
    let ctx = ui.ctx().clone();
    let frame = (cx.time * 10.0) as usize;
    let file = "cubes.cfg";
    let names = cx.s.names("cube");
    if st.id == 0 && !names.is_empty() {
        st.id = names
            .iter()
            .find(|(i, _)| *i > 0)
            .map_or(0, |(i, _)| *i as usize);
    }
    let sec = format!("cube{}", st.id);
    let tex = cube_textures(cx, st.id).unwrap_or([0; 6]);
    // iso preview of the cube
    let preview = cx
        .assets
        .as_mut()
        .and_then(|a| a.tiles.as_mut())
        .map(|tiles| {
            let mut cv = Canvas::new(180, 200);
            let iso = Iso::fit(1.0, 1.0, 1.0, 180.0, 200.0);
            draw_cube(
                &mut cv,
                &iso,
                tiles,
                frame,
                0.0,
                0.0,
                0.0,
                tex,
                Faces::ALL,
                None,
                1,
            );
            cv
        });
    let ptex = preview.map(|cv| cx.tex.put(&ctx, "iso:cube", &cv.img));
    egui::Panel::left("cube-list")
        .resizable(true)
        .default_size(210.0)
        .show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text("filter names / IDs"));
            egui::ScrollArea::vertical()
                .id_salt("cubes")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let f = st.filter.to_ascii_lowercase();
                    for (i, n) in &names {
                        let label = format!("{i:4} {n}");
                        if !f.is_empty() && !label.to_ascii_lowercase().contains(&f) {
                            continue;
                        }
                        let ov =
                            cx.s.cfg(file)
                                .is_some_and(|c| c.target_has_section(&format!("cube{i}")));
                        ui.horizontal(|ui| {
                            let t = cube_icon(&ctx, cx, *i);
                            image_box(ui, t.as_ref(), egui::vec2(18.0, 18.0));
                            let mut rt = RichText::new(label).monospace();
                            if ov {
                                rt = rt.color(layer_color(cx.s.target_kind()));
                            }
                            if ui.selectable_label(st.id == *i as usize, rt).clicked() {
                                st.id = *i as usize;
                            }
                        });
                    }
                });
        });
    egui::CentralPanel::default().show(ui, |ui| {
        let name = cx.s.cfg_value(file, &sec, "Name").unwrap_or_default();
        ui.heading(format!("{sec}  {name}"));
        ui.horizontal(|ui| {
            if let Some(t) = &ptex {
                image_fit(ui, t, egui::vec2(180.0, 200.0), egui::Sense::hover());
            }
            ui.vertical(|ui| {
                let mut faces = tex;
                egui::Grid::new("faces").show(ui, |ui| {
                    for (f, fname) in FACE_NAMES.iter().enumerate() {
                        ui.label(*fname);
                        let t = cx.tex.tile(&ctx, cx.assets, faces[f], frame);
                        if image_box(ui, t.as_ref(), egui::vec2(36.0, 36.0))
                            .on_hover_text("Click to pick from the atlas")
                            .clicked()
                        {
                            st.picking = Some(f);
                        }
                        let mut v = faces[f] as i64;
                        if ui
                            .add(egui::DragValue::new(&mut v).range(0..=1543))
                            .changed()
                        {
                            faces[f] = v as u16;
                        }
                        if (544..1000).contains(&faces[f]) {
                            ui.label(weak("animated"));
                        }
                        ui.end_row();
                    }
                });
                if faces != tex {
                    let v: Vec<String> = faces.iter().map(|x| x.to_string()).collect();
                    cx.s.set_cfg(file, &sec, "Textures", &v.join(" "));
                }
            });
            ui.vertical(|ui| {
                // ownership
                let og =
                    cx.s.cfg_value(file, &sec, "OwnershipGroup")
                        .and_then(|v| v.trim().parse::<i64>().ok())
                        .unwrap_or(0);
                let mut g = og;
                ui.horizontal(|ui| {
                    ui.label("Ownership group");
                    ui.add(egui::DragValue::new(&mut g).range(0..=19));
                });
                if g != og {
                    cx.s.set_cfg(file, &sec, "OwnershipGroup", &g.to_string());
                }
                let owner = cx.s.cfg_value(file, &sec, "Owner").unwrap_or_default();
                let mut o = owner.clone();
                egui::ComboBox::from_id_salt("owner")
                    .selected_text(if o.is_empty() { "-".into() } else { o.clone() })
                    .show_ui(ui, |ui| {
                        for p in PLAYER_NAMES {
                            ui.selectable_value(&mut o, p.to_string(), p);
                        }
                    });
                if o != owner {
                    cx.s.set_cfg(file, &sec, "Owner", &o);
                }
                if g == 0 && !o.is_empty() {
                    // src/config_cubes.c:94
                    ui.label(
                        RichText::new("Owner without OwnershipGroup is ignored")
                            .small()
                            .color(Color32::from_rgb(220, 180, 80)),
                    );
                }
                ui.label("Properties");
                let props = cx.s.cfg_value(file, &sec, "Properties").unwrap_or_default();
                let toks = split_tokens(&props);
                let mut set: Vec<String> = cube_flags::NAMES
                    .iter()
                    .filter(|(n, _)| toks.iter().any(|t| t.eq_ignore_ascii_case(n)))
                    .map(|(n, _)| n.to_string())
                    .collect();
                let mut changed = false;
                for (n, _) in cube_flags::NAMES {
                    let mut on = set.iter().any(|s| s == n);
                    if ui.checkbox(&mut on, n).changed() {
                        changed = true;
                        if on {
                            set.push(n.to_string());
                        } else {
                            set.retain(|s| s != n);
                        }
                    }
                }
                if changed {
                    cx.s.set_cfg(file, &sec, "Properties", &set.join(" "));
                }
            });
        });
        ui.separator();
        super::form::cfg_section_form(ui, cx, file, &sec, "");
    });
    // atlas picker window
    if let Some(face) = st.picking {
        let mut open = true;
        let mut chosen: Option<u16> = None;
        egui::Window::new(format!("texture for the {} face", FACE_NAMES[face]))
            .open(&mut open)
            .default_size([560.0, 520.0])
            .show(&ctx, |ui| {
                ui.horizontal(|ui| {
                    for (p, l) in ["tmapa 0-543", "animated 544-999", "tmapb 1000-1543"]
                        .iter()
                        .enumerate()
                    {
                        ui.selectable_value(&mut st.atlas_page, p, *l);
                    }
                });
                let range: Vec<u16> = match st.atlas_page {
                    0 => (0..544).collect(),
                    1 => {
                        let names = cx.s.names("texture");
                        names.iter().map(|(i, _)| *i as u16).collect()
                    }
                    _ => (1000..1544).collect(),
                };
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                            for id in range {
                                let t = cx.tex.tile(&ctx, cx.assets, id, frame);
                                if image_box(ui, t.as_ref(), egui::vec2(32.0, 32.0))
                                    .on_hover_text(id.to_string())
                                    .clicked()
                                {
                                    chosen = Some(id);
                                }
                            }
                        });
                    });
            });
        if let Some(id) = chosen {
            let mut faces = tex;
            faces[face] = id;
            let v: Vec<String> = faces.iter().map(|x| x.to_string()).collect();
            cx.s.set_cfg(file, &sec, "Textures", &v.join(" "));
            st.picking = None;
        } else if !open {
            st.picking = None;
        }
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
}
