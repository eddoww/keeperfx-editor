//! Checking and listing the constants valid for an argument kind.
//!
//! Behaviour follows `script_command_param_to_number` (src/lvl_script.c:236), `get_rid`/`get_id`
//! (src/config.c:1214, 1250: case-insensitive; `get_rid` also accepts `RANDOM`), `parse_get_varib`
//! (src/lvl_script_commands.c:526) and `get_map_location_id` (src/map_locations.c:396).
use crate::diag::Severity;
use crate::overlay::{ArgKind, Overlay};
use crate::provider::{ConstKind, ConstantProvider};
use crate::tables::Tables;
use std::collections::HashMap;

/// Names the script itself declares (`NEW_*_TYPE`, `COPY_CREATURE_TYPE`); upper case.
#[derive(Default, Clone, Debug)]
pub struct Extras {
    pub names: HashMap<ConstKind, Vec<String>>,
}

pub struct Env<'a> {
    pub provider: &'a dyn ConstantProvider,
    pub extras: &'a Extras,
    /// `level_file_version` in effect.
    pub version: i64,
}

/// A problem found while checking one value.
#[derive(Clone, Debug)]
pub struct Problem {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    /// The game discards the whole command.
    pub fatal: bool,
}

fn err(code: &'static str, message: String) -> Option<Problem> {
    Some(Problem {
        severity: Severity::Error,
        code,
        message,
        fatal: true,
    })
}
fn err_nonfatal(code: &'static str, message: String) -> Option<Problem> {
    Some(Problem {
        severity: Severity::Error,
        code,
        message,
        fatal: false,
    })
}
fn warn(code: &'static str, message: String) -> Option<Problem> {
    Some(Problem {
        severity: Severity::Warning,
        code,
        message,
        fatal: false,
    })
}

pub fn is_number(s: &str) -> bool {
    // parameter_is_number: optional '-' then digits
    let t = s.strip_prefix('-').unwrap_or(s);
    !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit())
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl<'a> Env<'a> {
    fn tbl(&self, name: &str) -> &'static [String] {
        Tables::get().named(name)
    }
    fn in_tbl(&self, name: &str, text: &str) -> bool {
        self.tbl(name).iter().any(|n| eq(n, text))
    }

    /// Membership in a config-defined set: `None` when unknown.
    pub fn in_const(&self, kind: ConstKind, text: &str) -> Option<bool> {
        let base = self.provider.names(kind);
        let ex = self
            .extras
            .names
            .get(&kind)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        if base.is_empty() {
            return None;
        }
        Some(base.iter().chain(ex.iter()).any(|n| eq(n, text)))
    }

    pub fn const_names(&self, kind: ConstKind) -> Vec<String> {
        let mut v: Vec<String> = self
            .provider
            .names(kind)
            .iter()
            .map(|s| s.to_ascii_uppercase())
            .collect();
        if let Some(e) = self.extras.names.get(&kind) {
            v.extend(e.iter().cloned());
        }
        v.sort();
        v.dedup();
        v
    }

    fn const_ok(&self, kind: ConstKind, text: &str, random: bool) -> bool {
        if random && text == "RANDOM" {
            return true;
        }
        self.in_const(kind, text).unwrap_or(true)
    }

    fn player_ok(&self, text: &str) -> bool {
        text == "RANDOM"
            || self.in_tbl("player_desc", text)
            || self.in_tbl("cmpgn_human_player_options", text)
    }

    fn variable_list(&self) -> &'static [String] {
        if self.version > 0 {
            self.tbl("variable_desc")
        } else {
            self.tbl("dk1_variable_desc")
        }
    }

    /// `KIND[arg]` forms: returns (head, inner) when `text` looks like `HEAD[INNER]`.
    fn bracket(text: &str) -> Option<(&str, &str)> {
        let o = text.find('[')?;
        let c = text[o..].find(']')? + o;
        Some((&text[..o], &text[o + 1..c]))
    }

    fn numbered(text: &str, prefix: &str, suffix: &str) -> bool {
        text.strip_prefix(prefix)
            .and_then(|r| r.strip_suffix(suffix))
            .map(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            .unwrap_or(false)
    }

    /// `parse_set_varib` (src/lvl_script_commands.c:482).
    fn flag_var_ok(&self, text: &str) -> bool {
        if self.in_tbl("flag_desc", text) || self.in_tbl("campaign_flag_desc", text) {
            return true;
        }
        if Self::numbered(text, "BOX", "_ACTIVATED") || Self::numbered(text, "TRAP", "_ACTIVATED") {
            return true;
        }
        if let Some((h, inner)) = Self::bracket(text) {
            if h == "SACRIFICED" || h == "REWARDED" {
                return self.const_ok(ConstKind::Creature, inner, false);
            }
        }
        false
    }

    /// `parse_get_varib` (src/lvl_script_commands.c:526).
    fn variable_ok(&self, text: &str) -> bool {
        if self.variable_list().iter().any(|n| eq(n, text)) {
            return true;
        }
        for k in [
            ConstKind::Creature,
            ConstKind::Room,
            ConstKind::Door,
            ConstKind::Trap,
        ] {
            // unknown provider kind: cannot reject
            if self.in_const(k, text).unwrap_or(true) {
                return true;
            }
        }
        if self.in_tbl("timer_desc", text)
            || self.in_tbl("flag_desc", text)
            || self.in_tbl("campaign_flag_desc", text)
        {
            return true;
        }
        if Self::numbered(text, "BOX", "_ACTIVATED") || Self::numbered(text, "TRAP", "_ACTIVATED") {
            return true;
        }
        if let Some((h, inner)) = Self::bracket(text) {
            match h {
                "KEEPERS_DESTROYED" | "BATTLES_WON" | "BATTLES_LOST" => {
                    return self.in_tbl("player_desc", inner)
                        || self.in_tbl("cmpgn_human_player_options", inner);
                }
                "SACRIFICED" | "REWARDED" => {
                    return self.in_const(ConstKind::Creature, inner).unwrap_or(true)
                }
                _ => {}
            }
        }
        false
    }

    fn avail_variable_ok(&self, text: &str) -> bool {
        if self.in_tbl("available_variable_desc", text) {
            return true;
        }
        [
            ConstKind::Door,
            ConstKind::Trap,
            ConstKind::Room,
            ConstKind::Power,
            ConstKind::Creature,
        ]
        .iter()
        .any(|&k| self.in_const(k, text).unwrap_or(true))
    }

    fn ctrl_variable_ok(&self, text: &str) -> bool {
        self.in_tbl("controls_variable_desc", text)
            || self.in_const(ConstKind::Creature, text).unwrap_or(true)
    }

    /// Action point number to AP existence (`None` when the map is unknown).
    pub fn ap_exists(&self, n: i64) -> Option<bool> {
        if !self.provider.action_points_known() {
            return None;
        }
        Some(self.provider.action_points().iter().any(|&a| a as i64 == n))
    }

    pub fn gate_exists(&self, n: i64) -> Option<bool> {
        if !self.provider.hero_gates_known() {
            return None;
        }
        Some(self.provider.hero_gates().iter().any(|&a| a as i64 == n))
    }

    fn check_location(&self, text: &str) -> Option<Problem> {
        if text.is_empty() {
            return None;
        }
        if self.player_ok_loc(text)
            || self.in_const(ConstKind::Creature, text) == Some(true)
            || self.in_const(ConstKind::Room, text) == Some(true)
        {
            return None;
        }
        if text == "LAST_EVENT" {
            return None;
        }
        for pre in ["COMBAT", "LAST_DEATH_EVENT", "LAST_TRAP_EVENT", "CTA"] {
            if text.starts_with(pre) {
                if text == pre {
                    return None;
                }
                // get_player_name_from_location_string: needs [PLAYER]
                if let Some((_, inner)) = Self::bracket(text) {
                    if self.player_ok(inner) {
                        return None;
                    }
                }
                return err(
                    "bad-location",
                    format!("Invalid LOCATION = '{text}' (expected {pre} or {pre}[PLAYERn])"),
                );
            }
        }
        // atol(): leading integer
        let (neg, rest) = match text.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let n: i64 = digits.parse().unwrap_or(0);
        if digits.is_empty() || n == 0 {
            return err_nonfatal(
                "bad-location",
                format!(
                    "Invalid LOCATION = '{text}': not a \
                player, creature, room, action point (>0) or hero gate (<0)"
                ),
            );
        }
        if neg {
            if self.gate_exists(n) == Some(false) {
                return err(
                    "missing-hero-gate",
                    format!("Non-existing Hero Door, no {n}"),
                );
            }
        } else if self.ap_exists(n) == Some(false) {
            return err(
                "missing-action-point",
                format!("Non-existing Action Point, no {n}"),
            );
        }
        None
    }

    fn player_ok_loc(&self, text: &str) -> bool {
        self.player_ok(text)
    }

    /// Check `text` against `kind`. `letter` is the table's type letter (upper case).
    pub fn check(&self, kind: ArgKind, letter: u8, extended: bool, text: &str) -> Option<Problem> {
        let ov = Overlay::get();
        let bad = |code: &'static str, what: &str| err(code, format!("Unknown {what}, \"{text}\""));
        let in_named = |t: &str| self.in_tbl(t, text);
        let random_named = |t: &str| text == "RANDOM" || self.in_tbl(t, text);
        let in_list = |l: &str| ov.list(l).iter().any(|n| eq(n, text));
        match kind {
            ArgKind::Player | ArgKind::PlayerColor => {
                if kind == ArgKind::PlayerColor {
                    if random_named("cmpgn_human_player_options") || is_number(text) {
                        return None;
                    }
                    return bad("unknown-constant", "colour");
                }
                if self.player_ok(text) {
                    None
                } else {
                    err("unknown-player", format!("Invalid player name, '{text}'"))
                }
            }
            ArgKind::Creature => {
                if extended && text == "ANY_CREATURE" {
                    return None;
                }
                if self.const_ok(ConstKind::Creature, text, true) {
                    None
                } else {
                    bad("unknown-creature", "creature")
                }
            }
            ArgKind::Room => (!self.const_ok(ConstKind::Room, text, true))
                .then(|| bad("unknown-room", "room kind"))
                .flatten(),
            ArgKind::Slab => (!self.const_ok(ConstKind::Slab, text, true))
                .then(|| bad("unknown-slab", "slab kind"))
                .flatten(),
            ArgKind::Trap => (!self.const_ok(ConstKind::Trap, text, true))
                .then(|| bad("unknown-trap", "trap"))
                .flatten(),
            ArgKind::Door => (!self.const_ok(ConstKind::Door, text, true))
                .then(|| bad("unknown-door", "door"))
                .flatten(),
            ArgKind::Power => (!self.const_ok(ConstKind::Power, text, true))
                .then(|| bad("unknown-power", "power"))
                .flatten(),
            ArgKind::Spell => (!self.const_ok(ConstKind::Spell, text, true))
                .then(|| bad("unknown-spell", "spell"))
                .flatten(),
            ArgKind::Object => (!self.const_ok(ConstKind::Object, text, true))
                .then(|| bad("unknown-object", "object"))
                .flatten(),
            ArgKind::Instance => (!self.const_ok(ConstKind::Instance, text, true))
                .then(|| bad("unknown-instance", "creature instance"))
                .flatten(),
            ArgKind::Effect => {
                if is_number(text) || self.const_ok(ConstKind::Effect, text, false) {
                    None
                } else {
                    bad("unknown-effect", "effect")
                }
            }
            ArgKind::EffectGen => {
                if is_number(text) || self.const_ok(ConstKind::EffectGen, text, false) {
                    None
                } else {
                    bad("unknown-effect-generator", "effect generator")
                }
            }
            ArgKind::Operator => (!(text == "RANDOM" || in_named("comparison_desc")))
                .then(|| bad("unknown-operator", "operator"))
                .flatten(),
            ArgKind::Bool => {
                if text == "RANDOM" || in_named("script_boolean_desc") || extended {
                    None
                } else {
                    bad("unknown-boolean", "boolean value")
                }
            }
            ArgKind::Location => self.check_location(text),
            ArgKind::Timer => (!random_named("timer_desc"))
                .then(|| bad("unknown-timer", "timer"))
                .flatten(),
            ArgKind::CampaignFlag => (!random_named("campaign_flag_desc"))
                .then(|| bad("unknown-flag", "campaign flag"))
                .flatten(),
            ArgKind::FlagVar => (!self.flag_var_ok(text))
                .then(|| bad("unknown-flag", "flag"))
                .flatten(),
            ArgKind::Variable => (!self.variable_ok(text))
                .then(|| bad("unknown-variable", "variable"))
                .flatten(),
            ArgKind::AvailVariable => (!self.avail_variable_ok(text))
                .then(|| bad("unknown-variable", "variable"))
                .flatten(),
            ArgKind::CtrlVariable => (!self.ctrl_variable_ok(text))
                .then(|| bad("unknown-variable", "variable"))
                .flatten(),
            ArgKind::TexturePack => (!(is_number(text) || random_named("texture_pack_desc")))
                .then(|| bad("unknown-constant", "texture pack"))
                .flatten(),
            ArgKind::HeroObjective => {
                let head = text.split('[').next().unwrap_or(text);
                if text == "RANDOM" || in_named_head(self, "hero_objective_desc", head) {
                    None
                } else {
                    bad("unknown-constant", "party member objective")
                }
            }
            ArgKind::Criterion => {
                if in_named("creature_select_criteria_desc") {
                    return None;
                }
                if let Some((h, inner)) = Self::bracket(text) {
                    if h == "AT_ACTION_POINT" && is_number(inner) {
                        if self.ap_exists(inner.parse().unwrap_or(0)) == Some(false) {
                            return err(
                                "missing-action-point",
                                format!("Unknown action point at criteria, '{text}'"),
                            );
                        }
                        return None;
                    }
                }
                bad("unknown-constant", "select criteria")
            }
            ArgKind::FillType => {
                if text.is_empty() || in_named("fill_desc") {
                    None
                } else {
                    warn(
                        "unknown-constant",
                        format!("Fill type {text} not recognized"),
                    )
                }
            }
            ArgKind::LockState => (!(in_named("locked_desc") || in_named("script_boolean_desc")))
                .then(|| bad("unknown-constant", "lock state"))
                .flatten(),
            ArgKind::FreeState => (!(in_named("is_free_desc") || in_named("script_boolean_desc")))
                .then(|| bad("unknown-constant", "free state"))
                .flatten(),
            ArgKind::Orientation => (!(is_number(text) || in_named("orientation_desc")))
                .then(|| bad("unknown-constant", "orientation"))
                .flatten(),
            ArgKind::SpawnType => (!(text == "RANDOM" || in_list("spawn_types")))
                .then(|| bad("unknown-constant", "spawn type"))
                .flatten(),
            ArgKind::ResearchType => (!(text == "RANDOM" || in_list("research_types")))
                .then(|| bad("unknown-constant", "research type"))
                .flatten(),
            ArgKind::SacrificeAction => (!(text == "RANDOM" || in_list("sacrifice_actions")))
                .then(|| bad("unknown-constant", "sacrifice action"))
                .flatten(),
            ArgKind::CreatureProperty => (!(text == "RANDOM" || in_list("creature_properties")))
                .then(|| bad("unknown-constant", "creature property"))
                .flatten(),
            ArgKind::Tendency => (!(text == "RANDOM" || in_named("tendency_desc")))
                .then(|| bad("unknown-constant", "tendency"))
                .flatten(),
            ArgKind::Modifier => (!in_named("modifier_desc"))
                .then(|| bad("unknown-constant", "player modifier"))
                .flatten(),
            ArgKind::OnExperience => (!in_named("on_experience_desc"))
                .then(|| bad("unknown-constant", "experience variable"))
                .flatten(),
            ArgKind::MsgType => (!in_named("msgtype_desc"))
                .then(|| bad("unknown-constant", "message type"))
                .flatten(),
            ArgKind::HandRule => (!in_named("hand_rule_desc"))
                .then(|| bad("unknown-constant", "hand rule"))
                .flatten(),
            ArgKind::RuleSlot => (!in_named("rule_slot_desc"))
                .then(|| bad("unknown-constant", "hand rule slot"))
                .flatten(),
            ArgKind::RuleAction => (!in_named("rule_action_desc"))
                .then(|| bad("unknown-constant", "hand rule action"))
                .flatten(),
            ArgKind::HeadFor => (!(text == "RANDOM" || in_named("head_for_desc")))
                .then(|| bad("unknown-constant", "heading objective"))
                .flatten(),
            ArgKind::ScriptOperator => (!(text == "RANDOM" || in_named("script_operator_desc")))
                .then(|| bad("unknown-constant", "operation"))
                .flatten(),
            ArgKind::ComputerModel => {
                if is_number(text) || eq(text, "ROAMING") || eq(text, "OFF") {
                    None
                } else {
                    err(
                        "unknown-constant",
                        format!("invalid COMPUTER_PLAYER param '{text}'"),
                    )
                }
            }
            ArgKind::ConcealMode => {
                if text.is_empty() || text == "ALL" || in_named("script_boolean_desc") {
                    None
                } else {
                    warn(
                        "unknown-constant",
                        format!("Hide value \"{text}\" not recognized"),
                    )
                }
            }
            ArgKind::Number
            | ArgKind::Count
            | ArgKind::Amount
            | ArgKind::Level
            | ArgKind::Turns
            | ArgKind::SubtileX
            | ArgKind::SubtileY
            | ArgKind::SlabX
            | ArgKind::SlabY
            | ArgKind::ApNumber
            | ArgKind::HeroGate
            | ArgKind::BoxNumber
            | ArgKind::QuickMsg => {
                if letter == b'N' && !extended {
                    let (v, whole) = crate::doc::strtol0(text);
                    if !whole {
                        return warn(
                            "number-format",
                            format!("Numerical value \"{text}\" interpreted as {v}"),
                        );
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Candidate constants for completion lists. Empty when the kind is free-form.
    pub fn candidates(&self, kind: ArgKind, extended: bool) -> Vec<String> {
        let ov = Overlay::get();
        let t = |n: &str| Tables::get().named(n).to_vec();
        let c = |k: ConstKind| self.const_names(k);
        let mut v: Vec<String> = match kind {
            ArgKind::Player => [t("player_desc"), t("cmpgn_human_player_options")].concat(),
            ArgKind::PlayerColor => t("cmpgn_human_player_options"),
            ArgKind::Creature => {
                let mut v = c(ConstKind::Creature);
                if extended {
                    v.push("ANY_CREATURE".into());
                }
                v
            }
            ArgKind::Room => c(ConstKind::Room),
            ArgKind::Slab => c(ConstKind::Slab),
            ArgKind::Trap => c(ConstKind::Trap),
            ArgKind::Door => c(ConstKind::Door),
            ArgKind::Power => c(ConstKind::Power),
            ArgKind::Spell => c(ConstKind::Spell),
            ArgKind::Object => c(ConstKind::Object),
            ArgKind::Instance => c(ConstKind::Instance),
            ArgKind::Effect => c(ConstKind::Effect),
            ArgKind::EffectGen => c(ConstKind::EffectGen),
            ArgKind::Operator => t("comparison_desc"),
            ArgKind::Bool => t("script_boolean_desc"),
            ArgKind::Location => {
                let mut v = t("player_desc");
                v.extend(c(ConstKind::Creature));
                v.extend(c(ConstKind::Room));
                v.extend(
                    [
                        "LAST_EVENT",
                        "COMBAT",
                        "LAST_DEATH_EVENT",
                        "LAST_TRAP_EVENT",
                        "CTA",
                    ]
                    .map(String::from),
                );
                v.extend(self.provider.action_points().iter().map(|a| a.to_string()));
                v.extend(
                    self.provider
                        .hero_gates()
                        .iter()
                        .map(|g| (-g.abs()).to_string()),
                );
                v
            }
            ArgKind::Timer => t("timer_desc"),
            ArgKind::CampaignFlag => t("campaign_flag_desc"),
            ArgKind::FlagVar => [t("flag_desc"), t("campaign_flag_desc")].concat(),
            ArgKind::Variable => {
                let mut v = self.variable_list().to_vec();
                v.extend(c(ConstKind::Creature));
                v.extend(c(ConstKind::Room));
                v.extend(t("timer_desc"));
                v.extend(t("flag_desc"));
                v.extend(c(ConstKind::Door));
                v.extend(c(ConstKind::Trap));
                v.extend(t("campaign_flag_desc"));
                v
            }
            ArgKind::AvailVariable => {
                let mut v = t("available_variable_desc");
                for k in [
                    ConstKind::Door,
                    ConstKind::Trap,
                    ConstKind::Room,
                    ConstKind::Power,
                    ConstKind::Creature,
                ] {
                    v.extend(c(k));
                }
                v
            }
            ArgKind::CtrlVariable => {
                let mut v = t("controls_variable_desc");
                v.extend(c(ConstKind::Creature));
                v
            }
            ArgKind::TexturePack => t("texture_pack_desc"),
            ArgKind::HeroObjective => t("hero_objective_desc"),
            ArgKind::Criterion => t("creature_select_criteria_desc"),
            ArgKind::FillType => t("fill_desc"),
            ArgKind::LockState => t("locked_desc"),
            ArgKind::FreeState => t("is_free_desc"),
            ArgKind::Orientation => t("orientation_desc"),
            ArgKind::SpawnType => ov.list("spawn_types").to_vec(),
            ArgKind::ResearchType => ov.list("research_types").to_vec(),
            ArgKind::SacrificeAction => ov.list("sacrifice_actions").to_vec(),
            ArgKind::SacrificeParam => [
                ov.list("sacrifice_unique").to_vec(),
                c(ConstKind::Creature),
                t("flag_desc"),
            ]
            .concat(),
            ArgKind::CreatureProperty => ov.list("creature_properties").to_vec(),
            ArgKind::Tendency => t("tendency_desc"),
            ArgKind::Modifier => t("modifier_desc"),
            ArgKind::OnExperience => t("on_experience_desc"),
            ArgKind::MsgType => t("msgtype_desc"),
            ArgKind::HandRule => t("hand_rule_desc"),
            ArgKind::RuleSlot => t("rule_slot_desc"),
            ArgKind::RuleAction => t("rule_action_desc"),
            ArgKind::HeadFor => t("head_for_desc"),
            ArgKind::ScriptOperator => t("script_operator_desc"),
            ArgKind::ComputerModel => vec!["ROAMING".into(), "OFF".into()],
            ArgKind::ConcealMode => vec!["ALL".into()],
            ArgKind::GuiButton => {
                let mut v = c(ConstKind::Room);
                v.extend(c(ConstKind::Power));
                v.extend(c(ConstKind::Trap));
                v.extend(c(ConstKind::Door));
                v
            }
            _ => vec![],
        };
        v.sort();
        v.dedup();
        v
    }
}

fn in_named_head(env: &Env, table: &str, head: &str) -> bool {
    env.in_tbl(table, head)
}
