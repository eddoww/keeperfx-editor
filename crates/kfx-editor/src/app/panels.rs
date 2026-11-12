//! The thing palette and the right panel (inspector, things list, layers, validation), the
//! inspector's field values and the orientation compass.
use super::things::{class_name, model_name, owner_rgb, PlaceKind, ThingEntry, FIELDS};
use super::*;
use crate::document::Regen;
use crate::sprites::{icon_image, Icon, Visual};
use kfx_formats::level_fx::FxThing;
use kfx_formats::levelinfo::LevelInfo;
use kfx_map::things::{class, thing_class, ItemRef};
use kfx_map::validate::{self, Location, Severity};
use kfx_prefab::i18n::{trd, trf, tx, Tx};

/// Kinds of the things list filter: the value kept in the filter and its label.
const LIST_KINDS: [(&str, Tx); 8] = [
    ("All", tx("kfx_editor.all", "All")),
    ("Creatures", tx("kfx_editor.creatures_f8b1", "Creatures")),
    ("Objects", tx("kfx_editor.objects", "Objects")),
    ("Traps", tx("kfx_editor.traps", "Traps")),
    ("Doors", tx("kfx_editor.doors", "Doors")),
    ("Effects", tx("kfx_editor.effects", "Effects")),
    (
        "Action points",
        tx("kfx_editor.action_points", "Action points"),
    ),
    ("Lights", tx("kfx_editor.lights", "Lights")),
];

/// Object genres to palette groups, in display order.
pub const OBJECT_GROUPS: &[(Tx, &[&str])] = &[
    (tx("kfx_editor.decorations", "Decorations"), &["DECORATION"]),
    (tx("kfx_editor.furniture_7365", "Furniture"), &["FURNITURE"]),
    (
        tx("kfx_editor.treasure_and_gold", "Treasure and gold"),
        &["TREASURE_HOARD", "VALUABLE"],
    ),
    (tx("kfx_editor.spellbooks", "Spellbooks"), &["SPELLBOOK"]),
    (
        tx("kfx_editor.workshop_boxes", "Workshop boxes"),
        &["WORKSHOPBOX"],
    ),
    (tx("kfx_editor.specials", "Specials"), &["SPECIALBOX"]),
    (tx("kfx_editor.food_8329", "Food"), &["FOOD"]),
    (tx("kfx_editor.power_related", "Power related"), &["POWER"]),
    (tx("kfx_editor.hero_gates", "Hero gates"), &["HEROGATE"]),
    (tx("kfx_editor.lairs", "Lairs"), &["LAIR_TOTEM"]),
    (tx("kfx_editor.effects", "Effects"), &["EFFECT"]),
];

/// Texture pack names by `.slx` value (`texture_pack_desc`, src/lvl_script_commands.c:439-457, from
/// `data/script_tables.json`): value 0 is the level's own set, value `n` is `tmapa{n-1}`
/// (`get_conf_parameter_text(texture_pack_desc, game.texture_id + 1)`, src/lua_api_map.c:81).
pub fn texture_pack_names() -> Vec<(u32, String)> {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../../data/script_tables.json"))
            .unwrap_or_default();
    let mut v: Vec<(u32, String)> = vec![];
    if let Some(a) = json
        .get("named_tables")
        .and_then(|t| t.get("texture_pack_desc"))
        .and_then(|t| t.as_array())
    {
        for e in a {
            let (Some(n), Some(val)) = (
                e.get("name").and_then(|x| x.as_str()),
                e.get("value")
                    .and_then(|x| x.as_str())
                    .and_then(|x| x.parse::<u32>().ok()),
            ) else {
                continue;
            };
            if !v.iter().any(|(x, _)| *x == val) {
                v.push((val, n.to_string()));
            }
        }
    }
    v
}

/// Name of texture set (tmapa index) `set`.
pub fn texture_set_name(names: &[(u32, String)], set: u32) -> String {
    names
        .iter()
        .find(|(v, _)| *v == set + 1)
        .map(|(_, n)| n.clone())
        .unwrap_or_else(|| format!("SET_{set}"))
}

pub(super) fn rgba_tex(
    ctx: &egui::Context,
    name: &str,
    img: &kfx_gfx::Rgba,
) -> Option<egui::TextureHandle> {
    (img.w > 0 && img.h > 0)
        .then(|| ctx.load_texture(name, img_to_color(img), egui::TextureOptions::NEAREST))
}

/// Crop transparent borders.
fn trim(img: &kfx_gfx::Rgba) -> kfx_gfx::Rgba {
    let (mut x0, mut y0, mut x1, mut y1) = (img.w, img.h, 0, 0);
    for y in 0..img.h {
        for x in 0..img.w {
            if img.pixel(x, y)[3] > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 {
        return img.clone();
    }
    img.crop(x0, y0, x1 - x0 + 1, y1 - y0 + 1)
        .unwrap_or_else(|| img.clone())
}

/// State of the level properties dialog.
pub struct PropsDialog {
    pub info: LevelInfo,
    pub texture: u8,
    /// Where the info is written: `Some(cfg)` = campaign `[mapNNNNN]`, `None` = `.lof`.
    pub target: Option<PathBuf>,
    pub thumbs: Vec<(u32, String, Option<egui::TextureHandle>)>,
    pub error: Option<String>,
    /// The text the game shows for `NAME_ID` (string tables of the level).
    pub id_text: Option<String>,
    /// The land view / ensign picker over the dialog (`landview.rs`).
    pub land: Option<super::landview::LandPicker>,
}

pub struct ResizeDialog {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub fill: String,
}

pub struct RegenDialog {
    pub what: Regen,
    pub selection: bool,
    pub confirm: bool,
}

impl EditorApp {
    // ---------------------------------------------------------------------- palette

    /// Thing palette entries from the loaded configs (modded / campaign things included).
    pub(super) fn build_thing_palette(&mut self) {
        self.thing_palette.clear();
        let (Some(doc), Some(lib)) = (&self.doc, self.sprites.as_mut()) else {
            return;
        };
        let gd = doc.gd.clone();
        let gui2 = self.assets.as_ref().and_then(|a| a.gui2.as_ref());
        let gui1 = self.assets.as_ref().and_then(|a| a.gui1.as_ref());
        let ctx = self.ctx.clone();
        let mut out = vec![];
        let sprite_icon =
            |lib: &mut crate::sprites::SpriteLib, cls: u8, m: i64| -> Option<kfx_gfx::Rgba> {
                match lib.visual(&gd, cls, m, 1) {
                    Visual::Sprite { anim, .. } => anim
                        .first_key()
                        .and_then(|k| lib.frame(&anim, k))
                        .map(|f| trim(&f.image)),
                    Visual::Marker => Some(icon_image(Icon::Marker)),
                    Visual::Unknown => Some(icon_image(Icon::Unknown)),
                }
            };
        // creatures: portrait (QuerySymbol), else the standing sprite
        for (i, name) in gd.creatures.iter().enumerate() {
            let m = i as i64 + 1;
            let info = lib.creatures.get(i).cloned().unwrap_or_default();
            // QuerySymbol is a gui1-64 button sprite (GBS_creature_portrait_*, sprites.h) or a
            // custom icon name
            let img = info
                .query_symbol
                .as_deref()
                .and_then(|q| lib.icon(q, gui1))
                .or_else(|| sprite_icon(lib, class::CREATURE, m));
            let group = if info.evil {
                trd("kfx_editor.evil", "Evil")
            } else {
                trd("kfx_editor.good", "Good")
            };
            let src = info
                .sources
                .last()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| trd("kfx_editor.no_creature_cfg", "no creature cfg").into());
            out.push(ThingEntry {
                kind: PlaceKind::Creature(m),
                group: group.clone(),
                label: name.clone(),
                tooltip: trf(
                    "kfx_editor.name_creature_m_group_src",
                    "{name} (creature {m}, {group})\n{src}",
                    &[("name", &name), ("m", &m), ("group", &group), ("src", &src)],
                ),
                tex: img.and_then(|i| rgba_tex(&ctx, "crtr", &i)),
            });
        }
        let (crates, books) = (
            super::workshop::crates(&gd),
            super::workshop::spellbooks(&gd),
        );
        for (m, name) in gd.object_names() {
            if m == 0 {
                continue;
            }
            let genre = gd.object_genre(m).unwrap_or("").trim().to_ascii_uppercase();
            let group = OBJECT_GROUPS
                .iter()
                .find(|(_, g)| g.contains(&genre.as_str()))
                .map(|(n, _)| n.text())
                .unwrap_or_else(|| trd("kfx_editor.others", "Others"));
            let mut img = sprite_icon(lib, class::OBJECT, m as i64);
            let mut tooltip = trf(
                "kfx_editor.name_object_m_genre_genre",
                "{name} (object {m}, genre {genre})",
                &[("name", &name), ("m", &m), ("genre", &genre)],
            );
            // workshop box: the trap / door icon over the box (trapdoor.cfg Crate)
            if let Some(c) = crates.get(&m) {
                let cls = if c.door { class::DOOR } else { class::TRAP };
                let icon = c
                    .symbol
                    .as_deref()
                    .and_then(|s| lib.icon(s, gui2))
                    .or_else(|| {
                        if c.door {
                            Some(icon_image(Icon::Door))
                        } else {
                            sprite_icon(lib, cls, c.model as i64)
                        }
                    });
                img = super::workshop::with_crate_icon(img, icon.as_ref());
                tooltip = trf(
                    "kfx_editor.tooltip_workshop_box_of_door_model_name",
                    "{tooltip}\nworkshop box of {door} {model} ({name})",
                    &[
                        ("tooltip", &tooltip),
                        (
                            "door",
                            &(if c.door {
                                trd("kfx_editor.door_4ee9", "door")
                            } else {
                                trd("kfx_editor.trap_1322", "trap")
                            }),
                        ),
                        ("model", &c.model),
                        ("name", &c.name),
                    ],
                );
            }
            if let Some(p) = books.get(&m) {
                tooltip = trf(
                    "kfx_editor.tooltip_spellbook_of_p_magic_cfg",
                    "{tooltip}\nspellbook of {p} (magic.cfg)",
                    &[("tooltip", &tooltip), ("p", &p)],
                );
            }
            out.push(ThingEntry {
                kind: PlaceKind::Object(m as i64),
                group: group.into(),
                label: name.clone(),
                tooltip,
                tex: img.and_then(|i| rgba_tex(&ctx, "obj", &i)),
            });
        }
        for (m, name) in gd.trap_names() {
            if m == 0 {
                continue;
            }
            let sym = gd
                .trapdoor_cfg
                .get(&format!("trap{m}"), "SymbolSprites")
                .map(str::to_string);
            let img = sym
                .as_deref()
                .and_then(|s| lib.icon(s, gui2))
                .or_else(|| sprite_icon(lib, class::TRAP, m as i64));
            out.push(ThingEntry {
                kind: PlaceKind::Trap(m as i64),
                group: trd("kfx_editor.traps", "Traps").into(),
                label: name.clone(),
                tooltip: trf(
                    "kfx_editor.name_trap_m",
                    "{name} (trap {m})",
                    &[("name", &name), ("m", &m)],
                ),
                tex: img.and_then(|i| rgba_tex(&ctx, "trap", &i)),
            });
        }
        for (m, name) in gd.door_names() {
            if m == 0 {
                continue;
            }
            let sym = gd
                .trapdoor_cfg
                .get(&format!("door{m}"), "SymbolSprites")
                .map(str::to_string);
            let img = sym
                .as_deref()
                .and_then(|s| lib.icon(s, gui2))
                .or_else(|| Some(icon_image(Icon::Door)));
            out.push(ThingEntry {
                kind: PlaceKind::Door(m as i64),
                group: trd("kfx_editor.doors", "Doors").into(),
                label: name.clone(),
                tooltip: trf(
                    "kfx_editor.name_door_m_orientation_from_the_walls_around",
                    "{name} (door {m}); orientation from the walls around",
                    &[("name", &name), ("m", &m)],
                ),
                tex: img.and_then(|i| rgba_tex(&ctx, "door", &i)),
            });
        }
        for (m, name) in gd.effect_generator_names() {
            if m == 0 {
                continue;
            }
            out.push(ThingEntry {
                kind: PlaceKind::EffectGen(m as i64),
                group: trd("kfx_editor.effect_generators", "Effect generators").into(),
                label: name.trim_start_matches("EFFECTGENERATOR_").to_string(),
                tooltip: trf(
                    "kfx_editor.name_effect_generator_m",
                    "{name} (effect generator {m})",
                    &[("name", &name), ("m", &m)],
                ),
                tex: rgba_tex(&ctx, "eff", &icon_image(Icon::EffectGen)),
            });
        }
        out.push(ThingEntry {
            kind: PlaceKind::Light,
            group: trd("kfx_editor.lights", "Lights").into(),
            label: trd("kfx_editor.light", "Light").into(),
            tooltip: trd("kfx_editor.static_light", "Static light").into(),
            tex: rgba_tex(&ctx, "light", &icon_image(Icon::Light)),
        });
        out.push(ThingEntry {
            kind: PlaceKind::ActionPoint,
            group: trd("kfx_editor.action_points", "Action points").into(),
            label: trd("kfx_editor.action_point", "Action point").into(),
            tooltip: trd(
                "kfx_editor.action_point_next_free_number",
                "Action point (next free number)",
            )
            .into(),
            tex: rgba_tex(&ctx, "ap", &icon_image(Icon::ActionPoint)),
        });
        self.thing_palette = out;
    }

    /// The thing tabs of the bottom panel.
    pub(super) fn thing_palette_ui(&mut self, ui: &mut egui::Ui) {
        let cat = self.category;
        let matches_cat = |k: &PlaceKind| match cat {
            Category::Creatures => matches!(k, PlaceKind::Creature(_)),
            Category::Objects => matches!(k, PlaceKind::Object(_)),
            Category::Traps => matches!(k, PlaceKind::Trap(_)),
            Category::Doors => matches!(k, PlaceKind::Door(_)),
            Category::Effects => matches!(k, PlaceKind::EffectGen(_)),
            Category::Lights => matches!(k, PlaceKind::Light),
            Category::ActionPoints => matches!(k, PlaceKind::ActionPoint),
            _ => false,
        };
        let q = self.palette_search.trim().to_ascii_lowercase();
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let d = &mut self.place_defaults;
                match cat {
                    Category::Creatures => {
                        ui.label(trd("kfx_editor.level", "Level"));
                        for l in 1..=10 {
                            ui.selectable_value(&mut d.level, l, l.to_string());
                        }
                        ui.separator();
                        ui.label(trd("kfx_editor.gold", "Gold"));
                        ui.add(egui::DragValue::new(&mut d.creature_gold).range(0..=1_000_000_000));
                        ui.label(trd("kfx_editor.health", "Health %"));
                        ui.add(egui::DragValue::new(&mut d.health).range(0..=100));
                    }
                    Category::Doors => {
                        ui.checkbox(&mut d.locked, trd("kfx_editor.locked", "Locked"));
                        ui.label(
                            RichText::new(trd(
                                "kfx_editor.orientation_follows_the_walls_around_the_slab",
                                "Orientation follows the walls around the slab",
                            ))
                            .small()
                            .color(TEXT_DIM),
                        );
                    }
                    Category::Lights => {
                        ui.label(trd("kfx_editor.range", "Range"));
                        ui.add(
                            egui::DragValue::new(&mut d.light_range)
                                .range(0.0..=255.0)
                                .speed(0.1),
                        );
                        ui.label(trd("kfx_editor.intensity", "Intensity"));
                        ui.add(egui::DragValue::new(&mut d.light_intensity).range(0..=255));
                        ui.label(trd("kfx_editor.height", "Height"));
                        ui.add(
                            egui::DragValue::new(&mut d.light_height)
                                .range(0.0..=8.0)
                                .speed(0.1),
                        );
                    }
                    Category::ActionPoints => {
                        ui.label(trd("kfx_editor.range", "Range"));
                        ui.add(
                            egui::DragValue::new(&mut d.ap_range)
                                .range(0.0..=255.0)
                                .speed(0.1),
                        );
                    }
                    Category::Effects => {
                        ui.label(trd("kfx_editor.range", "Range"));
                        ui.add(
                            egui::DragValue::new(&mut d.effect_range)
                                .range(0.0..=255.0)
                                .speed(0.1),
                        );
                    }
                    Category::Objects => {
                        ui.label(trd("kfx_editor.gold_value", "Gold value"));
                        ui.add(egui::DragValue::new(&mut d.gold_value).range(0..=1_000_000_000));
                        ui.label(trd("kfx_editor.box_number", "Box number"));
                        ui.add(egui::DragValue::new(&mut d.custom_box).range(0..=255));
                    }
                    _ => {}
                }
                ui.separator();
                ui.label(trd("kfx_editor.facing", "Facing"));
                ui.add(egui::DragValue::new(&mut d.orientation).range(0..=2047));
                ui.checkbox(
                    &mut self.free_place,
                    trd("kfx_editor.free_sub_position", "Free sub-position"),
                )
                .on_hover_text(trd(
                    "kfx_editor.off_things_snap_to_the_subtile_centre",
                    "Off: things snap to the subtile centre",
                ));
            });
            let entries: Vec<usize> = (0..self.thing_palette.len())
                .filter(|&i| {
                    let e = &self.thing_palette[i];
                    matches_cat(&e.kind)
                        && (q.is_empty()
                            || e.label.to_ascii_lowercase().contains(&q)
                            || e.tooltip.to_ascii_lowercase().contains(&q))
                })
                .collect();
            if entries.is_empty() {
                ui.label(
                    RichText::new(if self.doc.is_some() {
                        trd("kfx_editor.nothing_matches", "Nothing matches")
                    } else {
                        trd(
                            "kfx_editor.open_a_map_to_see_its_things",
                            "Open a map to see its things",
                        )
                    })
                    .color(TEXT_DIM),
                );
                return;
            }
            let mut groups: Vec<String> = vec![];
            for &i in &entries {
                let g = &self.thing_palette[i].group;
                if !groups.contains(g) {
                    groups.push(g.clone());
                }
            }
            let mut chosen = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for g in groups {
                        ui.label(RichText::new(&g).small().color(GOLD_DIM));
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = EVec2::new(4.0, 4.0);
                            for &i in &entries {
                                let e = &self.thing_palette[i];
                                if e.group != g {
                                    continue;
                                }
                                let sel = self.place.as_ref() == Some(&e.kind);
                                let (rect, resp) = ui.allocate_exact_size(
                                    EVec2::new(64.0, 64.0),
                                    egui::Sense::click(),
                                );
                                let painter = ui.painter_at(rect.expand(2.0));
                                painter.rect_filled(rect, 2.0, Color32::from_rgb(12, 11, 10));
                                let img_rect = egui::Rect::from_min_max(
                                    rect.min + EVec2::new(3.0, 3.0),
                                    rect.max - EVec2::new(3.0, 14.0),
                                );
                                if let Some(tex) = &e.tex {
                                    let ts = tex.size_vec2();
                                    let scale = (img_rect.width() / ts.x)
                                        .min(img_rect.height() / ts.y)
                                        .min(2.0);
                                    let r =
                                        egui::Rect::from_center_size(img_rect.center(), ts * scale);
                                    painter.image(
                                        tex.id(),
                                        r,
                                        egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        Color32::WHITE,
                                    );
                                }
                                let label: String = e.label.chars().take(11).collect();
                                painter.text(
                                    egui::pos2(rect.center().x, rect.max.y - 7.0),
                                    egui::Align2::CENTER_CENTER,
                                    label,
                                    egui::FontId::proportional(9.0),
                                    theme::TEXT,
                                );
                                let stroke = if sel {
                                    egui::Stroke::new(2.0, Color32::from_rgb(200, 50, 60))
                                } else if resp.hovered() {
                                    egui::Stroke::new(1.0, GOLD)
                                } else {
                                    egui::Stroke::new(1.0, Color32::from_rgb(60, 52, 44))
                                };
                                painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Outside);
                                resp.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::Role::Button,
                                        true,
                                        sel,
                                        &e.label,
                                    )
                                });
                                if resp.on_hover_text(&e.tooltip).clicked() {
                                    chosen = Some(e.kind.clone());
                                }
                            }
                        });
                    }
                });
            if let Some(k) = chosen {
                self.place = Some(k);
                self.set_tool(Tool::Place);
            }
        });
    }

    // ---------------------------------------------------------------------- right panel

    pub(super) fn right_panel(&mut self, ui: &mut egui::Ui) {
        let left = self.settings.layout.tabs_left;
        let tabs = [
            ("tree", trd("kfx_editor.tree", "Tree")),
            ("history", trd("kfx_editor.history", "History")),
            ("inspector", trd("kfx_editor.inspector", "Inspector")),
            ("things", trd("kfx_editor.things", "Things")),
            ("layers", trd("kfx_editor.layers", "Layers")),
            ("validate", trd("kfx_editor.validate", "Validate")),
            ("notes", trd("kfx_editor.notes", "Notes")),
            ("files", trd("kfx_editor.files", "Files")),
        ];
        ui.horizontal_wrapped(|ui| {
            for (k, label) in tabs {
                if k == "tree" && !left {
                    continue;
                }
                let label = if k == "validate" {
                    format!("{label} ({})", self.issues.len())
                } else {
                    label.to_string()
                };
                if ui
                    .add(egui::Button::selectable(
                        self.settings.layout.right_tab == k,
                        label,
                    ))
                    .clicked()
                {
                    self.settings.layout.right_tab = k.into();
                }
            }
        });
        ui.separator();
        match self.settings.layout.right_tab.as_str() {
            "inspector" => self.inspector_ui(ui),
            "things" => self.things_list_ui(ui),
            "layers" => self.layers_ui(ui),
            "validate" => self.validate_ui(ui),
            "notes" => self.notes_ui(ui),
            "files" => self.level_files_ui(ui),
            "tree" if left => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.players_panel(ui));
            }
            _ => self.history_panel(ui),
        }
    }

    fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else {
            ui.label(RichText::new(trd("kfx_editor.no_map_open", "No map open")).color(TEXT_DIM));
            return;
        };
        if self.selected.is_empty() {
            ui.label(
                RichText::new(trd(
                    "kfx_editor.nothing_selected_use_the_things_tool_click_shift",
                    "Nothing selected. Use the Things tool (click, Shift+click, drag a \
                    box) or the Things list.",
                ))
                .color(TEXT_DIM),
            );
            return;
        }
        let gd = doc.gd.clone();
        let items: Vec<ItemRef> = self.selected.iter().copied().collect();
        // read-only info
        let infos: Vec<String> = items
            .iter()
            .take(6)
            .map(|it| match *it {
                ItemRef::Thing(i) => doc
                    .doc
                    .things
                    .items
                    .get(i)
                    .map(|t| {
                        let src = match thing_class(t) {
                            Some(class::CREATURE) => self
                                .sprites
                                .as_ref()
                                .and_then(|l| {
                                    l.creatures.get(t.subtype.unwrap_or(1).max(1) as usize - 1)
                                })
                                .and_then(|c| c.sources.last())
                                .map(|p| p.display().to_string()),
                            Some(class::OBJECT) => config_source(
                                &doc.layers,
                                "objects.cfg",
                                &format!("object{}", t.subtype.unwrap_or(-1)),
                            ),
                            Some(class::TRAP) => config_source(
                                &doc.layers,
                                "trapdoor.cfg",
                                &format!("trap{}", t.subtype.unwrap_or(-1)),
                            ),
                            Some(class::DOOR) => config_source(
                                &doc.layers,
                                "trapdoor.cfg",
                                &format!("door{}", t.subtype.unwrap_or(-1)),
                            ),
                            Some(class::EFFECT_GEN) => doc
                                .layers
                                .files("effects.toml")
                                .last()
                                .map(|p| p.0.display().to_string()),
                            _ => None,
                        };
                        trf(
                            "kfx_editor.i_t_t2_model_subtype_parent_tile_config_src",
                            "#{i} {t} {t2} (model {subtype}){parent_tile}\n  config: {src}",
                            &[
                                ("i", &i),
                                ("t", &class_name(thing_class(t))),
                                ("t2", &model_name(&gd, t)),
                                ("subtype", &t.subtype.unwrap_or(-1)),
                                (
                                    "parent_tile",
                                    &(if t.parent_tile.is_some() {
                                        trd("kfx_editor.slab_object", ", slab object")
                                    } else {
                                        String::new()
                                    }),
                                ),
                                ("src", &src.unwrap_or_else(|| "?".into())),
                            ],
                        )
                    })
                    .unwrap_or_default(),
                ItemRef::ActionPoint(i) => trf(
                    "kfx_editor.action_point_i",
                    "Action point #{i}",
                    &[("i", &i)],
                ),
                ItemRef::Light(i) => trf("kfx_editor.light_i", "Light #{i}", &[("i", &i)]),
            })
            .collect();
        // fields that apply to at least one selected item, value when all agree
        let dims = doc.dims();
        let mut rows: Vec<(&str, Tx, Option<String>)> = vec![];
        for (f, help) in FIELDS {
            let vals: Vec<String> = items
                .iter()
                .filter_map(|it| field_value(&doc.doc, *it, f, dims))
                .collect();
            if vals.is_empty() {
                continue;
            }
            let same = vals.iter().all(|v| *v == vals[0]);
            rows.push((f, *help, same.then(|| vals[0].clone())));
        }
        let rev = (doc.revision, self.selected.clone());
        if self.inspector_rev.as_ref() != Some(&rev) {
            self.inspector_buf = rows
                .iter()
                .map(|(f, _, v)| (f.to_string(), v.clone().unwrap_or_default()))
                .collect();
            self.inspector_rev = Some(rev);
        }
        // model choices when every selected item is a thing of the same class (doors excluded)
        let classes: BTreeSet<Option<u8>> = items
            .iter()
            .map(|it| {
                if let ItemRef::Thing(i) = it {
                    doc.doc.things.items.get(*i).and_then(thing_class)
                } else {
                    None
                }
            })
            .collect();
        let model_class = (classes.len() == 1)
            .then(|| *classes.iter().next().unwrap())
            .flatten()
            .filter(|c| *c != class::DOOR);
        let models: Vec<(i64, String)> = match model_class {
            Some(class::CREATURE) => gd
                .creatures
                .iter()
                .enumerate()
                .map(|(i, n)| (i as i64 + 1, n.clone()))
                .collect(),
            Some(class::OBJECT) => gd
                .object_names()
                .into_iter()
                .filter(|x| x.0 > 0)
                .map(|(i, n)| (i as i64, n))
                .collect(),
            Some(class::TRAP) => gd
                .trap_names()
                .into_iter()
                .filter(|x| x.0 > 0)
                .map(|(i, n)| (i as i64, n))
                .collect(),
            Some(class::EFFECT_GEN) => gd
                .effect_generator_names()
                .into_iter()
                .filter(|x| x.0 > 0)
                .map(|(i, n)| (i as i64, n))
                .collect(),
            _ => vec![],
        };
        let cur_model: Option<String> = {
            let names: BTreeSet<String> = items
                .iter()
                .filter_map(|it| {
                    if let ItemRef::Thing(i) = it {
                        doc.doc.things.items.get(*i).map(|t| model_name(&gd, t))
                    } else {
                        None
                    }
                })
                .collect();
            (names.len() == 1).then(|| names.into_iter().next().unwrap())
        };
        ui.label(
            RichText::new(trf(
                "kfx_editor.items_selected",
                "{items} selected",
                &[("items", &items.len())],
            ))
            .color(GOLD)
            .strong(),
        );
        let mut apply = None;
        let mut act = None;
        let bufs = &mut self.inspector_buf;
        let search = &mut self.model_search;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for t in &infos {
                    ui.label(RichText::new(t).small().color(TEXT_DIM));
                }
                if items.len() > 6 {
                    ui.label(
                        RichText::new(trf(
                            "kfx_editor.and_items_more",
                            "… and {items} more",
                            &[("items", &(items.len() - 6))],
                        ))
                        .small()
                        .color(TEXT_DIM),
                    );
                }
                ui.separator();
                egui::Grid::new("inspector")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        if !models.is_empty() {
                            ui.label(trd("kfx_editor.model", "model"))
                                .on_hover_text(trd(
                                    "kfx_editor.model_of_the_selected_things_searchable",
                                    "model of the selected things (searchable)",
                                ));
                            let text = cur_model.clone().unwrap_or_else(|| "(mixed)".into());
                            egui::ComboBox::from_id_salt("insp-model")
                                .width(170.0)
                                .height(320.0)
                                .selected_text(text)
                                .show_ui(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(search)
                                            .hint_text(trd("kfx_editor.search_e1e9", "search"))
                                            .desired_width(160.0),
                                    );
                                    let q = search.trim().to_ascii_lowercase();
                                    for (m, n) in &models {
                                        if !q.is_empty()
                                            && !n.to_ascii_lowercase().contains(&q)
                                            && m.to_string() != q
                                        {
                                            continue;
                                        }
                                        if ui
                                            .selectable_label(
                                                cur_model.as_deref() == Some(n.as_str()),
                                                format!("{n}  ({m})"),
                                            )
                                            .clicked()
                                        {
                                            apply = Some(("model".to_string(), m.to_string()));
                                        }
                                    }
                                });
                            ui.end_row();
                        }
                        for (f, help, v) in &rows {
                            let lr = ui.label(*f).on_hover_text(*help);
                            match *f {
                                "owner" => {
                                    let cur: Option<u8> = v.as_deref().and_then(|x| x.parse().ok());
                                    let label = cur
                                        .map(|o| PLAYERS[o.min(8) as usize].0.to_string())
                                        .unwrap_or_else(|| "(mixed)".into());
                                    egui::ComboBox::from_id_salt("insp-owner")
                                        .width(150.0)
                                        .selected_text(label)
                                        .show_ui(ui, |ui| {
                                            for o in 0..9u8 {
                                                let (name, c) = PLAYERS[o as usize];
                                                let r = ui.horizontal(|ui| {
                                                    let (rect, _) = ui.allocate_exact_size(
                                                        EVec2::splat(12.0),
                                                        egui::Sense::hover(),
                                                    );
                                                    ui.painter().circle_filled(
                                                        rect.center(),
                                                        5.5,
                                                        Color32::from_rgb(c[0], c[1], c[2]),
                                                    );
                                                    ui.selectable_label(
                                                        cur == Some(o),
                                                        format!("{o}: {name}"),
                                                    )
                                                });
                                                if r.inner.clicked() {
                                                    apply =
                                                        Some(("owner".to_string(), o.to_string()));
                                                }
                                            }
                                        });
                                }
                                "orientation" => {
                                    let mut a: i64 =
                                        v.as_deref().and_then(|x| x.parse().ok()).unwrap_or(0);
                                    ui.horizontal(|ui| {
                                        if let Some(n) = compass_dial(ui, a, v.is_none()) {
                                            apply =
                                                Some(("orientation".to_string(), n.to_string()));
                                        }
                                        if ui
                                            .add(egui::DragValue::new(&mut a).range(0..=2047))
                                            .changed()
                                        {
                                            apply =
                                                Some(("orientation".to_string(), a.to_string()));
                                        }
                                        ui.label(
                                            RichText::new(compass_name(a)).small().color(TEXT_DIM),
                                        );
                                    });
                                }
                                "level" => {
                                    let mut l: i64 =
                                        v.as_deref().and_then(|x| x.parse().ok()).unwrap_or(1);
                                    if ui
                                        .add(
                                            egui::Slider::new(&mut l, 1..=10)
                                                .text(if v.is_none() { "(mixed)" } else { "" }),
                                        )
                                        .changed()
                                    {
                                        apply = Some(("level".to_string(), l.to_string()));
                                    }
                                }
                                "locked" | "dynamic" => {
                                    let mut b = v.as_deref() == Some("true");
                                    let r = ui.checkbox(
                                        &mut b,
                                        if v.is_none() {
                                            trd("kfx_editor.mixed", "(mixed)")
                                        } else {
                                            String::new()
                                        },
                                    );
                                    if r.changed() {
                                        apply = Some((f.to_string(), b.to_string()));
                                    }
                                }
                                _ => {
                                    let buf = bufs.entry(f.to_string()).or_default();
                                    let r = ui
                                        .add(
                                            egui::TextEdit::singleline(buf)
                                                .desired_width(120.0)
                                                .hint_text(if v.is_none() {
                                                    trd("kfx_editor.mixed", "(mixed)")
                                                } else {
                                                    String::new()
                                                }),
                                        )
                                        .labelled_by(lr.id);
                                    if r.lost_focus()
                                        && v.as_deref() != Some(buf.as_str())
                                        && !(v.is_none() && buf.is_empty())
                                    {
                                        apply = Some((f.to_string(), buf.clone()));
                                    }
                                }
                            }
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(trd("kfx_editor.delete", "🗑 Delete")).clicked() {
                        act = Some(0);
                    }
                    if ui
                        .button(trd("kfx_editor.duplicate", "Duplicate"))
                        .clicked()
                    {
                        act = Some(1);
                    }
                    if ui
                        .button(trd("kfx_editor.turn_left", "⟲ Turn left"))
                        .clicked()
                    {
                        act = Some(2);
                    }
                    if ui
                        .button(trd("kfx_editor.turn_right", "⟳ Turn right"))
                        .clicked()
                    {
                        act = Some(3);
                    }
                });
            });
        if let Some((f, v)) = apply {
            if let Err(e) = self.set_selection_field(&f, &v) {
                self.status = format!("{f}: {e}");
                self.inspector_rev = None;
            }
        }
        match act {
            Some(0) => self.delete_things(),
            Some(1) => self.duplicate_things(),
            Some(2) => self.rotate_things(-1),
            Some(3) => self.rotate_things(1),
            _ => {}
        }
    }

    fn things_list_ui(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else {
            ui.label(RichText::new(trd("kfx_editor.no_map_open", "No map open")).color(TEXT_DIM));
            return;
        };
        let gd = doc.gd.clone();
        let t = &doc.doc.things.items;
        let creatures = t
            .iter()
            .filter(|x| thing_class(x) == Some(class::CREATURE))
            .count();
        ui.label(
            RichText::new(trf(
                "kfx_editor.things_t_validate_creatures_creatures_validate2",
                "Things {t}/{max_map_things} · creatures {creatures}/{max_creatures} · \
                action points {items}/{max_action_points} · lights {items2}/{max_lights}",
                &[
                    ("t", &t.len()),
                    ("max_map_things", &validate::MAX_MAP_THINGS),
                    ("creatures", &creatures),
                    ("max_creatures", &(validate::MAX_CREATURES - 1)),
                    ("items", &doc.doc.action_points.items.len()),
                    ("max_action_points", &validate::MAX_ACTION_POINTS),
                    ("items2", &doc.doc.lights.items.len()),
                    ("max_lights", &validate::MAX_LIGHTS),
                ],
            ))
            .small(),
        );
        ui.horizontal(|ui| {
            let kind_label = LIST_KINDS
                .iter()
                .find(|(k, _)| *k == self.list_filter.kind)
                .map(|(_, l)| l.text())
                .unwrap_or_else(|| self.list_filter.kind.clone());
            egui::ComboBox::from_id_salt("list-kind")
                .selected_text(kind_label)
                .width(110.0)
                .show_ui(ui, |ui| {
                    for (k, label) in LIST_KINDS {
                        ui.selectable_value(&mut self.list_filter.kind, k.to_string(), label);
                    }
                });
            egui::ComboBox::from_id_salt("list-owner")
                .selected_text(
                    self.list_filter
                        .owner
                        .map(|o| PLAYERS[o as usize].0.to_string())
                        .unwrap_or_else(|| trd("kfx_editor.any_owner", "Any owner").into()),
                )
                .width(110.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.list_filter.owner,
                        None,
                        trd("kfx_editor.any_owner", "Any owner"),
                    );
                    for o in 0..9u8 {
                        ui.selectable_value(
                            &mut self.list_filter.owner,
                            Some(o),
                            PLAYERS[o as usize].0,
                        );
                    }
                });
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.list_filter.text)
                .hint_text(trd("kfx_editor.filter_by_name", "filter by name"))
                .desired_width(f32::INFINITY),
        );
        let kind = self.list_filter.kind.clone();
        let owner = self.list_filter.owner;
        let text = self.list_filter.text.to_ascii_lowercase();
        let want = |k: &str| kind == "All" || kind == k;
        let mut rows: Vec<(ItemRef, String, Option<i64>)> = vec![];
        for (i, th) in t.iter().enumerate() {
            let k = match thing_class(th) {
                Some(class::CREATURE) => "Creatures",
                Some(class::OBJECT) => "Objects",
                Some(class::TRAP) => "Traps",
                Some(class::DOOR) => "Doors",
                Some(class::EFFECT_GEN) => "Effects",
                _ => "Objects",
            };
            if !want(k) || owner.is_some_and(|o| th.owner != Some(o as i64)) {
                continue;
            }
            let name = model_name(&gd, th);
            let pos =
                th.x.zip(th.y)
                    .map(|(x, y)| format!("({},{})", x.stl, y.stl))
                    .unwrap_or_default();
            let lvl = th
                .creature_level
                .map(|l| format!(" L{l}"))
                .unwrap_or_default();
            let label = format!("{name}{lvl} {pos}");
            if !text.is_empty() && !label.to_ascii_lowercase().contains(&text) {
                continue;
            }
            rows.push((ItemRef::Thing(i), label, th.owner));
        }
        if owner.is_none() {
            if want("Action points") {
                for (i, a) in doc.doc.action_points.items.iter().enumerate() {
                    let label = trf(
                        "kfx_editor.action_point_number_stl_stl2",
                        "Action point {number} ({stl},{stl2})",
                        &[
                            ("number", &a.number.unwrap_or(0)),
                            ("stl", &a.x.map_or(0, |c| c.stl)),
                            ("stl2", &a.y.map_or(0, |c| c.stl)),
                        ],
                    );
                    if text.is_empty() || label.to_ascii_lowercase().contains(&text) {
                        rows.push((ItemRef::ActionPoint(i), label, None));
                    }
                }
            }
            if want("Lights") {
                for (i, l) in doc.doc.lights.items.iter().enumerate() {
                    let label = trf(
                        "kfx_editor.light_parent_tile_stl_stl2",
                        "Light{parent_tile} ({stl},{stl2})",
                        &[
                            (
                                "parent_tile",
                                &(if l.parent_tile.is_some() {
                                    trd("kfx_editor.slab", " (slab)")
                                } else {
                                    String::new()
                                }),
                            ),
                            ("stl", &l.x.map_or(0, |c| c.stl)),
                            ("stl2", &l.y.map_or(0, |c| c.stl)),
                        ],
                    );
                    if text.is_empty() || label.to_ascii_lowercase().contains(&text) {
                        rows.push((ItemRef::Light(i), label, None));
                    }
                }
            }
        }
        ui.label(
            RichText::new(trf(
                "kfx_editor.rows_shown",
                "{rows} shown",
                &[("rows", &rows.len())],
            ))
            .small()
            .color(TEXT_DIM),
        );
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 18.0, rows.len(), |ui, range| {
                for (it, label, o) in &rows[range] {
                    let c = o.map(owner_rgb).unwrap_or([200, 200, 200]);
                    let sel = self.selected.contains(it);
                    let r = ui.add(egui::Button::selectable(
                        sel,
                        RichText::new(label).color(Color32::from_rgb(
                            c[0].max(90),
                            c[1].max(90),
                            c[2].max(90),
                        )),
                    ));
                    if r.clicked() {
                        clicked = Some(*it);
                    }
                }
            });
        if let Some(it) = clicked {
            if ui.input(|i| i.modifiers.shift) {
                self.selected.insert(it);
            } else {
                self.selected = std::collections::BTreeSet::from([it]);
            }
            self.focus_item(it);
        }
    }

    fn layers_ui(&mut self, ui: &mut egui::Ui) {
        let l = &mut self.settings.layout.layers;
        for (name, label) in [
            ("terrain", trd("kfx_editor.terrain", "Terrain")),
            ("creatures", trd("kfx_editor.creatures_f8b1", "Creatures")),
            ("objects", trd("kfx_editor.objects", "Objects")),
            ("traps", trd("kfx_editor.traps", "Traps")),
            ("doors", trd("kfx_editor.doors", "Doors")),
            (
                "effects",
                trd("kfx_editor.effect_generators", "Effect generators"),
            ),
            (
                "slab_things",
                trd(
                    "kfx_editor.slab_generated_things_torches_furniture",
                    "Slab-generated things (torches, furniture)",
                ),
            ),
            (
                "action_points",
                trd("kfx_editor.action_points", "Action points"),
            ),
            ("lights", trd("kfx_editor.lights", "Lights")),
            (
                "ranges",
                trd("kfx_editor.show_all_ranges", "Show all ranges"),
            ),
            (
                "owner_tint",
                trd(
                    "kfx_editor.owner_tint_on_creatures",
                    "Owner tint on creatures",
                ),
            ),
            (
                "level_badges",
                trd("kfx_editor.creature_level_badges", "Creature level badges"),
            ),
            (
                "gate_numbers",
                trd("kfx_editor.hero_gate_numbers", "Hero gate numbers"),
            ),
            (
                "script_markers",
                trd(
                    "kfx_editor.script_markers_references_of_the_level_scripts",
                    "Script markers (references of the level scripts)",
                ),
            ),
            (
                "animate",
                trd("kfx_editor.animate_sprites", "Animate sprites"),
            ),
        ] {
            if let Some(v) = l.get_mut(name) {
                ui.checkbox(v, label);
            }
        }
        ui.separator();
        let lay = &mut self.settings.layout;
        ui.checkbox(
            &mut lay.show_ownership,
            trd(
                "kfx_editor.terrain_ownership_tint",
                "Terrain ownership tint",
            ),
        );
        ui.checkbox(&mut lay.show_grid, trd("kfx_editor.slab_grid", "Slab grid"));
        ui.checkbox(&mut lay.lighting, trd("kfx_editor.lighting", "Lighting"));
    }

    fn validate_ui(&mut self, ui: &mut egui::Ui) {
        if self.doc.is_none() {
            ui.label(RichText::new(trd("kfx_editor.no_map_open", "No map open")).color(TEXT_DIM));
            return;
        }
        let (e, w, i) = self.issue_counts();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(trf("kfx_editor.e_errors", "{e} errors", &[("e", &e)]))
                    .color(Color32::from_rgb(240, 90, 80)),
            );
            ui.label(
                RichText::new(trf("kfx_editor.w_warnings", "{w} warnings", &[("w", &w)]))
                    .color(Color32::from_rgb(240, 190, 80)),
            );
            ui.label(
                RichText::new(trf("kfx_editor.i_notes", "{i} notes", &[("i", &i)])).color(TEXT_DIM),
            );
            if ui.button(trd("kfx_editor.re_check", "Re-check")).clicked() {
                self.revalidate();
            }
        });
        let mut go = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (k, iss) in self.issues.iter().enumerate() {
                    let c = match iss.severity {
                        Severity::Error => Color32::from_rgb(240, 90, 80),
                        Severity::Warning => Color32::from_rgb(240, 190, 80),
                        Severity::Info => TEXT_DIM,
                    };
                    let r = ui
                        .add(
                            egui::Button::selectable(
                                false,
                                RichText::new(format!("[{}] {}", iss.code, iss.message)).color(c),
                            )
                            .wrap(),
                        )
                        .on_hover_text(trd("kfx_editor.click_to_locate", "Click to locate"));
                    if r.clicked() {
                        go = Some(k);
                    }
                }
            });
        if let Some(k) = go {
            self.locate_issue(k);
        }
    }

    pub(super) fn issue_counts(&self) -> (usize, usize, usize) {
        let c = |s| self.issues.iter().filter(|i| i.severity == s).count();
        (c(Severity::Error), c(Severity::Warning), c(Severity::Info))
    }

    /// Run the checks now.
    pub fn revalidate(&mut self) {
        let links = self.script_link_issues();
        if let Some(d) = &self.doc {
            let mut v: Vec<(validate::Issue, Option<(String, kfx_script::Span)>)> =
                validate::validate(&d.doc, &d.gd)
                    .into_iter()
                    .map(|i| (i, None))
                    .collect();
            v.extend(
                document::extra_issues(&d.doc, &d.gd)
                    .into_iter()
                    .map(|i| (i, None)),
            );
            v.extend(self.analysis_findings().into_iter().map(|i| (i, None)));
            v.extend(links);
            v.sort_by(|a, b| b.0.severity.cmp(&a.0.severity));
            let (issues, links): (Vec<_>, Vec<_>) = v.into_iter().unzip();
            self.issues = issues;
            self.issue_links = links;
            self.issues_rev = d.revision;
        }
    }

    /// Debounced checks: half a second after the last change.
    pub(super) fn validate_tick(&mut self) {
        let Some(d) = &self.doc else { return };
        if d.revision == self.issues_rev || d.in_stroke() {
            return;
        }
        if self.issues_seen_rev != d.revision {
            self.issues_seen_rev = d.revision;
            self.issues_changed = Instant::now();
            return;
        }
        if self.issues_changed.elapsed() >= Duration::from_millis(500) || self.fixed_frame.is_some()
        {
            self.revalidate();
        } else {
            self.ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    /// Centre on an issue and select what it is about.
    pub fn locate_issue(&mut self, k: usize) {
        let Some(iss) = self.issues.get(k).cloned() else {
            return;
        };
        if let Some(Some((tab, span))) = self.issue_links.get(k).cloned() {
            self.reveal_script(&tab, span);
            self.status = iss.message;
            return;
        }
        match iss.at {
            Location::Map => {}
            Location::Slab(x, y) => {
                self.camera.target = Vec3::new(
                    x as f32 * 3.0 + 1.5,
                    self.camera.target.y,
                    y as f32 * 3.0 + 1.5,
                );
                self.selection = Some(((x, y), (x, y)));
            }
            Location::Subtile(x, y) => {
                self.camera.target = Vec3::new(x as f32 + 0.5, self.camera.target.y, y as f32 + 0.5)
            }
            Location::Item(it) => {
                self.selected = std::collections::BTreeSet::from([it]);
                self.focus_item(it);
            }
        }
        self.status = iss.message;
    }
}

/// The last config layer whose `file` defines `section`.
fn config_source(layers: &ConfigLayers, file: &str, section: &str) -> Option<String> {
    layers
        .files(file)
        .into_iter()
        .rev()
        .find(|(p, _)| kfx_formats::cfg::CfgDoc::load(p).is_ok_and(|d| d.has_section(section)))
        .map(|(p, _)| p.display().to_string())
}

/// Current value of an inspector field for an item, `None` when it does not apply.
pub fn field_value(d: &MapDoc, it: ItemRef, f: &str, _dims: MapDims) -> Option<String> {
    let c =
        |x: Option<kfx_formats::level_fx::StlCoord>| x.map(|c| c.stl as f32 + c.sub as f32 / 256.0);
    let fmt = |v: f32| format!("{}", (v * 256.0).round() / 256.0);
    match it {
        ItemRef::Thing(i) => {
            let t: &FxThing = d.things.items.get(i)?;
            let cls = thing_class(t);
            match f {
                "owner" => Some(t.owner.unwrap_or(5).to_string()),
                "x" => t.x.map(|c| c.stl.to_string()),
                "y" => t.y.map(|c| c.stl.to_string()),
                "sub_x" => t.x.map(|c| c.sub.to_string()),
                "sub_y" => t.y.map(|c| c.sub.to_string()),
                "z" => Some(t.z.map_or(0, |c| c.stl).to_string()),
                "sub_z" => Some(t.z.map_or(0, |c| c.sub).to_string()),
                "orientation" if cls != Some(class::DOOR) => {
                    Some(t.orientation.unwrap_or(0).to_string())
                }
                "level" if cls == Some(class::CREATURE) => {
                    Some(t.creature_level.unwrap_or(1).to_string())
                }
                "creature_gold" if cls == Some(class::CREATURE) => {
                    Some(t.creature_gold.unwrap_or(0).to_string())
                }
                "health" if cls == Some(class::CREATURE) => {
                    Some(t.creature_initial_health.unwrap_or(100).to_string())
                }
                "name" if cls == Some(class::CREATURE) => {
                    Some(t.creature_name.clone().unwrap_or_default())
                }
                "gold_value" if cls == Some(class::OBJECT) => {
                    Some(t.gold_value.unwrap_or(0).to_string())
                }
                "herogate" if t.herogate_number.is_some() => {
                    Some(t.herogate_number.unwrap_or(0).to_string())
                }
                "custom_box" if t.custom_box.is_some() => {
                    Some(t.custom_box.unwrap_or(0).to_string())
                }
                "effect_range" if cls == Some(class::EFFECT_GEN) => {
                    Some(fmt(c(t.effect_range).unwrap_or(0.0)))
                }
                "locked" if cls == Some(class::DOOR) => {
                    Some((t.door_locked.unwrap_or(0) > 0).to_string())
                }
                "door_orientation" if cls == Some(class::DOOR) => {
                    Some(t.door_orientation.unwrap_or(0).to_string())
                }
                "parent" if cls != Some(class::CREATURE) => Some(
                    t.parent_tile
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "none".into()),
                ),
                _ => None,
            }
        }
        ItemRef::ActionPoint(i) => {
            let a = d.action_points.items.get(i)?;
            match f {
                "x" => a.x.map(|c| c.stl.to_string()),
                "y" => a.y.map(|c| c.stl.to_string()),
                "sub_x" => a.x.map(|c| c.sub.to_string()),
                "sub_y" => a.y.map(|c| c.sub.to_string()),
                "number" => Some(a.number.unwrap_or(0).to_string()),
                "range" => Some(fmt(c(a.range).unwrap_or(0.0))),
                _ => None,
            }
        }
        ItemRef::Light(i) => {
            let l = d.lights.items.get(i)?;
            match f {
                "x" => l.x.map(|c| c.stl.to_string()),
                "y" => l.y.map(|c| c.stl.to_string()),
                "sub_x" => l.x.map(|c| c.sub.to_string()),
                "sub_y" => l.y.map(|c| c.sub.to_string()),
                "z" => Some(l.z.map_or(0, |c| c.stl).to_string()),
                "sub_z" => Some(l.z.map_or(0, |c| c.sub).to_string()),
                "light_range" => Some(fmt(c(l.range).unwrap_or(0.0))),
                "intensity" => Some(l.intensity.unwrap_or(0).to_string()),
                "dynamic" => Some(l.dynamic.is_some_and(|f| f.is_set()).to_string()),
                "parent" => Some(
                    l.parent_tile
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "none".into()),
                ),
                _ => None,
            }
        }
    }
}

/// Compass name of an orientation (0 north, 512 east, clockwise).
pub fn compass_name(a: i64) -> &'static str {
    const N: [&str; 8] = [
        "north",
        "north-east",
        "east",
        "south-east",
        "south",
        "south-west",
        "west",
        "north-west",
    ];
    N[(((a.rem_euclid(2048) + 128) / 256) % 8) as usize]
}

/// A small compass dial (north up, clockwise like the game's angles); drag or click to set the
/// angle (snaps to 1/16 of a turn unless Shift is held). Returns the new angle.
pub fn compass_dial(ui: &mut egui::Ui, a: i64, mixed: bool) -> Option<i64> {
    let (rect, resp) = ui.allocate_exact_size(EVec2::splat(44.0), egui::Sense::click_and_drag());
    let p = ui.painter_at(rect.expand(1.0));
    let c = rect.center();
    let r = rect.width() / 2.0 - 2.0;
    p.circle(
        c,
        r,
        Color32::from_rgb(20, 18, 16),
        egui::Stroke::new(1.0, GOLD_DIM),
    );
    for k in 0..8 {
        let t = k as f32 * std::f32::consts::TAU / 8.0;
        let d = EVec2::new(t.sin(), -t.cos());
        p.line_segment(
            [c + d * (r - 4.0), c + d * r],
            egui::Stroke::new(1.0, GOLD_DIM),
        );
    }
    p.text(
        c + EVec2::new(0.0, -r + 7.0),
        egui::Align2::CENTER_CENTER,
        "N",
        egui::FontId::proportional(8.0),
        TEXT_DIM,
    );
    if !mixed {
        let t = a as f32 * std::f32::consts::TAU / 2048.0;
        let d = EVec2::new(t.sin(), -t.cos());
        p.line_segment([c, c + d * (r - 3.0)], egui::Stroke::new(2.0, GOLD));
        p.circle_filled(c + d * (r - 3.0), 2.5, GOLD);
    }
    if resp.drag_stopped() || resp.clicked() {
        let pos = resp.interact_pointer_pos()?;
        let v = pos - c;
        if v.length() < 3.0 {
            return None;
        }
        let ang = v.x.atan2(-v.y).rem_euclid(std::f32::consts::TAU);
        let mut n = (ang / std::f32::consts::TAU * 2048.0).round() as i64 % 2048;
        if !ui.input(|i| i.modifiers.shift) {
            n = ((n + 64) / 128 * 128) % 2048;
        }
        return (n != a || mixed).then_some(n);
    }
    None
}
