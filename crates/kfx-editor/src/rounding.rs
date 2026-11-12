//! Rounded corners: a corner subtile of a block that touches another terrain on both sides of
//! the corner takes that terrain's column, so rock, earth or gold next to paths or liquids, and
//! water next to lava, look rounded instead of square. A block
//! surrounded on all four sides (a "solo" block) rounds a random half of its corners, and earth
//! and gold next to liquids only round solo blocks.
//!
//! The corner copies the column (and the
//! wibble) of the neighbouring slab's subtile next to it, so it matches that slab's style and
//! decoration. This is decoration only: the game keeps the map's columns as saved
//! (`load_map_data_file`, lvl_filesdk1.c) until a slab is changed in play.
use kfx_map::autotile::kinds;
use kfx_map::autotile::SlabRect;
use kfx_map::mapdoc::MapDoc;
use kfx_prefab::i18n::trd;
use serde::{Deserialize, Serialize};

/// Which corners are rounded (Preferences > Placement).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rounding {
    pub rock_near_liquid: bool,
    pub earth_near_liquid: bool,
    pub gold_near_liquid: bool,
    pub path_near_liquid: bool,
    pub water_near_lava: bool,
    pub rock_near_path: bool,
    pub earth_near_path: bool,
    pub gold_near_path: bool,
}

impl Rounding {
    pub fn any(&self) -> bool {
        *self != Rounding::default()
    }

    /// (option label, field) for the preferences.
    pub fn options(&mut self) -> [(String, &mut bool); 8] {
        [
            (
                trd(
                    "kfx_editor.rock_next_to_water_lava",
                    "Rock next to water / lava",
                ),
                &mut self.rock_near_liquid,
            ),
            (
                trd(
                    "kfx_editor.earth_next_to_water_lava_single_blocks",
                    "Earth next to water / lava (single blocks)",
                ),
                &mut self.earth_near_liquid,
            ),
            (
                trd(
                    "kfx_editor.gold_next_to_water_lava_single_blocks",
                    "Gold next to water / lava (single blocks)",
                ),
                &mut self.gold_near_liquid,
            ),
            (
                trd(
                    "kfx_editor.path_next_to_water_lava",
                    "Path next to water / lava",
                ),
                &mut self.path_near_liquid,
            ),
            (
                trd(
                    "kfx_editor.water_next_to_lava_and_lava_next_to_water",
                    "Water next to lava and lava next to water",
                ),
                &mut self.water_near_lava,
            ),
            (
                trd("kfx_editor.rock_next_to_path", "Rock next to path"),
                &mut self.rock_near_path,
            ),
            (
                trd("kfx_editor.earth_next_to_path", "Earth next to path"),
                &mut self.earth_near_path,
            ),
            (
                trd("kfx_editor.gold_next_to_path", "Gold next to path"),
                &mut self.gold_near_path,
            ),
        ]
    }

    /// The terrains slab kind `k` rounds against, and whether only solo blocks are rounded.
    fn against(&self, k: u8) -> (Vec<u8>, bool) {
        let liquids = [kinds::WATER, kinds::LAVA];
        let mut v = vec![];
        let mut solo_liquid = false;
        match k {
            kinds::ROCK => {
                if self.rock_near_path {
                    v.push(kinds::PATH);
                }
                if self.rock_near_liquid {
                    v.extend(liquids);
                }
            }
            kinds::EARTH => {
                if self.earth_near_path {
                    v.push(kinds::PATH);
                }
                if self.earth_near_liquid {
                    v.extend(liquids);
                    solo_liquid = true;
                }
            }
            kinds::GOLD => {
                if self.gold_near_path {
                    v.push(kinds::PATH);
                }
                if self.gold_near_liquid {
                    v.extend(liquids);
                    solo_liquid = true;
                }
            }
            kinds::PATH if self.path_near_liquid => v.extend(liquids),
            kinds::LAVA if self.water_near_lava => v.push(kinds::WATER),
            kinds::WATER if self.water_near_lava => v.push(kinds::LAVA),
            _ => {}
        }
        (v, solo_liquid)
    }
}

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Round the corners of the slabs of `rect` (after their columns were generated). Writes are
/// declared with `touch_subtile` (safe inside `MapDoc::record_tracked`). Returns the number of
/// corners changed.
pub fn round_corners(d: &mut MapDoc, r: &Rounding, rect: SlabRect, seed: u64) -> usize {
    if !r.any() {
        return 0;
    }
    let dm = d.dims;
    let rect = rect.clip(dm);
    let kind = |d: &MapDoc, x: i32, y: i32| -> Option<u8> {
        (x >= 0 && y >= 0 && x < dm.w as i32 && y < dm.h as i32)
            .then(|| d.kinds[dm.slab_index(x as usize, y as usize)] as u8)
    };
    let mut n = 0;
    for (x, y) in rect.iter() {
        let i = dm.slab_index(x as usize, y as usize);
        if d.custom_cells[i] != 0 {
            continue;
        }
        let Some(k) = kind(d, x, y) else { continue };
        let (against, solo_only) = r.against(k);
        if against.is_empty() {
            continue;
        }
        // N, E, S, W neighbour kinds that are rounding terrain
        let side = |dx: i32, dy: i32| {
            kind(d, x + dx, y + dy).filter(|nk| *nk != k && against.contains(nk))
        };
        let (mut cn, mut ce, mut cs, mut cw) = (side(0, -1), side(1, 0), side(0, 1), side(-1, 0));
        let solo = cn.is_some() && ce.is_some() && cs.is_some() && cw.is_some();
        if solo {
            let h = mix(seed ^ ((x as u64) << 32) ^ y as u64);
            for (b, c) in [(0, &mut cn), (1, &mut ce), (2, &mut cs), (3, &mut cw)] {
                if h >> b & 1 == 1 {
                    *c = None;
                }
            }
        } else if solo_only {
            continue;
        }
        // corner subtile (0..9) <- (neighbour offset, its subtile next to the corner)
        let corners = [
            (cn, cw, 0usize, (0, -1), 6usize),
            (cn, ce, 2, (0, -1), 8),
            (cs, cw, 6, (0, 1), 0),
            (cs, ce, 8, (0, 1), 2),
        ];
        for (a, b, corner, (dx, dy), from) in corners {
            let (Some(a), Some(b)) = (a, b) else { continue };
            if a != b {
                continue;
            }
            let (nx, ny) = (x + dx, y + dy);
            let src = dm.stl_index(3 * nx as usize + from % 3, 3 * ny as usize + from / 3);
            let (cx, cy) = (3 * x as usize + corner % 3, 3 * y as usize + corner / 3);
            let dst = dm.stl_index(cx, cy);
            if d.columns[dst] != d.columns[src] || d.wibble[dst] != d.wibble[src] {
                d.touch_subtile(cx, cy);
                d.columns[dst] = d.columns[src];
                d.wibble[dst] = d.wibble[src];
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rock_corner_next_to_path_takes_the_path_column() {
        let Some((_, d)) = crate::document::tests::load_shipped(1) else {
            return;
        };
        let mut m = d.doc.clone();
        let dm = m.dims;
        // a rock slab with path north and east (and the north-east slab path too)
        let (x, y) = (10i32, 10i32);
        for (dx, dy, k) in [
            (0, 0, kinds::ROCK),
            (0, -1, kinds::PATH),
            (1, 0, kinds::PATH),
            (1, -1, kinds::PATH),
            (-1, 0, kinds::ROCK),
            (0, 1, kinds::ROCK),
        ] {
            m.kinds[dm.slab_index((x + dx) as usize, (y + dy) as usize)] = k as u16;
        }
        let before = m.columns[dm.stl_index(3 * x as usize + 2, 3 * y as usize)];
        let r = Rounding {
            rock_near_path: true,
            ..Default::default()
        };
        let n = round_corners(&mut m, &r, SlabRect::single(x, y), 1);
        let after = m.columns[dm.stl_index(3 * x as usize + 2, 3 * y as usize)];
        let path_col = m.columns[dm.stl_index(3 * x as usize + 2, 3 * (y - 1) as usize + 2)];
        assert_eq!(n, 1);
        assert_eq!(after, path_col);
        assert!(before != after || before == path_col);
        // nothing when the option is off
        let mut m2 = d.doc.clone();
        assert_eq!(
            round_corners(&mut m2, &Rounding::default(), SlabRect::single(x, y), 1),
            0
        );
    }
}
