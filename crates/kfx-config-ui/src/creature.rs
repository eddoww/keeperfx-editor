//! Creature helpers: the experience level formulas, the stats comparison table, the growth
//! table and "new creature from copy".
use crate::session::ConfigSession;
use kfx_map::gamedata::parse_number;

/// `CREATURE_MAX_LEVEL` (src/thing_stats.h:29).
pub const MAX_LEVEL: i64 = 10;
/// `CREATURE_PROPERTY_INCREASE_ON_EXP` (src/config_creature.h:41).
pub const DEFAULT_INCREASE: i64 = 35;

/// `[experience]` percentages of creature.cfg with the game's defaults when absent
/// (`parse_creaturetype_experience_blocks`, src/config_creature.c:747-765), plus the
/// `OVERFLOW_8BIT` classic bug of rules.cfg `[game] PreserveClassicBugs` (src/config_rules.c:64,115),
/// which changes the 8-bit caps of the level formulas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpRates {
    pub health: i64,
    pub strength: i64,
    pub dexterity: i64,
    pub defense: i64,
    pub armour: i64,
    pub loyalty: i64,
    pub pay: i64,
    pub training_cost: i64,
    pub scavenging_cost: i64,
    /// `JobValueIncreaseOnExp` (research, manufacture, scavenge values).
    pub job_value: i64,
    /// `ClscBug_Overflow8bitVal` is set.
    pub overflow_8bit: bool,
}

impl Default for ExpRates {
    fn default() -> Self {
        ExpRates {
            health: DEFAULT_INCREASE,
            strength: DEFAULT_INCREASE,
            dexterity: DEFAULT_INCREASE,
            defense: DEFAULT_INCREASE,
            armour: 0,
            loyalty: DEFAULT_INCREASE,
            pay: DEFAULT_INCREASE,
            training_cost: 0,
            scavenging_cost: 0,
            job_value: DEFAULT_INCREASE,
            overflow_8bit: false,
        }
    }
}

impl ExpRates {
    pub fn load(s: &mut ConfigSession) -> ExpRates {
        let d = ExpRates::default();
        let mut g = |k: &str, d: i64| {
            s.cfg_value("creature.cfg", "experience", k)
                .and_then(|v| parse_number(v.split_whitespace().next()?))
                .unwrap_or(d)
        };
        let r = ExpRates {
            health: g("HealthIncreaseOnExp", d.health),
            strength: g("StrengthIncreaseOnExp", d.strength),
            dexterity: g("DexterityIncreaseOnExp", d.dexterity),
            defense: g("DefenseIncreaseOnExp", d.defense),
            armour: g("ArmourIncreaseOnExp", d.armour),
            loyalty: g("LoyaltyIncreaseOnExp", d.loyalty),
            pay: g("PayIncreaseOnExp", d.pay),
            training_cost: g("TrainingCostIncreaseOnExp", d.training_cost),
            scavenging_cost: g("ScavengingCostIncreaseOnExp", d.scavenging_cost),
            job_value: g("JobValueIncreaseOnExp", d.job_value),
            overflow_8bit: false,
        };
        let bugs = s
            .cfg_value("rules.cfg", "game", "PreserveClassicBugs")
            .unwrap_or_default();
        ExpRates {
            overflow_8bit: bugs
                .split_whitespace()
                .any(|t| t.eq_ignore_ascii_case("OVERFLOW_8BIT")),
            ..r
        }
    }
}

/// How a creature stat changes with the experience level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Growth {
    /// Same value at every level.
    None,
    /// `compute_creature_max_health` (src/thing_stats.c:358).
    Health,
    /// `compute_creature_max_strength` (src/thing_stats.c:375).
    Strength,
    /// `compute_creature_max_armour` (src/thing_stats.c:390).
    Armour,
    /// `compute_creature_max_defense` (src/thing_stats.c:403).
    Defence,
    /// `compute_creature_max_dexterity` (src/thing_stats.c:419).
    Dexterity,
    /// `compute_creature_max_loyalty` (src/thing_stats.c:432), applied to `ScavengeRequirement`
    /// (`calculate_correct_creature_loyalty`, src/thing_stats.c:774).
    Loyalty,
    /// `compute_creature_max_pay` (src/thing_stats.c:445).
    Pay,
    /// `compute_creature_max_training_cost` (src/thing_stats.c:458).
    TrainingCost,
    /// `compute_creature_max_scavenging_cost` (src/thing_stats.c:471).
    ScavengingCost,
    /// `compute_creature_work_value` (src/thing_stats.c:593) at full room efficiency, before the
    /// rules.cfg room efficiency factor (src/thing_stats.c:614-627).
    JobValue,
    /// `compute_creature_max_luck` = `compute_creature_max_unaffected` (src/thing_stats.h:100,
    /// src/thing_stats.c:850): clamped, level independent.
    Luck,
}

/// `saturate_set_unsigned` (src/bflib_basics.c:81): masks with the 8-bit classic bug, else clamps.
fn sat_unsigned(v: i64, bits: u32, overflow_8bit: bool) -> i64 {
    let max = (1i64 << bits) - 1;
    if bits == 8 && overflow_8bit {
        return v & max;
    }
    v.min(max)
}

/// `saturate_set_signed` (src/bflib_basics.c:66).
fn sat_signed(v: i64, bits: u32) -> i64 {
    let max = (1i64 << (bits - 1)) - 1;
    v.clamp(-max, max)
}

impl Growth {
    /// The `[experience]` percentage this stat grows by, `None` when level independent.
    pub fn rate(self, r: &ExpRates) -> Option<i64> {
        Some(match self {
            Growth::Health => r.health,
            Growth::Strength => r.strength,
            Growth::Armour => r.armour,
            Growth::Defence => r.defense,
            Growth::Dexterity => r.dexterity,
            Growth::Loyalty => r.loyalty,
            Growth::Pay => r.pay,
            Growth::TrainingCost => r.training_cost,
            Growth::ScavengingCost => r.scavenging_cost,
            Growth::JobValue => r.job_value,
            Growth::None | Growth::Luck => return None,
        })
    }

    /// creature.cfg `[experience]` key of [`Growth::rate`].
    pub fn rate_key(self) -> Option<&'static str> {
        Some(match self {
            Growth::Health => "HealthIncreaseOnExp",
            Growth::Strength => "StrengthIncreaseOnExp",
            Growth::Armour => "ArmourIncreaseOnExp",
            Growth::Defence => "DefenseIncreaseOnExp",
            Growth::Dexterity => "DexterityIncreaseOnExp",
            Growth::Loyalty => "LoyaltyIncreaseOnExp",
            Growth::Pay => "PayIncreaseOnExp",
            Growth::TrainingCost => "TrainingCostIncreaseOnExp",
            Growth::ScavengingCost => "ScavengingCostIncreaseOnExp",
            Growth::JobValue => "JobValueIncreaseOnExp",
            Growth::None | Growth::Luck => return None,
        })
    }

    /// The value at experience level `exp_level` (0-based: the game's level 1 is 0, shown as
    /// "L1"), as the `compute_creature_max_*` functions compute it from the cfg value `base`.
    /// Levels past `CREATURE_MAX_LEVEL - 1` use the last level, like the game. Dungeon
    /// modifiers and spells are not applied.
    pub fn at_level(self, base: i64, exp_level: i64, r: &ExpRates) -> i64 {
        let lvl = exp_level.clamp(0, MAX_LEVEL - 1);
        let grow =
            |b: i64, pct: i64| b.saturating_add(pct.saturating_mul(b).saturating_mul(lvl) / 100);
        let capped = |b: i64, cap: i64| if b <= 0 { None } else { Some(b.min(cap)) };
        match self {
            Growth::None => base,
            Growth::Health => grow(base, r.health).min(i32::MAX as i64),
            Growth::Strength => {
                let v = grow(base, r.strength);
                // DK1 limited shot damage to 256 (src/thing_stats.c:382-385)
                if r.overflow_8bit {
                    v.min(256)
                } else {
                    v
                }
            }
            Growth::Armour => capped(base, 60000).map_or(0, |b| grow(b, r.armour)),
            Growth::Defence => capped(base, 10000).map_or(0, |b| {
                let v = grow(b, r.defense);
                if v >= 255 && !r.overflow_8bit {
                    255
                } else {
                    v
                }
            }),
            Growth::Dexterity => capped(base, 10000).map_or(0, |b| {
                sat_unsigned(grow(b, r.dexterity), 8, r.overflow_8bit)
            }),
            Growth::Loyalty => capped(base, 60000)
                .map_or(0, |b| sat_unsigned(grow(b, r.loyalty), 24, r.overflow_8bit)),
            Growth::Pay => capped(base, 100000).map_or(0, |b| sat_signed(grow(b, r.pay), 16)),
            Growth::TrainingCost => {
                capped(base, 100000).map_or(0, |b| sat_signed(grow(b, r.training_cost), 16))
            }
            Growth::ScavengingCost => {
                capped(base, 100000).map_or(0, |b| sat_signed(grow(b, r.scavenging_cost), 16))
            }
            Growth::JobValue => grow(base.clamp(-100000, 100000), r.job_value),
            Growth::Luck => capped(base, 10000).map_or(0, |b| sat_unsigned(b, 8, r.overflow_8bit)),
        }
    }
}

/// A numeric stat of a creature model file (`creatrs/<name>.cfg`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatDef {
    /// Column title.
    pub label: &'static str,
    pub section: &'static str,
    pub key: &'static str,
    /// Which value of the key (`Size = <xy> <z>`).
    pub token: usize,
    pub growth: Growth,
    /// Where the game reads / uses it.
    pub doc: &'static str,
}

const fn st(
    label: &'static str,
    section: &'static str,
    key: &'static str,
    growth: Growth,
    doc: &'static str,
) -> StatDef {
    StatDef {
        label,
        section,
        key,
        token: 0,
        growth,
        doc,
    }
}

/// The stats of the comparison table. Keys: `creatmodel_attributes_commands`
/// (src/config_crtrmodel.c:58-100), `creatmodel_attraction_commands` (:144-150),
/// `creatmodel_jobs_commands` (:226-238).
pub const STATS: &[StatDef] = &[
    st(
        "Health",
        "attributes",
        "Health",
        Growth::Health,
        // src/config_crtrmodel.c:60
        "hit points",
    ),
    st(
        "Strength",
        "attributes",
        "Strength",
        Growth::Strength,
        // src/config_crtrmodel.c:63
        "melee damage base",
    ),
    st(
        "Armour",
        "attributes",
        "Armour",
        Growth::Armour,
        // src/config_crtrmodel.c:64
        "damage reduction",
    ),
    st(
        "Defence",
        "attributes",
        "Defence",
        Growth::Defence,
        // src/config_crtrmodel.c:68
        "chance to avoid hits",
    ),
    st(
        "Dexterity",
        "attributes",
        "Dexterity",
        Growth::Dexterity,
        // src/config_crtrmodel.c:65
        "chance to hit",
    ),
    st(
        "Luck",
        "attributes",
        "Luck",
        Growth::Luck,
        // src/config_crtrmodel.c:69
        "chance of a critical hit",
    ),
    st(
        "Speed",
        "attributes",
        "BaseSpeed",
        Growth::None,
        // calculate_correct_creature_maxspeed, src/thing_stats.c:744
        "move speed; level independent",
    ),
    st(
        "Loyalty",
        "attraction",
        "ScavengeRequirement",
        Growth::Loyalty,
        // src/config_crtrmodel.c:148; src/thing_stats.c:774
        "loyalty against scavenging",
    ),
    st(
        "Pay",
        "attributes",
        "Pay",
        Growth::Pay,
        // src/config_crtrmodel.c:79
        "salary per pay day",
    ),
    st(
        "Train cost",
        "jobs",
        "TrainingCost",
        Growth::TrainingCost,
        // src/config_crtrmodel.c:232
        "gold per training step",
    ),
    st(
        "Scav cost",
        "jobs",
        "ScavengerCost",
        Growth::ScavengingCost,
        // src/config_crtrmodel.c:234
        "gold per scavenging step",
    ),
    st(
        "Research",
        "jobs",
        "ResearchValue",
        Growth::JobValue,
        // src/config_crtrmodel.c:235; src/thing_stats.c:614
        "research skill",
    ),
    st(
        "Manufacture",
        "jobs",
        "ManufactureValue",
        Growth::JobValue,
        // src/config_crtrmodel.c:236; src/thing_stats.c:618
        "manufacture skill",
    ),
    st(
        "Training",
        "jobs",
        "TrainingValue",
        Growth::None,
        // src/config_crtrmodel.c:231; src/thing_stats.c:621-623
        "training skill; does not grow with level",
    ),
    st(
        "Scavenge",
        "jobs",
        "ScavengeValue",
        Growth::JobValue,
        // src/config_crtrmodel.c:233; src/thing_stats.c:627
        "scavenging skill",
    ),
    st(
        "Fear wounded",
        "attributes",
        "FearWounded",
        Growth::None,
        // src/config_crtrmodel.c:66
        "flees below this % of health",
    ),
    st(
        "Fear stronger",
        "attributes",
        "FearStronger",
        Growth::None,
        // src/config_crtrmodel.c:67
        "flees from enemies this much stronger",
    ),
    st(
        "Fearsome",
        "attributes",
        "FearsomeFactor",
        Growth::None,
        // src/config_crtrmodel.c:88
        "how much it frightens others",
    ),
    st(
        "Hunger rate",
        "attributes",
        "HungerRate",
        Growth::None,
        // src/config_crtrmodel.c:71
        "game turns between hunger",
    ),
    st(
        "Hunger fill",
        "attributes",
        "HungerFill",
        Growth::None,
        // src/config_crtrmodel.c:72
        "chickens per meal",
    ),
    st(
        "Gold hold",
        "attributes",
        "GoldHold",
        Growth::None,
        // src/config_crtrmodel.c:76
        "gold it can carry",
    ),
    st(
        "Slaps",
        "attributes",
        "SlapsToKill",
        Growth::None,
        // src/config_crtrmodel.c:81
        "slaps to kill",
    ),
    st(
        "Size",
        "attributes",
        "Size",
        Growth::None,
        // src/config_crtrmodel.c:77
        "clipbox size XY (first value)",
    ),
];

/// One cell of the comparison table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatCell {
    /// The cfg value (first number of the key), `None` when absent or not a number.
    pub base: Option<i64>,
    /// The value at the requested level.
    pub value: Option<i64>,
    /// Layer the cfg value comes from (all layers, as the game ends up with it).
    pub layer: Option<usize>,
    /// Layer the `[experience]` rate comes from, when the stat grows and the rate is set.
    pub rate_layer: Option<usize>,
}

/// One creature of the comparison table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatRow {
    pub name: String,
    /// `creatrs/<lower-case name>.cfg`.
    pub key: String,
    /// Listed in creature.cfg `[common] Creatures` (unlisted models are not loaded by the game).
    pub listed: bool,
    /// One per [`STATS`] entry.
    pub cells: Vec<StatCell>,
}

/// Creature model files present in the base / mod / campaign creature folders but not in
/// creature.cfg `[common] Creatures` (upper case names, sorted).
pub fn unlisted_creatures(s: &mut ConfigSession) -> Vec<String> {
    let listed = s.creature_names();
    let mut out: Vec<String> = vec![];
    for l in &s.layers.layers {
        if matches!(l.kind, crate::layers::LayerKind::Map(_)) {
            continue;
        }
        let Some(d) = &l.creature_dir else { continue };
        for f in s.fs.list_dir(d).unwrap_or_default() {
            let Some(stem) = f.strip_suffix(".cfg").or_else(|| f.strip_suffix(".CFG")) else {
                continue;
            };
            if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                continue;
            }
            let n = stem.to_ascii_uppercase();
            if !listed.iter().any(|x| x.eq_ignore_ascii_case(&n)) && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out.sort();
    out
}

/// The comparison table: every creature of `names` (`(name, listed)`) with every [`STATS`]
/// value at 1-based level `level` (1..=10), values as the game ends up with them (all layers,
/// unsaved edits included).
pub fn stat_rows(s: &mut ConfigSession, names: &[(String, bool)], level: u8) -> Vec<StatRow> {
    let rates = ExpRates::load(s);
    let rate_layers: Vec<Option<usize>> = STATS
        .iter()
        .map(|d| {
            let k = d.growth.rate_key()?;
            s.cfg("creature.cfg")
                .and_then(|c| c.effective("experience", k))
                .map(|p| p.layer)
        })
        .collect();
    let exp_level = level.clamp(1, MAX_LEVEL as u8) as i64 - 1;
    let mut out = vec![];
    for (name, listed) in names {
        let key = format!("creatrs/{}.cfg", name.to_ascii_lowercase());
        let Some(c) = s.cfg(&key) else { continue };
        let cells = STATS
            .iter()
            .zip(&rate_layers)
            .map(|(d, rl)| {
                let p = c.effective(d.section, d.key);
                let base = p.as_ref().and_then(|p| {
                    p.value
                        .split_whitespace()
                        .nth(d.token)
                        .and_then(parse_number)
                });
                StatCell {
                    base,
                    value: base.map(|b| d.growth.at_level(b, exp_level, &rates)),
                    layer: p.map(|p| p.layer),
                    rate_layer: *rl,
                }
            })
            .collect();
        out.push(StatRow {
            name: name.clone(),
            key,
            listed: *listed,
            cells,
        });
    }
    out
}

/// One row of the growth table: `(stat, values for levels 1..=10)` of the stats that grow with
/// experience ([`Growth::at_level`]). Stats whose base value is missing are left out.
pub fn growth_table(
    base: &dyn Fn(&str) -> Option<i64>,
    r: &ExpRates,
) -> Vec<(&'static str, [i64; 10])> {
    STATS
        .iter()
        .filter(|d| d.growth.rate(r).is_some())
        .filter_map(|d| {
            let b = base(d.key)?;
            let mut out = [0i64; 10];
            for (lvl, o) in out.iter_mut().enumerate() {
                *o = d.growth.at_level(b, lvl as i64, r);
            }
            Some((d.label, out))
        })
        .collect()
}

/// Is `name` usable as a new creature name (letters, digits, underscore; not taken)?
pub fn check_new_name(s: &mut ConfigSession, name: &str) -> Result<(), String> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err("use letters, digits and _ only".into());
    }
    if s.creature_names()
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
    {
        return Err(format!("{name} already exists"));
    }
    Ok(())
}

/// Create creature `new_name` as a copy of `from` at the target layer: `creatrs/<new>.cfg`
/// with the effective content of `from`'s file (every section and key as of the target layer),
/// and `new_name` appended to `creature.cfg [common] Creatures`. Nothing is written until the
/// session is saved.
pub fn new_creature_from_copy(
    s: &mut ConfigSession,
    from: &str,
    new_name: &str,
) -> Result<String, String> {
    check_new_name(s, new_name)?;
    let src_key = format!("creatrs/{}.cfg", from.to_ascii_lowercase());
    let dst_key = format!("creatrs/{}.cfg", new_name.to_ascii_lowercase());
    let tl = s.layers.layers[s.target()].clone();
    if tl.location(&dst_key).is_none() {
        return Err("the target layer has no creature folder".into());
    }
    // effective content of the source
    let mut lines: Vec<(String, Vec<(String, String)>)> = vec![];
    {
        let Some(src) = s.cfg(&src_key) else {
            return Err("source is not a cfg".into());
        };
        for sec in src.section_names() {
            let mut kv = vec![];
            for k in src.keys(&sec) {
                if let Some(p) = src.at_target(&sec, &k) {
                    kv.push((k, p.value));
                }
            }
            lines.push((sec, kv));
        }
        if lines.is_empty() {
            return Err(format!("{from} has no creature file"));
        }
    }
    let Some(dst) = s.cfg(&dst_key) else {
        return Err("destination is not a cfg".into());
    };
    for (sec, kv) in &lines {
        dst.target.add_section(sec);
        for (k, v) in kv {
            dst.target.add(sec, k, v);
        }
    }
    s.touched(&dst_key);
    let mut names = s.creature_names();
    names.push(new_name.to_ascii_uppercase());
    s.set_cfg("creature.cfg", "common", "Creatures", &names.join(" "));
    Ok(dst_key)
}
