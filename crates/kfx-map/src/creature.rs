//! Typed per-creature configuration (`creatrs/<name>.cfg`).
//!
//! The game builds a creature model from the layers base `creatrs/`, campaign
//! `CREATURES_LOCATION`, per-map `map%05lu.<name>.cfg` (and the mod layers, see [`crate::mods`]):
//! `load_creaturemodel_config` (src/config_crtrmodel.c:2894-2946), each file through
//! `load_creaturemodel_config_file` (src/config_crtrmodel.c:2780-2825) which parses the blocks
//! `[attributes]` (`parse_creaturemodel_attributes_blocks` :264), `[attraction]`, `[annoyance]`,
//! `[senses]`, `[appearance]`, `[experience]`, `[jobs]`, `[sprites]` (:2216) and `[sounds]`. Later
//! layers override single keys; a missing key keeps the earlier value, and the first file is
//! applied on top of zeroed stats (`init_creature_model_stats`).
//!
//! [`CreatureConfig`] keeps the whole layered file ([`CreatureConfig::cfg`], with the layer every
//! value came from) and decodes what an editor needs into typed fields: all of `[attributes]`, the
//! appearance fields, and every `[sprites]` entry. Any other field is available through
//! [`CreatureConfig::raw`].
use crate::gamedata::parse_number;
use crate::gamedata::ConfigLayer;
use crate::layered::LayeredCfg;
use crate::mods::{LayerOrigin, LayerStack};
use crate::{Error, Result};

/// `creatmodel_properties_commands` (src/config_crtrmodel.c:102-143), including the deprecated names.
pub const PROPERTY_NAMES: [&str; 38] = [
    "BLEEDS",
    "UNAFFECTED_BY_WIND",
    "IMMUNE_TO_GAS",
    "HUMANOID_SKELETON",
    "PISS_ON_DEAD",
    "FLYING",
    "SEE_INVISIBLE",
    "PASS_LOCKED_DOORS",
    "SPECIAL_DIGGER",
    "ARACHNID",
    "DIPTERA",
    "LORD",
    "SPECTATOR",
    "EVIL",
    "NEVER_CHICKENS",
    "IMMUNE_TO_BOULDER",
    "NO_CORPSE_ROTTING",
    "NO_ENMHEART_ATTCK",
    "TREMBLING_FAT",
    "FEMALE",
    "INSECT",
    "ONE_OF_KIND",
    "NO_IMPRISONMENT",
    "IMMUNE_TO_DISEASE",
    "ILLUMINATED",
    "ALLURING_SCVNGR",
    "NO_RESURRECT",
    "NO_TRANSFER",
    "TREMBLING",
    "FAT",
    "NO_STEAL_HERO",
    "PREFER_STEAL",
    "EVENTFUL_DEATH",
    "DIGGING_CREATURE",
    "NO_HEALTH_FLOWER",
    "CANNOT_PICK_UP",
    "DROP_ON_PATH",
    "CANNOT_POSSESS",
];

/// `creature_graphics_desc` (src/config_creature.c:183-210): the `[sprites]` keys.
pub const SPRITE_KEYS: [&str; 25] = [
    "Stand",
    "Ambulate",
    "Drag",
    "Attack",
    "Dig",
    "Smoke",
    "Relax",
    "PrettyDance",
    "GotHit",
    "PowerGrab",
    "GotSlapped",
    "Celebrate",
    "Sleep",
    "EatChicken",
    "Torture",
    "Scream",
    "DropDead",
    "DeadSplat",
    "Roar",
    "QuerySymbol",
    "HandSymbol",
    "Piss",
    "CastSpell",
    "RangedAttack",
    "Custom",
];

/// A `[sprites]` value: a number, or a name (animation / icon name, e.g. of a custom sprite).
/// Animations go through `get_anim_id_`, the two symbols through `get_icon_id`
/// (src/config_crtrmodel.c:2231-2260).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpriteRef {
    Index(i32),
    Name(String),
}

impl SpriteRef {
    pub fn parse(s: &str) -> SpriteRef {
        match parse_number(s) {
            Some(n) => SpriteRef::Index(n as i32),
            None => SpriteRef::Name(s.trim().to_string()),
        }
    }
}

/// `[attributes]` (src/config_crtrmodel.c:264-1000). Numbers default to 0 like the game's zeroed
/// stats; read [`CreatureConfig::raw`] to tell "absent" from 0.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreatureAttributes {
    /// Deprecated `Name` (the display name is `NameTextID` in the string table).
    pub name: String,
    pub name_text_id: i32,
    pub health: i32,
    pub heal_requirement: i32,
    pub heal_threshold: i32,
    pub strength: i32,
    pub armour: i32,
    pub dexterity: i32,
    pub fear_wounded: i32,
    pub fear_stronger: i32,
    pub fearsome_factor: i32,
    pub defence: i32,
    pub luck: i32,
    pub recovery: i32,
    pub toking_recovery: i32,
    pub hunger_rate: i32,
    pub hunger_fill: i32,
    pub lair_size: i32,
    pub hurt_by_lava: i32,
    pub base_speed: i32,
    pub gold_hold: i32,
    /// `Size`: clipbox XY and Z (`size_xy`, `size_z`).
    pub size_xy: i32,
    pub size_z: i32,
    /// `ThingSize`: thing XY and Z, one cube is 256 (`thing_size_xy`, `thing_size_z`).
    pub thing_size_xy: i32,
    pub thing_size_z: i32,
    /// `MELEE` / `RANGED`, as written.
    pub attack_preference: String,
    pub pay: i32,
    pub slaps_to_kill: i32,
    pub damage_to_boulder: i32,
    /// `Properties`, upper-cased, in file order.
    pub properties: Vec<String>,
    pub lair_object: String,
    pub prison_kind: String,
    pub torture_kind: String,
    pub hostile_towards: String,
    /// `SpellImmunity` as written (spell flag names or a number).
    pub spell_immunity: String,
    pub inertia: (i32, i32),
    pub fall_acceleration: i32,
    pub corpse_vanish_effect: String,
    pub footstep_pitch: i32,
}

/// `[appearance]` (config_crtrmodel.c `parse_creaturemodel_appearance_blocks`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreatureAppearance {
    pub walking_anim_speed: i32,
    pub fixed_anim_speed: i32,
    pub visual_range: i32,
    pub possess_swipe_index: i32,
    pub natural_death_kind: String,
    pub shot_origin: [i32; 3],
    pub pickup_offset: (i32, i32),
    pub status_offset: i32,
    /// `TransparencyFlags` as written; the game stores it shifted left by 4 (render flags,
    /// src/config_crtrmodel.c:1787), see [`CreatureAppearance::render_transparency_bits`].
    pub transparency_flags: i32,
}

impl CreatureAppearance {
    /// `transparency_flags` as the game keeps it (`k << 4`).
    pub fn render_transparency_bits(&self) -> i32 {
        if self.transparency_flags >= 0 {
            self.transparency_flags << 4
        } else {
            0
        }
    }
}

/// The typed view of one creature's layered configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatureConfig {
    /// Lower-case file name without `.cfg` (`imp`).
    pub name: String,
    /// The layered file with the provenance of every value.
    pub cfg: LayeredCfg,
    pub attributes: CreatureAttributes,
    pub appearance: CreatureAppearance,
    /// Every `[sprites]` key present, in the order of [`SPRITE_KEYS`].
    pub sprites: Vec<(&'static str, SpriteRef)>,
}

fn int(s: Option<&str>) -> i32 {
    s.and_then(|s| s.split_whitespace().next())
        .and_then(parse_number)
        .unwrap_or(0) as i32
}

fn pair(s: Option<&str>) -> (i32, i32) {
    let mut it = s
        .unwrap_or("")
        .split_whitespace()
        .map(|t| parse_number(t).unwrap_or(0) as i32);
    (it.next().unwrap_or(0), it.next().unwrap_or(0))
}

fn word(s: Option<&str>) -> String {
    s.and_then(|s| s.split_whitespace().next())
        .unwrap_or("")
        .to_string()
}

impl CreatureConfig {
    /// Load `<name>.cfg` through the creature layers (`layers`, base first). Errors when no layer
    /// has the file.
    pub fn load(name: &str, layers: &[(LayerOrigin, ConfigLayer)]) -> Result<CreatureConfig> {
        let lower = name.to_ascii_lowercase();
        let f = format!("{lower}.cfg");
        let files: Vec<_> = layers
            .iter()
            .filter_map(|(o, l)| l.file(&f).map(|p| (o.clone(), p)))
            .collect();
        if files.is_empty() {
            return Err(Error::Invalid(format!(
                "no creature config {f} in any layer"
            )));
        }
        Ok(Self::from_cfg(&lower, LayeredCfg::from_cfg_files(files)?))
    }

    /// [`CreatureConfig::load`] with a [`LayerStack`] (mods included).
    pub fn load_stack(name: &str, stack: &LayerStack) -> Result<CreatureConfig> {
        Self::load(name, &stack.creatures)
    }

    /// Decode an already layered file.
    pub fn from_cfg(name: &str, cfg: LayeredCfg) -> CreatureConfig {
        let a = |k: &str| cfg.value("attributes", k);
        let p = |k: &str| cfg.value("appearance", k);
        let attributes = CreatureAttributes {
            name: word(a("Name")),
            name_text_id: int(a("NameTextID")),
            health: int(a("Health")),
            heal_requirement: int(a("HealRequirement")),
            heal_threshold: int(a("HealThreshold")),
            strength: int(a("Strength")),
            armour: int(a("Armour")),
            dexterity: int(a("Dexterity")),
            fear_wounded: int(a("FearWounded")),
            fear_stronger: int(a("FearStronger")),
            fearsome_factor: int(a("FearsomeFactor")),
            defence: int(a("Defence")),
            luck: int(a("Luck")),
            recovery: int(a("Recovery")),
            toking_recovery: int(a("TokingRecovery")),
            hunger_rate: int(a("HungerRate")),
            hunger_fill: int(a("HungerFill")),
            lair_size: int(a("LairSize")),
            hurt_by_lava: int(a("HurtByLava")),
            base_speed: int(a("BaseSpeed")),
            gold_hold: int(a("GoldHold")),
            size_xy: pair(a("Size")).0,
            size_z: pair(a("Size")).1,
            thing_size_xy: pair(a("ThingSize")).0,
            thing_size_z: pair(a("ThingSize")).1,
            attack_preference: word(a("AttackPreference")),
            pay: int(a("Pay")),
            slaps_to_kill: int(a("SlapsToKill")),
            damage_to_boulder: int(a("DamageToBoulder")),
            properties: a("Properties")
                .unwrap_or("")
                .split_whitespace()
                .map(|s| s.to_ascii_uppercase())
                .collect(),
            lair_object: word(a("LairObject")),
            prison_kind: word(a("PrisonKind")),
            torture_kind: word(a("TortureKind")),
            hostile_towards: a("HostileTowards").unwrap_or("").trim().to_string(),
            spell_immunity: a("SpellImmunity").unwrap_or("").trim().to_string(),
            inertia: pair(a("Inertia")),
            fall_acceleration: int(a("FallAcceleration")),
            corpse_vanish_effect: word(a("CorpseVanishEffect")),
            footstep_pitch: int(a("FootstepPitch")),
        };
        let so = p("ShotOrigin")
            .unwrap_or("")
            .split_whitespace()
            .map(|t| parse_number(t).unwrap_or(0) as i32)
            .collect::<Vec<_>>();
        let appearance = CreatureAppearance {
            walking_anim_speed: int(p("WalkingAnimSpeed")),
            fixed_anim_speed: int(p("FixedAnimSpeed")),
            visual_range: int(p("VisualRange")),
            possess_swipe_index: int(p("PossessSwipeIndex")),
            natural_death_kind: word(p("NaturalDeathKind")),
            shot_origin: [
                so.first().copied().unwrap_or(0),
                so.get(1).copied().unwrap_or(0),
                so.get(2).copied().unwrap_or(0),
            ],
            pickup_offset: pair(p("PickUpOffset")),
            status_offset: int(p("StatusOffset")),
            transparency_flags: int(p("TransparencyFlags")),
        };
        let sprites = SPRITE_KEYS
            .iter()
            .filter_map(|k| {
                cfg.value("sprites", k)
                    .and_then(|v| v.split_whitespace().next())
                    .map(|v| (*k, SpriteRef::parse(v)))
            })
            .collect();
        CreatureConfig {
            name: name.to_string(),
            cfg,
            attributes,
            appearance,
            sprites,
        }
    }

    /// The raw effective value of any field.
    pub fn raw(&self, section: &str, key: &str) -> Option<&str> {
        self.cfg.value(section, key)
    }

    /// The layer a field's effective value came from.
    pub fn origin(&self, section: &str, key: &str) -> Option<&LayerOrigin> {
        self.cfg.origin(section, key)
    }

    /// A `[sprites]` entry by key (case-insensitive), e.g. `"Stand"`.
    pub fn sprite(&self, key: &str) -> Option<&SpriteRef> {
        self.sprites
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }
    /// `QuerySymbol`: the icon of the creature in the query panel.
    pub fn query_symbol(&self) -> Option<&SpriteRef> {
        self.sprite("QuerySymbol")
    }
    /// `HandSymbol`: the icon of the creature held in the hand.
    pub fn hand_symbol(&self) -> Option<&SpriteRef> {
        self.sprite("HandSymbol")
    }

    pub fn has_property(&self, name: &str) -> bool {
        self.attributes
            .properties
            .iter()
            .any(|p| p.eq_ignore_ascii_case(name))
    }
    pub fn is_flying(&self) -> bool {
        self.has_property("FLYING")
    }
    pub fn is_evil(&self) -> bool {
        self.has_property("EVIL")
    }
    pub fn is_special_digger(&self) -> bool {
        self.has_property("SPECIAL_DIGGER")
    }
    pub fn is_digging_creature(&self) -> bool {
        self.has_property("DIGGING_CREATURE")
    }
    pub fn is_lord(&self) -> bool {
        self.has_property("LORD")
    }
    pub fn is_spectator(&self) -> bool {
        self.has_property("SPECTATOR")
    }
    /// Properties not known to the game (typos), for the editor to flag.
    pub fn unknown_properties(&self) -> Vec<&str> {
        self.attributes
            .properties
            .iter()
            .map(String::as_str)
            .filter(|p| !PROPERTY_NAMES.iter().any(|n| n.eq_ignore_ascii_case(p)))
            .collect()
    }
}
