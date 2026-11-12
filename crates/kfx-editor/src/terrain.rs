//! Terrain as the renderer and picker see it: one column per drawn subtile, copied from the
//! document's current column grid (`MapDoc::columns`), plus the per-slab texture variation
//! and owner.
use kfx_formats::level_bin::Column;
use kfx_map::autotile::SlabRect;
use kfx_map::mapdoc::MapDoc;

#[derive(Clone, Debug, Default)]
pub struct Terrain {
    /// Drawn subtile grid size, `3W x 3H` (the extra row/column of the files is not drawn).
    pub stl_w: usize,
    pub stl_h: usize,
    /// Column of every subtile, `stl_w * stl_h`, row-major.
    pub columns: Vec<Column>,
    /// `.slx` texture variation per slab (low 5 bits), `W * H`.
    pub slab_variation: Vec<u8>,
    /// Owner per slab.
    pub slab_owner: Vec<u8>,
    pub slab_w: usize,
}

static EMPTY: Column = Column {
    use_count: 0,
    bitfields: 0,
    solidmask: 0,
    floor_texture: 0,
    orient: 0,
    cubes: [0; 8],
};

impl Terrain {
    pub fn from_doc(doc: &MapDoc) -> Terrain {
        let d = doc.dims;
        let (sw, sh) = (d.stl_w(), d.stl_h());
        let mut t = Terrain {
            stl_w: sw,
            stl_h: sh,
            columns: vec![Column::default(); sw * sh],
            slab_variation: doc.slx.clone(),
            slab_owner: doc.owners.clone(),
            slab_w: d.w as usize,
        };
        for y in 0..sh {
            for x in 0..sw {
                t.columns[y * sw + x] = doc.columns[d.stl_index(x, y)];
            }
        }
        t
    }

    /// Copy the slabs of `r` from the document again.
    pub fn update(&mut self, doc: &MapDoc, r: SlabRect) {
        let d = doc.dims;
        let r = r.clip(d);
        if r.is_empty() {
            return;
        }
        for (sx, sy) in r.iter() {
            let si = d.slab_index(sx as usize, sy as usize);
            self.slab_variation[si] = doc.slx[si];
            self.slab_owner[si] = doc.owners[si];
            for k in 0..9 {
                let (x, y) = (3 * sx as usize + k % 3, 3 * sy as usize + k / 3);
                self.columns[y * self.stl_w + x] = doc.columns[d.stl_index(x, y)];
            }
        }
    }

    pub fn column_at(&self, x: usize, y: usize) -> &Column {
        if x >= self.stl_w || y >= self.stl_h {
            return &EMPTY;
        }
        &self.columns[y * self.stl_w + x]
    }

    /// Solid mask at subtile (x, y); out of range is empty (0).
    pub fn solidmask(&self, x: isize, y: isize) -> u16 {
        if x < 0 || y < 0 {
            return 0;
        }
        self.column_at(x as usize, y as usize).solidmask
    }

    /// Height (in cubes) of the top of the highest solid cube; 0 for a flat floor.
    pub fn top_height(&self, x: usize, y: usize) -> usize {
        let m = self.column_at(x, y).solidmask & 0xFF;
        16 - m.leading_zeros() as usize
    }

    /// Highest top over the 9 subtiles of slab (sx, sy).
    pub fn slab_top(&self, sx: usize, sy: usize) -> usize {
        let mut h = 0;
        for y in sy * 3..sy * 3 + 3 {
            for x in sx * 3..sx * 3 + 3 {
                h = h.max(self.top_height(x, y));
            }
        }
        h
    }

    /// Texture variation (0 = level set, n = set n-1) of the slab holding subtile (x, y)
    /// (`src/engine_render.c:3965-3971` `engine_remap_texture_blocks`).
    pub fn variation_at(&self, x: usize, y: usize) -> u8 {
        let (sx, sy) = (x / 3, y / 3);
        self.slab_variation
            .get(sy * self.slab_w + sx)
            .copied()
            .unwrap_or(0)
            & 0x1F
    }

    /// Owner of the slab holding subtile (x, y); 255 outside.
    pub fn owner_at(&self, x: usize, y: usize) -> u8 {
        let (sx, sy) = (x / 3, y / 3);
        if sx >= self.slab_w {
            return 255;
        }
        self.slab_owner
            .get(sy * self.slab_w + sx)
            .copied()
            .unwrap_or(255)
    }

    /// Texture sets the terrain uses: slot 0 (the `.inf` set) and every `.slx` value present.
    pub fn used_variations(&self) -> Vec<u8> {
        let mut used = [false; 32];
        used[0] = true;
        for v in &self.slab_variation {
            used[(*v & 0x1F) as usize] = true;
        }
        (0..32u8).filter(|&i| used[i as usize]).collect()
    }
}

/// Set index loaded into variation slot `v` for a level whose `.inf` set is `level_set`
/// (`src/engine_textures.c:227-250` `load_texture_map_file`).
pub fn variation_set(v: u8, level_set: u32) -> u32 {
    if v == 0 {
        level_set
    } else {
        v as u32 - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variations() {
        assert_eq!(variation_set(0, 3), 3);
        assert_eq!(variation_set(1, 3), 0);
        assert_eq!(variation_set(14, 3), 13);
    }

    #[test]
    fn from_doc_matches_document_columns() {
        let Some((_, d)) = crate::document::tests::load_shipped(1) else {
            return;
        };
        let t = Terrain::from_doc(&d.doc);
        let dims = d.doc.dims;
        assert_eq!(t.stl_w, dims.stl_w());
        for (x, y) in [(0, 0), (100, 50), (254, 254)] {
            assert_eq!(*t.column_at(x, y), d.doc.columns[dims.stl_index(x, y)]);
        }
        assert_eq!(
            t.owner_at(3 * 10 + 1, 3 * 20 + 2),
            d.doc.owners[dims.slab_index(10, 20)]
        );
    }
}
