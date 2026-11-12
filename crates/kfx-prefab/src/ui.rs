//! The prefab library browser (egui, host independent).
//!
//! [`PrefabBrowser::show`] draws the library as a grid of thumbnails with a search box and tag
//! chips on the left, and a details pane on the right: a larger top-down preview in the chosen
//! orientation, the metadata, the requirements check against the current map (names the map's
//! configuration lacks, with the skip / substitute policy), placement options (rotation, flip,
//! mask mode, player slots), the script fragment, import / export / delete, and a "save
//! selection as prefab" dialog.
//!
//! Thumbnails: [`PrefabBrowser::thumbnails`] is an optional host callback (for example the
//! editor's 3D renderer); without it the PNG stored in the prefab is used, else the built-in
//! top-down renderer ([`crate::render`]).
//!
//! The host receives [`BrowserEvent`]s: `BeginPlacing` when the user wants to place a prefab (the
//! host drives the ghost with [`crate::apply::preview`] and calls
//! [`PrefabBrowser::apply_at`] or its own [`crate::apply::apply`] on click), `Applied` once a
//! prefab was applied through the browser, `LibraryChanged` after import / save / delete.
use crate::apply::{
    check_requirements, ApplyOptions, MaskMode, MissingPolicy, OwnerMap, Report, Resolution,
};
use crate::capture::{capture, CaptureOptions, Selection};
use crate::library::{Library, Origin};
use crate::model::{Prefab, ThingClass};
use crate::render;
use crate::transform::{Orientation, Rotation};
use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions, Ui};
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_map::patch::Patch;
use std::collections::HashMap;

/// What the browser tells the host.
#[derive(Clone, Debug)]
pub enum BrowserEvent {
    /// The user wants to place this prefab with these options.
    BeginPlacing {
        prefab: Box<Prefab>,
        options: Box<ApplyOptions>,
    },
    /// A prefab was applied through [`PrefabBrowser::apply_at`].
    Applied { name: String, report: Box<Report> },
    /// Import, save or delete changed the library.
    LibraryChanged,
}

/// The map the browser works against (all optional: without a map, requirements and saving are
/// disabled).
#[derive(Default)]
pub struct BrowserContext<'a> {
    pub gd: Option<&'a GameData>,
    pub doc: Option<&'a MapDoc>,
    /// The current selection of the map, for "save selection as prefab".
    pub selection: Option<Selection>,
}

/// Fields of the "save selection as prefab" dialog.
#[derive(Clone, Debug)]
pub struct SaveDialog {
    pub open: bool,
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: String,
    pub mask_empty: bool,
    pub things: bool,
    pub destination: Origin,
}

impl Default for SaveDialog {
    fn default() -> Self {
        SaveDialog {
            open: false,
            name: String::new(),
            author: String::new(),
            description: String::new(),
            tags: String::new(),
            mask_empty: false,
            things: true,
            destination: Origin::User,
        }
    }
}

type ThumbFn = Box<dyn Fn(&Prefab) -> Option<ColorImage>>;

pub struct PrefabBrowser {
    pub library: Library,
    pub query: String,
    pub tag_filter: Vec<String>,
    pub selected: Option<usize>,
    pub orientation: Orientation,
    pub mask: MaskMode,
    pub owners: OwnerMap,
    pub missing: MissingPolicy,
    pub path_text: String,
    pub message: Option<(bool, String)>,
    pub save_dialog: SaveDialog,
    pub last_report: Option<(String, Report)>,
    /// Size in points of a card's thumbnail.
    pub card_size: f32,
    /// Host thumbnail renderer.
    pub thumbnails: Option<ThumbFn>,
    /// The host shows its own "save selection as prefab" dialog: the button only sets
    /// [`PrefabBrowser::save_requested`].
    pub host_save_dialog: bool,
    /// "Save selection as prefab..." was clicked (with [`PrefabBrowser::host_save_dialog`]).
    pub save_requested: bool,
    events: Vec<BrowserEvent>,
    textures: HashMap<usize, Option<TextureHandle>>,
    detail: Option<(usize, Orientation, TextureHandle)>,
}

impl Default for PrefabBrowser {
    fn default() -> Self {
        PrefabBrowser::new(Library::new())
    }
}

fn to_color_image(img: &render::Image) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], &img.rgba)
}

impl PrefabBrowser {
    pub fn new(library: Library) -> PrefabBrowser {
        PrefabBrowser {
            library,
            query: String::new(),
            tag_filter: vec![],
            selected: None,
            orientation: Orientation::default(),
            mask: MaskMode::Replace,
            owners: OwnerMap::default(),
            missing: MissingPolicy::Skip,
            path_text: String::new(),
            message: None,
            save_dialog: SaveDialog::default(),
            last_report: None,
            card_size: 96.0,
            thumbnails: None,
            host_save_dialog: false,
            save_requested: false,
            events: vec![],
            textures: HashMap::new(),
            detail: None,
        }
    }

    /// Re-read the library folders and forget cached images.
    pub fn rescan(&mut self) {
        self.library.scan();
        self.textures.clear();
        self.detail = None;
        if self
            .selected
            .is_some_and(|i| i >= self.library.entries.len())
        {
            self.selected = None;
        }
    }

    /// Forget the cached thumbnail textures (the host's renderer has new images); the library
    /// is not read again.
    pub fn forget_thumbnails(&mut self) {
        self.textures.clear();
        self.detail = None;
    }

    /// The options for placing the selected prefab as the browser has them set.
    pub fn options(&self) -> ApplyOptions {
        ApplyOptions {
            orientation: self.orientation,
            mask: self.mask,
            owners: self.owners.clone(),
            missing: self.missing,
            ..Default::default()
        }
    }

    /// Select the first prefab whose name contains `name` (case-insensitive). Returns whether one
    /// was found.
    pub fn select_by_name(&mut self, name: &str) -> bool {
        let n = name.to_lowercase();
        self.selected = self
            .library
            .entries
            .iter()
            .position(|e| e.prefab.meta.name.to_lowercase().contains(&n));
        self.selected.is_some()
    }

    /// The selected prefab.
    pub fn selected_prefab(&self) -> Option<&Prefab> {
        self.selected
            .and_then(|i| self.library.entries.get(i))
            .map(|e| &e.prefab)
    }

    /// Apply the selected prefab with the browser's options and record the report (the host
    /// pushes the patch to its undo stack). Emits [`BrowserEvent::Applied`].
    pub fn apply_at(
        &mut self,
        doc: &mut MapDoc,
        gd: &GameData,
        at: (i32, i32),
    ) -> crate::Result<Patch> {
        let p = self
            .selected_prefab()
            .cloned()
            .ok_or_else(|| crate::Error::Invalid("no prefab selected".into()))?;
        let (patch, report) = crate::apply::apply(doc, gd, &p, at, &self.options())?;
        self.notify_applied(&p.meta.name, report);
        Ok(patch)
    }

    /// Tell the browser a prefab was applied by the host (shown in the details pane).
    pub fn notify_applied(&mut self, name: &str, report: Report) {
        self.last_report = Some((name.to_string(), report.clone()));
        self.events.push(BrowserEvent::Applied {
            name: name.to_string(),
            report: Box::new(report),
        });
    }

    /// Events not yet taken (also returned by [`PrefabBrowser::show`]).
    pub fn take_events(&mut self) -> Vec<BrowserEvent> {
        std::mem::take(&mut self.events)
    }

    fn thumb(&mut self, ctx: &egui::Context, i: usize) -> Option<TextureHandle> {
        if let Some(t) = self.textures.get(&i) {
            return t.clone();
        }
        let p = &self.library.entries.get(i)?.prefab;
        let img = self
            .thumbnails
            .as_ref()
            .and_then(|f| f(p))
            .or_else(|| {
                p.thumbnail
                    .as_deref()
                    .and_then(render::Image::from_png)
                    .map(|m| to_color_image(&m))
            })
            .unwrap_or_else(|| to_color_image(&render::thumbnail(p, 96)));
        let t = ctx.load_texture(format!("prefab-thumb-{i}"), img, TextureOptions::NEAREST);
        self.textures.insert(i, Some(t.clone()));
        Some(t)
    }

    fn detail_texture(&mut self, ctx: &egui::Context, i: usize) -> Option<TextureHandle> {
        if let Some((j, o, t)) = &self.detail {
            if *j == i && *o == self.orientation {
                return Some(t.clone());
            }
        }
        let p = self
            .library
            .entries
            .get(i)?
            .prefab
            .transformed(self.orientation);
        let px = (320 / p.w.max(p.h)).clamp(4, 24);
        let img = render::render(&p, px, &self.owners);
        let t = ctx.load_texture(
            format!("prefab-detail-{i}"),
            to_color_image(&img),
            TextureOptions::NEAREST,
        );
        self.detail = Some((i, self.orientation, t.clone()));
        Some(t)
    }

    /// Draw the browser.
    pub fn show(&mut self, ui: &mut Ui, bctx: &BrowserContext) -> Vec<BrowserEvent> {
        let full = ui.available_width();
        ui.horizontal_top(|ui| {
            let left_w = (full * 0.58).max(260.0);
            ui.allocate_ui_with_layout(
                egui::vec2(left_w, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.show_list(ui),
            );
            ui.separator();
            ui.vertical(|ui| self.show_details(ui, bctx));
        });
        self.take_events()
    }

    fn show_list(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("name, tag, author…")
                    .desired_width(180.0),
            );
            if ui
                .small_button("x")
                .on_hover_text("Clear the search and the tags")
                .clicked()
            {
                self.query.clear();
                self.tag_filter.clear();
            }
            ui.label(RichText::new(format!("{} prefabs", self.library.entries.len())).weak());
        });
        ui.horizontal_wrapped(|ui| {
            for (tag, n) in self.library.all_tags().into_iter().take(24) {
                let on = self.tag_filter.contains(&tag);
                if ui.selectable_label(on, format!("{tag} ({n})")).clicked() {
                    if on {
                        self.tag_filter.retain(|t| *t != tag);
                    } else {
                        self.tag_filter.push(tag);
                    }
                }
            }
        });
        ui.separator();
        let hits = self.library.search(&self.query, &self.tag_filter);
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical()
            .id_salt("prefab-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if hits.is_empty() {
                    ui.label(RichText::new("No prefab matches.").weak());
                }
                let mut last_origin = None;
                ui.horizontal_wrapped(|ui| {
                    for &i in &hits {
                        let origin = self.library.entries[i].origin;
                        if last_origin != Some(origin) {
                            last_origin = Some(origin);
                            ui.end_row();
                            ui.label(RichText::new(origin.label()).strong());
                            ui.end_row();
                        }
                        let tex = self.thumb(&ctx, i);
                        let selected = self.selected == Some(i);
                        let name = self.library.entries[i].prefab.meta.name.clone();
                        let size = self.card_size;
                        let resp = ui
                            .allocate_ui_with_layout(
                                egui::vec2(size + 12.0, size + 40.0),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    let fill = if selected {
                                        ui.visuals().selection.bg_fill
                                    } else {
                                        ui.visuals().faint_bg_color
                                    };
                                    egui::Frame::new()
                                        .fill(fill)
                                        .corner_radius(4.0)
                                        .inner_margin(4.0)
                                        .show(ui, |ui| {
                                            ui.set_width(size);
                                            ui.set_height(size + 28.0);
                                            if let Some(t) = &tex {
                                                let ts = t.size_vec2();
                                                let k = (size / ts.x).min(size / ts.y);
                                                let (rect, _) = ui.allocate_exact_size(
                                                    egui::vec2(size, size),
                                                    egui::Sense::hover(),
                                                );
                                                ui.painter().rect_filled(
                                                    rect,
                                                    2.0,
                                                    Color32::from_gray(26),
                                                );
                                                let r = egui::Rect::from_center_size(
                                                    rect.center(),
                                                    ts * k,
                                                );
                                                ui.painter().image(
                                                    t.id(),
                                                    r,
                                                    egui::Rect::from_min_max(
                                                        egui::pos2(0.0, 0.0),
                                                        egui::pos2(1.0, 1.0),
                                                    ),
                                                    Color32::WHITE,
                                                );
                                            }
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(name.as_str()).small(),
                                                )
                                                .truncate(),
                                            );
                                        });
                                },
                            )
                            .response;
                        let resp = resp.interact(egui::Sense::click());
                        if resp.clicked() {
                            self.selected = Some(i);
                        }
                        if resp.double_clicked() {
                            self.begin_placing();
                        }
                        resp.on_hover_text(name);
                    }
                });
            });
    }

    fn begin_placing(&mut self) {
        if let Some(p) = self.selected_prefab().cloned() {
            let o = self.options();
            self.events.push(BrowserEvent::BeginPlacing {
                prefab: Box::new(p),
                options: Box::new(o),
            });
        }
    }

    fn show_details(&mut self, ui: &mut Ui, bctx: &BrowserContext) {
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical()
            .id_salt("prefab-details")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let Some(i) = self.selected.filter(|&i| i < self.library.entries.len()) else {
                    ui.label(RichText::new("Select a prefab.").weak());
                    self.show_library_tools(ui, bctx);
                    return;
                };
                let entry = self.library.entries[i].clone();
                let p = &entry.prefab;
                ui.heading(&p.meta.name);
                let (ow, oh) = self.orientation.size(p.w, p.h);
                ui.label(
                    RichText::new(format!(
                        "{} - {} x {} slabs{}",
                        entry.origin.label(),
                        ow,
                        oh,
                        if entry.read_only { " - read-only" } else { "" }
                    ))
                    .weak(),
                );
                if !p.meta.author.is_empty() {
                    ui.label(format!("by {}", p.meta.author));
                }
                if let Some(t) = self.detail_texture(&ctx, i) {
                    let ts = t.size_vec2();
                    let (rect, _) =
                        ui.allocate_exact_size(ts + egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 3.0, Color32::from_gray(26));
                    ui.painter().image(
                        t.id(),
                        rect.shrink(4.0),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                ui.label(&p.meta.description);
                if !p.meta.tags.is_empty() {
                    ui.label(RichText::new(p.meta.tags.join(", ")).weak().italics());
                }
                ui.label(format!(
                    "{} thing(s), {} action point(s), {} light(s){}",
                    p.things.len(),
                    p.action_points.len(),
                    p.lights.len(),
                    if p.scripts.is_empty() {
                        ""
                    } else {
                        ", script fragment"
                    }
                ));
                ui.separator();
                // placement options
                ui.horizontal(|ui| {
                    ui.label("Rotate");
                    for r in Rotation::ALL {
                        if ui
                            .selectable_label(
                                self.orientation.rotation == r,
                                format!("{}", r.degrees()),
                            )
                            .clicked()
                        {
                            self.orientation.rotation = r;
                        }
                    }
                    ui.checkbox(&mut self.orientation.flip, "Flip");
                });
                ui.horizontal(|ui| {
                    ui.label("Existing slabs");
                    egui::ComboBox::from_id_salt("prefab-mask")
                        .selected_text(match self.mask {
                            MaskMode::Replace => "Replace",
                            MaskMode::SkipEmpty => "Skip empty slabs of the prefab",
                            MaskMode::KeepExisting => "Keep existing structures",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.mask, MaskMode::Replace, "Replace");
                            ui.selectable_value(
                                &mut self.mask,
                                MaskMode::SkipEmpty,
                                "Skip empty slabs of the prefab",
                            );
                            ui.selectable_value(
                                &mut self.mask,
                                MaskMode::KeepExisting,
                                "Keep existing structures",
                            );
                        });
                });
                let slots = slots_used(p);
                if !slots.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label("Players");
                        for s in slots {
                            ui.label(((b'A' + s) as char).to_string());
                            let mut v = self.owners.slots[s as usize] as i32;
                            if ui.add(egui::DragValue::new(&mut v).range(0..=8)).changed() {
                                self.owners.slots[s as usize] = v.clamp(0, 8) as u8;
                                self.detail = None;
                            }
                        }
                    });
                }
                // requirements against the current map
                match bctx.gd {
                    Some(gd) => {
                        let miss = check_requirements(p, gd);
                        if miss.is_empty() {
                            ui.colored_label(
                                Color32::from_rgb(90, 200, 110),
                                "This map defines everything the prefab uses.",
                            );
                        } else {
                            ui.colored_label(
                                Color32::from_rgb(235, 120, 90),
                                format!(
                                    "Not defined in this map's configuration ({}):",
                                    miss.len()
                                ),
                            );
                            for m in &miss {
                                ui.label(format!("  {} {} (x{})", m.kind.label(), m.name, m.count));
                            }
                            ui.horizontal(|ui| {
                                ui.label("Missing names");
                                ui.selectable_value(&mut self.missing, MissingPolicy::Skip, "Skip");
                                ui.selectable_value(
                                    &mut self.missing,
                                    MissingPolicy::Substitute,
                                    "Substitute where possible",
                                );
                            });
                            let r = crate::apply::resolve(p, gd, &self.options());
                            for m in &r.report.missing {
                                if let Resolution::Substituted(w) = &m.resolution {
                                    ui.label(
                                        RichText::new(format!("  {} -> {}", m.name, w)).weak(),
                                    );
                                }
                            }
                        }
                    }
                    None => {
                        ui.label(RichText::new("Open a map to check the requirements.").weak());
                    }
                }
                if !p.scripts.is_empty() {
                    ui.collapsing("Script fragment", |ui| {
                        for (label, text) in [("DK script", &p.scripts.dk), ("Lua", &p.scripts.lua)]
                        {
                            if let Some(t) = text.as_deref().filter(|t| !t.is_empty()) {
                                ui.label(RichText::new(label).strong());
                                ui.label(RichText::new(t).monospace().small());
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui
                        .button("Place on map")
                        .on_hover_text(
                            "Start placing with the ghost (double click a card does the same)",
                        )
                        .clicked()
                    {
                        self.begin_placing();
                    }
                    if !entry.read_only
                        && ui
                            .button("Delete")
                            .on_hover_text("Delete the file of this prefab")
                            .clicked()
                    {
                        match self.library.delete(i) {
                            Ok(()) => {
                                self.selected = None;
                                self.textures.clear();
                                self.message = Some((false, "Deleted.".into()));
                                self.events.push(BrowserEvent::LibraryChanged);
                            }
                            Err(e) => self.message = Some((true, e.to_string())),
                        }
                    }
                });
                if let Some((name, r)) = &self.last_report {
                    ui.separator();
                    ui.label(RichText::new(format!("Last applied: {name}")).strong());
                    ui.label(RichText::new(r.summary()).small());
                }
                ui.separator();
                self.show_library_tools(ui, bctx);
            });
    }

    fn show_library_tools(&mut self, ui: &mut Ui, bctx: &BrowserContext) {
        ui.label(RichText::new("Library").strong());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.path_text)
                    .hint_text("path of a .kfxprefab file")
                    .desired_width(170.0),
            );
            if ui.button("Import").clicked() {
                let path = std::path::PathBuf::from(self.path_text.trim());
                match self.library.import_file(&path, Origin::User) {
                    Ok(i) => {
                        self.textures.clear();
                        self.selected = Some(i);
                        self.message = Some((false, format!("Imported {}.", path.display())));
                        self.events.push(BrowserEvent::LibraryChanged);
                    }
                    Err(e) => self.message = Some((true, e.to_string())),
                }
            }
            if ui
                .add_enabled(self.selected.is_some(), egui::Button::new("Export"))
                .clicked()
            {
                if let Some(i) = self.selected {
                    let path = std::path::PathBuf::from(self.path_text.trim());
                    self.message = Some(match self.library.export(i, &path, false) {
                        Ok(()) => (false, format!("Exported to {}.", path.display())),
                        Err(e) => (true, e.to_string()),
                    });
                }
            }
        });
        ui.horizontal(|ui| {
            let can = bctx.doc.is_some() && bctx.gd.is_some() && bctx.selection.is_some();
            if ui
                .add_enabled(can, egui::Button::new("Save selection as prefab…"))
                .on_disabled_hover_text("Select an area of the map first")
                .clicked()
            {
                if self.host_save_dialog {
                    self.save_requested = true;
                } else {
                    self.save_dialog.open = true;
                }
            }
        });
        if self.save_dialog.open {
            self.show_save_dialog(ui, bctx);
        }
        if let Some((err, m)) = &self.message {
            let c = if *err {
                Color32::from_rgb(235, 120, 90)
            } else {
                Color32::from_rgb(90, 200, 110)
            };
            ui.colored_label(c, m);
        }
        for (path, e) in &self.library.problems {
            ui.label(
                RichText::new(format!("{}: {e}", path.display()))
                    .small()
                    .color(Color32::from_rgb(235, 120, 90)),
            );
        }
    }

    fn show_save_dialog(&mut self, ui: &mut Ui, bctx: &BrowserContext) {
        let d = &mut self.save_dialog;
        let mut save = false;
        let mut cancel = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(RichText::new("Save selection as prefab").strong());
            egui::Grid::new("prefab-save-grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut d.name);
                    ui.end_row();
                    ui.label("Author");
                    ui.text_edit_singleline(&mut d.author);
                    ui.end_row();
                    ui.label("Description");
                    ui.text_edit_multiline(&mut d.description);
                    ui.end_row();
                    ui.label("Tags");
                    ui.add(egui::TextEdit::singleline(&mut d.tags).hint_text("comma separated"));
                    ui.end_row();
                });
            ui.checkbox(&mut d.mask_empty, "Leave plain rock and earth out (mask)");
            ui.checkbox(&mut d.things, "Include things, action points and lights");
            ui.horizontal(|ui| {
                ui.label("Save to");
                ui.selectable_value(&mut d.destination, Origin::User, "User library");
                ui.selectable_value(&mut d.destination, Origin::Campaign, "Campaign library");
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!d.name.trim().is_empty(), egui::Button::new("Save"))
                    .clicked()
                {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.save_dialog.open = false;
        }
        if save {
            let (Some(doc), Some(gd), Some(sel)) = (bctx.doc, bctx.gd, bctx.selection.as_ref())
            else {
                return;
            };
            let d = self.save_dialog.clone();
            let opts = CaptureOptions {
                name: d.name.trim().to_string(),
                author: d.author.clone(),
                description: d.description.clone(),
                tags: d
                    .tags
                    .split(',')
                    .map(|t| t.trim().to_lowercase())
                    .filter(|t| !t.is_empty())
                    .collect(),
                mask_empty: d.mask_empty,
                things: d.things,
                action_points: d.things,
                lights: d.things,
                ..Default::default()
            };
            match capture(doc, gd, sel, &opts).and_then(|mut p| {
                p.thumbnail = Some(render::thumbnail_png(&p, 96));
                self.library.save_new(d.destination, p)
            }) {
                Ok(i) => {
                    self.textures.clear();
                    self.selected = Some(i);
                    self.save_dialog = SaveDialog::default();
                    self.message = Some((false, "Saved.".into()));
                    self.events.push(BrowserEvent::LibraryChanged);
                }
                Err(e) => self.message = Some((true, e.to_string())),
            }
        }
    }
}

/// The player slots (0 = A) a prefab uses.
fn slots_used(p: &Prefab) -> Vec<u8> {
    let mut v: Vec<u8> = vec![];
    let mut add = |o: crate::model::Owner| {
        if let crate::model::Owner::Slot(s) = o {
            if !v.contains(&s) {
                v.push(s);
            }
        }
    };
    for c in p.cells.iter().flatten() {
        add(c.owner);
    }
    for t in &p.things {
        add(t.owner);
    }
    v.sort();
    v
}

/// Names of the thing classes for lists (used by the tests and hosts).
pub fn class_label(c: ThingClass) -> &'static str {
    c.name()
}
