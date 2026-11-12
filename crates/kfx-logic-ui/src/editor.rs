//! The egui widget: [`LogicEditor`].
//!
//! The host owns the editor, calls [`LogicEditor::show`] every frame with a [`LogicHost`] and
//! reacts to the returned [`LogicEvent`]s. Script bytes go in with [`LogicEditor::load`] and come
//! out with [`LogicEditor::script_bytes`]; wiring it to a script workspace is the host's job.
use crate::analysis::{self, Check, LocRef, MapLoc};
use crate::model::*;
use egui::{Color32, RichText, Ui};
use kfx_script::ConstantProvider;
use std::collections::{HashMap, HashSet};

/// What the host supplies.
pub trait LogicHost {
    /// Config-defined names and map facts (action points, hero gates).
    fn provider(&self) -> &dyn ConstantProvider;
    /// Creature portrait texture (party designer); `None` shows the name only.
    fn creature_portrait(&self, _creature: &str) -> Option<egui::TextureId> {
        None
    }
    /// Fighting strength of a creature at an experience level, for party estimates.
    fn creature_strength(&self, _creature: &str, _level: u32) -> Option<f32> {
        None
    }
    /// Text of a string id of the level (objective / information messages).
    fn string_text(&self, _id: i64) -> Option<String> {
        None
    }
}

/// Which map value a pick asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind {
    ActionPoint,
    HeroGate,
    /// Action point, hero gate or player heart (a `location` argument).
    Location,
    Subtile,
    Slab,
}

/// A value picked on the map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickValue {
    ActionPoint(i32),
    HeroGate(i32),
    /// A player's dungeon heart (`PLAYER0`...).
    Heart(String),
    Subtile {
        x: i32,
        y: i32,
    },
    Slab {
        x: i32,
        y: i32,
    },
}

/// Where a pick answer goes: argument `slot` of the condition / action `node` (for coordinates,
/// `slot` is the x argument and the y argument follows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickRequest {
    pub node: NodeId,
    pub slot: usize,
}

/// Events for the host.
#[derive(Clone, Debug, PartialEq)]
pub enum LogicEvent {
    /// The script changed (any edit, undo or redo). Read it with [`LogicEditor::script_bytes`].
    Changed,
    /// Pick a value on the map; answer with [`LogicEditor::answer_pick`].
    PickLocation {
        request: PickRequest,
        kind: PickKind,
    },
    /// Pick (or create) a string id; answer with [`LogicEditor::answer_string`].
    PickString { request: PickRequest },
    /// Highlight these map locations (the selected event's), or clear with an empty list.
    Highlight {
        event: Option<NodeId>,
        locations: Vec<LocRef>,
    },
    /// Show where a party spawns and where it heads, so the map can draw the path.
    ShowRoute {
        party: String,
        spawn: Option<MapLoc>,
        head_for: Option<MapLoc>,
    },
    /// Open the script text at this 1-based line.
    OpenInText { line: usize },
}

/// The views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Cards,
    Graph,
    Timeline,
    Parties,
    Variables,
    Lua,
}

impl View {
    pub const ALL: [View; 6] = [
        View::Cards,
        View::Graph,
        View::Timeline,
        View::Parties,
        View::Variables,
        View::Lua,
    ];
    pub fn label(self) -> &'static str {
        match self {
            View::Cards => "Events",
            View::Graph => "Graph",
            View::Timeline => "Timeline",
            View::Parties => "Parties & waves",
            View::Variables => "Variables",
            View::Lua => "Lua export",
        }
    }
}

/// Structural edits requested while drawing (applied after the frame's UI).
#[derive(Clone, Debug)]
pub(crate) enum Op {
    Delete(NodeId),
    Duplicate(NodeId),
    /// Move `node` before (or after) `target`, into `target`'s list.
    MoveNear {
        node: NodeId,
        target: NodeId,
        after: bool,
    },
    /// Move `node` to the end of event `into`'s body.
    MoveInto {
        node: NodeId,
        into: NodeId,
    },
    AddAction {
        event: NodeId,
        name: String,
    },
    AddCondition {
        event: NodeId,
        name: String,
    },
    RemoveCondition {
        event: NodeId,
        cond: NodeId,
    },
    AddEvent {
        conditions: bool,
    },
    AddNested {
        event: NodeId,
    },
    Select(NodeId),
    Open(NodeId),
    Pick(PickRequest, PickKind),
    PickString(PickRequest),
    /// Show the route of the party spawned by action `.1` (all of its spawns when `None`).
    Route(String, Option<NodeId>),
    Connect {
        from: NodeId,
        to: NodeId,
    },
    Retime {
        cond: NodeId,
        value: i64,
    },
    ShowParty(NodeId),
    /// Insert built events (wave templates) at the end of the document.
    Insert(Vec<Node>),
}

pub(crate) struct Validation {
    pub check: Check,
    /// Per node: (errors, warnings, messages).
    pub badges: HashMap<NodeId, (usize, usize, Vec<String>)>,
    pub lines: Vec<(NodeId, usize, usize)>,
}

pub(crate) struct GraphState {
    pub pan: egui::Vec2,
    pub zoom: f32,
    pub pos: HashMap<NodeId, egui::Pos2>,
    pub laid_out_for: u64,
    pub drag_wire: Option<NodeId>,
    pub fit: bool,
}

pub(crate) struct TimelineState {
    pub px_per_turn: f32,
}

/// The logic editor widget and its state.
pub struct LogicEditor {
    pub(crate) doc: LogicDoc,
    undo: Vec<LogicDoc>,
    redo: Vec<LogicDoc>,
    pub(crate) view: View,
    pub(crate) selected: Option<NodeId>,
    pub(crate) search: String,
    pub(crate) show_setup: bool,
    pub(crate) show_parties: bool,
    pub(crate) collapsed: HashSet<NodeId>,
    pub(crate) notes_open: HashSet<NodeId>,
    pub(crate) scroll_to: Option<NodeId>,
    pub(crate) validation: Option<Validation>,
    pub(crate) graph: GraphState,
    pub(crate) timeline: TimelineState,
    pub(crate) party_sel: Option<NodeId>,
    pub(crate) wave: crate::ui_parties::WaveForm,
    pub(crate) lua_index: Option<kfx_script::lua::ApiIndex>,
    pub(crate) lua_cache: Option<(u64, crate::lua::LuaExport)>,
    generation: u64,
    validated_gen: u64,
    last_edit: Option<(egui::Id, f64)>,
    out: Vec<LogicEvent>,
    last_highlight: Option<Option<NodeId>>,
}

impl Default for LogicEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl LogicEditor {
    pub fn new() -> LogicEditor {
        LogicEditor {
            doc: crate::import::import(b""),
            undo: vec![],
            redo: vec![],
            view: View::Cards,
            selected: None,
            search: String::new(),
            show_setup: true,
            show_parties: false,
            collapsed: HashSet::new(),
            notes_open: HashSet::new(),
            scroll_to: None,
            validation: None,
            graph: GraphState {
                pan: egui::Vec2::ZERO,
                zoom: 1.0,
                pos: HashMap::new(),
                laid_out_for: u64::MAX,
                drag_wire: None,
                fit: true,
            },
            timeline: TimelineState { px_per_turn: 0.05 },
            party_sel: None,
            wave: Default::default(),
            lua_index: None,
            lua_cache: None,
            generation: 1,
            validated_gen: 0,
            last_edit: None,
            out: vec![],
            last_highlight: None,
        }
    }

    /// Load script bytes (clears undo history).
    pub fn load(&mut self, bytes: &[u8]) {
        self.doc = crate::import::import(bytes);
        self.undo.clear();
        self.redo.clear();
        self.selected = None;
        self.party_sel = None;
        self.bump();
        // collapse level-start groups by default: they are long setup lists
        self.collapsed = self
            .doc
            .nodes
            .iter()
            .filter_map(|n| {
                if let Node::Event(e) = n {
                    e.conditions.is_empty().then_some(e.id)
                } else {
                    None
                }
            })
            .collect();
    }

    /// The script bytes the model generates.
    pub fn script_bytes(&self) -> Vec<u8> {
        crate::export::export_bytes(&self.doc)
    }

    /// `Some(reason)` when the script cannot be edited faithfully (view only).
    pub fn read_only(&self) -> Option<&str> {
        self.doc.read_only.as_deref()
    }

    pub fn doc(&self) -> &LogicDoc {
        &self.doc
    }

    /// Change the model programmatically (one undo step).
    pub fn edit(&mut self, f: impl FnOnce(&mut LogicDoc)) {
        if self.doc.read_only.is_some() {
            return;
        }
        let before = self.doc.clone();
        f(&mut self.doc);
        if self.doc != before {
            self.undo.push(before);
            self.redo.clear();
            self.bump();
            self.out.push(LogicEvent::Changed);
        }
    }

    pub fn view(&self) -> View {
        self.view
    }
    pub fn set_view(&mut self, v: View) {
        self.view = v;
    }
    pub fn selected(&self) -> Option<NodeId> {
        self.selected
    }
    /// Select a node (and scroll the cards to it).
    pub fn select(&mut self, id: NodeId) {
        self.selected = Some(id);
        self.scroll_to = Some(id);
        // open the cards on the way
        let mut cur = Some(id);
        while let Some(c) = cur {
            self.collapsed.remove(&c);
            cur = self.doc.locate(c).and_then(|(p, _)| p);
        }
    }
    /// Show party `name` in the party / wave designer. False when there is no such party.
    pub fn show_party(&mut self, name: &str) -> bool {
        let Some(id) = self.doc.party(name).map(|p| p.id) else {
            return false;
        };
        self.view = View::Parties;
        self.party_sel = Some(id);
        true
    }
    /// The party shown in the party / wave designer.
    pub fn shown_party(&self) -> Option<&str> {
        let id = self.party_sel?;
        self.doc
            .parties()
            .find(|p| p.id == id)
            .map(|p| p.name.as_str())
    }
    pub fn set_search(&mut self, s: &str) {
        self.search = s.to_string();
    }
    /// Use this Lua API index for the Lua export (default: the stock library index).
    pub fn set_lua_index(&mut self, index: kfx_script::lua::ApiIndex) {
        self.lua_index = Some(index);
        self.lua_cache = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self) {
        if let Some(d) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.doc, d));
            self.last_edit = None;
            self.bump();
            self.out.push(LogicEvent::Changed);
        }
    }
    pub fn redo(&mut self) {
        if let Some(d) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.doc, d));
            self.last_edit = None;
            self.bump();
            self.out.push(LogicEvent::Changed);
        }
    }

    fn bump(&mut self) {
        self.generation += 1;
    }
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Map markers per event.
    pub fn locations(&self) -> Vec<(NodeId, Vec<LocRef>)> {
        analysis::locations(&self.doc)
    }

    /// Retime a time condition (what dragging a timeline marker does).
    pub fn retime(&mut self, cond: NodeId, value: i64) {
        self.edit(|d| retime(d, cond, value));
    }

    /// Chain two events with a new flag (what the graph's port drag does): `from` gains
    /// `SET_FLAG(p, FLAGn, 1)`, `to` gains the condition `IF(p, FLAGn == 1)`, and the flag gets an
    /// alias.
    pub fn connect(&mut self, from: NodeId, to: NodeId) {
        self.edit(|d| connect(d, from, to));
    }

    /// Answer a [`LogicEvent::PickLocation`].
    pub fn answer_pick(&mut self, req: PickRequest, value: PickValue) {
        self.edit(|d| {
            let (a, b): (String, Option<String>) = match &value {
                PickValue::ActionPoint(n) => (n.to_string(), None),
                PickValue::HeroGate(n) => ((-n.abs()).to_string(), None),
                PickValue::Heart(p) => (p.clone(), None),
                PickValue::Subtile { x, y } | PickValue::Slab { x, y } => {
                    (x.to_string(), Some(y.to_string()))
                }
            };
            // a hero gate number for a HeroGate-kind argument is written positive
            let apply = |cmd: &mut Cmd, version: i64| {
                let (k, _, _) = crate::syntax::arg_kind(&cmd.name, req.slot, version);
                let v = if k == kfx_script::ArgKind::HeroGate {
                    a.trim_start_matches('-').to_string()
                } else {
                    a.clone()
                };
                cmd.set_arg(req.slot, v);
                if let Some(b) = &b {
                    cmd.set_arg(req.slot + 1, b.clone());
                }
            };
            let v = d.version;
            if let Some(x) = d.action_mut(req.node) {
                apply(&mut x.cmd, v);
                x.touch();
            } else if let Some(c) = d.condition_mut(req.node) {
                apply(&mut c.cmd, v);
                c.touch();
            }
        });
    }

    /// Answer a [`LogicEvent::PickString`] with a string id.
    pub fn answer_string(&mut self, req: PickRequest, id: i64) {
        self.edit(|d| {
            if let Some(x) = d.action_mut(req.node) {
                x.cmd.set_arg(req.slot, id.to_string());
                x.touch();
            }
        });
    }

    /// Run validation now (normally done lazily after edits).
    pub fn validate_now(&mut self, provider: &dyn ConstantProvider) {
        let check = analysis::check(&self.doc, provider);
        let lines = analysis::node_lines(&self.doc);
        let mut badges: HashMap<NodeId, (usize, usize, Vec<String>)> = HashMap::new();
        for d in &check.diagnostics {
            if d.severity == kfx_script::Severity::Info {
                continue;
            }
            // innermost node holding the line
            let best = lines
                .iter()
                .filter(|(_, s, e)| *s <= d.line && d.line <= *e)
                .min_by_key(|(_, s, e)| e - s);
            if let Some((id, _, _)) = best {
                let b = badges.entry(*id).or_default();
                if d.severity == kfx_script::Severity::Error {
                    b.0 += 1;
                } else {
                    b.1 += 1;
                }
                b.2.push(format!("line {}: {}", d.line, d.message));
            }
        }
        self.validation = Some(Validation {
            check,
            badges,
            lines,
        });
        self.validated_gen = self.generation;
    }

    /// Diagnostics of the generated script (after the last validation).
    pub fn diagnostics(&self) -> &[kfx_script::Diagnostic] {
        self.validation
            .as_ref()
            .map(|v| v.check.diagnostics.as_slice())
            .unwrap_or(&[])
    }

    /// Resource meters of the generated script (after the last validation).
    pub fn meters(&self) -> Vec<analysis::Meter> {
        self.validation
            .as_ref()
            .map(|v| v.check.meters.clone())
            .unwrap_or_default()
    }

    /// Generated line range of a node (after validation).
    pub fn node_line(&self, id: NodeId) -> Option<usize> {
        let lines = self
            .validation
            .as_ref()
            .map(|v| v.lines.clone())
            .unwrap_or_else(|| analysis::node_lines(&self.doc));
        lines.iter().find(|(n, _, _)| *n == id).map(|(_, s, _)| *s)
    }

    /// Draw the editor; returns the events of this frame.
    pub fn show(&mut self, ui: &mut Ui, host: &dyn LogicHost) -> Vec<LogicEvent> {
        if self.validated_gen != self.generation {
            self.validate_now(host.provider());
        }
        let before = self.doc.clone();
        let gen_before = self.generation;
        let mut ops: Vec<Op> = vec![];
        self.toolbar(ui, host, &mut ops);
        ui.separator();
        match self.view {
            View::Cards => crate::ui_cards::show(self, ui, host, &mut ops),
            View::Graph => crate::ui_graph::show(self, ui, host, &mut ops),
            View::Timeline => crate::ui_timeline::show(self, ui, host, &mut ops),
            View::Parties => crate::ui_parties::show(self, ui, host, &mut ops),
            View::Variables => crate::ui_parties::show_variables(self, ui, host, &mut ops),
            View::Lua => crate::ui_parties::show_lua(self, ui, host, &mut ops),
        }
        let read_only = self.doc.read_only.is_some();
        if self.generation != gen_before {
            // undo / redo from the toolbar already recorded their step
        } else if read_only {
            self.doc = before;
        } else if self.doc != before {
            // in-place edit by a widget: one undo step per focused widget burst
            let focus = ui.ctx().memory(|m| m.focused());
            let now = ui.ctx().input(|i| i.time);
            let coalesce = match (self.last_edit, focus) {
                (Some((id, t)), Some(f)) => id == f && now - t < 2.0,
                _ => false,
            };
            if !coalesce {
                self.undo.push(before);
                self.redo.clear();
            }
            self.last_edit = focus.map(|f| (f, now));
            self.bump();
            self.out.push(LogicEvent::Changed);
        }
        for op in ops {
            self.apply(op, read_only);
        }
        // highlight the selected event's locations when the selection changes
        let sel_event = self.selected.and_then(|s| self.enclosing_event(s));
        if self.last_highlight != Some(sel_event) {
            self.last_highlight = Some(sel_event);
            let locations = sel_event
                .and_then(|e| self.doc.event(e))
                .map(|e| analysis::event_locations(&self.doc, e))
                .unwrap_or_default();
            self.out.push(LogicEvent::Highlight {
                event: sel_event,
                locations,
            });
        }
        std::mem::take(&mut self.out)
    }

    /// The event holding node `id` (itself when it is an event).
    pub fn enclosing_event(&self, id: NodeId) -> Option<NodeId> {
        if self.doc.event(id).is_some() {
            return Some(id);
        }
        for (e, _) in self.doc.events() {
            if e.conditions.iter().any(|c| c.id == id) || e.body.iter().any(|n| n.id() == id) {
                return Some(e.id);
            }
        }
        None
    }

    fn toolbar(&mut self, ui: &mut Ui, _host: &dyn LogicHost, ops: &mut Vec<Op>) {
        ui.horizontal_wrapped(|ui| {
            for v in View::ALL {
                if ui.selectable_label(self.view == v, v.label()).clicked() {
                    self.view = v;
                }
            }
            ui.separator();
            if ui
                .add_enabled(self.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(self.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                self.redo();
            }
            ui.separator();
            let ro = self.doc.read_only.is_some();
            ui.add_enabled_ui(!ro, |ui| {
                // (never wrapped: a narrow panel moves the buttons to the next row instead)
                if ui
                    .add(egui::Button::new("+ Event").wrap_mode(egui::TextWrapMode::Extend))
                    .on_hover_text("New event with a condition")
                    .clicked()
                {
                    ops.push(Op::AddEvent { conditions: true });
                }
                if ui
                    .add(egui::Button::new("+ Level start").wrap_mode(egui::TextWrapMode::Extend))
                    .on_hover_text("New group of commands run when the level loads")
                    .clicked()
                {
                    ops.push(Op::AddEvent { conditions: false });
                }
            });
        });
        if let Some(r) = &self.doc.read_only {
            egui::Frame::new()
                .fill(Color32::from_rgb(90, 60, 20))
                .inner_margin(6.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "View only: {r}. The editor will not rewrite this script; \
                                edit it as text."
                        ))
                        .color(Color32::WHITE),
                    );
                });
        }
        // meters and validation summary
        if let Some(v) = &self.validation {
            ui.horizontal_wrapped(|ui| {
                let errs = v
                    .check
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == kfx_script::Severity::Error)
                    .count();
                let warns = v
                    .check
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == kfx_script::Severity::Warning)
                    .count();
                badge(ui, errs, warns, "Validation of the generated script");
                for m in &v.check.meters {
                    meter(ui, m);
                }
            });
        }
    }

    fn apply(&mut self, op: Op, read_only: bool) {
        match op {
            Op::Select(id) => {
                self.selected = Some(id);
                return;
            }
            Op::Open(id) => {
                self.select(id);
                if let Some(l) = self.node_line(id) {
                    self.out.push(LogicEvent::OpenInText { line: l });
                }
                return;
            }
            Op::Pick(request, kind) => {
                if !read_only {
                    self.out.push(LogicEvent::PickLocation { request, kind });
                }
                return;
            }
            Op::PickString(request) => {
                if !read_only {
                    self.out.push(LogicEvent::PickString { request });
                }
                return;
            }
            Op::ShowParty(id) => {
                self.view = View::Parties;
                self.party_sel = Some(id);
                return;
            }
            Op::Route(party, only) => {
                for (aid, spawn, head_for) in crate::waves::party_routes(&self.doc, &party) {
                    if only.is_some_and(|o| o != aid) {
                        continue;
                    }
                    self.out.push(LogicEvent::ShowRoute {
                        party: party.clone(),
                        spawn,
                        head_for,
                    });
                }
                return;
            }
            _ => {}
        }
        if read_only {
            return;
        }
        let mut new_sel = None;
        self.edit(|d| match op {
            Op::Delete(id) => {
                if d.remove(id).is_none() {
                    // a party member
                    for n in &mut d.nodes {
                        if let Node::Party(p) = n {
                            p.members.retain(|m| m.id != id);
                        }
                    }
                }
            }
            Op::Duplicate(id) => {
                if let Some((parent, i)) = d.locate(id) {
                    let mut copy = d.list_mut(parent).unwrap()[i].clone();
                    d.reid(&mut copy);
                    if let Node::Party(p) = &mut copy {
                        p.name = format!("{}_COPY", p.name);
                        p.raw = None;
                        for m in &mut p.members {
                            m.raw = None;
                        }
                    }
                    new_sel = Some(copy.id());
                    d.insert(parent, i + 1, copy);
                }
            }
            Op::MoveNear {
                node,
                target,
                after,
            } => {
                if node == target || is_inside(d, target, node) {
                    return;
                }
                if let Some(n) = d.remove(node) {
                    if let Some((parent, i)) = d.locate(target) {
                        let idx = if after { i + 1 } else { i };
                        d.insert(parent, idx, n);
                    } else {
                        d.nodes.push(n);
                    }
                }
            }
            Op::MoveInto { node, into } => {
                if node == into || is_inside(d, into, node) {
                    return;
                }
                if let Some(n) = d.remove(node) {
                    if let Some(e) = d.event_mut(into) {
                        e.body.push(n);
                    }
                }
            }
            Op::AddAction { event, name } => {
                let cmd = default_cmd(&name, d.version);
                let a = d.new_action(cmd);
                new_sel = Some(a.id);
                if let Some(e) = d.event_mut(event) {
                    e.body.push(Node::Action(a));
                }
            }
            Op::AddCondition { event, name } => {
                let c = d.new_condition(default_cmd(&name, d.version));
                if let Some(e) = d.event_mut(event) {
                    if !e.conditions.is_empty()
                        && e.conditions.iter().all(|c| c.endif_raw.is_some())
                    {
                        // keep the existing ENDIF lines; the new one is innermost
                    }
                    e.conditions.push(c);
                }
            }
            Op::RemoveCondition { event, cond } => {
                if let Some(e) = d.event_mut(event) {
                    e.conditions.retain(|c| c.id != cond);
                }
            }
            Op::AddEvent { conditions } => {
                let conds = if conditions {
                    vec![d.new_condition(default_cmd("IF", d.version))]
                } else {
                    vec![]
                };
                let mut e = d.new_event(conds, vec![]);
                e.pre = vec![String::new()];
                new_sel = Some(e.id);
                d.nodes.push(Node::Event(e));
            }
            Op::AddNested { event } => {
                let c = d.new_condition(default_cmd("IF", d.version));
                let e = d.new_event(vec![c], vec![]);
                new_sel = Some(e.id);
                if let Some(p) = d.event_mut(event) {
                    p.body.push(Node::Event(e));
                }
            }
            Op::Connect { from, to } => connect(d, from, to),
            Op::Insert(nodes) => {
                for mut n in nodes {
                    d.reid(&mut n);
                    new_sel = Some(n.id());
                    d.nodes.push(n);
                }
            }
            Op::Retime { cond, value } => retime(d, cond, value),
            _ => {}
        });
        if let Some(s) = new_sel {
            self.select(s);
        }
    }

    pub(crate) fn lua_index(&mut self) -> &kfx_script::lua::ApiIndex {
        self.lua_index
            .get_or_insert_with(kfx_script::lua::ApiIndex::stock)
    }
}

fn is_inside(d: &LogicDoc, inner: NodeId, outer: NodeId) -> bool {
    let mut cur = d.locate(inner).and_then(|(p, _)| p);
    while let Some(c) = cur {
        if c == outer {
            return true;
        }
        cur = d.locate(c).and_then(|(p, _)| p);
    }
    false
}

/// A flag chain from event `from` to event `to`: `from` sets a free flag of `PLAYER0` (or the
/// first player its conditions name), `to` gains the condition `flag == 1`.
pub(crate) fn connect(d: &mut LogicDoc, from: NodeId, to: NodeId) {
    if from == to {
        return;
    }
    let player = d
        .event(from)
        .and_then(|e| {
            e.conditions
                .iter()
                .map(|c| c.cmd.arg(0).to_ascii_uppercase())
                .find(|p| p.starts_with("PLAYER") && p != "PLAYER_GOOD")
        })
        .unwrap_or_else(|| "PLAYER0".into());
    let Some(slot) = analysis::free_slot(d, &player, VarFamily::Flag) else {
        return;
    };
    let var = format!("FLAG{slot}");
    let set = d.new_action(Cmd::new("SET_FLAG", &[&player, &var, "1"]));
    let cond = d.new_condition(Cmd::new("IF", &[&player, &var, "==", "1"]));
    let name = format!("Chain{slot}");
    if d.alias_of(&player, &var).is_none() {
        d.variables.push(VarAlias {
            name,
            player: player.clone(),
            family: VarFamily::Flag,
            index: slot,
        });
    }
    if let Some(e) = d.event_mut(from) {
        e.body.push(Node::Action(set));
    }
    if let Some(e) = d.event_mut(to) {
        e.conditions.push(cond);
    }
}

/// Set the compared value of a time condition (`IF(p, GAME_TURN >= value)` / `TIMERn >= value`).
pub(crate) fn retime(d: &mut LogicDoc, cond: NodeId, value: i64) {
    if let Some(c) = d.condition_mut(cond) {
        c.cmd.set_arg(3, value.max(0).to_string());
        c.touch();
    }
}

/// A command with default arguments for its required slots.
pub fn default_cmd(name: &str, version: i64) -> Cmd {
    use kfx_script::ArgKind as K;
    let n = crate::syntax::required(name, version);
    let mut args = vec![];
    for i in 0..n {
        let (k, _, _) = crate::syntax::arg_kind(name, i, version);
        let v = match k {
            K::Player => {
                if i == 0 && (name.starts_with("ADD_") && name.ends_with("TO_LEVEL")) {
                    "PLAYER_GOOD"
                } else {
                    "PLAYER0"
                }
            }
            K::Operator => "==",
            K::Variable => "GAME_TURN",
            K::AvailVariable | K::CtrlVariable => "TOTAL_CREATURES",
            K::Creature => "KNIGHT",
            K::Location => "1",
            K::ApNumber => "1",
            K::HeroGate => "1",
            K::FlagVar => "FLAG0",
            K::Timer => "TIMER0",
            K::CampaignFlag => "CAMPAIGN_FLAG0",
            K::HeadFor => "DUNGEON_HEART",
            K::HeroObjective => "ATTACK_ENEMIES",
            K::Level => "1",
            K::Count => "1",
            K::Bool => "1",
            K::Text | K::NewName => "\"\"",
            K::Party | K::PartyDef => "PARTY",
            K::Room => "TREASURE",
            K::Slab => "PATH",
            _ => "0",
        };
        args.push(v.to_string());
    }
    if name == "IF" && args.len() >= 4 {
        args[3] = "2000".into();
        args[2] = ">=".into();
    }
    Cmd {
        name: name.to_string(),
        args,
    }
}

pub(crate) fn badge(ui: &mut Ui, errs: usize, warns: usize, hover: &str) -> egui::Response {
    let (text, color) = if errs > 0 {
        (format!("{errs} error(s)"), Color32::from_rgb(220, 70, 60))
    } else if warns > 0 {
        (
            format!("{warns} warning(s)"),
            Color32::from_rgb(220, 170, 40),
        )
    } else {
        ("OK".to_string(), Color32::from_rgb(70, 170, 90))
    };
    egui::Frame::new()
        .fill(color.gamma_multiply(0.25))
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).color(color).small()))
        .response
        .on_hover_text(hover)
}

fn meter(ui: &mut Ui, m: &analysis::Meter) {
    let frac = if m.limit == 0 {
        0.0
    } else {
        m.used as f32 / m.limit as f32
    };
    let color = if frac > 1.0 {
        Color32::from_rgb(220, 70, 60)
    } else if frac >= 0.8 {
        Color32::from_rgb(220, 170, 40)
    } else {
        Color32::from_rgb(90, 150, 220)
    };
    // a fixed-size cell: a `vertical` inside the wrapping row would take the row's whole
    // remaining height in a narrow panel
    let text = RichText::new(format!("{} {}/{}", m.label, m.used, m.limit)).small();
    let galley = egui::WidgetText::from(text.clone()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Small,
    );
    let size = egui::vec2(galley.size().x.max(90.0), galley.size().y + 6.0);
    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_max_size(size);
        ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
        let (r, p) = ui.allocate_painter(egui::vec2(90.0, 4.0), egui::Sense::hover());
        p.rect_filled(r.rect, 2.0, ui.visuals().extreme_bg_color);
        let mut f = r.rect;
        f.set_width(r.rect.width() * frac.clamp(0.0, 1.0));
        p.rect_filled(f, 2.0, color);
    });
}
