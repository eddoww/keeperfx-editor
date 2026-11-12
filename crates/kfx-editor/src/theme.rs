//! Dark theme with warm gold accents.
use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub const GOLD: Color32 = Color32::from_rgb(214, 170, 72);
pub const GOLD_DIM: Color32 = Color32::from_rgb(120, 92, 40);
pub const BG: Color32 = Color32::from_rgb(18, 16, 14);
pub const PANEL: Color32 = Color32::from_rgb(26, 23, 20);
pub const WIDGET: Color32 = Color32::from_rgb(38, 34, 30);
pub const TEXT: Color32 = Color32::from_rgb(222, 212, 196);
pub const TEXT_DIM: Color32 = Color32::from_rgb(140, 130, 118);

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = Color32::from_rgb(30, 27, 24);
    v.extreme_bg_color = BG;
    v.faint_bg_color = Color32::from_rgb(32, 29, 26);
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = GOLD;
    v.selection.bg_fill = Color32::from_rgb(92, 70, 30);
    v.selection.stroke = Stroke::new(1.0, GOLD);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(70, 60, 45));
    let r = CornerRadius::same(3);
    for (w, fill, stroke) in [
        (
            &mut v.widgets.noninteractive,
            PANEL,
            Color32::from_rgb(56, 50, 42),
        ),
        (
            &mut v.widgets.inactive,
            WIDGET,
            Color32::from_rgb(60, 53, 44),
        ),
        (
            &mut v.widgets.hovered,
            Color32::from_rgb(56, 48, 36),
            GOLD_DIM,
        ),
        (&mut v.widgets.active, Color32::from_rgb(76, 60, 32), GOLD),
        (&mut v.widgets.open, Color32::from_rgb(50, 43, 34), GOLD_DIM),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, stroke);
        w.corner_radius = r;
    }
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(250, 230, 190));
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 3.0);
    });
}
