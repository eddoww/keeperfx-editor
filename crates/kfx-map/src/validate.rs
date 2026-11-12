//! Map checks against the game's limits and load rules.
//!
//! Every check cites where the game enforces or depends on it. Severity: `Error` = the game
//! drops or refuses something, or the map will not play as drawn; `Warning` = legal but very
//! likely a mistake; `Info` = worth knowing.
//!
//! Not checked (no rule in the game to check against, or needs data not loaded here): the
//! per-player creature limit (set by the level script's `MAX_CREATURES`), room sizes (the game has
//! no minimum room size; creature attraction needs per-creature `EntranceRoom` sizes from the
//! creature configs, `src/room_entrance.c:229`). Reachability is a heuristic (`thing-unreachable`, see
//! [`unreachable_things`]).
//!
//! Codes added after the first version: `file-unreadable` (Error: a thing / light / action
//! point file or the sidecar exists but could not be read, `MapDoc::unreadable`),
//! `thing-class-not-loadable` and `thing-subtype-name` (Error: the FX loader fails the whole level
//! on them, [`fx_thing_rejection`]), `door-cannot-stand` (Error, `door_can_stand`
//! src/thing_doors.c:379, used by `process_door` src/thing_doors.c:575), `gold-in-treasury` (Info,
//! [`loose_gold_in_treasury`]), `thing-unreachable` (Warning, objects / traps) and
//! `creature-unreachable` (Info).
use crate::autotile::{self, PLAYER_NEUTRAL};
use crate::column::ColumnPool;
use crate::gamedata::{slab_flags, GameData};
use crate::mapdoc::MapDoc;
use crate::things::{
    self, class, column_floor_height, door_of_slab_kind, door_slab_kinds, thing_class, ItemRef,
    COLUMN_WALL_HEIGHT,
};
use std::collections::{HashMap, VecDeque};

/// `THINGS_COUNT - 2` things are read from a map (`SYNCED_THINGS_COUNT + UNSYNCED_THINGS_COUNT`
/// src/thing_list.h:35-37, src/lvl_filesdk1.c:756/867).
pub const MAX_MAP_THINGS: usize = 8192 + 4096 - 2;
/// `CREATURES_COUNT` (src/thing_data.h:26); `create_creature` refuses more (src/thing_creature.c:4917).
pub const MAX_CREATURES: usize = 1024;
/// `ACTN_POINTS_COUNT - 1` (src/actionpt.h:29, src/lvl_filesdk1.c:886/912).
pub const MAX_ACTION_POINTS: usize = 255;
/// `LIGHTS_COUNT - 1` (src/light_data.h:26, src/lvl_filesdk1.c:1263/1297).
pub const MAX_LIGHTS: usize = 2047;
/// Hero gate numbers 1..`HERO_GATES_COUNT - 1` (src/actionpt.h:30, src/thing_list.c:2361).
pub const MAX_HERO_GATE: i64 = 255;
/// Owners 0..`PLAYERS_COUNT - 1` (src/player_data.h:34); the FX loader discards `owner >
/// PLAYERS_COUNT` (src/thing_factory.c:254) and the legacy one `owner > 5` (src/thing_factory.c:135).
pub const PLAYERS_COUNT: u8 = 9;
/// The hero player (`game.hero_player_num`, white): no dungeon heart needed.
pub const PLAYER_GOOD: u8 = 4;

/// The name level scripts use for an owner number (`player_desc`,
/// src/lvl_script_commands.c:77): owners 0-3 are `PLAYER0`-`PLAYER3`, 4 is `PLAYER_GOOD`,
/// 5 is `PLAYER_NEUTRAL` and 6-8 are `PLAYER4`-`PLAYER6`.
pub fn owner_script_name(owner: u8) -> &'static str {
    match owner {
        0 => "PLAYER0",
        1 => "PLAYER1",
        2 => "PLAYER2",
        3 => "PLAYER3",
        4 => "PLAYER_GOOD",
        5 => "PLAYER_NEUTRAL",
        6 => "PLAYER4",
        7 => "PLAYER5",
        8 => "PLAYER6",
        _ => "an unknown player",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Location {
    Map,
    Slab(i32, i32),
    Subtile(i32, i32),
    Item(ItemRef),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// Stable short code, e.g. `things-limit`.
    pub code: &'static str,
    pub message: String,
    pub at: Location,
}

fn issue(
    v: &mut Vec<Issue>,
    severity: Severity,
    code: &'static str,
    at: Location,
    message: String,
) {
    v.push(Issue {
        severity,
        code,
        message,
        at,
    });
}

/// Whether object `model` is a dungeon heart (`OMF_Heart`: `HEART` in `Properties`,
/// src/thing_objects.c:379, src/config_objects.c:59).
pub fn is_heart_model(gd: &GameData, model: i64) -> bool {
    gd.objects_cfg
        .get(&format!("object{model}"), "Properties")
        .is_some_and(|p| {
            p.split_whitespace()
                .any(|w| w.eq_ignore_ascii_case("HEART"))
        })
}

/// Thing classes `thing_create_thing_adv` creates (src/thing_factory.c:259-412: Object, Creature,
/// EffectGen, Trap, Door, `TCls_unusedparam10/11`); any other makes it return failure.
pub const FX_LOADABLE_CLASSES: [i64; 7] = [1, 5, 7, 8, 9, 10, 11];

/// Why the game's FX loader refuses thing `t` (and with it the whole level:
/// `load_tngfx_file`'s failure is the level load result, src/lvl_filesdk1.c:1469), `None` when it
/// creates it as far as the file can tell (an unknown numeric model is checked by [`validate`], it
/// depends on the configs).
pub fn fx_thing_rejection(t: &kfx_formats::level_fx::FxThing, gd: &GameData) -> Option<String> {
    // value_parse_class (src/value_util.c:59): an int32 or one of the class names
    let Some(cls) = t
        .class
        .as_ref()
        .and_then(|c| c.id())
        .filter(|c| i32::try_from(*c).is_ok())
    else {
        // src/thing_factory.c:221
        return Some("no or an unknown ThingType".into());
    };
    if !FX_LOADABLE_CLASSES.contains(&cls) {
        let name = kfx_formats::level_fx::FxClass::Id(cls)
            .name()
            .map(|n| format!(" ({n})"))
            .unwrap_or_default();
        // src/thing_factory.c:412
        return Some(format!("class {cls}{name} cannot be created from a .tngfx"));
    }
    if let Some(v) = t.extra.get("SubtypeStringID") {
        // src/thing_factory.c:227
        return Some(format!(
            "SubtypeStringID is a {}, not a string",
            v.type_str()
        ));
    }
    if let Some(name) = &t.subtype_name {
        if subtype_by_name(gd, cls, name).is_none() {
            // src/thing_factory.c:233
            return Some(format!("unknown SubtypeStringID \"{name}\""));
        }
    } else if t.subtype.is_none() {
        // src/thing_factory.c:244
        return Some("no Subtype".into());
    }
    // src/thing_factory.c:248 and :254
    match t.owner {
        None => Some("no Ownership".into()),
        Some(o) if o > PLAYERS_COUNT as i64 => Some(format!("owner {o} above {PLAYERS_COUNT}")),
        _ => None,
    }
}

/// `value_parse_model` (src/value_util.c:93): the model of a config name for a class
/// (case-insensitive, `get_id` src/config.c:1214).
pub fn subtype_by_name(gd: &GameData, cls: i64, name: &str) -> Option<i64> {
    let find = |list: Vec<(usize, String)>| {
        list.into_iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(name))
            .map(|(m, _)| m as i64)
    };
    match cls {
        1 | 12 => find(gd.object_names()),
        4 | 5 => gd
            .creatures
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .map(|i| i as i64 + 1),
        7 => find(gd.effect_generator_names()),
        8 => find(gd.trap_names()),
        9 => find(gd.door_names()),
        _ => None,
    }
}

/// Run every check. Issues are sorted most severe first.
pub fn validate(doc: &MapDoc, gd: &GameData) -> Vec<Issue> {
    let mut v = vec![];
    let d = doc.dims;
    let things = &doc.things.items;

    // ---- files that could not be read (the document is read-only)
    for u in &doc.unreadable {
        issue(
            &mut v,
            Severity::Error,
            "file-unreadable",
            Location::Map,
            format!(
                "{} ({}) could not be read: {}; the map is read-only{}",
                u.path.display(),
                u.what,
                u.error,
                if doc.unreadable_discarded {
                    " until saved (the file will be replaced)"
                } else {
                    ""
                }
            ),
        );
    }
    // ---- counts
    if things.len() > MAX_MAP_THINGS {
        issue(
            &mut v,
            Severity::Error,
            "things-limit",
            Location::Map,
            format!(
                "{} things; the game loads at most {MAX_MAP_THINGS}",
                things.len()
            ),
        );
    }
    let creatures = things
        .iter()
        .filter(|t| thing_class(t) == Some(class::CREATURE))
        .count();
    if creatures >= MAX_CREATURES {
        issue(
            &mut v,
            Severity::Error,
            "creatures-limit",
            Location::Map,
            format!(
                "{creatures} creatures; the game creates at most {}",
                MAX_CREATURES - 1
            ),
        );
    }
    if doc.action_points.items.len() > MAX_ACTION_POINTS {
        issue(
            &mut v,
            Severity::Error,
            "action-points-limit",
            Location::Map,
            format!(
                "{} action points; the game loads at most {MAX_ACTION_POINTS}",
                doc.action_points.items.len()
            ),
        );
    }
    if doc.lights.items.len() > MAX_LIGHTS {
        issue(
            &mut v,
            Severity::Error,
            "lights-limit",
            Location::Map,
            format!(
                "{} lights; the game loads at most {MAX_LIGHTS}",
                doc.lights.items.len()
            ),
        );
    }
    // ---- column pool (map columns + the ones the game adds, src/lvl_filesdk1.c:1434-1438)
    if let Err(e) = ColumnPool::build(gd, d, &doc.columns) {
        issue(
            &mut v,
            Severity::Error,
            "column-pool",
            Location::Map,
            e.to_string(),
        );
    }

    // ---- slabs
    let slab_count = gd.slab_count();
    let mut slab_owners: HashMap<u8, usize> = HashMap::new();
    for y in 0..d.h as i32 {
        for x in 0..d.w as i32 {
            let i = d.slab_index(x as usize, y as usize);
            let k = doc.kinds[i] as usize;
            if k >= slab_count {
                issue(
                    &mut v,
                    Severity::Warning,
                    "slab-kind-unknown",
                    Location::Slab(x, y),
                    // src/lvl_filesdk1.c:1209
                    format!("slab kind {k} not in terrain.cfg; the game loads it as rock"),
                );
            }
            let o = doc.owners[i];
            if o >= PLAYERS_COUNT {
                issue(
                    &mut v,
                    Severity::Error,
                    "slab-owner-range",
                    Location::Slab(x, y),
                    format!("owner {o} out of range 0..8"),
                );
            } else if o != PLAYER_NEUTRAL {
                *slab_owners.entry(o).or_default() += 1;
            }
            match doc.custom_cells[i] {
                0 => {}
                id => match doc.sidecar.custom_slab(id) {
                    None => issue(
                        &mut v,
                        Severity::Warning,
                        "custom-slab-undefined",
                        Location::Slab(x, y),
                        format!(
                            "custom slab id {id} has no definition in the sidecar; the slab is \
                        regenerated like a normal one"
                        ),
                    ),
                    Some(c) if c.recognized_as as usize != k => issue(
                        &mut v,
                        Severity::Warning,
                        "custom-slab-kind",
                        Location::Slab(x, y),
                        format!(
                            "custom slab {id} is recognised as kind {} but the slab layer \
                        holds {k}",
                            c.recognized_as
                        ),
                    ),
                    _ => {}
                },
            }
            if gd.slab(k).block_flags & slab_flags::IS_DOOR != 0 && doc.door_on_slab(x, y).is_none()
            {
                issue(
                    &mut v,
                    Severity::Warning,
                    "door-slab-without-door",
                    Location::Slab(x, y),
                    // src/thing_doors.c:140
                    "door slab without a door thing (only a door thing makes a working door)"
                        .into(),
                );
            }
        }
    }

    // ---- things
    let objects = gd.object_names();
    let traps = gd.trap_names();
    let doors = gd.door_names();
    let effects = gd.effect_generator_names();
    let mut hearts: HashMap<u8, usize> = HashMap::new();
    let slabset_models: std::collections::HashSet<i64> = gd
        .slab_objects
        .iter()
        .flatten()
        .filter(|o| o.is_light == 0 && o.class_id == class::OBJECT)
        .map(|o| o.model as i64)
        .collect();
    let mut gates: HashMap<i64, usize> = HashMap::new();
    for (i, t) in things.iter().enumerate() {
        let at = Location::Item(ItemRef::Thing(i));
        let cls = thing_class(t);
        let model = t.subtype.unwrap_or(-1);
        let owner = t.owner.unwrap_or(-1);
        if t.class.is_none() || cls.is_none() {
            issue(
                &mut v,
                Severity::Error,
                "thing-class",
                at,
                // src/thing_factory.c:221
                "thing without a known ThingType: the game refuses to load the level".into(),
            );
            continue;
        }
        if let Some(c) = cls.filter(|c| !FX_LOADABLE_CLASSES.contains(&(*c as i64))) {
            issue(
                &mut v,
                Severity::Error,
                "thing-class-not-loadable",
                at,
                // src/thing_factory.c:412
                format!(
                    "class {c} cannot be loaded from a .tngfx: the game refuses to load the \
                    level; the editor will not save it"
                ),
            );
            continue;
        }
        if let Some(why) = fx_thing_rejection(t, gd) {
            if t.extra.contains_key("SubtypeStringID") || t.subtype_name.is_some() {
                issue(
                    &mut v,
                    Severity::Error,
                    "thing-subtype-name",
                    at,
                    format!(
                        "{why}: the game refuses to load the level; the editor will not save it"
                    ),
                );
            }
        }
        if t.subtype.is_none() && t.subtype_name.is_none() {
            issue(
                &mut v,
                Severity::Error,
                "thing-model",
                at,
                // src/thing_factory.c:244
                "thing without Subtype: the game refuses to load the level".into(),
            );
        }
        if owner == PLAYERS_COUNT as i64 {
            issue(
                &mut v,
                Severity::Warning,
                "thing-owner-range",
                at,
                // the FX loader rejects only `owner > PLAYERS_COUNT` (src/thing_factory.c:254)
                format!(
                    "owner {owner} is not a player; the KeeperFX format accepts it, the \
                    legacy format cannot hold it"
                ),
            );
        } else if !(0..PLAYERS_COUNT as i64).contains(&owner) {
            issue(
                &mut v,
                Severity::Error,
                "thing-owner-range",
                at,
                // src/thing_factory.c:248-256
                format!("owner {owner} out of range 0..{PLAYERS_COUNT}"),
            );
        }
        let known = match cls {
            Some(class::CREATURE) => model >= 1 && (model as usize) <= gd.creatures.len(),
            Some(class::OBJECT) => objects.iter().any(|(m, _)| *m as i64 == model),
            Some(class::TRAP) => model > 0 && traps.iter().any(|(m, _)| *m as i64 == model),
            Some(class::DOOR) => model > 0 && doors.iter().any(|(m, _)| *m as i64 == model),
            Some(class::EFFECT_GEN) => effects.iter().any(|(m, _)| *m as i64 == model),
            _ => true,
        };
        if t.subtype.is_some() && !known {
            issue(
                &mut v,
                Severity::Error,
                "thing-model-unknown",
                at,
                format!(
                    "model {model} of class {} not in the loaded configs",
                    cls.unwrap_or(0)
                ),
            );
        }
        let Some((sx, sy)) = things::position(t.x, t.y) else {
            issue(
                &mut v,
                Severity::Error,
                "thing-position",
                at,
                "thing without SubtileX/SubtileY".into(),
            );
            continue;
        };
        if sx < 0 || sy < 0 || sx >= d.stl_w() as i32 || sy >= d.stl_h() as i32 {
            issue(
                &mut v,
                Severity::Error,
                "thing-outside",
                at,
                format!("subtile ({sx},{sy}) outside the map"),
            );
            continue;
        }
        let (bx, by) = (sx / 3, sy / 3);
        let kind = doc.kinds[d.slab_index(bx as usize, by as usize)];
        match cls {
            Some(class::DOOR) => {
                let k = door_slab_kinds(gd, model);
                let orient = t.door_orientation.unwrap_or(0).clamp(0, 1) as usize;
                if gd.slab(kind as usize).block_flags & slab_flags::IS_DOOR == 0 {
                    issue(
                        &mut v,
                        Severity::Warning,
                        "door-not-on-door-slab",
                        at,
                        // src/thing_doors.c:140
                        format!(
                            "door on a {} slab; the game turns the slab into a door when it \
                        loads",
                            gd.slab_name(kind as usize).unwrap_or("?")
                        ),
                    );
                } else if k.is_some_and(|k| k[orient] != kind) {
                    issue(
                        &mut v,
                        Severity::Info,
                        "door-slab-orientation",
                        at,
                        "door slab kind \
                        does not match the door's model / orientation; the game re-places it from \
                        the thing"
                            .into(),
                    );
                }
                if !doc.door_can_stand(gd, bx, by) {
                    let w = doc.door_stand_walls(gd, bx, by).unwrap_or([false; 4]);
                    issue(
                        &mut v,
                        Severity::Error,
                        "door-cannot-stand",
                        at,
                        // door_can_stand, src/thing_doors.c:379; process_door, :575
                        format!(
                            "a door needs \
                        walls on both east and west, or on both north and south (not on all four \
                        sides); slab ({bx},{by}) has walls N={} E={} S={} W={}: the game destroys \
                        the door as soon as it runs",
                            w[0], w[1], w[2], w[3]
                        ),
                    );
                }
                if door_of_slab_kind(gd, kind).is_some() && (sx % 3 != 1 || sy % 3 != 1) {
                    issue(
                        &mut v,
                        Severity::Warning,
                        "door-off-centre",
                        at,
                        "door not on the centre subtile of its slab".into(),
                    );
                }
            }
            Some(class::OBJECT) => {
                if is_heart_model(gd, model) && (0..PLAYERS_COUNT as i64).contains(&owner) {
                    *hearts.entry(owner as u8).or_default() += 1;
                }
                if things::is_hero_gate(gd, t) {
                    match t.herogate_number {
                        Some(n) if (1..=MAX_HERO_GATE).contains(&n) => {
                            *gates.entry(n).or_default() += 1
                        }
                        n => issue(
                            &mut v,
                            Severity::Error,
                            "hero-gate-number",
                            at,
                            format!("hero gate number {n:?} outside 1..{MAX_HERO_GATE}"),
                        ),
                    }
                }
            }
            _ => {}
        }
        // buried things; slab objects (torches...) sit in walls and door keys inside the closed
        // door, so objects attached to their slab, models the slabset places, and keys are fine
        let attached = t
            .parent_tile
            .is_some_and(|p| p == autotile::slab_number(d, bx, by) as i64)
            || model == things::OBJ_SPINNING_KEY
            || slabset_models.contains(&model);
        if matches!(cls, Some(class::CREATURE | class::TRAP))
            || (cls == Some(class::OBJECT) && !attached)
        {
            let c = &doc.columns[d.stl_index(sx as usize, sy as usize)];
            if column_floor_height(c) >= COLUMN_WALL_HEIGHT {
                issue(
                    &mut v,
                    Severity::Warning,
                    "thing-in-solid",
                    at,
                    format!(
                        "class {} model {model} inside a solid column at subtile ({sx},{sy})",
                        cls.unwrap_or(0)
                    ),
                );
            }
        }
    }
    for (n, c) in &gates {
        if *c > 1 {
            issue(
                &mut v,
                Severity::Warning,
                "hero-gate-duplicate",
                Location::Map,
                format!("{c} hero gates numbered {n}"),
            );
        }
    }

    // ---- keepers and hearts
    for (&o, &n) in &slab_owners {
        if o == PLAYER_GOOD {
            continue;
        }
        match hearts.get(&o).copied().unwrap_or(0) {
            0 => issue(
                &mut v,
                Severity::Warning,
                "keeper-without-heart",
                Location::Map,
                format!(
                    "{} owns {n} slabs but has no dungeon heart",
                    owner_script_name(o)
                ),
            ),
            1 => {}
            h => issue(
                &mut v,
                Severity::Info,
                "keeper-several-hearts",
                Location::Map,
                // src/room_data.c:3534
                format!(
                    "{} has {h} dungeon hearts (the extra ones are backups)",
                    owner_script_name(o)
                ),
            ),
        }
    }

    // ---- action points / lights
    let mut nums: HashMap<i64, usize> = HashMap::new();
    for (i, a) in doc.action_points.items.iter().enumerate() {
        let at = Location::Item(ItemRef::ActionPoint(i));
        match a.number {
            Some(n) if n >= 0 => *nums.entry(n).or_default() += 1,
            n => issue(
                &mut v,
                Severity::Error,
                "action-point-number",
                at,
                // src/actionpt.c:84
                match n {
                    Some(n) => format!("action point number {n} is rejected"),
                    None => "action point without a number is rejected".to_string(),
                },
            ),
        }
        if let Some((x, y)) = things::position(a.x, a.y) {
            if x < 0 || y < 0 || x >= d.stl_w() as i32 || y >= d.stl_h() as i32 {
                issue(
                    &mut v,
                    Severity::Error,
                    "action-point-outside",
                    at,
                    format!("subtile ({x},{y}) outside the map"),
                );
            }
        }
    }
    for (n, c) in &nums {
        if *c > 1 {
            issue(
                &mut v,
                Severity::Warning,
                "action-point-duplicate",
                Location::Map,
                // src/actionpt.c:103
                format!("{c} action points numbered {n}; scripts find only one"),
            );
        }
    }
    for (i, l) in doc.lights.items.iter().enumerate() {
        if let Some((x, y)) = things::position(l.x, l.y) {
            if x < 0 || y < 0 || x >= d.stl_w() as i32 || y >= d.stl_h() as i32 {
                issue(
                    &mut v,
                    Severity::Error,
                    "light-outside",
                    Location::Item(ItemRef::Light(i)),
                    format!("subtile ({x},{y}) outside the map"),
                );
            }
        }
    }
    for (i, (x, y)) in loose_gold_in_treasury(doc, gd) {
        issue(
            &mut v,
            Severity::Info,
            "gold-in-treasury",
            Location::Item(ItemRef::Thing(i)),
            // count_gold_hoardes_in_room (src/room_treasure.c:152) via update_room_contents,
            // treasure_room_eats_gold_piles (src/room_treasure.c:95)
            format!(
                "loose gold on the owned treasury slab ({x},{y}) is swept into gold hoards \
            when the level starts"
            ),
        );
    }
    for (i, why) in unreachable_things(doc, gd) {
        // Creatures sealed into vaults are common in shipped maps (scripts open them, 64 in 7 of
        // 274 maps), so they are only a note; collectable objects and traps are a warning.
        let (sev, code) = if thing_class(&doc.things.items[i]) == Some(class::CREATURE) {
            (Severity::Info, "creature-unreachable")
        } else {
            (Severity::Warning, "thing-unreachable")
        };
        issue(&mut v, sev, code, Location::Item(ItemRef::Thing(i)), why);
    }
    v.sort_by(|a, b| b.severity.cmp(&a.severity));
    v
}

/// Things (index, slab) that are loose gold (`Genre = VALUABLE`, `object_is_gold_pile`
/// src/thing_objects.c:464) on a slab of a room whose `UsedCapacity` is `gold_hoardes_in_room` (the
/// treasury) owned by a real player. When the level starts, `reinitialise_map_rooms`
/// (src/room_data.c:1147) runs `do_room_integration` -> `update_room_contents` ->
/// `count_gold_hoardes_in_room` (src/room_treasure.c:152, src/config_terrain.c:320), which deletes every
/// gold pile on the slab's subtiles and creates / tops up a hoard with their gold
/// (`treasure_room_eats_gold_piles` src/room_treasure.c:95; neutral rooms are skipped, line 97).
pub fn loose_gold_in_treasury(doc: &MapDoc, gd: &GameData) -> Vec<(usize, (i32, i32))> {
    let d = doc.dims;
    let mut out = vec![];
    for (i, t) in doc.things.items.iter().enumerate() {
        if thing_class(t) != Some(class::OBJECT)
            || !t
                .subtype
                .is_some_and(|m| things::object_genre_is(gd, m, "VALUABLE"))
        {
            continue;
        }
        let Some((sx, sy)) = things::position(t.x, t.y) else {
            continue;
        };
        let (bx, by) = (sx.div_euclid(3), sy.div_euclid(3));
        if !autotile::in_map(d, bx, by) {
            continue;
        }
        let idx = d.slab_index(bx as usize, by as usize);
        if doc.owners[idx] == PLAYER_NEUTRAL {
            continue;
        }
        let room = gd.slab_room(doc.kinds[idx] as usize);
        let hoards = room > 0
            && gd.rooms.get(room).is_some_and(|r| {
                r.raw
                    .get("UsedCapacity")
                    .and_then(|u| u.split_whitespace().next())
                    .is_some_and(|f| f.eq_ignore_ascii_case("gold_hoardes_in_room"))
            });
        if hoards {
            out.push((i, (bx, by)));
        }
    }
    out
}

/// Things the player (or the heroes) must be able to reach, standing on open floor that no
/// keeper can get to: neither walking nor digging through diggable slabs leads there from any
/// dungeon heart or hero gate (liquids count as passable, doors as open, so islands and door-sealed
/// vaults are not flagged). Only creatures, traps and collectable objects (genre `SPELLBOOK`,
/// `WORKSHOPBOX`, `SPECIALBOX`, `VALUABLE`, `TREASURE_HOARD`) are reported, and nothing when the
/// map has no heart / hero gate at all (such maps are scripted or unfinished). Returns `(thing
/// index, message)`.
///
/// This is a heuristic with no rule in the game behind it (creatures can still be moved by
/// scripts, spells and possession), hence a warning.
pub fn unreachable_things(doc: &MapDoc, gd: &GameData) -> Vec<(usize, String)> {
    let d = doc.dims;
    let (w, h) = (d.w as i32, d.h as i32);
    let slab_of = |t: &kfx_formats::level_fx::FxThing| -> Option<(i32, i32)> {
        let (sx, sy) = things::position(t.x, t.y)?;
        let (bx, by) = (sx.div_euclid(3), sy.div_euclid(3));
        autotile::in_map(d, bx, by).then_some((bx, by))
    };
    let mut anchors = vec![];
    for t in &doc.things.items {
        let model = t.subtype.unwrap_or(-1);
        let anchor = thing_class(t) == Some(class::OBJECT)
            && (is_heart_model(gd, model) || things::is_hero_gate(gd, t));
        if let (true, Some(s)) = (anchor, slab_of(t)) {
            anchors.push(s);
        }
    }
    if anchors.is_empty() {
        return vec![];
    }
    let open = |x: i32, y: i32| {
        let k = doc.kinds[d.slab_index(x as usize, y as usize)] as usize;
        gd.slab(k).block_flags & slab_flags::IS_DOOR != 0
            || doc.floor_height(3 * x + 1, 3 * y + 1) < COLUMN_WALL_HEIGHT
    };
    let diggable = |x: i32, y: i32| {
        let s = gd.slab(doc.kinds[d.slab_index(x as usize, y as usize)] as usize);
        s.is_diggable != 0 || s.block_flags & (slab_flags::DIGGABLE | slab_flags::VALUABLE) != 0
    };
    let mut reached = vec![false; d.slabs()];
    let mut q = VecDeque::new();
    for (x, y) in anchors {
        let i = d.slab_index(x as usize, y as usize);
        if !reached[i] {
            reached[i] = true;
            q.push_back((x, y));
        }
    }
    while let Some((x, y)) = q.pop_front() {
        for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                continue;
            }
            let j = d.slab_index(nx as usize, ny as usize);
            if !reached[j] && (open(nx, ny) || diggable(nx, ny)) {
                reached[j] = true;
                q.push_back((nx, ny));
            }
        }
    }
    let mut out = vec![];
    for (i, t) in doc.things.items.iter().enumerate() {
        let Some((bx, by)) = slab_of(t) else { continue };
        if reached[d.slab_index(bx as usize, by as usize)] || !open(bx, by) {
            continue;
        }
        let model = t.subtype.unwrap_or(-1);
        let wanted = match thing_class(t) {
            Some(class::CREATURE | class::TRAP) => true,
            Some(class::OBJECT) => {
                t.parent_tile.is_none()
                    && [
                        "SPELLBOOK",
                        "WORKSHOPBOX",
                        "SPECIALBOX",
                        "VALUABLE",
                        "TREASURE_HOARD",
                    ]
                    .iter()
                    .any(|g| things::object_genre_is(gd, model, g))
            }
            _ => false,
        };
        if wanted {
            out.push((
                i,
                format!(
                    "class {} model {model} on slab ({bx},{by}) in an area no keeper \
                can reach: walled in by slabs that cannot be dug, with no dungeon heart or hero \
                gate inside",
                    thing_class(t).unwrap_or(0)
                ),
            ));
        }
    }
    out
}
