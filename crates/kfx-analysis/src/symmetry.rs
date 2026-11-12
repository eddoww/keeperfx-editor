//! Symmetry detection: which [`SymmetryMode`] the terrain and things satisfy, with the slabs and
//! things breaking it.
//!
//! Slabs are compared by a normalised kind: earth and torch dirt are the same (torch dirt is the
//! game's own decoration of earth, `alter_rock_style` src/map_blocks.c:1613), every fortified wall
//! decoration is "wall", a door slab is its door model (the two orientation kinds rotate into each
//! other). Owners are compared through the owner permutation that most slabs agree on for each
//! transform (mirrored player colours).
use crate::sites::{Sites, ThingKind};
use crate::terrain::{Class, Terrain};
use kfx_formats::level_bin::MapDims;
use kfx_map::gamedata::GameData;
use kfx_map::transform::{Symmetry, SymmetryMode, Transform};

pub const ALL_MODES: [SymmetryMode; 5] = [
    SymmetryMode::NorthSouth,
    SymmetryMode::EastWest,
    SymmetryMode::FourWay,
    SymmetryMode::Rot2,
    SymmetryMode::Rot4,
];

/// One thing whose image is missing.
#[derive(Clone, Debug, PartialEq)]
pub struct ThingBreak {
    /// What (heart / gate / thing description).
    pub what: String,
    pub slab: (i32, i32),
    /// Where its image should be.
    pub expected: (i32, i32),
    /// Index into [`Sites::things`] (`None` for hearts and gates).
    pub site: Option<usize>,
}

/// The result for one mode.
#[derive(Clone, Debug, PartialEq)]
pub struct SymmetryCheck {
    pub mode: SymmetryMode,
    /// Slabs that differ from one of their images (both slabs of a differing pair are listed).
    pub slab_breaks: Vec<(i32, i32)>,
    pub thing_breaks: Vec<ThingBreak>,
    /// Fraction of slabs matching their images (0..1).
    pub match_fraction: f32,
    /// Owner permutation per transform used: `(transform, [owner -> owner; 9])`.
    pub owner_maps: Vec<(Transform, [u8; 9])>,
}

impl SymmetryCheck {
    /// No slab and no thing breaks the symmetry.
    pub fn exact(&self) -> bool {
        self.slab_breaks.is_empty() && self.thing_breaks.is_empty()
    }
    /// The terrain alone is symmetric.
    pub fn terrain_exact(&self) -> bool {
        self.slab_breaks.is_empty()
    }
}

fn norm_key(gd: &GameData, t: &Terrain, x: i32, y: i32) -> u32 {
    let c = t.at(x, y);
    match c.class {
        Class::Earth
            if c.room == 0
                && gd.slab(c.kind as usize).category
                    == kfx_map::gamedata::category::FRIABLE_DIRT =>
        {
            0x1_0000
        }
        Class::Wall
            if gd.slab(c.kind as usize).category == kfx_map::gamedata::category::FORTIFIED_WALL
                && c.room == 0 =>
        {
            0x1_0001
        }
        Class::Door => kfx_map::things::door_of_slab_kind(gd, c.kind)
            .map(|(m, _)| 0x2_0000 + m as u32)
            .unwrap_or(c.kind as u32),
        _ => c.kind as u32,
    }
}

fn transforms(mode: SymmetryMode, w: i32, h: i32) -> Vec<Transform> {
    match mode {
        SymmetryMode::NorthSouth => vec![Transform::FlipY],
        SymmetryMode::EastWest => vec![Transform::FlipX],
        SymmetryMode::FourWay => vec![Transform::FlipX, Transform::FlipY, Transform::Rot180],
        SymmetryMode::Rot2 => vec![Transform::Rot180],
        SymmetryMode::Rot4 if w == h => {
            vec![Transform::Rot90, Transform::Rot180, Transform::Rot270]
        }
        SymmetryMode::Rot4 => vec![],
    }
}

/// A thing's identity for matching: category and model.
fn thing_key(k: &ThingKind) -> String {
    match k {
        ThingKind::Creature { model, .. } => format!("creature {model}"),
        ThingKind::Gold { .. } => "gold".into(),
        ThingKind::Hoard => "hoard".into(),
        ThingKind::Pickup { name, .. } => format!("pickup {name}"),
        ThingKind::Trap { model } => format!("trap {model}"),
    }
}

/// Check one mode (`Rot4` on a non-square map is reported with every slab breaking).
pub fn check(gd: &GameData, t: &Terrain, sites: &Sites, mode: SymmetryMode) -> SymmetryCheck {
    let (w, h) = (t.w, t.h);
    let ts = transforms(mode, w, h);
    if ts.is_empty() {
        return SymmetryCheck {
            mode,
            slab_breaks: vec![],
            thing_breaks: vec![],
            match_fraction: 0.0,
            owner_maps: vec![],
        };
    }
    let keys: Vec<u32> = (0..t.len())
        .map(|i| {
            let (x, y) = t.xy(i);
            norm_key(gd, t, x, y)
        })
        .collect();
    let img = |tr: Transform, x: i32, y: i32| {
        let (a, b) = tr.slab(w as i64, h as i64, x as i64, y as i64);
        (a as i32, b as i32)
    };
    // owner permutation per transform: the most frequent image owner of each owner
    let mut owner_maps = vec![];
    for &tr in &ts {
        let mut m = [[0usize; 9]; 9];
        for i in 0..t.len() {
            let (x, y) = t.xy(i);
            let (a, b) = img(tr, x, y);
            let (o1, o2) = (t.cells[i].owner as usize, t.at(a, b).owner as usize);
            if o1 < 9 && o2 < 9 && keys[i] == keys[t.idx(a, b)] {
                m[o1][o2] += 1;
            }
        }
        let mut map: [u8; 9] = std::array::from_fn(|i| i as u8);
        for (o, row) in m.iter().enumerate() {
            if let Some((best, &n)) = row
                .iter()
                .enumerate()
                .max_by_key(|&(k, &n)| (n, usize::MAX - k))
            {
                if n > 0 {
                    map[o] = best as u8;
                }
            }
        }
        owner_maps.push((tr, map));
    }
    let mut slab_breaks = vec![];
    let mut matched = 0usize;
    for i in 0..t.len() {
        let (x, y) = t.xy(i);
        let mut ok = true;
        for (tr, map) in &owner_maps {
            let (a, b) = img(*tr, x, y);
            let j = t.idx(a, b);
            let o = t.cells[i].owner;
            let mo = map.get(o as usize).copied().unwrap_or(o);
            if keys[i] != keys[j] || t.cells[j].owner != mo {
                ok = false;
                break;
            }
        }
        if ok {
            matched += 1;
        } else {
            slab_breaks.push((x, y));
        }
    }
    // things: hearts, gates and interesting things must have an image of the same kind
    let mut items: Vec<(String, u8, (i32, i32), Option<usize>)> = vec![];
    for hrt in &sites.hearts {
        items.push(("heart".into(), hrt.owner, hrt.slab, None));
    }
    for g in &sites.gates {
        items.push(("hero gate".into(), crate::sites::HEROES, g.slab, None));
    }
    for (k, s) in sites.things.iter().enumerate() {
        items.push((thing_key(&s.kind), s.owner, s.slab, Some(k)));
    }
    let mut thing_breaks = vec![];
    for (what, owner, slab, site) in &items {
        for (tr, map) in &owner_maps {
            let e = img(*tr, slab.0, slab.1);
            let mo = map.get(*owner as usize).copied().unwrap_or(*owner);
            let found = items.iter().any(|(w2, o2, s2, _)| {
                w2 == what && *o2 == mo && (s2.0 - e.0).abs() <= 1 && (s2.1 - e.1).abs() <= 1
            });
            if !found {
                thing_breaks.push(ThingBreak {
                    what: format!("{what} ({})", crate::sites::owner_name(*owner)),
                    slab: *slab,
                    expected: e,
                    site: *site,
                });
                break;
            }
        }
    }
    SymmetryCheck {
        mode,
        slab_breaks,
        thing_breaks,
        match_fraction: matched as f32 / t.len().max(1) as f32,
        owner_maps,
    }
}

/// Check every mode.
pub fn detect(gd: &GameData, t: &Terrain, sites: &Sites) -> Vec<SymmetryCheck> {
    ALL_MODES
        .iter()
        .filter(|&&m| m != SymmetryMode::Rot4 || t.w == t.h)
        .map(|&m| check(gd, t, sites, m))
        .collect()
}

/// The images of slab (x, y) under `mode` (itself first), via [`Symmetry::images`].
pub fn images(mode: SymmetryMode, w: i32, h: i32, x: i32, y: i32) -> Vec<(i32, i32)> {
    Symmetry::new(mode)
        .images(MapDims::new(w as u16, h as u16), x, y)
        .iter()
        .map(|i| (i.x, i.y))
        .collect()
}
