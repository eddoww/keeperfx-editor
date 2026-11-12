//! Things in the editor: placing, selecting, moving, deleting, duplicating, rotating, picking,
//! and the property edits of the inspector; drawing them is in `draw`.
use super::*;
use crate::render::SpriteInstance;
use crate::sprites::{Icon, Slot, SpriteLib, Visual, SPRITE_UNITS};
use kfx_formats::level_fx::{FxActionPoint, FxFlag, FxLight, FxThing, StlCoord};
use kfx_map::things::{self as kthings, class, thing_class, ItemRef};
use kfx_map::transform::{Symmetry, Transform};
use kfx_prefab::i18n::{trd, trf, trn, tx, Tx};
use std::collections::BTreeSet;

mod draw;

/// What the Place tool puts down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    Creature(i64),
    Object(i64),
    Trap(i64),
    Door(i64),
    EffectGen(i64),
    Light,
    ActionPoint,
}

impl PlaceKind {
    pub fn class(&self) -> Option<u8> {
        match self {
            PlaceKind::Creature(_) => Some(class::CREATURE),
            PlaceKind::Object(_) => Some(class::OBJECT),
            PlaceKind::Trap(_) => Some(class::TRAP),
            PlaceKind::Door(_) => Some(class::DOOR),
            PlaceKind::EffectGen(_) => Some(class::EFFECT_GEN),
            _ => None,
        }
    }
    pub fn model(&self) -> i64 {
        match self {
            PlaceKind::Creature(m)
            | PlaceKind::Object(m)
            | PlaceKind::Trap(m)
            | PlaceKind::Door(m)
            | PlaceKind::EffectGen(m) => *m,
            _ => 0,
        }
    }
    /// `creature:IMP`, `object:GOLD`, `trap:BOULDER`, `door:WOOD`, `effect:NAME`, `light`,
    /// `action_point` (names or model numbers; a bare name is searched in every class).
    pub fn parse(s: &str, gd: &GameData) -> Option<PlaceKind> {
        let s = s.trim();
        let (k, n) = s
            .split_once(':')
            .map(|(a, b)| (a.trim().to_ascii_lowercase(), b.trim()))
            .unwrap_or((String::new(), s));
        let num = |list: Vec<(usize, String)>| -> Option<i64> {
            n.parse::<i64>()
                .ok()
                .filter(|m| list.iter().any(|(i, _)| *i as i64 == *m))
                .or_else(|| {
                    list.iter()
                        .find(|(_, x)| x.eq_ignore_ascii_case(n))
                        .map(|(i, _)| *i as i64)
                })
        };
        let creature = || {
            n.parse::<i64>()
                .ok()
                .filter(|&m| m >= 1 && m as usize <= gd.creatures.len())
                .or_else(|| gd.creature_id(n).map(|m| m as i64))
        };
        let effects = || gd.effect_generator_names();
        match k.as_str() {
            "creature" => creature().map(PlaceKind::Creature),
            "object" => num(gd.object_names()).map(PlaceKind::Object),
            "trap" => num(gd.trap_names()).map(PlaceKind::Trap),
            "door" => num(gd.door_names()).map(PlaceKind::Door),
            "effect" | "effectgen" => num(effects()).map(PlaceKind::EffectGen),
            _ if n.eq_ignore_ascii_case("light") => Some(PlaceKind::Light),
            _ if n.eq_ignore_ascii_case("action_point")
                || n.eq_ignore_ascii_case("actionpoint") =>
            {
                Some(PlaceKind::ActionPoint)
            }
            "" => creature()
                .map(PlaceKind::Creature)
                .or_else(|| num(gd.object_names()).map(PlaceKind::Object))
                .or_else(|| num(gd.trap_names()).map(PlaceKind::Trap))
                .or_else(|| num(gd.door_names()).map(PlaceKind::Door))
                .or_else(|| num(effects()).map(PlaceKind::EffectGen)),
            _ => None,
        }
    }
    pub fn label(&self, gd: &GameData) -> String {
        let find = |list: Vec<(usize, String)>, m: i64| {
            list.into_iter()
                .find(|(i, _)| *i as i64 == m)
                .map(|(_, n)| n)
                .unwrap_or_else(|| format!("#{m}"))
        };
        match self {
            PlaceKind::Creature(m) => gd
                .creatures
                .get(*m as usize - 1)
                .cloned()
                .unwrap_or_else(|| trf("kfx_editor.creature_m", "creature #{m}", &[("m", &m)])),
            PlaceKind::Object(m) => find(gd.object_names(), *m),
            PlaceKind::Trap(m) => find(gd.trap_names(), *m),
            PlaceKind::Door(m) => find(gd.door_names(), *m),
            PlaceKind::EffectGen(m) => find(gd.effect_generator_names(), *m),
            PlaceKind::Light => trd("kfx_editor.light", "Light").into(),
            PlaceKind::ActionPoint => trd("kfx_editor.action_point", "Action point").into(),
        }
    }
}

/// Field values new things get (the Create defaults; Alt+click copies a thing's). Persisted in
/// the settings.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PlaceDefaults {
    pub level: i64,
    pub orientation: i64,
    pub creature_gold: i64,
    pub health: i64,
    pub name: String,
    pub gold_value: i64,
    pub custom_box: i64,
    pub effect_range: f32,
    pub locked: bool,
    pub light_range: f32,
    pub light_intensity: i64,
    pub light_height: f32,
    pub ap_range: f32,
}

impl Default for PlaceDefaults {
    fn default() -> Self {
        PlaceDefaults {
            level: 1,
            orientation: 0,
            creature_gold: 0,
            health: 100,
            name: String::new(),
            gold_value: 0,
            custom_box: 0,
            effect_range: 0.0,
            locked: false,
            light_range: 6.0,
            light_intensity: 32,
            light_height: 2.0,
            ap_range: 0.0,
        }
    }
}

/// `rules.cfg` `[game]` gold values (src/config_rules.c:84), for the size of gold piles: the game
/// scales a gold object's sprite with its value (`add_gold_to_pile`, src/thing_objects.c:2134).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GoldRules {
    pub pot: i64,
    pub chest: i64,
    pub bag: i64,
    pub pile: i64,
    pub pile_max: i64,
}

impl Default for GoldRules {
    fn default() -> Self {
        GoldRules {
            pot: 250,
            chest: 500,
            bag: 100,
            pile: 200,
            pile_max: 1024,
        }
    }
}

impl GoldRules {
    /// The values of the layered `rules.cfg` (later layers override).
    pub fn load(layers: &ConfigLayers) -> GoldRules {
        let mut g = GoldRules::default();
        for (p, _) in layers.files("rules.cfg") {
            let Ok(doc) = kfx_formats::cfg::CfgDoc::load(&p) else {
                continue;
            };
            let v = |k: &str| {
                doc.get("game", k)
                    .and_then(|x| x.trim().parse::<i64>().ok())
            };
            g.pot = v("PotOfGoldHolds").unwrap_or(g.pot);
            g.chest = v("ChestGoldHold").unwrap_or(g.chest);
            g.bag = v("BagGoldHold").unwrap_or(g.bag);
            g.pile = v("GoldPileValue").unwrap_or(g.pile);
            g.pile_max = v("GoldPileMaximum").unwrap_or(g.pile_max);
        }
        g
    }

    /// `gold_object_typical_value` (src/thing_objects.c:2106): by hard-coded model number.
    pub fn typical(&self, model: i64) -> i64 {
        match model {
            3 => self.chest,
            6 => self.pot,
            43 => self.pile,
            136 => self.bag,
            128 => self.pile_max,
            _ => 0,
        }
    }

    /// Sprite size of a gold object holding `value` (the configured `MaximumSize` when the game
    /// keeps it: no value given, or a model without a typical value).
    pub fn sprite_size(&self, model: i64, value: Option<i64>, max_size: i32) -> i32 {
        let t = self.typical(model);
        match value {
            Some(v) if t > 0 && v != 0 => {
                let v = if v < 0 { i32::MAX as i64 } else { v };
                let s = if v < t {
                    196 * v / t + 128
                } else {
                    196 + 24 * (v - t) / t + 128
                };
                s.min(640) as i32
            }
            _ => max_size,
        }
    }
}

/// A palette entry of the thing tabs.
pub struct ThingEntry {
    pub kind: PlaceKind,
    pub group: String,
    pub label: String,
    pub tooltip: String,
    pub tex: Option<egui::TextureHandle>,
}

/// Screen box of a drawn item (viewport pixels).
#[derive(Clone, Copy, Debug)]
pub struct HitBox {
    pub item: ItemRef,
    pub min: Vec2,
    pub max: Vec2,
    pub depth: f32,
}

/// A number drawn next to an item (creature level, hero gate, action point).
#[derive(Clone, Debug)]
pub struct Badge {
    pub pos: Vec3,
    pub text: String,
    pub color: [u8; 3],
}

/// Moving the selection with the mouse.
#[derive(Clone, Copy, Debug)]
pub struct ThingDrag {
    pub start: Vec3,
    pub cur: Vec3,
}

/// Rectangle selection in progress (viewport pixels).
#[derive(Clone, Copy, Debug)]
pub struct RectSel {
    pub start: Vec2,
    pub cur: Vec2,
    pub add: bool,
}

/// Map positions as raw 1/256 subtile units.
pub fn raw(c: Option<StlCoord>) -> Option<i64> {
    c.map(|c| c.stl as i64 * 256 + c.sub as i64)
}

pub fn coord(r: i64) -> StlCoord {
    StlCoord::new(r.div_euclid(256) as i32, r.rem_euclid(256) as i32)
}

fn world_of(x: Option<StlCoord>, y: Option<StlCoord>, z: Option<StlCoord>) -> Option<Vec3> {
    let f = |c: StlCoord| c.stl as f32 + c.sub as f32 / 256.0;
    Some(Vec3::new(f(x?), z.map(f).unwrap_or(0.0), f(y?)))
}

/// Items of `new` that are not in `old` (multiset difference), as indices into `new`.
fn added<T: PartialEq>(old: &[T], new: &[T]) -> Vec<usize> {
    let mut used = vec![false; old.len()];
    let mut out = vec![];
    for (i, t) in new.iter().enumerate() {
        match old.iter().enumerate().position(|(j, o)| !used[j] && o == t) {
            Some(j) => used[j] = true,
            None => out.push(i),
        }
    }
    out
}

/// Snapshot of the item lists, to select what an operation added or changed.
pub struct ItemsBefore {
    things: Vec<FxThing>,
    aps: Vec<FxActionPoint>,
    lights: Vec<FxLight>,
}

impl ItemsBefore {
    pub fn of(d: &MapDoc) -> ItemsBefore {
        ItemsBefore {
            things: d.things.items.clone(),
            aps: d.action_points.items.clone(),
            lights: d.lights.items.clone(),
        }
    }
    /// New or changed items (door keys excluded).
    pub fn changed(&self, d: &MapDoc) -> BTreeSet<ItemRef> {
        let mut s = BTreeSet::new();
        for i in added(&self.things, &d.things.items) {
            let t = &d.things.items[i];
            if !(thing_class(t) == Some(class::OBJECT)
                && t.subtype == Some(kthings::OBJ_SPINNING_KEY))
            {
                s.insert(ItemRef::Thing(i));
            }
        }
        s.extend(
            added(&self.aps, &d.action_points.items)
                .into_iter()
                .map(ItemRef::ActionPoint),
        );
        s.extend(
            added(&self.lights, &d.lights.items)
                .into_iter()
                .map(ItemRef::Light),
        );
        s
    }
}

/// Owner colour of a player (`PLAYERS`).
pub fn owner_rgb(o: i64) -> [u8; 3] {
    PLAYERS
        .get(o.clamp(0, 8) as usize)
        .map(|p| p.1)
        .unwrap_or([128, 128, 128])
}

/// Every editable property of the inspector, with its game range.
pub const FIELDS: &[(&str, Tx)] = &[
    ("owner", tx("kfx_editor.owner_0_8", "owner 0..8")),
    ("x", tx("kfx_editor.subtile_x", "subtile x")),
    ("y", tx("kfx_editor.subtile_y", "subtile y")),
    ("sub_x", tx("kfx_editor.0_255", "0..255")),
    ("sub_y", tx("kfx_editor.0_255", "0..255")),
    ("z", tx("kfx_editor.height_subtiles", "height (subtiles)")),
    ("sub_z", tx("kfx_editor.0_255", "0..255")),
    (
        "orientation",
        tx(
            "kfx_editor.0_2047_0_north_512_east",
            "0..2047 (0 north, 512 east)",
        ),
    ),
    (
        "level",
        tx("kfx_editor.creature_level_1_10", "creature level 1..10"),
    ),
    (
        "creature_gold",
        tx(
            "kfx_editor.gold_carried_0_1000000000",
            "gold carried 0..1000000000",
        ),
    ),
    (
        "health",
        tx("kfx_editor.initial_health_0_100", "initial health % 0..100"),
    ),
    (
        "name",
        tx(
            "kfx_editor.unique_name_up_to_24_characters",
            "unique name (up to 24 characters)",
        ),
    ),
    (
        "gold_value",
        tx(
            "kfx_editor.gold_value_0_1000000000",
            "gold value 0..1000000000",
        ),
    ),
    (
        "herogate",
        tx(
            "kfx_editor.hero_gate_number_1_255",
            "hero gate number 1..255",
        ),
    ),
    (
        "custom_box",
        tx(
            "kfx_editor.custom_box_number_0_255",
            "custom box number 0..255",
        ),
    ),
    (
        "effect_range",
        tx(
            "kfx_editor.effect_range_in_subtiles_0_255",
            "effect range in subtiles 0..255",
        ),
    ),
    (
        "locked",
        tx(
            "kfx_editor.door_locked_true_false",
            "door locked: true or false",
        ),
    ),
    (
        "door_orientation",
        tx("kfx_editor.door_orientation_0_1", "door orientation 0/1"),
    ),
    (
        "parent",
        tx(
            "kfx_editor.parent_slab_number_1_or_none_for_none",
            "parent slab number, -1 or none for none",
        ),
    ),
    (
        "light_range",
        tx(
            "kfx_editor.light_range_in_subtiles_0_255",
            "light range in subtiles 0..255",
        ),
    ),
    (
        "intensity",
        tx("kfx_editor.light_intensity_0_255", "light intensity 0..255"),
    ),
    (
        "dynamic",
        tx(
            "kfx_editor.dynamic_light_true_false",
            "dynamic light: true or false",
        ),
    ),
    (
        "number",
        tx(
            "kfx_editor.action_point_number_1_255",
            "action point number 1..255",
        ),
    ),
    (
        "range",
        tx(
            "kfx_editor.action_point_range_in_subtiles_0_255",
            "action point range in subtiles 0..255",
        ),
    ),
];

fn parse_bool(v: &str) -> Result<bool, String> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(trf(
            "kfx_editor.v_is_not_true_false",
            "{v} is not true or false",
            &[("v", &format!("{:?}", v))],
        )),
    }
}

fn parse_int(v: &str, lo: i64, hi: i64) -> Result<i64, String> {
    let n: i64 = v.trim().parse().map_err(|_| {
        trf(
            "kfx_editor.v_is_not_a_whole_number",
            "{v} is not a whole number",
            &[("v", &format!("{:?}", v))],
        )
    })?;
    if n < lo || n > hi {
        return Err(trf(
            "kfx_editor.n_outside_lo_hi",
            "{n} outside {lo}..{hi}",
            &[("n", &n), ("lo", &lo), ("hi", &hi)],
        ));
    }
    Ok(n)
}

fn parse_stl(v: &str, hi: f32) -> Result<StlCoord, String> {
    let f: f32 = v.trim().parse().map_err(|_| {
        trf(
            "kfx_editor.v_is_not_a_number",
            "{v} is not a number",
            &[("v", &format!("{:?}", v))],
        )
    })?;
    if !(0.0..=hi).contains(&f) {
        return Err(trf(
            "kfx_editor.f_outside_0_hi",
            "{f} outside 0..{hi}",
            &[("f", &f), ("hi", &hi)],
        ));
    }
    Ok(coord((f * 256.0).round() as i64))
}

/// Apply one inspector field to a thing; `Ok(false)` when it does not apply to the class.
pub fn set_thing_field(t: &mut FxThing, f: &str, v: &str, dims: MapDims) -> Result<bool, String> {
    let cls = thing_class(t);
    match f {
        "owner" => t.owner = Some(parse_int(v, 0, 8)?),
        "x" => {
            t.x = Some(StlCoord::new(
                parse_int(v, 0, dims.stl_w() as i64 - 1)? as i32,
                t.x.map_or(128, |c| c.sub),
            ))
        }
        "y" => {
            t.y = Some(StlCoord::new(
                parse_int(v, 0, dims.stl_h() as i64 - 1)? as i32,
                t.y.map_or(128, |c| c.sub),
            ))
        }
        "sub_x" => {
            t.x = Some(StlCoord::new(
                t.x.map_or(0, |c| c.stl),
                parse_int(v, 0, 255)? as i32,
            ))
        }
        "sub_y" => {
            t.y = Some(StlCoord::new(
                t.y.map_or(0, |c| c.stl),
                parse_int(v, 0, 255)? as i32,
            ))
        }
        "z" => {
            t.z = Some(StlCoord::new(
                parse_int(v, -255, 255)? as i32,
                t.z.map_or(0, |c| c.sub),
            ))
        }
        "sub_z" => {
            t.z = Some(StlCoord::new(
                t.z.map_or(0, |c| c.stl),
                parse_int(v, 0, 255)? as i32,
            ))
        }
        "orientation" => t.orientation = Some(parse_int(v, 0, 2047)?),
        "level" if cls == Some(class::CREATURE) => t.creature_level = Some(parse_int(v, 1, 10)?),
        "creature_gold" if cls == Some(class::CREATURE) => {
            t.creature_gold = Some(parse_int(v, 0, 1_000_000_000)?)
        }
        "health" if cls == Some(class::CREATURE) => {
            t.creature_initial_health = Some(parse_int(v, 0, 100)?)
        }
        "name" if cls == Some(class::CREATURE) => {
            let n = v.trim();
            if n.chars().count() > 24 {
                return Err(trd(
                    "kfx_editor.names_are_at_most_24_characters",
                    "names are at most 24 characters",
                ));
            }
            t.creature_name = (!n.is_empty()).then(|| n.to_string());
        }
        "gold_value" if cls == Some(class::OBJECT) => {
            t.gold_value = Some(parse_int(v, 0, 1_000_000_000)?)
        }
        "herogate" if cls == Some(class::OBJECT) => t.herogate_number = Some(parse_int(v, 1, 255)?),
        "custom_box" if cls == Some(class::OBJECT) => t.custom_box = Some(parse_int(v, 0, 255)?),
        "effect_range" if cls == Some(class::EFFECT_GEN) => {
            t.effect_range = Some(parse_stl(v, 255.0)?)
        }
        "locked" if cls == Some(class::DOOR) => t.door_locked = Some(parse_bool(v)? as i64),
        "door_orientation" if cls == Some(class::DOOR) => {
            t.door_orientation = Some(parse_int(v, 0, 1)?)
        }
        "parent" => {
            let s = v.trim();
            t.parent_tile = if s.is_empty() || s.eq_ignore_ascii_case("none") || s == "-1" {
                None
            } else {
                Some(parse_int(s, 0, dims.slabs() as i64 - 1)?)
            };
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub fn set_light_field(l: &mut FxLight, f: &str, v: &str, dims: MapDims) -> Result<bool, String> {
    match f {
        "x" => {
            l.x = Some(StlCoord::new(
                parse_int(v, 0, dims.stl_w() as i64 - 1)? as i32,
                l.x.map_or(128, |c| c.sub),
            ))
        }
        "y" => {
            l.y = Some(StlCoord::new(
                parse_int(v, 0, dims.stl_h() as i64 - 1)? as i32,
                l.y.map_or(128, |c| c.sub),
            ))
        }
        "sub_x" => {
            l.x = Some(StlCoord::new(
                l.x.map_or(0, |c| c.stl),
                parse_int(v, 0, 255)? as i32,
            ))
        }
        "sub_y" => {
            l.y = Some(StlCoord::new(
                l.y.map_or(0, |c| c.stl),
                parse_int(v, 0, 255)? as i32,
            ))
        }
        "z" => {
            l.z = Some(StlCoord::new(
                parse_int(v, 0, 255)? as i32,
                l.z.map_or(0, |c| c.sub),
            ))
        }
        "sub_z" => {
            l.z = Some(StlCoord::new(
                l.z.map_or(0, |c| c.stl),
                parse_int(v, 0, 255)? as i32,
            ))
        }
        "light_range" | "range" => l.range = Some(parse_stl(v, 255.0)?),
        "intensity" => l.intensity = Some(parse_int(v, 0, 255)?),
        "dynamic" => l.dynamic = Some(FxFlag::Bool(parse_bool(v)?)),
        "parent" => {
            let s = v.trim();
            l.parent_tile = if s.is_empty() || s.eq_ignore_ascii_case("none") || s == "-1" {
                None
            } else {
                Some(parse_int(s, 0, dims.slabs() as i64 - 1)?)
            };
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub fn set_ap_field(
    a: &mut FxActionPoint,
    f: &str,
    v: &str,
    dims: MapDims,
) -> Result<bool, String> {
    match f {
        "x" => {
            a.x = Some(StlCoord::new(
                parse_int(v, 0, dims.stl_w() as i64 - 1)? as i32,
                a.x.map_or(128, |c| c.sub),
            ))
        }
        "y" => {
            a.y = Some(StlCoord::new(
                parse_int(v, 0, dims.stl_h() as i64 - 1)? as i32,
                a.y.map_or(128, |c| c.sub),
            ))
        }
        "sub_x" => {
            a.x = Some(StlCoord::new(
                a.x.map_or(0, |c| c.stl),
                parse_int(v, 0, 255)? as i32,
            ))
        }
        "sub_y" => {
            a.y = Some(StlCoord::new(
                a.y.map_or(0, |c| c.stl),
                parse_int(v, 0, 255)? as i32,
            ))
        }
        "number" => a.number = Some(parse_int(v, 1, 255)?),
        "range" => a.range = Some(parse_stl(v, 255.0)?),
        _ => return Ok(false),
    }
    Ok(true)
}

/// Whether layers `l` show thing `t`.
pub fn thing_visible(l: &crate::settings::Layers, t: &FxThing) -> bool {
    if t.parent_tile.is_some() && !l.slab_things && thing_class(t) != Some(class::DOOR) {
        return false;
    }
    match thing_class(t) {
        Some(class::CREATURE) => l.creatures,
        Some(class::OBJECT) => l.objects,
        Some(class::TRAP) => l.traps,
        Some(class::DOOR) => l.doors,
        Some(class::EFFECT_GEN) => l.effects,
        _ => l.objects,
    }
}

/// Class name shown to the user.
pub fn class_name(c: Option<u8>) -> Tx {
    match c {
        Some(class::OBJECT) => tx("kfx_editor.object", "Object"),
        Some(class::CREATURE) => tx("kfx_editor.creature", "Creature"),
        Some(class::EFFECT_GEN) => tx("kfx_editor.effect_generator", "Effect generator"),
        Some(class::TRAP) => tx("kfx_editor.trap", "Trap"),
        Some(class::DOOR) => tx("kfx_editor.door", "Door"),
        _ => tx("kfx_editor.unknown", "Unknown"),
    }
}

/// Model name of a thing.
pub fn model_name(gd: &GameData, t: &FxThing) -> String {
    let m = t.subtype.unwrap_or(-1);
    let find = |list: Vec<(usize, String)>| {
        list.into_iter()
            .find(|(i, _)| *i as i64 == m)
            .map(|(_, n)| n)
    };
    let n = match thing_class(t) {
        Some(class::CREATURE) => (m >= 1)
            .then(|| gd.creatures.get(m as usize - 1).cloned())
            .flatten(),
        Some(class::OBJECT) => find(gd.object_names()),
        Some(class::TRAP) => find(gd.trap_names()),
        Some(class::DOOR) => find(gd.door_names()),
        Some(class::EFFECT_GEN) => find(gd.effect_generator_names()),
        _ => None,
    };
    n.or_else(|| t.subtype_name.clone())
        .unwrap_or_else(|| format!("#{m}"))
}

impl EditorApp {
    // ------------------------------------------------------------------------------- picking

    /// Items under viewport pixel `p`, nearest first.
    pub(super) fn items_at_pixel(&self, p: Vec2) -> Vec<ItemRef> {
        let mut v: Vec<&HitBox> = self
            .hitboxes
            .iter()
            .filter(|h| p.x >= h.min.x && p.x <= h.max.x && p.y >= h.min.y && p.y <= h.max.y)
            .collect();
        v.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        let mut out: Vec<ItemRef> = vec![];
        for h in v {
            if !out.contains(&h.item) {
                out.push(h.item);
            }
        }
        out
    }

    pub(super) fn item_visible(&self, it: ItemRef) -> bool {
        let Some(doc) = &self.doc else { return false };
        let l = &self.settings.layout.layers;
        match it {
            ItemRef::Thing(i) => doc
                .doc
                .things
                .items
                .get(i)
                .is_some_and(|t| self.thing_visible(t)),
            ItemRef::ActionPoint(i) => l.action_points && i < doc.doc.action_points.items.len(),
            ItemRef::Light(i) => {
                l.lights
                    && doc
                        .doc
                        .lights
                        .items
                        .get(i)
                        .is_some_and(|x| x.parent_tile.is_none() || l.slab_things)
            }
        }
    }

    /// Select by clicking: `hits` nearest first. Shift toggles; repeated clicks on the same
    /// stack cycle through it.
    pub(super) fn click_select(&mut self, hits: Vec<ItemRef>, shift: bool, at: Vec2) {
        if hits.is_empty() {
            if !shift {
                self.selected.clear();
            }
            self.last_click = None;
            return;
        }
        let idx = match &self.last_click {
            Some((p, k, prev)) if (*p - at).length() < 4.0 && *prev == hits => (k + 1) % hits.len(),
            _ => 0,
        };
        self.last_click = Some((at, idx, hits.clone()));
        let it = hits[idx];
        if shift {
            if !self.selected.remove(&it) {
                self.selected.insert(it);
            }
        } else {
            self.selected = BTreeSet::from([it]);
        }
        self.status = trf(
            "kfx_editor.selected_it_idx_of_hits_here",
            "Selected {it} ({idx} of {hits} here)",
            &[
                ("it", &self.item_label(it)),
                ("idx", &(idx + 1)),
                ("hits", &hits.len()),
            ],
        );
    }

    /// Select everything inside the subtile rectangle (inclusive).
    pub fn select_items_in(&mut self, a: (i32, i32), b: (i32, i32), add: bool) {
        let Some(doc) = &self.doc else { return };
        let ((x0, y0), (x1, y1)) = ordered(a, b);
        let items: Vec<ItemRef> = doc
            .doc
            .items_in(kthings::StlRect { x0, y0, x1, y1 })
            .into_iter()
            .filter(|it| self.item_visible(*it))
            .collect();
        if !add {
            self.selected.clear();
        }
        self.selected.extend(items);
        self.status = trf(
            "kfx_editor.selected_items_selected",
            "{selected} items selected",
            &[("selected", &self.selected.len())],
        );
    }

    /// Select the items whose anchors project into the screen rectangle.
    pub(super) fn select_items_in_screen(&mut self, a: Vec2, b: Vec2, add: bool) {
        let (lo, hi) = (a.min(b), a.max(b));
        let mut items: Vec<ItemRef> = self
            .hitboxes
            .iter()
            .filter(|h| {
                let c = (h.min + h.max) * 0.5;
                let bottom = Vec2::new(c.x, h.max.y);
                bottom.x >= lo.x && bottom.x <= hi.x && bottom.y >= lo.y && bottom.y <= hi.y
            })
            .map(|h| h.item)
            .collect();
        items.dedup();
        if !add {
            self.selected.clear();
        }
        self.selected.extend(items);
        self.status = trf(
            "kfx_editor.selected_items_selected",
            "{selected} items selected",
            &[("selected", &self.selected.len())],
        );
    }

    pub fn item_label(&self, it: ItemRef) -> String {
        let Some(doc) = &self.doc else {
            return String::new();
        };
        match it {
            ItemRef::Thing(i) => doc
                .doc
                .things
                .items
                .get(i)
                .map(|t| format!("{} {}", class_name(thing_class(t)), model_name(&doc.gd, t)))
                .unwrap_or_default(),
            ItemRef::ActionPoint(i) => trf(
                "kfx_editor.action_point_number",
                "Action point {number}",
                &[(
                    "number",
                    &doc.doc
                        .action_points
                        .items
                        .get(i)
                        .and_then(|a| a.number)
                        .unwrap_or(0),
                )],
            ),
            ItemRef::Light(i) => trf("kfx_editor.light_i", "Light #{i}", &[("i", &i)]),
        }
    }

    /// Drop selected items that no longer exist.
    pub(super) fn prune_selection(&mut self) {
        let Some(doc) = &self.doc else {
            self.selected.clear();
            return;
        };
        let (nt, na, nl) = (
            doc.doc.things.items.len(),
            doc.doc.action_points.items.len(),
            doc.doc.lights.items.len(),
        );
        self.selected.retain(|it| match *it {
            ItemRef::Thing(i) => i < nt,
            ItemRef::ActionPoint(i) => i < na,
            ItemRef::Light(i) => i < nl,
        });
    }

    // ------------------------------------------------------------------------------- editing

    /// Choose what the Place tool puts down.
    pub fn select_thing_palette(&mut self, s: &str) -> Result<(), String> {
        let gd = self.doc.as_ref().map(|d| d.gd.clone()).ok_or("no map")?;
        let k = PlaceKind::parse(s, &gd).ok_or_else(|| format!("no thing named {s:?}"))?;
        self.category = match k {
            PlaceKind::Creature(_) => Category::Creatures,
            PlaceKind::Object(_) => Category::Objects,
            PlaceKind::Trap(_) => Category::Traps,
            PlaceKind::Door(_) => Category::Doors,
            PlaceKind::EffectGen(_) => Category::Effects,
            PlaceKind::Light => Category::Lights,
            PlaceKind::ActionPoint => Category::ActionPoints,
        };
        self.place = Some(k);
        self.set_tool(Tool::Place);
        Ok(())
    }

    /// The thing a placement makes at a raw position (z from the floor).
    fn new_item_thing(
        &self,
        k: &PlaceKind,
        u: i64,
        v: i64,
        owner: u8,
        orient: i64,
    ) -> Option<FxThing> {
        let doc = self.doc.as_ref()?;
        let cls = k.class()?;
        let (x, y) = (coord(u), coord(v));
        let mut t = kthings::new_thing(cls, k.model(), owner, x, y, doc.doc.floor_z(x.stl, y.stl));
        let d = &self.place_defaults;
        match cls {
            class::CREATURE => {
                t.creature_level = Some(d.level.clamp(1, 10));
                if d.creature_gold > 0 {
                    t.creature_gold = Some(d.creature_gold);
                }
                if d.health != 100 {
                    t.creature_initial_health = Some(d.health);
                }
                if !d.name.is_empty() {
                    t.creature_name = Some(d.name.clone());
                }
                t.orientation = Some(orient);
            }
            class::OBJECT => {
                if d.gold_value > 0 {
                    t.gold_value = Some(d.gold_value);
                }
                if d.custom_box > 0 && kthings::object_genre_is(&doc.gd, k.model(), "SPECIALBOX") {
                    t.custom_box = Some(d.custom_box);
                }
                if orient != 0 {
                    t.orientation = Some(orient);
                }
            }
            class::EFFECT_GEN => {
                t.effect_range = Some(coord((d.effect_range * 256.0) as i64));
                if orient != 0 {
                    t.orientation = Some(orient);
                }
            }
            class::TRAP => {
                if orient != 0 {
                    t.orientation = Some(orient);
                }
            }
            _ => {}
        }
        Some(t)
    }

    /// Place the current palette thing at subtile `stl` (sub-position `sub` when free placement
    /// is on, else the subtile centre), with its symmetric copies. Returns the new items.
    pub fn place_thing_at(
        &mut self,
        stl: (i32, i32),
        sub: (i32, i32),
    ) -> Result<BTreeSet<ItemRef>, String> {
        let k = self.place.clone().ok_or(trd(
            "kfx_editor.choose_a_thing_on_the_palette_first",
            "choose a thing on the palette first",
        ))?;
        let doc = self.doc.as_ref().ok_or("no map")?;
        let dims = doc.dims();
        if stl.0 < 0 || stl.1 < 0 || stl.0 >= dims.stl_w() as i32 || stl.1 >= dims.stl_h() as i32 {
            return Err(trd("kfx_editor.outside_the_map", "outside the map"));
        }
        let gd = doc.gd.clone();
        if let PlaceKind::Door(_) = k {
            let (bx, by) = (stl.0 / 3, stl.1 / 3);
            if !crate::document::door_can_stand(&doc.doc, &gd, bx, by) {
                return Err(trf(
                    "kfx_editor.a_door_on_slab_bx_by_needs_walls_on_two_opposite",
                    // door_can_stand, src/thing_doors.c:379
                    "a door on slab ({bx},{by}) needs walls on two opposite sides, \
                    else the game destroys it",
                    &[("bx", &bx), ("by", &by)],
                ));
            }
        }
        if !self.settings.place_anywhere
            && !matches!(
                k,
                PlaceKind::ActionPoint | PlaceKind::Light | PlaceKind::Door(_)
            )
        {
            // things are refused on tall columns unless "Place things anywhere" is on
            let h = doc.doc.floor_height(stl.0, stl.1);
            if h >= 5 {
                return Err(trf(
                    "kfx_editor.subtile_stl_stl2_is_inside_a_wall_or_rock_column",
                    "subtile ({stl},{stl2}) is inside a wall or rock (column {h} \
                    cubes tall): turn on Map > Place things anywhere to place there",
                    &[("stl", &stl.0), ("stl2", &stl.1), ("h", &h)],
                ));
            }
        }
        let sub = if self.free_place {
            (sub.0.clamp(0, 255), sub.1.clamp(0, 255))
        } else {
            (128, 128)
        };
        let (u, v) = (
            stl.0 as i64 * 256 + sub.0 as i64,
            stl.1 as i64 * 256 + sub.1 as i64,
        );
        let images: Vec<(i64, i64, usize, Option<Transform>)> = match &self.symmetry {
            Some(s) => s.point_images(dims, u, v),
            None => vec![(u, v, 0, None)],
        };
        let from = images[0].2;
        let name = trf(
            "kfx_editor.place_gd",
            "Place {key}",
            &[("key", &k.label(&gd))],
        );
        let before = ItemsBefore::of(&self.doc.as_ref().unwrap().doc);
        let d0 = self.place_defaults.clone();
        let base_owner = self.active_owner;
        let mut plan = vec![];
        for (iu, iv, sector, t) in images {
            let owner = self
                .symmetry
                .as_ref()
                .map(|s| s.map_owner(from, sector, base_owner))
                .unwrap_or(base_owner);
            let orient = t.map(|t| t.angle(d0.orientation)).unwrap_or(d0.orientation);
            plan.push((
                iu,
                iv,
                owner,
                orient,
                self.new_item_thing(&k, iu, iv, owner, orient),
            ));
        }
        let doc = self.doc.as_mut().unwrap();
        doc.begin_stroke(&name);
        let mut rf = Refresh::default();
        for (iu, iv, owner, _orient, t) in plan {
            let (x, y) = (coord(iu), coord(iv));
            let r = match &k {
                PlaceKind::Door(m) => {
                    let (sx, sy) = ((x.stl / 3) as usize, (y.stl / 3) as usize);
                    let m = *m;
                    let r = doc
                        .op(&name, |d, g| {
                            d.place_door(g, sx, sy, m, owner, d0.locked)
                                .map(|(_, p)| ((), p))
                                .unwrap_or(((), kfx_map::patch::Patch::new()))
                        })
                        .1;
                    // `place_door` changes the slab without re-placing the slab objects around it
                    // (torches on the walls next to the doorway); the game would (src/map_blocks.c:590)
                    let area = SlabRect::single(sx as i32, sy as i32).grow(1);
                    let dec = doc.decorator.clone();
                    let r2 = doc
                        .edit(&name, |d, g, _| {
                            d.apply_slab_objects(g, area);
                            d.decorate(&dec, area);
                        })
                        .1;
                    r.merge(r2)
                }
                PlaceKind::ActionPoint => {
                    doc.op(&name, |d, _| {
                        d.add_action_point(x, y, coord((d0.ap_range * 256.0) as i64), None)
                    })
                    .1
                }
                PlaceKind::Light => {
                    let h = doc.doc.floor_height(x.stl, y.stl) as i64 * 256
                        + (d0.light_height * 256.0) as i64;
                    doc.op(&name, |d, _| {
                        d.add_light(
                            x,
                            y,
                            coord(h),
                            coord((d0.light_range * 256.0) as i64),
                            d0.light_intensity,
                        )
                    })
                    .1
                }
                _ => match t {
                    Some(t) => doc.op(&name, |d, g| d.add_thing(g, t)).1,
                    None => Refresh::default(),
                },
            };
            rf = rf.merge(r);
        }
        doc.end_stroke();
        let new = before.changed(&self.doc.as_ref().unwrap().doc);
        self.after_change(rf);
        self.status = trn(
            "kfx_editor.name_new_item_s",
            new.len() as i64,
            "{name}: {n} item",
            "{name}: {n} items",
            &[("name", &name)],
        );
        Ok(new)
    }

    /// Delete the selected items.
    pub fn delete_things(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        let items: Vec<ItemRef> = self.selected.iter().copied().collect();
        let n = items.len();
        let Some(doc) = self.doc.as_mut() else { return };
        doc.end_stroke();
        let (_, rf) = doc.op(
            &trf(
                "kfx_editor.delete_n_items",
                "Delete {n} items",
                &[("n", &n)],
            ),
            |d, g| ((), d.remove_items(g, &items)),
        );
        self.selected.clear();
        self.after_change(rf);
        self.status = trf(
            "kfx_editor.deleted_n_items",
            "Deleted {n} items",
            &[("n", &n)],
        );
    }

    /// Move the selected items by `(du, dv)` raw units (1/256 subtile).
    pub fn move_things(&mut self, du: i64, dv: i64) {
        if self.selected.is_empty() || (du == 0 && dv == 0) {
            return;
        }
        let items: Vec<ItemRef> = self.selected.iter().copied().collect();
        let Some(doc) = self.doc.as_mut() else { return };
        doc.end_stroke();
        let dims = doc.dims();
        let (wr, hr) = (dims.stl_w() as i64 * 256 - 1, dims.stl_h() as i64 * 256 - 1);
        let before = ItemsBefore::of(&doc.doc);
        let things: Vec<FxThing> = items
            .iter()
            .filter_map(|it| {
                if let ItemRef::Thing(i) = it {
                    doc.doc.things.items.get(*i).cloned()
                } else {
                    None
                }
            })
            .collect();
        let aps: Vec<usize> = items
            .iter()
            .filter_map(|it| {
                if let ItemRef::ActionPoint(i) = it {
                    Some(*i)
                } else {
                    None
                }
            })
            .collect();
        let lights: Vec<usize> = items
            .iter()
            .filter_map(|it| {
                if let ItemRef::Light(i) = it {
                    Some(*i)
                } else {
                    None
                }
            })
            .collect();
        let n = items.len();
        let (_, rf) = doc.edit(
            &trf("kfx_editor.move_n_items", "Move {n} items", &[("n", &n)]),
            |d, g, _| {
                for t in &things {
                    let Some(i) = d.things.items.iter().position(|x| x == t) else {
                        continue;
                    };
                    let (Some(u), Some(v)) = (raw(t.x), raw(t.y)) else {
                        continue;
                    };
                    let _ = d.move_thing(
                        g,
                        i,
                        coord((u + du).clamp(0, wr)),
                        coord((v + dv).clamp(0, hr)),
                        None,
                    );
                }
                for &i in &aps {
                    let a = &d.action_points.items[i];
                    let (Some(u), Some(v)) = (raw(a.x), raw(a.y)) else {
                        continue;
                    };
                    let _ = d.move_action_point(
                        i,
                        coord((u + du).clamp(0, wr)),
                        coord((v + dv).clamp(0, hr)),
                    );
                }
                for &i in &lights {
                    let l = &d.lights.items[i];
                    let (Some(u), Some(v)) = (raw(l.x), raw(l.y)) else {
                        continue;
                    };
                    // lights keep their height above the floor
                    let above =
                        l.z.map(|z| {
                            z.raw() as i64
                                - ((d.floor_height(l.x.unwrap().stl, l.y.unwrap().stl) as i64) << 8)
                        })
                        .unwrap_or(512)
                        .max(0);
                    let (nx, ny) = (coord((u + du).clamp(0, wr)), coord((v + dv).clamp(0, hr)));
                    let z = coord(((d.floor_height(nx.stl, ny.stl) as i64) << 8) + above);
                    let _ = d.move_light(i, nx, ny, Some(z));
                }
            },
        );
        let doc = self.doc.as_ref().unwrap();
        let mut sel = before.changed(&doc.doc);
        // items that did not change (e.g. clamped at the edge) stay selected too
        for it in items {
            if let ItemRef::ActionPoint(_) | ItemRef::Light(_) = it {
                sel.insert(it);
            }
        }
        self.selected = sel;
        self.prune_selection();
        self.after_change(rf);
        self.status = trf("kfx_editor.moved_n_items", "Moved {n} items", &[("n", &n)]);
    }

    /// Turn the selected things' facing by `steps` eighths of a turn (clockwise).
    pub fn rotate_things(&mut self, steps: i64) {
        let Some(doc) = self.doc.as_mut() else { return };
        let idx: Vec<usize> = self
            .selected
            .iter()
            .filter_map(|it| {
                if let ItemRef::Thing(i) = it {
                    Some(*i)
                } else {
                    None
                }
            })
            .collect();
        if idx.is_empty() {
            return;
        }
        doc.end_stroke();
        doc.begin_stroke(&trd("kfx_editor.rotate_things", "Rotate things"));
        let mut rf = Refresh::default();
        for i in idx {
            let Some(mut t) = doc.doc.things.items.get(i).cloned() else {
                continue;
            };
            if thing_class(&t) == Some(class::DOOR) {
                continue;
            }
            t.orientation = Some((t.orientation.unwrap_or(0) + steps * 256).rem_euclid(2048));
            rf = rf.merge(
                doc.op(&trd("kfx_editor.rotate_things", "Rotate things"), |d, g| {
                    ((), d.update_thing(g, i, t))
                })
                .1,
            );
        }
        doc.end_stroke();
        self.after_change(rf);
    }

    /// Copies of the selected things / action points / lights one subtile to the south-east.
    pub fn duplicate_things(&mut self) {
        let Some(doc) = self.doc.as_mut() else { return };
        if self.selected.is_empty() {
            return;
        }
        doc.end_stroke();
        let gd = doc.gd.clone();
        let before = ItemsBefore::of(&doc.doc);
        let dims = doc.dims();
        let shift = |c: Option<StlCoord>, hi: usize| {
            c.map(|c| StlCoord::new((c.stl + 1).min(hi as i32 - 1), c.sub))
        };
        let mut things = vec![];
        let mut aps = vec![];
        let mut lights = vec![];
        for it in &self.selected {
            match *it {
                ItemRef::Thing(i) => {
                    let Some(t) = doc.doc.things.items.get(i) else {
                        continue;
                    };
                    if thing_class(t) == Some(class::DOOR) {
                        continue;
                    }
                    let mut t = t.clone();
                    t.x = shift(t.x, dims.stl_w());
                    t.y = shift(t.y, dims.stl_h());
                    t.parent_tile = None;
                    if kthings::is_hero_gate(&gd, &t) {
                        t.herogate_number = None;
                    }
                    if let (Some(x), Some(y)) = (t.x, t.y) {
                        let above = t.z.map(|z| z.stl).unwrap_or(0)
                            - doc.doc.floor_height(x.stl - 1, y.stl - 1) as i32;
                        t.z = Some(StlCoord::new(
                            doc.doc.floor_height(x.stl, y.stl) as i32 + above.max(0),
                            t.z.map_or(0, |z| z.sub),
                        ));
                    }
                    things.push(t);
                }
                ItemRef::ActionPoint(i) => {
                    if let Some(a) = doc.doc.action_points.items.get(i) {
                        aps.push((
                            shift(a.x, dims.stl_w()),
                            shift(a.y, dims.stl_h()),
                            a.range.unwrap_or_default(),
                        ));
                    }
                }
                ItemRef::Light(i) => {
                    if let Some(l) = doc.doc.lights.items.get(i) {
                        let mut l = l.clone();
                        l.x = shift(l.x, dims.stl_w());
                        l.y = shift(l.y, dims.stl_h());
                        l.parent_tile = None;
                        lights.push(l);
                    }
                }
            }
        }
        let n = things.len() + aps.len() + lights.len();
        let (_, rf) = doc.edit(
            &trf(
                "kfx_editor.duplicate_n_items",
                "Duplicate {n} items",
                &[("n", &n)],
            ),
            |d, g, _| {
                for t in things {
                    let _ = d.add_thing(g, t);
                }
                for (x, y, r) in aps {
                    if let (Some(x), Some(y)) = (x, y) {
                        let _ = d.add_action_point(x, y, r, None);
                    }
                }
                d.lights.items.extend(lights);
            },
        );
        self.selected = before.changed(&self.doc.as_ref().unwrap().doc);
        self.after_change(rf);
        self.status = trf(
            "kfx_editor.duplicated_n_items",
            "Duplicated {n} items",
            &[("n", &n)],
        );
    }

    /// Alt+click with a thing tool: copy the thing's kind and settings into the placement
    /// defaults.
    pub(super) fn pick_thing(&mut self, it: ItemRef) {
        let Some(doc) = &self.doc else { return };
        let d = &mut self.place_defaults;
        match it {
            ItemRef::Thing(i) => {
                let Some(t) = doc.doc.things.items.get(i) else {
                    return;
                };
                let m = t.subtype.unwrap_or(0);
                self.place = match thing_class(t) {
                    Some(class::CREATURE) => Some(PlaceKind::Creature(m)),
                    Some(class::OBJECT) => Some(PlaceKind::Object(m)),
                    Some(class::TRAP) => Some(PlaceKind::Trap(m)),
                    Some(class::DOOR) => Some(PlaceKind::Door(m)),
                    Some(class::EFFECT_GEN) => Some(PlaceKind::EffectGen(m)),
                    _ => None,
                };
                d.level = t.creature_level.unwrap_or(d.level);
                d.orientation = t.orientation.unwrap_or(0);
                d.creature_gold = t.creature_gold.unwrap_or(0);
                d.health = t.creature_initial_health.unwrap_or(100);
                d.name = t.creature_name.clone().unwrap_or_default();
                d.gold_value = t.gold_value.unwrap_or(0);
                d.custom_box = t.custom_box.unwrap_or(0);
                d.locked = t.door_locked.unwrap_or(0) > 0;
                if let Some(r) = t.effect_range {
                    d.effect_range = r.stl as f32 + r.sub as f32 / 256.0;
                }
                if let Some(o) = t.owner {
                    self.active_owner = o.clamp(0, 8) as u8;
                }
            }
            ItemRef::ActionPoint(i) => {
                self.place = Some(PlaceKind::ActionPoint);
                if let Some(r) = doc.doc.action_points.items.get(i).and_then(|a| a.range) {
                    d.ap_range = r.stl as f32 + r.sub as f32 / 256.0;
                }
            }
            ItemRef::Light(i) => {
                self.place = Some(PlaceKind::Light);
                if let Some(l) = doc.doc.lights.items.get(i) {
                    d.light_range = l
                        .range
                        .map(|r| r.stl as f32 + r.sub as f32 / 256.0)
                        .unwrap_or(d.light_range);
                    d.light_intensity = l.intensity.unwrap_or(d.light_intensity);
                }
            }
        }
        let label = self
            .place
            .as_ref()
            .map(|k| k.label(&doc.gd))
            .unwrap_or_default();
        self.status = trf(
            "kfx_editor.picked_label_into_the_placement_settings",
            "Picked {label} into the placement settings",
            &[("label", &label)],
        );
    }

    /// Set an inspector field on every selected item it applies to (one undo step).
    pub fn set_selection_field(&mut self, field: &str, value: &str) -> Result<usize, String> {
        if field == "model" {
            return self.set_selection_model(value);
        }
        if !FIELDS.iter().any(|(f, _)| *f == field) {
            return Err(trf(
                "kfx_editor.unknown_field_field",
                "unknown field {field}",
                &[("field", &format!("{:?}", field))],
            ));
        }
        let items: Vec<ItemRef> = self.selected.iter().copied().collect();
        let doc = self.doc.as_mut().ok_or("no map")?;
        let dims = doc.dims();
        let before = ItemsBefore::of(&doc.doc);
        let mut things = vec![];
        let mut aps = vec![];
        let mut lights = vec![];
        for it in &items {
            match *it {
                ItemRef::Thing(i) => {
                    let Some(t) = doc.doc.things.items.get(i) else {
                        continue;
                    };
                    let mut n = t.clone();
                    if set_thing_field(&mut n, field, value, dims)? && n != *t {
                        things.push((t.clone(), n));
                    }
                }
                ItemRef::ActionPoint(i) => {
                    let Some(a) = doc.doc.action_points.items.get(i) else {
                        continue;
                    };
                    let mut n = a.clone();
                    if set_ap_field(&mut n, field, value, dims)? && n != *a {
                        aps.push((i, n));
                    }
                }
                ItemRef::Light(i) => {
                    let Some(l) = doc.doc.lights.items.get(i) else {
                        continue;
                    };
                    let mut n = l.clone();
                    if set_light_field(&mut n, field, value, dims)? && n != *l {
                        lights.push((i, n));
                    }
                }
            }
        }
        let n = things.len() + aps.len() + lights.len();
        if n == 0 {
            return Ok(0);
        }
        doc.end_stroke();
        let (_, rf) = doc.edit(
            &trf(
                "kfx_editor.set_field_on_n_items",
                "Set {field} on {n} items",
                &[("field", &field), ("n", &n)],
            ),
            |d, g, _| {
                for (old, new) in things {
                    if let Some(i) = d.things.items.iter().position(|x| *x == old) {
                        let _ = d.update_thing(g, i, new);
                    }
                }
                for (i, a) in aps {
                    let _ = d.update_action_point(i, a);
                }
                for (i, l) in lights {
                    let _ = d.update_light(i, l);
                }
            },
        );
        let doc = self.doc.as_ref().unwrap();
        let mut sel = before.changed(&doc.doc);
        for it in items {
            if !matches!(it, ItemRef::Thing(_)) {
                sel.insert(it);
            }
        }
        self.selected = sel;
        self.prune_selection();
        self.after_change(rf);
        Ok(n)
    }

    /// Change the model of the selected things (a name or a number of their class; doors are
    /// left alone). One undo step.
    pub fn set_selection_model(&mut self, value: &str) -> Result<usize, String> {
        let doc = self.doc.as_mut().ok_or("no map")?;
        let gd = doc.gd.clone();
        let items: Vec<usize> = self
            .selected
            .iter()
            .filter_map(|it| {
                if let ItemRef::Thing(i) = it {
                    Some(*i)
                } else {
                    None
                }
            })
            .collect();
        let mut changes = vec![];
        for i in items {
            let Some(t) = doc.doc.things.items.get(i) else {
                continue;
            };
            let prefix = match thing_class(t) {
                Some(class::CREATURE) => "creature",
                Some(class::OBJECT) => "object",
                Some(class::TRAP) => "trap",
                Some(class::EFFECT_GEN) => "effect",
                _ => continue,
            };
            let k = PlaceKind::parse(&format!("{prefix}:{}", value.trim()), &gd)
                .ok_or_else(|| format!("no {prefix} {value:?}"))?;
            let mut n = t.clone();
            n.subtype = Some(k.model());
            if n.subtype_name.is_some() {
                n.subtype_name = Some(k.label(&gd));
            }
            if n != *t {
                changes.push((t.clone(), n));
            }
        }
        let n = changes.len();
        if n == 0 {
            return Ok(0);
        }
        doc.end_stroke();
        let before = ItemsBefore::of(&doc.doc);
        let (_, rf) = doc.edit(
            &trf(
                "kfx_editor.set_model_on_n_things",
                "Set model on {n} things",
                &[("n", &n)],
            ),
            |d, g, _| {
                for (old, new) in changes {
                    if let Some(i) = d.things.items.iter().position(|x| *x == old) {
                        let _ = d.update_thing(g, i, new);
                    }
                }
            },
        );
        self.selected = before.changed(&self.doc.as_ref().unwrap().doc);
        self.prune_selection();
        self.after_change(rf);
        Ok(n)
    }

    /// Centre the camera on an item.
    pub fn focus_item(&mut self, it: ItemRef) {
        let Some(doc) = &self.doc else { return };
        let p = match it {
            ItemRef::Thing(i) => doc
                .doc
                .things
                .items
                .get(i)
                .and_then(|t| world_of(t.x, t.y, t.z)),
            ItemRef::ActionPoint(i) => doc
                .doc
                .action_points
                .items
                .get(i)
                .and_then(|a| world_of(a.x, a.y, None)),
            ItemRef::Light(i) => doc
                .doc
                .lights
                .items
                .get(i)
                .and_then(|l| world_of(l.x, l.y, l.z)),
        };
        if let Some(p) = p {
            self.camera.target = Vec3::new(p.x, self.camera.target.y, p.z);
        }
    }

    /// Load the sprites of the open document (call after `set_document`).
    pub(super) fn load_sprites(&mut self) {
        let (Some(assets), Some(doc)) = (&self.assets, &self.doc) else {
            return;
        };
        let t = Instant::now();
        let lib = SpriteLib::load(
            &assets.install,
            &assets.palette,
            &doc.gd,
            &doc.layers,
            &doc.doc.folder,
            doc.doc.level,
        );
        self.perf.sprites_ms = t.elapsed().as_secs_f32() * 1000.0;
        self.sprite_atlas.clear();
        if !lib.warnings.is_empty() {
            self.status = trf(
                "kfx_editor.status_sprites_warnings",
                "{status} (sprites: {warnings})",
                &[
                    ("status", &self.status),
                    ("warnings", &lib.warnings.join("; ")),
                ],
            );
        }
        self.sprite_zips = lib.zips.len();
        self.sprites = Some(lib);
    }

    /// Ground point under a viewport pixel (floor of the picked subtile).
    pub(super) fn ground_at(&self, px: Vec2, vp: Vec2) -> Option<Vec3> {
        let h = self.pick_at(px, vp)?;
        Some(h.hit.point)
    }

    /// Thing tools: button pressed at viewport pixel `px` over subtile hit `hit`.
    pub(super) fn things_press(&mut self, px: Vec2, vp: Vec2, shift: bool, alt: bool) {
        let hits = self.items_at_pixel(px);
        if alt {
            if let Some(&it) = hits.first() {
                self.pick_thing(it);
            }
            return;
        }
        match self.tool {
            Tool::Place => {
                let Some(h) = self.pick_at(px, vp) else {
                    return;
                };
                let sub = (
                    ((h.hit.point.x.fract()) * 256.0) as i32,
                    ((h.hit.point.z.fract()) * 256.0) as i32,
                );
                match self.place_thing_at((h.hit.stl_x as i32, h.hit.stl_y as i32), sub) {
                    Ok(new) => {
                        self.selected = new;
                        if !shift {
                            self.set_tool(Tool::Things);
                        }
                    }
                    Err(e) => self.status = e,
                }
            }
            _ => {
                let over_selected = hits.first().is_some_and(|h| self.selected.contains(h));
                if !hits.is_empty() && !shift {
                    if !over_selected {
                        self.click_select(hits, false, px);
                    } else {
                        // a second click on the same stack cycles
                        let cycled = matches!(
                            &self.last_click,
                            Some((p, _, prev)) if (*p - px).length() < 4.0 && *prev == hits
                        );
                        if cycled && hits.len() > 1 {
                            self.click_select(hits, false, px);
                        } else {
                            self.last_click = Some((px, 0, hits.clone()));
                        }
                    }
                    if let Some(g) = self.ground_at(px, vp) {
                        self.thing_drag = Some(ThingDrag { start: g, cur: g });
                    }
                } else if !hits.is_empty() {
                    self.click_select(hits, true, px);
                } else {
                    self.rect_sel = Some(RectSel {
                        start: px,
                        cur: px,
                        add: shift,
                    });
                }
            }
        }
    }

    pub(super) fn things_drag(&mut self, px: Vec2, vp: Vec2) {
        if let Some(r) = self.rect_sel.as_mut() {
            r.cur = px;
        }
        if let Some(mut d) = self.thing_drag {
            if let Some(g) = self.camera.ray(px, vp).hit_plane_y(d.start.y) {
                d.cur = g;
                self.thing_drag = Some(d);
            }
        }
    }

    pub(super) fn things_release(&mut self) {
        if let Some(r) = self.rect_sel.take() {
            if (r.cur - r.start).length() > 3.0 {
                self.select_items_in_screen(r.start, r.cur, r.add);
            } else if !r.add {
                self.selected.clear();
            }
        }
        if let Some(d) = self.thing_drag.take() {
            let off = self.drag_offset(d);
            let (du, dv) = (
                (off.x * 256.0).round() as i64,
                (off.z * 256.0).round() as i64,
            );
            if du != 0 || dv != 0 {
                self.move_things(du, dv);
            }
        }
    }

    /// Symmetry on (`None` = off) with per-sector owners.
    pub fn set_symmetry(&mut self, s: Option<Symmetry>) {
        self.symmetry = s;
        self.store_symmetry();
    }
}

#[cfg(test)]
mod gold_tests {
    use super::GoldRules;

    /// `add_gold_to_pile` (src/thing_objects.c:2134-2158) with the stock rules.
    #[test]
    fn gold_pile_sizes() {
        let g = GoldRules::default();
        // a pile (model 43, typical 200): half the typical value -> 196*100/200+128 = 226
        assert_eq!(g.sprite_size(43, Some(100), 300), 226);
        assert_eq!(g.sprite_size(43, Some(200), 300), 324);
        assert_eq!(
            g.sprite_size(43, Some(1200), 300),
            196 + 24 * 1000 / 200 + 128
        );
        assert_eq!(g.sprite_size(43, Some(100_000), 300), 640);
        // no value: the configured size stays
        assert_eq!(g.sprite_size(43, None, 300), 300);
        // a valuable without a typical value keeps its size
        assert_eq!(g.sprite_size(5, Some(500), 300), 300);
    }
}
