//! Whole-map and rectangle transforms: resize from any edge, flip, rotate; and the symmetry
//! helpers that map one slab edit to its mirrored counterparts.
//!
//! Positions of things, action points and lights move with their slab (exact on the 1/256
//! subtile grid), `ParentTile` slab numbers are re-computed, thing `Orientation` angles
//! (0 = north, 512 = east, clockwise; `ANGLE_NORTH` / `ANGLE_EAST` src/globals.h:170-172) are turned
//! with the map, and door orientation flips under quarter turns (the door slab kind follows, see
//! [`crate::things`]). Columns of the changed area (and the ring around it) are regenerated,
//! because cube faces are direction-specific and cannot simply be moved; run [`MapDoc::decorate`]
//! afterwards for varied cubes.
//!
//! The game's map size limit is `MAX_TILES_X` = 170 slabs (src/globals.h:163).
use crate::autotile::{self, SlabRect};
use crate::gamedata::GameData;
use crate::mapdoc::MapDoc;
use crate::patch::Patch;
use crate::things::{class, thing_class};
use crate::{Error, Result};
use kfx_formats::level_bin::{Column, MapDims};
use kfx_formats::level_fx::StlCoord;

/// `MAX_TILES_X` (src/globals.h:163); the same bound is used for both axes.
pub const MAX_MAP_SLABS: u16 = 170;
const RAW_PER_SLAB: i64 = 768;

/// A flip or rotation (rotations are clockwise as seen on the map, north up).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    /// Mirror east <-> west.
    FlipX,
    /// Mirror north <-> south.
    FlipY,
    Rot90,
    Rot180,
    Rot270,
}

impl Transform {
    pub fn swaps_axes(self) -> bool {
        matches!(self, Transform::Rot90 | Transform::Rot270)
    }
    /// Slab (x, y) of a `w x h` region -> position in the transformed region.
    pub fn slab(self, w: i64, h: i64, x: i64, y: i64) -> (i64, i64) {
        match self {
            Transform::FlipX => (w - 1 - x, y),
            Transform::FlipY => (x, h - 1 - y),
            Transform::Rot90 => (h - 1 - y, x),
            Transform::Rot180 => (w - 1 - x, h - 1 - y),
            Transform::Rot270 => (y, w - 1 - x),
        }
    }
    /// Continuous (raw 1/256 subtile) position in a region of raw size `wr x hr`.
    pub fn raw(self, wr: i64, hr: i64, u: i64, v: i64) -> (i64, i64) {
        match self {
            Transform::FlipX => (wr - u, v),
            Transform::FlipY => (u, hr - v),
            Transform::Rot90 => (hr - v, u),
            Transform::Rot180 => (wr - u, hr - v),
            Transform::Rot270 => (v, wr - u),
        }
    }
    /// Angle 0..2047 (0 = north, clockwise).
    pub fn angle(self, a: i64) -> i64 {
        (match self {
            Transform::FlipX => 2048 - a,
            Transform::FlipY => 1024 - a,
            Transform::Rot90 => a + 512,
            Transform::Rot180 => a + 1024,
            Transform::Rot270 => a + 1536,
        })
        .rem_euclid(2048)
    }
}

fn raw_of(c: Option<StlCoord>) -> Option<i64> {
    c.map(|c| c.stl as i64 * 256 + c.sub as i64)
}
fn coord(raw: i64) -> StlCoord {
    StlCoord::new(raw.div_euclid(256) as i32, raw.rem_euclid(256) as i32)
}

impl MapDoc {
    /// Move every item by `(dx, dy)` slabs into a map of `nd` dims (old dims `od`); items that
    /// fall outside are dropped, slab parents re-numbered.
    fn shift_items(&mut self, od: MapDims, nd: MapDims, dx: i64, dy: i64) {
        let inside = |u: i64, v: i64| {
            u >= 0 && v >= 0 && u < nd.w as i64 * RAW_PER_SLAB && v < nd.h as i64 * RAW_PER_SLAB
        };
        let reparent = |p: Option<i64>| {
            p.map(|p| {
                if p < 0 || p >= od.slabs() as i64 {
                    return p;
                }
                let (x, y) = (p % od.w as i64 + dx, p / od.w as i64 + dy);
                if x < 0 || y < 0 || x >= nd.w as i64 || y >= nd.h as i64 {
                    return p;
                }
                autotile::slab_number(nd, x as i32, y as i32) as i64
            })
        };
        let (ox, oy) = (dx * RAW_PER_SLAB, dy * RAW_PER_SLAB);
        self.things.items.retain_mut(|t| {
            let (Some(u), Some(v)) = (raw_of(t.x), raw_of(t.y)) else {
                return true;
            };
            let (u, v) = (u + ox, v + oy);
            if !inside(u, v) {
                return false;
            }
            t.x = Some(coord(u));
            t.y = Some(coord(v));
            t.parent_tile = reparent(t.parent_tile);
            true
        });
        self.action_points.items.retain_mut(|a| {
            let (Some(u), Some(v)) = (raw_of(a.x), raw_of(a.y)) else {
                return true;
            };
            let (u, v) = (u + ox, v + oy);
            if !inside(u, v) {
                return false;
            }
            a.x = Some(coord(u));
            a.y = Some(coord(v));
            true
        });
        self.lights.items.retain_mut(|l| {
            let (Some(u), Some(v)) = (raw_of(l.x), raw_of(l.y)) else {
                return true;
            };
            let (u, v) = (u + ox, v + oy);
            if !inside(u, v) {
                return false;
            }
            l.x = Some(coord(u));
            l.y = Some(coord(v));
            l.parent_tile = reparent(l.parent_tile);
            true
        });
    }

    /// Grow (positive) or shrink (negative) the map at each edge by that many slabs. New slabs
    /// are `fill_kind` owned by `fill_owner`; items move with the map, items cut off are removed.
    /// Columns of kept slabs are kept except next to the new / cut edge, which is regenerated.
    pub fn resize(
        &mut self,
        gd: &GameData,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        fill_kind: u16,
        fill_owner: u8,
    ) -> Result<Patch> {
        let od = self.dims;
        let nw = od.w as i32 + left + right;
        let nh = od.h as i32 + top + bottom;
        if nw < 1 || nh < 1 || nw > MAX_MAP_SLABS as i32 || nh > MAX_MAP_SLABS as i32 {
            return Err(Error::Invalid(format!(
                "map size {nw}x{nh} outside 1..={MAX_MAP_SLABS}"
            )));
        }
        let nd = MapDims::new(nw as u16, nh as u16);
        Ok(self
            .record_tracked(|d| {
                d.touch_whole();
                let n = nd.slabs();
                let mut kinds = vec![fill_kind; n];
                let mut owners = vec![fill_owner; n];
                let mut wlb = vec![gd.slab(fill_kind as usize).wlb_type; n];
                let mut slx = vec![0u8; n];
                let mut custom = vec![0u16; n];
                let mut columns = vec![Column::default(); nd.stl_grid()];
                let mut wibble = vec![0u8; nd.stl_grid()];
                for y in 0..od.h as i32 {
                    for x in 0..od.w as i32 {
                        let (nx, ny) = (x + left, y + top);
                        if nx < 0 || ny < 0 || nx >= nw || ny >= nh {
                            continue;
                        }
                        let (i, j) = (
                            od.slab_index(x as usize, y as usize),
                            nd.slab_index(nx as usize, ny as usize),
                        );
                        kinds[j] = d.kinds[i];
                        owners[j] = d.owners[i];
                        wlb[j] = d.wlb[i];
                        slx[j] = d.slx[i];
                        custom[j] = d.custom_cells[i];
                        for k in 0..9 {
                            let a = od.stl_index(3 * x as usize + k % 3, 3 * y as usize + k / 3);
                            let b = nd.stl_index(3 * nx as usize + k % 3, 3 * ny as usize + k / 3);
                            columns[b] = d.columns[a];
                            wibble[b] = d.wibble[a];
                        }
                    }
                }
                d.dims = nd;
                d.kinds = kinds;
                d.owners = owners;
                d.wlb = wlb;
                d.slx = slx;
                d.custom_cells = custom;
                d.columns = columns;
                d.wibble = wibble;
                d.own_loaded = None;
                d.flg = None;
                d.shift_items(od, nd, left as i64, top as i64);
                // regenerate everything that is new or lost a neighbour
                let kept = SlabRect {
                    x0: left.max(0),
                    y0: top.max(0),
                    x1: (od.w as i32 - 1 + left).min(nw - 1),
                    y1: (od.h as i32 - 1 + top).min(nh - 1),
                };
                for (x, y) in (SlabRect {
                    x0: 0,
                    y0: 0,
                    x1: nw - 1,
                    y1: nh - 1,
                })
                .iter()
                {
                    let inner = x > kept.x0 && y > kept.y0 && x < kept.x1 && y < kept.y1;
                    if !inner {
                        d.regenerate_rect(gd, SlabRect::single(x, y));
                    }
                }
            })
            .1)
    }

    /// Flip / rotate the whole map (a quarter turn of a non-square map swaps its size).
    pub fn transform_map(&mut self, gd: &GameData, t: Transform) -> Patch {
        let r = SlabRect {
            x0: 0,
            y0: 0,
            x1: self.dims.w as i32 - 1,
            y1: self.dims.h as i32 - 1,
        };
        self.transform_impl(gd, r, t, true).unwrap_or_default()
    }

    /// Flip / rotate the slabs and items of `rect` in place. Quarter turns need a square rect.
    pub fn transform_rect(&mut self, gd: &GameData, rect: SlabRect, t: Transform) -> Result<Patch> {
        let rect = rect.clip(self.dims);
        if rect.is_empty() {
            return Ok(Patch::new());
        }
        if t.swaps_axes() && rect.x1 - rect.x0 != rect.y1 - rect.y0 {
            return Err(Error::Invalid(
                "a quarter turn of a rectangle needs a square".into(),
            ));
        }
        self.transform_impl(gd, rect, t, false)
    }

    fn transform_impl(
        &mut self,
        gd: &GameData,
        rect: SlabRect,
        t: Transform,
        whole: bool,
    ) -> Result<Patch> {
        Ok(self
            .record_tracked(|d| {
                d.touch_whole();
                let od = d.dims;
                let (rw, rh) = (
                    (rect.x1 - rect.x0 + 1) as i64,
                    (rect.y1 - rect.y0 + 1) as i64,
                );
                let nd = if whole && t.swaps_axes() {
                    MapDims::new(od.h, od.w)
                } else {
                    od
                };
                // slabs
                let (mut kinds, mut owners, mut wlb, mut slx) = (
                    d.kinds.clone(),
                    d.owners.clone(),
                    d.wlb.clone(),
                    d.slx.clone(),
                );
                let mut custom = d.custom_cells.clone();
                let mut custom_map: std::collections::HashMap<u16, u16> = Default::default();
                if nd != od {
                    let n = nd.slabs();
                    kinds = vec![0; n];
                    owners = vec![0; n];
                    wlb = vec![0; n];
                    slx = vec![0; n];
                    custom = vec![0; n];
                }
                let mut slab_map = std::collections::HashMap::new();
                for y in 0..rh {
                    for x in 0..rw {
                        let (nx, ny) = t.slab(rw, rh, x, y);
                        let (sx, sy) =
                            ((rect.x0 as i64 + x) as usize, (rect.y0 as i64 + y) as usize);
                        let (tx, ty) = (
                            (rect.x0 as i64 + nx) as usize,
                            (rect.y0 as i64 + ny) as usize,
                        );
                        let (i, j) = (od.slab_index(sx, sy), nd.slab_index(tx, ty));
                        kinds[j] = d.kinds[i];
                        owners[j] = d.owners[i];
                        wlb[j] = d.wlb[i];
                        slx[j] = d.slx[i];
                        custom[j] = match d.custom_cells[i] {
                            0 => 0,
                            id => *custom_map
                                .entry(id)
                                .or_insert_with(|| d.sidecar.transformed_custom_slab(id, t)),
                        };
                        slab_map.insert(
                            autotile::slab_number(od, sx as i32, sy as i32) as i64,
                            autotile::slab_number(nd, tx as i32, ty as i32) as i64,
                        );
                    }
                }
                d.dims = nd;
                d.kinds = kinds;
                d.owners = owners;
                d.wlb = wlb;
                d.slx = slx;
                d.custom_cells = custom;
                if nd != od {
                    d.columns = vec![Column::default(); nd.stl_grid()];
                    d.wibble = vec![0; nd.stl_grid()];
                }
                if whole {
                    d.own_loaded = None;
                    d.flg = None;
                }
                // items inside the rect
                let (x0, y0) = (rect.x0 as i64 * RAW_PER_SLAB, rect.y0 as i64 * RAW_PER_SLAB);
                let (wr, hr) = (rw * RAW_PER_SLAB, rh * RAW_PER_SLAB);
                let inside = |u: i64, v: i64| u >= x0 && v >= y0 && u < x0 + wr && v < y0 + hr;
                let map_pos = |u: i64, v: i64| {
                    let (a, b) = t.raw(wr, hr, u - x0, v - y0);
                    // a position exactly on the far edge maps onto the near edge: keep it inside
                    (
                        x0 + a.min((if t.swaps_axes() { hr } else { wr }) - 1),
                        y0 + b.min((if t.swaps_axes() { wr } else { hr }) - 1),
                    )
                };
                let reparent = |p: Option<i64>| p.map(|p| *slab_map.get(&p).unwrap_or(&p));
                for th in d.things.items.iter_mut() {
                    let (Some(u), Some(v)) = (raw_of(th.x), raw_of(th.y)) else {
                        continue;
                    };
                    if !inside(u, v) {
                        continue;
                    }
                    let (a, b) = map_pos(u, v);
                    th.x = Some(coord(a));
                    th.y = Some(coord(b));
                    th.parent_tile = reparent(th.parent_tile);
                    if let Some(o) = th.orientation {
                        th.orientation = Some(t.angle(o));
                    }
                    if thing_class(th) == Some(class::DOOR) && t.swaps_axes() {
                        th.door_orientation =
                            Some(1 - th.door_orientation.unwrap_or(0).clamp(0, 1));
                    }
                }
                for a in d.action_points.items.iter_mut() {
                    let (Some(u), Some(v)) = (raw_of(a.x), raw_of(a.y)) else {
                        continue;
                    };
                    if inside(u, v) {
                        let (p, q) = map_pos(u, v);
                        a.x = Some(coord(p));
                        a.y = Some(coord(q));
                    }
                }
                for l in d.lights.items.iter_mut() {
                    let (Some(u), Some(v)) = (raw_of(l.x), raw_of(l.y)) else {
                        continue;
                    };
                    if inside(u, v) {
                        let (p, q) = map_pos(u, v);
                        l.x = Some(coord(p));
                        l.y = Some(coord(q));
                        l.parent_tile = reparent(l.parent_tile);
                    }
                }
                // door slab kinds follow the (possibly flipped) door orientation
                let doors: Vec<usize> = (0..d.things.items.len())
                    .filter(|&i| thing_class(&d.things.items[i]) == Some(class::DOOR))
                    .collect();
                for i in doors {
                    let th = d.things.items[i].clone();
                    let (Some(x), Some(y)) = (th.x, th.y) else {
                        continue;
                    };
                    let (sx, sy) = (x.stl.div_euclid(3), y.stl.div_euclid(3));
                    if !autotile::in_map(d.dims, sx, sy) {
                        continue;
                    }
                    if let Some(k) = th
                        .subtype
                        .and_then(|m| crate::things::door_slab_kinds(gd, m))
                    {
                        let j = d.dims.slab_index(sx as usize, sy as usize);
                        d.kinds[j] = k[th.door_orientation.unwrap_or(0).clamp(0, 1) as usize];
                    }
                }
                let r = if whole {
                    SlabRect {
                        x0: 0,
                        y0: 0,
                        x1: nd.w as i32 - 1,
                        y1: nd.h as i32 - 1,
                    }
                } else {
                    rect.grow(1)
                };
                d.regenerate_rect(gd, r);
            })
            .1)
    }
}

// ---------------------------------------------------------------------------------------------
// Symmetry

/// Kind of symmetry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymmetryMode {
    /// Mirror across the horizontal middle line (north half <-> south half). Sectors: 0 north,
    /// 1 south.
    NorthSouth,
    /// Mirror across the vertical middle line. Sectors: 0 west, 1 east.
    EastWest,
    /// Both mirrors. Sectors: 0 NW, 1 NE, 2 SW, 3 SE.
    FourWay,
    /// Point symmetry (half turn). Sectors: 0 north half, 1 south half.
    Rot2,
    /// Quarter-turn symmetry (square maps). Sectors clockwise: 0 NW, 1 NE, 2 SE, 3 SW.
    Rot4,
}

/// A symmetry with an owner mapping per sector: `owners[s][c]` is the owner used in sector `s`
/// for owner `c` of sector 0 (identity by default).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symmetry {
    pub mode: SymmetryMode,
    pub owners: Vec<[u8; 9]>,
}

/// One image of an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Image {
    pub x: i32,
    pub y: i32,
    pub sector: usize,
    /// The transform from the edited position to this one (`None` for the edit itself).
    pub transform: Option<Transform>,
}

impl Symmetry {
    pub fn new(mode: SymmetryMode) -> Symmetry {
        let n = match mode {
            SymmetryMode::NorthSouth | SymmetryMode::EastWest | SymmetryMode::Rot2 => 2,
            _ => 4,
        };
        let id: [u8; 9] = std::array::from_fn(|i| i as u8);
        Symmetry {
            mode,
            owners: vec![id; n],
        }
    }

    /// Sector of slab (x, y).
    pub fn sector(&self, d: MapDims, x: i32, y: i32) -> usize {
        let (w, h) = (d.w as i32, d.h as i32);
        let north = 2 * y < h;
        let east = 2 * x >= w;
        match self.mode {
            SymmetryMode::NorthSouth => (!north) as usize,
            SymmetryMode::EastWest => east as usize,
            SymmetryMode::FourWay => (!north) as usize * 2 + east as usize,
            SymmetryMode::Rot2 => (!(2 * y < h - 1 || (2 * y == h - 1 && 2 * x < w))) as usize,
            SymmetryMode::Rot4 => match (north, !east) {
                (true, true) => 0,
                (true, false) => 1,
                (false, false) => 2,
                (false, true) => 3,
            },
        }
    }

    /// Every distinct slab an edit of (x, y) maps to, the edit itself first. `Rot4` on a
    /// non-square map degrades to `Rot2`.
    pub fn images(&self, d: MapDims, x: i32, y: i32) -> Vec<Image> {
        let (w, h) = (d.w as i64, d.h as i64);
        let ts: &[Transform] = match self.mode {
            SymmetryMode::NorthSouth => &[Transform::FlipY],
            SymmetryMode::EastWest => &[Transform::FlipX],
            SymmetryMode::FourWay => &[Transform::FlipX, Transform::FlipY, Transform::Rot180],
            SymmetryMode::Rot2 => &[Transform::Rot180],
            SymmetryMode::Rot4 if w == h => {
                &[Transform::Rot90, Transform::Rot180, Transform::Rot270]
            }
            SymmetryMode::Rot4 => &[Transform::Rot180],
        };
        let mut out = vec![Image {
            x,
            y,
            sector: self.sector(d, x, y),
            transform: None,
        }];
        for &t in ts {
            let (nx, ny) = t.slab(w, h, x as i64, y as i64);
            let (nx, ny) = (nx as i32, ny as i32);
            if !out.iter().any(|i| i.x == nx && i.y == ny) {
                out.push(Image {
                    x: nx,
                    y: ny,
                    sector: self.sector(d, nx, ny),
                    transform: Some(t),
                });
            }
        }
        out
    }

    /// Owner to use in sector `to` for an edit with `owner` made in sector `from`.
    pub fn map_owner(&self, from: usize, to: usize, owner: u8) -> u8 {
        let Some(src) = self.owners.get(from) else {
            return owner;
        };
        let base = src
            .iter()
            .position(|&o| o == owner)
            .map(|b| b as u8)
            .unwrap_or(owner);
        self.owners
            .get(to)
            .and_then(|t| t.get(base as usize))
            .copied()
            .unwrap_or(owner)
    }

    /// The slab edits (x, y, kind, owner) a symmetric edit of (x, y) makes.
    pub fn slab_edits(
        &self,
        d: MapDims,
        x: i32,
        y: i32,
        kind: u16,
        owner: u8,
    ) -> Vec<(i32, i32, u16, u8)> {
        let imgs = self.images(d, x, y);
        let from = imgs[0].sector;
        imgs.iter()
            .map(|i| (i.x, i.y, kind, self.map_owner(from, i.sector, owner)))
            .collect()
    }

    /// Images of a raw (1/256 subtile) position, with the angle transform to apply.
    pub fn point_images(
        &self,
        d: MapDims,
        u: i64,
        v: i64,
    ) -> Vec<(i64, i64, usize, Option<Transform>)> {
        let (sx, sy) = ((u / RAW_PER_SLAB) as i32, (v / RAW_PER_SLAB) as i32);
        let (wr, hr) = (d.w as i64 * RAW_PER_SLAB, d.h as i64 * RAW_PER_SLAB);
        self.images(d, sx, sy)
            .into_iter()
            .map(|i| match i.transform {
                None => (u, v, i.sector, None),
                Some(t) => {
                    let (a, b) = t.raw(wr, hr, u, v);
                    (a.min(wr - 1), b.min(hr - 1), i.sector, Some(t))
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_compose() {
        let (w, h) = (7i64, 4i64);
        for x in 0..w {
            for y in 0..h {
                let (a, b) = Transform::Rot90.slab(w, h, x, y);
                let (c, e) = Transform::Rot270.slab(h, w, a, b);
                assert_eq!((c, e), (x, y));
                let (a, b) = Transform::Rot180.slab(w, h, x, y);
                assert_eq!(Transform::Rot180.slab(w, h, a, b), (x, y));
            }
        }
        // the centre of subtile 5 mirrors onto the centre of subtile 3W-1-5
        let wr = 85 * 768;
        assert_eq!(
            Transform::FlipX.raw(wr, wr, 5 * 256 + 128, 0).0,
            (254 - 5) * 256 + 128
        );
        assert_eq!(Transform::Rot90.angle(0), 512);
        assert_eq!(Transform::FlipX.angle(512), 1536);
        assert_eq!(Transform::FlipY.angle(0), 1024);
    }

    #[test]
    fn symmetry_images_and_owners() {
        let d = MapDims::new(10, 10);
        let mut s = Symmetry::new(SymmetryMode::FourWay);
        s.owners[1][0] = 1;
        s.owners[2][0] = 2;
        s.owners[3][0] = 3;
        let e = s.slab_edits(d, 1, 2, 11, 0);
        assert_eq!(
            e,
            vec![(1, 2, 11, 0), (8, 2, 11, 1), (1, 7, 11, 2), (8, 7, 11, 3)]
        );
        // an edit in sector 1 with owner 1 maps back to owner 0 in sector 0
        let e = s.slab_edits(d, 8, 2, 11, 1);
        assert!(e.contains(&(1, 2, 11, 0)));
        let r4 = Symmetry::new(SymmetryMode::Rot4);
        let imgs = r4.images(d, 0, 0);
        assert_eq!(imgs.len(), 4);
        let sectors: Vec<usize> = imgs.iter().map(|i| i.sector).collect();
        assert_eq!(sectors, vec![0, 1, 2, 3]);
        // the centre of an odd map maps onto itself only
        assert_eq!(
            Symmetry::new(SymmetryMode::Rot2)
                .images(MapDims::new(5, 5), 2, 2)
                .len(),
            1
        );
    }
}
