//! Party / wave designer, variables table and Lua export views.
use crate::analysis::{var_table, VarRow};
use crate::editor::{LogicEditor, LogicHost, Op, View};
use crate::lua::LuaFlagKind;
use crate::model::*;
use crate::ui_args::pick_combo;
use crate::waves::*;
use egui::{Color32, Id, RichText, Ui};
use kfx_script::consts::{Env, Extras};
use kfx_script::ArgKind;

/// [`CreatureStats`] through the host.
pub(crate) struct HostStats<'a>(pub &'a dyn LogicHost);

impl CreatureStats for HostStats<'_> {
    fn strength(&self, creature: &str, level: u32) -> Option<f32> {
        self.0
            .creature_strength(creature, level)
            .or_else(|| FlatStats.strength(creature, level))
    }
}

/// The wave template form.
#[derive(Clone, Debug)]
pub(crate) struct WaveForm {
    pub kind: usize,
    pub waves: EscalatingWaves,
    pub tunneller: bool,
    pub head_for: String,
    pub target: String,
    pub ap: u32,
    pub player: String,
    pub message: String,
    pub variable: String,
    pub op: String,
    pub value: String,
}

impl Default for WaveForm {
    fn default() -> Self {
        WaveForm {
            kind: 0,
            waves: EscalatingWaves::default(),
            tunneller: false,
            head_for: "DUNGEON_HEART".into(),
            target: "0".into(),
            ap: 1,
            player: "PLAYER0".into(),
            message: String::new(),
            variable: "FLAG0".into(),
            op: "==".into(),
            value: "1".into(),
        }
    }
}

fn candidates(host: &dyn LogicHost, kind: ArgKind, version: i64) -> Vec<String> {
    let extras = Extras::default();
    Env {
        provider: host.provider(),
        extras: &extras,
        version,
    }
    .candidates(kind, false)
}

pub(crate) fn show(ed: &mut LogicEditor, ui: &mut Ui, host: &dyn LogicHost, ops: &mut Vec<Op>) {
    let version = ed.doc.version;
    let creatures = {
        let c = candidates(host, ArgKind::Creature, version);
        if c.is_empty() {
            [
                "IMP",
                "KNIGHT",
                "ARCHER",
                "BARBARIAN",
                "DWARFA",
                "WIZARD",
                "GIANT",
                "FAIRY",
                "THIEF",
                "MONK",
                "SAMURAI",
                "AVATAR",
                "TUNNELLER",
                "WITCH",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect()
        } else {
            c
        }
    };
    let objectives = candidates(host, ArgKind::HeroObjective, version);
    let stats = HostStats(host);
    egui::Panel::left("logic-party-list")
        .resizable(true)
        .default_size(220.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Parties");
                if ui
                    .add_enabled(ed.doc.read_only.is_none(), egui::Button::new("+ New"))
                    .clicked()
                {
                    let mut n = 1;
                    while ed.doc.party(&format!("PARTY{n}")).is_some() {
                        n += 1;
                    }
                    let name = format!("PARTY{n}");
                    let mut created = None;
                    ed.edit(|d| {
                        let mut p = d.new_party(&name);
                        p.pre = vec![String::new()];
                        let m = d.new_member("KNIGHT", 3, 200, "ATTACK_ENEMIES", 0);
                        p.members.push(m);
                        created = Some(p.id);
                        // after the last party, or after the first level-start group
                        let at = d
                            .nodes
                            .iter()
                            .rposition(|n| matches!(n, Node::Party(_)))
                            .map(|i| i + 1)
                            .unwrap_or(d.nodes.len().min(1));
                        d.nodes.insert(at, Node::Party(p));
                    });
                    ed.party_sel = created;
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("party-list")
                .show(ui, |ui| {
                    for p in ed.doc.parties() {
                        let (s, _) = party_strength(p, &stats);
                        let label = format!("{}  ({} / ~{s:.0})", p.name, p.members.len());
                        if ui
                            .selectable_label(ed.party_sel == Some(p.id), label)
                            .clicked()
                        {
                            ed.party_sel = Some(p.id);
                        }
                    }
                });
        });
    let sel = ed
        .party_sel
        .filter(|id| ed.doc.parties().any(|p| p.id == *id))
        .or_else(|| ed.doc.parties().next().map(|p| p.id));
    ed.party_sel = sel;
    egui::ScrollArea::vertical()
        .id_salt("party-main")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if let Some(pid) = sel {
                roster(ed, ui, host, ops, pid, &creatures, &objectives);
                ui.separator();
            } else {
                ui.label("No party yet: create one with + New.");
            }
            templates(ed, ui, host, ops);
        });
}

fn roster(
    ed: &mut LogicEditor,
    ui: &mut Ui,
    host: &dyn LogicHost,
    ops: &mut Vec<Op>,
    pid: NodeId,
    creatures: &[String],
    objectives: &[String],
) {
    let stats = HostStats(host);
    let routes = {
        let name = ed
            .doc
            .parties()
            .find(|p| p.id == pid)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        party_routes(&ed.doc, &name)
    };
    let doc_snapshot = ed.doc.clone();
    let Some(p) = ed.doc.party_mut(pid) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.heading(format!("Party {}", p.name));
        let mut name = p.name.clone();
        ui.label("Rename");
        if ui
            .add(egui::TextEdit::singleline(&mut name).desired_width(140.0))
            .changed()
        {
            let new = name.to_ascii_uppercase().replace(' ', "_");
            if !new.is_empty() {
                p.name = new;
                p.raw = None;
                for m in &mut p.members {
                    m.raw = None;
                }
                let new_name = p.name.clone();
                let old_name = doc_snapshot
                    .parties()
                    .find(|q| q.id == pid)
                    .map(|q| q.name.clone())
                    .unwrap_or_default();
                ui.data_mut(|d| d.insert_temp(Id::unique("party-rename"), (old_name, new_name)));
            }
        }
        if ui.button("Dup").clicked() {
            ops.push(Op::Duplicate(pid));
        }
        if ui.button("Delete").clicked() {
            ops.push(Op::Delete(pid));
        }
    });
    let (total, unknown) = party_strength(p, &stats);
    ui.label(
        RichText::new(format!(
            "Estimated strength: {total:.0}{}  ({} of 30 members)",
            if unknown > 0 {
                format!(" + {unknown} unknown")
            } else {
                String::new()
            },
            p.members.len()
        ))
        .strong(),
    );
    egui::Grid::new(("roster", pid))
        .striped(true)
        .num_columns(8)
        .show(ui, |ui| {
            for h in [
                "",
                "Creature",
                "Level",
                "Gold",
                "Objective",
                "Countdown",
                "Strength",
                "",
            ] {
                ui.label(RichText::new(h).small().weak());
            }
            ui.end_row();
            let mut del = None;
            let mut dup = None;
            for (i, m) in p.members.iter_mut().enumerate() {
                match host.creature_portrait(&m.creature) {
                    Some(t) => {
                        ui.add(egui::Image::new((t, egui::vec2(28.0, 28.0))));
                    }
                    None => {
                        ui.label(
                            RichText::new(m.creature.chars().take(2).collect::<String>())
                                .monospace()
                                .weak(),
                        );
                    }
                }
                let mut changed = false;
                changed |= pick_combo(
                    ui,
                    Id::unique(("m-cr", m.id)),
                    &mut m.creature,
                    creatures,
                    120.0,
                    &|_| None,
                );
                let mut lv = crate::syntax::num(&m.level).unwrap_or(1);
                if ui
                    .add(egui::DragValue::new(&mut lv).range(1..=10).speed(0.1))
                    .changed()
                {
                    m.level = lv.to_string();
                    changed = true;
                }
                let mut gold = crate::syntax::num(&m.gold).unwrap_or(0);
                if ui
                    .add(egui::DragValue::new(&mut gold).range(0..=100000).speed(10))
                    .changed()
                {
                    m.gold = gold.to_string();
                    changed = true;
                }
                changed |= pick_combo(
                    ui,
                    Id::unique(("m-ob", m.id)),
                    &mut m.objective,
                    objectives,
                    150.0,
                    &|_| None,
                );
                let mut cd = crate::syntax::num(&m.countdown).unwrap_or(0);
                if ui
                    .add(egui::DragValue::new(&mut cd).range(0..=100000).speed(5))
                    .changed()
                {
                    m.countdown = cd.to_string();
                    changed = true;
                }
                let s = stats.strength(&m.creature, lv as u32).unwrap_or(0.0);
                ui.label(format!("{s:.0}"));
                ui.horizontal(|ui| {
                    if ui.small_button("Dup").clicked() {
                        dup = Some(i);
                    }
                    if ui.small_button("x").clicked() {
                        del = Some(i);
                    }
                });
                if changed {
                    m.raw = None;
                }
                ui.end_row();
            }
            if let Some(i) = dup {
                if p.members.len() < 30 {
                    let mut m = p.members[i].clone();
                    m.raw = None;
                    m.pre.clear();
                    m.id = u64::MAX - p.members.len() as u64; // renumbered below
                    p.members.insert(i + 1, m);
                }
            }
            if let Some(i) = del {
                p.members.remove(i);
            }
        });
    if ui
        .add_enabled(p.members.len() < 30, egui::Button::new("+ Member"))
        .clicked()
    {
        let last = p.members.last().cloned();
        let mut m = Member {
            id: u64::MAX - 100 - p.members.len() as u64,
            pre: vec![],
            creature: "KNIGHT".into(),
            level: "3".into(),
            gold: "200".into(),
            objective: "ATTACK_ENEMIES".into(),
            countdown: "0".into(),
            raw: None,
            trail: None,
            line: 0,
        };
        if let Some(l) = last {
            m.creature = l.creature;
            m.level = l.level;
            m.gold = l.gold;
            m.objective = l.objective;
        }
        p.members.push(m);
    }
    // give fresh ids to new members
    let needs: Vec<usize> = p
        .members
        .iter()
        .enumerate()
        .filter(|(_, m)| m.id > u64::MAX / 2)
        .map(|(i, _)| i)
        .collect();
    if !needs.is_empty() {
        let pname = p.name.clone();
        let _ = pname;
        for i in needs {
            let nid = ed.doc.new_id();
            if let Some(p) = ed.doc.party_mut(pid) {
                p.members[i].id = nid;
            }
        }
    }
    // rename references
    if let Some((old, new)) =
        ui.data_mut(|d| d.remove_temp::<(String, String)>(Id::unique("party-rename")))
    {
        rename_party_refs(&mut ed.doc, &old, &new);
        // the wave form names the party too
        if ed.wave.waves.party.eq_ignore_ascii_case(&old) {
            ed.wave.waves.party = new;
        }
    }
    ui.add_space(6.0);
    ui.strong("Used by");
    if routes.is_empty() {
        ui.label(RichText::new("No action spawns this party yet.").weak());
    }
    for (aid, spawn, head) in &routes {
        ui.horizontal(|ui| {
            let ev = ed.enclosing_event(*aid);
            let txt = ev
                .and_then(|e| ed.doc.event(e))
                .map(|e| crate::analysis::event_summary(&ed.doc, e))
                .unwrap_or_default();
            ui.label(format!(
                "spawn at {} {}",
                spawn
                    .as_ref()
                    .map(|l| l.label())
                    .unwrap_or_else(|| "?".into()),
                head.as_ref()
                    .map(|h| format!("-> {}", h.label()))
                    .unwrap_or_default()
            ));
            ui.label(RichText::new(txt).small().weak());
            if ui.small_button("Show on map").clicked() {
                let name = ed
                    .doc
                    .parties()
                    .find(|p| p.id == pid)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                ops.push(Op::Route(name, Some(*aid)));
            }
            if ui.small_button("Card").clicked() {
                if let Some(e) = ev {
                    ed.view = View::Cards;
                    ed.select(e);
                }
            }
        });
    }
}

/// Rename every reference to a party (arguments of kind party).
pub(crate) fn rename_party_refs(doc: &mut LogicDoc, old: &str, new: &str) {
    let version = doc.version;
    fn walk(nodes: &mut [Node], old: &str, new: &str, version: i64) {
        for n in nodes {
            match n {
                Node::Action(a) if !a.opaque => {
                    let mut hit = false;
                    for i in 0..a.cmd.args.len() {
                        let (k, _, _) = crate::syntax::arg_kind(&a.cmd.name, i, version);
                        if matches!(
                            k,
                            kfx_script::ArgKind::Party | kfx_script::ArgKind::PartyDef
                        ) && a.cmd.args[i].eq_ignore_ascii_case(old)
                        {
                            a.cmd.args[i] = new.to_string();
                            hit = true;
                        }
                    }
                    if hit {
                        a.touch();
                    }
                }
                Node::Event(e) => walk(&mut e.body, old, new, version),
                _ => {}
            }
        }
    }
    walk(&mut doc.nodes, old, new, version);
}

fn templates(ed: &mut LogicEditor, ui: &mut Ui, host: &dyn LogicHost, ops: &mut Vec<Op>) {
    let version = ed.doc.version;
    let parties: Vec<String> = ed.doc.parties().map(|p| p.name.clone()).collect();
    let players = candidates(host, ArgKind::Player, version);
    let heads = candidates(host, ArgKind::HeadFor, version);
    let f = &mut ed.wave;
    ui.heading("Wave templates");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut f.kind, 0, "Escalating waves");
        ui.selectable_value(&mut f.kind, 1, "Ambush on action point");
        ui.selectable_value(&mut f.kind, 2, "Final party on condition");
    });
    if f.waves.party.is_empty() {
        if let Some(p) = parties.first() {
            f.waves.party = p.clone();
        }
    }
    egui::Grid::new("wave-form").num_columns(2).show(ui, |ui| {
        ui.label("Party");
        pick_combo(
            ui,
            Id::unique("wf-party"),
            &mut f.waves.party,
            &parties,
            140.0,
            &|_| None,
        );
        ui.end_row();
        ui.label("Owner");
        pick_combo(
            ui,
            Id::unique("wf-owner"),
            &mut f.waves.owner,
            &players,
            140.0,
            &|_| None,
        );
        ui.end_row();
        match f.kind {
            0 => {
                let mut loc = match &f.waves.spawn {
                    Spawn::Party { location } | Spawn::Tunneller { location, .. } => {
                        location.clone()
                    }
                };
                ui.label("Spawn at");
                ui.add(egui::TextEdit::singleline(&mut loc).desired_width(80.0))
                    .on_hover_text("Action point (n), hero gate (-n) or PLAYERn");
                ui.end_row();
                ui.label("Tunneller party");
                ui.checkbox(&mut f.tunneller, "Dig towards a target");
                ui.end_row();
                if f.tunneller {
                    ui.label("Head for");
                    ui.horizontal(|ui| {
                        pick_combo(
                            ui,
                            Id::unique("wf-head"),
                            &mut f.head_for,
                            &heads,
                            130.0,
                            &|_| None,
                        );
                        ui.add(egui::TextEdit::singleline(&mut f.target).desired_width(40.0))
                            .on_hover_text("Action point number or player number");
                    });
                    ui.end_row();
                    f.waves.spawn = Spawn::Tunneller {
                        location: loc,
                        head_for: f.head_for.clone(),
                        target: f.target.clone(),
                        level: 3,
                        gold: 200,
                    };
                } else {
                    f.waves.spawn = Spawn::Party { location: loc };
                }
                ui.label("First wave at turn");
                ui.add(
                    egui::DragValue::new(&mut f.waves.first_turn)
                        .speed(50)
                        .range(0..=1_000_000),
                );
                ui.end_row();
                ui.label("Every N turns");
                ui.add(
                    egui::DragValue::new(&mut f.waves.interval)
                        .speed(50)
                        .range(1..=1_000_000),
                );
                ui.end_row();
                ui.label("Waves");
                ui.add(egui::DragValue::new(&mut f.waves.count).range(1..=50));
                ui.end_row();
                ui.label("Copies (first, +step)");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut f.waves.copies).range(1..=20));
                    ui.add(egui::DragValue::new(&mut f.waves.copies_step).range(0..=10));
                });
                ui.end_row();
            }
            1 => {
                ui.label("Action point");
                ui.add(egui::DragValue::new(&mut f.ap).range(1..=255));
                ui.end_row();
                ui.label("Reached by");
                pick_combo(
                    ui,
                    Id::unique("wf-pl"),
                    &mut f.player,
                    &players,
                    140.0,
                    &|_| None,
                );
                ui.end_row();
                ui.label("Message (optional)");
                ui.text_edit_singleline(&mut f.message);
                ui.end_row();
            }
            _ => {
                ui.label("Condition");
                ui.horizontal(|ui| {
                    pick_combo(
                        ui,
                        Id::unique("wf-cpl"),
                        &mut f.player,
                        &players,
                        100.0,
                        &|_| None,
                    );
                    ui.add(egui::TextEdit::singleline(&mut f.variable).desired_width(110.0));
                    ui.add(egui::TextEdit::singleline(&mut f.op).desired_width(28.0));
                    ui.add(egui::TextEdit::singleline(&mut f.value).desired_width(50.0));
                });
                ui.end_row();
                ui.label("Spawn at");
                let mut loc = match &f.waves.spawn {
                    Spawn::Party { location } | Spawn::Tunneller { location, .. } => {
                        location.clone()
                    }
                };
                if ui
                    .add(egui::TextEdit::singleline(&mut loc).desired_width(80.0))
                    .changed()
                {
                    f.waves.spawn = Spawn::Party { location: loc };
                }
                ui.end_row();
                ui.label("Objective text (optional)");
                ui.text_edit_singleline(&mut f.message);
                ui.end_row();
            }
        }
    });
    let ok = !f.waves.party.is_empty() && ed.doc.read_only.is_none();
    if ui
        .add_enabled(ok, egui::Button::new("Create events"))
        .clicked()
    {
        let mut scratch = ed.doc.clone();
        let f = ed.wave.clone();
        let built: Vec<Node> = match f.kind {
            0 => f
                .waves
                .build(&mut scratch)
                .into_iter()
                .map(Node::Event)
                .collect(),
            1 => vec![Node::Event(
                Ambush {
                    owner: f.waves.owner.clone(),
                    party: f.waves.party.clone(),
                    action_point: f.ap,
                    player: f.player.clone(),
                    location: String::new(),
                    copies: 1,
                    message: (!f.message.is_empty()).then(|| f.message.clone()),
                }
                .build(&mut scratch),
            )],
            _ => {
                let location = match &f.waves.spawn {
                    Spawn::Party { location } | Spawn::Tunneller { location, .. } => {
                        location.clone()
                    }
                };
                vec![Node::Event(
                    BossOnCondition {
                        owner: f.waves.owner.clone(),
                        party: f.waves.party.clone(),
                        location,
                        player: f.player.clone(),
                        variable: f.variable.clone(),
                        op: f.op.clone(),
                        value: f.value.clone(),
                        objective: (!f.message.is_empty()).then(|| f.message.clone()),
                    }
                    .build(&mut scratch),
                )]
            }
        };
        ops.push(Op::Insert(built));
    }
}

pub(crate) fn show_variables(
    ed: &mut LogicEditor,
    ui: &mut Ui,
    _host: &dyn LogicHost,
    ops: &mut Vec<Op>,
) {
    let rows: Vec<VarRow> = var_table(&ed.doc);
    ui.horizontal(|ui| {
        ui.label("Flags, timers and campaign flags: aliases, and which events read / write them.");
        ui.separator();
        for fam in VarFamily::ALL {
            if ui
                .add_enabled(
                    ed.doc.read_only.is_none(),
                    egui::Button::new(format!("+ {} alias", fam.prefix())),
                )
                .clicked()
            {
                ed.edit(|d| {
                    if let Some(i) = crate::analysis::free_slot(d, "PLAYER0", fam) {
                        d.variables.push(VarAlias {
                            name: format!("{}{i}", fam.prefix().to_lowercase()),
                            player: "PLAYER0".into(),
                            family: fam,
                            index: i,
                        });
                    }
                });
            }
        }
    });
    for fam in VarFamily::ALL {
        let used: Vec<String> = (0..fam.slots())
            .map(|i| {
                let n = rows
                    .iter()
                    .filter(|r| r.var == format!("{}{i}", fam.prefix()))
                    .count();
                if n > 0 {
                    format!("{i}:{n}")
                } else {
                    format!("{i}:-")
                }
            })
            .collect();
        ui.label(
            RichText::new(format!(
                "{} slots (players using each): {}",
                fam.prefix(),
                used.join("  ")
            ))
            .small()
            .weak(),
        );
    }
    let titles: Vec<(NodeId, String)> = ed
        .doc
        .events()
        .into_iter()
        .map(|(e, _)| (e.id, short_title(&ed.doc, e)))
        .collect();
    let title = |id: NodeId| {
        titles
            .iter()
            .find(|t| t.0 == id)
            .map(|t| t.1.clone())
            .unwrap_or_default()
    };
    let mut goto = None;
    egui::ScrollArea::vertical()
        .id_salt("vars")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("var-table")
                .striped(true)
                .num_columns(5)
                .show(ui, |ui| {
                    for h in ["Player", "Variable", "Alias", "Written by", "Read by"] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    for r in &rows {
                        ui.label(&r.player);
                        ui.monospace(&r.var);
                        let mut alias = r.alias.clone().unwrap_or_default();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut alias)
                                    .desired_width(120.0)
                                    .hint_text("name"),
                            )
                            .changed()
                        {
                            let (p, v) = (r.player.clone(), r.var.clone());
                            if let Some(a) = ed
                                .doc
                                .variables
                                .iter_mut()
                                .find(|a| a.player == p && a.var() == v)
                            {
                                a.name = alias.clone();
                            } else if let Some((family, index)) = VarFamily::parse(&v) {
                                ed.doc.variables.push(VarAlias {
                                    name: alias.clone(),
                                    player: p,
                                    family,
                                    index,
                                });
                            }
                            ed.doc.variables.retain(|a| !a.name.trim().is_empty());
                        }
                        for list in [&r.writers, &r.readers] {
                            ui.vertical(|ui| {
                                ui.set_min_width(330.0);
                                if list.is_empty() {
                                    ui.label(
                                        RichText::new("none")
                                            .weak()
                                            .color(Color32::from_rgb(220, 170, 40)),
                                    );
                                }
                                for id in list {
                                    if ui.link(title(*id)).clicked() {
                                        goto = Some(*id);
                                    }
                                }
                            });
                        }
                        ui.end_row();
                    }
                });
        });
    if let Some(id) = goto {
        ops.push(Op::Select(id));
        ed.view = View::Cards;
        ed.select(id);
    }
}

fn short_title(doc: &LogicDoc, e: &Event) -> String {
    let n = crate::analysis::notes_of(&e.pre);
    let t = match n.lines().next() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => crate::analysis::event_summary(doc, e),
    };
    if t.chars().count() > 60 {
        format!("{}…", t.chars().take(59).collect::<String>())
    } else {
        t
    }
}

pub(crate) fn show_lua(ed: &mut LogicEditor, ui: &mut Ui, host: &dyn LogicHost, ops: &mut Vec<Op>) {
    let gen = ed.generation();
    if ed.lua_cache.as_ref().map(|c| c.0) != Some(gen) {
        let idx = ed.lua_index().clone();
        let x = crate::lua::export_lua(&ed.doc, &idx, host.provider());
        ed.lua_cache = Some((gen, x));
    }
    let (_, x) = ed.lua_cache.as_ref().unwrap();
    let ne = x
        .flags
        .iter()
        .filter(|f| f.kind == LuaFlagKind::NoEquivalent)
        .count();
    let ap = x
        .flags
        .iter()
        .filter(|f| f.kind == LuaFlagKind::Approximate)
        .count();
    let constructs = if ne == 1 { "construct" } else { "constructs" };
    ui.label(format!(
        "Lua level script on the shipped trigger library. {ne} \
            {constructs} without a Lua equivalent, {ap} approximate."
    ));
    let flags = x.flags.clone();
    let mut lua = x.lua.clone();
    egui::Panel::right("lua-flags")
        .resizable(true)
        .default_size(320.0)
        .show(ui, |ui| {
            ui.strong("Not converted exactly");
            egui::ScrollArea::vertical()
                .id_salt("lua-flags-list")
                .show(ui, |ui| {
                    for f in &flags {
                        let color = if f.kind == LuaFlagKind::NoEquivalent {
                            Color32::from_rgb(220, 90, 70)
                        } else {
                            Color32::from_rgb(220, 170, 40)
                        };
                        ui.horizontal_wrapped(|ui| {
                            if ui.link(RichText::new(&f.what).color(color)).clicked() {
                                ops.push(Op::Select(f.node));
                                ui.data_mut(|d| d.insert_temp(Id::unique("lua-goto"), f.node));
                            }
                            ui.label(RichText::new(&f.reason).small());
                        });
                    }
                });
        });
    egui::ScrollArea::both()
        .id_salt("lua-text")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut lua)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .interactive(true),
            );
        });
    if let Some(id) = ui.data_mut(|d| d.remove_temp::<NodeId>(Id::unique("lua-goto"))) {
        ed.view = View::Cards;
        ed.select(id);
    }
}

/// The current Lua export text (for hosts that save it).
pub fn lua_text(ed: &mut LogicEditor, host: &dyn LogicHost) -> String {
    let idx = ed.lua_index().clone();
    crate::lua::export_lua(&ed.doc, &idx, host.provider()).lua
}
