//! What the model means: readable summaries, map locations, variable usage, timing, resources.
use crate::export::export_bytes;
use crate::model::*;
use crate::syntax::{arg_kind, num};
use kfx_script::{ArgKind, ConstantProvider};

// ---- map locations -------------------------------------------------------------------------

/// A place on the map the logic refers to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MapLoc {
    ActionPoint(i32),
    HeroGate(i32),
    /// A player's dungeon heart (`PLAYER0`, `PLAYER_GOOD`, ...).
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

impl MapLoc {
    pub fn label(&self) -> String {
        match self {
            MapLoc::ActionPoint(n) => format!("AP {n}"),
            MapLoc::HeroGate(n) => format!("Gate {n}"),
            MapLoc::Heart(p) => format!("{p} heart"),
            MapLoc::Subtile { x, y } => format!("stl {x},{y}"),
            MapLoc::Slab { x, y } => format!("slab {x},{y}"),
        }
    }
}

/// What a location is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocRole {
    /// A condition watches it (action point reached, slab owner/type).
    Trigger,
    /// Creatures / a party / a tunneller appear there.
    SpawnAt,
    /// A tunneller heads for it.
    HeadFor,
    /// Any other use (messages, reveal, zoom, effects...).
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocRef {
    pub loc: MapLoc,
    pub role: LocRole,
    /// Condition, action or member node holding it.
    pub node: NodeId,
    pub command: String,
}

/// Interpret a `location` argument (`get_map_location_id`, `src/map_locations.c:396`): a positive
/// number is an action point, a negative one a hero gate, a player name that player's heart.
pub fn parse_location(text: &str) -> Option<MapLoc> {
    if let Some(n) = num(text) {
        return match n {
            n if n > 0 => Some(MapLoc::ActionPoint(n as i32)),
            n if n < 0 => Some(MapLoc::HeroGate((-n) as i32)),
            _ => None,
        };
    }
    let u = text.to_ascii_uppercase();
    if u.starts_with("PLAYER")
        && kfx_script::Tables::get().is_named("player_desc", &u)
        && u != "ALL_PLAYERS"
    {
        return Some(MapLoc::Heart(u));
    }
    None
}

fn player_by_number(n: i64) -> String {
    match n {
        0..=3 => format!("PLAYER{n}"),
        4 => "PLAYER_GOOD".into(),
        5 => "PLAYER_NEUTRAL".into(),
        n => format!("PLAYER{}", n - 1),
    }
}

/// Locations of one command.
pub fn cmd_locations(cmd: &Cmd, version: i64, node: NodeId, is_condition: bool) -> Vec<LocRef> {
    let mut out = vec![];
    let push = |out: &mut Vec<LocRef>, loc: MapLoc, role: LocRole| {
        out.push(LocRef {
            loc,
            role,
            node,
            command: cmd.name.clone(),
        })
    };
    let spawn = matches!(
        cmd.name.as_str(),
        "ADD_PARTY_TO_LEVEL"
            | "ADD_CREATURE_TO_LEVEL"
            | "ADD_TUNNELLER_TO_LEVEL"
            | "ADD_TUNNELLER_PARTY_TO_LEVEL"
            | "ADD_OBJECT_TO_LEVEL"
    );
    let mut sx: Option<(i32, bool)> = None;
    for (i, a) in cmd.args.iter().enumerate() {
        let (k, _, _) = arg_kind(&cmd.name, i, version);
        match k {
            ArgKind::Location => {
                if let Some(l) = parse_location(a) {
                    push(
                        &mut out,
                        l,
                        if spawn {
                            LocRole::SpawnAt
                        } else {
                            LocRole::Other
                        },
                    );
                }
            }
            ArgKind::ApNumber => {
                if let Some(n) = num(a).filter(|n| *n > 0) {
                    push(
                        &mut out,
                        MapLoc::ActionPoint(n as i32),
                        if is_condition {
                            LocRole::Trigger
                        } else {
                            LocRole::Other
                        },
                    );
                }
            }
            ArgKind::HeroGate => {
                if let Some(n) = num(a).filter(|n| *n != 0) {
                    push(
                        &mut out,
                        MapLoc::HeroGate(n.unsigned_abs() as i32),
                        LocRole::Other,
                    );
                }
            }
            ArgKind::SubtileX | ArgKind::SlabX => {
                if let Some(n) = num(a) {
                    sx = Some((n as i32, k == ArgKind::SlabX));
                }
            }
            ArgKind::SubtileY | ArgKind::SlabY => {
                if let (Some((x, slab)), Some(y)) = (sx.take(), num(a)) {
                    let loc = if slab {
                        MapLoc::Slab { x, y: y as i32 }
                    } else {
                        MapLoc::Subtile { x, y: y as i32 }
                    };
                    push(
                        &mut out,
                        loc,
                        if is_condition {
                            LocRole::Trigger
                        } else {
                            LocRole::Other
                        },
                    );
                }
            }
            ArgKind::HeadFor => {
                // ADD_TUNNELLER*: heading, then target (action point or player number)
                let target = cmd.args.get(i + 1).and_then(|t| num(t));
                match (a.to_ascii_uppercase().as_str(), target) {
                    ("ACTION_POINT", Some(n)) if n > 0 => {
                        push(&mut out, MapLoc::ActionPoint(n as i32), LocRole::HeadFor)
                    }
                    ("DUNGEON" | "DUNGEON_HEART", Some(n)) => push(
                        &mut out,
                        MapLoc::Heart(player_by_number(n)),
                        LocRole::HeadFor,
                    ),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

/// Locations used by an event's own conditions and direct actions (not nested events).
pub fn event_locations(doc: &LogicDoc, e: &Event) -> Vec<LocRef> {
    let mut out = vec![];
    for c in &e.conditions {
        out.extend(cmd_locations(&c.cmd, doc.version, c.id, true));
    }
    for n in &e.body {
        if let Node::Action(a) = n {
            if !a.opaque {
                out.extend(cmd_locations(&a.cmd, doc.version, a.id, false));
            }
        }
    }
    out
}

/// Map markers per event (every event of the tree).
pub fn locations(doc: &LogicDoc) -> Vec<(NodeId, Vec<LocRef>)> {
    doc.events()
        .into_iter()
        .map(|(e, _)| (e.id, event_locations(doc, e)))
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

// ---- variables -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Access {
    Read,
    Write,
}

/// One read or write of a flag / timer / campaign flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarUse {
    pub player: String,
    /// `FLAG3`, `TIMER0`, `CAMPAIGN_FLAG1`.
    pub var: String,
    pub access: Access,
    /// The innermost event holding the condition or action.
    pub event: NodeId,
    pub node: NodeId,
    pub command: String,
    pub line: usize,
}

fn var_uses_cmd(cmd: &Cmd, is_cond: bool) -> Vec<(String, String, Access)> {
    let a = |i: usize| cmd.arg(i).to_ascii_uppercase();
    let mut v: Vec<(String, String, Access)> = vec![];
    match cmd.name.as_str() {
        "IF" if is_cond => {
            v.push((a(0), a(1), Access::Read));
            if !cmd.arg(4).is_empty() {
                v.push((a(3), a(4), Access::Read));
            }
        }
        "SET_FLAG"
        | "ADD_TO_FLAG"
        | "RANDOMISE_FLAG"
        | "RANDOMIZE_FLAG"
        | "SET_CAMPAIGN_FLAG"
        | "ADD_TO_CAMPAIGN_FLAG"
        | "SET_TIMER"
        | "ADD_TO_TIMER" => v.push((a(0), a(1), Access::Write)),
        "COMPUTE_FLAG" => {
            v.push((a(0), a(1), Access::Write));
            v.push((a(3), a(4), Access::Read));
        }
        "EXPORT_VARIABLE" => {
            v.push((a(0), a(1), Access::Read));
            v.push((a(0), a(2), Access::Write));
        }
        "COUNT_CREATURES_AT_ACTION_POINT" => v.push((a(3), a(4), Access::Write)),
        "DISPLAY_VARIABLE"
        | "DISPLAY_TIMER"
        | "DISPLAY_COUNTDOWN"
        | "DISPLAY_VARIABLE_WITH_LABEL" => v.push((a(0), a(1), Access::Read)),
        _ => {}
    }
    v.retain(|(_, var, _)| VarFamily::parse(var).is_some());
    v
}

/// Every flag / timer / campaign flag read and write, in document order.
pub fn var_usage(doc: &LogicDoc) -> Vec<VarUse> {
    let mut out = vec![];
    for (e, _) in doc.events() {
        for c in &e.conditions {
            for (p, var, access) in var_uses_cmd(&c.cmd, true) {
                out.push(VarUse {
                    player: p,
                    var,
                    access,
                    event: e.id,
                    node: c.id,
                    command: c.cmd.name.clone(),
                    line: c.line,
                });
            }
        }
        for n in &e.body {
            if let Node::Action(a) = n {
                if a.opaque {
                    continue;
                }
                for (p, var, access) in var_uses_cmd(&a.cmd, false) {
                    out.push(VarUse {
                        player: p,
                        var,
                        access,
                        event: e.id,
                        node: a.id,
                        command: a.cmd.name.clone(),
                        line: a.line,
                    });
                }
            }
        }
    }
    out
}

/// One row of the variable table: a (player, variable) with its alias and users.
#[derive(Clone, Debug, PartialEq)]
pub struct VarRow {
    pub player: String,
    pub var: String,
    pub alias: Option<String>,
    pub readers: Vec<NodeId>,
    pub writers: Vec<NodeId>,
}

/// Variable table: every aliased or used variable.
pub fn var_table(doc: &LogicDoc) -> Vec<VarRow> {
    let mut rows: Vec<VarRow> = vec![];
    let row = |rows: &mut Vec<VarRow>, p: &str, v: &str| -> usize {
        if let Some(i) = rows.iter().position(|r| r.player == p && r.var == v) {
            return i;
        }
        rows.push(VarRow {
            player: p.to_string(),
            var: v.to_string(),
            alias: None,
            readers: vec![],
            writers: vec![],
        });
        rows.len() - 1
    };
    for a in &doc.variables {
        let i = row(&mut rows, &a.player, &a.var());
        rows[i].alias = Some(a.name.clone());
    }
    for u in var_usage(doc) {
        let i = row(&mut rows, &u.player, &u.var);
        let list = if u.access == Access::Read {
            &mut rows[i].readers
        } else {
            &mut rows[i].writers
        };
        if !list.contains(&u.event) {
            list.push(u.event);
        }
    }
    rows.sort_by(|a, b| {
        (VarFamily::parse(&a.var), &a.player).cmp(&(VarFamily::parse(&b.var), &b.player))
    });
    rows
}

/// First slot of `family` for `player` that is neither used nor aliased.
pub fn free_slot(doc: &LogicDoc, player: &str, family: VarFamily) -> Option<u32> {
    let used: Vec<u32> = var_table(doc)
        .iter()
        .filter(|r| r.player.eq_ignore_ascii_case(player) || r.player == "ALL_PLAYERS")
        .filter_map(|r| {
            VarFamily::parse(&r.var)
                .filter(|(f, _)| *f == family)
                .map(|(_, i)| i)
        })
        .collect();
    (0..family.slots()).find(|i| !used.contains(i))
}

// ---- readable text -------------------------------------------------------------------------

fn var_label(doc: &LogicDoc, player: &str, var: &str) -> String {
    match doc.alias_of(player, var) {
        Some(a) => format!("{} ({var})", a.name),
        None => var.to_string(),
    }
}

/// One-line reading of a condition.
pub fn condition_text(doc: &LogicDoc, c: &Condition) -> String {
    let m = &c.cmd;
    let a = |i: usize| m.arg(i).to_string();
    match m.name.as_str() {
        "IF" => {
            let rhs = if m.arg(4).is_empty() {
                a(3)
            } else {
                format!("{} {}", a(3), var_label(doc, m.arg(3), m.arg(4)))
            };
            format!(
                "{}: {} {} {rhs}",
                a(0),
                var_label(doc, m.arg(0), m.arg(1)),
                a(2)
            )
        }
        "IF_ACTION_POINT" => format!("{} reaches action point {}", a(1), a(0)),
        "IF_AVAILABLE" => format!("{}: {} available {} {}", a(0), a(1), a(2), a(3)),
        "IF_CONTROLS" => format!("{} controls {} {} {}", a(0), a(1), a(2), a(3)),
        "IF_SLAB_OWNER" => format!("slab {},{} owned by {}", a(0), a(1), a(2)),
        "IF_SLAB_TYPE" => format!("slab {},{} is {}", a(0), a(1), a(2)),
        "IF_ALLIED" => format!("{} allied with {} {} {}", a(0), a(1), a(2), a(3)),
        _ => crate::export::condition_line(doc, c),
    }
}

/// Short reading of an action.
pub fn action_text(doc: &LogicDoc, x: &Action) -> String {
    if x.opaque {
        return x.raw.clone().unwrap_or_default().trim().to_string();
    }
    let m = &x.cmd;
    let a = |i: usize| m.arg(i).to_string();
    let loc = |i: usize| {
        parse_location(m.arg(i))
            .map(|l| l.label())
            .unwrap_or_else(|| a(i))
    };
    let r = if x.reusable { " (every turn)" } else { "" };
    let s = match m.name.as_str() {
        "ADD_PARTY_TO_LEVEL" => {
            format!("Spawn party {} for {} at {} x{}", a(1), a(0), loc(2), a(3))
        }
        "ADD_TUNNELLER_PARTY_TO_LEVEL" => format!(
            "Tunneller party {} for {} at {} -> {} {}",
            a(1),
            a(0),
            loc(2),
            a(3),
            a(4)
        ),
        "ADD_TUNNELLER_TO_LEVEL" => {
            format!("Tunneller for {} at {} -> {} {}", a(0), loc(1), a(2), a(3))
        }
        "ADD_CREATURE_TO_LEVEL" => format!(
            "Spawn {} x{} {} lvl {} at {}",
            a(1),
            a(3),
            a(0),
            a(4),
            loc(2)
        ),
        "DISPLAY_OBJECTIVE" => format!("Objective #{}", a(0)),
        "DISPLAY_INFORMATION" => format!("Information #{}", a(0)),
        "QUICK_OBJECTIVE" => format!("Objective: {}", m.arg_unquoted(1)),
        "QUICK_INFORMATION" => format!("Information: {}", m.arg_unquoted(1)),
        "SET_FLAG" => format!("{} {} = {}", a(0), var_label(doc, m.arg(0), m.arg(1)), a(2)),
        "ADD_TO_FLAG" => format!(
            "{} {} += {}",
            a(0),
            var_label(doc, m.arg(0), m.arg(1)),
            a(2)
        ),
        "SET_TIMER" => format!("Start {} {}", a(0), var_label(doc, m.arg(0), m.arg(1))),
        "WIN_GAME" => "WIN the level".into(),
        "LOSE_GAME" => "LOSE the level".into(),
        "ADD_GOLD_TO_PLAYER" => format!("{} +{} gold", a(0), a(1)),
        "REVEAL_MAP_LOCATION" => format!("Reveal {} for {} (range {})", loc(1), a(0), a(2)),
        "SET_NEXT_LEVEL" => format!("Next level {}", a(0)),
        _ => crate::export::action_line(doc, x),
    };
    format!("{s}{r}")
}

/// "When ... -> Do ..." summary of an event.
pub fn event_summary(doc: &LogicDoc, e: &Event) -> String {
    let when = if e.conditions.is_empty() {
        "Level start".to_string()
    } else {
        e.conditions
            .iter()
            .map(|c| condition_text(doc, c))
            .collect::<Vec<_>>()
            .join(" AND ")
    };
    let n = e.body.len();
    format!("When {when} -> {n} action(s)")
}

/// Notes of a node: its comment lines without `REM` / `/* */`.
pub fn notes_of(pre: &[String]) -> String {
    let mut out: Vec<String> = vec![];
    for l in pre {
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        let t = if t.len() >= 3 && t[..3].eq_ignore_ascii_case("REM") {
            t[3..].trim().to_string()
        } else {
            t.trim_start_matches("/*")
                .trim_end_matches("*/")
                .trim()
                .to_string()
        };
        if t.chars().all(|c| c == '*' || c == '-' || c == '=') {
            continue;
        }
        out.push(t);
    }
    out.join("\n")
}

/// Replace the comment lines of `pre` by `notes` (one `REM` line each), keeping blank lines
/// before them.
pub fn set_notes(pre: &mut Vec<String>, notes: &str) {
    let blanks = pre.iter().take_while(|l| l.trim().is_empty()).count();
    pre.truncate(blanks.min(pre.len()));
    for l in notes.lines() {
        if l.trim().is_empty() {
            pre.push("REM".into());
        } else {
            pre.push(format!("REM {l}"));
        }
    }
}

// ---- timing --------------------------------------------------------------------------------

/// How an event is gated by time.
#[derive(Clone, Debug, PartialEq)]
pub enum TimeGate {
    /// `IF(player, GAME_TURN >= n)` (or `>`, `==`).
    GameTurn { cond: NodeId, turn: i64 },
    /// `IF(player, TIMERn >= t)`: `t` turns after a `SET_TIMER` of that timer.
    Timer {
        cond: NodeId,
        player: String,
        timer: String,
        turns: i64,
        started_by: Vec<NodeId>,
    },
}

impl TimeGate {
    pub fn cond(&self) -> NodeId {
        match self {
            TimeGate::GameTurn { cond, .. } | TimeGate::Timer { cond, .. } => *cond,
        }
    }
}

/// The time gate of an event's own conditions (the first one that is time based).
pub fn time_gate(doc: &LogicDoc, e: &Event) -> Option<TimeGate> {
    for c in &e.conditions {
        if c.cmd.name != "IF" || !c.cmd.arg(4).is_empty() {
            continue;
        }
        let op = c.cmd.arg(2);
        let Some(v) = num(c.cmd.arg(3)) else { continue };
        let v = match op {
            ">=" | "==" | "=" => v,
            ">" => v + 1,
            _ => continue,
        };
        let var = c.cmd.arg(1).to_ascii_uppercase();
        if var == "GAME_TURN" {
            return Some(TimeGate::GameTurn {
                cond: c.id,
                turn: v,
            });
        }
        if let Some((VarFamily::Timer, _)) = VarFamily::parse(&var) {
            let player = c.cmd.arg(0).to_ascii_uppercase();
            let started_by: Vec<NodeId> = var_usage(doc)
                .into_iter()
                .filter(|u| {
                    u.access == Access::Write
                        && u.command == "SET_TIMER"
                        && u.var == var
                        && (u.player == player || u.player == "ALL_PLAYERS")
                })
                .map(|u| u.event)
                .collect();
            return Some(TimeGate::Timer {
                cond: c.id,
                player,
                timer: var,
                turns: v,
                started_by,
            });
        }
    }
    None
}

/// Earliest game turn an event can fire, when it follows from the logic alone: level-start
/// events are turn 0, `GAME_TURN` gates give their turn, timer gates add their delay to the
/// earliest start of the timer; nested events start no earlier than their parent.
pub fn earliest_turns(doc: &LogicDoc) -> std::collections::HashMap<NodeId, i64> {
    use std::collections::HashMap;
    let mut known: HashMap<NodeId, i64> = HashMap::new();
    let parent: HashMap<NodeId, NodeId> = {
        let mut m = HashMap::new();
        for (e, _) in doc.events() {
            for n in &e.body {
                if let Node::Event(c) = n {
                    m.insert(c.id, e.id);
                }
            }
        }
        m
    };
    let events: Vec<&Event> = doc.events().into_iter().map(|(e, _)| e).collect();
    for _ in 0..8 {
        let mut changed = false;
        for e in &events {
            let base = match parent.get(&e.id) {
                Some(p) => match known.get(p) {
                    Some(t) => Some(*t),
                    None => None,
                },
                None => Some(0),
            };
            let Some(base) = base else { continue };
            let t = if e.conditions.is_empty() {
                Some(base)
            } else {
                match time_gate(doc, e) {
                    Some(TimeGate::GameTurn { turn, .. }) if e.conditions.len() == 1 => {
                        Some(turn.max(base))
                    }
                    Some(TimeGate::Timer {
                        turns, started_by, ..
                    }) if e.conditions.len() == 1 => {
                        let starts: Vec<i64> = started_by
                            .iter()
                            .filter_map(|s| known.get(s).copied())
                            .collect();
                        if starts.is_empty() || starts.len() < started_by.len() {
                            None
                        } else {
                            Some(starts.iter().min().unwrap() + turns)
                        }
                    }
                    _ => None,
                }
            };
            if let Some(t) = t {
                if known.get(&e.id) != Some(&t) {
                    known.insert(e.id, t);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    known
}

// ---- resources -----------------------------------------------------------------------------

/// A resource meter (used vs game limit, `src/lvl_script.h:30-37`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meter {
    pub label: &'static str,
    pub used: usize,
    pub limit: usize,
}

/// Validation of the generated script.
#[derive(Clone, Debug)]
pub struct Check {
    pub diagnostics: Vec<kfx_script::Diagnostic>,
    pub meters: Vec<Meter>,
}

/// Validate the script the model generates.
pub fn check(doc: &LogicDoc, provider: &dyn ConstantProvider) -> Check {
    use kfx_script::validate::*;
    let bytes = export_bytes(doc);
    let d = kfx_script::Document::parse(&bytes);
    let r = kfx_script::validate(&d, provider);
    let s = r.stats;
    let meters = vec![
        Meter {
            label: "Conditions",
            used: s.conditions,
            limit: CONDITIONS_COUNT,
        },
        Meter {
            label: "Script values",
            used: s.values,
            limit: SCRIPT_VALUES_COUNT,
        },
        Meter {
            label: "Parties",
            used: s.parties,
            limit: CREATURE_PARTYS_COUNT,
        },
        Meter {
            label: "Party triggers",
            used: s.party_triggers,
            limit: PARTY_TRIGGERS_COUNT,
        },
        Meter {
            label: "Tunneller triggers",
            used: s.tunneller_triggers,
            limit: TUNNELLER_TRIGGERS_COUNT,
        },
        Meter {
            label: "Win conditions",
            used: s.win_conditions,
            limit: WIN_CONDITIONS_COUNT,
        },
        Meter {
            label: "Lose conditions",
            used: s.lose_conditions,
            limit: WIN_CONDITIONS_COUNT,
        },
    ];
    Check {
        diagnostics: r.diagnostics,
        meters,
    }
}

/// Line ranges (1-based, inclusive) of every node in the generated script, for mapping
/// diagnostics back to nodes and for "open in text editor".
pub fn node_lines(doc: &LogicDoc) -> Vec<(NodeId, usize, usize)> {
    // Re-export with each node measured: count lines exactly like export does.
    let text = crate::export::export(doc);
    let total = text.lines().count().max(1);
    let mut out = vec![];
    let mut line = 1usize;
    let count = |s: &str| s.split('\n').count();
    fn pre_lines(p: &[String]) -> usize {
        p.iter().map(|l| l.split('\n').count()).sum()
    }
    let vars_first = !doc.variables.is_empty();
    fn walk(
        nodes: &[Node],
        line: &mut usize,
        out: &mut Vec<(NodeId, usize, usize)>,
        first_vars: &mut usize,
        count: &dyn Fn(&str) -> usize,
        doc: &LogicDoc,
        depth: usize,
    ) {
        for n in nodes {
            *line += pre_lines(n.pre());
            if *first_vars > 0 {
                *line += *first_vars;
                *first_vars = 0;
            }
            let start = *line;
            match n {
                Node::Action(a) => {
                    if a.reusable {
                        *line += 1;
                    }
                    *line += a.raw.as_deref().map(count).unwrap_or(1);
                    out.push((a.id, start, *line - 1));
                }
                Node::Party(p) => {
                    *line += p.raw.as_deref().map(count).unwrap_or(1);
                    for m in &p.members {
                        *line += pre_lines(&m.pre);
                        let s = *line;
                        *line += m.raw.as_deref().map(count).unwrap_or(1);
                        out.push((m.id, s, *line - 1));
                    }
                    out.push((p.id, start, *line - 1));
                }
                Node::Event(e) => {
                    let text = crate::export::event_text(doc, e);
                    let _ = depth;
                    if !e.enabled {
                        *line += text.split('\n').count();
                        out.push((e.id, start, *line - 1));
                        continue;
                    }
                    for c in &e.conditions {
                        *line += pre_lines(&c.pre);
                        out.push((c.id, *line, *line));
                        *line += c.raw.as_deref().map(count).unwrap_or(1);
                    }
                    walk(&e.body, line, out, &mut 0, count, doc, depth + 1);
                    *line += pre_lines(&e.tail);
                    *line += e.conditions.iter().filter(|c| !c.unclosed).count();
                    out.push((e.id, start, *line - 1));
                }
            }
        }
    }
    let mut fv = if vars_first { doc.variables.len() } else { 0 };
    walk(&doc.nodes, &mut line, &mut out, &mut fv, &count, doc, 0);
    let _ = total;
    out
}
