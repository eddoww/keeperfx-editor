//! CPU side of the terrain texture atlas.
//!
//! One array layer per loaded texture set (1024 x 1088 px: 32 x 34 tiles; tiles 0..543 are the
//! set's tmapa, 544..1087 its tmapb), with a box-filtered mip chain computed per tile so that
//! no mip level mixes neighbouring tiles. The shader fetches texels with `textureLoad` inside
//! the tile, so there is no filtering across tile borders at all (no bleeding).
//!
//! A lookup table maps each global slot (`variation * 1544 + texture id`) and animation frame
//! to a physical tile `layer * 1088 + tile`. Variation slot 0 holds the level's `.inf` set and
//! slot n holds set n-1 (`src/engine_textures.c:227-250`); animated ids 544..999 resolve per frame
//! through `textureanim.toml` within the same variation (`update_animating_texture_maps`, :92-117).
use crate::mesh::{BLOCKS_PER_VARIATION, VARIATIONS};
use crate::terrain::variation_set;
use kfx_gfx::textures::{
    static_tile, Part, TextureAnim, TextureSearch, TextureSet, ANIM_FRAMES, MISSING_FILL,
    TILES_PER_FILE,
};
use kfx_gfx::Palette;
use kfx_prefab::i18n::trf;

pub const LAYER_W: u32 = 1024;
pub const LAYER_H: u32 = 1088;
pub const LAYER_TILES: u32 = 1088;
pub const TILES_PER_ROW: u32 = 32;
pub const MIP_LEVELS: u32 = 6;

pub struct AtlasData {
    /// Per layer, per mip level: RGBA8 pixels.
    pub layers: Vec<Vec<Vec<u8>>>,
    /// `lookup[(frame * 32 + variation) * 1544 + id]` = physical tile.
    pub lookup: Vec<u32>,
    /// Set index of each layer (for display/debug).
    pub layer_sets: Vec<u32>,
    pub warnings: Vec<String>,
}

fn layer_pixels(set: Option<&TextureSet>, pal: &Palette) -> Vec<u8> {
    let mut out = vec![0u8; (LAYER_W * LAYER_H * 4) as usize];
    let fill = pal.rgba(MISSING_FILL);
    for tile in 0..LAYER_TILES {
        let (part, t) = if tile < TILES_PER_FILE as u32 {
            (Part::A, tile)
        } else {
            (Part::B, tile - TILES_PER_FILE as u32)
        };
        let idx = set.and_then(|s| {
            s.tile_indexed(kfx_gfx::TileRef {
                part,
                tile: t as u16,
            })
        });
        let (tx, ty) = ((tile % TILES_PER_ROW) * 32, (tile / TILES_PER_ROW) * 32);
        for y in 0..32u32 {
            for x in 0..32u32 {
                let c = match &idx {
                    Some(v) => pal.rgba(v[(y * 32 + x) as usize]),
                    None => fill,
                };
                let o = (((ty + y) * LAYER_W + tx + x) * 4) as usize;
                out[o..o + 4].copy_from_slice(&c);
            }
        }
    }
    out
}

/// 2x2 box filter of a level whose width/height are multiples of the tile size, so every
/// output texel averages 4 texels of the same tile.
fn downsample(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (nw, nh) = (w / 2, h / 2);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            for c in 0..4 {
                let p = |xx: u32, yy: u32| src[((yy * w + xx) * 4 + c) as usize] as u32;
                let s = p(2 * x, 2 * y)
                    + p(2 * x + 1, 2 * y)
                    + p(2 * x, 2 * y + 1)
                    + p(2 * x + 1, 2 * y + 1);
                out[((y * nw + x) * 4 + c) as usize] = ((s + 2) / 4) as u8;
            }
        }
    }
    out
}

pub fn mip_chain(base: Vec<u8>) -> Vec<Vec<u8>> {
    let mut levels = vec![base];
    let (mut w, mut h) = (LAYER_W, LAYER_H);
    for _ in 1..MIP_LEVELS {
        let next = downsample(levels.last().unwrap(), w, h);
        w /= 2;
        h /= 2;
        levels.push(next);
    }
    levels
}

/// Physical tile of a static texture id in `layer`; `None` for animated / out-of-range ids.
pub fn physical(layer: u32, id: u16) -> Option<u32> {
    let t = static_tile(id)?;
    let local = match t.part {
        Part::A => t.tile as u32,
        Part::B => TILES_PER_FILE as u32 + t.tile as u32,
    };
    Some(layer * LAYER_TILES + local)
}

/// Build the lookup table for the given variation -> layer assignment.
pub fn build_lookup(var_layer: &[u32; 32], anim: &TextureAnim) -> Vec<u32> {
    let per_frame = (VARIATIONS * BLOCKS_PER_VARIATION) as usize;
    let mut lut = vec![0u32; per_frame * ANIM_FRAMES];
    for frame in 0..ANIM_FRAMES {
        for v in 0..VARIATIONS as usize {
            let layer = var_layer[v];
            for id in 0..BLOCKS_PER_VARIATION as u16 {
                let phys = match anim.frames(id) {
                    Some(f) => physical(layer, f[frame]).unwrap_or(layer * LAYER_TILES),
                    None => physical(layer, id).unwrap_or(layer * LAYER_TILES),
                };
                lut[frame * per_frame + v * BLOCKS_PER_VARIATION as usize + id as usize] = phys;
            }
        }
    }
    lut
}

/// Load the sets a level uses (`used_variations`) and build atlas layers + lookup.
pub fn build(
    search: &TextureSearch,
    lvnum: Option<u32>,
    level_set: u32,
    used_variations: &[u8],
    pal: &Palette,
    anim: &TextureAnim,
) -> AtlasData {
    let mut layer_sets: Vec<u32> = vec![];
    let mut layers = vec![];
    let mut warnings = vec![];
    let mut var_layer = [0u32; 32];
    for &v in used_variations {
        let set = variation_set(v, level_set);
        let layer = match layer_sets.iter().position(|&s| s == set) {
            Some(l) => l as u32,
            None => {
                let ts = match TextureSet::load(search, set, lvnum) {
                    Ok(ts) => Some(ts),
                    Err(e) => {
                        warnings.push(trf(
                            "kfx_editor.texture_set_set_e",
                            "texture set {set}: {e}",
                            &[("set", &set), ("e", &e)],
                        ));
                        None
                    }
                };
                layers.push(mip_chain(layer_pixels(ts.as_ref(), pal)));
                layer_sets.push(set);
                (layer_sets.len() - 1) as u32
            }
        };
        var_layer[v as usize] = layer;
    }
    AtlasData {
        lookup: build_lookup(&var_layer, anim),
        layers,
        layer_sets,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_tiles() {
        assert_eq!(physical(0, 0), Some(0));
        assert_eq!(physical(0, 543), Some(543));
        assert_eq!(physical(0, 600), None);
        assert_eq!(physical(0, 1000), Some(544));
        assert_eq!(physical(2, 1543), Some(2 * 1088 + 1087));
    }

    #[test]
    fn lookup_animates_within_variation() {
        let anim =
            TextureAnim::parse("[texture544]\nframes = [1, 2, 3, 4, 5, 6, 7, 1001]\n").unwrap();
        let mut vl = [0u32; 32];
        vl[3] = 5;
        let lut = build_lookup(&vl, &anim);
        let per = (VARIATIONS * BLOCKS_PER_VARIATION) as usize;
        let at = |f: usize, v: usize, id: usize| lut[f * per + v * 1544 + id];
        assert_eq!(at(0, 0, 544), 1);
        assert_eq!(at(2, 0, 544), 3);
        assert_eq!(at(7, 0, 544), 545);
        assert_eq!(at(1, 3, 544), 5 * 1088 + 2);
        assert_eq!(at(0, 3, 17), 5 * 1088 + 17);
        assert_eq!(at(0, 3, 1010), 5 * 1088 + 554);
    }

    #[test]
    fn mips_stay_inside_tiles() {
        // two neighbouring tiles of different colour never blend at any mip level
        let mut base = vec![0u8; (LAYER_W * LAYER_H * 4) as usize];
        for y in 0..32 {
            for x in 32..64 {
                let o = ((y * LAYER_W + x) * 4) as usize;
                base[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let chain = mip_chain(base);
        for (k, lvl) in chain.iter().enumerate() {
            let w = LAYER_W >> k;
            let ts = 32 >> k;
            for x in 0..ts {
                assert_eq!(lvl[(x * 4) as usize], 0, "level {k}");
                assert_eq!(lvl[((ts + x) * 4) as usize], 255, "level {k}");
            }
            assert_eq!(lvl.len(), (w * (LAYER_H >> k) * 4) as usize);
        }
    }
}
