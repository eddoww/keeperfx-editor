//! The game rules the analyses need, read from the layered configuration of a [`GameData`].
//!
//! Every value falls back to the shipped default (KeeperFX `config/fxdata`) when the config
//! does not have it. Times are game turns; the game runs 20 turns per second by default
//! (`FRAMES_PER_SECOND` in `keeperfx.cfg`, `start_params.num_fps`, src/config_keeperfx.c:878).
use kfx_map::gamedata::{parse_number, GameData};

/// `terrain_health_commands` (src/config_terrain.c:331): the `[block_health]` names, index =
/// `BlockHealthIndex`.
pub const BLOCK_HEALTH_NAMES: [&str; 10] = [
    "DIRT",
    "GOLD",
    "PRETTY",
    "FLOOR",
    "ROOM",
    "DOOR_WOODEN",
    "DOOR_BRACE",
    "DOOR_STEEL",
    "DOOR_MAGIC",
    "DENSE_GOLD",
];
/// Shipped `[block_health]` values (config/fxdata/terrain.cfg:1463).
const BLOCK_HEALTH_DEFAULT: [u32; 10] = [2, 20, 20, 5, 20, 20, 80, 255, 255, 50];

/// Experience growth percentages of `creature.cfg [experience]` used by
/// `compute_creature_kind_score` (src/thing_stats.c:347).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpGrowth {
    pub health: i64,
    pub strength: i64,
    pub dexterity: i64,
    pub defense: i64,
    pub armour: i64,
}

/// Rule values used by the analyses (see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// Turns an imp needs to walk one slab: 768 coordinate units (`COORD_PER_STL` 256 x 3) /
    /// `BaseSpeed` of `imp.cfg` (96 shipped: 8 turns).
    pub walk_turns: u32,
    /// Turns between two dig hits: `Time` of the `DIG` instance in `creature.cfg` (8 shipped,
    /// `instance_dig` src/creature_instances.c:650).
    pub dig_hit_turns: u32,
    /// `DefaultImpDigDamage` / `DefaultImpDigOwnDamage` (`rules.cfg [workers]`,
    /// `calculate_damage_did_to_slab_with_single_hit` src/thing_stats.c:1163).
    pub dig_damage: u32,
    pub dig_own_damage: u32,
    /// `[block_health]` of `terrain.cfg` (`game.block_health`, src/config_terrain.c:458).
    pub block_health: [u32; 10],
    /// Game turns per second (20, the game's default).
    pub turns_per_second: u32,
    /// `GoldPerHoard` (rules.cfg [game]): gold one treasury slab holds.
    pub gold_per_hoard: u32,
    /// Typical value of the loose gold objects (`gold_object_typical_value`,
    /// src/thing_objects.c:2106): `(model, gold)`.
    pub gold_objects: Vec<(i64, u32)>,
    pub exp: ExpGrowth,
    /// Turns to break a door, per `Health` point of `trapdoor.cfg` (an assumption: about 2
    /// damage per turn, one Strength 16 attacker hitting every 8 turns).
    pub door_turns_per_health_div: u32,
}

fn num(s: Option<&str>) -> Option<i64> {
    s.and_then(|s| s.split_whitespace().next())
        .and_then(parse_number)
}

impl Rules {
    /// The shipped defaults (no configuration needed).
    pub fn defaults() -> Rules {
        Rules {
            walk_turns: 8,
            dig_hit_turns: 8,
            dig_damage: 1,
            dig_own_damage: 2,
            block_health: BLOCK_HEALTH_DEFAULT,
            turns_per_second: 20,
            gold_per_hoard: 2000,
            gold_objects: vec![(3, 500), (6, 250), (43, 200), (136, 100)],
            exp: ExpGrowth {
                health: 35,
                strength: 35,
                dexterity: 4,
                defense: 5,
                armour: 0,
            },
            door_turns_per_health_div: 2,
        }
    }

    /// Rules from the configuration layers of `gd`.
    pub fn from_game(gd: &GameData) -> Rules {
        let mut r = Rules::defaults();
        if let Some(imp) = gd.creature_config("imp") {
            if imp.attributes.base_speed > 0 {
                r.walk_turns = (768 / imp.attributes.base_speed as u32).max(1);
            }
        }
        // the DIG instance: find the [instanceN] named DIG
        for (name, f) in &gd.creature_cfg.sections {
            if name.to_ascii_lowercase().starts_with("instance")
                && f.get("Name")
                    .is_some_and(|n| n.trim().eq_ignore_ascii_case("DIG"))
            {
                if let Some(t) = num(f.get("Time")).filter(|&t| t > 0) {
                    r.dig_hit_turns = t as u32;
                }
            }
        }
        if let Some(f) = gd.creature_cfg.section("experience") {
            let g = |k: &str, d: i64| num(f.get(k)).unwrap_or(d);
            r.exp = ExpGrowth {
                health: g("HealthIncreaseOnExp", 35),
                strength: g("StrengthIncreaseOnExp", 35),
                dexterity: g("DexterityIncreaseOnExp", 4),
                defense: g("DefenseIncreaseOnExp", 5),
                armour: g("ArmourIncreaseOnExp", 0),
            };
        }
        if let Ok(rules) = gd.layered_cfg("rules.cfg") {
            let v = |s: &str, k: &str| num(rules.value(s, k));
            if let Some(d) = v("workers", "DefaultImpDigDamage").filter(|&d| d > 0) {
                r.dig_damage = d as u32;
            }
            if let Some(d) = v("workers", "DefaultImpDigOwnDamage").filter(|&d| d > 0) {
                r.dig_own_damage = d as u32;
            }
            if let Some(g) = v("game", "GoldPerHoard").filter(|&g| g > 0) {
                r.gold_per_hoard = g as u32;
            }
            let keys = [
                (3, "ChestGoldHold"),
                (6, "PotOfGoldHolds"),
                (43, "GoldPileValue"),
                (136, "BagGoldHold"),
            ];
            for (m, k) in keys {
                if let Some(g) = v("game", k).filter(|&g| g >= 0) {
                    if let Some(e) = r.gold_objects.iter_mut().find(|e| e.0 == m) {
                        e.1 = g as u32;
                    }
                }
            }
        }
        if let Ok(t) = gd.layered_cfg("terrain.cfg") {
            for (i, n) in BLOCK_HEALTH_NAMES.iter().enumerate() {
                if let Some(h) = num(t.value("block_health", n)).filter(|&h| h > 0) {
                    r.block_health[i] = h as u32;
                }
            }
        }
        r
    }

    /// `game.block_health[index]` (0 for an index outside the table).
    pub fn health(&self, index: i16) -> u32 {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.block_health.get(i))
            .copied()
            .unwrap_or(0)
    }

    /// Turns for one imp to dig out a slab of block health `health` (hits = ceil(health /
    /// damage), `instance_dig` src/creature_instances.c:670-700).
    pub fn dig_turns(&self, health: u32, own: bool) -> u32 {
        let dmg = if own {
            self.dig_own_damage
        } else {
            self.dig_damage
        }
        .max(1);
        health.max(1).div_ceil(dmg) * self.dig_hit_turns
    }

    /// Gold one hit takes out of a slab (`calculate_gold_digged_out_of_slab_with_single_hit`,
    /// src/thing_stats.c:1174), at least 1.
    pub fn gold_per_hit(&self, gold_held: i32, health: u32) -> u32 {
        if gold_held <= 0 || health == 0 {
            return 0;
        }
        ((self.dig_damage as i64 * gold_held as i64) / health as i64).max(1) as u32
    }

    /// `compute_creature_kind_score` (src/thing_stats.c:347) for raw stats at experience level
    /// `exp_level` (0-based, capped at 9).
    pub fn creature_score(
        &self,
        health: i64,
        strength: i64,
        dexterity: i64,
        defense: i64,
        armour: i64,
        exp_level: i64,
    ) -> i64 {
        let l = exp_level.clamp(0, 9);
        let grow = |base: i64, pct: i64| base + pct * base * l / 100;
        let defense = if defense <= 0 {
            0
        } else {
            grow(defense.min(10000), self.exp.defense).min(255)
        };
        let armour = if armour <= 0 {
            0
        } else {
            grow(armour.min(60000), self.exp.armour)
        };
        grow(health, self.exp.health)
            + defense
            + grow(dexterity, self.exp.dexterity)
            + armour
            + grow(strength, self.exp.strength)
    }

    /// Typical gold of a loose gold object model (0 when not gold).
    pub fn gold_object_value(&self, model: i64) -> u32 {
        self.gold_objects
            .iter()
            .find(|e| e.0 == model)
            .map(|e| e.1)
            .unwrap_or(0)
    }

    /// Turns to convert to seconds.
    pub fn seconds(&self, turns: u32) -> f32 {
        turns as f32 / self.turns_per_second.max(1) as f32
    }
}
