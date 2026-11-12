//! The egui widget: [`CampaignEditor`].
//!
//! Host independent: the widget draws into any `egui::Ui` and returns [`Event`]s; it never opens
//! a window, a file dialog or a game. Tabs:
//! * Overview: every `[common]` key, the pack's folders, save and package.
//! * Levels: the level list (kind tabs, drag reordering, bonus links, add / remove) and the
//!   per-level form.
//! * Land view: the decoded land view picture with a draggable ensign per level, the zoom
//!   preview frame, the progression path and the bonus level markers.
//! * Texts: the language tables side by side, missing translations highlighted.
//! * Health: the validation issues with their fixes.
use crate::files::{self, SourceEntry};
use crate::landview::{self, MAP_H, MAP_W};
use crate::pack::*;
use crate::texts::{CellState, Texts};
use crate::validate::{validate, Issue, Severity, Target};
use egui::{
    pos2, vec2, Align2, Color32, CursorIcon, FontId, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Ui, Vec2,
};
use kfx_formats::campaign::PackKind;
use kfx_formats::levelinfo::LevelKind;
use kfx_formats::strings::Lang;
use std::collections::HashMap;
use std::path::PathBuf;

/// What the widget tells its host.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The user asks to open the level in the map editor.
    OpenLevel { folder: PathBuf, number: u32 },
    /// The user asks to play the level (`-campaign <campaign> -level <number>`).
    PlayLevel { campaign: String, number: u32 },
    /// The pack changed (cfg in memory, level files on disk, or texts).
    Changed,
    /// The cfg (and texts) were written.
    Saved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Levels,
    LandView,
    Texts,
    Health,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindTab {
    All,
    Single,
    Bonus,
    Extra,
    Multi,
    Free,
}

impl KindTab {
    fn label(self) -> &'static str {
        match self {
            KindTab::All => "All",
            KindTab::Single => "Single",
            KindTab::Bonus => "Bonus",
            KindTab::Extra => "Extra",
            KindTab::Multi => "Multi",
            KindTab::Free => "Free",
        }
    }
    fn matches(self, e: &LevelEntry) -> bool {
        match self {
            KindTab::All => true,
            KindTab::Single => e.kinds.contains(&LevelKind::Single),
            KindTab::Bonus => e.kinds.contains(&LevelKind::Bonus),
            KindTab::Extra => e.kinds.contains(&LevelKind::Extra),
            KindTab::Multi => e.kinds.contains(&LevelKind::Multi),
            KindTab::Free => e.kinds.contains(&LevelKind::Free),
        }
    }
}

const OK: Color32 = Color32::from_rgb(110, 200, 120);
const WARN: Color32 = Color32::from_rgb(235, 170, 70);
const BAD: Color32 = Color32::from_rgb(235, 100, 90);
const DIM: Color32 = Color32::from_gray(150);

/// Where to drop a dragged row: the index in `rects` order whose vertical middle is nearest below
/// the pointer (`rects.len() - 1` when below all).
pub fn drop_index(rects: &[Rect], pointer_y: f32) -> usize {
    let mut idx = 0;
    for (i, r) in rects.iter().enumerate() {
        if pointer_y >= r.center().y {
            idx = i;
        }
    }
    idx
}

/// Picture pixels to screen and back for a canvas rectangle.
pub fn to_screen(canvas: Rect, p: (i32, i32)) -> egui::Pos2 {
    pos2(
        canvas.min.x + p.0 as f32 / MAP_W as f32 * canvas.width(),
        canvas.min.y + p.1 as f32 / MAP_H as f32 * canvas.height(),
    )
}

pub fn to_picture(canvas: Rect, s: egui::Pos2) -> (i32, i32) {
    let x = ((s.x - canvas.min.x) / canvas.width() * MAP_W as f32).round() as i32;
    let y = ((s.y - canvas.min.y) / canvas.height() * MAP_H as f32).round() as i32;
    (x.clamp(1, MAP_W as i32 - 1), y.clamp(1, MAP_H as i32 - 1))
}

#[derive(Clone, Copy, Debug)]
enum ListKind {
    Single,
    Extra,
}

/// The campaign / map pack editor widget.
pub struct CampaignEditor {
    pub pack: Pack,
    pub texts: Texts,
    pub tab: Tab,
    pub kind_tab: KindTab,
    pub selected: Option<u32>,
    pub issues: Vec<Issue>,
    pub status: String,
    /// Land view picture shown in the canvas; `None` = the selected level's own.
    pub view_override: Option<String>,
    /// Dragging an ensign moves its zoom point along while both were equal.
    pub link_zoom: bool,
    pub show_path: bool,
    pub show_frame: bool,
    /// Also delete the level files when removing a level.
    pub remove_files: bool,
    pub package_path: String,
    pub extra_text_ids: Vec<u32>,
    sources: Option<Vec<SourceEntry>>,
    /// The "add existing level" panel is open.
    pub add_open: bool,
    add_filter: String,
    add_kind: LevelKind,
    textures: HashMap<String, Option<TextureHandle>>,
    drag_row: Option<(ListKind, usize)>,
    grab: Vec2,
    /// Where the land view picture was drawn last frame (for hosts and tests that drive the
    /// pointer).
    pub last_canvas: Option<Rect>,
    /// Level list rows of the last frame: (level number, row rectangle).
    pub last_rows: Vec<(u32, Rect)>,
    events: Vec<Event>,
    new_lang: Option<Lang>,
}

impl CampaignEditor {
    pub fn new(pack: Pack) -> CampaignEditor {
        let texts = Texts::load(&pack);
        let issues = validate(&pack);
        let selected = pack.entries().first().map(|e| e.number);
        CampaignEditor {
            pack,
            texts,
            tab: Tab::Overview,
            kind_tab: KindTab::All,
            selected,
            issues,
            status: String::new(),
            view_override: None,
            link_zoom: true,
            show_path: true,
            show_frame: true,
            remove_files: false,
            package_path: String::new(),
            extra_text_ids: vec![],
            sources: None,
            add_open: false,
            add_filter: String::new(),
            add_kind: LevelKind::Single,
            textures: HashMap::new(),
            drag_row: None,
            grab: Vec2::ZERO,
            last_canvas: None,
            last_rows: vec![],
            events: vec![],
            new_lang: None,
        }
    }

    /// Re-run the validation (after any change).
    pub fn revalidate(&mut self) {
        self.issues = validate(&self.pack);
    }

    fn changed(&mut self) {
        self.revalidate();
        self.events.push(Event::Changed);
    }

    /// Save the cfg and the edited texts.
    pub fn save(&mut self) {
        let r = self.pack.save().and_then(|_| self.texts.save().map(|_| ()));
        match r {
            Ok(()) => {
                self.status = format!("saved {}", self.pack.cfg_path.display());
                self.events.push(Event::Saved);
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
        self.revalidate();
    }

    fn counts(&self) -> (usize, usize) {
        (
            self.issues
                .iter()
                .filter(|i| i.severity == Severity::Error)
                .count(),
            self.issues
                .iter()
                .filter(|i| i.severity == Severity::Warning)
                .count(),
        )
    }

    /// Draw the editor. Returns the events produced this frame.
    pub fn show(&mut self, ui: &mut Ui) -> Vec<Event> {
        self.events.clear();
        self.top_bar(ui);
        ui.separator();
        egui::Panel::bottom("kfx-camp-status")
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(if self.status.is_empty() {
                        "ready"
                    } else {
                        &self.status
                    })
                    .small()
                    .color(DIM),
                );
            });
        match self.tab {
            Tab::Overview => self.overview(ui),
            Tab::Levels => self.levels(ui),
            Tab::LandView => self.land_view(ui),
            Tab::Texts => self.texts_tab(ui),
            Tab::Health => self.health(ui),
        }
        std::mem::take(&mut self.events)
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let name = self.pack.common().name.unwrap_or_else(|| self.pack.stem());
            ui.heading(name);
            let kind = match self.pack.kind {
                PackKind::Campaign => "campaign",
                PackKind::Mappack => "map pack",
                PackKind::MultiplayerMappack => "multiplayer map pack",
            };
            ui.label(RichText::new(format!("{kind}  {}", self.pack.stem())).color(DIM));
            let dirty = self.pack.is_dirty() || self.texts.is_dirty();
            if ui
                .add_enabled(
                    dirty,
                    egui::Button::new(if dirty { "Save *" } else { "Save" }),
                )
                .clicked()
            {
                self.save();
            }
            ui.separator();
            let (e, w) = self.counts();
            let health = format!("Health ({e} errors, {w} warnings)");
            for (t, label) in [
                (Tab::Overview, "Overview".to_string()),
                (Tab::Levels, "Levels".to_string()),
                (Tab::LandView, "Land view".to_string()),
                (Tab::Texts, "Texts".to_string()),
                (Tab::Health, health),
            ] {
                if ui.selectable_label(self.tab == t, label).clicked() {
                    self.tab = t;
                }
            }
        });
    }

    // ------------------------------------------------------------------ overview

    fn overview(&mut self, ui: &mut Ui) {
        let mut c = self.pack.common();
        let orig = c.clone();
        let is_campaign = self.pack.is_campaign();
        egui::ScrollArea::vertical()
            .id_salt("kfx-camp-overview")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.columns(2, |cols| {
                    let ui = &mut cols[0];
                    ui.strong("Names");
                    egui::Grid::new("ov-names")
                        .num_columns(2)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("NAME");
                            opt_text(ui, &mut c.name, 300.0);
                            ui.end_row();
                            ui.label("NAME_TEXT_ID");
                            opt_text(ui, &mut c.name_text_id, 120.0);
                            ui.end_row();
                            ui.label("");
                            let id = c
                                .name_text_id
                                .as_deref()
                                .and_then(|t| t.trim().parse::<u32>().ok());
                            let shown = id.and_then(|id| self.texts.gui_text(Lang::English, id));
                            ui.label(
                                RichText::new(
                                    shown
                                        .map(|s| format!("menu text: {s}"))
                                        .unwrap_or_else(|| "no menu text for this ID".into()),
                                )
                                .small()
                                .color(DIM),
                            );
                            ui.end_row();
                        });
                    ui.add_space(8.0);
                    ui.strong("Folders (relative to the install)");
                    egui::Grid::new("ov-folders")
                        .num_columns(3)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            let folder_row =
                                |ui: &mut Ui,
                                 label: &str,
                                 v: &mut Option<String>,
                                 pack: &Pack,
                                 fix: &mut Option<PathBuf>| {
                                    ui.label(label);
                                    opt_text(ui, v, 260.0);
                                    match v.as_deref() {
                                        Some(l) if pack.loc(l).is_dir() => {
                                            ui.colored_label(OK, "exists");
                                        }
                                        Some(l) => {
                                            if ui
                                                .small_button("Create")
                                                .on_hover_text("Create the missing folder")
                                                .clicked()
                                            {
                                                *fix = Some(pack.loc(l));
                                            }
                                        }
                                        None => {
                                            ui.label("");
                                        }
                                    }
                                    ui.end_row();
                                };
                            let mut fix = None;
                            folder_row(
                                ui,
                                "LEVELS_LOCATION",
                                &mut c.levels_location,
                                &self.pack,
                                &mut fix,
                            );
                            folder_row(
                                ui,
                                "LAND_LOCATION",
                                &mut c.land_location,
                                &self.pack,
                                &mut fix,
                            );
                            folder_row(
                                ui,
                                "CREATURES_LOCATION",
                                &mut c.creatures_location,
                                &self.pack,
                                &mut fix,
                            );
                            folder_row(
                                ui,
                                "CONFIGS_LOCATION",
                                &mut c.configs_location,
                                &self.pack,
                                &mut fix,
                            );
                            folder_row(
                                ui,
                                "MEDIA_LOCATION",
                                &mut c.media_location,
                                &self.pack,
                                &mut fix,
                            );
                            if let Some(p) = fix {
                                match std::fs::create_dir_all(&p) {
                                    Ok(()) => self.status = format!("created {}", p.display()),
                                    Err(e) => {
                                        self.status = format!("cannot create {}: {e}", p.display())
                                    }
                                }
                                self.revalidate();
                            }
                        });
                    ui.add_space(8.0);
                    ui.strong("Media");
                    egui::Grid::new("ov-media")
                        .num_columns(2)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            for (label, v) in [
                                ("CREDITS", &mut c.credits),
                                ("INTRO_MOVIE", &mut c.intro_movie),
                                ("OUTRO_MOVIE", &mut c.outro_movie),
                                ("SOUNDTRACK", &mut c.soundtrack),
                            ] {
                                ui.label(label);
                                opt_text(ui, v, 220.0);
                                ui.end_row();
                            }
                            ui.label("HIGH_SCORES");
                            ui.horizontal(|ui| {
                                let (mut n, mut f) =
                                    c.high_scores.clone().unwrap_or((1024, String::new()));
                                let a = ui.add(
                                    egui::DragValue::new(&mut n)
                                        .range(0..=99999)
                                        .clamp_existing_to_range(false),
                                );
                                let b =
                                    ui.add(egui::TextEdit::singleline(&mut f).desired_width(160.0));
                                if a.changed() || b.changed() {
                                    c.high_scores = (!f.is_empty()).then_some((n, f));
                                }
                            });
                            ui.end_row();
                        });

                    let ui = &mut cols[1];
                    ui.strong("Players");
                    egui::Grid::new("ov-players")
                        .num_columns(2)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("HUMAN_PLAYER");
                            combo_opt(ui, "human", &mut c.human_player, &HUMAN_PLAYERS);
                            ui.end_row();
                            ui.label("ASSIGN_CPU_KEEPERS");
                            combo_opt(ui, "cpu", &mut c.assign_cpu_keepers, &["ON", "OFF"]);
                            ui.end_row();
                        });
                    ui.add_space(8.0);
                    if is_campaign || c.land_location.is_some() {
                        ui.strong("Land view");
                        egui::Grid::new("ov-land")
                            .num_columns(2)
                            .spacing([10.0, 6.0])
                            .show(ui, |ui| {
                                ui.label("LAND_MARKERS");
                                combo_opt(ui, "markers", &mut c.land_markers, &LAND_MARKERS);
                                ui.end_row();
                                for (label, v) in [
                                    ("LAND_VIEW_START", &mut c.land_view_start),
                                    ("LAND_VIEW_END", &mut c.land_view_end),
                                ] {
                                    ui.label(label);
                                    ui.horizontal(|ui| {
                                        let (mut a, mut b) = v.clone().unwrap_or_default();
                                        let r1 = ui.add(
                                            egui::TextEdit::singleline(&mut a).desired_width(100.0),
                                        );
                                        let r2 = ui.add(
                                            egui::TextEdit::singleline(&mut b).desired_width(100.0),
                                        );
                                        if r1.changed() || r2.changed() {
                                            *v = (!a.is_empty() || !b.is_empty()).then_some((a, b));
                                        }
                                    });
                                    ui.end_row();
                                }
                                ui.label("LAND_AMBIENT");
                                ui.horizontal(|ui| {
                                    let (mut g, mut b) = c.land_ambient.unwrap_or((189, 190));
                                    let r1 = ui.add(egui::DragValue::new(&mut g));
                                    let r2 = ui.add(egui::DragValue::new(&mut b));
                                    if r1.changed() || r2.changed() {
                                        c.land_ambient = Some((g, b));
                                    }
                                });
                                ui.end_row();
                                ui.label("SHOW_DESCRIPTION");
                                let mut sd = c.show_description.unwrap_or(false);
                                if ui.checkbox(&mut sd, "").changed() {
                                    c.show_description = Some(sd);
                                }
                                ui.end_row();
                                ui.label("DESCRIPTION_GEO");
                                ui.horizontal(|ui| {
                                    let mut g = c.description_geo.unwrap_or([230, 250, 500, 140]);
                                    let mut ch = false;
                                    for v in g.iter_mut() {
                                        ch |= ui.add(egui::DragValue::new(v)).changed();
                                    }
                                    if ch {
                                        c.description_geo = Some(g);
                                    }
                                });
                                ui.end_row();
                            });
                        ui.add_space(8.0);
                    }
                    ui.strong("Package");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.package_path)
                                .hint_text("path of the zip to write")
                                .desired_width(260.0),
                        );
                        if ui
                            .add_enabled(
                                !self.package_path.is_empty(),
                                egui::Button::new("Package as zip"),
                            )
                            .clicked()
                        {
                            let out = PathBuf::from(&self.package_path);
                            match files::package(&self.pack, &out, files::PackageOptions::default())
                            {
                                Ok(r) => {
                                    self.status = format!(
                                        "packaged {} files ({} bytes) into {}",
                                        r.files.len(),
                                        r.bytes,
                                        out.display()
                                    );
                                    if !r.not_packed.is_empty() {
                                        self.status.push_str(&format!(
                                            "; not packed: {}",
                                            r.not_packed
                                                .iter()
                                                .map(|(a, b)| format!("{a} ({b})"))
                                                .collect::<Vec<_>>()
                                                .join(", ")
                                        ));
                                    }
                                }
                                Err(e) => self.status = format!("package failed: {e}"),
                            }
                        }
                    });
                });
            });
        if c != orig {
            self.pack.set_common(&c);
            self.changed();
        }
    }

    // ------------------------------------------------------------------ levels

    fn levels(&mut self, ui: &mut Ui) {
        egui::Panel::left("kfx-camp-levels")
            .resizable(true)
            .default_size(380.0)
            .show(ui, |ui| self.level_list(ui));
        egui::CentralPanel::default().show(ui, |ui| self.level_form(ui));
    }

    fn level_list(&mut self, ui: &mut Ui) {
        let is_campaign = self.pack.is_campaign();
        let tabs: &[KindTab] = if is_campaign {
            &[
                KindTab::All,
                KindTab::Single,
                KindTab::Bonus,
                KindTab::Extra,
            ]
        } else {
            &[
                KindTab::All,
                KindTab::Single,
                KindTab::Multi,
                KindTab::Bonus,
                KindTab::Extra,
                KindTab::Free,
            ]
        };
        let entries = self.pack.entries();
        ui.horizontal(|ui| {
            for &t in tabs {
                let n = entries.iter().filter(|e| t.matches(e)).count();
                if ui
                    .selectable_label(self.kind_tab == t, format!("{} {n}", t.label()))
                    .clicked()
                {
                    self.kind_tab = t;
                }
            }
        });
        ui.separator();
        let slots = self.pack.single_slots();
        let extra = self.pack.extra_levels();
        let issue_count = |n: u32, issues: &[Issue]| {
            issues
                .iter()
                .filter(|i| i.target == Target::Level(n) && i.severity != Severity::Info)
                .count()
        };
        self.last_rows.clear();
        let mut select = None;
        let mut remove: Option<u32> = None;
        let mut rects: Vec<(ListKind, usize, Rect)> = vec![];
        let mut drop: Option<(ListKind, usize, usize)> = None;
        let mut stopped: Option<(ListKind, usize)> = None;
        let issues = self.issues.clone();
        egui::ScrollArea::vertical()
            .id_salt("kfx-camp-level-list")
            .max_height(ui.available_height() - 150.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for e in entries.iter().filter(|e| self.kind_tab.matches(e)) {
                    let (list, idx) = match (e.kind(), e.slot) {
                        (LevelKind::Single, Some(i)) if is_campaign => (Some(ListKind::Single), i),
                        (LevelKind::Extra, _) if is_campaign => (
                            Some(ListKind::Extra),
                            extra.iter().position(|&n| n == e.number).unwrap_or(0),
                        ),
                        _ => (None, 0),
                    };
                    let row = ui.horizontal(|ui| {
                        if let Some(l) = list {
                            let h = ui.allocate_response(vec2(16.0, 20.0), Sense::drag());
                            ui.painter().text(
                                h.rect.center(),
                                Align2::CENTER_CENTER,
                                "::",
                                FontId::monospace(14.0),
                                if h.hovered() { Color32::WHITE } else { DIM },
                            );
                            if h.hovered() || h.dragged() {
                                ui.ctx().set_cursor_icon(CursorIcon::Grab);
                            }
                            if h.drag_started() {
                                self.drag_row = Some((l, idx));
                            }
                            if h.drag_stopped() {
                                // the drop target is computed once every row of this frame is known
                                if let Some((dl, from)) = self.drag_row.take() {
                                    stopped = Some((dl, from));
                                }
                            }
                        } else {
                            ui.add_space(20.0);
                        }
                        let kind_label = e
                            .kinds
                            .iter()
                            .map(|k| match k {
                                LevelKind::Single => "S",
                                LevelKind::Bonus => "B",
                                LevelKind::Extra => "X",
                                LevelKind::Multi => "M",
                                LevelKind::Free => "F",
                            })
                            .collect::<Vec<_>>()
                            .join("");
                        let num = match (e.kind(), e.slot) {
                            (LevelKind::Single, Some(i)) => format!("{:>2}. ", i + 1),
                            _ => String::new(),
                        };
                        let name = if e.display_name().is_empty() {
                            "(unnamed)".to_string()
                        } else {
                            e.display_name()
                        };
                        let bonus = if e.kind() == LevelKind::Single {
                            slots
                                .iter()
                                .find(|s| s.level == e.number && s.bonus > 0)
                                .map(|s| format!("  + bonus {}", s.bonus))
                                .unwrap_or_default()
                        } else if e.kind() == LevelKind::Bonus {
                            e.slot
                                .and_then(|i| slots.get(i))
                                .map(|s| format!("  of level {}", s.level))
                                .unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let bad = issue_count(e.number, &issues);
                        let label = format!("{num}[{kind_label}] {:>5}  {name}{bonus}", e.number);
                        let sel = self.selected == Some(e.number);
                        let mut text = RichText::new(label).monospace();
                        if !e.exists {
                            text = text.color(BAD);
                        }
                        if ui.selectable_label(sel, text).clicked() {
                            select = Some(e.number);
                        }
                        if bad > 0 {
                            ui.colored_label(WARN, format!("! {bad}"));
                        }
                    });
                    self.last_rows.push((e.number, row.response.rect));
                    if let Some(l) = list {
                        rects.push((l, idx, row.response.rect));
                    }
                }
            });
        // draw the insertion marker while dragging
        if let (Some((dl, _)), Some(p)) = (self.drag_row, ui.ctx().pointer_interact_pos()) {
            let rs: Vec<Rect> = rects
                .iter()
                .filter(|(k, _, _)| std::mem::discriminant(k) == std::mem::discriminant(&dl))
                .map(|(_, _, r)| *r)
                .collect();
            if !rs.is_empty() {
                let r = rs[drop_index(&rs, p.y)];
                ui.painter().line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(2.0, Color32::from_rgb(120, 180, 255)),
                );
            }
        }
        if let (Some((dl, from)), Some(p)) = (
            stopped,
            ui.ctx()
                .pointer_interact_pos()
                .or(ui.ctx().input(|i| i.pointer.latest_pos())),
        ) {
            let rs: Vec<Rect> = rects
                .iter()
                .filter(|(k, _, _)| std::mem::discriminant(k) == std::mem::discriminant(&dl))
                .map(|(_, _, r)| *r)
                .collect();
            if !rs.is_empty() {
                drop = Some((dl, from, drop_index(&rs, p.y)));
            }
        }
        if let Some((l, from, to)) = drop {
            let r = match l {
                ListKind::Single => self.pack.move_single(from, to),
                ListKind::Extra => self.pack.move_extra(from, to),
            };
            match r {
                Ok(()) => self.changed(),
                Err(e) => self.status = e.to_string(),
            }
        }
        if let Some(n) = select {
            self.selected = Some(n);
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Add existing level…").clicked() {
                self.add_open = !self.add_open;
            }
            if self.add_open && self.sources.is_none() {
                self.sources = Some(files::install_levels(&self.pack.install));
            }
            if let Some(n) = self.selected {
                if ui
                    .button("Remove")
                    .on_hover_text("Take the level out of the pack")
                    .clicked()
                {
                    remove = Some(n);
                }
                ui.checkbox(&mut self.remove_files, "Delete its files");
            }
        });
        if let Some(n) = remove {
            match self.pack.remove_level(n, self.remove_files) {
                Ok(v) => {
                    self.status = format!(
                        "removed level(s) {}",
                        v.iter()
                            .map(|n| n.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    self.selected = self.pack.entries().first().map(|e| e.number);
                    self.changed();
                }
                Err(e) => self.status = e.to_string(),
            }
        }
        if self.add_open {
            self.add_panel(ui);
        }
    }

    fn add_panel(&mut self, ui: &mut Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label("Add as");
                let kinds: &[LevelKind] = if self.pack.is_campaign() {
                    &[LevelKind::Single, LevelKind::Bonus, LevelKind::Extra]
                } else {
                    &[
                        LevelKind::Free,
                        LevelKind::Single,
                        LevelKind::Multi,
                        LevelKind::Bonus,
                        LevelKind::Extra,
                    ]
                };
                for &k in kinds {
                    ui.selectable_value(&mut self.add_kind, k, k.name());
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.add_filter)
                        .hint_text("filter")
                        .desired_width(120.0),
                );
            });
            let mut chosen = None;
            let sources = self.sources.clone().unwrap_or_default();
            let f = self.add_filter.to_ascii_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("kfx-camp-add-list")
                .max_height(160.0)
                .show(ui, |ui| {
                    for s in sources
                        .iter()
                        .filter(|s| f.is_empty() || s.label.to_ascii_lowercase().contains(&f))
                        .take(300)
                    {
                        if ui.selectable_label(false, &s.label).clicked() {
                            chosen = Some(s.clone());
                        }
                    }
                });
            if let Some(s) = chosen {
                let req = AddLevel::new(s.source.clone(), self.add_kind);
                match self.pack.add_level(&req) {
                    Ok(a) => {
                        self.status = format!(
                            "added {} as level {} ({} files copied)",
                            s.label,
                            a.number,
                            a.copied.len()
                        );
                        self.selected = Some(a.number);
                        self.changed();
                    }
                    Err(e) => self.status = format!("cannot add: {e}"),
                }
            }
        });
    }

    fn level_form(&mut self, ui: &mut Ui) {
        let Some(n) = self.selected else {
            ui.label("No level selected.");
            return;
        };
        let Some(entry) = self.pack.entries().into_iter().find(|e| e.number == n) else {
            ui.label("No level selected.");
            return;
        };
        let mut info = entry.info.clone();
        let orig = info.clone();
        let is_campaign = self.pack.is_campaign();
        let levels_dir = self.pack.levels_dir();
        let land_dir = self.pack.land_dir();
        ui.horizontal(|ui| {
            ui.heading(format!("Level {n}"));
            ui.label(
                RichText::new(
                    entry
                        .kinds
                        .iter()
                        .map(|k| k.name())
                        .collect::<Vec<_>>()
                        .join(" "),
                )
                .color(DIM),
            );
            if !entry.exists {
                ui.colored_label(BAD, "Map file missing");
            }
        });
        ui.horizontal(|ui| {
            if let Some(d) = &levels_dir {
                if ui
                    .add_enabled(entry.exists, egui::Button::new("Open in editor"))
                    .clicked()
                {
                    self.events.push(Event::OpenLevel {
                        folder: d.clone(),
                        number: n,
                    });
                }
            }
            if ui
                .add_enabled(entry.exists, egui::Button::new("Play"))
                .clicked()
            {
                self.events.push(Event::PlayLevel {
                    campaign: self.pack.stem(),
                    number: n,
                });
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("kfx-camp-level-form")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("level-form")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("NAME_TEXT");
                        opt_text(ui, &mut info.name_text, 280.0);
                        ui.end_row();
                        ui.label("NAME_ID");
                        ui.horizontal(|ui| {
                            opt_text(ui, &mut info.name_id, 90.0);
                            let id = info.name_id.as_deref().and_then(|t| t.parse::<u32>().ok());
                            if let Some(id) = id {
                                let t = if self.texts.langs.is_empty() {
                                    None
                                } else {
                                    self.texts.effective(0, id)
                                };
                                ui.label(
                                    RichText::new(t.unwrap_or_else(|| "(no text)".into()))
                                        .small()
                                        .color(DIM),
                                );
                            }
                        });
                        ui.end_row();
                        if is_campaign {
                            ui.label("ENSIGN_POS");
                            pair_edit(
                                ui,
                                &mut info.ensign_pos,
                                (MAP_W as i32 / 2, MAP_H as i32 / 2),
                                1..=MAP_W as i32 - 1,
                                1..=MAP_H as i32 - 1,
                            );
                            ui.end_row();
                            ui.label("ENSIGN_ZOOM");
                            pair_edit(
                                ui,
                                &mut info.ensign_zoom,
                                (MAP_W as i32 / 2, MAP_H as i32 / 2),
                                1..=MAP_W as i32 - 1,
                                1..=MAP_H as i32 - 1,
                            );
                            ui.end_row();
                        }
                        ui.label("PLAYERS");
                        let mut pl = info.players.unwrap_or(1);
                        if ui
                            .add(
                                egui::DragValue::new(&mut pl)
                                    .range(1..=8)
                                    .clamp_existing_to_range(false),
                            )
                            .changed()
                        {
                            info.players = Some(pl);
                        }
                        ui.end_row();
                        ui.label("ENSIGN");
                        combo_opt(ui, "ensign", &mut info.ensign, &ENSIGN_KINDS);
                        ui.end_row();
                        ui.label("SPEECH");
                        ui.vertical(|ui| {
                            let (mut a, mut b) = info.speech.clone().unwrap_or_default();
                            let dir = self.pack.default_speech_dir();
                            let mut ch = false;
                            for (what, v) in [("before", &mut a), ("after", &mut b)] {
                                ui.horizontal(|ui| {
                                    ch |= ui
                                        .add(
                                            egui::TextEdit::singleline(v)
                                                .desired_width(160.0)
                                                .hint_text(what),
                                        )
                                        .changed();
                                    if !v.is_empty() {
                                        let name = if v.contains('.') {
                                            v.clone()
                                        } else {
                                            format!("{v}.wav")
                                        };
                                        let found = dir.as_ref().is_some_and(|d| {
                                            kfx_formats::testutil::resolve_ci(d, &name)
                                                .is_some_and(|p| p.is_file())
                                        });
                                        ui.colored_label(
                                            if found { OK } else { WARN },
                                            if found { "found" } else { "missing" },
                                        );
                                    }
                                });
                            }
                            if ch {
                                info.speech = (!a.is_empty() || !b.is_empty()).then_some((a, b));
                            }
                        });
                        ui.end_row();
                        if is_campaign {
                            ui.label("LAND_VIEW");
                            ui.vertical(|ui| {
                                let (mut a, mut b) = info.land_view.clone().unwrap_or_default();
                                let views = land_dir
                                    .as_ref()
                                    .map(|d| landview::available_views(d))
                                    .unwrap_or_default();
                                let windows = land_dir
                                    .as_ref()
                                    .map(|d| landview::available_windows(d))
                                    .unwrap_or_default();
                                let mut ch = false;
                                ui.horizontal(|ui| {
                                    ch |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut a).desired_width(110.0),
                                        )
                                        .changed();
                                    ui.menu_button("Pick", |ui| {
                                        for v in &views {
                                            if ui.button(v).clicked() {
                                                a = v.clone();
                                                ch = true;
                                                ui.close();
                                            }
                                        }
                                    });
                                });
                                ui.horizontal(|ui| {
                                    ch |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut b).desired_width(110.0),
                                        )
                                        .changed();
                                    ui.menu_button("Pick", |ui| {
                                        for v in &windows {
                                            if ui.button(v).clicked() {
                                                b = v.clone();
                                                ch = true;
                                                ui.close();
                                            }
                                        }
                                    });
                                });
                                if ch {
                                    info.land_view =
                                        (!a.is_empty() || !b.is_empty()).then_some((a, b));
                                }
                            });
                            ui.end_row();
                            ui.label("INTRO_DESC_ID");
                            opt_text(ui, &mut info.intro_desc_id, 220.0);
                            ui.end_row();
                        }
                        ui.label("MAPSIZE");
                        ui.vertical(|ui| {
                            pair_edit(ui, &mut info.mapsize, (85, 85), 1..=255, 1..=255);
                            let real = levels_dir
                                .as_ref()
                                .and_then(|d| real_map_size(d, n, info.mapsize));
                            let declared = info.mapsize.unwrap_or((85, 85));
                            match real {
                                Some(r) if r == declared => {
                                    ui.colored_label(
                                        OK,
                                        format!("matches the map file ({}x{})", r.0, r.1),
                                    );
                                }
                                Some(r) => {
                                    ui.colored_label(
                                        BAD,
                                        format!("the map file is {}x{}", r.0, r.1),
                                    );
                                    if ui.small_button("Use the real size").clicked() {
                                        info.mapsize = Some(r);
                                    }
                                }
                                None if entry.exists => {
                                    ui.colored_label(
                                        WARN,
                                        "Map file size is not a square: cannot be checked",
                                    );
                                }
                                None => {}
                            }
                        });
                        ui.end_row();
                        if !is_campaign {
                            ui.label("KIND");
                            ui.horizontal(|ui| {
                                for k in LevelKind::ALL {
                                    let mut on = info.kinds.contains(&k);
                                    if ui.checkbox(&mut on, k.name()).changed() {
                                        if on {
                                            info.kinds.push(k);
                                        } else {
                                            info.kinds.retain(|x| *x != k);
                                        }
                                    }
                                }
                            });
                            ui.end_row();
                        }
                    });
                if is_campaign && entry.kind() == LevelKind::Single {
                    ui.add_space(8.0);
                    self.bonus_link(ui, &entry);
                }
            });
        if info != orig {
            self.pack.set_level_info(n, &info);
            self.changed();
        }
    }

    fn bonus_link(&mut self, ui: &mut Ui, entry: &LevelEntry) {
        let Some(slot) = entry.slot else { return };
        let slots = self.pack.single_slots();
        let cur = slots.get(slot).map(|s| s.bonus).unwrap_or(0);
        let candidates: Vec<(u32, String)> = self
            .pack
            .entries()
            .iter()
            .filter(|e| e.kinds.contains(&LevelKind::Bonus) && e.slot.map_or(true, |s| s == slot))
            .map(|e| (e.number, e.display_name()))
            .collect();
        ui.horizontal(|ui| {
            ui.label("Bonus level revealed after this level:");
            let mut sel = cur;
            egui::ComboBox::from_id_salt("bonus-link")
                .selected_text(if cur == 0 {
                    "none".to_string()
                } else {
                    cur.to_string()
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut sel, 0, "None");
                    for (n, name) in &candidates {
                        ui.selectable_value(&mut sel, *n, format!("{n} {name}"));
                    }
                });
            if sel != cur {
                match self.pack.set_bonus(slot, sel) {
                    Ok(()) => self.changed(),
                    Err(e) => self.status = e.to_string(),
                }
            }
        });
    }

    // ------------------------------------------------------------------ land view

    fn texture(&mut self, ctx: &egui::Context, name: &str) -> Option<TextureHandle> {
        if let Some(t) = self.textures.get(name) {
            return t.clone();
        }
        let tex = self
            .pack
            .land_dir()
            .and_then(|d| landview::load(&d, name).ok())
            .map(|img| {
                let ci =
                    egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.rgba);
                ctx.load_texture(format!("kfx-land-{name}"), ci, TextureOptions::LINEAR)
            });
        self.textures.insert(name.to_string(), tex.clone());
        tex
    }

    /// The land view picture name the canvas shows.
    pub fn canvas_view(&self) -> Option<String> {
        if let Some(v) = &self.view_override {
            return Some(v.clone());
        }
        let own = self
            .selected
            .and_then(|n| self.pack.level_info(n).land_view)
            .map(|(v, _)| v);
        own.or_else(|| self.pack.common().land_view_start.map(|(v, _)| v))
            .or_else(|| {
                self.pack
                    .land_dir()
                    .and_then(|d| landview::available_views(&d).into_iter().next())
            })
    }

    fn land_view(&mut self, ui: &mut Ui) {
        if !self.pack.is_campaign() {
            ui.label(
                "Map packs have no ensigns of their own: levels are picked \
                    from a list. Land view editing applies to campaigns.",
            );
            return;
        }
        let views = self
            .pack
            .land_dir()
            .map(|d| landview::available_views(&d))
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label("Picture:");
            let cur = self.canvas_view().unwrap_or_default();
            egui::ComboBox::from_id_salt("land-picture")
                .selected_text(if self.view_override.is_some() {
                    cur.clone()
                } else {
                    format!("{cur} (level's own)")
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.view_override.is_none(), "Level's own")
                        .clicked()
                    {
                        self.view_override = None;
                    }
                    for v in &views {
                        if ui
                            .selectable_label(self.view_override.as_deref() == Some(v.as_str()), v)
                            .clicked()
                        {
                            self.view_override = Some(v.clone());
                        }
                    }
                });
            ui.checkbox(&mut self.show_path, "Progression path");
            ui.checkbox(&mut self.show_frame, "Zoom frame");
            ui.checkbox(&mut self.link_zoom, "Zoom follows ensign");
        });
        let Some(name) = self.canvas_view() else {
            ui.label("The pack has no land view pictures.");
            return;
        };
        let tex = self.texture(ui.ctx(), &name);
        let mut avail = ui.available_size();
        avail.y -= 26.0;
        let w = avail
            .x
            .min(avail.y * MAP_W as f32 / MAP_H as f32)
            .max(200.0);
        let size = vec2(w, w * MAP_H as f32 / MAP_W as f32);
        let (resp, painter) = ui.allocate_painter(size, Sense::hover());
        let canvas = resp.rect;
        self.last_canvas = Some(canvas);
        match &tex {
            Some(t) => {
                painter.image(
                    t.id(),
                    canvas,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                painter.rect_filled(canvas, 0.0, Color32::from_gray(30));
                painter.text(
                    canvas.center(),
                    Align2::CENTER_CENTER,
                    format!("{name}: picture not found or unreadable"),
                    FontId::proportional(16.0),
                    BAD,
                );
            }
        }
        painter.rect_stroke(
            canvas,
            0.0,
            Stroke::new(1.0, Color32::from_gray(90)),
            StrokeKind::Inside,
        );
        let entries = self.pack.entries();
        let slots = self.pack.single_slots();
        let center = (MAP_W as i32 / 2, MAP_H as i32 / 2);
        // an ensign outside the picture is drawn on the nearest edge (and ringed in red below)
        let outside = |e: &LevelEntry| {
            e.info
                .ensign_pos
                .is_some_and(|(x, y)| x as usize >= MAP_W || y as usize >= MAP_H || x < 1 || y < 1)
        };
        let pos_of = |e: &LevelEntry| {
            let (x, y) = e.info.ensign_pos.unwrap_or(center);
            (x.clamp(1, MAP_W as i32 - 1), y.clamp(1, MAP_H as i32 - 1))
        };
        let scale = canvas.width() / MAP_W as f32;
        // zoom frame of the selected level
        let sel_entry = self
            .selected
            .and_then(|n| entries.iter().find(|e| e.number == n))
            .cloned();
        if let (true, Some(e)) = (self.show_frame, &sel_entry) {
            let z = e.info.ensign_zoom.unwrap_or_else(|| pos_of(e));
            let (x, y, w, h) = landview::view_rect(z);
            let r =
                Rect::from_min_max(to_screen(canvas, (x, y)), to_screen(canvas, (x + w, y + h)));
            let dim = Color32::from_black_alpha(110);
            painter.rect_filled(
                Rect::from_min_max(canvas.min, pos2(canvas.max.x, r.min.y)),
                0.0,
                dim,
            );
            painter.rect_filled(
                Rect::from_min_max(pos2(canvas.min.x, r.max.y), canvas.max),
                0.0,
                dim,
            );
            painter.rect_filled(
                Rect::from_min_max(pos2(canvas.min.x, r.min.y), pos2(r.min.x, r.max.y)),
                0.0,
                dim,
            );
            painter.rect_filled(
                Rect::from_min_max(pos2(r.max.x, r.min.y), pos2(canvas.max.x, r.max.y)),
                0.0,
                dim,
            );
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(2.0, Color32::from_rgb(255, 220, 90)),
                StrokeKind::Inside,
            );
            painter.text(
                r.left_top() + vec2(4.0, 2.0),
                Align2::LEFT_TOP,
                format!("zoom view of level {}", e.number),
                FontId::proportional(12.0),
                Color32::from_rgb(255, 220, 90),
            );
        }
        // progression path through the single levels, bonus links dashed
        if self.show_path {
            let pts: Vec<egui::Pos2> = slots
                .iter()
                .filter_map(|s| entries.iter().find(|e| e.number == s.level))
                .map(|e| to_screen(canvas, pos_of(e)))
                .collect();
            for pair in pts.windows(2) {
                painter.line_segment(
                    [pair[0], pair[1]],
                    Stroke::new(2.5, Color32::from_rgba_unmultiplied(255, 255, 255, 190)),
                );
                let mid = pair[0] + (pair[1] - pair[0]) * 0.55;
                let dir = (pair[1] - pair[0]).normalized();
                let n = vec2(-dir.y, dir.x);
                painter.add(Shape::convex_polygon(
                    vec![
                        mid + dir * 7.0,
                        mid - dir * 5.0 + n * 5.0,
                        mid - dir * 5.0 - n * 5.0,
                    ],
                    Color32::WHITE,
                    Stroke::NONE,
                ));
            }
            for s in slots.iter().filter(|s| s.bonus > 0) {
                let (Some(a), Some(b)) = (
                    entries.iter().find(|e| e.number == s.level),
                    entries.iter().find(|e| e.number == s.bonus),
                ) else {
                    continue;
                };
                let strong = self.selected == Some(a.number) || self.selected == Some(b.number);
                let alpha = if strong { 230 } else { 70 };
                painter.add(Shape::dashed_line(
                    &[to_screen(canvas, pos_of(a)), to_screen(canvas, pos_of(b))],
                    Stroke::new(2.0, Color32::from_rgba_unmultiplied(120, 190, 255, alpha)),
                    8.0,
                    5.0,
                ));
            }
        }
        // ensigns
        let mut moved: Option<(u32, (i32, i32))> = None;
        let mut zoom_moved: Option<(u32, (i32, i32))> = None;
        let mut clicked = None;
        for e in &entries {
            let p = to_screen(canvas, pos_of(e));
            let kind = e.kind();
            let col = match kind {
                LevelKind::Single => Color32::from_rgb(230, 70, 60),
                LevelKind::Bonus => Color32::from_rgb(90, 170, 255),
                LevelKind::Extra => Color32::from_rgb(190, 120, 255),
                _ => Color32::from_rgb(200, 200, 200),
            };
            let h = (34.0 * scale.max(0.5)).max(22.0);
            let sel = self.selected == Some(e.number);
            let hit = Rect::from_min_max(p + vec2(-12.0, -h - 4.0), p + vec2(12.0, 4.0));
            let r = ui.interact(
                hit,
                ui.scope_id().with(("ensign", e.number)),
                Sense::click_and_drag(),
            );
            if r.drag_started() {
                self.grab = ui
                    .input(|i| i.pointer.press_origin())
                    .or(r.interact_pointer_pos())
                    .map_or(Vec2::ZERO, |m| p - m);
            }
            if r.clicked() || r.drag_started() {
                clicked = Some(e.number);
            }
            if r.dragged() {
                if let Some(m) = r.interact_pointer_pos() {
                    moved = Some((e.number, to_picture(canvas, m + self.grab)));
                }
            }
            if r.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            }
            // pole + flag with the bottom centre on the position
            let top = p - vec2(0.0, h);
            painter.line_segment([p, top], Stroke::new(2.5, Color32::BLACK));
            painter.line_segment([p, top], Stroke::new(1.2, Color32::from_gray(220)));
            if kind == LevelKind::Bonus {
                let c = top + vec2(7.0, 6.0);
                painter.add(Shape::convex_polygon(
                    vec![
                        c + vec2(0.0, -8.0),
                        c + vec2(8.0, 0.0),
                        c + vec2(0.0, 8.0),
                        c + vec2(-8.0, 0.0),
                    ],
                    col,
                    Stroke::new(1.0, Color32::BLACK),
                ));
            } else {
                painter.add(Shape::convex_polygon(
                    vec![top, top + vec2(16.0, 6.0), top + vec2(0.0, 13.0)],
                    col,
                    Stroke::new(1.0, Color32::BLACK),
                ));
            }
            painter.circle_filled(p, 3.0, Color32::BLACK);
            let label = match (kind, e.slot) {
                (LevelKind::Single, Some(i)) => format!("{}", i + 1),
                _ => format!("{}", e.number),
            };
            painter.text(
                top + vec2(0.0, -2.0),
                Align2::CENTER_BOTTOM,
                label,
                FontId::proportional(13.0),
                Color32::WHITE,
            );
            if sel {
                painter.rect_stroke(
                    hit,
                    3.0,
                    Stroke::new(2.0, Color32::from_rgb(255, 220, 90)),
                    StrokeKind::Outside,
                );
            }
            if outside(e) {
                painter.rect_stroke(hit, 3.0, Stroke::new(2.0, BAD), StrokeKind::Outside);
                painter.text(
                    p + vec2(0.0, 12.0),
                    Align2::CENTER_TOP,
                    "outside the picture",
                    FontId::proportional(11.0),
                    BAD,
                );
            }
        }
        // the zoom point of the selected level
        if let Some(e) = &sel_entry {
            let z = e.info.ensign_zoom.unwrap_or_else(|| pos_of(e));
            let p = to_screen(canvas, z);
            let hit = Rect::from_center_size(p, vec2(18.0, 18.0));
            let r = ui.interact(hit, ui.scope_id().with(("zoom", e.number)), Sense::drag());
            painter.line_segment(
                [p - vec2(9.0, 0.0), p + vec2(9.0, 0.0)],
                Stroke::new(2.0, Color32::from_rgb(255, 220, 90)),
            );
            painter.line_segment(
                [p - vec2(0.0, 9.0), p + vec2(0.0, 9.0)],
                Stroke::new(2.0, Color32::from_rgb(255, 220, 90)),
            );
            if r.dragged() {
                if let Some(m) = r.interact_pointer_pos() {
                    zoom_moved = Some((e.number, to_picture(canvas, m)));
                }
            }
            if r.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            }
        }
        if let Some(n) = clicked {
            self.selected = Some(n);
        }
        if let Some((n, np)) = moved {
            let mut info = self.pack.level_info(n);
            let old = info.ensign_pos.unwrap_or(center);
            if np != old {
                let zoom_was_linked = info.ensign_zoom.is_none() || info.ensign_zoom == Some(old);
                info.ensign_pos = Some(np);
                if self.link_zoom && zoom_was_linked {
                    info.ensign_zoom = Some(np);
                }
                self.pack.set_level_info(n, &info);
                self.changed();
            }
        }
        if let Some((n, np)) = zoom_moved {
            let mut info = self.pack.level_info(n);
            if info.ensign_zoom != Some(np) {
                info.ensign_zoom = Some(np);
                self.pack.set_level_info(n, &info);
                self.changed();
            }
        }
        if let Some(e) = self
            .selected
            .and_then(|n| self.pack.entries().into_iter().find(|e| e.number == n))
        {
            let p = e.info.ensign_pos;
            let z = e.info.ensign_zoom;
            ui.label(
                RichText::new(format!(
                    "level {}: ENSIGN_POS {}   ENSIGN_ZOOM {}   (picture {name}, {MAP_W}x{MAP_H})",
                    e.number,
                    fmt_pair(p),
                    fmt_pair(z)
                ))
                .small()
                .color(DIM),
            );
        }
    }

    // ------------------------------------------------------------------ texts

    fn texts_tab(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Add language:");
            let present: Vec<Lang> = self.texts.langs.iter().filter_map(|l| l.lang).collect();
            let sel = self.new_lang;
            egui::ComboBox::from_id_salt("new-lang")
                .selected_text(sel.map_or("choose".to_string(), |l| l.code().to_string()))
                .show_ui(ui, |ui| {
                    for l in Lang::ALL.iter().filter(|l| !present.contains(l)) {
                        ui.selectable_value(&mut self.new_lang, Some(*l), l.code());
                    }
                });
            if ui
                .add_enabled(self.new_lang.is_some(), egui::Button::new("Add"))
                .clicked()
            {
                if let Some(l) = self.new_lang.take() {
                    let is_campaign = self.pack.is_campaign();
                    let speech = format!(
                        "{}_{}",
                        self.pack
                            .common()
                            .levels_location
                            .unwrap_or_default()
                            .trim_end_matches('/'),
                        l.lower()
                    );
                    let r = self.texts.add_language(
                        &mut self.pack,
                        l,
                        is_campaign.then_some(speech.as_str()),
                    );
                    match r {
                        Ok(_) => self.changed(),
                        Err(e) => self.status = e.to_string(),
                    }
                }
            }
            if ui
                .add_enabled(self.texts.is_dirty(), egui::Button::new("Save texts"))
                .clicked()
            {
                match self.texts.save() {
                    Ok(w) => self.status = format!("wrote {} string files", w.len()),
                    Err(e) => self.status = format!("cannot save texts: {e}"),
                }
            }
            if ui.button("New ID").clicked() {
                let id = self.texts.free_id(1000);
                self.extra_text_ids.push(id);
            }
        });
        ui.separator();
        if self.texts.langs.is_empty() {
            ui.label("The pack has no [strings] entries.");
            return;
        }
        let mut ids = Texts::used_ids(&self.pack);
        for &id in &self.extra_text_ids {
            if !ids.iter().any(|u| u.id == id) {
                ids.push(crate::texts::UsedId {
                    id,
                    label: "new".into(),
                });
            }
        }
        let nl = self.texts.langs.len();
        let mut edits: Vec<(usize, u32, String)> = vec![];
        let mut result_err = None;
        egui::ScrollArea::both()
            .id_salt("kfx-camp-texts")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("texts-grid")
                    .num_columns(2 + nl)
                    .spacing([8.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("ID");
                        ui.strong("Used by");
                        for l in &self.texts.langs {
                            let mut t = RichText::new(&l.code).strong();
                            if l.shared {
                                t = t.color(DIM);
                            }
                            let r = ui.label(t);
                            if l.shared {
                                r.on_hover_text(format!(
                                    "{} is shared with the game: read only",
                                    l.file
                                ));
                            } else if l.dirty {
                                r.on_hover_text("Edited, not saved");
                            }
                        }
                        ui.end_row();
                        for u in &ids {
                            ui.label(u.id.to_string());
                            ui.label(RichText::new(&u.label).small().color(DIM));
                            for i in 0..nl {
                                let state = self.texts.cell(i, u.id);
                                let shared = self.texts.langs[i].shared;
                                let mut s = self.texts.get(i, u.id).unwrap_or_default();
                                let fill = match state {
                                    CellState::Ok => Color32::TRANSPARENT,
                                    CellState::Missing => {
                                        Color32::from_rgba_unmultiplied(235, 150, 40, 70)
                                    }
                                    CellState::Empty => {
                                        Color32::from_rgba_unmultiplied(220, 60, 60, 60)
                                    }
                                };
                                egui::Frame::new()
                                    .fill(fill)
                                    .inner_margin(2)
                                    .corner_radius(3)
                                    .show(ui, |ui| {
                                        let r = ui
                                            .add_enabled_ui(
                                                !shared && self.texts.langs[i].table.is_some(),
                                                |ui| {
                                                    ui.add_sized(
                                                        vec2(210.0, 22.0),
                                                        egui::TextEdit::singleline(&mut s),
                                                    )
                                                },
                                            )
                                            .inner;
                                        if r.changed() {
                                            edits.push((i, u.id, s.clone()));
                                        }
                                        if state == CellState::Missing {
                                            r.on_hover_text("Missing translation");
                                        }
                                    });
                            }
                            ui.end_row();
                        }
                    });
            });
        for (i, id, text) in edits {
            if let Err(e) = self.texts.set(i, id, &text) {
                result_err = Some(e.to_string());
            }
        }
        if let Some(e) = result_err {
            self.status = e;
        }
    }

    // ------------------------------------------------------------------ health

    fn health(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui.button("Re-check").clicked() {
                self.revalidate();
            }
            let fixable: Vec<_> = self.issues.iter().filter_map(|i| i.fix.clone()).collect();
            if ui
                .add_enabled(
                    !fixable.is_empty(),
                    egui::Button::new(format!("Apply all fixes ({})", fixable.len())),
                )
                .clicked()
            {
                for f in fixable {
                    if let Err(e) = self.pack.apply_fix(&f) {
                        self.status = format!("fix failed: {e}");
                    }
                }
                self.texts = Texts::load(&self.pack);
                self.changed();
            }
        });
        ui.separator();
        if self.issues.is_empty() {
            ui.colored_label(OK, "No problems found.");
        }
        let mut apply = None;
        let mut jump = None;
        egui::ScrollArea::vertical()
            .id_salt("kfx-camp-health")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (k, i) in self.issues.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        let (c, tag) = match i.severity {
                            Severity::Error => (BAD, "ERROR"),
                            Severity::Warning => (WARN, "WARN "),
                            Severity::Info => (DIM, "info "),
                        };
                        ui.label(RichText::new(tag).color(c).monospace());
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(&i.message);
                            });
                            ui.label(
                                RichText::new(format!("rule: {}", i.rule))
                                    .small()
                                    .monospace()
                                    .color(DIM),
                            );
                        });
                        if let Target::Level(n) = i.target {
                            if ui.small_button(format!("Level {n}")).clicked() {
                                jump = Some(n);
                            }
                        }
                        if let Some(f) = &i.fix {
                            if ui.small_button("Fix").clicked() {
                                apply = Some((k, f.clone()));
                            }
                        }
                    });
                    ui.separator();
                }
            });
        if let Some(n) = jump {
            self.selected = Some(n);
            self.tab = Tab::Levels;
        }
        if let Some((_, f)) = apply {
            match self.pack.apply_fix(&f) {
                Ok(()) => {
                    self.texts = Texts::load(&self.pack);
                    self.status = format!("applied: {f:?}");
                    self.changed();
                }
                Err(e) => self.status = format!("fix failed: {e}"),
            }
        }
    }
}

fn fmt_pair(p: Option<(i32, i32)>) -> String {
    p.map_or("(default)".to_string(), |(a, b)| format!("{a} {b}"))
}

/// A text field for an optional value: empty = `None`.
fn opt_text(ui: &mut Ui, v: &mut Option<String>, width: f32) {
    let mut s = v.clone().unwrap_or_default();
    if ui
        .add_sized(vec2(width, 22.0), egui::TextEdit::singleline(&mut s))
        .changed()
    {
        *v = (!s.is_empty()).then_some(s);
    }
}

fn combo_opt(ui: &mut Ui, id: &str, v: &mut Option<String>, options: &[&str]) {
    let text = v.clone().unwrap_or_else(|| "(unset)".into());
    egui::ComboBox::from_id_salt(id)
        .selected_text(text)
        .show_ui(ui, |ui| {
            if ui.selectable_label(v.is_none(), "(unset)").clicked() {
                *v = None;
            }
            for o in options {
                let on = v.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(o));
                if ui.selectable_label(on, *o).clicked() {
                    *v = Some(o.to_string());
                }
            }
        });
}

fn pair_edit(
    ui: &mut Ui,
    v: &mut Option<(i32, i32)>,
    default: (i32, i32),
    rx: std::ops::RangeInclusive<i32>,
    ry: std::ops::RangeInclusive<i32>,
) {
    ui.horizontal(|ui| {
        let mut a = v.unwrap_or(default);
        let r1 = ui.add(
            egui::DragValue::new(&mut a.0)
                .range(rx)
                .clamp_existing_to_range(false),
        );
        let r2 = ui.add(
            egui::DragValue::new(&mut a.1)
                .range(ry)
                .clamp_existing_to_range(false),
        );
        if r1.changed() || r2.changed() {
            *v = Some(a);
        }
        if v.is_none() {
            ui.label(RichText::new("default").small().color(DIM));
        }
    });
}
