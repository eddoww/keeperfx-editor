//! Tileset editor: any texture set found in the layers (picker), shown as the game loads it
//! (all layers, the source layer of each file named), tmapa / tmapb sheet view with zoom;
//! export a whole file as PNG or DAT, import a PNG (palette quantisation, optional dithering)
//! or DAT into the target layer's `tmapXNNN.dat` override (or the campaign's, see
//! [`crate::tiles::tmap_write_layer`]); export one tile as PNG, paste an image onto a tile;
//! texture packs (export a `filelist_tmapXNNN.txt` + PNGs, import them back, live reload while
//! they are edited); revert one file or both to the layer below; and the `textureanim.toml`
//! frame editor with playback.
//!
//! The shown set replaces the one in [`crate::assets::Assets::tiles`], so the other editors'
//! previews use it too.
//!
//! Texture files are written through the host's [`crate::ConfigFs`] at once (they are not part
//! of the session's unsaved edits); each write or removal is reported as
//! [`crate::ConfigEvent::Saved`].
use super::common::{badge, layer_color};
use super::common::{image_box, weak};
use super::Cx;
use crate::tiles::{
    atlas_png, available_sets, export_pack, import_atlas_dat, import_atlas_png, import_pack,
    load_effective_set, lower_tmap, pack_fingerprint, paste_tile, tile_rgba, tmap_override_path,
    tmap_write_layer, Quantizer,
};
use egui::{Color32, RichText, Ui};
use kfx_gfx::textures::{Part, TextureAnim, ATLAS_H, ATLAS_W, TILES_PER_ROW, TILE_DIM};
use kfx_gfx::Rgba;
use std::path::{Path, PathBuf};

pub struct TileState {
    /// Texture set shown (`None`: the one the host loaded).
    pub set: Option<u32>,
    /// Write map-local even when [`crate::tiles::tmap_write_layer`] would pick the campaign.
    pub map_local: bool,
    /// `(set, target, epoch)` of the loaded view.
    view: Option<(u32, usize, u64)>,
    epoch: u64,
    /// Source `(layer, file)` of the shown tmapa / tmapb.
    sources: [Option<(usize, PathBuf)>; 2],
    sets: Vec<u32>,
    pub part: Part,
    pub tile: usize,
    pub zoom: f32,
    pub dither: bool,
    /// File for export / import (`.dat` = raw texture file, else PNG).
    pub path: String,
    /// Folder of the texture pack.
    pub pack_dir: String,
    /// Re-import the pack whenever its filelist or PNGs change.
    pub live_reload: bool,
    pack_seen: [Option<u64>; 2],
    pack_poll: f64,
    /// Overrides of the target layer: `(time, target, set, files)`, refreshed every 2 s.
    ov_cache: Option<(f64, usize, u32, Vec<(Part, PathBuf)>)>,
    pub message: Option<(String, bool)>,
    pub anim_slot: Option<i64>,
    pub show_sheet: bool,
    anim_gen: u64,
}

impl Default for TileState {
    fn default() -> Self {
        TileState {
            set: None,
            map_local: false,
            view: None,
            epoch: 0,
            sources: [None, None],
            sets: vec![],
            part: Part::A,
            tile: 0,
            zoom: 2.0,
            dither: false,
            path: std::env::temp_dir().join("tmap.png").display().to_string(),
            pack_dir: std::env::temp_dir()
                .join("kfx-tmap-pack")
                .display()
                .to_string(),
            live_reload: false,
            pack_seen: [None, None],
            pack_poll: 0.0,
            ov_cache: None,
            message: None,
            anim_slot: None,
            show_sheet: true,
            anim_gen: u64::MAX,
        }
    }
}

fn texture_id(part: Part, tile: usize) -> u16 {
    tile as u16 + if part == Part::B { 1000 } else { 0 }
}

/// Rebuild the animation table used by previews from the edited `textureanim.toml`.
fn sync_anim(cx: &mut Cx) {
    let Some(t) = cx.s.toml("textureanim.toml") else {
        return;
    };
    let mut table = toml::Table::new();
    for p in t.section_paths() {
        if p.len() != 1 {
            continue;
        }
        let mut sec = toml::Table::new();
        for k in t.keys(&p) {
            let mut kp = p.clone();
            kp.push(k.clone());
            if let Some(v) = t.at_target(&kp) {
                sec.insert(k, v.value);
            }
        }
        table.insert(p[0].clone(), toml::Value::Table(sec));
    }
    if let Ok(a) = TextureAnim::parse(&table.to_string()) {
        if let Some(tiles) = cx.assets.as_mut().and_then(|a| a.tiles.as_mut()) {
            tiles.set_anim(a);
        }
        cx.tex.forget_prefix("tile:");
        cx.tex.forget_prefix("iso:");
    }
}

/// Load set `index` as the game sees it (all layers) into the shared tile cache.
fn load_view(cx: &mut Cx, st: &mut TileState, index: u32) -> Result<(), String> {
    let (set, src) = load_effective_set(&*cx.s.fs, &cx.s.layers, index)?;
    let tiles = cx
        .assets
        .as_mut()
        .and_then(|a| a.tiles.as_mut())
        .ok_or("No texture set loaded")?;
    tiles.set_textures(set);
    cx.tex.forget_prefix("tile:");
    cx.tex.forget_prefix("iso:");
    cx.tex.forget_prefix("atlas:");
    st.sources = src;
    Ok(())
}

/// Where an import of `part` goes: `(layer, path, campaign rule applied)`.
fn destination(
    cx: &Cx,
    part: Part,
    index: u32,
    map_local: bool,
) -> Result<(usize, PathBuf, bool), String> {
    let (l, rule) = tmap_write_layer(
        &*cx.s.fs,
        &cx.s.layers,
        cx.s.target(),
        part,
        index,
        map_local,
    );
    let layer = &cx.s.layers.layers[l];
    let path = crate::tiles::layer_tmap(&*cx.s.fs, layer, part, index)
        .or_else(|| tmap_override_path(layer, part, index))
        .ok_or("the target layer cannot hold texture files (pick a map, campaign or mod layer)")?;
    Ok((l, path, rule))
}

/// Write a new tmap part (to [`destination`]); the view reloads afterwards.
fn write_part(cx: &mut Cx, part: Part, bytes: Vec<u8>, map_local: bool) -> Result<PathBuf, String> {
    let index = cx
        .assets
        .as_ref()
        .and_then(|a| a.tiles.as_ref())
        .ok_or("No texture set loaded")?
        .set
        .index;
    let (_, path, _) = destination(cx, part, index, map_local)?;
    cx.s.fs
        .write(&path, &bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    cx.s.push_event(crate::session::ConfigEvent::Saved {
        file: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path: path.clone(),
        deleted: false,
    });
    Ok(path)
}

/// The target layer's override file of `part`, if it exists.
fn target_override(cx: &Cx, part: Part, index: u32) -> Option<PathBuf> {
    crate::tiles::layer_tmap(&*cx.s.fs, &cx.s.layers.layers[cx.s.target()], part, index)
}

/// Remove the target layer's override of `part` (the layer below applies; the view reloads).
pub fn revert_part(cx: &mut Cx, part: Part) -> Result<String, String> {
    let index = cx
        .assets
        .as_ref()
        .and_then(|a| a.tiles.as_ref())
        .ok_or("No texture set loaded")?
        .set
        .index;
    let path =
        target_override(cx, part, index).ok_or("the target layer has no override of this file")?;
    let lower = lower_tmap(&*cx.s.fs, &cx.s.layers, cx.s.target(), part, index);
    if part == Part::A && lower.is_none() {
        return Err("no tmapa below the target layer".into());
    }
    cx.s.fs
        .remove(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    cx.s.push_event(crate::session::ConfigEvent::Saved {
        file: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path: path.clone(),
        deleted: true,
    });
    Ok(match lower {
        Some((l, p)) => format!(
            "removed {}; below: {} ({})",
            path.display(),
            cx.s.layers.layers[l].kind.badge(),
            p.display()
        ),
        None => format!("removed {}; no tmapb below", path.display()),
    })
}

fn read_path(cx: &Cx, p: &str) -> Result<Vec<u8>, String> {
    cx.s.fs
        .read(Path::new(p))
        .map_err(|e| e.to_string())
        .and_then(|b| b.ok_or_else(|| format!("{p}: file not found")))
}

fn is_dat(p: &str) -> bool {
    p.to_ascii_lowercase().ends_with(".dat")
}

/// Import both parts of the pack (those that have a filelist).
fn pack_import(cx: &mut Cx, dir: &Path, dither: bool, map_local: bool) -> Result<String, String> {
    let (index, pal, cur) = {
        let t = cx
            .assets
            .as_ref()
            .and_then(|a| a.tiles.as_ref())
            .ok_or("No texture set loaded")?;
        (
            t.set.index,
            t.palette.clone(),
            [
                t.set.atlas_indexed(Part::A).to_vec(),
                t.set.atlas_indexed(Part::B).to_vec(),
            ],
        )
    };
    let mut done = vec![];
    for (k, part) in [Part::A, Part::B].into_iter().enumerate() {
        if cx
            .s
            .fs
            .read(&dir.join(crate::tiles::filelist_name(part, index)))
            .ok()
            .flatten()
            .is_none()
        {
            continue;
        }
        let bytes = import_pack(&*cx.s.fs, dir, part, index, &cur[k], &pal, dither)?;
        if bytes != cur[k] {
            done.push(
                write_part(cx, part, bytes, map_local)?
                    .display()
                    .to_string(),
            );
        }
    }
    if done.is_empty() {
        return Ok("pack imported: nothing changed".into());
    }
    Ok(format!("pack imported, wrote {}", done.join(", ")))
}

pub fn show(ui: &mut Ui, cx: &mut Cx, st: &mut TileState) {
    let ctx = ui.ctx().clone();
    let frame = (cx.time * 10.0) as usize;
    if st.anim_gen != cx.s.generation {
        sync_anim(cx);
        st.anim_gen = cx.s.generation;
    }
    let Some(host_index) = cx
        .assets
        .as_ref()
        .and_then(|a| a.tiles.as_ref())
        .map(|t| t.set.index)
    else {
        ui.label("No texture set loaded");
        return;
    };
    let want = st.set.unwrap_or(host_index);
    let key = (want, cx.s.target(), st.epoch);
    if st.view != Some(key) {
        st.view = Some(key);
        st.sets = available_sets(&*cx.s.fs, &cx.s.layers);
        if let Err(e) = load_view(cx, st, want) {
            st.message = Some((e, true));
        }
    }
    let Some(tiles) = cx.assets.as_ref().and_then(|a| a.tiles.as_ref()) else {
        return;
    };
    let set_index = tiles.set.index;
    let pal = tiles.palette.clone();
    let msg_before = st.message.clone();
    ui.horizontal_wrapped(|ui| {
        ui.label("Texture set");
        let mut pick = set_index;
        egui::ComboBox::from_id_salt("tileset-set")
            .selected_text(format!("{pick:03}"))
            .width(70.0)
            .show_ui(ui, |ui| {
                for n in st.sets.clone() {
                    ui.selectable_value(&mut pick, n, format!("{n:03}"));
                }
            });
        if pick != set_index {
            st.set = Some(pick);
        }
        ui.selectable_value(&mut st.part, Part::A, "tmapa");
        ui.selectable_value(&mut st.part, Part::B, "tmapb");
        ui.checkbox(&mut st.show_sheet, "Sheet");
        ui.add(egui::Slider::new(&mut st.zoom, 1.0..=4.0).text("zoom"));
    });
    ui.horizontal_wrapped(|ui| {
        let k = if st.part == Part::A { 0 } else { 1 };
        ui.label("Shown from");
        match &st.sources[k] {
            Some((l, p)) => {
                let kind = cx.s.layers.layers[*l].kind.clone();
                badge(ui, &kind.badge(), layer_color(&kind)).on_hover_text(p.display().to_string());
                ui.label(weak(p.display().to_string()));
            }
            None => {
                ui.label(weak("No file in any layer (tiles are blank)"));
            }
        }
        ui.separator();
        match destination(cx, st.part, set_index, st.map_local) {
            Ok((l, p, rule)) => {
                let kind = cx.s.layers.layers[l].kind.clone();
                ui.label("Imports write");
                badge(ui, &kind.badge(), layer_color(&kind));
                ui.label(weak(p.display().to_string()));
                if rule || st.map_local {
                    ui.checkbox(&mut st.map_local, "Map-local").on_hover_text(
                        "The campaign already has its own texture files, so imports \
                            go to the campaign (every level of the campaign sees them); \
                            tick to write a map-local mapNNNNN.tmapXNNN.dat instead",
                    );
                }
                if let Some((sl, _)) = &st.sources[k] {
                    if *sl > l {
                        let sk = cx.s.layers.layers[*sl].kind.badge();
                        ui.label(
                            RichText::new(format!(
                                "{sk} overrides this file: the game shows that one, not what \
                            is written here"
                            ))
                            .small()
                            .color(Color32::from_rgb(230, 90, 80)),
                        );
                    }
                }
            }
            Err(e) => {
                ui.label(
                    RichText::new(e)
                        .small()
                        .color(Color32::from_rgb(220, 180, 80)),
                );
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("File");
        ui.add(egui::TextEdit::singleline(&mut st.path).desired_width(300.0))
            .on_hover_text("A .dat path is a raw texture file, anything else PNG");
        ui.checkbox(&mut st.dither, "Dither");
        let part_name = if st.part == Part::A { "tmapa" } else { "tmapb" };
        if ui
            .button(format!("Export {part_name}"))
            .on_hover_text("The whole file as PNG (or DAT when the path ends with .dat)")
            .clicked()
        {
            let r = cx
                .assets
                .as_ref()
                .and_then(|a| a.tiles.as_ref())
                .ok_or("no set".to_string())
                .and_then(|t| {
                    if is_dat(&st.path) {
                        Ok(t.set.atlas_indexed(st.part).to_vec())
                    } else {
                        atlas_png(&t.set, st.part, &pal)
                    }
                });
            st.message = Some(
                match r.and_then(|b| {
                    cx.s.fs
                        .write(Path::new(&st.path), &b)
                        .map_err(|e| e.to_string())
                }) {
                    Ok(()) => (format!("wrote {}", st.path), false),
                    Err(e) => (e, true),
                },
            );
        }
        if ui
            .button(format!("Import {part_name}"))
            .on_hover_text(format!(
                "{ATLAS_W}x{ATLAS_H} PNG (quantised to the game palette) or \
                    a .dat, written to the target layer"
            ))
            .clicked()
        {
            let r = read_path(cx, &st.path).and_then(|b| {
                if is_dat(&st.path) {
                    import_atlas_dat(&b)
                } else {
                    import_atlas_png(&b, &pal, st.dither)
                }
            });
            st.message = Some(
                match r.and_then(|bytes| write_part(cx, st.part, bytes, st.map_local)) {
                    Ok(p) => (format!("wrote {}", p.display()), false),
                    Err(e) => (e, true),
                },
            );
        }
        if ui
            .button("Export tile")
            .on_hover_text("The selected tile as a 32x32 PNG")
            .clicked()
        {
            let r = cx
                .assets
                .as_ref()
                .and_then(|a| a.tiles.as_ref())
                .and_then(|t| tile_rgba(t.set.atlas_indexed(st.part), st.tile, &pal))
                .ok_or("no tile".to_string())
                .and_then(|img| img.to_png().map_err(|e| e.to_string()));
            st.message = Some(
                match r.and_then(|b| {
                    cx.s.fs
                        .write(Path::new(&st.path), &b)
                        .map_err(|e| e.to_string())
                }) {
                    Ok(()) => (
                        format!(
                            "tile {} written to {}",
                            texture_id(st.part, st.tile),
                            st.path
                        ),
                        false,
                    ),
                    Err(e) => (e, true),
                },
            );
        }
        if ui
            .button("Paste image on tile")
            .on_hover_text("Any size; resized to 32x32")
            .clicked()
        {
            let (part, map_local) = (st.part, st.map_local);
            let r = read_path(cx, &st.path);
            let r = r
                .and_then(|png| Rgba::from_png(&png).map_err(|e| e.to_string()))
                .and_then(|img| {
                    let t = cx
                        .assets
                        .as_ref()
                        .and_then(|a| a.tiles.as_ref())
                        .ok_or("no set")?;
                    let mut atlas = t.set.atlas_indexed(part).to_vec();
                    let mut q = Quantizer::new(&pal);
                    paste_tile(&mut atlas, st.tile, &img, &mut q, st.dither)?;
                    write_part(cx, part, atlas, map_local)
                });
            st.message = Some(match r {
                Ok(p) => (
                    format!("tile {} pasted, wrote {}", st.tile, p.display()),
                    false,
                ),
                Err(e) => (e, true),
            });
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Pack folder");
        ui.add(egui::TextEdit::singleline(&mut st.pack_dir).desired_width(300.0));
        let dir = PathBuf::from(&st.pack_dir);
        if ui
            .button("Export pack")
            .on_hover_text(
                "filelist_tmapXNNN.txt + PNGs of both files; an existing filelist's layout is kept",
            )
            .clicked()
        {
            let r = cx
                .assets
                .as_ref()
                .and_then(|a| a.tiles.as_ref())
                .ok_or("no set".to_string())
                .and_then(|t| {
                    let mut n = export_pack(&*cx.s.fs, &dir, &t.set, Part::A, &pal)?;
                    if t.set.has_b {
                        n += export_pack(&*cx.s.fs, &dir, &t.set, Part::B, &pal)?;
                    }
                    Ok(n)
                });
            st.message = Some(match r {
                Ok(n) => (format!("wrote {n} PNG(s) to {}", dir.display()), false),
                Err(e) => (e, true),
            });
            st.pack_seen =
                [Part::A, Part::B].map(|p| pack_fingerprint(&*cx.s.fs, &dir, p, set_index));
        }
        if ui.button("Import pack").clicked() {
            st.message = Some(match pack_import(cx, &dir, st.dither, st.map_local) {
                Ok(m) => (m, false),
                Err(e) => (e, true),
            });
        }
        if ui
            .checkbox(&mut st.live_reload, "Live reload")
            .on_hover_text("Re-import the pack (every second) when its filelist or PNGs change")
            .changed()
        {
            st.pack_seen =
                [Part::A, Part::B].map(|p| pack_fingerprint(&*cx.s.fs, &dir, p, set_index));
        }
        if ui.button("Open folder").clicked() {
            cx.s.push_event(crate::session::ConfigEvent::RequestOpenFile { path: dir.clone() });
        }
        if st.live_reload && cx.time - st.pack_poll >= 1.0 {
            st.pack_poll = cx.time;
            let now = [Part::A, Part::B].map(|p| pack_fingerprint(&*cx.s.fs, &dir, p, set_index));
            if now != st.pack_seen {
                st.pack_seen = now;
                st.message = Some(match pack_import(cx, &dir, st.dither, st.map_local) {
                    Ok(m) => (format!("live reload: {m}"), false),
                    Err(e) => (format!("live reload: {e}"), true),
                });
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        let target = cx.s.target();
        let stale = st
            .ov_cache
            .as_ref()
            .is_none_or(|c| cx.time - c.0 > 2.0 || c.1 != target || c.2 != set_index);
        if stale {
            let v = [Part::A, Part::B]
                .into_iter()
                .filter_map(|p| target_override(cx, p, set_index).map(|x| (p, x)))
                .collect();
            st.ov_cache = Some((cx.time, target, set_index, v));
        }
        let ov = st
            .ov_cache
            .as_ref()
            .map(|c| c.3.clone())
            .unwrap_or_default();
        if ov.is_empty() {
            ui.label(weak("The target layer has no texture override of this set"));
        }
        for (p, path) in &ov {
            let n = if *p == Part::A { "tmapa" } else { "tmapb" };
            if ui
                .button(format!("Revert {n}"))
                .on_hover_text(format!(
                    "Remove {} (the layer below applies)",
                    path.display()
                ))
                .clicked()
            {
                st.message = Some(match revert_part(cx, *p) {
                    Ok(m) => (m, false),
                    Err(e) => (e, true),
                });
            }
        }
        if ov.len() == 2 && ui.button("Revert both").clicked() {
            let r = revert_part(cx, Part::A)
                .and_then(|a| Ok(format!("{a}; {}", revert_part(cx, Part::B)?)));
            st.message = Some(match r {
                Ok(m) => (m, false),
                Err(e) => (e, true),
            });
        }
        if let Some((m, e)) = &st.message {
            ui.label(RichText::new(m).color(if *e {
                Color32::from_rgb(230, 90, 80)
            } else {
                Color32::from_rgb(110, 200, 110)
            }));
        }
    });
    if st.message != msg_before {
        // something was written or removed: reload what the game sees
        st.epoch += 1;
        st.ov_cache = None;
    }
    ui.separator();
    ui.horizontal_top(|ui| {
        // sheet
        egui::ScrollArea::both()
            .id_salt("sheet")
            .max_width(ATLAS_W as f32 * st.zoom + 20.0)
            .auto_shrink([true, false])
            .show(ui, |ui| {
                if st.show_sheet {
                    let part = st.part;
                    let assets = &*cx.assets;
                    // the atlas is 256 x 2176: two textures of half height (GPU side limit 2048)
                    let half = (ATLAS_H / 2) as u32;
                    let halves: Vec<Option<egui::TextureHandle>> = (0..2u32)
                        .map(|k| {
                            cx.tex.get(&ctx, &format!("atlas:{part:?}:{k}"), || {
                                let t = assets.as_ref()?.tiles.as_ref()?;
                                t.set.atlas_rgba(part, &t.palette).crop(
                                    0,
                                    k * half,
                                    ATLAS_W as u32,
                                    half,
                                )
                            })
                        })
                        .collect();
                    if halves.iter().all(|h| h.is_some()) {
                        let size = egui::vec2(ATLAS_W as f32, ATLAS_H as f32) * st.zoom;
                        let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
                        for (k, t) in halves.iter().enumerate() {
                            let t = t.as_ref().unwrap();
                            let hr = egui::Rect::from_min_size(
                                rect.min + egui::vec2(0.0, k as f32 * size.y / 2.0),
                                egui::vec2(size.x, size.y / 2.0),
                            );
                            ui.painter().image(
                                t.id(),
                                hr,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                        }
                        let cell = TILE_DIM as f32 * st.zoom;
                        let sel = egui::Rect::from_min_size(
                            rect.min
                                + egui::vec2(
                                    (st.tile % TILES_PER_ROW) as f32 * cell,
                                    (st.tile / TILES_PER_ROW) as f32 * cell,
                                ),
                            egui::vec2(cell, cell),
                        );
                        ui.painter().rect_stroke(
                            sel,
                            0.0,
                            egui::Stroke::new(2.0, Color32::YELLOW),
                            egui::StrokeKind::Inside,
                        );
                        if let Some(p) = r.hover_pos() {
                            let tx = ((p.x - rect.min.x) / cell) as usize;
                            let ty = ((p.y - rect.min.y) / cell) as usize;
                            let tile = ty * TILES_PER_ROW + tx;
                            if r.clicked() {
                                st.tile = tile;
                            }
                            r.on_hover_text(format!("Texture {}", texture_id(part, tile)));
                        }
                    }
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.set_max_width(ATLAS_W as f32 * st.zoom);
                        ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                        for t in 0..544 {
                            let id = texture_id(st.part, t);
                            let tx = cx.tex.tile(&ctx, cx.assets, id, 0);
                            if image_box(ui, tx.as_ref(), egui::vec2(32.0, 32.0))
                                .on_hover_text(id.to_string())
                                .clicked()
                            {
                                st.tile = t;
                            }
                        }
                    });
                }
            });
        ui.vertical(|ui| {
            let id = texture_id(st.part, st.tile);
            ui.heading(format!("Texture {id}"));
            let t = cx.tex.tile(&ctx, cx.assets, id, 0);
            image_box(ui, t.as_ref(), egui::vec2(128.0, 128.0));
            ui.separator();
            anim_editor(ui, cx, st, frame);
        });
    });
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
}

fn anim_editor(ui: &mut Ui, cx: &mut Cx, st: &mut TileState, frame: usize) {
    let ctx = ui.ctx().clone();
    let file = "textureanim.toml";
    let slots = cx.s.names("texture");
    ui.label(RichText::new("Animated textures (textureanim.toml)").strong());
    let mut name = st
        .anim_slot
        .and_then(|s| slots.iter().find(|(i, _)| *i == s))
        .map(|(_, n)| n.clone())
        .unwrap_or_default();
    if super::common::picker(ui, "animslot", &mut name, &slots, 220.0, |_, _, _| None) {
        st.anim_slot = slots.iter().find(|(_, n)| *n == name).map(|(i, _)| *i);
    }
    let Some(slot) = st.anim_slot else {
        ui.label(weak("Pick an animated texture"));
        return;
    };
    let path = vec![format!("texture{slot}"), "frames".to_string()];
    let Some(t) = cx.s.toml(file) else { return };
    let cur = t.at_target(&path);
    let layer = cur.as_ref().map(|p| p.layer);
    let mut frames: Vec<i64> = cur
        .and_then(|p| p.value.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|v| v.as_integer().unwrap_or(0))
        .collect();
    frames.resize(8, 0);
    if let Some(l) = layer {
        let k = cx.s.layers.layers[l].kind.clone();
        super::common::badge(ui, &k.badge(), super::common::layer_color(&k));
    }
    // playback
    let f = frames[frame % 8].clamp(0, 1543) as u16;
    let tx = cx.tex.tile(&ctx, cx.assets, f, 0);
    image_box(ui, tx.as_ref(), egui::vec2(96.0, 96.0))
        .on_hover_text(format!("Frame {} = {f}", frame % 8));
    let mut changed = false;
    egui::Grid::new("animframes").show(ui, |ui| {
        for (i, v) in frames.iter_mut().enumerate() {
            let t = cx.tex.tile(&ctx, cx.assets, (*v).clamp(0, 1543) as u16, 0);
            image_box(ui, t.as_ref(), egui::vec2(28.0, 28.0));
            changed |= ui.add(egui::DragValue::new(v).range(0..=1543)).changed();
            if ui
                .small_button("< sel")
                .on_hover_text("Use the selected tile")
                .clicked()
            {
                *v = texture_id(st.part, st.tile) as i64;
                changed = true;
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
    if changed {
        let v = toml::Value::Array(frames.iter().map(|x| toml::Value::Integer(*x)).collect());
        if cx.s.toml(file).is_some_and(|t| t.set(&path, v)) {
            cx.s.touched(file);
        }
    }
}
