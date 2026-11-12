//! A small software isometric rasteriser for column / cube / slab previews, drawing textured
//! cube faces from the block atlas into an RGBA image (so the crate needs no GPU renderer).
//!
//! World units: one cube = 1 x 1 x 1; x grows east, y grows south (map coordinates), z up. The
//! camera looks from the south-east, so the visible faces are top, south and east. Projection:
//! `sx = (x - y) * s`, `sy = (x + y) * s / 2 - z * s * HEIGHT_RATIO`. Painter's order: columns by
//! `x + y`, cubes bottom-up. An id buffer records which column drew each pixel, for picking.
use kfx_gfx::textures::{TextureAnim, TextureSet, TILE_DIM};
use kfx_gfx::{Palette, Rgba};
use std::collections::HashMap;

/// Vertical size of a cube relative to `s`.
pub const HEIGHT_RATIO: f32 = 1.2;

/// Cube face order of `Textures` in cubes.cfg: north, east, south, west, top, bottom.
pub const FACE_N: usize = 0;
pub const FACE_E: usize = 1;
pub const FACE_S: usize = 2;
pub const FACE_W: usize = 3;
pub const FACE_TOP: usize = 4;
pub const FACE_BOTTOM: usize = 5;

/// Decoded 32x32 tiles by (texture id, animation frame).
pub struct TileCache {
    pub palette: Palette,
    pub set: TextureSet,
    pub anim: TextureAnim,
    cache: HashMap<(u16, usize), Option<Rgba>>,
}

impl TileCache {
    pub fn new(palette: Palette, set: TextureSet, anim: TextureAnim) -> Self {
        TileCache {
            palette,
            set,
            anim,
            cache: HashMap::new(),
        }
    }
    /// Tile of texture `id` at animation frame `frame` (animated ids 544..999 resolve through
    /// `textureanim.toml`).
    pub fn tile(&mut self, id: u16, frame: usize) -> Option<&Rgba> {
        let frame = if (544..1000).contains(&id) {
            frame % 8
        } else {
            0
        };
        let (set, anim, pal) = (&self.set, &self.anim, &self.palette);
        self.cache
            .entry((id, frame))
            .or_insert_with(|| set.id_rgba(id, frame, anim, pal))
            .as_ref()
    }
    /// Forget decoded tiles (after the texture set or animation table changed).
    pub fn clear(&mut self) {
        self.cache.clear();
    }
    pub fn set_textures(&mut self, set: TextureSet) {
        self.set = set;
        self.cache.clear();
    }
    pub fn set_anim(&mut self, anim: TextureAnim) {
        self.anim = anim;
        self.cache.clear();
    }
}

/// An RGBA canvas with an id buffer.
pub struct Canvas {
    pub img: Rgba,
    /// Per pixel: 0 = nothing, else the `id` passed to the draw call.
    pub ids: Vec<u32>,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Self {
        Canvas {
            img: Rgba::new(w, h),
            ids: vec![0; (w * h) as usize],
        }
    }
    pub fn id_at(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.img.w as i32 || y >= self.img.h as i32 {
            return 0;
        }
        self.ids[(y as u32 * self.img.w + x as u32) as usize]
    }
}

/// Projection parameters.
#[derive(Clone, Copy, Debug)]
pub struct Iso {
    /// Pixels per half diamond width.
    pub s: f32,
    pub ox: f32,
    pub oy: f32,
}

impl Iso {
    pub fn project(&self, x: f32, y: f32, z: f32) -> (f32, f32) {
        (
            self.ox + (x - y) * self.s,
            self.oy + (x + y) * self.s * 0.5 - z * self.s * HEIGHT_RATIO,
        )
    }
    /// A view that fits a `w x h` (cubes) footprint with `zmax` cubes of height into a canvas of
    /// `cw x ch` pixels.
    pub fn fit(w: f32, h: f32, zmax: f32, cw: f32, ch: f32) -> Iso {
        let s_w = cw / (w + h);
        let s_h = ch / ((w + h) * 0.5 + zmax * HEIGHT_RATIO);
        let s = s_w.min(s_h).max(0.5);
        let total_h = (w + h) * 0.5 * s + zmax * HEIGHT_RATIO * s;
        let oy = (ch - total_h) / 2.0 + zmax * HEIGHT_RATIO * s;
        let ox = cw / 2.0 + (h - w) * s / 2.0;
        Iso { s, ox, oy }
    }
}

/// Draw a textured parallelogram: `p0` is texel (0,0), `p1` (1,0), `p3` (0,1).
#[allow(clippy::too_many_arguments)]
fn draw_face(
    c: &mut Canvas,
    p0: (f32, f32),
    p1: (f32, f32),
    p3: (f32, f32),
    tile: Option<&Rgba>,
    shade: f32,
    tint: Option<[u8; 4]>,
    id: u32,
) {
    let (ax, ay) = (p1.0 - p0.0, p1.1 - p0.1);
    let (bx, by) = (p3.0 - p0.0, p3.1 - p0.1);
    let det = ax * by - ay * bx;
    if det.abs() < 1e-6 {
        return;
    }
    let p2 = (p1.0 + bx, p1.1 + by);
    let xs = [p0.0, p1.0, p2.0, p3.0];
    let ys = [p0.1, p1.1, p2.1, p3.1];
    let x0 = xs.iter().cloned().fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let x1 = xs
        .iter()
        .cloned()
        .fold(f32::MIN, f32::max)
        .ceil()
        .min(c.img.w as f32) as i32;
    let y0 = ys.iter().cloned().fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let y1 = ys
        .iter()
        .cloned()
        .fold(f32::MIN, f32::max)
        .ceil()
        .min(c.img.h as f32) as i32;
    let td = TILE_DIM as f32;
    for py in y0..y1 {
        for px in x0..x1 {
            let (dx, dy) = (px as f32 + 0.5 - p0.0, py as f32 + 0.5 - p0.1);
            let u = (dx * by - dy * bx) / det;
            let v = (ax * dy - ay * dx) / det;
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            let mut col = match tile {
                Some(t) => t.pixel(
                    ((u * td) as u32).min(t.w - 1),
                    ((v * td) as u32).min(t.h - 1),
                ),
                None => {
                    // missing texture: checker
                    let chk = (((u * 4.0) as i32 + (v * 4.0) as i32) & 1) == 0;
                    if chk {
                        [200, 0, 200, 255]
                    } else {
                        [40, 0, 40, 255]
                    }
                }
            };
            if col[3] == 0 {
                continue;
            }
            for ch in col.iter_mut().take(3) {
                *ch = (*ch as f32 * shade).min(255.0) as u8;
            }
            if let Some(t) = tint {
                let a = t[3] as f32 / 255.0;
                for k in 0..3 {
                    col[k] = (col[k] as f32 * (1.0 - a) + t[k] as f32 * a) as u8;
                }
            }
            col[3] = 255;
            c.img.put(px as u32, py as u32, col);
            c.ids[(py as u32 * c.img.w + px as u32) as usize] = id;
        }
    }
}

/// Visible faces of a cube to draw.
#[derive(Clone, Copy, Debug)]
pub struct Faces {
    pub top: bool,
    pub south: bool,
    pub east: bool,
}

impl Faces {
    pub const ALL: Faces = Faces {
        top: true,
        south: true,
        east: true,
    };
}

/// Draw one cube at (x, y, z) with face textures `tex` (cubes.cfg order).
#[allow(clippy::too_many_arguments)]
pub fn draw_cube(
    c: &mut Canvas,
    v: &Iso,
    tiles: &mut TileCache,
    frame: usize,
    x: f32,
    y: f32,
    z: f32,
    tex: [u16; 6],
    faces: Faces,
    tint: Option<[u8; 4]>,
    id: u32,
) {
    let p = |x: f32, y: f32, z: f32| v.project(x, y, z);
    if faces.south {
        let t = tiles.tile(tex[FACE_S], frame).cloned();
        draw_face(
            c,
            p(x, y + 1.0, z + 1.0),
            p(x + 1.0, y + 1.0, z + 1.0),
            p(x, y + 1.0, z),
            t.as_ref(),
            0.78,
            tint,
            id,
        );
    }
    if faces.east {
        let t = tiles.tile(tex[FACE_E], frame).cloned();
        draw_face(
            c,
            p(x + 1.0, y + 1.0, z + 1.0),
            p(x + 1.0, y, z + 1.0),
            p(x + 1.0, y + 1.0, z),
            t.as_ref(),
            0.62,
            tint,
            id,
        );
    }
    if faces.top {
        let t = tiles.tile(tex[FACE_TOP], frame).cloned();
        draw_face(
            c,
            p(x, y, z + 1.0),
            p(x + 1.0, y, z + 1.0),
            p(x, y + 1.0, z + 1.0),
            t.as_ref(),
            1.0,
            tint,
            id,
        );
    }
}

/// Draw a flat floor tile at height `z` (texture `tex`).
#[allow(clippy::too_many_arguments)]
pub fn draw_floor(
    c: &mut Canvas,
    v: &Iso,
    tiles: &mut TileCache,
    frame: usize,
    x: f32,
    y: f32,
    z: f32,
    tex: u16,
    tint: Option<[u8; 4]>,
    id: u32,
) {
    let t = tiles.tile(tex, frame).cloned();
    draw_face(
        c,
        v.project(x, y, z),
        v.project(x + 1.0, y, z),
        v.project(x, y + 1.0, z),
        t.as_ref(),
        1.0,
        tint,
        id,
    );
}

/// One column to draw: 8 cubes bottom to top and the floor texture.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColumnDraw {
    pub x: i32,
    pub y: i32,
    pub cubes: [u16; 8],
    pub floor: u16,
    pub tint: Option<[u8; 4]>,
    pub id: u32,
}

/// Height in cubes of the tallest column (at least 1), to fit views.
pub fn max_height(cols: &[ColumnDraw]) -> f32 {
    cols.iter()
        .map(|d| d.cubes.iter().rposition(|&c| c != 0).map_or(0, |i| i + 1))
        .max()
        .unwrap_or(0)
        .max(1) as f32
}

/// Draw columns in painter's order. `faces(cube)` gives a cube's 6 texture ids.
pub fn draw_columns(
    c: &mut Canvas,
    v: &Iso,
    tiles: &mut TileCache,
    frame: usize,
    cols: &[ColumnDraw],
    faces: &dyn Fn(u16) -> [u16; 6],
) {
    let mut order: Vec<&ColumnDraw> = cols.iter().collect();
    order.sort_by_key(|d| (d.x + d.y, d.x));
    let solid = |x: i32, y: i32, z: usize| {
        cols.iter()
            .any(|d| d.x == x && d.y == y && d.cubes.get(z).is_some_and(|&c| c != 0))
    };
    for d in order {
        if d.cubes[0] == 0 {
            draw_floor(
                c, v, tiles, frame, d.x as f32, d.y as f32, 0.0, d.floor, d.tint, d.id,
            );
        }
        for (z, &cube) in d.cubes.iter().enumerate() {
            if cube == 0 {
                continue;
            }
            let f = Faces {
                top: z + 1 >= 8 || d.cubes[z + 1] == 0,
                south: !solid(d.x, d.y + 1, z),
                east: !solid(d.x + 1, d.y, z),
            };
            draw_cube(
                c,
                v,
                tiles,
                frame,
                d.x as f32,
                d.y as f32,
                z as f32,
                faces(cube),
                f,
                d.tint,
                d.id,
            );
        }
    }
}
