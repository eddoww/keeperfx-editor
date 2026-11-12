//! Conversion to and from the editor's clipboard shape.
//!
//! The editor's `Clipboard` (`kfx-editor/src/tools.rs`) lives in the binary crate, so it cannot be
//! named here. [`ClipBlock`] / [`ClipItem`] mirror it field for field (slab block of
//! `(kind, owner)`, items with raw offsets from the block's top-left); the editor implements the
//! two-method trait [`ClipboardLike`] for its `Clipboard` (a field-by-field move) and then
//! [`Prefab::to_clipboard`] / [`Prefab::from_clipboard`] convert.
//!
//! Limits of the clipboard shape, which has no mask and no names: masked prefab cells become
//! earth, hero gate / action point keys and script fragments are not carried (the editor's paste
//! clears numbers anyway), and `z` is the raw height above the floor of the copy source, which
//! the paste keeps as is. Use [`crate::apply`] directly where the editor can: it keeps all of it.
use crate::apply::OwnerMap;
use crate::model::*;
use crate::names::Names;
use kfx_formats::level_fx::{FxActionPoint, FxLight, FxThing, StlCoord};
use kfx_map::autotile::PLAYER_NEUTRAL;
use kfx_map::gamedata::GameData;
use kfx_map::things;

/// Mirror of the editor's `ClipItem`.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipItem {
    Thing(FxThing),
    ActionPoint(FxActionPoint),
    Light(FxLight),
}

/// Mirror of the editor's `Clipboard`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipBlock {
    pub w: i32,
    pub h: i32,
    pub cells: Vec<(u16, u8)>,
    /// (u, v) raw offset from the block's top-left, item.
    pub items: Vec<(i64, i64, ClipItem)>,
}

/// Implemented by the editor for its clipboard type.
pub trait ClipboardLike: Sized {
    fn from_block(b: ClipBlock) -> Self;
    fn to_block(&self) -> ClipBlock;
}

fn coord(raw: i64) -> StlCoord {
    StlCoord::new(raw.div_euclid(256) as i32, raw.rem_euclid(256) as i32)
}

impl Prefab {
    /// The prefab as clipboard data (names resolved with `gd`; unknown names are dropped).
    pub fn to_block(&self, gd: &GameData, owners: &OwnerMap) -> ClipBlock {
        let names = Names::new(gd);
        let cells = self
            .cells
            .iter()
            .map(|c| match c {
                Some(c) => (names.slab_id(&c.slab).unwrap_or(2), owners.map(c.owner)),
                None => (2, PLAYER_NEUTRAL),
            })
            .collect();
        let mut items = vec![];
        for t in &self.things {
            let Some(model) = names.model_id(t.class, &t.model) else {
                continue;
            };
            let mut th = things::new_thing(
                t.class.id(),
                model,
                owners.map(t.owner),
                coord(t.x as i64),
                coord(t.y as i64),
                coord(t.above as i64),
            );
            th.orientation = t.orientation;
            if t.class == ThingClass::Creature {
                th.creature_level = Some(t.level.unwrap_or(1));
                th.creature_gold = t.gold;
                th.creature_initial_health = t.health;
                th.creature_name = t.name.clone();
            }
            if t.class == ThingClass::Door {
                th.door_orientation = t.door_orientation;
                th.door_locked = Some(t.door_locked.unwrap_or(false) as i64);
            }
            th.custom_box = t.custom_box;
            th.gold_value = t.gold_value;
            th.effect_range = t.effect_range.map(|r| coord(r as i64));
            th.extra = t
                .extra
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            items.push((t.x as i64, t.y as i64, ClipItem::Thing(th)));
        }
        for a in &self.action_points {
            let ap = FxActionPoint {
                number: a.number,
                x: Some(coord(a.x as i64)),
                y: Some(coord(a.y as i64)),
                range: Some(coord(a.range as i64)),
                extra: Default::default(),
            };
            items.push((a.x as i64, a.y as i64, ClipItem::ActionPoint(ap)));
        }
        for l in &self.lights {
            let li = FxLight {
                dynamic: Some(kfx_formats::level_fx::FxFlag::Bool(l.dynamic)),
                x: Some(coord(l.x as i64)),
                y: Some(coord(l.y as i64)),
                z: Some(coord(l.above as i64)),
                range: Some(coord(l.range as i64)),
                intensity: Some(l.intensity),
                parent_tile: None,
                extra: Default::default(),
            };
            items.push((l.x as i64, l.y as i64, ClipItem::Light(li)));
        }
        ClipBlock {
            w: self.w as i32,
            h: self.h as i32,
            cells,
            items,
        }
    }

    /// A prefab from clipboard data. Players other than neutral and the hero player become
    /// relative slots in ascending order.
    pub fn from_block(b: &ClipBlock, gd: &GameData, name: &str) -> Prefab {
        let names = Names::new(gd);
        let (w, h) = (b.w.max(1) as u32, b.h.max(1) as u32);
        let mut p = Prefab::blank(name, w, h);
        let mut players: std::collections::BTreeSet<u8> = b.cells.iter().map(|c| c.1).collect();
        for (_, _, it) in &b.items {
            if let ClipItem::Thing(t) = it {
                if let Some(o) = t.owner {
                    players.insert(o.clamp(0, 255) as u8);
                }
            }
        }
        players.remove(&PLAYER_NEUTRAL);
        players.remove(&kfx_map::validate::PLAYER_GOOD);
        let slots: Vec<u8> = players.into_iter().collect();
        let owner = |pl: i64| -> Owner {
            let pl = pl.clamp(0, 255) as u8;
            if pl == PLAYER_NEUTRAL {
                Owner::Neutral
            } else if let Some(i) = slots.iter().position(|&s| s == pl) {
                Owner::Slot(i as u8)
            } else {
                Owner::Fixed(pl)
            }
        };
        for (i, &(kind, ow)) in b.cells.iter().enumerate().take((w * h) as usize) {
            p.cells[i] = Some(Cell::new(&names.slab_name(kind), owner(ow as i64)));
        }
        for (u, v, it) in &b.items {
            match it {
                ClipItem::Thing(t) => {
                    if t.parent_tile.is_some() || t.subtype == Some(things::OBJ_SPINNING_KEY) {
                        continue;
                    }
                    if let Some(th) =
                        crate::capture::thing_to_prefab(gd, &names, t, (0, 0), 0, &owner)
                    {
                        let mut th = th;
                        th.x = *u as i32;
                        th.y = *v as i32;
                        th.gate = None;
                        p.things.push(th);
                    }
                }
                ClipItem::ActionPoint(a) => p.action_points.push(PrefabAp {
                    key: format!("ap{}", p.action_points.len() + 1),
                    x: *u as i32,
                    y: *v as i32,
                    range: a.range.map_or(0, |c| c.stl * 256 + c.sub),
                    number: None,
                    extra: Default::default(),
                }),
                ClipItem::Light(l) => p.lights.push(PrefabLight {
                    x: *u as i32,
                    y: *v as i32,
                    above: l.z.map_or(0, |c| c.stl * 256 + c.sub),
                    range: l.range.map_or(0, |c| c.stl * 256 + c.sub),
                    intensity: l.intensity.unwrap_or(0),
                    dynamic: l.dynamic.is_some_and(|d| d.is_set()),
                    extra: Default::default(),
                }),
            }
        }
        p.sort_items();
        p.meta.requires = p.used_names();
        p
    }

    /// [`Prefab::to_block`] into the editor's clipboard type.
    pub fn to_clipboard<C: ClipboardLike>(&self, gd: &GameData, owners: &OwnerMap) -> C {
        C::from_block(self.to_block(gd, owners))
    }

    /// [`Prefab::from_block`] from the editor's clipboard type.
    pub fn from_clipboard<C: ClipboardLike>(c: &C, gd: &GameData, name: &str) -> Prefab {
        Prefab::from_block(&c.to_block(), gd, name)
    }
}
