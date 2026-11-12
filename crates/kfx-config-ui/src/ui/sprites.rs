//! Sprite / zip browser: built-in animations (creature.tab), custom zip animations and icons,
//! GUI sheets, zips of the target layer; import of PNG frames into the map / campaign zip with
//! generated `sprites.json` / `icons.json` ([`crate::spritezip`]).
use super::common::{image_box, weak};
use super::Cx;
use crate::layers::LayerKind;
use crate::spritezip::{check_sprite, list_zip, map_zip_name, write_zip, NewIcon, NewSprite};
use egui::{Color32, RichText, Ui};
use kfx_gfx::keeper::ANIMATION_COUNT;
use kfx_gfx::Rgba;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Builtin,
    Custom,
    Icons,
    Gui1,
    Gui2,
    Zips,
    Import,
}

pub struct SpriteState {
    pub view: View,
    pub page: usize,
    pub filter: String,
    pub selected: Option<String>,
    pub name: String,
    pub rotatable: bool,
    pub as_icon: bool,
    /// One PNG path per line; directions separated by a `---` line.
    pub paths: String,
    pub zip_name: String,
    pub message: Option<(String, bool)>,
}

impl Default for SpriteState {
    fn default() -> Self {
        SpriteState {
            view: View::Builtin,
            page: 0,
            filter: String::new(),
            selected: None,
            name: "MY_SPRITE".into(),
            rotatable: false,
            as_icon: false,
            paths: String::new(),
            zip_name: "custom_sprites.zip".into(),
            message: None,
        }
    }
}

const PAGE: usize = 96;

/// Where imports go for the target layer.
pub fn target_zip(cx: &Cx, zip_name: &str) -> Option<PathBuf> {
    let l = &cx.s.layers.layers[cx.s.target()];
    match &l.kind {
        LayerKind::Map(n) => Some(l.dir.as_ref()?.join(map_zip_name(*n))),
        LayerKind::Campaign | LayerKind::Mod(_) => Some(l.dir.as_ref()?.join(zip_name)),
        LayerKind::Base => None,
    }
}

pub fn show(ui: &mut Ui, cx: &mut Cx, st: &mut SpriteState) {
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        for (v, l) in [
            (View::Builtin, "built-in animations"),
            (View::Custom, "custom animations"),
            (View::Icons, "custom icons"),
            (View::Gui1, "gui1 sheet"),
            (View::Gui2, "gui2 sheet"),
            (View::Zips, "zips of the target layer"),
            (View::Import, "import PNG frames"),
        ] {
            if ui.selectable_label(st.view == v, l).clicked() {
                st.view = v;
                st.page = 0;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut st.filter)
                .hint_text("filter")
                .desired_width(160.0),
        );
        if ui.small_button("<").clicked() {
            st.page = st.page.saturating_sub(1);
        }
        ui.label(format!("page {}", st.page + 1));
        if ui.small_button(">").clicked() {
            st.page += 1;
        }
        if let Some(s) = &st.selected {
            ui.label(RichText::new(format!("selected: {s}")).strong());
        }
    });
    ui.separator();
    let frame_t = (cx.time * 10.0) as usize;
    egui::ScrollArea::vertical()
        .id_salt("sprites")
        .auto_shrink([false, false])
        .show(ui, |ui| match st.view {
            View::Builtin => {
                let ids: Vec<usize> = (1..ANIMATION_COUNT)
                    .filter(|i| st.filter.is_empty() || i.to_string().contains(&st.filter))
                    .collect();
                grid(
                    ui,
                    &ids.iter()
                        .skip(st.page * PAGE)
                        .take(PAGE)
                        .map(|i| (i.to_string(), i.to_string()))
                        .collect::<Vec<_>>(),
                    cx,
                    st,
                    frame_t,
                    &ctx,
                );
            }
            View::Custom => {
                let items: Vec<(String, String)> = cx
                    .assets
                    .as_ref()
                    .map(|a| {
                        a.custom
                            .animations()
                            .iter()
                            .map(|an| {
                                (
                                    an.name.clone(),
                                    format!("{} ({}, {} frames)", an.name, an.source, an.frames()),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let items: Vec<(String, String)> = items
                    .into_iter()
                    .filter(|(n, _)| {
                        st.filter.is_empty()
                            || n.to_ascii_lowercase()
                                .contains(&st.filter.to_ascii_lowercase())
                    })
                    .collect();
                grid(
                    ui,
                    &items
                        .into_iter()
                        .skip(st.page * PAGE)
                        .take(PAGE)
                        .collect::<Vec<_>>(),
                    cx,
                    st,
                    frame_t,
                    &ctx,
                );
            }
            View::Icons => {
                let items: Vec<(String, String)> = cx
                    .assets
                    .as_ref()
                    .map(|a| {
                        a.custom
                            .icons()
                            .iter()
                            .map(|i| (i.name.clone(), format!("{} ({})", i.name, i.source)))
                            .collect()
                    })
                    .unwrap_or_default();
                ui.horizontal_wrapped(|ui| {
                    for (n, label) in items
                        .iter()
                        .filter(|(n, _)| {
                            st.filter.is_empty()
                                || n.to_ascii_lowercase()
                                    .contains(&st.filter.to_ascii_lowercase())
                        })
                        .skip(st.page * PAGE)
                        .take(PAGE)
                    {
                        let assets = &*cx.assets;
                        let t = cx
                            .tex
                            .get(&ctx, &format!("cicon:{n}"), || assets.as_ref()?.panel(n));
                        ui.allocate_ui_with_layout(
                            egui::vec2(90.0, 100.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(90.0);
                                if image_box(ui, t.as_ref(), egui::vec2(64.0, 64.0))
                                    .on_hover_text(label.clone())
                                    .clicked()
                                {
                                    st.selected = Some(n.clone());
                                }
                                ui.label(RichText::new(n).small());
                            },
                        );
                    }
                });
            }
            View::Gui1 | View::Gui2 => {
                let gui1 = st.view == View::Gui1;
                let n = cx
                    .assets
                    .as_ref()
                    .and_then(|a| {
                        if gui1 {
                            a.gui1.as_ref()
                        } else {
                            a.gui2.as_ref()
                        }
                    })
                    .map_or(0, |s| s.len());
                ui.horizontal_wrapped(|ui| {
                    for i in (1..n)
                        .filter(|i| st.filter.is_empty() || i.to_string().contains(&st.filter))
                        .skip(st.page * PAGE)
                        .take(PAGE)
                    {
                        let assets = &*cx.assets;
                        let v = i.to_string();
                        let t = cx.tex.get(
                            &ctx,
                            &format!("icon:{}:{i}", if gui1 { 1 } else { 2 }),
                            || {
                                let a = assets.as_ref()?;
                                if gui1 {
                                    a.portrait(&v)
                                } else {
                                    a.panel(&v)
                                }
                            },
                        );
                        let name = if gui1 {
                            kfx_gfx::sheet::button_sprite_name(i as u16)
                        } else {
                            kfx_gfx::sheet::panel_sprite_name(i as u16)
                        };
                        ui.allocate_ui_with_layout(
                            egui::vec2(70.0, 90.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(70.0);
                                if image_box(ui, t.as_ref(), egui::vec2(56.0, 56.0))
                                    .on_hover_text(name.unwrap_or("").to_string())
                                    .clicked()
                                {
                                    st.selected = Some(i.to_string());
                                }
                                ui.label(RichText::new(i.to_string()).small());
                            },
                        );
                    }
                });
            }
            View::Zips => zips(ui, cx),
            View::Import => import(ui, cx, st),
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
}

fn grid(
    ui: &mut Ui,
    items: &[(String, String)],
    cx: &mut Cx,
    st: &mut SpriteState,
    frame: usize,
    ctx: &egui::Context,
) {
    ui.horizontal_wrapped(|ui| {
        for (value, label) in items {
            let frames = cx.tex.anim(ctx, cx.assets, value, 0);
            let t = (!frames.is_empty()).then(|| frames[frame % frames.len()].clone());
            ui.allocate_ui_with_layout(
                egui::vec2(84.0, 105.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(84.0);
                    if image_box(ui, t.as_ref(), egui::vec2(72.0, 72.0))
                        .on_hover_text(format!("{label}\n{} frames", frames.len()))
                        .clicked()
                    {
                        st.selected = Some(value.clone());
                    }
                    ui.label(RichText::new(value).small());
                },
            );
        }
    });
}

fn zips(ui: &mut Ui, cx: &mut Cx) {
    let l = cx.s.layers.layers[cx.s.target()].clone();
    let Some(dir) = l.dir.clone() else {
        ui.label(weak("The target layer has no folder"));
        return;
    };
    let names: Vec<String> =
        cx.s.fs
            .list_dir(&dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.to_ascii_lowercase().ends_with(".zip"))
            .collect();
    let names: Vec<String> = match &l.kind {
        LayerKind::Map(n) => names
            .into_iter()
            .filter(|z| z.eq_ignore_ascii_case(&map_zip_name(*n)))
            .collect(),
        _ => names,
    };
    if names.is_empty() {
        ui.label(weak(format!("no zips in {}", dir.display())));
    }
    for z in names {
        let p = dir.join(&z);
        egui::CollapsingHeader::new(RichText::new(&z).monospace()).show(ui, |ui| {
            match cx.s.fs.read(&p) {
                Ok(Some(b)) => match list_zip(&b) {
                    Ok(v) => {
                        for (n, s) in v {
                            ui.label(
                                RichText::new(format!("{n}  ({s} bytes)"))
                                    .monospace()
                                    .small(),
                            );
                        }
                    }
                    Err(e) => {
                        ui.label(RichText::new(e).color(Color32::from_rgb(230, 90, 80)));
                    }
                },
                _ => {
                    ui.label("unreadable");
                }
            }
        });
    }
}

fn load_pngs(cx: &Cx, text: &str) -> Result<Vec<Vec<Rgba>>, String> {
    let mut dirs: Vec<Vec<Rgba>> = vec![vec![]];
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line == "---" {
            dirs.push(vec![]);
            continue;
        }
        let b =
            cx.s.fs
                .read(Path::new(line))
                .map_err(|e| format!("{line}: {e}"))?
                .ok_or_else(|| format!("{line}: not found"))?;
        dirs.last_mut()
            .unwrap()
            .push(Rgba::from_png(&b).map_err(|e| format!("{line}: {e}"))?);
    }
    dirs.retain(|d| !d.is_empty());
    Ok(dirs)
}

fn import(ui: &mut Ui, cx: &mut Cx, st: &mut SpriteState) {
    let target = target_zip(cx, &st.zip_name);
    ui.label(RichText::new("Import PNG frames as a custom sprite or icon").strong());
    egui::Grid::new("import").show(ui, |ui| {
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut st.name).desired_width(200.0));
        ui.end_row();
        ui.label("Kind");
        ui.horizontal(|ui| {
            ui.radio_value(&mut st.as_icon, false, "Sprite (animation)");
            ui.radio_value(&mut st.as_icon, true, "Icon (GUI image)");
            if !st.as_icon {
                ui.checkbox(&mut st.rotatable, "Rotatable (5 directions)");
            }
        });
        ui.end_row();
        ui.label("PNG files");
        ui.add(
            egui::TextEdit::multiline(&mut st.paths)
                .desired_rows(6)
                .desired_width(420.0)
                .hint_text("/path/frame1.png\n/path/frame2.png\n---\n(next direction)"),
        );
        ui.end_row();
        if !matches!(cx.s.target_kind(), LayerKind::Map(_)) {
            ui.label("Zip name");
            ui.add(egui::TextEdit::singleline(&mut st.zip_name).desired_width(200.0));
            ui.end_row();
        }
        ui.label("Writes");
        ui.label(
            target
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(the base layer cannot hold zips)".into()),
        );
        ui.end_row();
    });
    let frames = load_pngs(cx, &st.paths);
    match &frames {
        Ok(d) if !d.is_empty() => {
            let n: Vec<usize> = d.iter().map(Vec::len).collect();
            ui.label(weak(format!("{} direction(s), frames {:?}", d.len(), n)));
            if !st.as_icon {
                let s = NewSprite {
                    name: st.name.clone(),
                    rotatable: st.rotatable,
                    td: d.clone(),
                    ..Default::default()
                };
                for p in check_sprite(&s) {
                    ui.label(
                        RichText::new(p)
                            .small()
                            .color(Color32::from_rgb(220, 180, 80)),
                    );
                }
            }
        }
        Ok(_) => {
            ui.label(weak("List PNG files, one per line"));
        }
        Err(e) => {
            ui.label(RichText::new(e).color(Color32::from_rgb(230, 90, 80)));
        }
    }
    if ui
        .add_enabled(
            target.is_some() && frames.as_ref().is_ok_and(|d| !d.is_empty()),
            egui::Button::new("Write into the zip"),
        )
        .clicked()
    {
        let target = target.unwrap();
        let d = frames.unwrap();
        let existing = cx.s.fs.read(&target).ok().flatten();
        let r = if st.as_icon {
            write_zip(
                existing.as_deref(),
                &[],
                &[NewIcon {
                    name: st.name.clone(),
                    frames: d.into_iter().flatten().collect(),
                }],
            )
        } else {
            write_zip(
                existing.as_deref(),
                &[NewSprite {
                    name: st.name.clone(),
                    rotatable: st.rotatable,
                    td: d,
                    ..Default::default()
                }],
                &[],
            )
        };
        st.message = Some(
            match r.and_then(|b| cx.s.fs.write(&target, &b).map_err(|e| e.to_string())) {
                Ok(()) => {
                    cx.s.push_event(crate::session::ConfigEvent::Saved {
                        file: target
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        path: target.clone(),
                        deleted: false,
                    });
                    (
                        format!("wrote {} (reload the assets to see it)", target.display()),
                        false,
                    )
                }
                Err(e) => (e, true),
            },
        );
    }
    if let Some((m, e)) = &st.message {
        ui.label(RichText::new(m).color(if *e {
            Color32::from_rgb(230, 90, 80)
        } else {
            Color32::from_rgb(110, 200, 110)
        }));
    }
}
