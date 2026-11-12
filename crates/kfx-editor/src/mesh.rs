//! CPU mesh building for the terrain, straight from the map's columns.
//!
//! Follows `do_a_plane_of_engine_columns_perspective` (`src/engine_render.c:4036-4170`):
//! * every solid cube (bit of the column's `solidmask`, `cubes[0]` at the bottom) gets a side
//!   face towards each neighbour subtile whose solidmask lacks the same bit;
//! * face textures come from the cube's `Textures` (N E S W Top Bottom) through `sideoris[0]`:
//!   the north (-y) face uses N, east (+x) E, south (+y) S, west (-x) W;
//! * the walkable surface is the top of the cube below the first gap
//!   (`floor_height_table`), or the column's `floor_texture` at height 0 when the bottom cube is
//!   empty; a solid run above the gap (lintel) gets its top face and the bottom face of the same
//!   cube (`lintel_top_height` / `lintel_bottom_height`);
//! * every texture id is offset by `(slx & 0x1F) * 1544` (`engine_remap_texture_blocks`, :3965).
//!
//! Texture orientation (`do_a_trig_gourad_tr/bl` UVs, :3183-3356): U runs left-to-right as seen
//! from outside the face, V from the top edge (0) down; on top faces U runs along +x and V along
//! +y (north edge = V 0); on lintel bottoms V runs from +y (0) to -y.
use crate::assets::{CubeFaces, FACE_BOTTOM, FACE_E, FACE_N, FACE_S, FACE_TOP, FACE_W};
use crate::terrain::Terrain;

/// Texture slots per variation (`TEXTURE_BLOCKS_COUNT`, src/engine_textures.h:43).
pub const BLOCKS_PER_VARIATION: u32 = 1544;
pub const VARIATIONS: u32 = 32;
/// Chunk edge in subtiles (8 slabs).
pub const CHUNK: usize = 24;

/// Face direction codes stored in [`Vertex::face`].
pub const DIR_TOP: u8 = 0;
pub const DIR_N: u8 = 1;
pub const DIR_E: u8 = 2;
pub const DIR_S: u8 = 3;
pub const DIR_W: u8 = 4;
pub const DIR_BOTTOM: u8 = 5;

/// 12-byte terrain vertex. Positions are integer subtile/cube coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub x: u16,
    pub z: u16,
    /// Height in cubes.
    pub h: u8,
    /// bit 0: u = 1, bit 1: v = 1.
    pub uv: u8,
    pub face: u8,
    /// Owner of the slab (for the ownership tint), 255 = none.
    pub owner: u8,
    /// Global texture slot `variation * 1544 + texture id`.
    pub slot: u32,
}

#[derive(Default, Clone, Debug)]
pub struct ChunkMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl ChunkMesh {
    #[cfg(test)]
    pub fn quads(&self) -> usize {
        self.vertices.len() / 4
    }
    #[cfg(test)]
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// Emit a quad; corners are (x, h, z, u, v) in counter-clockwise order seen from outside.
    fn quad(&mut self, c: [(usize, usize, usize, u8, u8); 4], face: u8, slot: u32, owner: u8) {
        let base = self.vertices.len() as u32;
        for (x, h, z, u, v) in c {
            self.vertices.push(Vertex {
                x: x as u16,
                z: z as u16,
                h: h as u8,
                uv: u | (v << 1),
                face,
                owner,
                slot,
            });
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Global slot for texture `tex` in variation `var`, wrapping into the next variation like the
/// game's flat `block_ptrs` array does; out of range gives slot 0.
pub fn slot(var: u8, tex: u16) -> u32 {
    let g = var as u32 * BLOCKS_PER_VARIATION + tex as u32;
    if g < VARIATIONS * BLOCKS_PER_VARIATION {
        g
    } else {
        0
    }
}

/// Heights derived from a solid mask like `src/engine_arrays.c:1010-1044`:
/// (floor height, lintel bottom, lintel top). Lintel top 0 = no lintel.
pub fn column_heights(mask: u16) -> (usize, usize, usize) {
    let m = (mask & 0xFF) as u32;
    let mut i = 0;
    while i < 8 && m & (1 << i) != 0 {
        i += 1;
    }
    let floor = i;
    while i < 8 && m & (1 << i) == 0 {
        i += 1;
    }
    if i < 8 {
        let btm = i;
        while i < 8 && m & (1 << i) != 0 {
            i += 1;
        }
        (floor, btm, i)
    } else {
        (floor, 0, 0)
    }
}

/// Build the mesh of the subtile rectangle `[x0, x1) x [y0, y1)`.
pub fn build_region(
    t: &Terrain,
    cubes: &CubeFaces,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
) -> ChunkMesh {
    let mut m = ChunkMesh::default();
    for y in y0..y1.min(t.stl_h) {
        for x in x0..x1.min(t.stl_w) {
            build_subtile(t, cubes, x, y, &mut m);
        }
    }
    m
}

/// Build chunk (cx, cy) of [`CHUNK`]-sized chunks.
pub fn build_chunk(t: &Terrain, cubes: &CubeFaces, cx: usize, cy: usize) -> ChunkMesh {
    build_region(
        t,
        cubes,
        cx * CHUNK,
        cy * CHUNK,
        (cx + 1) * CHUNK,
        (cy + 1) * CHUNK,
    )
}

pub fn chunk_counts(t: &Terrain) -> (usize, usize) {
    (t.stl_w.div_ceil(CHUNK), t.stl_h.div_ceil(CHUNK))
}

/// Chunk indices (row-major) to rebuild after the slabs of `r` changed: the chunks holding
/// those subtiles plus one subtile around them, whose side faces are culled against the
/// changed columns.
pub fn chunks_for_slabs(t: &Terrain, r: kfx_map::autotile::SlabRect) -> Vec<usize> {
    if r.is_empty() {
        return vec![];
    }
    let (cw, ch) = chunk_counts(t);
    let x0 = (r.x0.max(0) as usize * 3).saturating_sub(1) / CHUNK;
    let y0 = (r.y0.max(0) as usize * 3).saturating_sub(1) / CHUNK;
    let x1 = ((r.x1.max(0) as usize * 3 + 3).min(t.stl_w.saturating_sub(1)) / CHUNK)
        .min(cw.saturating_sub(1));
    let y1 = ((r.y1.max(0) as usize * 3 + 3).min(t.stl_h.saturating_sub(1)) / CHUNK)
        .min(ch.saturating_sub(1));
    let mut v = vec![];
    for cy in y0..=y1 {
        for cx in x0..=x1 {
            v.push(cy * cw + cx);
        }
    }
    v
}

fn build_subtile(t: &Terrain, cubes: &CubeFaces, x: usize, y: usize, m: &mut ChunkMesh) {
    let col = t.column_at(x, y);
    let var = t.variation_at(x, y);
    let own = t.owner_at(x, y);
    let mask = col.solidmask & 0xFF;
    let (xi, yi) = (x as isize, y as isize);
    let n_mask = t.solidmask(xi, yi - 1);
    let s_mask = t.solidmask(xi, yi + 1);
    let w_mask = t.solidmask(xi - 1, yi);
    let e_mask = t.solidmask(xi + 1, yi);
    let (x1, z0, z1) = (x + 1, y, y + 1);
    for h in 0..8usize {
        let bit = 1u16 << h;
        if mask & bit == 0 {
            continue;
        }
        let cube = col.cubes[h];
        let (b, tp) = (h, h + 1);
        if n_mask & bit == 0 {
            let s = slot(var, cubes.face(cube, FACE_N));
            m.quad(
                [
                    (x1, tp, z0, 0, 0),
                    (x1, b, z0, 0, 1),
                    (x, b, z0, 1, 1),
                    (x, tp, z0, 1, 0),
                ],
                DIR_N,
                s,
                own,
            );
        }
        if s_mask & bit == 0 {
            let s = slot(var, cubes.face(cube, FACE_S));
            m.quad(
                [
                    (x, tp, z1, 0, 0),
                    (x, b, z1, 0, 1),
                    (x1, b, z1, 1, 1),
                    (x1, tp, z1, 1, 0),
                ],
                DIR_S,
                s,
                own,
            );
        }
        if w_mask & bit == 0 {
            let s = slot(var, cubes.face(cube, FACE_W));
            m.quad(
                [
                    (x, tp, z0, 0, 0),
                    (x, b, z0, 0, 1),
                    (x, b, z1, 1, 1),
                    (x, tp, z1, 1, 0),
                ],
                DIR_W,
                s,
                own,
            );
        }
        if e_mask & bit == 0 {
            let s = slot(var, cubes.face(cube, FACE_E));
            m.quad(
                [
                    (x1, tp, z1, 0, 0),
                    (x1, b, z1, 0, 1),
                    (x1, b, z0, 1, 1),
                    (x1, tp, z0, 1, 0),
                ],
                DIR_E,
                s,
                own,
            );
        }
    }
    let (floor, lb, lt) = column_heights(mask);
    let top = |m: &mut ChunkMesh, h: usize, tex: u16| {
        m.quad(
            [
                (x, h, z0, 0, 0),
                (x, h, z1, 0, 1),
                (x1, h, z1, 1, 1),
                (x1, h, z0, 1, 0),
            ],
            DIR_TOP,
            slot(var, tex),
            own,
        );
    };
    if floor > 0 {
        top(m, floor, cubes.face(col.cubes[floor - 1], FACE_TOP));
    } else {
        top(m, 0, col.floor_texture);
    }
    if lt > 0 {
        let cube = col.cubes[lt - 1];
        top(m, lt, cubes.face(cube, FACE_TOP));
        let s = slot(var, cubes.face(cube, FACE_BOTTOM));
        m.quad(
            [
                (x, lb, z0, 0, 1),
                (x1, lb, z0, 1, 1),
                (x1, lb, z1, 1, 0),
                (x, lb, z1, 0, 0),
            ],
            DIR_BOTTOM,
            s,
            own,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kfx_formats::level_bin::Column;

    fn col(mask: u16, cube: u16) -> Column {
        let mut cubes = [0u16; 8];
        for (h, c) in cubes.iter_mut().enumerate() {
            if mask & (1 << h) != 0 {
                *c = cube;
            }
        }
        Column {
            solidmask: mask,
            floor_texture: 7,
            cubes,
            ..Default::default()
        }
    }

    /// `w x h` subtiles, all using column 0 except those listed.
    fn terrain(w: usize, h: usize, cols: Vec<Column>, set: &[(usize, usize, u32)]) -> Terrain {
        let mut columns = vec![cols[0]; w * h];
        for &(x, y, c) in set {
            columns[y * w + x] = cols[c as usize];
        }
        let slabs = w.div_ceil(3) * h.div_ceil(3);
        Terrain {
            stl_w: w,
            stl_h: h,
            columns,
            slab_variation: vec![0; slabs],
            slab_owner: vec![5; slabs],
            slab_w: w.div_ceil(3),
        }
    }

    fn cubes() -> CubeFaces {
        CubeFaces(vec![[0; 6], [10, 11, 12, 13, 14, 15]])
    }

    #[test]
    fn edit_chunks_include_neighbours() {
        use kfx_map::autotile::SlabRect;
        // 20x20 slabs = 60x60 subtiles = 3x3 chunks of 24
        let t = terrain(60, 60, vec![col(0, 0)], &[]);
        // slab 0 only touches chunk 0
        assert_eq!(chunks_for_slabs(&t, SlabRect::single(0, 0)), vec![0]);
        // slab 7 = subtiles 21..23, its east neighbour subtile 24 is in chunk 1
        assert_eq!(chunks_for_slabs(&t, SlabRect::single(7, 0)), vec![0, 1]);
        // slab 8 = subtiles 24..26: chunk 1 plus the west neighbour subtile 23 in chunk 0
        assert_eq!(
            chunks_for_slabs(&t, SlabRect::single(8, 8)),
            vec![0, 1, 3, 4]
        );
        assert_eq!(
            chunks_for_slabs(
                &t,
                SlabRect {
                    x0: 0,
                    y0: 0,
                    x1: 19,
                    y1: 19
                }
            )
            .len(),
            9
        );
        assert!(chunks_for_slabs(
            &t,
            SlabRect {
                x0: 1,
                y0: 1,
                x1: 0,
                y1: 0
            }
        )
        .is_empty());
    }

    fn normal(m: &ChunkMesh, q: usize) -> [i32; 3] {
        let p = |i: usize| {
            let v = m.vertices[q * 4 + i];
            [v.x as i32, v.h as i32, v.z as i32]
        };
        let (a, b, c) = (p(0), p(1), p(2));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ]
    }

    #[test]
    fn heights_table() {
        assert_eq!(column_heights(0), (0, 0, 0));
        assert_eq!(column_heights(0b11111), (5, 0, 0));
        assert_eq!(column_heights(0b11000), (0, 3, 5));
        assert_eq!(column_heights(0b11001), (1, 3, 5));
    }

    #[test]
    fn flat_floor_only_tops() {
        let t = terrain(3, 3, vec![col(0, 0)], &[]);
        let m = build_region(&t, &cubes(), 0, 0, 3, 3);
        assert_eq!(m.quads(), 9);
        assert!(m
            .vertices
            .iter()
            .all(|v| v.h == 0 && v.face == DIR_TOP && v.slot == 7));
    }

    #[test]
    fn single_pillar_faces() {
        // 3x3 floor with a 5-cube pillar in the middle: 4 sides x 5 cubes + 9 tops.
        let t = terrain(3, 3, vec![col(0, 0), col(0b11111, 1)], &[(1, 1, 1)]);
        let m = build_region(&t, &cubes(), 0, 0, 3, 3);
        assert_eq!(m.quads(), 20 + 9);
        assert_eq!(m.triangles(), 2 * 29);
        let pillar_top: Vec<_> = m
            .vertices
            .iter()
            .filter(|v| v.face == DIR_TOP && v.h == 5)
            .collect();
        assert_eq!(pillar_top.len(), 4);
        assert!(pillar_top.iter().all(|v| v.slot == 14));
        let north: Vec<_> = m.vertices.iter().filter(|v| v.face == DIR_N).collect();
        assert_eq!(north.len(), 20);
        assert!(north.iter().all(|v| v.slot == 10 && v.z == 1));
        assert!(m
            .vertices
            .iter()
            .filter(|v| v.face == DIR_E)
            .all(|v| v.slot == 11 && v.x == 2));
        assert!(m
            .vertices
            .iter()
            .filter(|v| v.face == DIR_S)
            .all(|v| v.slot == 12 && v.z == 2));
        assert!(m
            .vertices
            .iter()
            .filter(|v| v.face == DIR_W)
            .all(|v| v.slot == 13 && v.x == 1));
    }

    #[test]
    fn adjacent_solid_faces_are_culled() {
        // 2x1 wall of height 5 in a 4x3 map: the shared face disappears from both.
        let t = terrain(
            4,
            3,
            vec![col(0, 0), col(0b11111, 1)],
            &[(1, 1, 1), (2, 1, 1)],
        );
        let m = build_region(&t, &cubes(), 0, 0, 4, 3);
        // sides: 6 exposed faces per layer x 5 layers; tops: 12
        assert_eq!(m.quads(), 30 + 12);
        // a lower neighbour only hides the overlapping layers
        let t = terrain(
            4,
            3,
            vec![col(0, 0), col(0b11111, 1), col(0b00011, 1)],
            &[(1, 1, 1), (2, 1, 2)],
        );
        let m = build_region(&t, &cubes(), 0, 0, 4, 3);
        // tall: 3 sides x5 + east 3 (layers 2..4); low: 3 sides x2
        assert_eq!(m.quads() - 12, 15 + 3 + 6);
    }

    #[test]
    fn lintel_faces() {
        // door-like column: cubes 3 and 4 solid over an empty bottom
        let t = terrain(1, 1, vec![col(0b11000, 1)], &[]);
        let m = build_region(&t, &cubes(), 0, 0, 1, 1);
        // 4 sides x 2 cubes, floor at 0, lintel top at 5, lintel bottom at 3
        assert_eq!(m.quads(), 8 + 3);
        let bottom: Vec<_> = m.vertices.iter().filter(|v| v.face == DIR_BOTTOM).collect();
        assert_eq!(bottom.len(), 4);
        assert!(bottom.iter().all(|v| v.h == 3 && v.slot == 15));
        assert!(m
            .vertices
            .iter()
            .any(|v| v.face == DIR_TOP && v.h == 0 && v.slot == 7));
        assert!(m
            .vertices
            .iter()
            .any(|v| v.face == DIR_TOP && v.h == 5 && v.slot == 14));
    }

    #[test]
    fn winding_faces_outwards() {
        let t = terrain(3, 3, vec![col(0, 0), col(0b11000, 1)], &[(1, 1, 1)]);
        let m = build_region(&t, &cubes(), 0, 0, 3, 3);
        for q in 0..m.quads() {
            let n = normal(&m, q);
            let want = match m.vertices[q * 4].face {
                DIR_TOP => [0, 1, 0],
                DIR_BOTTOM => [0, -1, 0],
                DIR_N => [0, 0, -1],
                DIR_S => [0, 0, 1],
                DIR_E => [1, 0, 0],
                DIR_W => [-1, 0, 0],
                _ => unreachable!(),
            };
            let s: Vec<i32> = n.iter().map(|v| v.signum()).collect();
            assert_eq!(s, want.to_vec(), "quad {q}");
        }
    }

    #[test]
    fn variation_offsets_slot() {
        let mut t = terrain(3, 3, vec![col(0, 0)], &[]);
        t.slab_variation = vec![2];
        let m = build_region(&t, &cubes(), 0, 0, 1, 1);
        assert_eq!(m.vertices[0].slot, 2 * 1544 + 7);
        assert_eq!(slot(31, 1543), 31 * 1544 + 1543);
        assert_eq!(slot(31, 2000), 0);
        assert_eq!(slot(1, 2000), 1544 + 2000);
    }
}
