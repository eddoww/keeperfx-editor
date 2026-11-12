//! Slab classification: what a slab is for movement and digging.
//!
//! The class is derived from the slab kind's `terrain.cfg` attributes the same way the game
//! decides what an imp may dig and where creatures may walk:
//! * diggable for player `p` (`subtile_is_diggable_for_player`, src/map_data.c:805): the kind has
//!   `FILLED`, `DIGGABLE` or `VALUABLE`, `IsDiggable` is set, and it is not a `FILLED` slab of
//!   another owner (enemy and neutral fortified walls cannot be dug by imps). Indestructible kinds
//!   are never removed by digging (`instance_dig` src/creature_instances.c:670 only lowers health of
//!   destructible kinds): gems give gold forever but stay solid.
//! * walkable: not solid. Lava (`WlbType` lava) is an unsafe surface creatures only enter when
//!   they fly or `HurtByLava` is 0 (`navigation_rule_normal` src/ariadne.c:308,
//!   `creature_can_travel_over_lava` src/thing_navigate.c:390). Water is a safe surface for everyone
//!   (only lava and low sacrificial cubes are unsafe, `subtile_is_unsafe` src/map_columns.c:478).
//! * doors (`IS_DOOR`): a locked door blocks its owner's (and allies') navigation, an enemy door
//!   is walked to and broken (`get_navigation_colour_for_door` src/ariadne_update.c:1609).
//!
//! The classification uses the slab kind only (not the column heights), so it is the same for a
//! document and for a generator plan; room furniture and pillars inside a slab are ignored.
use crate::rules::Rules;
use kfx_map::autotile::{WLB_LAVA, WLB_WATER};
use kfx_map::gamedata::{slab_flags, GameData};
use kfx_map::mapdoc::MapDoc;

/// What a slab is for movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    /// Solid and never removable (impenetrable rock, indestructible solid kinds).
    Rock,
    /// Diggable, non-valuable (earth, torch dirt, damaged walls).
    Earth,
    /// Valuable and destructible (gold seams).
    Gold,
    /// Valuable and indestructible (gems).
    Gems,
    /// Fortified (`FILLED`) and destructible: walls; only their owner can dig them.
    Wall,
    /// Open floor (path, claimed floor).
    Floor,
    /// Open room floor (any `IS_ROOM` non-solid kind, bridges included).
    Room,
    Water,
    Lava,
    Door,
}

impl Class {
    pub fn is_solid(self) -> bool {
        matches!(
            self,
            Class::Rock | Class::Earth | Class::Gold | Class::Gems | Class::Wall
        )
    }
    pub fn is_open(self) -> bool {
        !self.is_solid()
    }
    pub fn name(self) -> &'static str {
        match self {
            Class::Rock => "rock",
            Class::Earth => "earth",
            Class::Gold => "gold",
            Class::Gems => "gems",
            Class::Wall => "wall",
            Class::Floor => "floor",
            Class::Room => "room",
            Class::Water => "water",
            Class::Lava => "lava",
            Class::Door => "door",
        }
    }
}

/// One classified slab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub kind: u16,
    pub owner: u8,
    pub class: Class,
    /// Block health of the kind (`game.block_health[BlockHealthIndex]`).
    pub health: u32,
    /// Gold held by the kind (`GoldHeld`).
    pub gold: i32,
    /// Room id of the kind (0 = none), `slab_corresponding_room`.
    pub room: u16,
    /// Door: locked.
    pub locked: bool,
    /// Door: `Health` of its model in `trapdoor.cfg` (0 when unknown).
    pub door_health: u32,
}

/// The classified slab grid of a map.
#[derive(Clone, Debug, PartialEq)]
pub struct Terrain {
    pub w: i32,
    pub h: i32,
    pub cells: Vec<Cell>,
}

/// Per-kind attributes, computed once per [`GameData`].
#[derive(Clone, Debug)]
pub struct KindTable {
    pub class: Vec<Class>,
    pub health: Vec<u32>,
    pub gold: Vec<i32>,
    pub room: Vec<u16>,
    pub door_health: Vec<u32>,
}

impl KindTable {
    pub fn new(gd: &GameData, rules: &Rules) -> KindTable {
        let n = gd.slab_count().max(1);
        let mut t = KindTable {
            class: vec![],
            health: vec![],
            gold: vec![],
            room: vec![],
            door_health: vec![],
        };
        // door model health by slab kind
        let mut door_h = vec![0u32; n];
        for (m, _) in gd.door_names() {
            let h = gd
                .trapdoor_cfg
                .get(&format!("door{m}"), "Health")
                .and_then(|v| v.split_whitespace().next())
                .and_then(parse_u32)
                .unwrap_or(0);
            if let Some(ks) = kfx_map::things::door_slab_kinds(gd, m as i64) {
                for k in ks {
                    if (k as usize) < n {
                        door_h[k as usize] = h;
                    }
                }
            }
        }
        for k in 0..n {
            let s = gd.slab(k);
            t.class.push(classify(gd, k));
            t.health.push(rules.health(s.block_health_index));
            t.gold.push(s.gold_held);
            t.room.push(gd.slab_room(k) as u16);
            t.door_health.push(door_h[k]);
        }
        t
    }
    fn get(&self, kind: u16) -> usize {
        // kinds beyond the count read slab 0 (`get_slab_kind_stats`, src/config_terrain.c:357)
        if (kind as usize) < self.class.len() {
            kind as usize
        } else {
            0
        }
    }
    pub fn cell(&self, kind: u16, owner: u8, locked: bool) -> Cell {
        let k = self.get(kind);
        Cell {
            kind,
            owner,
            class: self.class[k],
            health: self.health[k],
            gold: self.gold[k],
            room: self.room[k],
            locked,
            door_health: self.door_health[k],
        }
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    kfx_map::gamedata::parse_number(s).and_then(|v| u32::try_from(v).ok())
}

/// The class of slab kind `kind` (see the module docs).
pub fn classify(gd: &GameData, kind: usize) -> Class {
    let s = gd.slab(kind);
    let f = s.block_flags;
    if f & slab_flags::IS_DOOR != 0 {
        return Class::Door;
    }
    let solid = f & (slab_flags::FILLED | slab_flags::DIGGABLE | slab_flags::VALUABLE) != 0
        || (f & slab_flags::BLOCKING != 0 && f & slab_flags::IS_ROOM == 0);
    if solid {
        let diggable = s.is_diggable != 0
            && f & (slab_flags::FILLED | slab_flags::DIGGABLE | slab_flags::VALUABLE) != 0;
        if !diggable {
            return Class::Rock;
        }
        if s.indestructible != 0 {
            return if f & slab_flags::VALUABLE != 0 {
                Class::Gems
            } else {
                Class::Rock
            };
        }
        if f & slab_flags::VALUABLE != 0 {
            return Class::Gold;
        }
        if f & slab_flags::FILLED != 0 {
            return Class::Wall;
        }
        return Class::Earth;
    }
    match s.wlb_type {
        WLB_LAVA => Class::Lava,
        WLB_WATER => Class::Water,
        _ if f & slab_flags::IS_ROOM != 0 || gd.slab_room(kind) != 0 => Class::Room,
        _ => Class::Floor,
    }
}

impl Terrain {
    /// Classify kinds / owners (row-major `w x h`); `locked(x, y)` says whether the door on a
    /// door slab is locked.
    pub fn from_grid(
        table: &KindTable,
        w: i32,
        h: i32,
        kinds: &[u16],
        owners: &[u8],
        locked: impl Fn(i32, i32) -> bool,
    ) -> Terrain {
        let mut cells = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let k = kinds[i];
                let c = table.cell(k, owners[i], false);
                let l = c.class == Class::Door && locked(x, y);
                cells.push(Cell { locked: l, ..c });
            }
        }
        Terrain { w, h, cells }
    }

    /// Classify a document (door lock state from its door things).
    pub fn of_doc(doc: &MapDoc, table: &KindTable) -> Terrain {
        let (w, h) = (doc.dims.w as i32, doc.dims.h as i32);
        let locked = door_locks(doc);
        Terrain::from_grid(table, w, h, &doc.kinds, &doc.owners, |x, y| {
            locked.contains(&(x, y))
        })
    }

    /// Re-classify the slabs of a rectangle (inclusive, clipped) from the document.
    pub fn update_rect(
        &mut self,
        doc: &MapDoc,
        table: &KindTable,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    ) {
        let locked = door_locks(doc);
        for y in y0.max(0)..=y1.min(self.h - 1) {
            for x in x0.max(0)..=x1.min(self.w - 1) {
                let i = self.idx(x, y);
                let c = table.cell(doc.kinds[i], doc.owners[i], false);
                self.cells[i] = Cell {
                    locked: c.class == Class::Door && locked.contains(&(x, y)),
                    ..c
                };
            }
        }
    }

    pub fn idx(&self, x: i32, y: i32) -> usize {
        (y * self.w + x) as usize
    }
    pub fn xy(&self, i: usize) -> (i32, i32) {
        (i as i32 % self.w, i as i32 / self.w)
    }
    pub fn inside(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.w && y < self.h
    }
    pub fn at(&self, x: i32, y: i32) -> &Cell {
        &self.cells[self.idx(x, y)]
    }
    pub fn len(&self) -> usize {
        self.cells.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Whether player `p`'s imps can dig slab `c` (`subtile_is_diggable_for_player`
    /// src/map_data.c:805, then removable: not indestructible).
    pub fn diggable_by(c: &Cell, p: u8) -> bool {
        match c.class {
            Class::Earth | Class::Gold => true,
            Class::Wall => c.owner == p,
            _ => false,
        }
    }
}

/// Slabs holding a locked door thing.
pub fn door_locks(doc: &MapDoc) -> Vec<(i32, i32)> {
    doc.things
        .items
        .iter()
        .filter(|t| {
            kfx_map::things::thing_class(t) == Some(kfx_map::things::class::DOOR)
                && t.door_locked.unwrap_or(0) > 0
        })
        .filter_map(|t| {
            kfx_map::things::position(t.x, t.y).map(|(x, y)| (x.div_euclid(3), y.div_euclid(3)))
        })
        .collect()
}
