//! Party strength and wave templates.
//!
//! Templates generate ordinary events (so they round-trip like hand-written ones):
//!
//! * [`EscalatingWaves`]: `count` waves of a party, the first at `first_turn`, then every
//!   `interval` turns, each wave spawning `copies + i * copies_step` copies
//!   (`ADD_PARTY_TO_LEVEL` copies argument, `src/lvl_script_commands_old.c:93`), or a tunneller
//!   party heading for a target.
//! * [`Ambush`]: when a player reaches an action point, a party appears there.
//! * [`BossOnCondition`]: when a variable comparison holds, a final party appears, with an
//!   optional objective text.
use crate::model::*;
use crate::syntax::quote;

/// Host-supplied creature facts.
pub trait CreatureStats {
    /// Fighting strength of `creature` at experience `level` (1..=10); `None` when unknown.
    fn strength(&self, creature: &str, level: u32) -> Option<f32>;
}

/// Fallback when the host has no stats: level-scaled constant (one unit per level).
pub struct FlatStats;

impl CreatureStats for FlatStats {
    fn strength(&self, _creature: &str, level: u32) -> Option<f32> {
        Some(level.max(1) as f32)
    }
}

/// Estimated total strength of a party, and how many members had no known strength.
pub fn party_strength(p: &Party, stats: &dyn CreatureStats) -> (f32, usize) {
    let mut total = 0.0;
    let mut unknown = 0;
    for m in &p.members {
        let level: u32 = crate::syntax::num(&m.level).unwrap_or(1).clamp(1, 10) as u32;
        match stats.strength(&m.creature, level) {
            Some(s) => total += s,
            None => unknown += 1,
        }
    }
    (total, unknown)
}

/// Where a wave spawns: a party at a location, or a tunneller party heading somewhere.
#[derive(Clone, Debug, PartialEq)]
pub enum Spawn {
    Party {
        location: String,
    },
    Tunneller {
        location: String,
        head_for: String,
        target: String,
        level: u32,
        gold: u32,
    },
}

impl Spawn {
    fn action(&self, owner: &str, party: &str, copies: u32) -> Cmd {
        match self {
            Spawn::Party { location } => Cmd::new(
                "ADD_PARTY_TO_LEVEL",
                &[owner, party, location, &copies.to_string()],
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
                    owner,
                    party,
                    location,
                    head_for,
                    target,
                    &level.to_string(),
                    &gold.to_string(),
                ],
            ),
        }
    }
}

/// Escalating waves every N turns.
#[derive(Clone, Debug, PartialEq)]
pub struct EscalatingWaves {
    pub owner: String,
    pub party: String,
    pub spawn: Spawn,
    /// Player whose `GAME_TURN` is tested (any keeper; the turn is global).
    pub clock_player: String,
    pub first_turn: u32,
    pub interval: u32,
    pub count: u32,
    pub copies: u32,
    pub copies_step: u32,
}

impl Default for EscalatingWaves {
    fn default() -> Self {
        EscalatingWaves {
            owner: "PLAYER_GOOD".into(),
            party: String::new(),
            spawn: Spawn::Party {
                location: "-1".into(),
            },
            clock_player: "PLAYER0".into(),
            first_turn: 6000,
            interval: 3000,
            count: 3,
            copies: 1,
            copies_step: 1,
        }
    }
}

impl EscalatingWaves {
    /// Build the events (not inserted).
    pub fn build(&self, doc: &mut LogicDoc) -> Vec<Event> {
        let mut out = vec![];
        for i in 0..self.count {
            let turn = self.first_turn + i * self.interval;
            let c = doc.new_condition(Cmd::new(
                "IF",
                &[&self.clock_player, "GAME_TURN", ">=", &turn.to_string()],
            ));
            let a = doc.new_action(self.spawn.action(
                &self.owner,
                &self.party,
                self.copies + i * self.copies_step,
            ));
            let mut e = doc.new_event(vec![c], vec![Node::Action(a)]);
            e.pre = vec![
                String::new(),
                format!("REM Wave {} of {}: party {}", i + 1, self.count, self.party),
            ];
            out.push(e);
        }
        out
    }
}

/// A party ambush when a player reaches an action point.
#[derive(Clone, Debug, PartialEq)]
pub struct Ambush {
    pub owner: String,
    pub party: String,
    pub action_point: u32,
    pub player: String,
    /// Spawn location (defaults to the action point itself when empty).
    pub location: String,
    pub copies: u32,
    pub message: Option<String>,
}

impl Ambush {
    pub fn build(&self, doc: &mut LogicDoc) -> Event {
        let c = doc.new_condition(Cmd::new(
            "IF_ACTION_POINT",
            &[&self.action_point.to_string(), &self.player],
        ));
        let loc = if self.location.trim().is_empty() {
            self.action_point.to_string()
        } else {
            self.location.clone()
        };
        let mut body = vec![Node::Action(doc.new_action(Cmd::new(
            "ADD_PARTY_TO_LEVEL",
            &[&self.owner, &self.party, &loc, &self.copies.to_string()],
        )))];
        if let Some(m) = &self.message {
            let q = free_quick_index(doc).to_string();
            body.push(Node::Action(doc.new_action(Cmd::new(
                "QUICK_INFORMATION",
                &[&q, &quote(m), &loc],
            ))));
        }
        let mut e = doc.new_event(vec![c], body);
        e.pre = vec![
            String::new(),
            format!("REM Ambush at action point {}", self.action_point),
        ];
        e
    }
}

/// The final boss party when a condition holds.
#[derive(Clone, Debug, PartialEq)]
pub struct BossOnCondition {
    pub owner: String,
    pub party: String,
    pub location: String,
    pub player: String,
    pub variable: String,
    pub op: String,
    pub value: String,
    pub objective: Option<String>,
}

impl BossOnCondition {
    pub fn build(&self, doc: &mut LogicDoc) -> Event {
        let c = doc.new_condition(Cmd::new(
            "IF",
            &[&self.player, &self.variable, &self.op, &self.value],
        ));
        let mut body = vec![Node::Action(doc.new_action(Cmd::new(
            "ADD_PARTY_TO_LEVEL",
            &[&self.owner, &self.party, &self.location, "1"],
        )))];
        if let Some(t) = &self.objective {
            let q = free_quick_index(doc).to_string();
            body.push(Node::Action(doc.new_action(Cmd::new(
                "QUICK_OBJECTIVE",
                &[&q, &quote(t), &self.location],
            ))));
        }
        let mut e = doc.new_event(vec![c], body);
        e.pre = vec![String::new(), format!("REM Final party {}", self.party)];
        e
    }
}

/// First quick message index (`QUICK_*` commands, 0..`QUICK_MESSAGES_COUNT`=50,
/// `src/lvl_script_lib.h`) no command of the document uses; the game keeps one text per index.
pub fn free_quick_index(doc: &LogicDoc) -> u32 {
    let mut used = std::collections::HashSet::new();
    for (e, _) in doc.events() {
        for n in &e.body {
            if let Node::Action(a) = n {
                if a.cmd.name.starts_with("QUICK_") || a.cmd.name == "HEART_LOST_QUICK_OBJECTIVE" {
                    if let Some(i) = crate::syntax::num(a.cmd.arg(0)) {
                        used.insert(i);
                    }
                }
            }
        }
    }
    (0..50u32)
        .find(|i| !used.contains(&(*i as i64)))
        .unwrap_or(49)
}

/// Spawn and head-for locations of every action that brings `party` into the level.
pub fn party_routes(
    doc: &LogicDoc,
    party: &str,
) -> Vec<(
    NodeId,
    Option<crate::analysis::MapLoc>,
    Option<crate::analysis::MapLoc>,
)> {
    use crate::analysis::{cmd_locations, LocRole};
    let mut out = vec![];
    for (e, _) in doc.events() {
        for n in &e.body {
            let Node::Action(a) = n else { continue };
            if a.opaque
                || !matches!(
                    a.cmd.name.as_str(),
                    "ADD_PARTY_TO_LEVEL" | "ADD_TUNNELLER_PARTY_TO_LEVEL"
                )
                || !a.cmd.arg(1).eq_ignore_ascii_case(party)
            {
                continue;
            }
            let locs = cmd_locations(&a.cmd, doc.version, a.id, false);
            let spawn = locs
                .iter()
                .find(|l| l.role == LocRole::SpawnAt)
                .map(|l| l.loc.clone());
            let head = locs
                .iter()
                .find(|l| l.role == LocRole::HeadFor)
                .map(|l| l.loc.clone());
            out.push((a.id, spawn, head));
        }
    }
    out
}
