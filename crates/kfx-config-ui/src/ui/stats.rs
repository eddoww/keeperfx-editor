//! Creature stats comparison: every creature's main stats side by side at one experience level,
//! sortable by any column, with the cells whose value comes from an override layer (mod,
//! campaign, map) highlighted in that layer's colour.
//!
//! Values are the game's: the cfg value of `creatrs/<name>.cfg` as the layers end up with it
//! (unsaved edits included), grown to the chosen level by the `compute_creature_max_*`
//! formulas of thing_stats.c with the creature.cfg `[experience]` rates
//! ([`crate::creature::Growth::at_level`]). The stat list and its sources are
//! [`crate::creature::STATS`].
//!
//! # Host integration
//!
//! The widget is host independent: [`CreatureStats::show`] draws into any `Ui` from the
//! [`ConfigSession`] and returns what the user asked for. A host that already has a
//! [`crate::ConfigEditor`] only needs one call per frame:
//!
//! ```ignore
//! // menu: open it
//! editor.stats_open = true;
//! // every frame (draws the window while `stats_open`)
//! let f = editor.creature_stats_window(ctx);
//! if f.show_editor { /* make the configuration editor visible */ }
//! handle_config_events(f.events);
//! ```
//!
//! Clicking a creature name opens it in the editor's Creatures tab (`show_editor` is set);
//! right-click offers to open its cfg file (a [`crate::ConfigEvent::RequestOpenFile`]).
use super::common::{layer_color, weak};
use crate::cfgedit::LayeredCfg;
use crate::creature::{stat_rows, unlisted_creatures, ExpRates, Growth, StatRow, MAX_LEVEL, STATS};
use crate::session::ConfigSession;
use egui::{Color32, RichText, Ui};
use std::path::PathBuf;

/// Sort column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    /// Load order (creature.cfg `[common] Creatures`).
    Order,
    Name,
    /// Index into [`STATS`].
    Stat(usize),
}

/// What the user asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatsAction {
    /// Edit this creature (upper case name).
    Edit(String),
    /// Open this file in an external program.
    OpenFile(PathBuf),
}

/// State of the comparison table.
pub struct CreatureStats {
    /// Experience level, 1..=10.
    pub level: u8,
    pub sort: SortBy,
    pub ascending: bool,
    /// Case-insensitive part of the creature name.
    pub filter: String,
    /// Also list creature files that creature.cfg does not list (the game does not load them).
    pub show_unlisted: bool,
    cache: Option<(u64, u8, bool, Vec<StatRow>)>,
}

impl Default for CreatureStats {
    fn default() -> Self {
        CreatureStats {
            level: 1,
            sort: SortBy::Order,
            ascending: true,
            filter: String::new(),
            show_unlisted: false,
            cache: None,
        }
    }
}

/// Path of the file of `layer` in an open cfg.
fn layer_path(c: &LayeredCfg, layer: usize) -> Option<PathBuf> {
    if layer == c.target_layer {
        return c.target_path.clone();
    }
    c.lower
        .iter()
        .chain(&c.upper)
        .find(|d| d.layer == layer)
        .map(|d| d.path.clone())
}

impl CreatureStats {
    /// The rows at the current level and filters, in display order.
    pub fn rows(&mut self, s: &mut ConfigSession) -> Vec<StatRow> {
        let fresh = self
            .cache
            .as_ref()
            .is_some_and(|c| c.0 == s.generation && c.1 == self.level && c.2 == self.show_unlisted);
        if !fresh {
            let mut names: Vec<(String, bool)> =
                s.creature_names().into_iter().map(|n| (n, true)).collect();
            if self.show_unlisted {
                names.extend(unlisted_creatures(s).into_iter().map(|n| (n, false)));
            }
            let rows = stat_rows(s, &names, self.level);
            self.cache = Some((s.generation, self.level, self.show_unlisted, rows));
        }
        let f = self.filter.trim().to_ascii_lowercase();
        let mut rows: Vec<StatRow> = self
            .cache
            .as_ref()
            .unwrap()
            .3
            .iter()
            .filter(|r| f.is_empty() || r.name.to_ascii_lowercase().contains(&f))
            .cloned()
            .collect();
        let asc = self.ascending;
        match self.sort {
            SortBy::Order => {
                if !asc {
                    rows.reverse();
                }
            }
            SortBy::Name => rows.sort_by(|a, b| {
                if asc {
                    a.name.cmp(&b.name)
                } else {
                    b.name.cmp(&a.name)
                }
            }),
            SortBy::Stat(i) => rows.sort_by(|a, b| {
                let (x, y) = (
                    a.cells.get(i).and_then(|c| c.value),
                    b.cells.get(i).and_then(|c| c.value),
                );
                match (x, y) {
                    (Some(x), Some(y)) => {
                        (if asc { x.cmp(&y) } else { y.cmp(&x) }).then_with(|| a.name.cmp(&b.name))
                    }
                    // missing values last in both directions
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.name.cmp(&b.name),
                }
            }),
        }
        rows
    }

    /// Click on a column header: sort by it, or flip the direction when it already sorts.
    pub fn click_header(&mut self, by: SortBy) {
        if self.sort == by {
            self.ascending = !self.ascending;
        } else {
            self.sort = by;
            // numbers: biggest first
            self.ascending = !matches!(by, SortBy::Stat(_));
        }
    }

    /// Draw the table; returns the user's request of this frame. The header row stays visible
    /// while the rows scroll; wide tables scroll sideways (header and rows together).
    pub fn show(&mut self, ui: &mut Ui, s: &mut ConfigSession) -> Option<StatsAction> {
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            ui.label("Level");
            // hosts may shrink the default slider width
            ui.spacing_mut().slider_width = 160.0;
            ui.add(egui::Slider::new(&mut self.level, 1..=MAX_LEVEL as u8));
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("filter creatures")
                    .desired_width(140.0),
            );
            ui.checkbox(&mut self.show_unlisted, "Unlisted")
                .on_hover_text(
                    "Also show creature files that creature.cfg [common] \
                        Creatures does not list (the game does not load them)",
                );
            ui.separator();
            ui.label(weak("Override from:"));
            for l in s.layers.layers.iter().skip(1) {
                ui.label(
                    RichText::new(l.kind.badge())
                        .small()
                        .color(Color32::WHITE)
                        .background_color(layer_color(&l.kind)),
                );
            }
        });
        let rates = ExpRates::load(s);
        ui.label(weak(format!(
            "Values at level {} as the game computes them (thing_stats.c \
                compute_creature_max_*){}. Click a header to sort, a name to edit, right-click \
                a name to open its file.",
            self.level,
            if rates.overflow_8bit {
                "; OVERFLOW_8BIT classic bug on"
            } else {
                ""
            }
        )));
        ui.separator();
        let rows = self.rows(s);
        let mut clicked: Option<SortBy> = None;
        let arrow = |me: &Self, by: SortBy| {
            if me.sort == by {
                if me.ascending {
                    " ^"
                } else {
                    " v"
                }
            } else {
                ""
            }
        };
        // fixed column widths so the header (outside the vertical scroll) lines up with the rows
        let strong = |ui: &Ui, t: &str| {
            ui.painter()
                .layout_no_wrap(
                    format!("{t} v"),
                    egui::TextStyle::Button.resolve(ui.style()),
                    Color32::WHITE,
                )
                .size()
                .x
        };
        let mono = ui
            .painter()
            .layout_no_wrap(
                "0000000".into(),
                egui::TextStyle::Monospace.resolve(ui.style()),
                Color32::WHITE,
            )
            .size()
            .x;
        let name_w = rows
            .iter()
            .map(|r| {
                ui.painter()
                    .layout_no_wrap(
                        r.name.clone(),
                        egui::TextStyle::Monospace.resolve(ui.style()),
                        Color32::WHITE,
                    )
                    .size()
                    .x
            })
            .fold(strong(ui, "Creature"), f32::max)
            + 8.0;
        let num_w = strong(ui, "#").max(
            ui.painter()
                .layout_no_wrap(
                    "000".into(),
                    egui::TextStyle::Small.resolve(ui.style()),
                    Color32::WHITE,
                )
                .size()
                .x,
        ) + 6.0;
        let widths: Vec<f32> = STATS
            .iter()
            .map(|d| strong(ui, d.label).max(mono) + 10.0)
            .collect();
        let row_h = ui.spacing().interact_size.y;
        let cell = |ui: &mut Ui, w: f32, layout: egui::Layout, add: &mut dyn FnMut(&mut Ui)| {
            ui.allocate_ui_with_layout(egui::vec2(w, row_h), layout, |ui| {
                ui.set_min_size(egui::vec2(w, row_h));
                ui.set_max_width(w);
                add(ui);
            });
        };
        let left = egui::Layout::left_to_right(egui::Align::Center);
        let right = egui::Layout::right_to_left(egui::Align::Center);
        let order: Vec<String> = self
            .cache
            .as_ref()
            .map(|c| c.3.iter().map(|r| r.name.clone()).collect())
            .unwrap_or_default();
        egui::ScrollArea::horizontal()
            .id_salt("creature-stats-h")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // header
                ui.horizontal(|ui| {
                    cell(ui, num_w, left, &mut |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("#{}", arrow(self, SortBy::Order)))
                                        .strong(),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Load order")
                            .clicked()
                        {
                            clicked = Some(SortBy::Order);
                        }
                    });
                    cell(ui, name_w, left, &mut |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("Creature{}", arrow(self, SortBy::Name)))
                                        .strong(),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            clicked = Some(SortBy::Name);
                        }
                    });
                    for (i, d) in STATS.iter().enumerate() {
                        let mut tip =
                            format!("creatrs/<name>.cfg [{}] {}\n{}", d.section, d.key, d.doc);
                        if let (Some(k), Some(p)) = (d.growth.rate_key(), d.growth.rate(&rates)) {
                            tip.push_str(&format!(
                                "\n+{p}% per level (creature.cfg [experience] {k})"
                            ));
                        } else {
                            tip.push_str("\nsame at every level");
                        }
                        let label = format!("{}{}", d.label, arrow(self, SortBy::Stat(i)));
                        cell(ui, widths[i], right, &mut |ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(label.clone()).strong())
                                        .frame(false),
                                )
                                .on_hover_text(tip.clone())
                                .clicked()
                            {
                                clicked = Some(SortBy::Stat(i));
                            }
                        });
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("creature-stats-v")
                    .auto_shrink([true, false])
                    .show(ui, |ui| {
                        for (ri, row) in rows.iter().enumerate() {
                            let bg = ui.painter().add(egui::Shape::Noop);
                            let r = ui.horizontal(|ui| {
                                let n = order
                                    .iter()
                                    .position(|x| *x == row.name)
                                    .map(|i| i + 1)
                                    .unwrap_or(0);
                                cell(ui, num_w, left, &mut |ui| {
                                    ui.label(weak(if row.listed {
                                        n.to_string()
                                    } else {
                                        "-".into()
                                    }));
                                });
                                cell(ui, name_w, left, &mut |ui| {
                                    let mut name = RichText::new(&row.name).monospace();
                                    if !row.listed {
                                        name = name.italics().color(Color32::GRAY);
                                    }
                                    let r = ui
                                        .add(egui::Button::new(name).frame(false))
                                        .on_hover_text(if row.listed {
                                            "edit this creature"
                                        } else {
                                            "not in creature.cfg [common] Creatures: the game \
                                                does not load it"
                                        });
                                    if r.clicked() {
                                        action = Some(StatsAction::Edit(row.name.clone()));
                                    }
                                    r.context_menu(|ui| {
                                        let path = s
                                            .layers
                                            .layers
                                            .iter()
                                            .rev()
                                            .find_map(|l| l.existing(&*s.fs, &row.key));
                                        match path {
                                            Some(p) => {
                                                if ui
                                                    .button(format!("Open {}", p.display()))
                                                    .clicked()
                                                {
                                                    action = Some(StatsAction::OpenFile(p));
                                                    ui.close();
                                                }
                                            }
                                            None => {
                                                ui.label(weak("No file"));
                                            }
                                        }
                                    });
                                });
                                for (i, (d, c)) in STATS.iter().zip(&row.cells).enumerate() {
                                    let Some(v) = c.value else {
                                        cell(ui, widths[i], right, &mut |ui| {
                                            ui.label(weak("-"));
                                        });
                                        continue;
                                    };
                                    let grows = d.growth.rate(&rates).is_some_and(|p| p != 0)
                                        && self.level > 1;
                                    let over = c
                                        .layer
                                        .filter(|l| *l > 0)
                                        .or(c.rate_layer.filter(|l| *l > 0 && grows));
                                    let mut t = RichText::new(v.to_string()).monospace();
                                    if let Some(l) = over {
                                        t = t.color(Color32::WHITE).background_color(
                                            layer_color(&s.layers.layers[l].kind)
                                                .gamma_multiply(0.8),
                                        );
                                    }
                                    let tip = cell_tip(s, row, i, &rates, self.level);
                                    cell(ui, widths[i], right, &mut |ui| {
                                        ui.add_space(4.0);
                                        ui.label(t.clone()).on_hover_text(tip.clone());
                                    });
                                }
                            });
                            if ri % 2 == 1 {
                                ui.painter().set(
                                    bg,
                                    egui::Shape::rect_filled(
                                        r.response.rect,
                                        0.0,
                                        ui.visuals().faint_bg_color,
                                    ),
                                );
                            }
                        }
                        if rows.is_empty() {
                            ui.label(weak("No creature matches"));
                        }
                    });
            });
        if let Some(b) = clicked {
            self.click_header(b);
        }
        action
    }
}

/// Hover text of a cell: the cfg value, where it comes from, the level formula.
fn cell_tip(s: &mut ConfigSession, row: &StatRow, i: usize, rates: &ExpRates, level: u8) -> String {
    let d = &STATS[i];
    let c = &row.cells[i];
    let mut t = format!("{} [{}] {}", row.key, d.section, d.key);
    if let (Some(b), Some(l)) = (c.base, c.layer) {
        let kind = s.layers.layers[l].kind.badge();
        let path = s
            .cfg(&row.key)
            .and_then(|f| layer_path(f, l))
            .map(|p| format!(": {}", p.display()))
            .unwrap_or_default();
        t.push_str(&format!("\n= {b} from {kind}{path}"));
    }
    if let Some(p) = d.growth.rate(rates) {
        let src = c
            .rate_layer
            .map(|l| s.layers.layers[l].kind.badge())
            .unwrap_or_else(|| "game default".into());
        t.push_str(&format!(
            "\n+{p}% per level ({} from {src})",
            d.growth.rate_key().unwrap_or_default()
        ));
        if let (Some(b), Some(v)) = (c.base, c.value) {
            t.push_str(&format!("\nlevel {level}: {v}"));
            if v != b + p * b * (level as i64 - 1) / 100 {
                t.push_str(" (capped)");
            }
        }
    } else if d.growth == Growth::Luck {
        t.push_str("\nsame at every level (clamped to 0..255)");
    } else {
        t.push_str("\nsame at every level");
    }
    t
}
