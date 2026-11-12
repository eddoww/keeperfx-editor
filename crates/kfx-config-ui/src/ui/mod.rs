//! The egui widgets. [`ConfigEditor`] is the whole editor (target layer bar, tabs, every
//! specialised editor); each tab is also usable on its own through its module.
pub mod columns;
pub mod common;
pub mod creature;
pub mod form;
pub mod rules;
pub mod slabset;
pub mod sounds;
pub mod sprites;
pub mod stats;
pub mod tileset;

use crate::assets::Assets;
use crate::session::{ConfigEvent, ConfigSession};
use common::{badge, image_box, layer_color, weak, TexCache};
use egui::{Color32, RichText, Ui};
use form::{file_form, FormState};
use kfx_map::gamedata::GameData;
use std::path::PathBuf;

/// What the widgets need while drawing.
pub struct Cx<'a> {
    pub s: &'a mut ConfigSession,
    pub assets: &'a mut Option<Assets>,
    pub tex: &'a mut TexCache,
    /// Seconds, for animations.
    pub time: f64,
    /// `GameData` with the unsaved edits (see [`ConfigEditor::gamedata`]).
    pub gd: &'a mut GdCache,
    /// The host's sound browser and player ([`ConfigEditor::sound_host`]).
    pub sound: Option<&'a mut (dyn SoundHost + 'static)>,
}

/// Where the host's sound browser is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundPlace<'a> {
    /// The Sounds tab (`sounds.cfg`).
    SoundsTab,
    /// The Sounds page of a creature (`creatrs/<name>.cfg` `[sounds]`).
    Creature { name: &'a str },
}

/// What "Use" in the host's sound browser hands back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundPick {
    /// An alias, an id, a message name or a file name.
    pub value: String,
    /// `id count` of a `sound.dat` range, when the pick is one.
    pub id_count: Option<(u16, usize)>,
}

impl SoundPick {
    /// The text a `[sounds]` value takes: `id count` for a bank range, else the value.
    pub fn cfg_value(&self) -> String {
        match self.id_count {
            Some((id, n)) => format!("{id} {n}"),
            None => self.value.clone(),
        }
    }
}

/// A sound browser with playback, provided by the host (this crate decodes and plays nothing).
pub trait SoundHost {
    /// Draw the browser; returns the entry the user chose with "Use".
    fn browser(&mut self, ui: &mut Ui, place: SoundPlace<'_>) -> Option<SoundPick>;
}

/// Cached effective `GameData` for live previews, rebuilt when the session changed.
pub struct GdCache {
    pub scratch: PathBuf,
    pub data: Option<(u64, Result<GameData, String>)>,
}

impl GdCache {
    pub fn get(&mut self, s: &ConfigSession) -> Result<&GameData, String> {
        if self.data.as_ref().is_none_or(|d| d.0 != s.generation) {
            self.data = Some((s.generation, s.effective_gamedata(&self.scratch)));
        }
        self.data
            .as_ref()
            .unwrap()
            .1
            .as_ref()
            .map_err(|e| e.clone())
    }
}

/// Editor tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Files,
    Generic,
    Rules,
    Creatures,
    Objects,
    TrapsDoors,
    Rooms,
    Magic,
    Terrain,
    Slabset,
    Columns,
    Cubes,
    Tileset,
    Sprites,
    Sounds,
}

impl Tab {
    pub const ALL: [Tab; 15] = [
        Tab::Files,
        Tab::Generic,
        Tab::Rules,
        Tab::Creatures,
        Tab::Objects,
        Tab::TrapsDoors,
        Tab::Rooms,
        Tab::Magic,
        Tab::Terrain,
        Tab::Slabset,
        Tab::Columns,
        Tab::Cubes,
        Tab::Tileset,
        Tab::Sprites,
        Tab::Sounds,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Tab::Files => "Files",
            Tab::Generic => "Any config",
            Tab::Rules => "Rules",
            Tab::Creatures => "Creatures",
            Tab::Objects => "Objects",
            Tab::TrapsDoors => "Traps & doors",
            Tab::Rooms => "Rooms",
            Tab::Magic => "Magic",
            Tab::Terrain => "Terrain",
            Tab::Slabset => "Slabset",
            Tab::Columns => "Columns",
            Tab::Cubes => "Cubes",
            Tab::Tileset => "Tileset",
            Tab::Sprites => "Sprites",
            Tab::Sounds => "Sounds",
        }
    }
}

/// The whole configuration editor.
pub struct ConfigEditor {
    pub session: ConfigSession,
    pub assets: Option<Assets>,
    pub tab: Tab,
    pub tex: TexCache,
    pub gd: GdCache,
    pub generic: FormState,
    pub rules: rules::RulesState,
    pub objects: FormState,
    pub traps: FormState,
    pub rooms: FormState,
    pub magic: FormState,
    pub terrain: FormState,
    pub creature: creature::CreatureState,
    pub slabset: slabset::SlabsetState,
    pub columns: columns::ColumnState,
    pub cubes: columns::CubeState,
    pub tileset: tileset::TileState,
    pub sprites: sprites::SpriteState,
    pub sounds: sounds::SoundState,
    /// Sound browser and playback for the Sounds tab and the creature Sounds page (none: the
    /// pages edit the text only).
    pub sound_host: Option<Box<dyn SoundHost>>,
    /// The creature stats comparison ([`ConfigEditor::creature_stats_window`]).
    pub stats: stats::CreatureStats,
    /// Whether the creature stats window is shown.
    pub stats_open: bool,
    /// Seconds since start (the host may drive it; `show` advances it from egui's clock).
    pub time: f64,
    message: Option<(String, bool)>,
}

impl ConfigEditor {
    /// `scratch`: a private temporary folder for building the live-preview `GameData`.
    pub fn new(session: ConfigSession, assets: Option<Assets>, scratch: PathBuf) -> Self {
        ConfigEditor {
            session,
            assets,
            tab: Tab::Files,
            tex: TexCache::default(),
            gd: GdCache {
                scratch,
                data: None,
            },
            generic: FormState::new("rules.cfg"),
            rules: rules::RulesState::default(),
            objects: FormState::new("objects.cfg"),
            traps: FormState::new("trapdoor.cfg"),
            rooms: FormState::new("terrain.cfg"),
            magic: FormState::new("magic.cfg"),
            terrain: FormState::new("terrain.cfg"),
            creature: creature::CreatureState::default(),
            slabset: slabset::SlabsetState::default(),
            columns: columns::ColumnState::default(),
            cubes: columns::CubeState::default(),
            tileset: tileset::TileState::default(),
            sprites: sprites::SpriteState::default(),
            sounds: sounds::SoundState::default(),
            sound_host: None,
            stats: stats::CreatureStats::default(),
            stats_open: false,
            time: 0.0,
            message: None,
        }
    }

    /// The live-preview `GameData` (base + layers + unsaved edits).
    pub fn gamedata(&mut self) -> Result<&GameData, String> {
        self.gd.get(&self.session)
    }

    /// Draw the editor; returns the events of this frame.
    pub fn show(&mut self, ui: &mut Ui) -> Vec<ConfigEvent> {
        self.time = ui.input(|i| i.time);
        self.top_bar(ui);
        ui.separator();
        let ConfigEditor {
            session,
            assets,
            tex,
            gd,
            time,
            tab,
            generic,
            rules,
            objects,
            traps,
            rooms,
            magic,
            terrain,
            creature,
            slabset: slab_st,
            columns: col_st,
            cubes,
            tileset: tile_st,
            sprites: spr_st,
            sounds: snd_st,
            sound_host,
            ..
        } = &mut *self;
        let mut cx = Cx {
            s: session,
            assets,
            tex,
            time: *time,
            gd,
            sound: sound_host.as_deref_mut(),
        };
        match *tab {
            Tab::Files => files_tab(ui, &mut cx, generic, tab),
            Tab::Generic => {
                ui.horizontal(|ui| {
                    ui.label("File");
                    let files: Vec<String> = cx.s.file_list().into_iter().map(|f| f.key).collect();
                    egui::ComboBox::from_id_salt("generic-file")
                        .selected_text(generic.file.clone())
                        .height(400.0)
                        .show_ui(ui, |ui| {
                            for f in files {
                                if ui.selectable_label(generic.file == f, &f).clicked() {
                                    *generic = FormState::new(&f);
                                }
                            }
                        });
                });
                file_form(ui, &mut cx, generic, &[], None);
            }
            Tab::Rules => rules::show(ui, &mut cx, rules),
            Tab::Creatures => creature::show(ui, &mut cx, creature),
            Tab::Objects => file_form(
                ui,
                &mut cx,
                objects,
                &["object"],
                Some(&mut |ui, cx, sec| thing_preview(ui, cx, "objects.cfg", sec)),
            ),
            Tab::TrapsDoors => file_form(
                ui,
                &mut cx,
                traps,
                &["trap", "door"],
                Some(&mut |ui, cx, sec| thing_preview(ui, cx, "trapdoor.cfg", sec)),
            ),
            Tab::Rooms => file_form(
                ui,
                &mut cx,
                rooms,
                &["room"],
                Some(&mut |ui, cx, sec| thing_preview(ui, cx, "terrain.cfg", sec)),
            ),
            Tab::Magic => file_form(
                ui,
                &mut cx,
                magic,
                &[],
                Some(&mut |ui, cx, sec| thing_preview(ui, cx, "magic.cfg", sec)),
            ),
            Tab::Terrain => file_form(
                ui,
                &mut cx,
                terrain,
                &["slab", "block_health"],
                Some(&mut |ui, cx, sec| slabset::terrain_preview(ui, cx, sec)),
            ),
            Tab::Slabset => slabset::show(ui, &mut cx, slab_st),
            Tab::Columns => columns::show_columns(ui, &mut cx, col_st),
            Tab::Cubes => columns::show_cubes(ui, &mut cx, cubes),
            Tab::Tileset => tileset::show(ui, &mut cx, tile_st),
            Tab::Sprites => sprites::show(ui, &mut cx, spr_st),
            Tab::Sounds => sounds::show(ui, &mut cx, snd_st),
        }
        self.session.take_events()
    }

    /// The creature stats comparison window ([`stats`]), drawn while
    /// [`ConfigEditor::stats_open`]; call every frame, independent of [`ConfigEditor::show`].
    /// Returns the session events (like `show`) and whether the user asked to edit a creature
    /// (the editor is then on the Creatures tab with that creature: make it visible).
    pub fn creature_stats_window(&mut self, ctx: &egui::Context) -> StatsFrame {
        let mut show_editor = false;
        if self.stats_open {
            let mut open = true;
            let mut action = None;
            let ConfigEditor { stats, session, .. } = &mut *self;
            // fits small screens / large UI scales: the table scrolls inside
            let screen = ctx.content_rect();
            let size = egui::vec2(
                1250.0f32.min(screen.width() * 0.95),
                640.0f32.min(screen.height() * 0.85),
            );
            egui::Window::new("Creature stats")
                .id(egui::Id::unique("kfx-config-ui-creature-stats"))
                .open(&mut open)
                .default_size(size)
                .max_size(screen.size())
                .show(ctx, |ui| {
                    action = stats.show(ui, session);
                });
            self.stats_open = open;
            match action {
                Some(stats::StatsAction::Edit(n)) => {
                    self.tab = Tab::Creatures;
                    self.creature.name = n;
                    self.creature.page = creature::Page::Stats;
                    self.creature.anim = None;
                    show_editor = true;
                }
                Some(stats::StatsAction::OpenFile(path)) => self
                    .session
                    .push_event(ConfigEvent::RequestOpenFile { path }),
                None => {}
            }
        }
        StatsFrame {
            events: self.session.take_events(),
            show_editor,
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Target layer:");
            let cur = self.session.target();
            let layers = self.session.layers.layers.clone();
            for (i, l) in layers.iter().enumerate() {
                let base = i == 0;
                let text = if base {
                    "base game files (advanced)".to_string()
                } else {
                    l.kind.badge()
                };
                let enabled = !base || self.session.allow_base;
                let r = ui.add_enabled(
                    enabled,
                    egui::Button::selectable(
                        cur == i,
                        RichText::new(text).color(if cur == i {
                            Color32::WHITE
                        } else {
                            layer_color(&l.kind)
                        }),
                    ),
                );
                let r = r.on_hover_text(
                    l.dir
                        .as_ref()
                        .map(|d| d.display().to_string())
                        .unwrap_or_else(|| "no folder".into()),
                );
                if r.clicked() {
                    if let Err(e) = self.session.set_target(i) {
                        self.message = Some((e, true));
                    } else {
                        self.tex.forget_prefix("iso:");
                    }
                }
            }
            ui.checkbox(
                &mut self.session.allow_base,
                RichText::new("Allow base").small(),
            )
            .on_hover_text("Allow editing the install's fxdata files (advanced)");
            ui.separator();
            let pending = self.session.pending();
            if pending.is_empty() {
                ui.label(weak("No unsaved changes"));
            } else {
                badge(
                    ui,
                    &format!("{} unsaved", pending.len()),
                    Color32::from_rgb(200, 120, 40),
                )
                .on_hover_text(
                    pending
                        .iter()
                        .map(|(k, p, _)| format!("{k} -> {}", p.display()))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                if ui.button("Save").clicked() {
                    match self.session.save() {
                        Ok(v) => self.message = Some((format!("saved {} file(s)", v.len()), false)),
                        Err(e) => self.message = Some((e, true)),
                    }
                }
                if ui.button("Discard").clicked() {
                    self.session.discard_all();
                }
            }
            if let Some((m, err)) = &self.message {
                ui.label(RichText::new(m).color(if *err {
                    Color32::from_rgb(230, 90, 80)
                } else {
                    Color32::from_rgb(110, 200, 110)
                }));
            }
        });
        ui.horizontal_wrapped(|ui| {
            for t in Tab::ALL {
                if ui.selectable_label(self.tab == t, t.label()).clicked() {
                    self.tab = t;
                }
            }
            ui.separator();
            ui.toggle_value(&mut self.stats_open, "Creature stats")
                .on_hover_text("Compare every creature's stats in a window");
        });
    }
}

/// Result of [`ConfigEditor::creature_stats_window`].
#[derive(Debug, Default)]
pub struct StatsFrame {
    pub events: Vec<ConfigEvent>,
    /// The user clicked a creature: the editor shows it, make the editor visible.
    pub show_editor: bool,
}

/// Every config file with the layers that have it.
fn files_tab(ui: &mut Ui, cx: &mut Cx, generic: &mut FormState, tab: &mut Tab) {
    let list = cx.s.file_list();
    let layers = cx.s.layers.layers.clone();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("files")
                .striped(true)
                .num_columns(layers.len() + 2)
                .show(ui, |ui| {
                    ui.label(RichText::new("File").strong());
                    for l in &layers {
                        ui.label(
                            RichText::new(l.kind.badge())
                                .strong()
                                .color(layer_color(&l.kind)),
                        );
                    }
                    ui.label("");
                    ui.end_row();
                    for f in list {
                        ui.label(RichText::new(&f.key).monospace());
                        for (i, l) in layers.iter().enumerate() {
                            if f.present.contains(&i) {
                                let r = badge(ui, "*", layer_color(&l.kind));
                                if let Some(p) = l.existing(&*cx.s.fs, &f.key) {
                                    r.on_hover_text(p.display().to_string());
                                }
                            } else {
                                ui.label(weak("-"));
                            }
                        }
                        ui.horizontal(|ui| {
                            if ui.small_button("Edit").clicked() {
                                *generic = FormState::new(&f.key);
                                *tab = Tab::Generic;
                            }
                            if let Some(p) = layers[cx.s.target()].write_path(&*cx.s.fs, &f.key) {
                                if ui
                                    .small_button("Open")
                                    .on_hover_text(p.display().to_string())
                                    .clicked()
                                {
                                    cx.s.push_event(ConfigEvent::RequestOpenFile { path: p });
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
        });
}

/// Sprite / icon preview of an objects / trapdoor / terrain room / magic section.
pub fn thing_preview(ui: &mut Ui, cx: &mut Cx, file: &str, sec: &str) {
    let ctx = ui.ctx().clone();
    let get = |cx: &mut Cx, k: &str| {
        cx.s.cfg(file)
            .and_then(|c| c.at_target(sec, k))
            .map(|p| p.value)
    };
    let anim = get(cx, "AnimationID").or_else(|| get(cx, "Animation"));
    let symbols = get(cx, "SymbolSprites")
        .or_else(|| get(cx, "MedSym"))
        .or_else(|| get(cx, "PanelTabIcon"));
    let name = get(cx, "Name").unwrap_or_default();
    ui.horizontal(|ui| {
        if let Some(a) = anim.as_deref().filter(|a| a.trim() != "0") {
            let frames = cx.tex.anim(&ctx, cx.assets, a, 0);
            if !frames.is_empty() {
                let i = (cx.time * 8.0) as usize % frames.len();
                image_box(ui, Some(&frames[i]), egui::vec2(96.0, 96.0))
                    .on_hover_text(format!("AnimationID {a}: {} frames", frames.len()));
                ctx.request_repaint_after(std::time::Duration::from_millis(120));
            } else {
                ui.label(weak(format!("animation {a}: no sprite data")));
            }
        }
        if let Some(sy) = &symbols {
            for t in kfx_formats::cfg::split_tokens(sy).into_iter().take(2) {
                let assets = &*cx.assets;
                let tex = cx
                    .tex
                    .get(&ctx, &format!("panel:{t}"), || assets.as_ref()?.panel(&t));
                image_box(ui, tex.as_ref(), egui::vec2(64.0, 64.0))
                    .on_hover_text(format!("Symbol {t}"));
            }
        }
        if !name.is_empty() {
            ui.heading(name);
        }
    });
    ui.separator();
}
