//! The Events editor: the visual level logic editor of `kfx-logic-ui` as a tab of the script
//! workspace, over the same buffer as the DK script text tab.
//!
//! Hand-off between the two views of the one script:
//!
//! * The text of the DK script tab is the single source of truth.
//! * The Events model is (re)built from the text whenever the text changed since the model was
//!   built or last written (the text editor's generation is compared): typing, undo in the
//!   text tab, a wizard block, the renumbering prompt all show up in Events.
//! * Every Events change (any edit, its own undo or redo) is exported into the text at once,
//!   as one replacement of the changed lines (one text undo step), so the text tab, saving,
//!   validation, markers and play-testing always see it.
//! * Undo: the Events toolbar (and Ctrl+Z / Ctrl+Y over the Events tab) steps through the
//!   Events edits made since the model was last rebuilt; Ctrl+Z in the text tab undoes
//!   everything, Events edits included (the Events model follows). A rebuild from the text
//!   clears the Events undo list, never the text's.
//! * Scripts the model cannot represent faithfully are view-only in Events (banner); they stay
//!   editable as text.
//!
//! Map side: picks use the editor's pick-from-map mode, the selected event's locations are
//! ringed on the map, party routes are drawn from the spawn place to the target in the 3D and
//! 2D views, portraits come from the thing palette, strengths from the level's creature
//! configs, string ids from the level's string tables.
use super::*;
use kfx_logic_ui::analysis::{LocRole, MapLoc};
use kfx_logic_ui::{LogicEditor, LogicEvent, LogicHost, PickRequest, View};
use kfx_prefab::i18n::{trd, trf, trn};
use kfx_script::{ConstantProvider, Span};
use kfx_script_ui::MapTarget;
use serde::{Deserialize, Serialize};

/// Label of the Events tab in the script workspace.
pub const EVENTS_TAB: &str = "Events";

/// Persisted state of the Events editor (with the layout).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LogicSettings {
    /// The Events tab was the shown tab of the script workspace.
    pub events_tab: bool,
    /// The Events view (`cards`, `graph`, `timeline`, `parties`, `variables`, `lua`).
    pub view: String,
}

impl Default for LogicSettings {
    fn default() -> Self {
        LogicSettings {
            events_tab: false,
            view: "cards".into(),
        }
    }
}

/// The view's settings name.
pub fn view_name(v: View) -> &'static str {
    match v {
        View::Cards => "cards",
        View::Graph => "graph",
        View::Timeline => "timeline",
        View::Parties => "parties",
        View::Variables => "variables",
        View::Lua => "lua",
    }
}

pub fn view_by_name(s: &str) -> Option<View> {
    View::ALL
        .into_iter()
        .find(|v| view_name(*v) == s.to_ascii_lowercase() || v.label().eq_ignore_ascii_case(s))
}

/// A party route drawn on the map.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub party: String,
    pub spawn: Option<MapLoc>,
    /// Where it heads (tunnellers); `None`: the nearest keeper heart, drawn dashed.
    pub head_for: Option<MapLoc>,
}

/// The Events editor of the open level.
pub struct LogicState {
    pub ed: LogicEditor,
    key: (PathBuf, u32),
    /// Generation of the DK text editor the model matches (`None`: never built).
    synced: Option<u64>,
    /// A map pick asked by the Events editor.
    pick: Option<PickRequest>,
    /// String picker: the request and the search text.
    string_pick: Option<(PickRequest, String)>,
    /// Locations of the selected event.
    pub highlight: Vec<MapLoc>,
    pub routes: Vec<Route>,
    /// Experience growth rates, per GameData (pointer).
    rates: Option<(usize, kfx_config_ui::creature::ExpRates)>,
    /// Where the Events tab was drawn last frame (keys over it go to the Events editor).
    rect: Option<egui::Rect>,
}

/// What the logic editor gets from the editor.
struct Host<'a> {
    consts: &'a crate::scripts::MapConsts,
    portraits: HashMap<String, egui::TextureId>,
    gd: Option<Arc<GameData>>,
    rates: kfx_config_ui::creature::ExpRates,
    strings: Arc<std::sync::RwLock<Option<kfx_formats::strings::StringResolver>>>,
}

impl LogicHost for Host<'_> {
    fn provider(&self) -> &dyn ConstantProvider {
        self.consts
    }
    fn creature_portrait(&self, creature: &str) -> Option<egui::TextureId> {
        self.portraits.get(&creature.to_ascii_uppercase()).copied()
    }
    fn creature_strength(&self, creature: &str, level: u32) -> Option<f32> {
        creature_score(self.gd.as_deref()?, &self.rates, creature, level)
    }
    fn string_text(&self, id: i64) -> Option<String> {
        self.strings
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|r| r.get(id)))
            .filter(|t| !t.trim().is_empty())
    }
}

/// `compute_creature_kind_score` (src/thing_stats.c:347): health + defense + dexterity + armour +
/// strength at experience `level` (1-based as in scripts; the game's level index is one less),
/// from the layered creature config of the level.
pub fn creature_score(
    gd: &GameData,
    rates: &kfx_config_ui::creature::ExpRates,
    creature: &str,
    level: u32,
) -> Option<f32> {
    use kfx_config_ui::creature::Growth as G;
    let c = gd.creature_config(creature)?;
    let a = &c.attributes;
    let l = level.clamp(1, 10) as i64 - 1;
    let s = G::Health.at_level(a.health as i64, l, rates)
        + G::Defence.at_level(a.defence as i64, l, rates)
        + G::Dexterity.at_level(a.dexterity as i64, l, rates)
        + G::Armour.at_level(a.armour as i64, l, rates)
        + G::Strength.at_level(a.strength as i64, l, rates);
    Some(s as f32)
}

/// `[experience]` rates of `creature.cfg` and the `OVERFLOW_8BIT` classic bug of `rules.cfg`
/// through the level's layers (defaults where absent, src/config_creature.c:747-765).
fn exp_rates(gd: &GameData) -> kfx_config_ui::creature::ExpRates {
    let mut r = kfx_config_ui::creature::ExpRates::default();
    let num = |v: Option<&str>| {
        v.and_then(|s| s.split_whitespace().next())
            .and_then(kfx_map::gamedata::parse_number)
    };
    if let Some(f) = gd.creature_cfg.section("experience") {
        let g = |k: &str, d: i64| num(f.get(k)).unwrap_or(d);
        r.health = g("HealthIncreaseOnExp", r.health);
        r.strength = g("StrengthIncreaseOnExp", r.strength);
        r.dexterity = g("DexterityIncreaseOnExp", r.dexterity);
        r.defense = g("DefenseIncreaseOnExp", r.defense);
        r.armour = g("ArmourIncreaseOnExp", r.armour);
    }
    if let Ok(rules) = gd.layered_cfg("rules.cfg") {
        r.overflow_8bit = rules.value("game", "PreserveClassicBugs").is_some_and(|v| {
            v.split_whitespace()
                .any(|t| t.eq_ignore_ascii_case("OVERFLOW_8BIT"))
        });
    }
    r
}

/// The parameters of a wave added with [`EditorApp::events_add_wave`] (the `events_add_wave`
/// action of `--actions`, see docs/manual/cli.md).
#[derive(Clone, Debug, PartialEq)]
pub struct WaveSpec {
    /// An existing party, or the name of the party `creatures` creates (default: the next free
    /// `PARTYn`).
    pub party: Option<String>,
    /// Members of a new party: (creature, level 1..10, gold).
    pub creatures: Vec<(String, u32, u32)>,
    /// Objective of the new members (`ATTACK_ENEMIES` by default).
    pub objective: String,
    /// `PLAYER_GOOD` (heroes) or a keeper.
    pub owner: String,
    /// Location argument: `-n` hero gate n, `n` action point n, or `PLAYERn` (heart).
    pub spawn: String,
    /// When: `GAME_TURN >= turn` of `clock_player` ...
    pub turn: Option<u32>,
    /// ... or a condition `IF(player, variable op value)`.
    pub condition: Option<(String, String, String, String)>,
    pub clock_player: String,
    pub copies: u32,
    /// Waves (turn waves only), `interval` turns apart, each `copies_step` more copies.
    pub count: u32,
    pub interval: u32,
    pub copies_step: u32,
    /// A tunneller party digging towards (`DUNGEON_HEART` / `ACTION_POINT` / ..., target).
    pub tunneller: Option<(String, String)>,
}

impl Default for WaveSpec {
    fn default() -> Self {
        WaveSpec {
            party: None,
            creatures: vec![],
            objective: "ATTACK_ENEMIES".into(),
            owner: "PLAYER_GOOD".into(),
            spawn: "-1".into(),
            turn: None,
            condition: None,
            clock_player: "PLAYER0".into(),
            copies: 1,
            count: 1,
            interval: 3000,
            copies_step: 0,
            tunneller: None,
        }
    }
}

/// Owner index of a player name (`PLAYER0`.., `PLAYER_GOOD`, `PLAYER_NEUTRAL`).
fn player_index(p: &str) -> Option<u8> {
    let p = p.trim().to_ascii_uppercase();
    match p.as_str() {
        "PLAYER_GOOD" => Some(4),
        "PLAYER_NEUTRAL" => Some(5),
        _ => p.strip_prefix("PLAYER").and_then(|n| n.parse().ok()),
    }
}

/// World position of a script location on the map.
fn loc_pos(doc: &kfx_map::mapdoc::MapDoc, gd: &GameData, l: &MapLoc) -> Option<Vec3> {
    let t = match l {
        MapLoc::ActionPoint(n) => MapTarget::ActionPoint(*n),
        MapLoc::HeroGate(n) => MapTarget::HeroGate(*n),
        MapLoc::Subtile { x, y } => MapTarget::Subtile { x: *x, y: *y },
        MapLoc::Slab { x, y } => MapTarget::Slab { x: *x, y: *y },
        MapLoc::Heart(p) => {
            let (sx, sy) = crate::fortify::heart_of(doc, gd, player_index(p)?)?;
            let (x, z) = (sx as f32 * 3.0 + 1.5, sy as f32 * 3.0 + 1.5);
            return Some(Vec3::new(x, doc.floor_height(x as i32, z as i32) as f32, z));
        }
    };
    crate::scripts::target_pos(doc, gd, t)
}

/// The keeper hearts of the map (owner, position).
fn hearts(doc: &kfx_map::mapdoc::MapDoc, gd: &GameData) -> Vec<(u8, Vec3)> {
    (0..4u8)
        .filter_map(|o| loc_pos(doc, gd, &MapLoc::Heart(format!("PLAYER{o}"))).map(|p| (o, p)))
        .collect()
}

/// Replace the changed lines of `old` with those of `new`: (span of `old`, replacement bytes).
/// Both ends are moved to line starts so that a CR LF pair is never split.
fn line_diff(old: &[u8], new: &[u8]) -> Option<(Span, Vec<u8>)> {
    if old == new {
        return None;
    }
    let mut p = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    while p > 0 && old[p - 1] != b'\n' {
        p -= 1;
    }
    let max_s = (old.len() - p).min(new.len() - p);
    let mut s = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_s)
        .take_while(|(a, b)| a == b)
        .count();
    // the suffix starts right after a newline (or is the whole rest)
    while s > 0 && old[old.len() - s - 1] != b'\n' {
        s -= 1;
    }
    Some((Span::new(p, old.len() - s), new[p..new.len() - s].to_vec()))
}

impl EditorApp {
    /// The Events editor exists for the open level, its tab is registered in the workspace.
    fn ensure_logic(&mut self) {
        let Some(s) = self.scripts.as_mut() else {
            self.logic = None;
            return;
        };
        let key = (s.files.folder.clone(), s.files.level);
        let dk = s.ws.dk_name();
        // a new workspace (other level, Save As) gets the tab and a new model
        let new_ws = s.ws.set_host_tab(
            EVENTS_TAB,
            &dk,
            &trd(
                "kfx_editor.the_level_s_events_as_cards_a_graph_and_a_timeli",
                "The level's events as cards, a graph and a timeline, parties and waves: \
                the same DK script as the text tab.",
            ),
        );
        if !new_ws && self.logic.as_ref().is_some_and(|l| l.key == key) {
            return;
        }
        let mut ed = LogicEditor::new();
        // the Lua export knows the campaign's and the map's Lua libraries, like the Lua tab
        if let Some(d) = &self.doc {
            ed.set_lua_index(crate::scripts::lua_index(&d.doc.folder, &d.layers));
        }
        if let Some(v) = view_by_name(&self.settings.logic.view) {
            ed.set_view(v);
        }
        if self.settings.logic.events_tab {
            s.ws.activate_host_tab(EVENTS_TAB);
        }
        self.logic = Some(LogicState {
            ed,
            key,
            synced: None,
            pick: None,
            string_pick: None,
            highlight: vec![],
            routes: vec![],
            rates: None,
            rect: None,
        });
    }

    /// Rebuild the Events model when the DK text changed since it was built or written.
    fn logic_sync(&mut self) {
        // the map's action point / hero gate numbers into the script constants first: they are
        // otherwise refreshed once per frame, so headless actions (a prefab that adds a hero gate,
        // then a wave from it) would see the numbers of before the placement
        self.update_scripts();
        self.ensure_logic();
        let (Some(s), Some(l)) = (self.scripts.as_mut(), self.logic.as_mut()) else {
            return;
        };
        let dk = s.ws.dk_name();
        let Some(e) = s.ws.editor_mut(&dk) else {
            return;
        };
        e.apply_pending();
        let gen = e.generation();
        if l.synced == Some(gen) {
            return;
        }
        let sel = l.ed.selected();
        let view = l.ed.view();
        l.ed.load(&e.text_bytes());
        l.ed.set_view(view);
        // keep the selection where the node still exists (ids are positional on import)
        if let Some(id) = sel.filter(|id| l.ed.enclosing_event(*id).is_some()) {
            l.ed.select(id);
        }
        l.synced = Some(gen);
    }

    /// Export the Events model into the DK text (one replacement of the changed lines).
    fn logic_write_back(&mut self) {
        let (Some(s), Some(l)) = (self.scripts.as_mut(), self.logic.as_mut()) else {
            return;
        };
        let dk = s.ws.dk_name();
        let new = l.ed.script_bytes();
        let Some(e) = s.ws.editor_mut(&dk) else {
            return;
        };
        e.apply_pending();
        let old = e.text_bytes();
        if let Some((span, bytes)) = line_diff(&old, &new) {
            // the text before the change is an undo point of the text tab
            e.checkpoint_undo(&self.ctx);
            let text = kfx_script_ui::latin1::to_buffer_text(&bytes, e.line_ending());
            if e.replace_spans(&[(span, text)]) {
                e.apply_pending();
            } else {
                self.status = trd(
                    "kfx_editor.the_dk_script_is_read_only_the_events_change_was",
                    "The DK script is read-only: the Events change was not written.",
                );
            }
        }
        l.synced = Some(e.generation());
        self.issues_rev = u64::MAX;
        self.logic_refresh_marks(false);
    }

    /// Map marks of the selected event, again (its locations may have changed).
    fn logic_refresh_marks(&mut self, flash: bool) {
        let Some(l) = self.logic.as_ref() else { return };
        let ev = l.ed.selected().and_then(|s| l.ed.enclosing_event(s));
        let locations = ev
            .and_then(|e| l.ed.doc().event(e))
            .map(|e| kfx_logic_ui::analysis::event_locations(l.ed.doc(), e))
            .unwrap_or_default();
        self.logic_marks(ev, &locations, flash);
    }

    /// Highlight `locations` (of event `event`) and the routes of the parties it spawns.
    fn logic_marks(
        &mut self,
        event: Option<kfx_logic_ui::NodeId>,
        locations: &[kfx_logic_ui::analysis::LocRef],
        flash: bool,
    ) {
        let Some(l) = self.logic.as_mut() else { return };
        l.highlight = locations.iter().map(|r| r.loc.clone()).collect();
        l.highlight.dedup();
        l.routes.clear();
        for r in locations
            .iter()
            .filter(|r| r.role == LocRole::SpawnAt && r.command.contains("PARTY"))
        {
            let party = event.and_then(|e| l.ed.doc().event(e)).and_then(|e| {
                e.body.iter().find_map(|n| {
                    if let kfx_logic_ui::Node::Action(a) = n {
                        (a.id == r.node).then(|| a.cmd.arg(1).to_string())
                    } else {
                        None
                    }
                })
            });
            let head = locations
                .iter()
                .find(|h| h.node == r.node && h.role == LocRole::HeadFor)
                .map(|h| h.loc.clone());
            l.routes.push(Route {
                party: party.unwrap_or_default(),
                spawn: Some(r.loc.clone()),
                head_for: head,
            });
        }
        let first = l.highlight.first().cloned();
        if let (true, Some(loc), Some(d)) = (flash, first, self.doc.as_ref()) {
            if let Some(p) = loc_pos(&d.doc, &d.gd, &loc) {
                self.flash = Some((p, Instant::now()));
            }
        }
    }

    /// Experience rates of the open level's configs (cached per GameData).
    fn logic_rates(&mut self) -> kfx_config_ui::creature::ExpRates {
        let gd = self.doc.as_ref().map(|d| d.gd.clone());
        if let (Some(l), Some(g)) = (self.logic.as_mut(), gd.as_ref()) {
            let key = Arc::as_ptr(g) as usize;
            if l.rates.as_ref().map(|r| r.0) != Some(key) {
                l.rates = Some((key, exp_rates(g)));
            }
        }
        self.logic
            .as_ref()
            .and_then(|l| l.rates.map(|r| r.1))
            .unwrap_or_default()
    }

    fn logic_host(&self, rates: kfx_config_ui::creature::ExpRates) -> Option<Host<'_>> {
        let gd = self.doc.as_ref().map(|d| d.gd.clone());
        let portraits = self
            .thing_palette
            .iter()
            .filter(|e| matches!(e.kind, PlaceKind::Creature(_)))
            .filter_map(|e| {
                e.tex
                    .as_ref()
                    .map(|t| (e.label.to_ascii_uppercase(), t.id()))
            })
            .collect();
        let consts = &self.scripts.as_ref()?.consts;
        Some(Host {
            consts,
            portraits,
            gd,
            rates,
            strings: self.string_shared.clone(),
        })
    }

    /// The Events tab is shown (script workspace visible with the Events tab active).
    pub fn events_shown(&self) -> bool {
        (self.settings.script_dock != crate::settings::ScriptDock::Hidden || self.script_max)
            && self
                .scripts
                .as_ref()
                .is_some_and(|s| s.ws.host_tab_active() == Some(EVENTS_TAB))
    }

    /// The pointer is over the Events tab: map shortcuts stay off (Ctrl+Z is the Events undo).
    pub(super) fn logic_owns_keys(&self, ctx: &egui::Context) -> bool {
        self.events_shown()
            && self
                .logic
                .as_ref()
                .and_then(|l| l.rect)
                .is_some_and(|r| ctx.pointer_hover_pos().is_some_and(|p| r.contains(p)))
    }

    /// Draw the Events tab when it is the shown host tab (called after the workspace).
    pub(super) fn logic_tab_ui(&mut self, ui: &mut egui::Ui) {
        self.ensure_logic();
        let rect = self.scripts.as_ref().and_then(|s| s.ws.host_rect());
        let active = self
            .scripts
            .as_ref()
            .is_some_and(|s| s.ws.host_tab_active() == Some(EVENTS_TAB));
        self.settings.logic.events_tab = active;
        if let Some(l) = self.logic.as_mut() {
            l.rect = rect;
        }
        let Some(rect) = rect.filter(|_| active) else {
            return;
        };
        self.logic_sync();
        // Ctrl+Z / Ctrl+Y over the tab: the Events undo (map shortcuts are off there)
        let over = ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|p| rect.contains(p))
            && !ui.ctx().egui_wants_keyboard_input();
        if over {
            let (undo, redo) = ui.ctx().input_mut(|i| {
                let redo = i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                ) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y);
                (i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z), redo)
            });
            if let Some(l) = self.logic.as_mut() {
                if undo {
                    l.ed.undo();
                }
                if redo {
                    l.ed.redo();
                }
            }
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child
            .painter()
            .rect_filled(rect, 0.0, child.visuals().panel_fill);
        let rates = self.logic_rates();
        let mut logic = self.logic.take();
        let events = match (&mut logic, self.logic_host(rates)) {
            (Some(l), Some(host)) => {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(&mut child, |ui| {
                        ui.label(
                            RichText::new(trd(
                                "kfx_editor.same_script_as_the_text_tab_every_change_here_is",
                                "Same script as the text tab: every change here is written \
                                into the DK script at once (Ctrl+Z in the text tab undoes it).",
                            ))
                            .small()
                            .color(TEXT_DIM),
                        );
                        l.ed.show(ui, &host)
                    })
                    .inner
            }
            _ => vec![],
        };
        self.logic = logic;
        if let Some(l) = &self.logic {
            self.settings.logic.view = view_name(l.ed.view()).to_string();
        }
        self.handle_logic_events(events);
    }

    fn handle_logic_events(&mut self, events: Vec<LogicEvent>) {
        let mut routes_reset = false;
        for e in events {
            match e {
                LogicEvent::Changed => self.logic_write_back(),
                LogicEvent::PickLocation { request, kind } => {
                    use kfx_logic_ui::PickKind as L;
                    use kfx_script_ui::PickKind as K;
                    let k = match kind {
                        L::ActionPoint => K::ActionPoint,
                        L::HeroGate => K::HeroGate,
                        L::Location => K::Location,
                        L::Subtile => K::Subtile,
                        L::Slab => K::Slab,
                    };
                    self.script_max = false;
                    self.start_pick(k);
                    if let Some(l) = self.logic.as_mut() {
                        l.pick = Some(request);
                    }
                }
                LogicEvent::PickString { request } => {
                    if let Some(l) = self.logic.as_mut() {
                        l.string_pick = Some((request, String::new()));
                    }
                }
                LogicEvent::Highlight { event, locations } => {
                    self.logic_marks(event, &locations, true)
                }
                LogicEvent::ShowRoute {
                    party,
                    spawn,
                    head_for,
                } => {
                    let Some(l) = self.logic.as_mut() else {
                        continue;
                    };
                    if !routes_reset {
                        l.routes.clear();
                        routes_reset = true;
                    }
                    l.routes.push(Route {
                        party,
                        spawn: spawn.clone(),
                        head_for,
                    });
                    if let (Some(loc), Some(d)) = (spawn, self.doc.as_ref()) {
                        if let Some(p) = loc_pos(&d.doc, &d.gd, &loc) {
                            self.camera.target = Vec3::new(p.x, self.camera.target.y, p.z);
                            self.flash = Some((p, Instant::now()));
                        }
                    }
                    self.script_max = false;
                }
                LogicEvent::OpenInText { line } => {
                    if let Some(dk) = self.scripts.as_ref().map(|s| s.ws.dk_name()) {
                        self.reveal_script_line(&dk, line);
                        self.status = trf(
                            "kfx_editor.dk_line_line",
                            "{dk} line {line}",
                            &[("dk", &dk), ("line", &line)],
                        );
                    }
                }
            }
        }
    }

    /// A map pick finished: the Events editor takes it when it asked. Returns true when taken.
    pub(super) fn logic_take_pick(&mut self, v: kfx_script_ui::PickValue) -> bool {
        use kfx_logic_ui::PickValue as L;
        use kfx_script_ui::PickValue as K;
        let Some(req) = self.logic.as_mut().and_then(|l| l.pick.take()) else {
            return false;
        };
        let value = match v {
            K::ActionPoint(n) => L::ActionPoint(n),
            K::HeroGate(n) => L::HeroGate(n),
            K::Subtile { x, y } => L::Subtile { x, y },
            K::Slab { x, y } => L::Slab { x, y },
        };
        self.logic_sync();
        if let Some(l) = self.logic.as_mut() {
            l.ed.answer_pick(req, value);
        }
        self.logic_write_back();
        true
    }

    /// Per-frame upkeep and the string picker window.
    pub(super) fn logic_windows(&mut self, ctx: &egui::Context) {
        // a cancelled pick (Esc) is forgotten
        if self.pick.is_none() {
            if let Some(l) = self.logic.as_mut() {
                l.pick = None;
            }
        }
        let Some((req, mut search)) = self.logic.as_ref().and_then(|l| l.string_pick.clone())
        else {
            return;
        };
        let rows: Vec<(usize, String)> = self
            .string_shared
            .read()
            .ok()
            .and_then(|g| {
                g.as_ref().map(|r| {
                    (0..kfx_formats::strings::STRINGS_MAX)
                        .filter_map(|i| {
                            // what the game shows for the id: level, campaign, then GUI table
                            let t = r.get(i as i64);
                            (!t.trim().is_empty() && !t.starts_with("untranslated <"))
                                .then_some((i, t))
                        })
                        .collect()
                })
            })
            .unwrap_or_default();
        let mut open = true;
        let mut chosen: Option<i64> = None;
        let mut create = false;
        egui::Window::new(trd("kfx_editor.pick_a_string", "Pick a string"))
            .open(&mut open)
            .default_size([520.0, 420.0])
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(trd("kfx_editor.search", "Search"));
                    ui.add(egui::TextEdit::singleline(&mut search).desired_width(260.0));
                    if ui
                        .button(trd("kfx_editor.new_level_string", "New level string"))
                        .on_hover_text(trd(
                            "kfx_editor.a_new_string_of_this_level_type_its_text_in_the",
                            "A new string of this level; type its text in the Strings window",
                        ))
                        .clicked()
                    {
                        create = true;
                    }
                });
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.the_texts_the_game_shows_for_these_ids_level_cam",
                        "The texts the game shows for these IDs (level, campaign, then the \
                        game's own strings). The ID is written into the selected argument.",
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
                ui.separator();
                let q = search.to_ascii_lowercase();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (i, t) in rows
                            .iter()
                            .filter(|(i, t)| {
                                q.is_empty()
                                    || i.to_string() == q
                                    || t.to_ascii_lowercase().contains(&q)
                            })
                            .take(400)
                        {
                            let short: String = t.chars().take(110).collect();
                            if ui
                                .selectable_label(false, format!("{i:>4}  {short}"))
                                .clicked()
                            {
                                chosen = Some(*i as i64);
                            }
                        }
                    });
            });
        if create {
            match self.new_level_string("New message") {
                Ok(id) => {
                    chosen = Some(id as i64);
                    self.show_strings = true;
                }
                Err(e) => self.status = e,
            }
        }
        if let Some(id) = chosen {
            self.logic_sync();
            if let Some(l) = self.logic.as_mut() {
                l.ed.answer_string(req, id);
                l.string_pick = None;
            }
            self.logic_write_back();
            return;
        }
        if let Some(l) = self.logic.as_mut() {
            l.string_pick = if open { Some((req, search)) } else { None };
        }
    }

    // ------------------------------------------------------------------------- map drawing

    /// Route lines of the Events editor into the overlay (3D and 2D views).
    pub(super) fn logic_overlay(&self, top: &mut Vec<LineVertex>) {
        if !self.events_shown() {
            return;
        }
        let (Some(l), Some(d)) = (&self.logic, &self.doc) else {
            return;
        };
        let hearts = hearts(&d.doc, &d.gd);
        let lv = |p: Vec3, c: [u8; 4]| LineVertex {
            pos: p.to_array(),
            color: c,
        };
        let ground = |x: f32, z: f32| d.doc.floor_height(x as i32, z as i32) as f32 + 0.35;
        for r in &l.routes {
            let Some(a) = r.spawn.as_ref().and_then(|s| loc_pos(&d.doc, &d.gd, s)) else {
                continue;
            };
            let (b, dashed) = match r.head_for.as_ref().and_then(|h| loc_pos(&d.doc, &d.gd, h)) {
                Some(b) => (b, false),
                None => match hearts
                    .iter()
                    .min_by(|x, y| (x.1 - a).length().total_cmp(&(y.1 - a).length()))
                {
                    Some((_, b)) => (*b, true),
                    None => continue,
                },
            };
            let col = if dashed {
                [255, 170, 60, 230]
            } else {
                [255, 110, 40, 255]
            };
            let dir = Vec2::new(b.x - a.x, b.z - a.z);
            let len = dir.length();
            if len < 0.5 {
                continue;
            }
            let n = (len / 1.5).ceil().max(1.0) as usize;
            let side = Vec2::new(-dir.y, dir.x) / len;
            let at = |t: f32, off: f32| {
                let x = a.x + dir.x * t + side.x * off;
                let z = a.z + dir.y * t + side.y * off;
                Vec3::new(x, ground(x, z), z)
            };
            for k in 0..n {
                if dashed && k % 2 == 1 {
                    continue;
                }
                let (t0, t1) = (k as f32 / n as f32, (k + 1) as f32 / n as f32);
                for off in [-0.24, -0.12, 0.0, 0.12, 0.24] {
                    top.push(lv(at(t0, off), col));
                    top.push(lv(at(t1, off), col));
                }
            }
            // arrow head at the target
            let tip = at(1.0, 0.0);
            let back = 1.6 / len;
            for s in [-0.9, 0.9] {
                top.push(lv(tip, col));
                top.push(lv(at(1.0 - back, s), col));
            }
        }
    }

    /// Rings and labels of the selected event's locations and the route labels.
    pub(super) fn draw_logic_marks(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        vp: Vec2,
        ppp: f32,
    ) {
        if !self.events_shown() {
            return;
        }
        let (Some(l), Some(d)) = (&self.logic, &self.doc) else {
            return;
        };
        let to_ui = |p: Vec2| rect.min + EVec2::new(p.x / ppp, p.y / ppp);
        let gold = Color32::from_rgb(255, 215, 90);
        for loc in &l.highlight {
            let Some(p) = loc_pos(&d.doc, &d.gd, loc).and_then(|w| self.camera.project(w, vp))
            else {
                continue;
            };
            let c = to_ui(p);
            if !rect.contains(c) {
                continue;
            }
            painter.circle_stroke(c, 16.0, egui::Stroke::new(3.0, gold));
            painter.circle_stroke(c, 22.0, egui::Stroke::new(1.5, gold.gamma_multiply(0.6)));
            let galley = painter.layout_no_wrap(
                loc.label(),
                egui::FontId::proportional(11.0),
                Color32::BLACK,
            );
            let r = egui::Rect::from_min_size(
                c + EVec2::new(-galley.size().x / 2.0 - 4.0, 24.0),
                galley.size() + EVec2::new(8.0, 2.0),
            );
            painter.rect_filled(r, 3.0, gold);
            painter.galley(r.min + EVec2::new(4.0, 1.0), galley, Color32::BLACK);
        }
        let orange = Color32::from_rgb(255, 130, 50);
        for r in &l.routes {
            let Some(p) = r
                .spawn
                .as_ref()
                .and_then(|s| loc_pos(&d.doc, &d.gd, s))
                .and_then(|w| self.camera.project(w, vp))
            else {
                continue;
            };
            let c = to_ui(p);
            if !rect.contains(c) {
                continue;
            }
            let text = if r.head_for.is_some() {
                format!("{} ->", r.party)
            } else {
                trf(
                    "kfx_editor.party_nearest_heart",
                    "{party} -> nearest heart",
                    &[("party", &r.party)],
                )
            };
            let galley =
                painter.layout_no_wrap(text, egui::FontId::proportional(11.5), Color32::WHITE);
            let br = egui::Rect::from_min_size(
                c + EVec2::new(-galley.size().x / 2.0 - 5.0, -44.0),
                galley.size() + EVec2::new(10.0, 3.0),
            );
            painter.rect(
                br,
                4.0,
                Color32::from_rgba_unmultiplied(80, 30, 5, 230),
                egui::Stroke::new(1.0, orange),
                egui::StrokeKind::Inside,
            );
            painter.galley(br.min + EVec2::new(5.0, 1.5), galley, Color32::WHITE);
        }
    }

    // ------------------------------------------------------------------------- commands

    /// Show the Events tab (opens the script panel docked when hidden).
    pub fn open_events(&mut self) {
        if self.settings.script_dock == crate::settings::ScriptDock::Hidden && !self.script_max {
            self.settings.script_dock = crate::settings::ScriptDock::Docked;
        }
        self.ensure_logic();
        if let Some(s) = self.scripts.as_mut() {
            s.ws.activate_host_tab(EVENTS_TAB);
            self.settings.logic.events_tab = true;
        }
        self.logic_sync();
    }

    /// Back to the DK script text.
    pub fn events_to_text(&mut self) {
        if let Some(s) = self.scripts.as_mut() {
            let dk = s.ws.dk_name();
            s.ws.activate(&dk);
        }
        self.settings.logic.events_tab = false;
    }

    /// View > Events editor (Shift+F3): show the Events tab, or the text when it is shown.
    pub fn toggle_events(&mut self) {
        if self.events_shown() {
            self.events_to_text();
        } else {
            self.open_events();
        }
    }

    /// The Events editor of the open level, in sync with the text (tests, headless).
    pub fn events_editor(&mut self) -> Option<&mut LogicEditor> {
        self.logic_sync();
        self.logic.as_mut().map(|l| &mut l.ed)
    }

    /// Run a change of the Events model (one Events undo step) and write it into the text.
    pub fn events_edit(&mut self, f: impl FnOnce(&mut LogicEditor)) {
        self.logic_sync();
        if let Some(l) = self.logic.as_mut() {
            f(&mut l.ed);
        }
        self.logic_write_back();
    }

    pub fn events_set_view(&mut self, v: View) {
        self.ensure_logic();
        if let Some(l) = self.logic.as_mut() {
            l.ed.set_view(v);
        }
        self.settings.logic.view = view_name(v).to_string();
    }

    /// Select the `n`-th event (1-based, document order) or the first whose summary contains
    /// `text`. Returns its summary.
    pub fn events_select(
        &mut self,
        n: Option<usize>,
        text: Option<&str>,
    ) -> Result<String, String> {
        self.logic_sync();
        let l = self.logic.as_mut().ok_or("no level open")?;
        let doc = l.ed.doc();
        let evs = doc.events();
        let found = match (n, text) {
            (Some(n), _) => evs.get(n.saturating_sub(1)).map(|(e, _)| e.id),
            (None, Some(t)) => {
                let t = t.to_ascii_lowercase();
                evs.iter()
                    .find(|(e, _)| {
                        kfx_logic_ui::analysis::event_summary(doc, e)
                            .to_ascii_lowercase()
                            .contains(&t)
                    })
                    .map(|(e, _)| e.id)
            }
            _ => None,
        };
        let id = found.ok_or_else(|| format!("no such event ({} events)", evs.len()))?;
        let summary = doc
            .event(id)
            .map(|e| kfx_logic_ui::analysis::event_summary(doc, e))
            .unwrap_or_default();
        l.ed.select(id);
        // what the next frame's highlight does, now (headless captures)
        let locs =
            l.ed.doc()
                .event(id)
                .map(|e| kfx_logic_ui::analysis::event_locations(l.ed.doc(), e))
                .unwrap_or_default();
        self.handle_logic_events(vec![LogicEvent::Highlight {
            event: Some(id),
            locations: locs,
        }]);
        Ok(summary)
    }

    /// Add a wave (and its party when `creatures` are given) to the DK script through the
    /// Events model; one Events undo step, one text undo step. Returns a description.
    pub fn events_add_wave(&mut self, w: &WaveSpec) -> Result<String, String> {
        use kfx_logic_ui::waves::{EscalatingWaves, Spawn};
        use kfx_logic_ui::{Cmd, Node};
        // a level without a DK script gets the default one first
        {
            let s = self.scripts.as_mut().ok_or("no level open")?;
            let dk = s.ws.dk_name();
            if !s.ws.tab_infos().iter().any(|t| t.name == dk && t.exists) {
                s.ws.create_dk_script();
            }
        }
        self.logic_sync();
        // the spawn place must be on the map
        let consts = self
            .scripts
            .as_ref()
            .map(|s| {
                (
                    s.consts.action_points().to_vec(),
                    s.consts.hero_gates().to_vec(),
                )
            })
            .unwrap_or_default();
        match kfx_logic_ui::analysis::parse_location(&w.spawn) {
            Some(MapLoc::ActionPoint(n)) if !consts.0.contains(&n) => {
                return Err(trf(
                    "kfx_editor.action_point_n_is_not_on_the_map",
                    "action point {n} is not on the map",
                    &[("n", &n)],
                ))
            }
            Some(MapLoc::HeroGate(n)) if !consts.1.contains(&n) => {
                return Err(trf(
                    "kfx_editor.hero_gate_n_is_not_on_the_map",
                    "hero gate {n} is not on the map",
                    &[("n", &n)],
                ))
            }
            None => {
                return Err(trf(
                    "kfx_editor.bad_spawn_location_spawn",
                    "bad spawn location {spawn}",
                    &[("spawn", &format!("{:?}", w.spawn))],
                ))
            }
            _ => {}
        }
        let l = self.logic.as_mut().ok_or("no level open")?;
        if let Some(r) = l.ed.read_only() {
            return Err(trf(
                "kfx_editor.the_script_is_view_only_in_the_events_editor_r",
                "the script is view-only in the Events editor ({r})",
                &[("r", &r)],
            ));
        }
        let doc = l.ed.doc();
        let party = match &w.party {
            Some(p) => p.trim().to_ascii_uppercase().replace(' ', "_"),
            None => {
                let mut n = 1;
                while doc.party(&format!("PARTY{n}")).is_some() {
                    n += 1;
                }
                format!("PARTY{n}")
            }
        };
        let exists = doc.party(&party).is_some();
        if w.creatures.is_empty() && !exists {
            return Err(trf(
                "kfx_editor.party_party_does_not_exist_give_its_creatures",
                "party {party} does not exist: give its creatures",
                &[("party", &party)],
            ));
        }
        if exists && !w.creatures.is_empty() {
            return Err(trf(
                "kfx_editor.party_party_already_exists_leave_out_the_creatur",
                "party {party} already exists: leave out the creatures or choose another name",
                &[("party", &party)],
            ));
        }
        if w.creatures.len() > 30 {
            return Err(trd(
                "kfx_editor.a_party_holds_at_most_30_creatures",
                "a party holds at most 30 creatures",
            ));
        }
        let mut first_new = None;
        let spec = w.clone();
        let pname = party.clone();
        l.ed.edit(|d| {
            if !spec.creatures.is_empty() {
                let mut p = d.new_party(&pname);
                p.pre = vec![String::new()];
                for (c, lv, gold) in &spec.creatures {
                    let m = d.new_member(
                        &c.to_ascii_uppercase(),
                        (*lv).clamp(1, 10),
                        *gold,
                        &spec.objective,
                        0,
                    );
                    p.members.push(m);
                }
                let at = d
                    .nodes
                    .iter()
                    .rposition(|n| matches!(n, Node::Party(_)))
                    .map(|i| i + 1)
                    .unwrap_or(d.nodes.len().min(1));
                d.nodes.insert(at, Node::Party(p));
            }
            let spawn = match &spec.tunneller {
                Some((head, target)) => Spawn::Tunneller {
                    location: spec.spawn.clone(),
                    head_for: head.clone(),
                    target: target.clone(),
                    level: 3,
                    gold: 200,
                },
                None => Spawn::Party {
                    location: spec.spawn.clone(),
                },
            };
            let events: Vec<kfx_logic_ui::Event> = match (&spec.condition, spec.turn) {
                (Some((pl, var, op, val)), _) => {
                    let c = d.new_condition(Cmd::new("IF", &[pl, var, op, val]));
                    let a = d.new_action(match &spawn {
                        Spawn::Party { location } => Cmd::new(
                            "ADD_PARTY_TO_LEVEL",
                            &[
                                &spec.owner,
                                &pname,
                                location,
                                &spec.copies.max(1).to_string(),
                            ],
                        ),
                        Spawn::Tunneller {
                            location,
                            head_for,
                            target,
                            level,
                            gold,
                        } => Cmd::new(
                            "ADD_TUNNELLER_PARTY_TO_LEVEL",
                            &[
                                &spec.owner,
                                &pname,
                                location,
                                head_for,
                                target,
                                &level.to_string(),
                                &gold.to_string(),
                            ],
                        ),
                    });
                    let mut e = d.new_event(vec![c], vec![Node::Action(a)]);
                    e.pre = vec![
                        String::new(),
                        format!("REM Wave: party {pname} when {var} {op} {val}"),
                    ];
                    vec![e]
                }
                (None, turn) => EscalatingWaves {
                    owner: spec.owner.clone(),
                    party: pname.clone(),
                    spawn,
                    clock_player: spec.clock_player.clone(),
                    first_turn: turn.unwrap_or(0),
                    interval: spec.interval.max(1),
                    count: spec.count.clamp(1, 50),
                    copies: spec.copies.max(1),
                    copies_step: spec.copies_step,
                }
                .build(d),
            };
            for e in events {
                first_new.get_or_insert(e.id);
                d.nodes.push(Node::Event(e));
            }
        });
        if let Some(id) = first_new {
            l.ed.select(id);
        }
        let line = first_new.and_then(|id| l.ed.node_line(id));
        self.logic_write_back();
        if let Some(id) = first_new {
            let locs = self
                .logic
                .as_ref()
                .and_then(|l| {
                    l.ed.doc()
                        .event(id)
                        .map(|e| kfx_logic_ui::analysis::event_locations(l.ed.doc(), e))
                })
                .unwrap_or_default();
            self.handle_logic_events(vec![LogicEvent::Highlight {
                event: Some(id),
                locations: locs,
            }]);
        }
        let when = match (&w.condition, w.turn) {
            (Some((p, v, o, x)), _) => trf(
                "kfx_editor.when_p_v_o_x",
                "when {p} {v} {o} {x}",
                &[("p", &p), ("v", &v), ("o", &o), ("x", &x)],
            ),
            (None, t) => trf(
                "kfx_editor.from_turn_t",
                "from turn {t}",
                &[("t", &t.unwrap_or(0))],
            ),
        };
        let waves = if w.condition.is_some() {
            1
        } else {
            w.count.clamp(1, 50)
        };
        let msg = trn(
            "kfx_editor.added_w_wave_s_of_party_at_spawn_when_l",
            waves as i64,
            "Added {n} wave of {party} at {spawn} {when}{l}",
            "Added {n} waves of {party} at {spawn} {when}{l}",
            &[
                ("party", &party),
                ("spawn", &w.spawn),
                ("when", &when),
                (
                    "l",
                    &line.map(|l| format!(" (line {l})")).unwrap_or_default(),
                ),
            ],
        );
        self.status = msg.clone();
        Ok(msg)
    }

    /// Text summary of the Events model (headless `events_print`).
    pub fn events_report(&mut self) -> String {
        self.logic_sync();
        let consts_ok = self.scripts.is_some();
        let Some(l) = self.logic.as_mut() else {
            return "no level open".into();
        };
        let mut out = vec![];
        if let Some(r) = l.ed.read_only() {
            out.push(format!("view only: {r}"));
        }
        let doc = l.ed.doc();
        for (i, (e, depth)) in doc.events().iter().enumerate() {
            out.push(format!(
                "{:>3} {}{}",
                i + 1,
                "  ".repeat(*depth),
                kfx_logic_ui::analysis::event_summary(doc, e)
            ));
        }
        for p in doc.parties() {
            out.push(format!("party {} ({} members)", p.name, p.members.len()));
        }
        if consts_ok {
            let consts = self.scripts.as_ref().map(|s| &s.consts);
            if let (Some(c), Some(l)) = (consts, self.logic.as_mut()) {
                l.ed.validate_now(c);
                let d = l.ed.diagnostics();
                let errs = d
                    .iter()
                    .filter(|d| d.severity == kfx_script::Severity::Error)
                    .count();
                out.push(format!(
                    "generated script: {errs} error(s), {} other diagnostic(s)",
                    d.len() - errs
                ));
                for x in d
                    .iter()
                    .filter(|d| d.severity == kfx_script::Severity::Error)
                {
                    out.push(format!("  error: {x:?}"));
                }
            }
        }
        if let (Some(l), Some(d)) = (self.logic.as_ref(), self.doc.as_ref()) {
            let pos = |m: &Option<MapLoc>| {
                m.as_ref()
                    .and_then(|m| loc_pos(&d.doc, &d.gd, m))
                    .map(|p| format!("({:.0},{:.0})", p.x, p.z))
                    .unwrap_or_else(|| "-".into())
            };
            for r in &l.routes {
                out.push(format!(
                    "route {}: {} {} -> {} {}",
                    r.party,
                    r.spawn.as_ref().map(|m| m.label()).unwrap_or_default(),
                    pos(&r.spawn),
                    r.head_for
                        .as_ref()
                        .map(|m| m.label())
                        .unwrap_or_else(|| "nearest heart".into()),
                    pos(&r.head_for)
                ));
            }
            if !l.highlight.is_empty() {
                out.push(format!(
                    "highlighted: {}",
                    l.highlight
                        .iter()
                        .map(|m| m.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        out.join("\n")
    }

    /// Validate the DK script text now: (errors, warnings) of the DK tab (tests).
    #[cfg(test)]
    pub fn dk_script_problems(&mut self) -> (usize, usize) {
        let Some(s) = self.scripts.as_mut() else {
            return (0, 0);
        };
        s.ws.analyze_now(&s.consts);
        let dk = s.ws.dk_name();
        let p = s.ws.problems();
        let n = |sev: kfx_script::Severity| {
            p.iter()
                .filter(|x| x.tab == dk && x.diagnostic.severity == sev)
                .count()
        };
        (
            n(kfx_script::Severity::Error),
            n(kfx_script::Severity::Warning),
        )
    }

    /// The map pick mode in progress (tests).
    #[cfg(test)]
    pub fn pick_kind(&self) -> Option<kfx_script_ui::PickKind> {
        self.pick
    }

    /// The DK script text (tests).
    #[cfg(test)]
    pub fn dk_script_text(&self) -> String {
        let Some(s) = self.scripts.as_ref() else {
            return String::new();
        };
        s.ws.editor(&s.ws.dk_name())
            .map(|e| String::from_utf8_lossy(&e.text_bytes()).into_owned())
            .unwrap_or_default()
    }

    /// Where the Events tab is drawn (tests).
    #[cfg(test)]
    pub fn events_rect(&self) -> Option<egui::Rect> {
        self.logic.as_ref().and_then(|l| l.rect)
    }

    /// Routes and highlighted places of the Events editor (tests, headless).
    #[cfg(test)]
    pub fn events_marks(&self) -> (Vec<Route>, Vec<MapLoc>) {
        self.logic
            .as_ref()
            .map(|l| (l.routes.clone(), l.highlight.clone()))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::line_diff;
    use kfx_script::Span;

    #[test]
    fn line_diff_replaces_whole_lines() {
        let old = b"A\r\nB\r\nC\r\n";
        let new = b"A\r\nBB\r\nC\r\n";
        // the change starts and ends at line starts (never inside a CR LF pair)
        assert_eq!(line_diff(old, new), Some((Span::new(3, 3), b"B".to_vec())));
        assert_eq!(
            line_diff(b"A\r\nB\r\n", b"A\r\nC\r\n"),
            Some((Span::new(3, 6), b"C\r\n".to_vec()))
        );
        // appended text
        let (s, b) = line_diff(b"A\nB\n", b"A\nB\nC\n").unwrap();
        assert_eq!((s, b), (Span::new(4, 4), b"C\n".to_vec()));
        // removed line
        let (s, b) = line_diff(b"A\nB\nC\n", b"A\nC\n").unwrap();
        assert_eq!((s, b.as_slice()), (Span::new(2, 4), &b""[..]));
        assert_eq!(line_diff(b"same", b"same"), None);
        // no final newline
        let (s, b) = line_diff(b"A\nB", b"A\nC").unwrap();
        assert_eq!((s, b), (Span::new(2, 3), b"C".to_vec()));
    }
}
