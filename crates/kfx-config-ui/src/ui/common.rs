//! Shared UI pieces: texture cache, provenance badges, searchable pickers.
use crate::assets::{compose_frames, Assets};
use crate::layers::LayerKind;
use egui::{Color32, Rect, RichText, Sense, TextureHandle, TextureOptions, Ui, Vec2};
use kfx_gfx::Rgba;
use std::collections::HashMap;

/// RGBA image to an egui image.
pub fn color_image(img: &Rgba) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], &img.data)
}

/// Textures by key, created on first use. `None` results are remembered too.
#[derive(Default)]
pub struct TexCache {
    map: HashMap<String, Option<TextureHandle>>,
    /// Frame counts of cached animations.
    anims: HashMap<String, usize>,
}

impl TexCache {
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        make: impl FnOnce() -> Option<Rgba>,
    ) -> Option<TextureHandle> {
        if let Some(t) = self.map.get(key) {
            return t.clone();
        }
        let t = make()
            .filter(|i| i.w > 0 && i.h > 0)
            .map(|i| ctx.load_texture(key, color_image(&i), TextureOptions::NEAREST));
        self.map.insert(key.to_string(), t.clone());
        t
    }
    /// An existing texture (no creation).
    pub fn peek(&self, key: &str) -> Option<TextureHandle> {
        self.map.get(key).cloned().flatten()
    }
    /// Replace (or create) a texture with new pixels.
    pub fn put(&mut self, ctx: &egui::Context, key: &str, img: &Rgba) -> TextureHandle {
        match self.map.get_mut(key) {
            Some(Some(t)) => {
                t.set(color_image(img), TextureOptions::NEAREST);
                t.clone()
            }
            _ => {
                let t = ctx.load_texture(key, color_image(img), TextureOptions::NEAREST);
                self.map.insert(key.to_string(), Some(t.clone()));
                t
            }
        }
    }
    pub fn forget_prefix(&mut self, prefix: &str) {
        self.map.retain(|k, _| !k.starts_with(prefix));
        self.anims.retain(|k, _| !k.starts_with(prefix));
    }

    /// Texture of tile `id` (animation frame `frame`).
    pub fn tile(
        &mut self,
        ctx: &egui::Context,
        assets: &mut Option<Assets>,
        id: u16,
        frame: usize,
    ) -> Option<TextureHandle> {
        let f = if (544..1000).contains(&id) {
            frame % 8
        } else {
            0
        };
        self.get(ctx, &format!("tile:{id}:{f}"), || {
            assets.as_mut()?.tiles.as_mut()?.tile(id, f).cloned()
        })
    }

    /// Frame textures of an animation value (composited on a common anchor).
    pub fn anim(
        &mut self,
        ctx: &egui::Context,
        assets: &Option<Assets>,
        value: &str,
        quarter: u8,
    ) -> Vec<TextureHandle> {
        let base = format!("anim:{value}:{quarter}");
        if !self.anims.contains_key(&base) {
            let frames = assets
                .as_ref()
                .map(|a| a.anim_frames(value, quarter))
                .unwrap_or_default();
            let (imgs, _) = compose_frames(&frames);
            for (i, img) in imgs.iter().enumerate() {
                self.map.insert(
                    format!("{base}:{i}"),
                    Some(ctx.load_texture(
                        format!("{base}:{i}"),
                        color_image(img),
                        TextureOptions::NEAREST,
                    )),
                );
            }
            self.anims.insert(base.clone(), imgs.len());
        }
        let n = self.anims[&base];
        (0..n)
            .filter_map(|i| self.map.get(&format!("{base}:{i}")).cloned().flatten())
            .collect()
    }
}

/// Draw a texture scaled to fit `max` (keeping aspect), returning the response.
pub fn image_fit(ui: &mut Ui, tex: &TextureHandle, max: Vec2, sense: Sense) -> egui::Response {
    let s = tex.size_vec2();
    let k = (max.x / s.x).min(max.y / s.y);
    let k = if k >= 1.0 { k.floor().max(1.0) } else { k };
    let (rect, resp) = ui.allocate_exact_size(s * k, sense);
    ui.painter().image(
        tex.id(),
        rect,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    resp
}

/// Draw a texture centred in a fixed box (so lists line up).
pub fn image_box(ui: &mut Ui, tex: Option<&TextureHandle>, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if let Some(t) = tex {
        let s = t.size_vec2();
        let k = (size.x / s.x).min(size.y / s.y);
        let k = if k >= 1.0 { k.floor() } else { k };
        let r = Rect::from_center_size(rect.center(), s * k);
        ui.painter().image(
            t.id(),
            r,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    resp
}

/// Colour of a layer kind.
pub fn layer_color(k: &LayerKind) -> Color32 {
    match k {
        LayerKind::Base => Color32::from_rgb(120, 120, 128),
        LayerKind::Mod(_) => Color32::from_rgb(160, 100, 200),
        LayerKind::Campaign => Color32::from_rgb(70, 130, 220),
        LayerKind::Map(_) => Color32::from_rgb(60, 170, 90),
    }
}

/// A small coloured badge.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(11.0),
        Color32::WHITE,
    );
    let size = galley.size() + egui::vec2(8.0, 2.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 3.0, color);
    ui.painter()
        .galley(rect.min + egui::vec2(4.0, 1.0), galley, Color32::WHITE);
    resp
}

/// Persistent filter text of a picker.
fn filter_text(ui: &Ui, id: egui::Id) -> String {
    ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default()
}

/// A searchable dropdown over `(id, name)` options; writes the chosen name (or the id when the
/// name is empty) into `value`. `icon(name)` may give a thumbnail.
pub fn picker(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    value: &mut String,
    options: &[(i64, String)],
    width: f32,
    mut icon: impl FnMut(&mut Ui, &str, i64) -> Option<TextureHandle>,
) -> bool {
    let id = ui.make_persistent_id(&salt).with("picker");
    let mut changed = false;
    let shown = if value.is_empty() {
        "-".to_string()
    } else {
        value.clone()
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(width)
        .height(320.0)
        .show_ui(ui, |ui| {
            let fid = id.with("filter");
            let mut f = filter_text(ui, fid);
            let r = ui.add(
                egui::TextEdit::singleline(&mut f)
                    .hint_text("search")
                    .desired_width(width.max(160.0)),
            );
            if r.changed() {
                ui.data_mut(|d| d.insert_temp(fid, f.clone()));
            }
            r.request_focus();
            let fl = f.to_ascii_lowercase();
            for (n, name) in options {
                let label = if name.is_empty() {
                    n.to_string()
                } else {
                    format!("{name}  ({n})")
                };
                if !fl.is_empty() && !label.to_ascii_lowercase().contains(&fl) {
                    continue;
                }
                ui.horizontal(|ui| {
                    if let Some(t) = icon(ui, name, *n) {
                        image_box(ui, Some(&t), egui::vec2(24.0, 24.0));
                    }
                    let sel = value.eq_ignore_ascii_case(name) || value.trim() == n.to_string();
                    if ui.selectable_label(sel, label).clicked() {
                        *value = if name.is_empty() {
                            n.to_string()
                        } else {
                            name.clone()
                        };
                        changed = true;
                        ui.data_mut(|d| d.remove::<String>(fid));
                    }
                });
            }
        });
    changed
}

/// Small grey text.
pub fn weak(t: impl Into<String>) -> RichText {
    RichText::new(t.into()).small().color(Color32::GRAY)
}

/// Section header text.
pub fn heading(t: impl Into<String>) -> RichText {
    RichText::new(t.into()).strong().size(15.0)
}
