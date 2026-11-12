//! Event list view: collapsible "When [conditions] -> Do [actions]" cards.
use crate::analysis::{action_text, condition_text, notes_of, set_notes, time_gate};
use crate::editor::{badge, LogicEditor, LogicHost, Op};
use crate::model::*;
use crate::ui_args::{cmd_args, pick_combo, ArgCtx};
use egui::{Align, Color32, Id, RichText, Stroke, Ui};
use std::collections::{HashMap, HashSet};

const CONDITION_KINDS: [&str; 7] = [
    "IF",
    "IF_ACTION_POINT",
    "IF_AVAILABLE",
    "IF_CONTROLS",
    "IF_SLAB_OWNER",
    "IF_SLAB_TYPE",
    "IF_ALLIED",
];

/// Commands offered for actions (everything but the block structure).
pub(crate) fn action_commands(version: i64) -> Vec<String> {
    let mut v: Vec<String> = kfx_script::Tables::get()
        .table(version.max(1))
        .iter()
        .map(|c| c.name.clone())
        .filter(|n| {
            !crate::syntax::is_if(n)
                && !matches!(n.as_str(), "ENDIF" | "REM" | "NEXT_COMMAND_REUSABLE")
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

pub(crate) struct CardCtx<'a> {
    pub cx: ArgCtx<'a>,
    pub badges: &'a HashMap<NodeId, (usize, usize, Vec<String>)>,
    pub collapsed: &'a mut HashSet<NodeId>,
    pub notes_open: &'a mut HashSet<NodeId>,
    pub selected: Option<NodeId>,
    pub scroll_to: &'a mut Option<NodeId>,
    pub textdoc: &'a LogicDoc,
    pub commands: &'a [String],
    pub strength: &'a dyn Fn(&Party) -> f32,
    pub read_only: bool,
}

fn accent(e: &Event, doc: &LogicDoc) -> Color32 {
    if !e.enabled {
        return Color32::GRAY;
    }
    if e.conditions.is_empty() {
        return Color32::from_rgb(110, 140, 180);
    }
    if time_gate(doc, e).is_some() {
        return Color32::from_rgb(220, 150, 60);
    }
    match e.conditions[0].cmd.name.as_str() {
        "IF_ACTION_POINT" => Color32::from_rgb(90, 190, 110),
        "IF_SLAB_OWNER" | "IF_SLAB_TYPE" => Color32::from_rgb(170, 120, 200),
        _ => Color32::from_rgb(200, 90, 90),
    }
}

fn node_badge(ui: &mut Ui, cc: &CardCtx, id: NodeId) {
    if let Some((e, w, msgs)) = cc.badges.get(&id) {
        badge(ui, *e, *w, &msgs.join("\n"));
    }
}

/// Drop target handling on a row: returns an op when something was dropped on it.
fn drop_on(ui: &Ui, resp: &egui::Response, target: NodeId) -> Option<Op> {
    let payload = resp.dnd_hover_payload::<NodeId>()?;
    if *payload == target {
        return None;
    }
    let pos = ui.ctx().pointer_interact_pos()?;
    let after = pos.y > resp.rect.center().y;
    let y = if after {
        resp.rect.bottom() + 1.0
    } else {
        resp.rect.top() - 1.0
    };
    ui.painter().hline(
        resp.rect.x_range(),
        y,
        Stroke::new(3.0, ui.visuals().selection.bg_fill),
    );
    let dropped = resp.dnd_release_payload::<NodeId>()?;
    Some(Op::MoveNear {
        node: *dropped,
        target,
        after,
    })
}

fn drag_handle(ui: &mut Ui, id: NodeId) {
    ui.dnd_drag_source(Id::unique(("logic-drag", id)), id, |ui| {
        ui.label(RichText::new("::").monospace().strong())
            .on_hover_text("Drag to reorder");
    });
}

pub(crate) fn event_card(ui: &mut Ui, cc: &mut CardCtx, e: &mut Event, depth: usize) {
    let id = e.id;
    let open = !cc.collapsed.contains(&id);
    let sel = cc.selected == Some(id);
    let color = accent(e, cc.textdoc);
    let mut frame = egui::Frame::group(ui.style()).inner_margin(egui::Margin {
        left: 10,
        right: 6,
        top: 4,
        bottom: 4,
    });
    if sel {
        frame = frame.stroke(Stroke::new(2.0, ui.visuals().selection.stroke.color));
    }
    let r = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        // header
        ui.horizontal(|ui| {
            drag_handle(ui, id);
            if ui
                .small_button(if open { "v" } else { ">" })
                .on_hover_text(if open { "Collapse" } else { "Expand" })
                .clicked()
            {
                if open {
                    cc.collapsed.insert(id);
                } else {
                    cc.collapsed.remove(&id);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if !cc.read_only {
                    if ui
                        .small_button("Del")
                        .on_hover_text("Delete this event")
                        .clicked()
                    {
                        cc.cx.ops.push(Op::Delete(id));
                    }
                    if ui.small_button("Dup").on_hover_text("Duplicate").clicked() {
                        cc.cx.ops.push(Op::Duplicate(id));
                    }
                }
                if ui
                    .small_button("Text")
                    .on_hover_text("Open in the text editor")
                    .clicked()
                {
                    cc.cx.ops.push(Op::Open(id));
                }
                if ui.small_button("Notes").clicked() {
                    if !cc.notes_open.insert(id) {
                        cc.notes_open.remove(&id);
                    }
                }
                if !cc.read_only {
                    ui.checkbox(&mut e.enabled, "")
                        .on_hover_text("Enabled (a disabled event is written as a REM block)");
                } else if !e.enabled {
                    ui.label(RichText::new("disabled").small().weak());
                }
                node_badge(ui, cc, id);
                if e.conditions.is_empty() {
                    ui.label(
                        RichText::new(format!("{} cmd", e.body.len()))
                            .small()
                            .weak(),
                    );
                }
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    let notes = notes_of(&e.pre);
                    let title = notes.lines().next().unwrap_or("").to_string();
                    let when = if e.conditions.is_empty() {
                        "LEVEL START".to_string()
                    } else {
                        format!(
                            "WHEN {}",
                            e.conditions
                                .iter()
                                .map(|c| condition_text(cc.textdoc, c))
                                .collect::<Vec<_>>()
                                .join("  AND  ")
                        )
                    };
                    let mut job = RichText::new(&when).strong();
                    if !e.enabled {
                        job = job.strikethrough().weak();
                    }
                    let hr = ui.add(egui::Label::new(job).sense(egui::Sense::click()).truncate());
                    if hr.clicked() {
                        cc.cx.ops.push(Op::Select(id));
                    }
                    if !title.is_empty() {
                        ui.add(
                            egui::Label::new(RichText::new(format!("- {title}")).weak().italics())
                                .truncate(),
                        );
                    }
                    if e.conditions.is_empty() && !open {
                        let names: Vec<String> = e
                            .body
                            .iter()
                            .filter_map(|n| match n {
                                Node::Action(a) if !a.opaque => Some(a.cmd.name.clone()),
                                _ => None,
                            })
                            .fold(Vec::<String>::new(), |mut v, n| {
                                if !v.contains(&n) {
                                    v.push(n);
                                }
                                v
                            });
                        let shown: Vec<String> = names.iter().take(4).cloned().collect();
                        let more = if names.len() > 4 { ", …" } else { "" };
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("{}{more}", shown.join(", ")))
                                    .small()
                                    .weak(),
                            )
                            .truncate(),
                        );
                    }
                });
            });
        });
        if !open {
            return;
        }
        let ro = cc.read_only;
        ui.add_enabled_ui(!ro, |ui| event_body(ui, cc, e, depth, color));
    });
    // accent strip
    let rect = r.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(4.0, rect.height())),
        2.0,
        color,
    );
    if *cc.scroll_to == Some(id) {
        r.response.scroll_to_me(Some(Align::TOP));
        *cc.scroll_to = None;
    }
}

fn event_body(ui: &mut Ui, cc: &mut CardCtx, e: &mut Event, depth: usize, color: Color32) {
    let id = e.id;
    {
        if cc.notes_open.contains(&id) {
            let mut notes = notes_of(&e.pre);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Notes").small().weak());
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut notes)
                            .desired_rows(2)
                            .desired_width(ui.available_width()),
                    )
                    .changed()
                {
                    set_notes(&mut e.pre, &notes);
                }
            });
        }
        // conditions
        let mut remove_cond = None;
        let n_cond = e.conditions.len();
        for (ci, c) in e.conditions.iter_mut().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if ci == 0 { "When" } else { "and" })
                        .color(color)
                        .strong(),
                );
                let mut kind = c.cmd.name.clone();
                let kinds: Vec<String> = CONDITION_KINDS.iter().map(|s| s.to_string()).collect();
                if pick_combo(
                    ui,
                    Id::unique(("cond-kind", c.id)),
                    &mut kind,
                    &kinds,
                    110.0,
                    &|_| None,
                ) && kind != c.cmd.name
                {
                    c.cmd = crate::editor::default_cmd(&kind, cc.cx.version);
                    c.touch();
                }
                let mut cmd = c.cmd.clone();
                if cmd_args(ui, &mut cc.cx, c.id, &mut cmd, false) {
                    c.cmd = cmd;
                    c.touch();
                }
                node_badge(ui, cc, c.id);
                if n_cond > 1
                    && ui
                        .small_button("x")
                        .on_hover_text("Remove this condition")
                        .clicked()
                {
                    remove_cond = Some(c.id);
                }
            });
        }
        if let Some(c) = remove_cond {
            cc.cx.ops.push(Op::RemoveCondition { event: id, cond: c });
        }
        if !e.conditions.is_empty() {
            ui.horizontal(|ui| {
                let mut k = String::new();
                let kinds: Vec<String> = CONDITION_KINDS.iter().map(|s| s.to_string()).collect();
                ui.label(RichText::new("+ AND").small().weak());
                if pick_combo(
                    ui,
                    Id::unique(("add-cond", id)),
                    &mut k,
                    &kinds,
                    110.0,
                    &|_| None,
                ) && !k.is_empty()
                {
                    cc.cx.ops.push(Op::AddCondition { event: id, name: k });
                }
                let reusable = e
                    .body
                    .iter()
                    .any(|n| matches!(n, Node::Action(a) if a.reusable));
                if reusable {
                    ui.label(
                        RichText::new("has reusable actions")
                            .small()
                            .color(Color32::from_rgb(220, 170, 40)),
                    );
                }
            });
            ui.label(RichText::new("Do").color(color).strong());
        }
        // body
        let mut body_ops: Vec<Op> = vec![];
        for n in e.body.iter_mut() {
            match n {
                Node::Action(a) => action_row(ui, cc, a, &mut body_ops),
                Node::Event(sub) => {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            let r = ui.scope(|ui| event_card(ui, cc, sub, depth + 1)).response;
                            if let Some(op) = drop_on(ui, &r, sub.id) {
                                body_ops.push(op);
                            }
                        });
                    });
                }
                Node::Party(p) => party_row(ui, cc, p),
            }
        }
        cc.cx.ops.extend(body_ops);
        if !e.tail.is_empty() && e.tail.iter().any(|l| !l.trim().is_empty()) {
            ui.label(RichText::new(notes_of(&e.tail)).small().weak().italics());
        }
        ui.horizontal(|ui| {
            let mut k = String::new();
            ui.label(RichText::new("+ action").small().weak());
            if pick_combo(
                ui,
                Id::unique(("add-act", id)),
                &mut k,
                cc.commands,
                180.0,
                &|c| {
                    Some(crate::syntax::command_doc(c))
                        .filter(|d| !d.is_empty())
                        .map(|d| d.chars().take(48).collect())
                },
            ) && !k.is_empty()
            {
                cc.cx.ops.push(Op::AddAction { event: id, name: k });
            }
            if !e.conditions.is_empty()
                && ui
                    .small_button("+ nested event")
                    .on_hover_text("A block that also needs another condition")
                    .clicked()
            {
                cc.cx.ops.push(Op::AddNested { event: id });
            }
            // drop here to move into this event
            let (_, dropped) =
                ui.dnd_drop_zone::<NodeId, ()>(egui::Frame::new().inner_margin(2.0), |ui| {
                    ui.label(RichText::new("Drop here to move inside").small().weak());
                });
            if let Some(d) = dropped {
                cc.cx.ops.push(Op::MoveInto { node: *d, into: id });
            }
        });
    }
}

fn action_row(ui: &mut Ui, cc: &mut CardCtx, a: &mut Action, ops: &mut Vec<Op>) {
    let sel = cc.selected == Some(a.id);
    let r = ui
        .horizontal_wrapped(|ui| {
            ui.add_space(12.0);
            drag_handle(ui, a.id);
            if !cc.read_only {
                if ui.small_button("x").on_hover_text("Delete").clicked() {
                    cc.cx.ops.push(Op::Delete(a.id));
                }
                if ui.small_button("+").on_hover_text("Duplicate").clicked() {
                    cc.cx.ops.push(Op::Duplicate(a.id));
                }
            }
            if !a.pre.is_empty() && a.pre.iter().any(|l| !l.trim().is_empty()) {
                ui.label(RichText::new("#").weak())
                    .on_hover_text(notes_of(&a.pre));
            }
            if a.opaque {
                let mut raw = a.raw.clone().unwrap_or_default();
                ui.label(
                    RichText::new("text")
                        .small()
                        .color(Color32::from_rgb(220, 170, 40)),
                )
                .on_hover_text("Not a command the editor understands: kept verbatim");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut raw)
                            .code_editor()
                            .desired_width(380.0),
                    )
                    .changed()
                {
                    a.raw = Some(raw);
                }
            } else {
                let mut name = a.cmd.name.clone();
                let lbl = RichText::new(&name).monospace().strong();
                let _ = lbl;
                if pick_combo(
                    ui,
                    Id::unique(("act-name", a.id)),
                    &mut name,
                    cc.commands,
                    170.0,
                    &|_| None,
                ) && name != a.cmd.name
                {
                    a.cmd = crate::editor::default_cmd(&name, cc.cx.version);
                    a.touch();
                }
                let mut cmd = a.cmd.clone();
                if cmd_args(ui, &mut cc.cx, a.id, &mut cmd, true) {
                    a.cmd = cmd;
                    a.touch();
                }
                if ui
                    .checkbox(&mut a.reusable, "Every turn")
                    .on_hover_text(
                        "NEXT_COMMAND_REUSABLE: run every turn the condition holds, not once",
                    )
                    .changed()
                {
                    a.reusable_raw = None;
                }
            }
            node_badge(ui, cc, a.id);
        })
        .response;
    if sel {
        ui.painter().rect_stroke(
            r.rect,
            3.0,
            Stroke::new(1.0, ui.visuals().selection.stroke.color),
            egui::StrokeKind::Outside,
        );
    }
    if r.interact(egui::Sense::click()).clicked() {
        cc.cx.ops.push(Op::Select(a.id));
    }
    if let Some(op) = drop_on(ui, &r, a.id) {
        ops.push(op);
    }
    if *cc.scroll_to == Some(a.id) {
        r.scroll_to_me(Some(Align::Center));
        *cc.scroll_to = None;
    }
    if a.reusable {
        // nothing more: the toggle shows the state
    }
    let _ = action_text;
}

fn party_row(ui: &mut Ui, cc: &mut CardCtx, p: &mut Party) {
    let s = (cc.strength)(p);
    let r = egui::Frame::group(ui.style())
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                drag_handle(ui, p.id);
                ui.label(
                    RichText::new(format!("PARTY {}", p.name))
                        .strong()
                        .color(Color32::from_rgb(200, 170, 90)),
                );
                let names: Vec<String> = p
                    .members
                    .iter()
                    .map(|m| format!("{} L{}", m.creature, m.level))
                    .collect();
                ui.label(
                    RichText::new(format!("{} member(s), strength ~{s:.0}", p.members.len()))
                        .weak(),
                );
                ui.label(RichText::new(names.join(", ")).small().weak());
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Designer").clicked() {
                        cc.cx.ops.push(Op::ShowParty(p.id));
                    }
                    node_badge(ui, cc, p.id);
                });
            });
        })
        .response;
    if let Some(op) = drop_on(ui, &r, p.id) {
        cc.cx.ops.push(op);
    }
}

fn matches_search(doc: &LogicDoc, n: &Node, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let q = q.to_ascii_uppercase();
    let text = match n {
        Node::Event(e) => format!(
            "{}\n{}",
            crate::export::event_text(doc, e),
            notes_of(&e.pre)
        ),
        Node::Party(p) => format!(
            "PARTY {} {}",
            p.name,
            p.members
                .iter()
                .map(|m| m.creature.clone())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Node::Action(a) => crate::export::action_line(doc, a),
    };
    text.to_ascii_uppercase().contains(&q)
}

pub(crate) fn show(ed: &mut LogicEditor, ui: &mut Ui, host: &dyn LogicHost, ops: &mut Vec<Op>) {
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.add(
            egui::TextEdit::singleline(&mut ed.search)
                .desired_width(220.0)
                .hint_text("command, creature, note…"),
        );
        if ui.small_button("Expand all").clicked() {
            ed.collapsed.clear();
        }
        if ui.small_button("Collapse all").clicked() {
            ed.collapsed = ed.doc.events().into_iter().map(|(e, _)| e.id).collect();
        }
        ui.checkbox(&mut ed.show_setup, "Level start");
        ui.checkbox(&mut ed.show_parties, "Parties");
        let n_ev = ed.doc.events().len();
        ui.label(
            RichText::new(format!(
                "{n_ev} events, {} parties",
                ed.doc.parties().count()
            ))
            .weak(),
        );
    });
    let textdoc = LogicDoc {
        variables: ed.doc.variables.clone(),
        version: ed.doc.version,
        ..LogicDoc::default()
    };
    let parties: Vec<String> = ed.doc.parties().map(|p| p.name.clone()).collect();
    let aliases = ed.doc.variables.clone();
    let empty = HashMap::new();
    let commands = action_commands(ed.doc.version);
    let version = ed.doc.version;
    let search = ed.search.clone();
    let strength =
        |p: &Party| crate::waves::party_strength(p, &crate::ui_parties::HostStats(host)).0;
    let read_only = ed.doc.read_only.is_some();
    let (show_setup, show_parties) = (ed.show_setup, ed.show_parties);
    let LogicEditor {
        doc,
        validation,
        collapsed,
        notes_open,
        selected,
        scroll_to,
        ..
    } = ed;
    let badges = validation.as_ref().map(|v| &v.badges).unwrap_or(&empty);
    let fulldoc_for_search = doc.clone();
    egui::ScrollArea::vertical()
        .id_salt("logic-cards")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut cc = CardCtx {
                cx: ArgCtx {
                    host,
                    version,
                    aliases: &aliases,
                    parties: &parties,
                    ops,
                },
                badges,
                collapsed,
                notes_open,
                selected: *selected,
                scroll_to,
                textdoc: &textdoc,
                commands: &commands,
                strength: &strength,
                read_only,
            };
            let hidden_parties = doc.parties().count();
            if !show_parties && hidden_parties > 0 {
                ui.label(
                    RichText::new(format!(
                        "{hidden_parties} parties hidden: see Parties & waves, or \
                            tick Parties above"
                    ))
                    .small()
                    .weak(),
                );
            }
            let mut local: Vec<Op> = vec![];
            for (i, n) in doc.nodes.iter_mut().enumerate() {
                if !matches_search(&fulldoc_for_search, &fulldoc_for_search.nodes[i], &search) {
                    continue;
                }
                match n {
                    Node::Party(_) if !show_parties => continue,
                    Node::Event(e) if e.conditions.is_empty() && !show_setup => continue,
                    _ => {}
                }
                match n {
                    Node::Event(e) => {
                        let id = e.id;
                        let r = ui.scope(|ui| event_card(ui, &mut cc, e, 0)).response;
                        if let Some(op) = drop_on(ui, &r, id) {
                            local.push(op);
                        }
                    }
                    Node::Party(p) => party_row(ui, &mut cc, p),
                    Node::Action(a) => action_row(ui, &mut cc, a, &mut local),
                }
                ui.add_space(2.0);
            }
            cc.cx.ops.extend(local);
            if !doc.tail.is_empty() && doc.tail.iter().any(|l| !l.trim().is_empty()) {
                ui.label(RichText::new(notes_of(&doc.tail)).small().weak().italics());
            }
        });
}
