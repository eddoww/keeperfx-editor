//! Fortify: placing an ownable slab of a keeper turns the earth around it into that keeper's
//! fortified walls, as the game does when imps reinforce
//! (`place_and_process_pretty_wall_slab`, src/map_blocks.c:2556).
//!
//! The wall art follows one of two rules (Preferences: "Fortify wall art"):
//! * [`WallArt::Grouped`]: the game's `choose_pretty_type` (src/map_blocks.c:2579): torch walls every
//!   fifth slab and drapes every other slab beside claimed ground, else twins / woman / pair
//!   bands by the distance from the keeper's dungeon heart (the map centre without one). The game
//!   measures that distance with `LbDiagonalLength` (an octagonal approximation,
//!   src/bflib_math.c:849); the Euclidean distance is used here, so a wall on a band edge may get
//!   the neighbouring band's art.
//! * [`WallArt::Random`]: one of the five wall kinds at random (seeded per slab), like the
//!   game's terrain cheat (src/packets_cheats.c:422).
use crate::document::Edit;
use kfx_map::autotile::{kinds, PLAYER_NEUTRAL};
use kfx_map::gamedata::{category, GameData};
use kfx_map::mapdoc::MapDoc;
use kfx_map::things::{self, class};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WallArt {
    #[default]
    Grouped,
    Random,
}

/// The game's `choose_pretty_type` for slab (x, y); `kind` gives the slab kinds (after the
/// edit), `heart` the keeper's heart slab.
pub fn pretty_wall(kind: &dyn Fn(i32, i32) -> u8, x: i32, y: i32, heart: (i32, i32)) -> u8 {
    let claimed = |a: (i32, i32), b: (i32, i32)| {
        kind(a.0, a.1) == kinds::CLAIMED || kind(b.0, b.1) == kinds::CLAIMED
    };
    if x % 5 == 0 && claimed((x, y - 1), (x, y + 1)) {
        return kinds::WALLTORCH;
    }
    if y % 5 == 0 && claimed((x - 1, y), (x + 1, y)) {
        return kinds::WALLTORCH;
    }
    if x & 1 != 0 && claimed((x, y - 1), (x, y + 1)) {
        return kinds::WALLDRAPE;
    }
    if y & 1 != 0 && claimed((x - 1, y), (x + 1, y)) {
        return kinds::WALLDRAPE;
    }
    let d = (((x - heart.0) as f32).powi(2) + ((y - heart.1) as f32).powi(2)).sqrt() as i32;
    [kinds::WALLWTWINS, kinds::WALLWWOMAN, kinds::WALLPAIRSHR][((d / 4) % 3) as usize]
}

/// Slab of the dungeon heart of `owner` (an object with the `HEART` property, objects.cfg).
pub fn heart_of(d: &MapDoc, gd: &GameData, owner: u8) -> Option<(i32, i32)> {
    d.things.items.iter().find_map(|t| {
        if things::thing_class(t) != Some(class::OBJECT) || t.owner != Some(owner as i64) {
            return None;
        }
        let sec = gd.objects_cfg.section(&format!("object{}", t.subtype?))?;
        if !sec.get("Properties").is_some_and(|p| {
            p.split_whitespace()
                .any(|w| w.eq_ignore_ascii_case("HEART"))
        }) {
            return None;
        }
        things::position(t.x, t.y).map(|(sx, sy)| (sx / 3, sy / 3))
    })
}

fn is_wall(gd: &GameData, k: u16) -> bool {
    gd.slab(k as usize).category == category::FORTIFIED_WALL
}

fn is_earth(k: u16) -> bool {
    k == kinds::EARTH as u16 || k == kinds::TORCHDIRT as u16
}

/// The wall edits that fortify around `edits` (slab placements), appended by the caller.
/// For each placed ownable, non-wall slab of a keeper, the earth on its four sides becomes a
/// wall of that keeper, and the two slabs beside such a side too (the corners), unless that side
/// is neither earth nor a wall.
pub fn fortify_edits(
    d: &MapDoc,
    gd: &GameData,
    edits: &[(i32, i32, Edit)],
    art: WallArt,
    seed: u64,
) -> Vec<(i32, i32, Edit)> {
    let dm = d.dims;
    let inside = |x: i32, y: i32| x >= 0 && y >= 0 && x < dm.w as i32 && y < dm.h as i32;
    let placed: HashMap<(i32, i32), u16> = edits
        .iter()
        .filter_map(|&(x, y, e)| match e {
            Edit::Place { kind, .. } => Some(((x, y), kind)),
            _ => None,
        })
        .collect();
    let kind_now = |x: i32, y: i32| -> u16 {
        if let Some(k) = placed.get(&(x, y)) {
            return *k;
        }
        if !inside(x, y) {
            return kinds::ROCK as u16;
        }
        d.kinds[dm.slab_index(x as usize, y as usize)]
    };
    let mut walls: Vec<(i32, i32, u8)> = vec![];
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    for &(x, y, e) in edits {
        let Edit::Place { kind, owner } = e else {
            continue;
        };
        if owner == PLAYER_NEUTRAL || gd.slab(kind as usize).is_ownable == 0 || is_wall(gd, kind) {
            continue;
        }
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let side = (x + dx, y + dy);
            if placed.contains_key(&side) {
                continue;
            }
            // the side, then the two beside it (perpendicular)
            let (px, py) = (dy.abs(), dx.abs());
            for (i, p) in [side, (side.0 + px, side.1 + py), (side.0 - px, side.1 - py)]
                .into_iter()
                .enumerate()
            {
                if !inside(p.0, p.1) || placed.contains_key(&p) {
                    continue;
                }
                let k = kind_now(p.0, p.1);
                if is_earth(k) {
                    if seen.insert(p) {
                        walls.push((p.0, p.1, owner));
                    }
                } else if i == 0 && !is_wall(gd, k) {
                    break;
                }
            }
        }
    }
    let k8 = |x: i32, y: i32| kind_now(x, y) as u8;
    let mut hearts: HashMap<u8, (i32, i32)> = HashMap::new();
    walls
        .into_iter()
        .map(|(x, y, owner)| {
            let kind = match art {
                WallArt::Grouped => {
                    let h = *hearts
                        .entry(owner)
                        .or_insert_with(|| heart_of(d, gd, owner).unwrap_or((42, 42)));
                    pretty_wall(&k8, x, y, h)
                }
                WallArt::Random => {
                    let h = crate::doodads::mix_pub(seed ^ ((x as u64) << 32) ^ y as u64);
                    kinds::WALLDRAPE + (h % 5) as u8
                }
            };
            (
                x,
                y,
                Edit::Place {
                    kind: kind as u16,
                    owner,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_wall_follows_the_game() {
        // claimed ground south of a slab at x = 10 (divisible by 5): torch wall
        let k = |x: i32, y: i32| {
            if (x, y) == (10, 6) {
                kinds::CLAIMED
            } else {
                kinds::EARTH
            }
        };
        assert_eq!(pretty_wall(&k, 10, 5, (0, 0)), kinds::WALLTORCH);
        // odd x beside claimed ground: drape
        let k = |x: i32, y: i32| {
            if (x, y) == (11, 6) {
                kinds::CLAIMED
            } else {
                kinds::EARTH
            }
        };
        assert_eq!(pretty_wall(&k, 11, 5, (0, 0)), kinds::WALLDRAPE);
        // no claimed ground: bands of 4 slabs from the heart
        let k = |_: i32, _: i32| kinds::EARTH;
        assert_eq!(pretty_wall(&k, 2, 0, (0, 0)), kinds::WALLWTWINS);
        assert_eq!(pretty_wall(&k, 6, 0, (0, 0)), kinds::WALLWWOMAN);
        assert_eq!(pretty_wall(&k, 8, 0, (0, 0)), kinds::WALLPAIRSHR);
    }

    #[test]
    fn fortify_walls_the_earth_around_claimed_ground() {
        let Some((_, d)) = crate::document::tests::load_shipped(1) else {
            return;
        };
        let gd = d.gd.clone();
        // find an earth slab with earth all around
        let dm = d.doc.dims;
        let earth = |x: i32, y: i32| is_earth(d.doc.kinds[dm.slab_index(x as usize, y as usize)]);
        let (x, y) = (2..dm.h as i32 - 2)
            .flat_map(|y| (2..dm.w as i32 - 2).map(move |x| (x, y)))
            .find(|&(x, y)| (-1..=1).all(|dy| (-1..=1).all(|dx| earth(x + dx, y + dy))))
            .unwrap();
        let edits = vec![(
            x,
            y,
            Edit::Place {
                kind: kinds::CLAIMED as u16,
                owner: 0,
            },
        )];
        let w = fortify_edits(&d.doc, &gd, &edits, WallArt::Grouped, 1);
        // the four sides and the four corners
        assert_eq!(w.len(), 8, "{w:?}");
        for (_, _, e) in &w {
            let Edit::Place { kind, owner } = *e else {
                panic!()
            };
            assert!(is_wall(&gd, kind) && owner == 0, "{e:?}");
        }
        // neutral claimed ground fortifies nothing
        let edits = vec![(
            x,
            y,
            Edit::Place {
                kind: kinds::CLAIMED as u16,
                owner: PLAYER_NEUTRAL,
            },
        )];
        assert!(fortify_edits(&d.doc, &gd, &edits, WallArt::Random, 1).is_empty());
    }
}
