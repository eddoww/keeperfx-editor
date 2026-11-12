//! Names and value rules of the config properties a level script can change, taken from the
//! embedded `data/config_schema.json` (made by `tools/extract_config_schema.py` from the
//! `NamedField` tables of the KeeperFX C source).
//!
//! * `SET_GAME_RULE` looks the rule up in the `ruleblocks` of `src/config_rules.c:220`
//!   (`rules.cfg` sections game, rooms, magic, creatures, computer, workers, health, plus the
//!   script-only `PayDayProgress`) with `get_named_field_id` (case-insensitive, src/config.c:1196) and
//!   parses the value with the field's parser (`value_default`, src/config.c:438: a number is clamped
//!   to the field's min/max with a warning, a non-number is accepted only when the field has a name
//!   list, else the value becomes 0 with a warning).
//! * `SET_ROOM/TRAP/DOOR/OBJECT/EFFECT_GENERATOR_CONFIGURATION` use `set_config_check`
//!   (src/lvl_script_commands.c:637): the id by name, then the property in the block's field table.
//! * `SET_CREATURE_CONFIGURATION` tries the nine creature blocks in a fixed order
//!   (src/lvl_script_commands.c:2418), `SET_POWER_CONFIGURATION` the `magic_power_commands` list
//!   (src/config_magic.c:304).
//!
//! Fields the schema marks `unknown` (present in shipped files but not in this version's C
//! source) are not offered: the game does not know them either.
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

const SCHEMA_JSON: &str = include_str!("../../../data/config_schema.json");

/// How the value of a property is read.
#[derive(Clone, Debug, PartialEq)]
pub enum ValueKind {
    /// A number; `min`/`max` are the clamps of `value_default` (`None`: not clamped here).
    Int { min: Option<i64>, max: Option<i64> },
    /// One of a list of names (`enum` is the C list; empty when it is built at run time).
    Enum { name: String, members: Vec<String> },
    /// Names combined with spaces, or a number (`value_flagsfield`).
    Flags { name: String, members: Vec<String> },
    /// Anything else (strings, references to other blocks, ...): not checked.
    Other(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PropArg {
    pub kind: ValueKind,
    pub label: String,
}

/// One rule or property.
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    pub name: String,
    pub doc: String,
    /// Config block it comes from (`game`, `attributes`, ...).
    pub block: String,
    /// The C parse function (`value_default`, `value_flagsfield`, ...), when known.
    pub parse: Option<String>,
    pub args: Vec<PropArg>,
    /// `file:line` in the C source.
    pub source: Option<String>,
}

/// What a `SET_*_CONFIGURATION` command changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfigTarget {
    Creature,
    Object,
    Trap,
    Door,
    Room,
    Power,
    EffectGen,
}

impl ConfigTarget {
    /// The target of a command name (`SET_TRAP_CONFIGURATION` ...).
    pub fn of_command(cmd: &str) -> Option<ConfigTarget> {
        Some(match cmd {
            "SET_CREATURE_CONFIGURATION" => ConfigTarget::Creature,
            "SET_OBJECT_CONFIGURATION" => ConfigTarget::Object,
            "SET_TRAP_CONFIGURATION" => ConfigTarget::Trap,
            "SET_DOOR_CONFIGURATION" => ConfigTarget::Door,
            "SET_ROOM_CONFIGURATION" => ConfigTarget::Room,
            "SET_POWER_CONFIGURATION" => ConfigTarget::Power,
            "SET_EFFECT_GENERATOR_CONFIGURATION" => ConfigTarget::EffectGen,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            ConfigTarget::Creature => "creature",
            ConfigTarget::Object => "object",
            ConfigTarget::Trap => "trap",
            ConfigTarget::Door => "door",
            ConfigTarget::Room => "room",
            ConfigTarget::Power => "power",
            ConfigTarget::EffectGen => "effect generator",
        }
    }
}

pub struct Schema {
    rules: Vec<Prop>,
    targets: HashMap<ConfigTarget, Vec<Prop>>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string())
}

fn enum_members(enums: &Value, name: &str) -> Vec<String> {
    enums
        .get(name)
        .and_then(|e| e.as_array())
        .map(|a| a.iter().filter_map(|m| s(m, "name")).collect())
        .unwrap_or_default()
}

fn parse_arg(a: &Value, enums: &Value) -> PropArg {
    let ty = s(a, "type").unwrap_or_default();
    let label = s(a, "label").unwrap_or_default();
    let en = s(a, "enum");
    let kind = match (ty.as_str(), en) {
        ("int", None) => ValueKind::Int {
            min: a.get("min").and_then(|x| x.as_i64()),
            max: a.get("max").and_then(|x| x.as_i64()),
        },
        ("enum", Some(n)) => ValueKind::Enum {
            members: enum_members(enums, &n),
            name: n,
        },
        ("flags", Some(n)) => ValueKind::Flags {
            members: enum_members(enums, &n),
            name: n,
        },
        (t, _) => ValueKind::Other(t.to_string()),
    };
    PropArg { kind, label }
}

fn parse_key(k: &Value, block: &str, enums: &Value) -> Option<Prop> {
    if s(k, "confidence").as_deref() == Some("unknown") {
        return None;
    }
    Some(Prop {
        name: s(k, "name")?,
        doc: s(k, "doc").unwrap_or_default(),
        block: block.to_string(),
        parse: s(k, "parse"),
        args: k
            .get("args")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().map(|x| parse_arg(x, enums)).collect())
            .unwrap_or_default(),
        source: s(k, "source"),
    })
}

fn section_keys(root: &Value, file: &str, section: &str, enums: &Value) -> Vec<Prop> {
    let Some(files) = root.get("files").and_then(|f| f.as_array()) else {
        return vec![];
    };
    let Some(f) = files.iter().find(|f| s(f, "file").as_deref() == Some(file)) else {
        return vec![];
    };
    let Some(secs) = f.get("sections").and_then(|x| x.as_array()) else {
        return vec![];
    };
    let Some(sec) = secs
        .iter()
        .find(|x| s(x, "name").as_deref() == Some(section))
    else {
        return vec![];
    };
    sec.get("keys")
        .and_then(|k| k.as_array())
        .map(|ks| {
            ks.iter()
                .filter_map(|k| parse_key(k, section, enums))
                .collect()
        })
        .unwrap_or_default()
}

impl Schema {
    pub fn get() -> &'static Schema {
        static S: OnceLock<Schema> = OnceLock::new();
        S.get_or_init(|| {
            let root: Value =
                serde_json::from_str(SCHEMA_JSON).expect("embedded config_schema.json");
            let enums = root.get("enums").cloned().unwrap_or(Value::Null);
            let mut rules = vec![];
            // ruleblocks (src/config_rules.c:220): game, rooms, magic, creatures, computer, workers,
            // health
            for b in [
                "game",
                "rooms",
                "magic",
                "creatures",
                "computer",
                "workers",
                "health",
            ] {
                rules.extend(section_keys(&root, "rules.cfg", b, &enums));
            }
            // rules_script_only_named_fields (src/config_rules.c:210): not in rules.cfg
            rules.push(Prop {
                name: "PayDayProgress".into(),
                doc: "Progress towards the next pay day, in game turns (can only be set from a \
                    script)."
                    .into(),
                block: "script".into(),
                parse: Some("value_default".into()),
                args: vec![PropArg {
                    kind: ValueKind::Int {
                        min: Some(0),
                        max: Some(i32::MAX as i64),
                    },
                    label: "pay_day_progress".into(),
                }],
                source: Some("src/config_rules.c:210".into()),
            });
            let mut targets = HashMap::new();
            targets.insert(
                ConfigTarget::Room,
                section_keys(&root, "terrain.cfg", "room", &enums),
            );
            targets.insert(
                ConfigTarget::Trap,
                section_keys(&root, "trapdoor.cfg", "trap", &enums),
            );
            targets.insert(
                ConfigTarget::Door,
                section_keys(&root, "trapdoor.cfg", "door", &enums),
            );
            targets.insert(
                ConfigTarget::Object,
                section_keys(&root, "objects.cfg", "object", &enums),
            );
            targets.insert(
                ConfigTarget::Power,
                section_keys(&root, "magic.cfg", "power", &enums),
            );
            targets.insert(
                ConfigTarget::EffectGen,
                section_keys(&root, "effects.toml", "effectGenerator", &enums),
            );
            // creature blocks in the order of set_creature_configuration_check
            // (src/lvl_script_commands.c:2418)
            let mut creature: Vec<Prop> = vec![];
            for (block, list) in [
                ("attributes", "creatmodel_attributes_commands"),
                ("jobs", "creatmodel_jobs_commands"),
                ("attraction", "creatmodel_attraction_commands"),
                ("sounds", "creatmodel_sounds_commands"),
                ("sprites", "creature_graphics_desc"),
                ("annoyance", "creatmodel_annoyance_commands"),
                ("experience", "creatmodel_experience_commands"),
                ("appearance", "creatmodel_appearance_commands"),
                ("senses", "creatmodel_senses_commands"),
            ] {
                let keys = section_keys(&root, "creatrs/*.cfg", block, &enums);
                for name in enum_members(&enums, list) {
                    if creature.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
                        continue;
                    }
                    let k = keys.iter().find(|k| k.name.eq_ignore_ascii_case(&name));
                    creature.push(match k {
                        Some(k) => Prop {
                            name: name.clone(),
                            ..k.clone()
                        },
                        None => Prop {
                            name,
                            doc: String::new(),
                            block: block.to_string(),
                            parse: None,
                            args: vec![],
                            source: None,
                        },
                    });
                }
            }
            targets.insert(ConfigTarget::Creature, creature);
            Schema { rules, targets }
        })
    }

    /// Every rule `SET_GAME_RULE` accepts.
    pub fn rules(&self) -> &[Prop] {
        &self.rules
    }

    /// The rule of that name (any case).
    pub fn rule(&self, name: &str) -> Option<&Prop> {
        self.rules
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(name))
    }

    /// The properties `SET_*_CONFIGURATION` accepts for `target`.
    pub fn props(&self, target: ConfigTarget) -> &[Prop] {
        self.targets
            .get(&target)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn prop(&self, target: ConfigTarget, name: &str) -> Option<&Prop> {
        self.props(target)
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

/// Problem with one value of a rule or property, as the C parser would log it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueProblem {
    pub code: &'static str,
    pub message: String,
}

/// Check `text` against how the game reads it (`value_default`, `value_flagsfield`). Only the
/// cases the C code reports are returned; `None` means the value is fine or not checkable.
pub fn check_value(prop: &Prop, arg: &PropArg, text: &str) -> Option<ValueProblem> {
    let is_num = crate::consts::is_number(text);
    let p = |code: &'static str, message: String| Some(ValueProblem { code, message });
    match &arg.kind {
        ValueKind::Int { min, max } => {
            if is_num {
                if prop.parse.as_deref() != Some("value_default") {
                    return None;
                }
                let v: i64 = text.parse().ok()?;
                if let Some(lo) = min.filter(|lo| v < *lo) {
                    return p(
                        "value-range",
                        format!(
                            "{v} is below the minimum {lo} of {}; the game clamps it",
                            prop.name
                        ),
                    );
                }
                if let Some(hi) = max.filter(|hi| v > *hi) {
                    return p(
                        "value-range",
                        format!(
                            "{v} is above the maximum {hi} of {}; the game clamps it",
                            prop.name
                        ),
                    );
                }
                None
            } else if prop.parse.as_deref() == Some("value_default") {
                p(
                    "value-type",
                    format!(
                        "{} expects a number, got '{text}'; the game uses 0",
                        prop.name
                    ),
                )
            } else {
                None
            }
        }
        ValueKind::Enum { members, .. } => {
            if is_num || members.is_empty() || members.iter().any(|m| m.eq_ignore_ascii_case(text))
            {
                None
            } else if prop.parse.as_deref() == Some("value_default") {
                p(
                    "value-name",
                    format!(
                        "'{text}' is not a valid value of {} (expected a number or one of {})",
                        prop.name,
                        list(members)
                    ),
                )
            } else {
                None
            }
        }
        ValueKind::Flags { members, .. } => {
            if is_num
                || members.is_empty()
                || text.eq_ignore_ascii_case("none")
                || prop.parse.as_deref() != Some("value_flagsfield")
            {
                return None;
            }
            for w in text.split_whitespace() {
                if !members.iter().any(|m| m.eq_ignore_ascii_case(w)) {
                    return p(
                        "value-name",
                        format!(
                            "'{w}' is not a flag of {} (expected one of {})",
                            prop.name,
                            list(members)
                        ),
                    );
                }
            }
            None
        }
        ValueKind::Other(_) => None,
    }
}

fn list(members: &[String]) -> String {
    let mut v: Vec<&str> = members.iter().take(8).map(|s| s.as_str()).collect();
    if members.len() > 8 {
        v.push("...");
    }
    v.join(", ")
}
