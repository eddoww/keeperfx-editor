//! "Image to map": an image is scaled to the map (one sample per slab) and every sample takes
//! the slab kind and owner of the nearest colour of an editable table.
use crate::assets::{slab_color, PLAYERS};
use eframe::egui::{Color32, ColorImage};
use kfx_gfx::Rgba;
use kfx_map::autotile::{kinds, PLAYER_NEUTRAL};
use kfx_map::gamedata::GameData;

/// One row of the colour table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorRule {
    pub color: [u8; 3],
    pub kind: u16,
    pub owner: u8,
}

/// The default table: the minimap colours of rock, gold, gems, earth, path, water and lava, and
/// claimed ground and fortified walls of the first four keepers.
pub fn default_rules(gd: &GameData) -> Vec<ColorRule> {
    let mut v = vec![];
    let mut add = |kind: u16, owner: u8| {
        v.push(ColorRule {
            color: slab_color(gd, kind, owner),
            kind,
            owner,
        })
    };
    for k in [
        kinds::ROCK,
        kinds::GOLD,
        kinds::GEMS,
        kinds::EARTH,
        kinds::PATH,
        kinds::WATER,
        kinds::LAVA,
    ] {
        if (k as usize) < gd.slab_count() {
            add(k as u16, PLAYER_NEUTRAL);
        }
    }
    let wall = gd
        .slab_id("WALL_DRAPE")
        .or_else(|| gd.slab_id("WALLDRAPE"))
        .unwrap_or(kinds::WALLDRAPE as usize) as u16;
    for o in [0u8, 1, 2, 3] {
        add(kinds::CLAIMED as u16, o);
        add(wall, o);
    }
    // the plain player colours too, mapped to claimed ground
    for o in [0u8, 1, 2, 3] {
        let c = PLAYERS[o as usize].1;
        if !v.iter().any(|r| r.color == c) {
            v.push(ColorRule {
                color: c,
                kind: kinds::CLAIMED as u16,
                owner: o,
            });
        }
    }
    v
}

fn dist2(a: [u8; 3], b: [u8; 3]) -> u32 {
    let d = |i: usize| (a[i] as i32 - b[i] as i32).pow(2) as u32;
    // weighted towards green like the eye (2, 4, 3)
    2 * d(0) + 4 * d(1) + 3 * d(2)
}

/// Index of the nearest rule.
pub fn nearest(rules: &[ColorRule], c: [u8; 3]) -> Option<usize> {
    (0..rules.len()).min_by_key(|&i| dist2(rules[i].color, c))
}

/// The slab (kind, owner) for every slab of a `w` x `h` map, `None` where the image is
/// transparent (alpha < 128): nearest-neighbour sampling at the slab centres.
pub fn map_image(img: &Rgba, w: usize, h: usize, rules: &[ColorRule]) -> Vec<Option<(u16, u8)>> {
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let sx = (((x as f32 + 0.5) / w as f32) * img.w as f32) as u32;
            let sy = (((y as f32 + 0.5) / h as f32) * img.h as f32) as u32;
            let p = img.pixel(
                sx.min(img.w.saturating_sub(1)),
                sy.min(img.h.saturating_sub(1)),
            );
            if p[3] < 128 {
                out.push(None);
                continue;
            }
            out.push(nearest(rules, [p[0], p[1], p[2]]).map(|i| (rules[i].kind, rules[i].owner)));
        }
    }
    out
}

/// The result as a minimap-coloured image (`keep` = the map's current slab where unmapped).
pub fn preview(
    cells: &[Option<(u16, u8)>],
    w: usize,
    h: usize,
    gd: &GameData,
    keep: impl Fn(usize) -> (u16, u8),
) -> ColorImage {
    let px = (0..w * h)
        .map(|i| {
            let (k, o) = cells.get(i).copied().flatten().unwrap_or_else(|| keep(i));
            let c = slab_color(gd, k, o);
            Color32::from_rgb(c[0], c[1], c[2])
        })
        .collect();
    ColorImage::new([w, h], px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_colour_mapping() {
        let rules = vec![
            ColorRule {
                color: [0, 0, 0],
                kind: 0,
                owner: 5,
            },
            ColorRule {
                color: [255, 0, 0],
                kind: 11,
                owner: 0,
            },
            ColorRule {
                color: [0, 0, 255],
                kind: 13,
                owner: 5,
            },
        ];
        let mut img = Rgba::new(4, 1);
        img.put(0, 0, [250, 10, 10, 255]);
        img.put(1, 0, [10, 10, 240, 255]);
        img.put(2, 0, [5, 5, 5, 255]);
        img.put(3, 0, [255, 255, 255, 0]);
        let m = map_image(&img, 4, 1, &rules);
        assert_eq!(m, vec![Some((11, 0)), Some((13, 5)), Some((0, 5)), None]);
        // scaling: a 2x1 map samples the centres of each half
        let m = map_image(&img, 2, 1, &rules);
        assert_eq!(m, vec![Some((13, 5)), None]);
    }
}
