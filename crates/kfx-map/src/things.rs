//! Things, action points and lights of a [`MapDoc`]: queries and reversible edits.
//!
//! Every mutation returns a [`Patch`] that undoes / redoes it ([`Patch::revert`] /
//! [`Patch::apply`]); the ones that create an item also return its index.
//!
//! # Rules taken from the game
//! * **Doors** are a thing plus the door slab under it. When the level loads, `create_door`
//!   (src/thing_doors.c:140) re-places the slab `slbkind[orientation]` of the door model, so the
//!   thing's `DoorOrientation` decides the slab kind. `trapdoor.cfg` `SlabKind = A B` fills
//!   `slbkind[1] = A`, `slbkind[0] = B` (two `SLABKIND` field entries, src/config_trapdoor.c:270-271,
//!   consumed in order by `assign_conf_command_field`, config.c). The orientation of a newly placed
//!   door is `determine_door_angle` (src/thing_doors.c:84): walls (columns at least
//!   `COLUMN_WALL_HEIGHT` = 5 high, `subtile_is_wall` src/map_columns.c:594) two subtiles north / east
//!   / south / west of the slab centre, through `build_door_angle` (src/thing_doors.c:52). Removing the
//!   door thing turns the slab into claimed path of the same owner, like `destroy_door`
//!   (src/thing_doors.c:284); a locked door's key (object 44, `ObjMdl_SpinningKey`, src/thing_objects.h:85)
//!   is removed with it (`remove_key_on_door`). Placing a non-door slab over a door removes the
//!   door thing ([`MapDoc::place_slab_with_things`]).
//! * **Hero gates** are objects whose `objects.cfg` genre is `HEROGATE`. A new gate gets the
//!   smallest free number 1..255 (`get_free_hero_gate_number`, src/thing_list.c:2361,
//!   `HERO_GATES_COUNT` src/actionpt.h:30).
//! * **Action points** get the smallest free number >= 1 (numbers are looked up by value,
//!   `action_point_get_by_number` src/actionpt.c:103; `ACTN_POINTS_COUNT` 256, src/actionpt.h:29).
//! * **Ownership**: when a room slab changes owner the game re-owns or re-creates its contents
//!   (`change_ownership_or_delete_object_thing_in_room`, src/room_data.c:3456): player-coloured objects
//!   are re-created in the new colour, spellbooks, workshop crates and gold hoards change owner,
//!   lair totems are removed, objects with a parent slab change owner. A door changes owner with
//!   its slab (`change_slab_owner_from_script`, src/room_util.c:353).
//!   [`MapDoc::set_slab_owner_with_things`] does the same.
use crate::autotile::{self, SlabRect};
use crate::gamedata::{parse_number, GameData};
use crate::mapdoc::MapDoc;
use crate::patch::Patch;
use crate::Result;
use kfx_formats::level_fx::{FxActionPoint, FxClass, FxFlag, FxLight, FxThing, StlCoord};

/// `TCls_*` thing classes (src/thing_list.h:40).
pub mod class {
    pub const OBJECT: u8 = 1;
    pub const CREATURE: u8 = 5;
    pub const EFFECT_GEN: u8 = 7;
    pub const TRAP: u8 = 8;
    pub const DOOR: u8 = 9;
}

/// `ObjMdl_SpinningKey` (src/thing_objects.h:85).
pub const OBJ_SPINNING_KEY: i64 = 44;
/// `COLUMN_WALL_HEIGHT` (src/map_columns.h:31).
pub const COLUMN_WALL_HEIGHT: u8 = 5;
/// `build_door_angle` (src/thing_doors.c:52), index = N<<3 | E<<2 | S<<1 | W (bit set = wall on
/// that side, `determine_door_angle` src/thing_doors.c:99-110 and `door_can_stand`
/// src/thing_doors.c:379-394 shift the flags left, so the first side tested, north, is the top bit).
/// `-1` means "no valid door angle": for `door_can_stand` the door is destroyed.
pub const BUILD_DOOR_ANGLE: [i8; 16] = [-1, -1, -1, -1, -1, 0, -1, 0, -1, -1, 1, 1, -1, 0, 1, -1];
/// `small_around` (src/map_utils.c:55): N, E, S, W.
pub const SMALL_AROUND: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// The index into [`BUILD_DOOR_ANGLE`] for the four booleans `[north, east, south, west]`.
pub fn door_wall_flags(walls: [bool; 4]) -> usize {
    walls.iter().fold(0, |a, &w| (a << 1) | w as usize)
}

/// `build_door_angle[flags]`: `Some(0 | 1)` or `None` when the wall pattern is not a door.
pub fn door_angle_for_walls(walls: [bool; 4]) -> Option<u8> {
    u8::try_from(BUILD_DOOR_ANGLE[door_wall_flags(walls)]).ok()
}

/// Any item of the document that has a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemRef {
    Thing(usize),
    ActionPoint(usize),
    Light(usize),
}

/// Inclusive subtile rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StlRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl StlRect {
    pub fn of_slab(x: i32, y: i32) -> StlRect {
        StlRect {
            x0: 3 * x,
            y0: 3 * y,
            x1: 3 * x + 2,
            y1: 3 * y + 2,
        }
    }
    pub fn of_slabs(r: SlabRect) -> StlRect {
        StlRect {
            x0: 3 * r.x0,
            y0: 3 * r.y0,
            x1: 3 * r.x1 + 2,
            y1: 3 * r.y1 + 2,
        }
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

/// Class id (`TCls_*`) of a thing, `None` when unknown.
pub fn thing_class(t: &FxThing) -> Option<u8> {
    t.class
        .as_ref()
        .and_then(|c| c.id())
        .and_then(|i| u8::try_from(i).ok())
}

fn stl_of(c: Option<StlCoord>) -> Option<i32> {
    c.map(|c| c.stl)
}

/// Subtile position of a thing / action point / light.
pub fn position(x: Option<StlCoord>, y: Option<StlCoord>) -> Option<(i32, i32)> {
    Some((stl_of(x)?, stl_of(y)?))
}

/// A centred coordinate on subtile `stl` (`[stl, 128]`).
pub fn centred(stl: i32) -> StlCoord {
    StlCoord::new(stl, 128)
}

/// A new thing of `class` at a subtile, with the fields the game needs for that class
/// (creature level 1, door orientation 0 / unlocked, effect generator range 0).
pub fn new_thing(
    class: u8,
    model: i64,
    owner: u8,
    x: StlCoord,
    y: StlCoord,
    z: StlCoord,
) -> FxThing {
    let mut t = FxThing {
        class: Some(FxClass::from_id(class)),
        subtype: Some(model),
        owner: Some(owner as i64),
        x: Some(x),
        y: Some(y),
        z: Some(z),
        ..Default::default()
    };
    match class {
        class::CREATURE => t.creature_level = Some(1),
        class::DOOR => {
            t.door_orientation = Some(0);
            t.door_locked = Some(0);
        }
        class::EFFECT_GEN => t.effect_range = Some(StlCoord::new(0, 0)),
        _ => {}
    }
    t
}

/// Whether object model `model` has `objects.cfg` genre `genre` (case-insensitive).
pub fn object_genre_is(gd: &GameData, model: i64, genre: &str) -> bool {
    model >= 0
        && gd
            .object_genre(model as usize)
            .is_some_and(|g| g.trim().eq_ignore_ascii_case(genre))
}

pub fn is_hero_gate(gd: &GameData, t: &FxThing) -> bool {
    thing_class(t) == Some(class::OBJECT)
        && t.subtype
            .is_some_and(|m| object_genre_is(gd, m, "HEROGATE"))
}

/// Model of the first `HEROGATE` object in `objects.cfg`.
pub fn hero_gate_model(gd: &GameData) -> Option<i64> {
    gd.object_names()
        .into_iter()
        .map(|(i, _)| i as i64)
        .find(|&i| object_genre_is(gd, i, "HEROGATE"))
}

fn slab_token(gd: &GameData, t: &str) -> Option<u16> {
    match parse_number(t) {
        Some(n) => u16::try_from(n).ok(),
        None => gd.slab_id(t).map(|v| v as u16),
    }
}

/// `slbkind[0..2]` of door model `model` (see the module doc for the swapped order).
pub fn door_slab_kinds(gd: &GameData, model: i64) -> Option<[u16; 2]> {
    let v = gd.trapdoor_cfg.get(&format!("door{model}"), "SlabKind")?;
    let mut it = v.split_whitespace();
    let a = slab_token(gd, it.next()?)?;
    let b = it.next().and_then(|t| slab_token(gd, t)).unwrap_or(0);
    Some([b, a])
}

/// The door model and orientation whose slab kind is `kind` (model 0, "NODOOR", excluded).
pub fn door_of_slab_kind(gd: &GameData, kind: u16) -> Option<(i64, u8)> {
    gd.door_names()
        .into_iter()
        .filter(|(i, _)| *i > 0)
        .find_map(|(i, _)| {
            let k = door_slab_kinds(gd, i as i64)?;
            k.iter()
                .position(|&s| s == kind)
                .map(|o| (i as i64, o as u8))
        })
}

/// The slab test of `door_can_stand` (src/thing_doors.c:386-388): fortified wall or friable dirt
/// category, or kind rock / gold / dense gold / gems. A kind outside terrain.cfg reads as slab 0
/// stats (`get_slab_kind_stats`, src/config_terrain.c:357) but keeps its own kind number.
pub fn slab_counts_as_door_wall(gd: &GameData, kind: usize) -> bool {
    let cat = gd.slab(kind).category;
    cat == crate::gamedata::category::FORTIFIED_WALL
        || cat == crate::gamedata::category::FRIABLE_DIRT
        || matches!(kind, 0 | 1 | 52 | 60)
}

/// Floor height of a column in subtiles (`find_column_height`, src/map_columns.c:185).
pub fn column_floor_height(c: &kfx_formats::level_bin::Column) -> u8 {
    if c.solidmask == 0 {
        0
    } else {
        c.cubes.iter().position(|&q| q == 0).unwrap_or(8) as u8
    }
}

impl MapDoc {
    // ------------------------------------------------------------------ queries

    /// Floor height (subtiles) at a subtile of the current column grid, 0 outside.
    pub fn floor_height(&self, stl_x: i32, stl_y: i32) -> u8 {
        if stl_x < 0
            || stl_y < 0
            || stl_x as usize > self.dims.stl_w()
            || stl_y as usize > self.dims.stl_h()
        {
            return 0;
        }
        column_floor_height(&self.columns[self.dims.stl_index(stl_x as usize, stl_y as usize)])
    }

    /// `[floor height, 0]`: the z of something standing on the floor of a subtile.
    pub fn floor_z(&self, stl_x: i32, stl_y: i32) -> StlCoord {
        StlCoord::new(self.floor_height(stl_x, stl_y) as i32, 0)
    }

    /// Indices of the things whose subtile lies in `r`.
    pub fn things_in(&self, r: StlRect) -> Vec<usize> {
        self.things
            .items
            .iter()
            .enumerate()
            .filter(|(_, t)| position(t.x, t.y).is_some_and(|(x, y)| r.contains(x, y)))
            .map(|(i, _)| i)
            .collect()
    }
    pub fn things_on_slab(&self, x: i32, y: i32) -> Vec<usize> {
        self.things_in(StlRect::of_slab(x, y))
    }
    pub fn things_at(&self, stl_x: i32, stl_y: i32) -> Vec<usize> {
        self.things_in(StlRect {
            x0: stl_x,
            y0: stl_y,
            x1: stl_x,
            y1: stl_y,
        })
    }
    pub fn action_points_in(&self, r: StlRect) -> Vec<usize> {
        let it = self.action_points.items.iter().enumerate();
        it.filter(|(_, a)| position(a.x, a.y).is_some_and(|(x, y)| r.contains(x, y)))
            .map(|(i, _)| i)
            .collect()
    }
    pub fn lights_in(&self, r: StlRect) -> Vec<usize> {
        let it = self.lights.items.iter().enumerate();
        it.filter(|(_, l)| position(l.x, l.y).is_some_and(|(x, y)| r.contains(x, y)))
            .map(|(i, _)| i)
            .collect()
    }
    /// Every item in `r`.
    pub fn items_in(&self, r: StlRect) -> Vec<ItemRef> {
        let mut v: Vec<ItemRef> = self.things_in(r).into_iter().map(ItemRef::Thing).collect();
        v.extend(
            self.action_points_in(r)
                .into_iter()
                .map(ItemRef::ActionPoint),
        );
        v.extend(self.lights_in(r).into_iter().map(ItemRef::Light));
        v
    }
    /// Index of the door thing on slab (x, y).
    pub fn door_on_slab(&self, x: i32, y: i32) -> Option<usize> {
        self.things_on_slab(x, y)
            .into_iter()
            .find(|&i| thing_class(&self.things.items[i]) == Some(class::DOOR))
    }
    /// Index of the action point numbered `n`.
    pub fn action_point_by_number(&self, n: i64) -> Option<usize> {
        self.action_points
            .items
            .iter()
            .position(|a| a.number == Some(n))
    }

    /// `determine_door_angle` (src/thing_doors.c:84) on the current columns: `Some(0 | 1)`, `None`
    /// when the walls around do not decide.
    pub fn door_angle(&self, x: i32, y: i32) -> Option<u8> {
        let (cx, cy) = (3 * x + 1, 3 * y + 1);
        let walls = SMALL_AROUND
            .map(|(dx, dy)| self.floor_height(cx + 2 * dx, cy + 2 * dy) >= COLUMN_WALL_HEIGHT);
        door_angle_for_walls(walls)
    }

    /// `door_can_stand` (src/thing_doors.c:379) for a door on slab (x, y): the four neighbouring
    /// slabs (N, E, S, W; outside the map counts as rock, `get_slabmap_block` returns the
    /// zeroed `bad_slabmap_block`, src/slab_data.c:83-90) are "walls" when their category is
    /// fortified wall or friable dirt, or their kind is rock, gold, dense gold or gems; the door
    /// stands only when [`BUILD_DOOR_ANGLE`] has an angle for that pattern, i.e. walls on both east
    /// and west, or on both north and south, but not on all four sides. `process_door`
    /// (src/thing_doors.c:575) destroys a door that cannot stand.
    pub fn door_can_stand(&self, gd: &GameData, x: i32, y: i32) -> bool {
        self.door_stand_walls(gd, x, y)
            .is_some_and(|w| door_angle_for_walls(w).is_some())
    }

    /// The wall flags `[north, east, south, west]` `door_can_stand` computes, `None` when the
    /// slab itself is outside the map.
    pub fn door_stand_walls(&self, gd: &GameData, x: i32, y: i32) -> Option<[bool; 4]> {
        if !autotile::in_map(self.dims, x, y) {
            return None;
        }
        Some(SMALL_AROUND.map(|(dx, dy)| {
            let (nx, ny) = (x + dx, y + dy);
            if !autotile::in_map(self.dims, nx, ny) {
                return true;
            }
            let k = self.kinds[self.dims.slab_index(nx as usize, ny as usize)] as usize;
            slab_counts_as_door_wall(gd, k)
        }))
    }

    /// Smallest free hero gate number (1..255), `None` when all are used.
    pub fn free_hero_gate_number(&self, gd: &GameData) -> Option<i64> {
        let used: Vec<i64> = self
            .things
            .items
            .iter()
            .filter(|t| is_hero_gate(gd, t))
            .filter_map(|t| t.herogate_number)
            .collect();
        (1..256).find(|n| !used.contains(n))
    }
    /// Smallest free action point number (1..255).
    pub fn free_action_point_number(&self) -> Option<i64> {
        let used: Vec<i64> = self
            .action_points
            .items
            .iter()
            .filter_map(|a| a.number)
            .collect();
        (1..256).find(|n| !used.contains(n))
    }

    // ------------------------------------------------------------------ things

    fn key_on(&self, stl_x: i32, stl_y: i32) -> Option<usize> {
        self.things_at(stl_x, stl_y).into_iter().find(|&i| {
            let t = &self.things.items[i];
            thing_class(t) == Some(class::OBJECT) && t.subtype == Some(OBJ_SPINNING_KEY)
        })
    }

    /// Door bookkeeping for a door thing at `idx`: put its slab (kind from model +
    /// orientation, owner from the thing) and its key (locked) in place.
    fn sync_door(&mut self, gd: &GameData, idx: usize) {
        self.touch_things();
        let t = self.things.items[idx].clone();
        let Some((sx, sy)) = position(t.x, t.y) else {
            return;
        };
        let (bx, by) = (sx.div_euclid(3), sy.div_euclid(3));
        if !autotile::in_map(self.dims, bx, by) {
            return;
        }
        let orient = t.door_orientation.unwrap_or(0).clamp(0, 1) as usize;
        let owner = t
            .owner
            .unwrap_or(autotile::PLAYER_NEUTRAL as i64)
            .clamp(0, 255) as u8;
        if let Some(kinds) = t.subtype.and_then(|m| door_slab_kinds(gd, m)) {
            let _ = self.set_slab(gd, bx as usize, by as usize, kinds[orient]);
            let _ = self.set_owner(gd, bx as usize, by as usize, owner);
        }
        let (cx, cy) = (3 * bx + 1, 3 * by + 1);
        let key = self.key_on(cx, cy);
        let locked = t.door_locked.unwrap_or(0) > 0;
        match (locked, key) {
            (true, None) => {
                let mut k = new_thing(
                    class::OBJECT,
                    OBJ_SPINNING_KEY,
                    owner,
                    centred(cx),
                    centred(cy),
                    StlCoord::new(5, 0),
                );
                k.parent_tile = None;
                self.things.items.push(k);
            }
            (false, Some(k)) => {
                self.things.items.remove(k);
            }
            (true, Some(k)) => self.things.items[k].owner = Some(owner as i64),
            _ => {}
        }
    }

    /// Remove door bookkeeping of the door at `idx` (before removing / moving it): key gone,
    /// slab back to claimed path of its owner (`destroy_door`, src/thing_doors.c:284).
    fn unsync_door(&mut self, gd: &GameData, idx: usize) {
        self.touch_things();
        let t = self.things.items[idx].clone();
        let Some((sx, sy)) = position(t.x, t.y) else {
            return;
        };
        let (bx, by) = (sx.div_euclid(3), sy.div_euclid(3));
        if let Some(k) = self.key_on(3 * bx + 1, 3 * by + 1) {
            self.things.items.remove(k);
        }
        if autotile::in_map(self.dims, bx, by) {
            let i = self.dims.slab_index(bx as usize, by as usize);
            if gd.slab(self.kinds[i] as usize).block_flags & crate::gamedata::slab_flags::IS_DOOR
                != 0
            {
                let _ = self.set_slab(
                    gd,
                    bx as usize,
                    by as usize,
                    autotile::kinds::CLAIMED as u16,
                );
            }
        }
    }

    /// Add a thing. Doors also place their slab and key (see [`MapDoc::place_door`] to choose
    /// the orientation from the walls); a hero gate without a number gets the smallest free one.
    pub fn add_thing(&mut self, gd: &GameData, t: FxThing) -> (usize, Patch) {
        self.record_tracked(|d| d.add_thing_inner(gd, t))
    }

    /// [`MapDoc::add_thing`] without the recording. A door also re-places the slab objects
    /// around it (the game's `create_door` re-places the door slab with
    /// `place_animating_slab_type_on_map`, src/thing_doors.c:177, which runs the slab-object
    /// creation of `place_single_slab_type_on_map` src/map_blocks.c:1374 for the 3x3 block);
    /// objects of its neighbours are refreshed because their torches depend on the new slab.
    fn add_thing_inner(&mut self, gd: &GameData, t: FxThing) -> usize {
        self.touch_things();
        let mut t = t;
        if is_hero_gate(gd, &t) && t.herogate_number.is_none() {
            t.herogate_number = self.free_hero_gate_number(gd);
        }
        let door = thing_class(&t) == Some(class::DOOR);
        let pos = position(t.x, t.y);
        if door {
            // one door per slab
            if let Some((sx, sy)) = pos {
                if let Some(old) = self.door_on_slab(sx.div_euclid(3), sy.div_euclid(3)) {
                    self.unsync_door(gd, old);
                    let old = self
                        .door_on_slab(sx.div_euclid(3), sy.div_euclid(3))
                        .unwrap_or(old);
                    self.things.items.remove(old);
                }
            }
        }
        self.things.items.push(t);
        let mut idx = self.things.items.len() - 1;
        if door {
            self.sync_door(gd, idx);
            if let Some((sx, sy)) = pos {
                let (bx, by) = (sx.div_euclid(3), sy.div_euclid(3));
                if autotile::in_map(self.dims, bx, by) {
                    let r = SlabRect::single(bx, by).grow(1).clip(self.dims);
                    self.apply_slab_objects(gd, r);
                    // the refresh may have shifted the index
                    idx = self.door_on_slab(bx, by).unwrap_or(idx);
                }
            }
        }
        idx
    }

    /// Place a door of `model` on slab (x, y), orientation from the walls around
    /// ([`MapDoc::door_angle`], falling back to the orientation matching the current slab kind,
    /// else 0). Returns the door's index.
    pub fn place_door(
        &mut self,
        gd: &GameData,
        x: usize,
        y: usize,
        model: i64,
        owner: u8,
        locked: bool,
    ) -> Result<(usize, Patch)> {
        if x >= self.dims.w as usize || y >= self.dims.h as usize {
            return Err(crate::Error::Invalid(format!(
                "slab ({x},{y}) outside the map"
            )));
        }
        let cur = self.kinds[self.dims.slab_index(x, y)];
        let orient = self
            .door_angle(x as i32, y as i32)
            .or_else(|| {
                door_of_slab_kind(gd, cur)
                    .filter(|(m, _)| *m == model)
                    .map(|(_, o)| o)
            })
            .unwrap_or(0);
        let (cx, cy) = (3 * x as i32 + 1, 3 * y as i32 + 1);
        let mut t = new_thing(
            class::DOOR,
            model,
            owner,
            centred(cx),
            centred(cy),
            StlCoord::new(1, 128),
        );
        t.door_orientation = Some(orient as i64);
        t.door_locked = Some(locked as i64);
        Ok(self.add_thing(gd, t))
    }

    /// Remove a thing; a door also gives back its slab as claimed path and loses its key.
    pub fn remove_thing(&mut self, gd: &GameData, idx: usize) -> Patch {
        self.record_tracked(|d| {
            d.touch_things();
            if idx >= d.things.items.len() {
                return;
            }
            let t = d.things.items[idx].clone();
            if thing_class(&t) == Some(class::DOOR) {
                d.unsync_door(gd, idx);
                // the key removal may have shifted the index
                if let Some(i) = d.things.items.iter().position(|x| *x == t) {
                    d.things.items.remove(i);
                }
            } else {
                d.things.items.remove(idx);
            }
        })
        .1
    }

    /// Replace thing `idx` (door slab / key follow a door's new model, orientation, lock state,
    /// owner and position).
    pub fn update_thing(&mut self, gd: &GameData, idx: usize, t: FxThing) -> Patch {
        self.record_tracked(|d| {
            d.touch_things();
            if idx >= d.things.items.len() {
                return;
            }
            let old = d.things.items[idx].clone();
            if thing_class(&old) == Some(class::DOOR) {
                d.unsync_door(gd, idx);
                let i = d.things.items.iter().position(|x| *x == old).unwrap_or(idx);
                d.things.items[i] = t.clone();
                if thing_class(&t) == Some(class::DOOR) {
                    d.sync_door(gd, i);
                }
            } else {
                d.things.items[idx] = t.clone();
                if thing_class(&t) == Some(class::DOOR) {
                    d.sync_door(gd, idx);
                }
            }
        })
        .1
    }

    /// Move thing `idx`. `z = None` keeps the height above the floor (objects, creatures, traps
    /// stand on the new floor). A door moves to the centre of the target slab with its slab.
    pub fn move_thing(
        &mut self,
        gd: &GameData,
        idx: usize,
        x: StlCoord,
        y: StlCoord,
        z: Option<StlCoord>,
    ) -> Patch {
        let Some(old) = self.things.items.get(idx).cloned() else {
            return Patch::new();
        };
        let mut t = old.clone();
        if thing_class(&t) == Some(class::DOOR) {
            t.x = Some(centred(3 * x.stl.div_euclid(3) + 1));
            t.y = Some(centred(3 * y.stl.div_euclid(3) + 1));
            t.z = z.or(t.z);
        } else {
            let z = z.unwrap_or_else(|| {
                let above = match (old.x, old.y, old.z) {
                    (Some(ox), Some(oy), Some(oz)) => {
                        (oz.raw() as i32 - ((self.floor_height(ox.stl, oy.stl) as i32) << 8)).max(0)
                    }
                    _ => 0,
                };
                StlCoord::from_raw((((self.floor_height(x.stl, y.stl) as i32) << 8) + above) as u32)
            });
            t.x = Some(x);
            t.y = Some(y);
            t.z = Some(z);
        }
        self.update_thing(gd, idx, t)
    }

    /// Renumber hero gates 1..n keeping their current order by number (then by index).
    pub fn renumber_hero_gates(&mut self, gd: &GameData) -> Patch {
        self.record_tracked(|d| {
            d.touch_things();
            let mut gates: Vec<(i64, usize)> = d
                .things
                .items
                .iter()
                .enumerate()
                .filter(|(_, t)| is_hero_gate(gd, t))
                .map(|(i, t)| (t.herogate_number.unwrap_or(i64::MAX), i))
                .collect();
            gates.sort();
            for (n, (_, i)) in gates.into_iter().enumerate() {
                d.things.items[i].herogate_number = Some(n as i64 + 1);
            }
        })
        .1
    }

    // ------------------------------------------------------------------ action points

    /// Add an action point with the smallest free number (or `number` when given).
    pub fn add_action_point(
        &mut self,
        x: StlCoord,
        y: StlCoord,
        range: StlCoord,
        number: Option<i64>,
    ) -> (usize, Patch) {
        self.record_tracked(|d| {
            d.touch_action_points();
            let n = number.or_else(|| d.free_action_point_number());
            d.action_points.items.push(FxActionPoint {
                number: n,
                x: Some(x),
                y: Some(y),
                range: Some(range),
                ..Default::default()
            });
            d.action_points.items.len() - 1
        })
    }
    pub fn remove_action_point(&mut self, idx: usize) -> Patch {
        self.record_tracked(|d| {
            d.touch_action_points();
            if idx < d.action_points.items.len() {
                d.action_points.items.remove(idx);
            }
        })
        .1
    }
    pub fn update_action_point(&mut self, idx: usize, a: FxActionPoint) -> Patch {
        self.record_tracked(|d| {
            d.touch_action_points();
            if let Some(s) = d.action_points.items.get_mut(idx) {
                *s = a;
            }
        })
        .1
    }
    pub fn move_action_point(&mut self, idx: usize, x: StlCoord, y: StlCoord) -> Patch {
        let Some(mut a) = self.action_points.items.get(idx).cloned() else {
            return Patch::new();
        };
        a.x = Some(x);
        a.y = Some(y);
        self.update_action_point(idx, a)
    }
    /// Renumber action points 1..n in their current number order.
    pub fn renumber_action_points(&mut self) -> Patch {
        self.record_tracked(|d| {
            d.touch_action_points();
            let mut v: Vec<(i64, usize)> = d
                .action_points
                .items
                .iter()
                .enumerate()
                .map(|(i, a)| (a.number.unwrap_or(i64::MAX), i))
                .collect();
            v.sort();
            for (n, (_, i)) in v.into_iter().enumerate() {
                d.action_points.items[i].number = Some(n as i64 + 1);
            }
        })
        .1
    }

    // ------------------------------------------------------------------ lights

    /// Add a static light (not attached to a slab).
    pub fn add_light(
        &mut self,
        x: StlCoord,
        y: StlCoord,
        z: StlCoord,
        range: StlCoord,
        intensity: i64,
    ) -> (usize, Patch) {
        self.record_tracked(|d| {
            d.touch_lights();
            d.lights.items.push(FxLight {
                dynamic: Some(FxFlag::Bool(false)),
                x: Some(x),
                y: Some(y),
                z: Some(z),
                range: Some(range),
                intensity: Some(intensity),
                ..Default::default()
            });
            d.lights.items.len() - 1
        })
    }
    pub fn remove_light(&mut self, idx: usize) -> Patch {
        self.record_tracked(|d| {
            d.touch_lights();
            if idx < d.lights.items.len() {
                d.lights.items.remove(idx);
            }
        })
        .1
    }
    pub fn update_light(&mut self, idx: usize, l: FxLight) -> Patch {
        self.record_tracked(|d| {
            d.touch_lights();
            if let Some(s) = d.lights.items.get_mut(idx) {
                *s = l;
            }
        })
        .1
    }
    pub fn move_light(
        &mut self,
        idx: usize,
        x: StlCoord,
        y: StlCoord,
        z: Option<StlCoord>,
    ) -> Patch {
        let Some(mut l) = self.lights.items.get(idx).cloned() else {
            return Patch::new();
        };
        l.x = Some(x);
        l.y = Some(y);
        if z.is_some() {
            l.z = z;
        }
        self.update_light(idx, l)
    }

    // ------------------------------------------------------------------ items generically

    /// Remove several items at once (indices refer to the current lists).
    pub fn remove_items(&mut self, gd: &GameData, items: &[ItemRef]) -> Patch {
        let mut items = items.to_vec();
        items.sort();
        items.dedup();
        let mut p = Patch::new();
        // highest index first so the others stay valid
        for it in items.into_iter().rev() {
            p.extend(match it {
                ItemRef::Thing(i) => self.remove_thing(gd, i),
                ItemRef::ActionPoint(i) => self.remove_action_point(i),
                ItemRef::Light(i) => self.remove_light(i),
            });
        }
        p
    }

    // ------------------------------------------------------------------ slabs with things

    /// Change a slab's owner like the game re-owns a slab (see the module doc): columns and
    /// slab objects of the 3x3 block regenerated, the door re-owned, room contents re-owned or
    /// re-coloured, lair totems removed.
    pub fn set_slab_owner_with_things(
        &mut self,
        gd: &GameData,
        x: usize,
        y: usize,
        owner: u8,
    ) -> Result<Patch> {
        self.check_slab(x, y)?;
        Ok(self
            .record_tracked(|d| {
                d.touch_things();
                let _ = d.set_owner(gd, x, y, owner);
                let r = SlabRect::single(x as i32, y as i32).grow(1).clip(d.dims);
                d.apply_slab_objects(gd, r);
                let kind = d.kinds[d.dims.slab_index(x, y)] as usize;
                let is_room = gd.slab(kind).block_flags & crate::gamedata::slab_flags::IS_ROOM != 0;
                let mut remove = vec![];
                for i in d.things_on_slab(x as i32, y as i32) {
                    let t = &mut d.things.items[i];
                    match thing_class(t) {
                        Some(class::DOOR) => t.owner = Some(owner as i64),
                        Some(class::OBJECT) if is_room => {
                            let m = t.subtype.unwrap_or(-1);
                            let base = gd.coloured_object_base(m as i16);
                            if base != 0 {
                                t.subtype =
                                    Some(gd.coloured_object_model(
                                        base,
                                        autotile::player_color_idx(owner),
                                    ) as i64);
                                t.owner = Some(owner as i64);
                            } else if object_genre_is(gd, m, "LAIR_TOTEM") {
                                remove.push(i);
                            } else if ["SPELLBOOK", "WORKSHOPBOX", "TREASURE_HOARD"]
                                .iter()
                                .any(|g| object_genre_is(gd, m, g))
                            {
                                t.owner = Some(owner as i64);
                            }
                        }
                        _ => {}
                    }
                }
                for i in remove.into_iter().rev() {
                    d.things.items.remove(i);
                }
                if let Some(di) = d.door_on_slab(x as i32, y as i32) {
                    if let Some(k) = d.key_on(3 * x as i32 + 1, 3 * y as i32 + 1) {
                        d.things.items[k].owner = Some(owner as i64);
                    }
                    let _ = di;
                }
            })
            .1)
    }

    /// [`MapDoc::place_slab`] plus everything that hangs on the slab: the slab objects of the
    /// regenerated 3x3 block ([`MapDoc::apply_slab_objects`]), the door thing (a door slab kind
    /// creates / updates a door of the matching model, oriented by the walls around
    /// ([`MapDoc::door_angle`], else the orientation of the given kind) with the slab kind
    /// following the orientation, keeping its lock state; a non-door kind over a door removes the
    /// door and its key), and things left inside a slab that became solid are removed (creatures,
    /// objects, traps). Action points and lights stay.
    pub fn place_slab_with_things(
        &mut self,
        gd: &GameData,
        x: usize,
        y: usize,
        kind: u16,
        owner: u8,
    ) -> Result<Patch> {
        self.check_slab(x, y)?;
        Ok(self
            .record_tracked(|d| {
                d.touch_things();
                let door = door_of_slab_kind(gd, kind);
                let old_door = d.door_on_slab(x as i32, y as i32);
                let rect = match d.place_slab(gd, x, y, kind, owner) {
                    Ok(r) => r,
                    Err(_) => return,
                };
                match (door, old_door) {
                    (Some((model, orient)), old) => {
                        // the orientation the walls ask for wins over the kind's own
                        let orient = d.door_angle(x as i32, y as i32).unwrap_or(orient);
                        let locked = old
                            .map(|i| d.things.items[i].door_locked.unwrap_or(0))
                            .unwrap_or(0);
                        if let Some(i) = old {
                            d.things.items.remove(i);
                        }
                        let (cx, cy) = (3 * x as i32 + 1, 3 * y as i32 + 1);
                        let mut t = new_thing(
                            class::DOOR,
                            model,
                            owner,
                            centred(cx),
                            centred(cy),
                            StlCoord::new(1, 128),
                        );
                        t.door_orientation = Some(orient as i64);
                        t.door_locked = Some(locked);
                        d.things.items.push(t);
                        let i = d.things.items.len() - 1;
                        d.sync_door(gd, i);
                    }
                    (None, Some(i)) => {
                        if let Some(k) = d.key_on(3 * x as i32 + 1, 3 * y as i32 + 1) {
                            d.things.items.remove(k);
                        }
                        if let Some(i) = d.door_on_slab(x as i32, y as i32).or(Some(i)) {
                            d.things.items.remove(i);
                        }
                    }
                    (None, None) => {}
                }
                d.apply_slab_objects(gd, rect);
                // things buried in a solid slab
                let filled = gd
                    .slab(d.kinds[d.dims.slab_index(x, y)] as usize)
                    .block_flags
                    & (crate::gamedata::slab_flags::FILLED | crate::gamedata::slab_flags::BLOCKING)
                    != 0
                    && door.is_none();
                if filled {
                    let slab = autotile::slab_number(d.dims, x as i32, y as i32) as i64;
                    let mut buried: Vec<usize> = d
                        .things_on_slab(x as i32, y as i32)
                        .into_iter()
                        .filter(|&i| {
                            let t = &d.things.items[i];
                            matches!(
                                thing_class(t),
                                Some(class::CREATURE | class::OBJECT | class::TRAP)
                            ) && t.parent_tile != Some(slab)
                        })
                        .collect();
                    buried.sort();
                    for i in buried.into_iter().rev() {
                        d.things.items.remove(i);
                    }
                }
            })
            .1)
    }

    fn check_slab(&self, x: usize, y: usize) -> Result<()> {
        if x >= self.dims.w as usize || y >= self.dims.h as usize {
            return Err(crate::Error::Invalid(format!(
                "slab ({x},{y}) outside {}x{}",
                self.dims.w, self.dims.h
            )));
        }
        Ok(())
    }
}
