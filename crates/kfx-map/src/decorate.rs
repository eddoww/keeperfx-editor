//! Decoration randomisation: swap decorative cubes / floor textures for other members of their
//! "variant family" (earth 1-3, path smooth/stones, gold, gems, brick triples, claimed floor, lava
//! surface ...), the way the shipped maps vary them.
//!
//! The game itself never randomises (map_blocks.c has no random choice): the slabset names one
//! cube per position, and a map's `.clm` carries whatever its editor chose. So this is an
//! optional post-pass over columns produced by [`crate::autotile`].
//!
//! # Families are derived from `cubes.cfg`, not hard-coded
//! A family is a run of **consecutive cube ids** (2..=6 members) that
//! * all have ownership group 0 (not player-coloured) and the same `Properties`,
//! * share a name base when named (`EARTH_STD1..3`, `GEMS1..4`; unnamed `CUBE<id>` entries
//!   group with unnamed neighbours) — a cube whose name does not end in a digit may extend a
//!   numbered run as an *extra* member (`PATH_LRGSTONES`, `PATH_SMLSTONES` after
//!   `PATH_SMOOTH1..3`), picked with [`Decorator::extra_rate`],
//! * look alike: on every visible face (north, east, south, west, top; the bottom is never seen
//!   from the game's camera) the members' texture ids form a set of consecutive ids (variant
//!   textures are stored next to each other in the tile atlas), and at least one face differs,
//!
//! **Hard-coded part.** Unnamed cubes (`CUBE<id>`) say nothing about themselves, and texture ids
//! alone cannot tell variants from the parts of a picture (heart / room floor mosaics), and neither
//! "twin columns in the columnset" nor "never stacked in one column" works as evidence: both reject
//! real variants (the original `slabs.clm` stacks random bricks, and has no twin for cube 74). So
//! unnamed families come from [`UNNAMED_FAMILIES`]: the runs the shipped maps vary (measured by the
//! golden test), each still checked against the rules above with the loaded `cubes.cfg` (a config
//! that changes them drops or shrinks them). Cubes 76-78 and 81-83, which shipped maps also swap,
//! are left out: their side textures are mirrored between members, so they are not the same look.
//!
//! Floor-texture families are the top textures of a cube family whose tops differ (path tops
//! 27-31, claimed tops 176-178, lava surfaces 546/547). A floor texture is only swapped on a
//! column with no cube at the bottom (otherwise it is not drawn).
//!
//! Only non-empty cubes are replaced by non-empty family members and floor textures by floor
//! textures, so solid mask, heights and lintel never change.
//!
//! # Determinism
//! The member is chosen by a hash of `(seed, subtile x, subtile y, cube height, family)`, so
//! regenerating a slab gives the same result and undo / redo restore exactly. Use the same seed for
//! the whole life of a map (for example [`Decorator::seed_for_level`]).
use crate::autotile::SlabRect;
use crate::gamedata::GameData;
use crate::mapdoc::MapDoc;
use kfx_formats::level_bin::Column;
use std::collections::{HashMap, HashSet};

/// Cube faces looked at: north, east, south, west, top.
const VISIBLE_FACES: usize = 5;
const MAX_FAMILY: usize = 6;

/// Families derived from the configs but off by default because the shipped maps never vary
/// them (golden test: `WOOD_FLOOR` 439/440 is 0% / 100% over 4716 positions).
pub const DEFAULT_OFF: &[&str] = &["WOOD_FLOOR"];

/// Unnamed variant runs (see the module doc): brick wall, claimed floor, library floor, the
/// bricks of cubes 263-265.
pub const UNNAMED_FAMILIES: &[&[u16]] = &[
    &[72, 73, 74],
    &[126, 127, 128],
    &[174, 175],
    &[263, 264, 265],
];

/// One family of interchangeable cubes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CubeFamily {
    /// Name shown to the user: the name base (`EARTH_STD`, `PATH_SMOOTH`) or `CUBE<first>`.
    pub name: String,
    /// Main members, picked uniformly.
    pub members: Vec<u16>,
    /// Extra members (unnumbered names extending a numbered run), picked with
    /// [`Decorator::extra_rate`].
    pub extra: Vec<u16>,
}

impl CubeFamily {
    pub fn all(&self) -> impl Iterator<Item = u16> + '_ {
        self.members.iter().chain(&self.extra).copied()
    }
}

/// A family of floor textures (the tops of a cube family).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloorFamily {
    pub name: String,
    pub members: Vec<u16>,
    pub extra: Vec<u16>,
}

/// The families derived from a game configuration.
#[derive(Clone, Debug, Default)]
pub struct Families {
    pub cubes: Vec<CubeFamily>,
    pub floors: Vec<FloorFamily>,
    cube_of: HashMap<u16, usize>,
    floor_of: HashMap<u16, usize>,
}

fn name_base(name: &str, id: usize) -> (String, bool) {
    if name.is_empty() || name.eq_ignore_ascii_case(&format!("CUBE{id}")) {
        return (String::new(), true);
    }
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
    (base.to_string(), base.len() < name.len())
}

fn consecutive(vals: &[u16]) -> bool {
    let mut v: Vec<u16> = vals.to_vec();
    v.sort();
    v.dedup();
    (v[v.len() - 1] - v[0]) as usize + 1 == v.len()
}

impl Families {
    /// Derive the families from `gd` (see the module doc for the rules).
    pub fn derive(gd: &GameData) -> Families {
        let ok_cube = |i: usize| {
            gd.cubes.get(i).is_some_and(|c| {
                c.ownership_group == 0 && c.textures.iter().any(|&t| t != 0) && i > 0
            })
        };
        let fits = |run: &[usize]| {
            let p = gd.cubes[run[0]].properties;
            if run.iter().any(|&i| gd.cubes[i].properties != p) {
                return false;
            }
            let mut differs = false;
            for f in 0..VISIBLE_FACES {
                let v: Vec<u16> = run.iter().map(|&i| gd.cubes[i].textures[f]).collect();
                if !consecutive(&v) {
                    return false;
                }
                differs |= v.iter().any(|&t| t != v[0]);
            }
            // every face that varies shifts by the same amount from member to member (a
            // mirrored or rotated texture layout is not the same look)
            let t0 = &gd.cubes[run[0]].textures;
            for &m in &run[1..] {
                let t = &gd.cubes[m].textures;
                let shifts: HashSet<i32> = (0..VISIBLE_FACES)
                    .map(|f| t[f] as i32 - t0[f] as i32)
                    .filter(|&d| d != 0)
                    .collect();
                if shifts.len() > 1 {
                    return false;
                }
            }
            differs
        };
        let mut fams = Families::default();
        let n = gd.cubes.len();
        let mut i = 1;
        while i < n {
            if !ok_cube(i) {
                i += 1;
                continue;
            }
            let (base, numbered) = name_base(&gd.cubes[i].name, i);
            let unnamed = base.is_empty();
            if !numbered {
                i += 1;
                continue;
            }
            let mut run = vec![i];
            let mut j = i + 1;
            while j < n && run.len() < MAX_FAMILY && ok_cube(j) {
                let (b, num) = name_base(&gd.cubes[j].name, j);
                let same = if unnamed {
                    b.is_empty()
                } else {
                    num && b == base
                };
                if !same {
                    break;
                }
                let mut r = run.clone();
                r.push(j);
                if !fits(&r) {
                    break;
                }
                run = r;
                j += 1;
            }
            // unnumbered named cubes right after a numbered run may extend it
            let mut extra = vec![];
            if !unnamed && run.len() >= 2 {
                while j < n && run.len() + extra.len() < MAX_FAMILY && ok_cube(j) {
                    let (b, num) = name_base(&gd.cubes[j].name, j);
                    if num || b.is_empty() {
                        break;
                    }
                    let mut r = run.clone();
                    r.extend(&extra);
                    r.push(j);
                    if !fits(&r) {
                        break;
                    }
                    extra.push(j);
                    j += 1;
                }
            }
            if unnamed {
                // unnamed runs: only the hard-coded ones, cut to what still fits
                match UNNAMED_FAMILIES.iter().find(|f| f[0] as usize == i) {
                    Some(f) => {
                        run.retain(|&c| f.contains(&(c as u16)));
                        j = i + run.len().max(1);
                    }
                    None => run.truncate(1),
                }
            }
            if run.len() >= 2 {
                {
                    let name = if unnamed {
                        format!("CUBE{}", run[0])
                    } else {
                        base.clone()
                    };
                    let members: Vec<u16> = run.iter().map(|&c| c as u16).collect();
                    let extra: Vec<u16> = extra.iter().map(|&c| c as u16).collect();
                    let tops: Vec<u16> = members
                        .iter()
                        .map(|&c| gd.cubes[c as usize].textures[4])
                        .collect();
                    let etops: Vec<u16> = extra
                        .iter()
                        .map(|&c| gd.cubes[c as usize].textures[4])
                        .collect();
                    let mut distinct = tops.clone();
                    distinct.sort();
                    distinct.dedup();
                    if distinct.len() == tops.len() && etops.iter().all(|t| !tops.contains(t)) {
                        fams.floors.push(FloorFamily {
                            name: name.clone(),
                            members: tops,
                            extra: etops,
                        });
                    }
                    fams.cubes.push(CubeFamily {
                        name,
                        members,
                        extra,
                    });
                }
                i = j;
            } else {
                i += 1;
            }
        }
        for (k, f) in fams.cubes.iter().enumerate() {
            for c in f.all() {
                fams.cube_of.insert(c, k);
            }
        }
        for (k, f) in fams.floors.iter().enumerate() {
            for t in f.members.iter().chain(&f.extra) {
                fams.floor_of.entry(*t).or_insert(k);
            }
        }
        fams
    }

    /// Family index of a cube.
    pub fn cube_family(&self, cube: u16) -> Option<usize> {
        self.cube_of.get(&cube).copied()
    }
    /// Family index of a floor texture.
    pub fn floor_family(&self, tex: u16) -> Option<usize> {
        self.floor_of.get(&tex).copied()
    }
}

/// Options of the decoration pass.
#[derive(Clone, Debug)]
pub struct Decorator {
    /// Master switch.
    pub enabled: bool,
    pub seed: u64,
    pub families: Families,
    /// Per cube family (same index as [`Families::cubes`]).
    pub cube_enabled: Vec<bool>,
    /// Per floor family (same index as [`Families::floors`]).
    pub floor_enabled: Vec<bool>,
    /// Probability of picking one of a family's extra members (path stones). The default, 0.25,
    /// is what the shipped maps show (24.6% of path cubes are 28/29, golden test output).
    pub extra_rate: f64,
}

fn mix(mut z: u64) -> u64 {
    // splitmix64
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Decorator {
    /// Everything on, derived from `gd`.
    pub fn new(gd: &GameData, seed: u64) -> Decorator {
        let families = Families::derive(gd);
        let mut d = Decorator {
            enabled: true,
            seed,
            cube_enabled: vec![true; families.cubes.len()],
            floor_enabled: vec![true; families.floors.len()],
            families,
            extra_rate: 0.25,
        };
        for name in DEFAULT_OFF {
            d.set_family(name, false);
        }
        d
    }
    /// A stable seed for a level number.
    pub fn seed_for_level(level: u32) -> u64 {
        mix(level as u64 ^ 0x4B46_584D_4150)
    }
    /// Enable / disable a family by name (cube and floor families of that name).
    pub fn set_family(&mut self, name: &str, on: bool) {
        for (k, f) in self.families.cubes.iter().enumerate() {
            if f.name.eq_ignore_ascii_case(name) {
                self.cube_enabled[k] = on;
            }
        }
        for (k, f) in self.families.floors.iter().enumerate() {
            if f.name.eq_ignore_ascii_case(name) {
                self.floor_enabled[k] = on;
            }
        }
    }

    fn pick(&self, members: &[u16], extra: &[u16], h: u64) -> u16 {
        let u = (h >> 11) as f64 / (1u64 << 53) as f64;
        if !extra.is_empty() && u < self.extra_rate {
            extra[(mix(h) % extra.len() as u64) as usize]
        } else {
            members[(mix(h ^ 0x5555) % members.len() as u64) as usize]
        }
    }

    /// The decorated version of the column at subtile (x, y).
    pub fn column(&self, c: &Column, x: usize, y: usize) -> Column {
        let mut out = *c;
        if !self.enabled {
            return out;
        }
        let base = mix(self.seed ^ ((x as u64) << 20) ^ ((y as u64) << 40));
        for z in 0..8 {
            let cube = c.cubes[z];
            if cube == 0 {
                continue;
            }
            if let Some(k) = self.families.cube_family(cube) {
                if self.cube_enabled[k] {
                    let f = &self.families.cubes[k];
                    out.cubes[z] = self.pick(
                        &f.members,
                        &f.extra,
                        mix(base ^ (z as u64 + 1) ^ ((k as u64) << 8)),
                    );
                }
            }
        }
        if c.cubes[0] == 0 {
            if let Some(k) = self.families.floor_family(c.floor_texture) {
                if self.floor_enabled[k] {
                    let f = &self.families.floors[k];
                    out.floor_texture =
                        self.pick(&f.members, &f.extra, mix(base ^ 0xF100 ^ ((k as u64) << 8)));
                }
            }
        }
        out
    }
}

impl MapDoc {
    /// Apply `dec` to the current columns of the slabs of `rect`. Call it on what was just
    /// regenerated (e.g. the rect returned by [`MapDoc::place_slab`]); it is idempotent and only
    /// depends on the position, so it is safe to call again on the same area.
    pub fn decorate(&mut self, dec: &Decorator, rect: SlabRect) {
        if !dec.enabled {
            return;
        }
        let rect = rect.clip(self.dims);
        for (x, y) in rect.iter() {
            for i in 0..9 {
                let (sx, sy) = (3 * x as usize + i % 3, 3 * y as usize + i / 3);
                let k = self.dims.stl_index(sx, sy);
                if self.custom_slab_at(x, y).is_some() {
                    continue; // hand-built columns are never randomised
                }
                self.j_stl(k);
                self.columns[k] = dec.column(&self.columns[k], sx, sy);
            }
        }
    }

    /// [`MapDoc::regenerate_rect`] followed by [`MapDoc::decorate`].
    pub fn regenerate_rect_decorated(&mut self, gd: &GameData, dec: &Decorator, rect: SlabRect) {
        self.regenerate_rect(gd, rect);
        self.decorate(dec, rect);
    }

    /// Regenerate every slab and decorate it (what [`crate::mapdoc::SaveColumns::Regenerate`]
    /// writes, plus decoration). Use with [`MapDoc::save`] and `SaveColumns::Current`.
    pub fn regenerate_columns_decorated(&mut self, gd: &GameData, dec: &Decorator) {
        let r = SlabRect {
            x0: 0,
            y0: 0,
            x1: self.dims.w as i32 - 1,
            y1: self.dims.h as i32 - 1,
        };
        self.regenerate_rect_decorated(gd, dec, r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_sets() {
        assert!(consecutive(&[3, 2, 4]));
        assert!(consecutive(&[5, 5, 5]));
        assert!(!consecutive(&[548, 552]));
        assert_eq!(name_base("EARTH_STD2", 2), ("EARTH_STD".into(), true));
        assert_eq!(name_base("CUBE72", 72), (String::new(), true));
        assert_eq!(
            name_base("PATH_LRGSTONES", 28),
            ("PATH_LRGSTONES".into(), false)
        );
    }
}
