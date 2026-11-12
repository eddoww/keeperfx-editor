//! The places and things the analyses start from or look at: dungeon hearts, hero gates,
//! portals, and the interesting things of a map (creatures, gold, boxes, books, traps).
use crate::rules::Rules;
use crate::terrain::Terrain;
use kfx_map::autotile::PLAYER_NEUTRAL;
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_map::things::{self, class, thing_class};

/// The hero player (`PLAYER_GOOD`, owner 4).
pub const HEROES: u8 = 4;
/// Keeper owners in script order: PLAYER0..3, PLAYER4..6 (owners 6, 7, 8).
pub const KEEPERS: [u8; 7] = [0, 1, 2, 3, 6, 7, 8];

/// Whether `o` is a keeper owner (not heroes, not neutral).
pub fn is_keeper(o: u8) -> bool {
    KEEPERS.contains(&o)
}

/// Display name of an owner (`PLAYER_NAMES`, src/config_campaigns.c:139).
pub fn owner_name(o: u8) -> &'static str {
    kfx_map::gamedata::PLAYER_NAMES
        .get(o as usize)
        .copied()
        .unwrap_or("?")
}

/// A dungeon heart.
#[derive(Clone, Debug, PartialEq)]
pub struct Heart {
    pub owner: u8,
    /// Slab of the heart object (or the centre of the heart room).
    pub slab: (i32, i32),
    /// Slabs the distance fields start from (the heart room slabs, else the heart slab).
    pub cells: Vec<(i32, i32)>,
}

/// A hero gate.
#[derive(Clone, Debug, PartialEq)]
pub struct Gate {
    pub number: i64,
    pub slab: (i32, i32),
    /// Index of the thing in the document (`None` for a plan).
    pub thing: Option<usize>,
}

/// A portal (connected entrance room slabs).
#[derive(Clone, Debug, PartialEq)]
pub struct Portal {
    pub owner: u8,
    pub cells: Vec<(i32, i32)>,
    pub centre: (i32, i32),
}

/// What an interesting thing is.
#[derive(Clone, Debug, PartialEq)]
pub enum ThingKind {
    /// A creature: model, name, 1-based level, `compute_creature_kind_score`.
    Creature {
        model: i64,
        name: String,
        level: i64,
        score: i64,
    },
    /// Loose gold (value in gold).
    Gold {
        value: u32,
    },
    /// A treasure hoard object.
    Hoard,
    /// Special box, workshop box or spell book (objects.cfg genre).
    Pickup {
        genre: String,
        name: String,
    },
    Trap {
        model: i64,
    },
}

/// An interesting thing.
#[derive(Clone, Debug, PartialEq)]
pub struct ThingSite {
    /// Index in the document's things (`None` for a plan).
    pub thing: Option<usize>,
    pub owner: u8,
    pub slab: (i32, i32),
    pub kind: ThingKind,
}

/// Everything the analyses start from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sites {
    pub hearts: Vec<Heart>,
    pub gates: Vec<Gate>,
    pub portals: Vec<Portal>,
    pub things: Vec<ThingSite>,
}

/// The model of a thing (its `Subtype`, else its `SubtypeStringID` resolved).
pub fn thing_model(gd: &GameData, t: &kfx_formats::level_fx::FxThing) -> Option<i64> {
    t.subtype.or_else(|| {
        let c = thing_class(t)? as i64;
        kfx_map::validate::subtype_by_name(gd, c, t.subtype_name.as_deref()?)
    })
}

/// Score of creature `model` at 1-based level `level` from its config (0 when not loadable).
pub fn creature_score(gd: &GameData, rules: &Rules, model: i64, level: i64) -> i64 {
    let Some(c) = usize::try_from(model)
        .ok()
        .and_then(|m| gd.creature_config_of_model(m))
    else {
        return 0;
    };
    let a = &c.attributes;
    rules.creature_score(
        a.health as i64,
        a.strength as i64,
        a.dexterity as i64,
        a.defence as i64,
        a.armour as i64,
        level - 1,
    )
}

/// The kind of interesting thing `t` is, `None` for decoration and slab objects.
pub fn thing_kind(
    gd: &GameData,
    rules: &Rules,
    t: &kfx_formats::level_fx::FxThing,
) -> Option<ThingKind> {
    let model = thing_model(gd, t)?;
    match thing_class(t)? {
        class::CREATURE => {
            let level = t.creature_level.unwrap_or(1).clamp(1, 10);
            let name = usize::try_from(model)
                .ok()
                .and_then(|m| gd.creatures.get(m.wrapping_sub(1)))
                .cloned()
                .unwrap_or_default();
            Some(ThingKind::Creature {
                model,
                name,
                level,
                score: creature_score(gd, rules, model, level),
            })
        }
        class::TRAP => Some(ThingKind::Trap { model }),
        class::OBJECT => {
            if t.parent_tile.is_some() {
                return None;
            }
            let genre = gd
                .object_genre(model as usize)
                .map(|g| g.trim().to_ascii_uppercase())
                .unwrap_or_default();
            match genre.as_str() {
                "VALUABLE" => {
                    let v = t
                        .gold_value
                        .and_then(|v| u32::try_from(v).ok())
                        .filter(|&v| v > 0)
                        .unwrap_or_else(|| rules.gold_object_value(model));
                    Some(ThingKind::Gold { value: v })
                }
                "TREASURE_HOARD" => Some(ThingKind::Hoard),
                "SPECIALBOX" | "WORKSHOPBOX" | "SPELLBOOK" => {
                    let name = gd
                        .objects_cfg
                        .get(&format!("object{model}"), "Name")
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    Some(ThingKind::Pickup { genre, name })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn slab_of(t: &kfx_formats::level_fx::FxThing, w: i32, h: i32) -> Option<(i32, i32)> {
    let (sx, sy) = things::position(t.x, t.y)?;
    let (x, y) = (sx.div_euclid(3), sy.div_euclid(3));
    (x >= 0 && y >= 0 && x < w && y < h).then_some((x, y))
}

/// Connected groups (4-neighbour) of slabs with the same room id and owner satisfying `keep`.
pub fn room_groups(t: &Terrain, keep: impl Fn(u16) -> bool) -> Vec<(u16, u8, Vec<(i32, i32)>)> {
    let mut seen = vec![false; t.len()];
    let mut out = vec![];
    for i in 0..t.len() {
        let c = t.cells[i];
        if seen[i] || c.room == 0 || !keep(c.room) {
            continue;
        }
        let mut cells = vec![];
        let mut stack = vec![i];
        seen[i] = true;
        while let Some(j) = stack.pop() {
            let (x, y) = t.xy(j);
            cells.push((x, y));
            for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let (nx, ny) = (x + dx, y + dy);
                if !t.inside(nx, ny) {
                    continue;
                }
                let k = t.idx(nx, ny);
                let n = t.cells[k];
                if !seen[k] && n.room == c.room && n.owner == c.owner {
                    seen[k] = true;
                    stack.push(k);
                }
            }
        }
        cells.sort_by_key(|&(x, y)| (y, x));
        out.push((c.room, c.owner, cells));
    }
    out
}

/// Centre (the member closest to the mean) of a slab group.
pub fn group_centre(cells: &[(i32, i32)]) -> (i32, i32) {
    let n = cells.len().max(1) as i64;
    let (sx, sy) = cells
        .iter()
        .fold((0i64, 0i64), |a, c| (a.0 + c.0 as i64, a.1 + c.1 as i64));
    let (mx2, my2) = (2 * sx / n, 2 * sy / n);
    cells
        .iter()
        .copied()
        .min_by_key(|&(x, y)| {
            let (dx, dy) = (2 * x as i64 - mx2, 2 * y as i64 - my2);
            (dx * dx + dy * dy, y, x)
        })
        .unwrap_or((0, 0))
}

/// Room ids of the heart (`DUNGEON_HEART`) and the portal (`ENTRANCE`).
pub fn special_rooms(gd: &GameData) -> (u16, u16) {
    (
        gd.room_id("DUNGEON_HEART").unwrap_or(usize::MAX) as u16,
        gd.room_id("ENTRANCE").unwrap_or(usize::MAX) as u16,
    )
}

impl Sites {
    /// Hearts, gates, portals and things of a document.
    pub fn of_doc(doc: &MapDoc, gd: &GameData, rules: &Rules, terrain: &Terrain) -> Sites {
        let (w, h) = (terrain.w, terrain.h);
        let mut s = Sites::default();
        let (heart_room, portal_room) = special_rooms(gd);
        let heart_groups: Vec<(u16, u8, Vec<(i32, i32)>)> =
            room_groups(terrain, |r| r == heart_room);
        for (i, t) in doc.things.items.iter().enumerate() {
            let Some(slab) = slab_of(t, w, h) else {
                continue;
            };
            let Some(model) = thing_model(gd, t) else {
                continue;
            };
            let owner = t
                .owner
                .and_then(|o| u8::try_from(o).ok())
                .unwrap_or(PLAYER_NEUTRAL);
            if thing_class(t) == Some(class::OBJECT) {
                if kfx_map::validate::is_heart_model(gd, model) {
                    let cells = heart_groups
                        .iter()
                        .find(|g| {
                            g.2.iter()
                                .any(|&c| (c.0 - slab.0).abs() <= 1 && (c.1 - slab.1).abs() <= 1)
                        })
                        .map(|g| g.2.clone())
                        .unwrap_or_else(|| vec![slab]);
                    s.hearts.push(Heart { owner, slab, cells });
                    continue;
                }
                if things::object_genre_is(gd, model, "HEROGATE") {
                    s.gates.push(Gate {
                        number: t.herogate_number.unwrap_or(0),
                        slab,
                        thing: Some(i),
                    });
                    continue;
                }
            }
            if let Some(kind) = thing_kind(gd, rules, t) {
                s.things.push(ThingSite {
                    thing: Some(i),
                    owner,
                    slab,
                    kind,
                });
            }
        }
        // heart rooms without a heart object still count for their keeper
        for (_, owner, cells) in &heart_groups {
            if is_keeper(*owner)
                && !s
                    .hearts
                    .iter()
                    .any(|h| h.owner == *owner && cells.contains(&h.slab))
            {
                s.hearts.push(Heart {
                    owner: *owner,
                    slab: group_centre(cells),
                    cells: cells.clone(),
                });
            }
        }
        s.hearts.sort_by_key(|h| (h.owner, h.slab.1, h.slab.0));
        s.portals = portals_of(terrain, portal_room);
        s
    }

    /// Keeper owners with at least one heart, sorted.
    pub fn keepers(&self) -> Vec<u8> {
        let mut v: Vec<u8> = self
            .hearts
            .iter()
            .map(|h| h.owner)
            .filter(|&o| is_keeper(o))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Start slabs of player `p`'s distance field (all its heart slabs).
    pub fn heart_cells(&self, p: u8) -> Vec<(i32, i32)> {
        self.hearts
            .iter()
            .filter(|h| h.owner == p)
            .flat_map(|h| h.cells.iter().copied())
            .collect()
    }
}

/// Portals: connected groups of the entrance room's slabs.
pub fn portals_of(t: &Terrain, portal_room: u16) -> Vec<Portal> {
    room_groups(t, |r| r == portal_room)
        .into_iter()
        .map(|(_, owner, cells)| Portal {
            owner,
            centre: group_centre(&cells),
            cells,
        })
        .collect()
}
