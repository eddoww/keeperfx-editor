//! [`ScriptWizard`]: the egui form of the script wizard (see [`crate::wizard`]).
use crate::wizard::*;
use egui::{Color32, DragValue, RichText, Ui};
use kfx_script::ConstantProvider;

/// Sections of the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardTab {
    General,
    Rooms,
    Powers,
    TrapsDoors,
    Creatures,
    Alliances,
    Victory,
    Preview,
}

impl WizardTab {
    pub const ALL: [WizardTab; 8] = [
        WizardTab::General,
        WizardTab::Rooms,
        WizardTab::Powers,
        WizardTab::TrapsDoors,
        WizardTab::Creatures,
        WizardTab::Alliances,
        WizardTab::Victory,
        WizardTab::Preview,
    ];

    pub fn title(self) -> &'static str {
        match self {
            WizardTab::General => "General",
            WizardTab::Rooms => "Rooms",
            WizardTab::Powers => "Powers",
            WizardTab::TrapsDoors => "Traps & doors",
            WizardTab::Creatures => "Creatures",
            WizardTab::Alliances => "Alliances",
            WizardTab::Victory => "Victory",
            WizardTab::Preview => "Preview",
        }
    }
}

/// What the form asks of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardEvent {
    /// Write the block: compute [`ScriptWizard::plan_for`] on the current script text and apply
    /// it (for an open editor, [`WizardPlan::apply_to_editor`]: one undo step).
    Apply,
    /// The user closed the form.
    Close,
}

/// The wizard form. Holds the model and a copy of the script it plans against
/// ([`ScriptWizard::set_script`] keeps it current).
pub struct ScriptWizard {
    lang: ScriptLang,
    script: Vec<u8>,
    model: WizardModel,
    tab: WizardTab,
    shown: [bool; 8],
    plan: Option<WizardPlan>,
}

const COL_W: f32 = 78.0;

impl ScriptWizard {
    /// A form for `script` (file bytes) of language `lang`, starting from what it already sets.
    pub fn new(lang: ScriptLang, script: &[u8]) -> ScriptWizard {
        let model = WizardModel::read_as(lang, script);
        let mut w = ScriptWizard {
            lang,
            script: script.to_vec(),
            model,
            tab: WizardTab::General,
            shown: [false; 8],
            plan: None,
        };
        w.default_shown();
        w
    }

    fn default_shown(&mut self) {
        // players the script names on their own (ALL_PLAYERS lines reach every slot and do not
        // count)
        for p in Player::ALL {
            let s = self.model.player(p);
            let used = s.computer.is_some()
                || s.start_money.is_some()
                || s.max_creatures.is_some()
                || s.win_when_enemies_destroyed
                || s.lose_when_heart_destroyed
                || self.model.alliances.keys().any(|&(a, b)| a == p || b == p);
            self.shown[p.index()] = used || matches!(p, Player::P0 | Player::P1);
        }
    }

    pub fn lang(&self) -> ScriptLang {
        self.lang
    }

    pub fn model(&self) -> &WizardModel {
        &self.model
    }

    /// Change the model directly (hosts, tests); the plan follows.
    pub fn model_mut(&mut self) -> &mut WizardModel {
        self.plan = None;
        &mut self.model
    }

    pub fn tab(&self) -> WizardTab {
        self.tab
    }

    pub fn set_tab(&mut self, tab: WizardTab) {
        self.tab = tab;
    }

    /// Show or hide a player's column.
    pub fn set_player_shown(&mut self, p: Player, on: bool) {
        self.shown[p.index()] = on;
    }

    pub fn player_shown(&self, p: Player) -> bool {
        self.shown[p.index()]
    }

    /// The script text changed (the user typed, the block was applied): plan against `script`
    /// from now on. The form keeps its values.
    pub fn set_script(&mut self, script: &[u8]) {
        if script != self.script.as_slice() {
            self.script = script.to_vec();
            self.plan = None;
        }
    }

    /// Throw the form's values away and read them from the script again.
    pub fn reread(&mut self) {
        self.model = WizardModel::read_as(self.lang, &self.script);
        self.plan = None;
    }

    /// The plan against the script last given to [`ScriptWizard::new`] /
    /// [`ScriptWizard::set_script`].
    pub fn plan(&mut self) -> &WizardPlan {
        if self.plan.is_none() {
            self.plan = Some(self.model.plan_as(self.lang, &self.script));
        }
        self.plan.as_ref().unwrap()
    }

    /// The plan against `script` (use the editor's current text when applying).
    pub fn plan_for(&self, script: &[u8]) -> WizardPlan {
        self.model.plan_as(self.lang, script)
    }

    fn players(&self) -> Vec<Player> {
        Player::ALL
            .iter()
            .copied()
            .filter(|p| {
                self.shown[p.index()]
                    && (self.model.version > 0 || self.lang == ScriptLang::Lua || p.in_dk1())
            })
            .collect()
    }

    /// Draw the form into `ui`. `consts` supplies the room, power, trap, door and creature names
    /// (stock names when it has none).
    pub fn show(&mut self, ui: &mut Ui, consts: &dyn ConstantProvider) -> Option<WizardEvent> {
        let before = self.model.clone();
        let mut event = None;
        let v1 = self.model.version > 0 || self.lang == ScriptLang::Lua;
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(match self.lang {
                    ScriptLang::Dk => {
                        if v1 {
                            "DK script, LEVEL_VERSION 1"
                        } else {
                            "DK script, level version 0 (original rules)"
                        }
                    }
                    ScriptLang::Lua => "Lua script",
                })
                .strong(),
            );
            ui.separator();
            ui.label("Players:");
            for p in Player::ALL {
                let ok = v1 || p.in_dk1();
                let mut on = self.shown[p.index()];
                let r = ui
                    .add_enabled(ok, egui::Checkbox::new(&mut on, p.short()))
                    .on_hover_text(p.name())
                    .on_disabled_hover_text("Needs LEVEL_VERSION(1)");
                if r.changed() {
                    self.shown[p.index()] = on;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for t in WizardTab::ALL {
                if ui.selectable_label(self.tab == t, t.title()).clicked() {
                    self.tab = t;
                }
            }
        });
        ui.separator();
        let footer_h = 128.0;
        let body_h = (ui.available_height() - footer_h).max(120.0);
        egui::ScrollArea::both()
            .id_salt("wizard-body")
            .max_height(body_h)
            .min_scrolled_height(body_h)
            .auto_shrink([false, false])
            .show(ui, |ui| match self.tab {
                WizardTab::General => self.general(ui),
                WizardTab::Rooms => self.avail_grid(ui, consts, ItemKind::Room),
                WizardTab::Powers => self.avail_grid(ui, consts, ItemKind::Power),
                WizardTab::TrapsDoors => {
                    self.stock_grid(ui, consts, ItemKind::Trap);
                    ui.add_space(10.0);
                    self.stock_grid(ui, consts, ItemKind::Door);
                }
                WizardTab::Creatures => self.creatures(ui, consts),
                WizardTab::Alliances => self.alliances(ui),
                WizardTab::Victory => self.victory(ui),
                WizardTab::Preview => self.preview(ui),
            });
        if self.model != before {
            self.plan = None;
        }
        ui.separator();
        // footer: conflicts and notes, buttons
        let script = self.script.clone();
        let plan = self.plan().clone();
        let warn = ui.visuals().warn_fg_color;
        egui::ScrollArea::vertical()
            .id_salt("wizard-conflicts")
            .max_height(footer_h - 44.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if plan.conflicts.is_empty() && plan.notes.is_empty() {
                    ui.weak("No conflicts with the rest of the script.");
                }
                // one row per conflict; the full text is in the tooltip
                for c in &plan.conflicts {
                    ui.horizontal(|ui| {
                        ui.colored_label(warn, format!("line {:<4}", c.line));
                        let full = format!("{}   {}: {}", c.text, c.what, c.message);
                        ui.add(egui::Label::new(RichText::new(&full).weak()).truncate())
                            .on_hover_text(&full);
                    });
                }
                for n in &plan.notes {
                    ui.add(egui::Label::new(RichText::new(n).color(warn)).truncate())
                        .on_hover_text(n);
                }
            });
        ui.horizontal(|ui| {
            let changes = plan.changes(&script);
            let label = if plan.lines.is_empty() && !plan.span.is_empty() {
                "Apply (remove block)"
            } else {
                "Apply"
            };
            if ui
                .add_enabled(changes, egui::Button::new(label))
                .on_hover_text("Write the wizard block into the script (one undo step)")
                .clicked()
            {
                event = Some(WizardEvent::Apply);
            }
            if ui
                .button("Re-read script")
                .on_hover_text("Discard the form's values and read them from the script again")
                .clicked()
            {
                self.reread();
            }
            if ui.button("Close").clicked() {
                event = Some(WizardEvent::Close);
            }
            ui.weak(format!(
                "{} conflict(s), block of {} line(s)",
                plan.conflicts.len(),
                plan.lines.len()
            ));
        });
        event
    }

    fn general(&mut self, ui: &mut Ui) {
        let players = self.players();
        let v1 = self.model.version > 0 || self.lang == ScriptLang::Lua;
        ui.horizontal(|ui| {
            ui.label("Portal speed (game turns between creatures, all players):");
            opt_num(
                ui,
                &mut self.model.generate_speed,
                400,
                1..=100_000,
                "Portal speed",
            );
        });
        ui.add_space(6.0);
        egui::Grid::new("wizard-general")
            .striped(true)
            .min_col_width(COL_W)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("");
                for p in &players {
                    ui.strong(p.short()).on_hover_text(p.name());
                }
                ui.end_row();
                ui.label("Computer player");
                for &p in &players {
                    let cur = self.model.player(p).computer;
                    let text = cur.map_or("human / none".to_string(), |c| c.label());
                    egui::ComboBox::from_id_salt(("wizard-ai", p))
                        .selected_text(text)
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            let s = &mut self.model.player_mut(p).computer;
                            ui.selectable_value(s, None, "Human / none (no COMPUTER_PLAYER)");
                            for (n, name) in COMPUTER_MODELS {
                                ui.selectable_value(
                                    s,
                                    Some(Computer::Model(n)),
                                    format!("{n}: {name}"),
                                );
                            }
                            if v1 {
                                ui.selectable_value(s, Some(Computer::Roaming), "ROAMING");
                                if self.lang == ScriptLang::Dk {
                                    ui.selectable_value(s, Some(Computer::Off), "OFF");
                                }
                            }
                        });
                }
                ui.end_row();
                ui.label("Start gold");
                for &p in &players {
                    ui.horizontal(|ui| {
                        opt_num(
                            ui,
                            &mut self.model.player_mut(p).start_money,
                            2500,
                            0..=1_000_000_000,
                            &format!("Start gold {}", p.name()),
                        )
                    });
                }
                ui.end_row();
                ui.label("Max creatures");
                for &p in &players {
                    ui.horizontal(|ui| {
                        opt_num(
                            ui,
                            &mut self.model.player_mut(p).max_creatures,
                            25,
                            0..=255,
                            &format!("Max creatures {}", p.name()),
                        )
                    });
                }
                ui.end_row();
            });
        ui.add_space(8.0);
        ui.weak(
            "A ticked box means the wizard block sets the value. \
                Computer models come from keepcompp.cfg.",
        );
    }

    fn avail_grid(&mut self, ui: &mut Ui, consts: &dyn ConstantProvider, kind: ItemKind) {
        let players = self.players();
        let names = item_names(kind, consts, &self.model);
        ui.weak("Click to cycle: (not set), No, Research, Yes.");
        egui::Grid::new(("wizard-avail", kind as u8))
            .striped(true)
            .min_col_width(COL_W)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("");
                for &p in &players {
                    ui.vertical(|ui| {
                        ui.strong(p.short()).on_hover_text(p.name());
                        ui.menu_button("Set all", |ui| {
                            for (label, v) in [
                                ("(not set)", None),
                                ("No", Some(Availability::Unavailable)),
                                ("Research", Some(Availability::Researchable)),
                                ("Yes", Some(Availability::Available)),
                            ] {
                                if ui.button(label).clicked() {
                                    let map = avail_map(self.model.player_mut(p), kind);
                                    for n in &names {
                                        match v {
                                            Some(v) => {
                                                map.insert(n.clone(), v);
                                            }
                                            None => {
                                                map.remove(n);
                                            }
                                        }
                                    }
                                    ui.close();
                                }
                            }
                        });
                    });
                }
                ui.end_row();
                for n in &names {
                    ui.label(pretty(n));
                    for &p in &players {
                        let map = avail_map(self.model.player_mut(p), kind);
                        let cur = map.get(n).copied();
                        let (text, fill) = avail_style(cur);
                        let b = egui::Button::new(RichText::new(text).color(Color32::WHITE))
                            .fill(fill)
                            .min_size(egui::vec2(COL_W, 0.0));
                        let r = ui
                            .add(b)
                            .accessible_name(format!("{n} {}", p.name()))
                            .on_hover_text(avail_hover(kind, cur));
                        if r.clicked() {
                            match Availability::cycle(cur) {
                                Some(v) => {
                                    map.insert(n.clone(), v);
                                }
                                None => {
                                    map.remove(n);
                                }
                            }
                        }
                    }
                    ui.end_row();
                }
            });
    }

    fn stock_grid(&mut self, ui: &mut Ui, consts: &dyn ConstantProvider, kind: ItemKind) {
        let players = self.players();
        let names = item_names(kind, consts, &self.model);
        ui.strong(if kind == ItemKind::Trap {
            "Traps"
        } else {
            "Doors"
        });
        ui.weak(
            "Click to cycle: (not set), No, Build. The number is how \
                many ready-made ones the player gets.",
        );
        egui::Grid::new(("wizard-stock", kind as u8))
            .striped(true)
            .min_col_width(COL_W)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("");
                for &p in &players {
                    ui.strong(p.short()).on_hover_text(p.name());
                }
                ui.end_row();
                for n in &names {
                    ui.label(pretty(n));
                    for &p in &players {
                        let s = self.model.player_mut(p);
                        let map = if kind == ItemKind::Trap {
                            &mut s.traps
                        } else {
                            &mut s.doors
                        };
                        ui.horizontal(|ui| {
                            let cur = map.get(n).copied();
                            let (text, fill) = match cur {
                                None => ("-", Color32::from_gray(70)),
                                Some(Stock {
                                    buildable: false, ..
                                }) => ("No", NO_FILL),
                                Some(Stock {
                                    buildable: true, ..
                                }) => ("Build", YES_FILL),
                            };
                            let b = egui::Button::new(RichText::new(text).color(Color32::WHITE))
                                .fill(fill)
                                .min_size(egui::vec2(48.0, 0.0));
                            if ui
                                .add(b)
                                .accessible_name(format!("{n} {}", p.name()))
                                .on_hover_text(
                                    "TRAP/DOOR_AVAILABLE: can be manufactured; ready-made amount",
                                )
                                .clicked()
                            {
                                match cur {
                                    None => {
                                        map.insert(
                                            n.clone(),
                                            Stock {
                                                buildable: false,
                                                amount: 0,
                                            },
                                        );
                                    }
                                    Some(Stock {
                                        buildable: false,
                                        amount,
                                    }) => {
                                        map.insert(
                                            n.clone(),
                                            Stock {
                                                buildable: true,
                                                amount,
                                            },
                                        );
                                    }
                                    Some(_) => {
                                        map.remove(n);
                                    }
                                }
                            }
                            if let Some(st) = map.get_mut(n) {
                                ui.add(DragValue::new(&mut st.amount).range(0..=255).speed(0.1))
                                    .accessible_name(format!("{n} {} amount", p.name()));
                            }
                        });
                    }
                    ui.end_row();
                }
            });
    }

    fn creatures(&mut self, ui: &mut Ui, consts: &dyn ConstantProvider) {
        let players = self.players();
        let names = item_names(ItemKind::Creature, consts, &self.model);
        let v1 = self.model.version > 0 || self.lang == ScriptLang::Lua;
        ui.weak(if v1 {
            "Pool: creatures waiting outside, shared by all portals. Per \
                player: (not set), No, Yes; the number is how many come without their rooms (forced)."
        } else {
            "Pool: creatures waiting outside, shared by all portals. Per \
                player: (not set), No, Yes."
        });
        egui::Grid::new("wizard-creatures")
            .striped(true)
            .min_col_width(COL_W)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("");
                ui.strong("Pool");
                for &p in &players {
                    ui.strong(p.short()).on_hover_text(p.name());
                }
                ui.end_row();
                for n in &names {
                    ui.label(pretty(n));
                    let mut cnt = self.model.pool.get(n).copied();
                    ui.horizontal(|ui| opt_num(ui, &mut cnt, 0, 0..=255, &format!("Pool {n}")));
                    match cnt {
                        Some(c) => {
                            self.model.pool.insert(n.clone(), c);
                        }
                        None => {
                            self.model.pool.remove(n);
                        }
                    }
                    for &p in &players {
                        let map = &mut self.model.player_mut(p).creatures;
                        ui.horizontal(|ui| {
                            let cur = map.get(n).copied();
                            let (text, fill) = match cur {
                                None => ("-", Color32::from_gray(70)),
                                Some(Attraction { allowed: false, .. }) => ("No", NO_FILL),
                                Some(Attraction { allowed: true, .. }) => ("Yes", YES_FILL),
                            };
                            let b = egui::Button::new(RichText::new(text).color(Color32::WHITE))
                                .fill(fill)
                                .min_size(egui::vec2(40.0, 0.0));
                            if ui
                                .add(b)
                                .accessible_name(format!("{n} {}", p.name()))
                                .on_hover_text("CREATURE_AVAILABLE: can come through the portal")
                                .clicked()
                            {
                                match cur {
                                    None => {
                                        map.insert(
                                            n.clone(),
                                            Attraction {
                                                allowed: false,
                                                forced: 0,
                                            },
                                        );
                                    }
                                    Some(Attraction {
                                        allowed: false,
                                        forced,
                                    }) => {
                                        map.insert(
                                            n.clone(),
                                            Attraction {
                                                allowed: true,
                                                forced,
                                            },
                                        );
                                    }
                                    Some(_) => {
                                        map.remove(n);
                                    }
                                }
                            }
                            if v1 {
                                if let Some(a) = map.get_mut(n).filter(|a| a.allowed) {
                                    ui.add(DragValue::new(&mut a.forced).range(0..=255).speed(0.1))
                                        .accessible_name(format!("{n} {} forced", p.name()))
                                        .on_hover_text(
                                            "Forced: come even without the rooms they want",
                                        );
                                }
                            }
                        });
                    }
                    ui.end_row();
                }
            });
    }

    fn alliances(&mut self, ui: &mut Ui) {
        let players = self.players();
        let v1 = self.model.version > 0 || self.lang == ScriptLang::Lua;
        ui.weak(if v1 {
            "ALLY_PLAYERS between two players; locked alliances cannot be changed in the game."
        } else {
            "Level version 0 can only make players allies."
        });
        egui::Grid::new("wizard-allies")
            .striped(true)
            .min_col_width(COL_W)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("");
                for &p in &players {
                    ui.strong(p.short()).on_hover_text(p.name());
                }
                ui.end_row();
                for (i, &a) in players.iter().enumerate() {
                    ui.strong(a.short()).on_hover_text(a.name());
                    for (j, &b) in players.iter().enumerate() {
                        if j <= i {
                            ui.label("");
                            continue;
                        }
                        let key = if a < b { (a, b) } else { (b, a) };
                        let cur = self.model.alliances.get(&key).copied();
                        let text = cur.map_or("-".to_string(), |s| {
                            ALLIANCE_STATES
                                .iter()
                                .find(|x| x.0 == s)
                                .map_or(s.to_string(), |x| x.1.to_string())
                        });
                        let mut sel = cur;
                        egui::ComboBox::from_id_salt(("wizard-ally", a, b))
                            .selected_text(text)
                            .width(COL_W + 40.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut sel, None, "-");
                                for (s, label) in ALLIANCE_STATES {
                                    if v1 || s == 1 {
                                        ui.selectable_value(&mut sel, Some(s), label);
                                    }
                                }
                            });
                        if sel != cur {
                            match sel {
                                Some(s) => {
                                    self.model.alliances.insert(key, s);
                                }
                                None => {
                                    self.model.alliances.remove(&key);
                                }
                            }
                        }
                    }
                    ui.end_row();
                }
            });
    }

    fn victory(&mut self, ui: &mut Ui) {
        let players: Vec<Player> = self
            .players()
            .into_iter()
            .filter(|p| p.is_keeper())
            .collect();
        ui.weak(
            "The usual conditions. When one is met, the game applies the \
                outcome to the human players.",
        );
        egui::Grid::new("wizard-victory")
            .striped(true)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                for p in players {
                    let s = self.model.player_mut(p);
                    ui.strong(format!("{} ({})", p.short(), p.name()));
                    ui.checkbox(
                        &mut s.win_when_enemies_destroyed,
                        "Win when all enemy dungeons are destroyed",
                    )
                    .on_hover_text(format!(
                        "IF({}, ALL_DUNGEONS_DESTROYED == 1) WIN_GAME",
                        p.name()
                    ));
                    ui.checkbox(
                        &mut s.lose_when_heart_destroyed,
                        "Lose when its own dungeon heart is destroyed",
                    )
                    .on_hover_text(format!(
                        "IF({}, DUNGEON_DESTROYED == 1) LOSE_GAME",
                        p.name()
                    ));
                    ui.end_row();
                }
            });
    }

    fn preview(&mut self, ui: &mut Ui) {
        let plan = self.plan().clone();
        if plan.lines.is_empty() {
            ui.weak("Nothing to write: the script already says everything the form sets.");
            return;
        }
        let at = if plan.span.is_empty() {
            format!("inserted at byte {}", plan.span.start)
        } else {
            format!("replaces bytes {}..{}", plan.span.start, plan.span.end)
        };
        ui.weak(format!("The block ({at}):"));
        let mut text = plan.lines.join("\n");
        ui.add(
            egui::TextEdit::multiline(&mut text)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .interactive(false),
        );
    }
}

const NO_FILL: Color32 = Color32::from_rgb(150, 52, 52);
const RESEARCH_FILL: Color32 = Color32::from_rgb(150, 112, 30);
const YES_FILL: Color32 = Color32::from_rgb(46, 120, 60);
const OTHER_FILL: Color32 = Color32::from_rgb(70, 82, 140);

fn avail_map(
    s: &mut PlayerSetup,
    kind: ItemKind,
) -> &mut std::collections::BTreeMap<String, Availability> {
    match kind {
        ItemKind::Power => &mut s.powers,
        _ => &mut s.rooms,
    }
}

fn avail_style(a: Option<Availability>) -> (String, Color32) {
    match a {
        None => ("-".into(), Color32::from_gray(70)),
        Some(Availability::Unavailable) => ("No".into(), NO_FILL),
        Some(Availability::Researchable) => ("Research".into(), RESEARCH_FILL),
        Some(Availability::Available) => ("Yes".into(), YES_FILL),
        Some(o) => (o.label(), OTHER_FILL),
    }
}

fn avail_hover(kind: ItemKind, a: Option<Availability>) -> String {
    let cmd = if kind == ItemKind::Power {
        "MAGIC_AVAILABLE"
    } else {
        "ROOM_AVAILABLE"
    };
    match a {
        None => format!(
            "Not set by the wizard (the game default, or what other \
                lines of the script say). {cmd}"
        ),
        Some(a) => {
            let (r, x) = a.args();
            let what = match a {
                Availability::Unavailable => "never available",
                Availability::Researchable => "can be researched",
                Availability::Available => "available from the start",
                Availability::Other { .. } => "special state kept as written",
            };
            format!("{what}: {cmd}(player, name, {r}, {x})")
        }
    }
}

/// `DARK_MISTRESS` -> `Dark mistress`, `POWER_CALL_TO_ARMS` -> `Call to arms`.
fn pretty(n: &str) -> String {
    let n = n.strip_prefix("POWER_").unwrap_or(n);
    let mut s: String = n.to_ascii_lowercase().replace('_', " ");
    if let Some(f) = s.get(0..1) {
        let up = f.to_ascii_uppercase();
        s.replace_range(0..1, &up);
    }
    s
}

/// A checkbox (set by the wizard or not) and the value. Editing the value ticks the box.
fn opt_num(
    ui: &mut Ui,
    val: &mut Option<i64>,
    default: i64,
    range: std::ops::RangeInclusive<i64>,
    name: &str,
) {
    let mut on = val.is_some();
    let mut v = val.unwrap_or(default);
    let c = ui
        .checkbox(&mut on, "")
        .accessible_name(format!("{name} set"))
        .on_hover_text("Set by the wizard");
    let speed = ((*range.end() - *range.start()) as f64 / 2000.0).clamp(0.05, 50.0);
    let r = ui.add_enabled_ui(true, |ui| {
        if !on {
            ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
        }
        ui.add(DragValue::new(&mut v).range(range).speed(speed))
            .accessible_name(name.to_string())
    });
    if r.inner.changed() {
        *val = Some(v);
    } else if c.changed() {
        *val = if on { Some(v) } else { None };
    }
}
