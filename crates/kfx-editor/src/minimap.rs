//! Top-down minimap: one pixel per slab coloured by kind and owner, the camera's view on the
//! ground, click to move the camera.
use crate::assets::slab_color;
use crate::camera::Camera;
use eframe::egui::{self, Color32, ColorImage};
use glam::{Vec2, Vec3};
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_prefab::i18n::trd;

/// The minimap image (W x H pixels).
pub fn image(doc: &MapDoc, gd: &GameData) -> ColorImage {
    let d = doc.dims;
    let (w, h) = (d.w as usize, d.h as usize);
    let mut px = Vec::with_capacity(w * h);
    for i in 0..w * h {
        let c = slab_color(gd, doc.kinds[i], doc.owners[i]);
        px.push(Color32::from_rgb(c[0], c[1], c[2]));
    }
    ColorImage::new([w, h], px)
}

/// Ground points (subtile units, y = 0 plane) seen at the four corners of the viewport, in
/// order top-left, top-right, bottom-right, bottom-left. Corners whose ray misses the plane
/// are clamped to the far side of the camera target.
pub fn frustum(cam: &Camera, vp: Vec2) -> [Vec2; 4] {
    let corners = [Vec2::ZERO, Vec2::new(vp.x, 0.0), vp, Vec2::new(0.0, vp.y)];
    corners.map(|p| {
        let r = cam.ray(p, vp);
        match r.hit_plane_y(0.0) {
            Some(h) => Vec2::new(h.x, h.z),
            None => {
                let f = Vec3::new(r.dir.x, 0.0, r.dir.z).normalize_or_zero() * 2000.0;
                Vec2::new(cam.target.x + f.x, cam.target.z + f.z)
            }
        }
    })
}

/// Frustum outline colour of the 3D view on the minimap.
pub const COLOR_3D: Color32 = Color32::from_rgb(255, 230, 120);
/// Frustum outline colour of the 2D (top-down) view on the minimap.
pub const COLOR_2D: Color32 = Color32::from_rgb(120, 200, 255);

/// Draw the minimap into `ui` (fitting the available width) with the outline of each visible
/// view (`views`: camera, its viewport in pixels, colour); returns a subtile position (x, y)
/// when the user clicked or dragged on it.
pub fn show(
    ui: &mut egui::Ui,
    tex: &egui::TextureHandle,
    map_w: usize,
    map_h: usize,
    views: &[(&Camera, Vec2, Color32)],
) -> Option<Vec2> {
    let avail = ui.available_width().max(60.0);
    let scale = (avail / map_w as f32).min(220.0 / map_h as f32).max(0.2);
    let size = egui::vec2(map_w as f32 * scale, map_h as f32 * scale);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(1.0));
    painter.image(
        tex.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, Color32::from_gray(90)),
        egui::StrokeKind::Outside,
    );
    // subtile -> minimap pixel
    let to_px = |p: Vec2| rect.min + egui::vec2(p.x / 3.0 * scale, p.y / 3.0 * scale);
    for (cam, vp, color) in views {
        let pts: Vec<egui::Pos2> = frustum(cam, *vp).iter().map(|&p| to_px(p)).collect();
        let stroke = egui::Stroke::new(1.5, *color);
        for i in 0..4 {
            painter.line_segment([pts[i], pts[(i + 1) % 4]], stroke);
        }
        painter.circle_filled(to_px(Vec2::new(cam.target.x, cam.target.z)), 2.5, *color);
    }
    let resp = resp.on_hover_text(trd(
        "kfx_editor.click_or_drag_to_move_the_view",
        "Click or drag to move the view",
    ));
    if resp.clicked() || resp.dragged() {
        if let Some(p) = resp.interact_pointer_pos() {
            let rel = (p - rect.min) / scale;
            return Some(Vec2::new(
                (rel.x * 3.0).clamp(0.0, map_w as f32 * 3.0),
                (rel.y * 3.0).clamp(0.0, map_h as f32 * 3.0),
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frustum_contains_target_for_iso_view() {
        let cam = Camera {
            target: Vec3::new(120.0, 0.0, 90.0),
            zoom: 0.5,
            ..Default::default()
        };
        let vp = Vec2::new(1200.0, 800.0);
        let f = frustum(&cam, vp);
        // the target lies inside the quadrilateral (sum of signed cross products all same sign)
        let t = Vec2::new(120.0, 90.0);
        let s: Vec<f32> = (0..4)
            .map(|i| (f[(i + 1) % 4] - f[i]).perp_dot(t - f[i]))
            .collect();
        assert!(
            s.iter().all(|&v| v > 0.0) || s.iter().all(|&v| v < 0.0),
            "{f:?}"
        );
    }
}
