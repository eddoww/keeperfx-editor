//! Capturing a prefab from a map document.
//!
//! Slab kinds, custom slabs, owners, `.wlb` / `.slx`, things, action points and lights of a
//! rectangle (or of a set of selected slabs) become a [`Prefab`]. Like the editor's clipboard,
//! things that belong to a slab (torches, room furniture: `ParentTile` set) and door keys are not
//! captured: applying the prefab re-creates them from the slabs, which is also what makes them
//! follow rotated layouts.
use crate::model::*;
use crate::names::Names;
use crate::script::{self, ScriptKind};
use crate::{Error, Result};
use kfx_map::autotile::{self, SlabRect, PLAYER_NEUTRAL};
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_map::sidecar::{CustomObjectKind, CustomSlab};
use kfx_map::things::{self, thing_class, StlRect, OBJ_SPINNING_KEY};
use std::collections::BTreeSet;

/// What to capture.
#[derive(Clone, Debug)]
pub enum Selection {
    /// Every slab of the rectangle.
    Rect(SlabRect),
    /// Only these slabs (the prefab covers their bounding box; the rest is masked).
    Cells(Vec<(i32, i32)>),
}

impl Selection {
    fn bounds(&self) -> Option<SlabRect> {
        match self {
            Selection::Rect(r) => Some(*r),
            Selection::Cells(c) => {
                let first = c.first()?;
                Some(
                    c.iter()
                        .fold(SlabRect::single(first.0, first.1), |r, &(x, y)| {
                            r.union(SlabRect::single(x, y))
                        }),
                )
            }
        }
    }
    fn contains(&self, x: i32, y: i32) -> bool {
        match self {
            Selection::Rect(r) => x >= r.x0 && x <= r.x1 && y >= r.y0 && y <= r.y1,
            Selection::Cells(c) => c.contains(&(x, y)),
        }
    }
}

/// Options of [`capture`].
#[derive(Clone, Debug)]
pub struct CaptureOptions {
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: Vec<String>,
    /// Treat plain rock / earth / torch earth slabs as not part of the prefab (masked), so a
    /// stamp does not carve rock around the structure.
    pub mask_empty: bool,
    /// Players that keep their number ("keep:N") instead of becoming a relative slot. The hero
    /// player (4) by default.
    pub keep_players: Vec<u8>,
    pub things: bool,
    pub action_points: bool,
    pub lights: bool,
    /// Script fragments to attach (DK script / Lua text using the action point and hero gate
    /// numbers of the document); they are turned into placeholder form.
    pub script_dk: Option<String>,
    pub script_lua: Option<String>,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        CaptureOptions {
            name: "Untitled prefab".into(),
            author: String::new(),
            description: String::new(),
            tags: vec![],
            mask_empty: false,
            keep_players: vec![kfx_map::validate::PLAYER_GOOD],
            things: true,
            action_points: true,
            lights: true,
            script_dk: None,
            script_lua: None,
        }
    }
}

/// Slab kinds `mask_empty` treats as empty (rock, earth, torch earth).
pub fn is_empty_kind(kind: u16) -> bool {
    kind < 256
        && matches!(
            kind as u8,
            autotile::kinds::ROCK | autotile::kinds::EARTH | autotile::kinds::TORCHDIRT
        )
}

fn raw(c: Option<kfx_formats::level_fx::StlCoord>) -> Option<i64> {
    c.map(|c| c.stl as i64 * 256 + c.sub as i64)
}

/// The cube name of a custom slab column cube (`#N` for an unnamed cube).
fn cube_name(gd: &GameData, id: u16) -> String {
    match gd.cube(id as usize) {
        Some(c) if !c.name.is_empty() => c.name.clone(),
        _ => format!("#{id}"),
    }
}

pub(crate) fn custom_to_def(gd: &GameData, names: &Names, c: &CustomSlab) -> CustomSlabDef {
    let columns: Vec<CustomColumn> = c
        .columns
        .iter()
        .map(|col| {
            let mut cubes: [String; 8] = Default::default();
            for (i, q) in col.cubes.iter().enumerate() {
                cubes[i] = cube_name(gd, *q);
            }
            CustomColumn {
                cubes,
                floor: col.floor_texture,
            }
        })
        .collect();
    CustomSlabDef {
        name: c.name.clone(),
        recognized_as: names.slab_name(c.recognized_as),
        liquid: c.liquid,
        columns: columns
            .try_into()
            .unwrap_or_else(|_| unreachable!("nine columns")),
        wibble: c.wibble,
        objects: c
            .objects
            .iter()
            .map(|o| match o.kind {
                CustomObjectKind::Object { model } => CustomObjectDef {
                    kind: "object".into(),
                    model: names.model_name(ThingClass::Object, model as i64),
                    dx: o.dx,
                    dy: o.dy,
                    z: o.z,
                    orientation: o.orientation,
                    range: 0,
                    intensity: 0,
                },
                CustomObjectKind::EffectGen { model, range } => CustomObjectDef {
                    kind: "effectgen".into(),
                    model: names.model_name(ThingClass::EffectGen, model as i64),
                    dx: o.dx,
                    dy: o.dy,
                    z: o.z,
                    orientation: o.orientation,
                    range,
                    intensity: 0,
                },
                CustomObjectKind::Light { intensity, radius } => CustomObjectDef {
                    kind: "light".into(),
                    model: String::new(),
                    dx: o.dx,
                    dy: o.dy,
                    z: o.z,
                    orientation: o.orientation,
                    range: radius,
                    intensity,
                },
            })
            .collect(),
    }
}

/// One document thing as a prefab thing. `origin` is the raw position of the prefab's north-west
/// corner, `floor` the raw floor height under the thing.
pub(crate) fn thing_to_prefab(
    gd: &GameData,
    names: &Names,
    t: &kfx_formats::level_fx::FxThing,
    origin: (i64, i64),
    floor: i64,
    owner: &dyn Fn(i64) -> Owner,
) -> Option<PrefabThing> {
    let class = thing_class(t).and_then(ThingClass::from_id)?;
    let (x, y) = (raw(t.x)?, raw(t.y)?);
    let model = match (&t.subtype_name, t.subtype) {
        (Some(n), _) => n.clone(),
        (None, Some(m)) => names.model_name(class, m),
        _ => return None,
    };
    let mut th = PrefabThing::new(
        class,
        &model,
        owner(t.owner.unwrap_or(PLAYER_NEUTRAL as i64)),
        (x - origin.0) as i32,
        (y - origin.1) as i32,
    );
    // a door's z is its own (a fixed height), not the floor under it
    th.above = if class == ThingClass::Door {
        0
    } else {
        (raw(t.z).unwrap_or(floor) - floor) as i32
    };
    th.orientation = t.orientation;
    th.level = t.creature_level;
    th.gold = t.creature_gold;
    th.health = t.creature_initial_health;
    th.name = t.creature_name.clone();
    th.door_orientation = t.door_orientation;
    th.door_locked = t.door_locked.map(|l| l > 0);
    th.custom_box = t.custom_box;
    th.gold_value = t.gold_value;
    th.effect_range = raw(t.effect_range).map(|v| v as i32);
    if things::is_hero_gate(gd, t) {
        th.gate = Some(format!("gate{}", t.herogate_number.unwrap_or(0)));
    }
    th.extra = t
        .extra
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Some(th)
}

/// Capture `sel` of `doc`. Fails when the selection is empty or outside the map, or larger than
/// [`MAX_SIDE`].
pub fn capture(
    doc: &MapDoc,
    gd: &GameData,
    sel: &Selection,
    opts: &CaptureOptions,
) -> Result<Prefab> {
    let names = Names::new(gd);
    let bounds = sel
        .bounds()
        .ok_or_else(|| Error::Invalid("empty selection".into()))?;
    let b = bounds.clip(doc.dims);
    if b.is_empty() || b != bounds {
        return Err(Error::Invalid(
            "the selection reaches outside the map".into(),
        ));
    }
    let (w, h) = ((b.x1 - b.x0 + 1) as u32, (b.y1 - b.y0 + 1) as u32);
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(Error::Invalid(format!(
            "{w}x{h} slabs: the largest prefab side is {MAX_SIDE}"
        )));
    }
    let mut p = Prefab::blank(&opts.name, w, h);
    p.meta.author = opts.author.clone();
    p.meta.description = opts.description.clone();
    p.meta.tags = opts.tags.clone();

    // players -> slots (ascending player number)
    let mut players: BTreeSet<u8> = BTreeSet::new();
    for (x, y) in b.iter().filter(|&(x, y)| sel.contains(x, y)) {
        players.insert(doc.owners[doc.dims.slab_index(x as usize, y as usize)]);
    }
    let in_sel_slab = |sx: i32, sy: i32| sel.contains(sx.div_euclid(3), sy.div_euclid(3));
    let rect = StlRect::of_slabs(b);
    let wants_thing = |t: &kfx_formats::level_fx::FxThing| -> bool {
        let Some((x, y)) = things::position(t.x, t.y) else {
            return false;
        };
        rect.contains(x, y)
            && in_sel_slab(x, y)
            && t.parent_tile.is_none()
            && t.subtype != Some(OBJ_SPINNING_KEY)
    };
    if opts.things {
        for t in doc.things.items.iter().filter(|t| wants_thing(t)) {
            if let Some(o) = t.owner {
                players.insert(o.clamp(0, 255) as u8);
            }
        }
    }
    players.remove(&PLAYER_NEUTRAL);
    let slot_of: std::collections::BTreeMap<u8, u8> = players
        .iter()
        .filter(|p| !opts.keep_players.contains(p))
        .enumerate()
        .map(|(i, &p)| (p, i as u8))
        .collect();
    let owner = |pl: i64| -> Owner {
        let pl = pl.clamp(0, 255) as u8;
        if pl == PLAYER_NEUTRAL {
            Owner::Neutral
        } else if let Some(&s) = slot_of.get(&pl) {
            Owner::Slot(s)
        } else {
            Owner::Fixed(pl)
        }
    };

    // slabs
    let mut custom_names: Vec<String> = vec![];
    for (x, y) in b.iter() {
        if !sel.contains(x, y) {
            continue;
        }
        let i = doc.dims.slab_index(x as usize, y as usize);
        let kind = doc.kinds[i];
        if opts.mask_empty && is_empty_kind(kind) && doc.custom_cells[i] == 0 {
            continue;
        }
        let mut cell = Cell::new(&names.slab_name(kind), owner(doc.owners[i] as i64));
        if doc.wlb[i] != gd.slab(kind as usize).wlb_type {
            cell.wlb = Some(doc.wlb[i]);
        }
        cell.slx = doc.slx[i];
        if let Some(c) = doc.custom_slab_at(x, y) {
            if !custom_names.contains(&c.name) {
                custom_names.push(c.name.clone());
                let mut def = custom_to_def(gd, &names, c);
                // two definitions with one name must not collide in the file
                if p.custom_slabs.iter().any(|d| d.name == def.name) {
                    def.name = format!("{} #{}", def.name, c.id);
                }
                p.custom_slabs.push(def);
            }
            cell.custom = Some(c.name.clone());
        }
        p.set_cell((x - b.x0) as u32, (y - b.y0) as u32, Some(cell));
    }
    if p.slab_count() == 0 {
        return Err(Error::Invalid(
            "nothing to capture: every slab of the selection is masked".into(),
        ));
    }

    let (ox, oy) = (b.x0 as i64 * 768, b.y0 as i64 * 768);
    let mut ap_keys: Vec<(i64, String)> = vec![];
    let mut gate_keys: Vec<(i64, String)> = vec![];
    if opts.things {
        for t in doc.things.items.iter().filter(|t| wants_thing(t)) {
            let Some(x) = raw(t.x) else { continue };
            let Some(y) = raw(t.y) else { continue };
            let floor = doc.floor_height((x / 256) as i32, (y / 256) as i32) as i64 * 256;
            let Some(th) = thing_to_prefab(gd, &names, t, (ox, oy), floor, &owner) else {
                continue;
            };
            if let Some(g) = &th.gate {
                gate_keys.push((t.herogate_number.unwrap_or(0), g.clone()));
            }
            p.things.push(th);
        }
    }
    if opts.action_points {
        for (i, a) in doc.action_points.items.iter().enumerate() {
            let Some((sx, sy)) = things::position(a.x, a.y) else {
                continue;
            };
            if !rect.contains(sx, sy) || !in_sel_slab(sx, sy) {
                continue;
            }
            let key = match a.number {
                Some(n) => format!("ap{n}"),
                None => format!("ap_i{i}"),
            };
            if p.action_points.iter().any(|q| q.key == key) {
                continue;
            }
            if let Some(n) = a.number {
                ap_keys.push((n, key.clone()));
            }
            p.action_points.push(PrefabAp {
                key,
                x: (raw(a.x).unwrap_or(0) - ox) as i32,
                y: (raw(a.y).unwrap_or(0) - oy) as i32,
                range: raw(a.range).unwrap_or(0) as i32,
                number: a.number,
                extra: a
                    .extra
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            });
        }
    }
    if opts.lights {
        for l in doc.lights.items.iter().filter(|l| l.parent_tile.is_none()) {
            let Some((sx, sy)) = things::position(l.x, l.y) else {
                continue;
            };
            if !rect.contains(sx, sy) || !in_sel_slab(sx, sy) {
                continue;
            }
            let floor = doc.floor_height(sx, sy) as i64 * 256;
            p.lights.push(PrefabLight {
                x: (raw(l.x).unwrap_or(0) - ox) as i32,
                y: (raw(l.y).unwrap_or(0) - oy) as i32,
                above: (raw(l.z).unwrap_or(floor) - floor) as i32,
                range: raw(l.range).unwrap_or(0) as i32,
                intensity: l.intensity.unwrap_or(0),
                dynamic: l.dynamic.is_some_and(|d| d.is_set()),
                extra: l
                    .extra
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            });
        }
    }
    // script fragments -> placeholder form
    let slots: Vec<(u8, char)> = slot_of
        .iter()
        .map(|(&pl, &s)| (pl, (b'A' + s) as char))
        .collect();
    if let Some(s) = opts.script_dk.as_deref().filter(|s| !s.trim().is_empty()) {
        p.scripts.dk = Some(script::placeholderise(
            s,
            ScriptKind::Dk,
            &ap_keys,
            &gate_keys,
            &slots,
        ));
    }
    if let Some(s) = opts.script_lua.as_deref().filter(|s| !s.trim().is_empty()) {
        p.scripts.lua = Some(script::placeholderise(
            s,
            ScriptKind::Lua,
            &ap_keys,
            &gate_keys,
            &slots,
        ));
    }
    p.sort_items();
    p.meta.requires = p.used_names();
    p.check()?;
    Ok(p)
}
