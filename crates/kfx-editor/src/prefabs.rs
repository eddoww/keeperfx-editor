//! Prefabs in the editor (the editor side of `kfx_prefab`): the persisted state of the Prefabs
//! panel, the thumbnail renderer and the clipboard bridge.
//!
//! Thumbnails are drawn by the editor's own terrain renderer (the one of the 3D / 2D view and of
//! File > Export image), looking straight down on a scratch map that holds only the prefab,
//! with the textures of the open map. Things, action points and lights are marked with small
//! dots on top. Without an open map the browser falls back to the PNG stored in the prefab (or
//! `kfx_prefab`'s colour renderer).
use crate::assets::CubeFaces;
use crate::atlas::AtlasData;
use crate::camera::{Camera, PX_PER_UNIT};
use crate::mesh;
use crate::render::{ViewFlags, ViewRenderer};
use crate::terrain::Terrain;
use crate::tools::{ClipItem, Clipboard};
use eframe::egui;
use egui_wgpu::wgpu;
use glam::Vec3;
use kfx_formats::level_bin::MapDims;
use kfx_map::autotile::kinds;
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_prefab::apply::{ApplyOptions, IconKind, OwnerMap};
use kfx_prefab::clip::{ClipBlock, ClipItem as PClipItem, ClipboardLike};
use kfx_prefab::Prefab;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Persisted state of the Prefabs panel (`Settings::prefabs`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PrefabSettings {
    /// The panel is shown.
    pub open: bool,
    /// Docked beside the map (else its own window).
    pub docked: bool,
    /// Width of the docked panel.
    pub dock_w: f32,
    /// Size of a card's thumbnail in points.
    pub card_size: f32,
}

impl Default for PrefabSettings {
    fn default() -> Self {
        PrefabSettings {
            open: false,
            docked: true,
            dock_w: 600.0,
            card_size: 96.0,
        }
    }
}

/// The user library: `prefabs/` in the editor's config folder.
pub fn user_folder() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("prefabs"))
}

impl ClipboardLike for Clipboard {
    fn from_block(b: ClipBlock) -> Self {
        let items = b
            .items
            .into_iter()
            .map(|(u, v, it)| {
                let it = match it {
                    PClipItem::Thing(t) => ClipItem::Thing(t),
                    PClipItem::ActionPoint(a) => ClipItem::ActionPoint(a),
                    PClipItem::Light(l) => ClipItem::Light(l),
                };
                (u, v, it)
            })
            .collect();
        Clipboard {
            w: b.w,
            h: b.h,
            cells: b.cells,
            items,
        }
    }
    fn to_block(&self) -> ClipBlock {
        let items = self
            .items
            .iter()
            .map(|(u, v, it)| {
                let it = match it {
                    ClipItem::Thing(t) => PClipItem::Thing(t.clone()),
                    ClipItem::ActionPoint(a) => PClipItem::ActionPoint(a.clone()),
                    ClipItem::Light(l) => PClipItem::Light(l.clone()),
                };
                (*u, *v, it)
            })
            .collect();
        ClipBlock {
            w: self.w,
            h: self.h,
            cells: self.cells.clone(),
            items,
        }
    }
}

/// FNV-1a of a prefab's file text: the thumbnail cache key.
pub fn prefab_key(p: &Prefab) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in p.to_toml().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Thumbnails rendered so far, shared with the browser's thumbnail callback.
pub type ThumbCache = Arc<Mutex<HashMap<u64, egui::ColorImage>>>;

/// Renders prefabs top-down with the editor's terrain renderer (own GPU targets, so the map
/// views are not disturbed).
pub struct ThumbRenderer {
    view: ViewRenderer,
    atlas_key: usize,
}

fn icon_color(k: IconKind) -> [u8; 4] {
    match k {
        IconKind::Creature => [240, 70, 60, 255],
        IconKind::Object => [240, 210, 80, 255],
        IconKind::Trap => [250, 140, 40, 255],
        IconKind::Door => [200, 160, 110, 255],
        IconKind::EffectGen => [120, 220, 240, 255],
        IconKind::HeroGate => [255, 255, 255, 255],
        IconKind::ActionPoint => [90, 170, 255, 255],
        IconKind::Light => [255, 250, 200, 255],
    }
}

impl ThumbRenderer {
    pub fn new(device: &wgpu::Device) -> ThumbRenderer {
        ThumbRenderer {
            view: ViewRenderer::with_samples(device, 1),
            atlas_key: 0,
        }
    }

    /// The prefab (as it is, no rotation) on a scratch map one slab of earth larger on every
    /// side, at most `max_px` pixels on its long side. `None` when the prefab cannot be laid
    /// out with this configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &Arc<AtlasData>,
        cubes: &CubeFaces,
        gd: &GameData,
        texture: u8,
        prefab: &Prefab,
        owners: &OwnerMap,
        max_px: u32,
    ) -> Option<kfx_gfx::Rgba> {
        let (w, h) = (prefab.w as usize + 2, prefab.h as usize + 2);
        if w > 120 || h > 120 {
            return None;
        }
        let dims = MapDims::new(w as u16, h as u16);
        let mut doc = MapDoc::new(std::path::Path::new(""), 1, dims, kinds::EARTH as u16, gd);
        doc.set_texture_id(texture);
        let opts = ApplyOptions {
            owners: owners.clone(),
            ..Default::default()
        };
        kfx_prefab::apply(&mut doc, gd, prefab, (1, 1), &opts).ok()?;
        let terrain = Terrain::from_doc(&doc);
        let (cw, ch) = mesh::chunk_counts(&terrain);
        let meshes: Vec<mesh::ChunkMesh> = (0..cw * ch)
            .map(|i| mesh::build_chunk(&terrain, cubes, i % cw, i / cw))
            .collect();
        let key = Arc::as_ptr(atlas) as usize;
        if key != self.atlas_key {
            self.view.set_atlas(device, queue, atlas);
            self.atlas_key = key;
        }
        self.view.set_chunks(device, &meshes);
        let pps = (max_px / w.max(h) as u32).clamp(2, 24);
        let (iw, ih) = (w as u32 * pps, h as u32 * pps);
        let (sw, sh) = (dims.stl_w() as f32, dims.stl_h() as f32);
        let cam = Camera {
            target: Vec3::new(sw / 2.0, 0.0, sh / 2.0),
            top_down: true,
            yaw: 0.0,
            perspective: false,
            zoom: (pps as f32 / 3.0) / PX_PER_UNIT,
            ..Default::default()
        };
        self.view.flags = ViewFlags {
            ownership: false,
            grid: false,
            lighting: true,
            terrain: true,
        };
        self.view
            .render(device, queue, &cam, (iw, ih), 0, &[], &[], &[], &[]);
        let (rw, rh, data) = self.view.read_back(device, queue)?;
        let mut img = kfx_gfx::Rgba { w: rw, h: rh, data };
        // things, action points and lights as dots
        let pv = kfx_prefab::preview(prefab, gd, None, (1, 1), &opts);
        let r = (pps as i32 / 4).max(1);
        for ic in &pv.icons {
            let (cx, cy) = ((ic.x * pps as f32) as i32, (ic.y * pps as f32) as i32);
            let c = icon_color(ic.kind);
            for dy in -r..=r {
                for dx in -r..=r {
                    let (x, y) = (cx + dx, cy + dy);
                    if x < 0 || y < 0 || x >= rw as i32 || y >= rh as i32 {
                        continue;
                    }
                    let edge = dx.abs() == r || dy.abs() == r;
                    let o = ((y as u32 * rw + x as u32) * 4) as usize;
                    img.data[o..o + 4].copy_from_slice(if edge { &[0, 0, 0, 255] } else { &c });
                }
            }
        }
        Some(img)
    }
}

pub fn to_color_image(img: &kfx_gfx::Rgba) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], &img.data)
}

/// A script fragment a prefab placement appended to a level script, paired with the map's undo
/// step: while that step is applied the text is in the script, undoing the step takes it out
/// again, redoing it puts it back.
#[derive(Clone, Debug)]
pub struct PairedFragment {
    /// The map (`.slb` path) whose history holds the step.
    pub doc: PathBuf,
    /// Journal index of the placement's undo step, its name and size (to recognise it).
    pub step: usize,
    pub name: String,
    pub changes: usize,
    /// (script tab, text appended at its end).
    pub texts: Vec<(String, String)>,
    /// The texts are in the scripts now.
    pub in_script: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_round_trip() {
        let c = Clipboard {
            w: 2,
            h: 1,
            cells: vec![(10, 0), (11, 5)],
            items: vec![],
        };
        let b = c.to_block();
        assert_eq!(<Clipboard as ClipboardLike>::from_block(b), c);
    }
}
