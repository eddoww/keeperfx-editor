//! TOML "effects" companions of a level: `mapNNNNN.tngfx` (things), `.aptfx` (action points),
//! `.lgtfx` (static lights). They replace the fixed-size legacy `.tng` / `.apt` / `.lgt` records
//! and add things the legacy files cannot hold (creature names, gold values, door state, ...).
//!
//! # File layout (confirmed from `load_kfx_toml_file`, `src/lvl_filesdk1.c:785-855`)
//! * The TOML must parse, and a top-level `[common]` table is mandatory (else the file is
//!   rejected, 806).
//! * Items are either an array of tables (`[[thing]]`, `[[actionpoint]]`, `[[light]]`, taken when
//!   the top-level key is an array, 815) or numbered tables `[thing0]`, `[thing1]`, ... whose count
//!   is `common.ThingsCount` / `ActionPointsCount` / `LightsCount` (819-840). Every real file in
//!   the test install uses the numbered form. A numbered table that is missing or is not a table
//!   makes the load report failure but the other items are still created (843-858).
//! * Maximum counts: things `THINGS_COUNT - 2`, action points `ACTN_POINTS_COUNT - 1` (255),
//!   lights `LIGHTS_COUNT - 1` (2047) (src/lvl_filesdk1.c:862, 910, 1297); exceeding only warns.
//!
//! # Keys (all optional unless said; types as read by the C accessors)
//!
//! Things, `thing_create_thing_adv` (`src/thing_factory.c:217`):
//! * `ThingType` int or string (`value_parse_class`, src/value_util.c:59: Object Shot EffectElem
//!   DeadCreature Creature Effect EffectGen Trap Door AmbientSnd CaveIn; ints are the `TCls_*`
//!   numbers of src/thing_list.h:40). Missing/unknown discards the thing (219).
//! * `Subtype` int; `SubtypeStringID` string overrides it using the config name tables
//!   (`value_parse_model`, src/value_util.c:93, 221-235). `Ownership` int, required, 0..=9
//!   (`PLAYERS_COUNT`).
//! * `SubtileX/Y/Z` = `[subtile, fraction]` (`value_read_stl_coord`, src/value_util.h:16): raw
//!   coordinate `(subtile << 8) | (fraction & 0xFF)`; both entries must be != -1.
//! * `ParentTile` int (Object, EffectGen, Trap, classes 10/11), `Orientation` int (Object,
//!   Creature, Trap; sets `move_angle_xy`), `HerogateNumber` int (hero gate objects only, 255),
//!   `CustomBox` int (custom special boxes, -1 means 0, 262), `GoldValue` int (gold piles and
//!   spinning coins, 275).
//! * Creatures: `CreatureLevel` 1..=10 (else 0 with a warning, 315), `CreatureGold`,
//!   `CreatureInitialHealth` (percent of max health), `CreatureName` string (<
//!   `CREATURE_NAME_MAX`).
//! * `EffectRange` `[subtile, fraction]` (EffectGen, 346). `DoorOrientation` int (-1 -> 0),
//!   `DoorLocked` int (-1 -> 0) (Door, 370).
//! * `Index` is written by existing tools (legacy thing index, present on creature / door / trap
//!   entries) but **never read** by the game's TOML path.
//!
//! Action points, `actnpoint_create_actnpoint_adv` (`src/actionpt.c:81`): `PointNumber` int >= 0
//! (else rejected), `SubtileX`, `SubtileY`, `PointRange` (all `[subtile, fraction]`).
//!
//! Lights, `light_create_light_adv` (`src/light_data.c:188`): `Dynamic` bool or int
//! (`value_coerce_bool`, src/value_util.h:10), `SubtileX/Y/Z`, `LightRange` (`[subtile, fraction]`),
//! `LightIntensity` uint, `ParentTile` uint (the attached slab).
//!
//! Unknown keys are preserved in `extra` (sorted by name) and re-emitted. A known key holding a
//! value of an unexpected type is also kept in `extra` and reported in `warnings`.
//!
//! # Reading like the game
//! Files are parsed with the port of the game's reader ([`crate::game_toml`]), so the editor
//! opens every file the game opens and sees the same keys (a quoted `"ThingType"` is not
//! `ThingType` for the game and stays an unknown key). Text the game's reader rejects is read with
//! the `toml` crate when that accepts it, with a warning ([`FxFile::game_error`]: the game ignores
//! such a file; saving writes it in a form the game reads). Text neither accepts is an error giving
//! the line and column. Text that is not UTF-8 is read as Latin-1 (the game does not look at the
//! encoding) and written back as Latin-1, so names keep their bytes.
//!
//! # Writing
//! Values, keys and preserved unknown tables are written by [`crate::toml_emit`], which only
//! produces forms the game's reader accepts and reads unchanged.
//!
//! # Output shape
//! [`FxFile::to_text`] writes what the existing tools write (verified byte-exact on all 48 real
//! files in the test install, including the final-newline difference, see `tests/level_fx.rs`):
//! `[common]`, `<Count> = N`, then for each item a blank line, `[thingN]` and `Key = value` lines
//! in a fixed key order; no trailing newline.
use crate::cfg::latin1_decode;
use crate::{game_toml, rnc, toml_emit, Error, Result};
use std::collections::BTreeMap;
use std::path::Path;
use toml::Value;

pub type Extra = BTreeMap<String, Value>;

/// `[subtile, fraction]` map coordinate (src/value_util.h:16).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StlCoord {
    pub stl: i32,
    pub sub: i32,
}

impl StlCoord {
    pub fn new(stl: i32, sub: i32) -> Self {
        StlCoord { stl, sub }
    }
    /// Raw 8.8 fixed point value as stored in the game (and in legacy files).
    pub fn raw(self) -> u32 {
        ((self.stl as u32) << 8) | (self.sub as u32 & 0xFF)
    }
    pub fn from_raw(raw: u32) -> Self {
        StlCoord {
            stl: (raw >> 8) as i32,
            sub: (raw & 0xFF) as i32,
        }
    }
    fn to_value(self) -> String {
        format!("[{}, {}]", self.stl, self.sub)
    }
}

/// `ThingType`: a class name or the numeric `TCls_*` value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FxClass {
    Name(String),
    Id(i64),
}

/// Class names accepted by `value_parse_class` with their `TCls_*` numbers (src/thing_list.h:40).
pub const CLASS_NAMES: [(&str, u8); 11] = [
    ("Object", 1),
    ("Shot", 2),
    ("EffectElem", 3),
    ("DeadCreature", 4),
    ("Creature", 5),
    ("Effect", 6),
    ("EffectGen", 7),
    ("Trap", 8),
    ("Door", 9),
    ("AmbientSnd", 12),
    ("CaveIn", 13),
];

impl FxClass {
    /// Numeric `TCls_*` value, `None` for unknown names.
    pub fn id(&self) -> Option<i64> {
        match self {
            FxClass::Id(i) => Some(*i),
            FxClass::Name(n) => CLASS_NAMES
                .iter()
                .find(|(c, _)| c == n)
                .map(|&(_, i)| i as i64),
        }
    }
    /// Class name (for numeric values with a known name).
    pub fn name(&self) -> Option<&str> {
        match self {
            FxClass::Name(n) => Some(n),
            FxClass::Id(i) => CLASS_NAMES
                .iter()
                .find(|&&(_, c)| c as i64 == *i)
                .map(|&(n, _)| n),
        }
    }
    /// The class by number, preferring the name spelling existing tools write.
    pub fn from_id(id: u8) -> FxClass {
        match CLASS_NAMES.iter().find(|&&(_, c)| c == id) {
            Some(&(n, _)) => FxClass::Name(n.to_string()),
            None => FxClass::Id(id as i64),
        }
    }
    pub fn is(&self, name: &str) -> bool {
        self.name() == Some(name)
    }
}

/// `Dynamic`: bool or int (`value_coerce_bool`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxFlag {
    Bool(bool),
    Int(i64),
}
impl FxFlag {
    pub fn is_set(self) -> bool {
        match self {
            FxFlag::Bool(b) => b,
            FxFlag::Int(i) => i != 0,
        }
    }
}

// ---------------------------------------------------------------- reading helpers

struct Rd<'a> {
    t: toml::Table,
    ctx: String,
    warnings: &'a mut Vec<String>,
}

impl Rd<'_> {
    fn bad(&mut self, key: &str, v: Value, want: &str) {
        self.warnings.push(format!(
            "{}: key {key} should be {want}, got {}; kept as extra",
            self.ctx,
            v.type_str()
        ));
        self.t.insert(key.to_string(), v);
    }
    fn int(&mut self, key: &str) -> Option<i64> {
        match self.t.remove(key)? {
            Value::Integer(i) => Some(i),
            v => {
                self.bad(key, v, "an integer");
                None
            }
        }
    }
    fn string(&mut self, key: &str) -> Option<String> {
        match self.t.remove(key)? {
            Value::String(s) => Some(s),
            v => {
                self.bad(key, v, "a string");
                None
            }
        }
    }
    fn coord(&mut self, key: &str) -> Option<StlCoord> {
        let v = self.t.remove(key)?;
        if let Value::Array(a) = &v {
            if let (Some(Value::Integer(x)), Some(Value::Integer(y))) = (a.first(), a.get(1)) {
                if let (Ok(x), Ok(y)) = (i32::try_from(*x), i32::try_from(*y)) {
                    if a.len() == 2 {
                        return Some(StlCoord { stl: x, sub: y });
                    }
                }
            }
        }
        self.bad(key, v, "[subtile, fraction]");
        None
    }
    fn flag(&mut self, key: &str) -> Option<FxFlag> {
        match self.t.remove(key)? {
            Value::Boolean(b) => Some(FxFlag::Bool(b)),
            Value::Integer(i) => Some(FxFlag::Int(i)),
            v => {
                self.bad(key, v, "a bool or integer");
                None
            }
        }
    }
    fn extra(self) -> Extra {
        self.t.into_iter().collect()
    }
}

struct Out(Vec<(String, String)>);
impl Out {
    fn int(&mut self, k: &str, v: Option<i64>) {
        if let Some(v) = v {
            self.0.push((k.into(), v.to_string()));
        }
    }
    fn string(&mut self, k: &str, v: &Option<String>) {
        if let Some(v) = v {
            self.0.push((k.into(), toml_emit::string(v)));
        }
    }
    fn coord(&mut self, k: &str, v: Option<StlCoord>) {
        if let Some(v) = v {
            self.0.push((k.into(), v.to_value()));
        }
    }
    fn extra(&mut self, e: &Extra) {
        for (k, v) in e {
            self.0.push((toml_emit::key(k), toml_emit::value(v)));
        }
    }
}

// ---------------------------------------------------------------- item types

/// One `[thingN]` of a `.tngfx`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FxThing {
    pub class: Option<FxClass>,
    pub subtype: Option<i64>,
    pub subtype_name: Option<String>,
    pub owner: Option<i64>,
    /// Written by existing tools, ignored by the game.
    pub index: Option<i64>,
    pub x: Option<StlCoord>,
    pub y: Option<StlCoord>,
    pub z: Option<StlCoord>,
    pub parent_tile: Option<i64>,
    pub orientation: Option<i64>,
    pub herogate_number: Option<i64>,
    pub custom_box: Option<i64>,
    pub gold_value: Option<i64>,
    pub creature_level: Option<i64>,
    pub creature_gold: Option<i64>,
    pub creature_initial_health: Option<i64>,
    pub creature_name: Option<String>,
    pub effect_range: Option<StlCoord>,
    pub door_orientation: Option<i64>,
    pub door_locked: Option<i64>,
    pub extra: Extra,
}

/// One `[actionpointN]` of an `.aptfx`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FxActionPoint {
    pub number: Option<i64>,
    pub x: Option<StlCoord>,
    pub y: Option<StlCoord>,
    pub range: Option<StlCoord>,
    pub extra: Extra,
}

/// One `[lightN]` of a `.lgtfx`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FxLight {
    pub dynamic: Option<FxFlag>,
    pub x: Option<StlCoord>,
    pub y: Option<StlCoord>,
    pub z: Option<StlCoord>,
    pub range: Option<StlCoord>,
    pub intensity: Option<i64>,
    pub parent_tile: Option<i64>,
    pub extra: Extra,
}

/// An item kind of an fx file.
pub trait FxItem: Sized + Default + PartialEq {
    /// Top-level key / table prefix (`thing`, `actionpoint`, `light`).
    const SECTION: &'static str;
    /// Key inside `[common]` holding the item count.
    const COUNT_KEY: &'static str;
    /// File format name for errors.
    const WHAT: &'static str;
    #[doc(hidden)]
    fn read(t: toml::Table, ctx: String, w: &mut Vec<String>) -> Self;
    #[doc(hidden)]
    fn write(&self) -> Vec<(String, String)>;
}

impl FxItem for FxThing {
    const SECTION: &'static str = "thing";
    const COUNT_KEY: &'static str = "ThingsCount";
    const WHAT: &'static str = "tngfx";
    fn read(t: toml::Table, ctx: String, w: &mut Vec<String>) -> Self {
        let mut r = Rd {
            t,
            ctx,
            warnings: w,
        };
        let class = match r.t.remove("ThingType") {
            Some(Value::String(s)) => Some(FxClass::Name(s)),
            Some(Value::Integer(i)) => Some(FxClass::Id(i)),
            Some(v) => {
                r.bad("ThingType", v, "a string or integer");
                None
            }
            None => None,
        };
        FxThing {
            class,
            subtype: r.int("Subtype"),
            subtype_name: r.string("SubtypeStringID"),
            owner: r.int("Ownership"),
            index: r.int("Index"),
            x: r.coord("SubtileX"),
            y: r.coord("SubtileY"),
            z: r.coord("SubtileZ"),
            parent_tile: r.int("ParentTile"),
            orientation: r.int("Orientation"),
            herogate_number: r.int("HerogateNumber"),
            custom_box: r.int("CustomBox"),
            gold_value: r.int("GoldValue"),
            creature_level: r.int("CreatureLevel"),
            creature_gold: r.int("CreatureGold"),
            creature_initial_health: r.int("CreatureInitialHealth"),
            creature_name: r.string("CreatureName"),
            effect_range: r.coord("EffectRange"),
            door_orientation: r.int("DoorOrientation"),
            door_locked: r.int("DoorLocked"),
            extra: r.extra(),
        }
    }
    fn write(&self) -> Vec<(String, String)> {
        let mut o = Out(vec![]);
        match &self.class {
            Some(FxClass::Name(n)) => o.0.push(("ThingType".into(), toml_emit::string(n))),
            Some(FxClass::Id(i)) => o.int("ThingType", Some(*i)),
            None => {}
        }
        o.int("Subtype", self.subtype);
        o.string("SubtypeStringID", &self.subtype_name);
        o.int("Ownership", self.owner);
        o.int("Index", self.index);
        o.coord("EffectRange", self.effect_range);
        o.int("ParentTile", self.parent_tile);
        o.int("HerogateNumber", self.herogate_number);
        o.int("CustomBox", self.custom_box);
        o.int("CreatureLevel", self.creature_level);
        o.string("CreatureName", &self.creature_name);
        o.int("CreatureGold", self.creature_gold);
        o.int("CreatureInitialHealth", self.creature_initial_health);
        o.int("DoorOrientation", self.door_orientation);
        o.int("DoorLocked", self.door_locked);
        o.int("Orientation", self.orientation);
        o.int("GoldValue", self.gold_value);
        o.coord("SubtileX", self.x);
        o.coord("SubtileY", self.y);
        o.coord("SubtileZ", self.z);
        o.extra(&self.extra);
        o.0
    }
}

impl FxItem for FxActionPoint {
    const SECTION: &'static str = "actionpoint";
    const COUNT_KEY: &'static str = "ActionPointsCount";
    const WHAT: &'static str = "aptfx";
    fn read(t: toml::Table, ctx: String, w: &mut Vec<String>) -> Self {
        let mut r = Rd {
            t,
            ctx,
            warnings: w,
        };
        FxActionPoint {
            number: r.int("PointNumber"),
            x: r.coord("SubtileX"),
            y: r.coord("SubtileY"),
            range: r.coord("PointRange"),
            extra: r.extra(),
        }
    }
    fn write(&self) -> Vec<(String, String)> {
        // existing tools write keys alphabetically
        let mut o = Out(vec![]);
        o.int("PointNumber", self.number);
        o.coord("PointRange", self.range);
        o.coord("SubtileX", self.x);
        o.coord("SubtileY", self.y);
        o.extra(&self.extra);
        o.0
    }
}

impl FxItem for FxLight {
    const SECTION: &'static str = "light";
    const COUNT_KEY: &'static str = "LightsCount";
    const WHAT: &'static str = "lgtfx";
    fn read(t: toml::Table, ctx: String, w: &mut Vec<String>) -> Self {
        let mut r = Rd {
            t,
            ctx,
            warnings: w,
        };
        FxLight {
            dynamic: r.flag("Dynamic"),
            intensity: r.int("LightIntensity"),
            range: r.coord("LightRange"),
            parent_tile: r.int("ParentTile"),
            x: r.coord("SubtileX"),
            y: r.coord("SubtileY"),
            z: r.coord("SubtileZ"),
            extra: r.extra(),
        }
    }
    fn write(&self) -> Vec<(String, String)> {
        // existing tools write keys alphabetically
        let mut o = Out(vec![]);
        match self.dynamic {
            Some(FxFlag::Bool(b)) => o.0.push(("Dynamic".into(), b.to_string())),
            Some(FxFlag::Int(i)) => o.int("Dynamic", Some(i)),
            None => {}
        }
        o.int("LightIntensity", self.intensity);
        o.coord("LightRange", self.range);
        o.int("ParentTile", self.parent_tile);
        o.coord("SubtileX", self.x);
        o.coord("SubtileY", self.y);
        o.coord("SubtileZ", self.z);
        o.extra(&self.extra);
        o.0
    }
}

// ---------------------------------------------------------------- file

/// How the items are laid out in the TOML.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FxForm {
    /// `[thing0]`, `[thing1]`, ... with `Count` in `[common]` (what every real file uses).
    #[default]
    Numbered,
    /// `[[thing]]` array of tables.
    Array,
}

/// A parsed `.tngfx` / `.aptfx` / `.lgtfx`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FxFile<T: FxItem> {
    pub items: Vec<T>,
    pub form: FxForm,
    /// Unknown keys of `[common]` (the count key is regenerated).
    pub common_extra: Extra,
    /// Other top-level entries (e.g. numbered tables beyond the count) preserved verbatim.
    pub other: Extra,
    /// Whether `[common]` carried the count key (matters for [`FxForm::Array`] only).
    pub had_count: bool,
    /// Whether the text ended with a newline (14 of 16 real file triples do not, 2 do).
    pub trailing_newline: bool,
    /// Non-fatal oddities found while parsing (not part of equality semantics).
    pub warnings: Vec<String>,
    /// The text was not UTF-8 and was read as Latin-1; it is written back as Latin-1.
    pub latin1: bool,
    /// Why the game's TOML reader rejects the loaded text although the editor could read it
    /// (the game then ignores the file; see the module docs).
    pub game_error: Option<String>,
}

pub type TngFx = FxFile<FxThing>;
pub type AptFx = FxFile<FxActionPoint>;
pub type LgtFx = FxFile<FxLight>;

impl<T: FxItem> FxFile<T> {
    pub fn new() -> Self {
        FxFile {
            items: vec![],
            form: FxForm::Numbered,
            common_extra: Extra::new(),
            other: Extra::new(),
            had_count: true,
            trailing_newline: false,
            warnings: vec![],
            latin1: false,
            game_error: None,
        }
    }

    /// Parse TOML text (see "Reading like the game" in the module docs). The error of a file
    /// neither reader accepts gives the line and column.
    pub fn parse(text: &str) -> Result<Self> {
        let (root, mut notes, game_error) = match game_toml::parse(text) {
            Ok(g) => match game_toml::to_toml_like_game(&g) {
                Ok(root) => (root, vec![], None),
                // a number the game cannot store (`0x10`, `1_000`): take the TOML reading
                Err(why) => match text.parse::<toml::Table>() {
                    Ok(root) => (root, vec![format!("{why}; the TOML value is kept")], None),
                    Err(e) => {
                        return Err(Error::format(
                            T::WHAT,
                            format!("TOML: {}", one_line(&e.to_string())),
                        ))
                    }
                },
            },
            Err(ge) => match text.parse::<toml::Table>() {
                Ok(root) => (root, vec![], Some(ge.to_string())),
                Err(e) => {
                    return Err(Error::format(
                        T::WHAT,
                        format!(
                            "TOML: {} (the game's reader: {ge})",
                            one_line(&e.to_string())
                        ),
                    ))
                }
            },
        };
        let mut f = Self::from_table(root, text)?;
        if let Some(g) = &game_error {
            notes.push(format!(
                "the game's TOML reader rejects this file ({g}); the game ignores \
                it until it is saved again"
            ));
        }
        f.warnings.splice(0..0, notes);
        f.game_error = game_error;
        Ok(f)
    }

    fn from_table(mut root: toml::Table, text: &str) -> Result<Self> {
        let mut common = match root.remove("common") {
            Some(Value::Table(t)) => t,
            _ => {
                return Err(Error::format(
                    T::WHAT,
                    "missing [common] table (the game rejects the file)",
                ))
            }
        };
        let mut warnings = vec![];
        let count_val = common.remove(T::COUNT_KEY);
        let had_count = count_val.is_some();
        let mut items = vec![];
        let form;
        match root.remove(T::SECTION) {
            Some(Value::Array(arr)) => {
                form = FxForm::Array;
                for (i, v) in arr.into_iter().enumerate() {
                    match v {
                        Value::Table(t) => {
                            items.push(T::read(t, format!("{}[{i}]", T::SECTION), &mut warnings))
                        }
                        _ => warnings.push(format!(
                            "{}[{i}] is not a table (game: Invalid section)",
                            T::SECTION
                        )),
                    }
                }
            }
            first => {
                form = FxForm::Numbered;
                if let Some(v) = first {
                    // a single `[thing]` table or scalar under the bare name: not a layout the game
                    // reads
                    root.insert(T::SECTION.to_string(), v);
                }
                let total = match count_val {
                    Some(Value::Integer(n)) => n,
                    other => {
                        // The game reads value_int32(missing); infer instead so the file stays
                        // usable.
                        let mut n = 0;
                        while root.contains_key(&format!("{}{n}", T::SECTION)) {
                            n += 1;
                        }
                        warnings.push(format!(
                            "{} missing or not an integer ({other:?}); inferred {n}",
                            T::COUNT_KEY
                        ));
                        n
                    }
                };
                if total < 0 {
                    return Err(Error::format(T::WHAT, format!("negative {}", T::COUNT_KEY)));
                }
                for k in 0..total {
                    let key = format!("{}{k}", T::SECTION);
                    match root.remove(&key) {
                        Some(Value::Table(t)) => items.push(T::read(t, key, &mut warnings)),
                        _ => warnings.push(format!(
                            "{key} missing or not a table (game: Invalid section)"
                        )),
                    }
                }
            }
        }
        Ok(FxFile {
            items,
            form,
            common_extra: common.into_iter().collect(),
            other: root.into_iter().collect(),
            had_count,
            trailing_newline: text.ends_with('\n'),
            warnings,
            latin1: false,
            game_error: None,
        })
    }

    /// Parse raw bytes (UTF-8; falls back to Latin-1 with a warning, since the game does not
    /// validate encoding).
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        match std::str::from_utf8(b) {
            Ok(s) => Self::parse(s),
            Err(_) => {
                let mut f = Self::parse(&latin1_decode(b))?;
                f.warnings.push(
                    "not valid UTF-8; decoded as Latin-1 (and written back as Latin-1)".into(),
                );
                f.latin1 = true;
                Ok(f)
            }
        }
    }

    /// Load through [`rnc::load_file`].
    pub fn load(path: &Path) -> Result<Self> {
        Self::from_bytes(&rnc::load_file(path)?)
    }

    /// Serialise in the shape existing tools use (see module docs).
    pub fn to_text(&self) -> String {
        // top-level values among the preserved entries must come before the first header
        let other: toml::Table = self.other.clone().into_iter().collect();
        let mut s = String::new();
        toml_emit::top_values(&mut s, &other);
        s.push_str("[common]\n");
        if self.form == FxForm::Numbered || self.had_count {
            s.push_str(&format!("{} = {}\n", T::COUNT_KEY, self.items.len()));
        }
        for (k, v) in &self.common_extra {
            toml_emit::keyval(&mut s, k, v);
        }
        for (i, it) in self.items.iter().enumerate() {
            match self.form {
                FxForm::Numbered => s.push_str(&format!("\n[{}{i}]\n", T::SECTION)),
                FxForm::Array => s.push_str(&format!("\n[[{}]]\n", T::SECTION)),
            }
            for (k, v) in it.write() {
                s.push_str(&format!("{k} = {v}\n"));
            }
        }
        // preserved tables, with the parent headers the game needs (see `toml_emit`)
        toml_emit::sections(&mut s, &other);
        // most existing tools do not terminate the last line
        if s.ends_with('\n') && !self.trailing_newline {
            s.pop();
        }
        s
    }

    /// [`FxFile::to_text`] encoded: UTF-8, or Latin-1 for a file read as Latin-1 (when every
    /// character fits).
    pub fn to_bytes(&self) -> Vec<u8> {
        let t = self.to_text();
        if self.latin1 && t.chars().all(|c| (c as u32) < 256) {
            crate::cfg::latin1_encode(&t)
        } else {
            t.into_bytes()
        }
    }

    /// The bytes to write, checked with the game's reader: `Err` when the game would reject
    /// them (nothing the writer produces should be; this is the last line of defence).
    pub fn to_game_bytes(&self) -> Result<Vec<u8>> {
        let t = self.to_text();
        game_toml::check(&t).map_err(|e| {
            Error::format(
                T::WHAT,
                format!("the written file would not load in the game ({e})"),
            )
        })?;
        Ok(self.to_bytes())
    }

    /// Write crash-safely ([`crate::atomic::atomic_write`]).
    pub fn save(&self, path: &Path) -> Result<()> {
        let b = self.to_game_bytes()?;
        crate::atomic::atomic_write(path, &b).map_err(|e| Error::io(path, e))
    }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------- legacy records

/// Plain field values of a legacy `.tng` record (`struct LegacyInitThing`, 21 bytes,
/// src/lvl_filesdk1.c:89). Coordinates are the raw 8.8 values. It sits between the `level_bin` record
/// and [`FxThing`] ([`FxThing::from_legacy`], [`FxThing::to_legacy`]), so this module does not
/// depend on the binary reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LegacyThingFields {
    pub x: u16,
    pub y: u16,
    pub z: u16,
    pub oclass: u8,
    pub model: u8,
    pub owner: u8,
    pub range: u16,
    pub index: u16,
    pub params: [u8; 8],
}

/// Plain field values of a legacy `.apt` record (`struct LegacyInitActionPoint`, 8 bytes, 99).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LegacyActionPointFields {
    pub x: u16,
    pub y: u16,
    pub range: u16,
    pub num: u16,
}

/// Plain field values of a legacy `.lgt` record (`struct LegacyInitLight`, 20 bytes, 105).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LegacyLightFields {
    pub radius: i16,
    pub intensity: u8,
    pub flags: u8,
    pub x: u16,
    pub y: u16,
    pub z: u16,
    pub is_dynamic: u8,
    pub attached_slb: i16,
}

/// A legacy Object record cannot say whether `params[1]` is a hero gate number or a custom box
/// kind (that depends on the object model's config), so the caller tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectRole {
    Plain,
    HeroGate,
    CustomBox,
}

fn c(raw: u16) -> Option<StlCoord> {
    Some(StlCoord::from_raw(raw as u32))
}

impl FxThing {
    /// Convert a legacy record following `thing_create_thing` (src/thing_factory.c:99-190): Object
    /// `index` = parent tile and `params[1]` = hero gate / custom box; Creature `params[1]` =
    /// 0-based level (file levels are 1-based); EffectGen `range`/`index`; Trap `index`; Door
    /// `params[0]` = orientation, `params[1]` = locked. Creature/Door/Trap also get `Index`, as in
    /// real files. Fields the legacy format cannot hold stay `None`.
    pub fn from_legacy(l: &LegacyThingFields, role: ObjectRole) -> FxThing {
        let mut t = FxThing {
            class: Some(FxClass::from_id(l.oclass)),
            subtype: Some(l.model as i64),
            owner: Some(l.owner as i64),
            x: c(l.x),
            y: c(l.y),
            z: c(l.z),
            ..Default::default()
        };
        match l.oclass {
            1 => {
                t.parent_tile = Some(l.index as i64);
                match role {
                    ObjectRole::HeroGate => t.herogate_number = Some(l.params[1] as i64),
                    ObjectRole::CustomBox => t.custom_box = Some(l.params[1] as i64),
                    ObjectRole::Plain => {}
                }
            }
            5 => {
                t.index = Some(l.index as i64);
                t.creature_level = Some(l.params[1] as i64 + 1);
            }
            7 => {
                t.effect_range = Some(StlCoord::from_raw(l.range as u32));
                t.parent_tile = Some(l.index as i64);
            }
            8 => t.index = Some(l.index as i64), // create_trap ignores the parent index
            9 => {
                t.index = Some(l.index as i64);
                t.door_orientation = Some(l.params[0] as i64);
                t.door_locked = Some(l.params[1] as i64);
            }
            _ => {}
        }
        t
    }

    /// Convert to a legacy record, `None` when the class/subtype is not numeric or a value does
    /// not fit the legacy field widths. Information the legacy format cannot hold (names, gold,
    /// health, orientation of objects, ...) is dropped.
    pub fn to_legacy(&self) -> Option<LegacyThingFields> {
        let oclass = u8::try_from(self.class.as_ref()?.id()?).ok()?;
        let raw = |c: Option<StlCoord>| c.map(|c| u16::try_from(c.raw()).ok()).unwrap_or(Some(0));
        let mut l = LegacyThingFields {
            x: raw(self.x)?,
            y: raw(self.y)?,
            z: raw(self.z)?,
            oclass,
            model: u8::try_from(self.subtype?).ok()?,
            owner: u8::try_from(self.owner?).ok()?,
            ..Default::default()
        };
        let idx = |v: Option<i64>| v.and_then(|v| u16::try_from(v).ok());
        match oclass {
            1 => {
                l.index = idx(self.parent_tile).unwrap_or(0);
                l.params[1] = self
                    .herogate_number
                    .or(self.custom_box)
                    .and_then(|v| u8::try_from(v).ok())
                    .unwrap_or(0);
            }
            5 => {
                l.index = idx(self.index).unwrap_or(0);
                l.params[1] = self
                    .creature_level
                    .map(|v| (v.clamp(1, 10) - 1) as u8)
                    .unwrap_or(0);
            }
            7 => {
                l.range = self.effect_range.map(|c| c.raw() as u16).unwrap_or(0);
                l.index = idx(self.parent_tile).unwrap_or(0);
            }
            8 => l.index = idx(self.index).or(idx(self.parent_tile)).unwrap_or(0),
            9 => {
                l.index = idx(self.index).unwrap_or(0);
                l.params[0] = self
                    .door_orientation
                    .and_then(|v| u8::try_from(v).ok())
                    .unwrap_or(0);
                l.params[1] = self
                    .door_locked
                    .and_then(|v| u8::try_from(v).ok())
                    .unwrap_or(0);
            }
            _ => {}
        }
        Some(l)
    }
}

impl FxActionPoint {
    pub fn from_legacy(l: &LegacyActionPointFields) -> FxActionPoint {
        FxActionPoint {
            number: Some(l.num as i64),
            x: c(l.x),
            y: c(l.y),
            range: Some(StlCoord::from_raw(l.range as u32)),
            extra: Extra::new(),
        }
    }
    pub fn to_legacy(&self) -> Option<LegacyActionPointFields> {
        let raw = |c: Option<StlCoord>| c.map(|c| u16::try_from(c.raw()).ok()).unwrap_or(Some(0));
        Some(LegacyActionPointFields {
            x: raw(self.x)?,
            y: raw(self.y)?,
            range: raw(self.range)?,
            num: u16::try_from(self.number?).ok()?,
        })
    }
}

impl FxLight {
    /// `flags` of the legacy record has no TOML counterpart (the game's TOML path does not read
    /// one, src/light_data.c:238) and is dropped.
    pub fn from_legacy(l: &LegacyLightFields) -> FxLight {
        FxLight {
            dynamic: Some(FxFlag::Int(l.is_dynamic as i64)),
            x: c(l.x),
            y: c(l.y),
            z: c(l.z),
            range: Some(StlCoord::from_raw(l.radius as u16 as u32)),
            intensity: Some(l.intensity as i64),
            parent_tile: Some(l.attached_slb as u16 as i64),
            extra: Extra::new(),
        }
    }
    pub fn to_legacy(&self) -> Option<LegacyLightFields> {
        let raw = |c: Option<StlCoord>| c.map(|c| u16::try_from(c.raw()).ok()).unwrap_or(Some(0));
        Some(LegacyLightFields {
            radius: raw(self.range)? as i16,
            intensity: u8::try_from(self.intensity.unwrap_or(0)).ok()?,
            flags: 0,
            x: raw(self.x)?,
            y: raw(self.y)?,
            z: raw(self.z)?,
            is_dynamic: self.dynamic.map(|d| d.is_set() as u8).unwrap_or(0),
            attached_slb: self.parent_tile.map(|v| v as u16 as i16).unwrap_or(0),
        })
    }
}
