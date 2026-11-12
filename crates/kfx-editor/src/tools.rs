//! Slab tool geometry (brush footprints, lines, boxes, circles, flood fill) and the clipboard
//! (slabs with the things, action points and lights on them, or items alone). Everything works
//! on map coordinates and is independent of the UI.
use crate::document::{Document, SlabState};
use kfx_formats::level_fx::{FxActionPoint, FxLight, FxThing, StlCoord};
use kfx_map::things::ItemRef;
use kfx_map::transform::Transform;
use kfx_prefab::i18n::{tx, Tx};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Paint,
    Line,
    Box,
    Circle,
    Fill,
    Eyedropper,
    Select,
    /// Put down the palette thing.
    Place,
    /// Select, move and edit things / action points / lights.
    Things,
}

impl Tool {
    /// English name (for tests).
    #[cfg(test)]
    pub fn name(self) -> &'static str {
        self.label().en
    }
    /// The name shown in the editor.
    pub fn label(self) -> Tx {
        match self {
            Tool::Paint => tx("kfx_editor.paint_c7b3", "Paint"),
            Tool::Line => tx("kfx_editor.line_f8e7", "Line"),
            Tool::Box => tx("kfx_editor.box_f152", "Box"),
            Tool::Circle => tx("kfx_editor.circle_3ea9", "Circle"),
            Tool::Fill => tx("kfx_editor.fill", "Fill"),
            Tool::Eyedropper => tx("kfx_editor.eyedropper", "Eyedropper"),
            Tool::Select => tx("kfx_editor.select_56ed", "Select"),
            Tool::Place => tx("kfx_editor.place_2d1c", "Place"),
            Tool::Things => tx("kfx_editor.things", "Things"),
        }
    }
    pub fn parse(s: &str) -> Option<Tool> {
        Some(match s.to_ascii_lowercase().as_str() {
            "paint" | "brush" => Tool::Paint,
            "line" => Tool::Line,
            "box" | "rect" => Tool::Box,
            "circle" => Tool::Circle,
            "fill" => Tool::Fill,
            "eyedropper" | "pick" => Tool::Eyedropper,
            "select" => Tool::Select,
            "place" => Tool::Place,
            "things" | "thing" | "thing_select" => Tool::Things,
            _ => return None,
        })
    }
}

pub const MAX_BRUSH: u32 = 9;

/// Offsets of a brush of `size` (1..9) slabs, centred on (0, 0) (even sizes extend one more
/// slab towards +x / +y). Round brushes keep the cells whose centre lies within
/// `size/2 - 1/4` of the brush centre.
pub fn brush_cells(size: u32, round: bool) -> Vec<(i32, i32)> {
    let n = size.clamp(1, MAX_BRUSH) as i32;
    let lo = -(n - 1) / 2;
    let c = (n - 1) as f32 / 2.0;
    let r = n as f32 / 2.0 - 0.25;
    let mut v = vec![];
    for j in 0..n {
        for i in 0..n {
            let (dx, dy) = (i as f32 - c, j as f32 - c);
            if !round || n <= 2 || dx * dx + dy * dy <= r * r + 1e-4 {
                v.push((lo + i, lo + j));
            }
        }
    }
    v
}

/// Bresenham line from `a` to `b`, both included; consecutive cells are 8-connected.
pub fn line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (mut x, mut y) = a;
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    let mut err = dx + dy;
    let mut v = vec![(x, y)];
    while (x, y) != b {
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
        v.push((x, y));
    }
    v
}

/// Cells of a brush stamped along the line `a`..`b` (the interpolated stroke of a drag),
/// without duplicates, in drawing order.
pub fn stroke_cells(a: (i32, i32), b: (i32, i32), brush: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut seen = std::collections::HashSet::new();
    let mut v = vec![];
    for (x, y) in line(a, b) {
        for &(ox, oy) in brush {
            let c = (x + ox, y + oy);
            if seen.insert(c) {
                v.push(c);
            }
        }
    }
    v
}

/// Box between two corners (any order).
pub fn rect_cells(a: (i32, i32), b: (i32, i32), filled: bool) -> Vec<(i32, i32)> {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    let mut v = vec![];
    for y in y0..=y1 {
        for x in x0..=x1 {
            if filled || x == x0 || x == x1 || y == y0 || y == y1 {
                v.push((x, y));
            }
        }
    }
    v
}

fn in_disc(dx: i32, dy: i32, r: i32) -> bool {
    dx * dx + dy * dy <= r * r + r
}

/// Disc of radius `r` slabs around `c`; the outline is the disc's cells that have a
/// 4-neighbour outside it (a closed, gap-free ring).
pub fn circle_cells(c: (i32, i32), r: i32, filled: bool) -> Vec<(i32, i32)> {
    let r = r.max(0);
    let mut v = vec![];
    for dy in -r..=r {
        for dx in -r..=r {
            if !in_disc(dx, dy, r) {
                continue;
            }
            let edge = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|&(ex, ey)| !in_disc(dx + ex, dy + ey, r));
            if filled || edge {
                v.push((c.0 + dx, c.1 + dy));
            }
        }
    }
    v
}

/// Radius of a circle dragged from `c` to `p` (rounded distance in slabs).
pub fn circle_radius(c: (i32, i32), p: (i32, i32)) -> i32 {
    let (dx, dy) = ((p.0 - c.0) as f32, (p.1 - c.1) as f32);
    (dx * dx + dy * dy).sqrt().round() as i32
}

/// 4-connected region of slabs with the same kind and owner as `start`, within the editable
/// area (`allowed`).
pub fn flood_fill(
    doc: &Document,
    start: (i32, i32),
    allowed: impl Fn(i32, i32) -> bool,
) -> Vec<(i32, i32)> {
    let Some(s0) = doc.state_at(start.0, start.1) else {
        return vec![];
    };
    if !allowed(start.0, start.1) {
        return vec![];
    }
    let d = doc.dims();
    let same = |s: SlabState| s.kind == s0.kind && s.owner == s0.owner;
    let mut seen = vec![false; d.slabs()];
    let mut q = VecDeque::from([start]);
    seen[d.slab_index(start.0 as usize, start.1 as usize)] = true;
    let mut out = vec![];
    while let Some((x, y)) = q.pop_front() {
        out.push((x, y));
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if !doc.in_map(nx, ny) || !allowed(nx, ny) {
                continue;
            }
            let i = d.slab_index(nx as usize, ny as usize);
            if seen[i] {
                continue;
            }
            if doc.state_at(nx, ny).is_some_and(same) {
                seen[i] = true;
                q.push_back((nx, ny));
            }
        }
    }
    out
}

/// A copied item with its position relative to the clipboard origin (raw 1/256 subtiles).
#[derive(Clone, Debug, PartialEq)]
pub enum ClipItem {
    Thing(FxThing),
    ActionPoint(FxActionPoint),
    Light(FxLight),
}

/// A copied block of slabs (kind, owner), row-major, and the items on it. A block of size
/// 0 x 0 holds only items, relative to the subtile `(0, 0)` of the stamp point.
#[derive(Clone, Debug, PartialEq)]
pub struct Clipboard {
    pub w: i32,
    pub h: i32,
    pub cells: Vec<(u16, u8)>,
    /// (u, v) raw offset from the block's top-left, item.
    pub items: Vec<(i64, i64, ClipItem)>,
}

fn raw_of(c: Option<StlCoord>) -> Option<i64> {
    c.map(|c| c.stl as i64 * 256 + c.sub as i64)
}

fn coord(r: i64) -> StlCoord {
    StlCoord::new(r.div_euclid(256) as i32, r.rem_euclid(256) as i32)
}

impl ClipItem {
    fn pos(&self) -> Option<(i64, i64)> {
        match self {
            ClipItem::Thing(t) => Some((raw_of(t.x)?, raw_of(t.y)?)),
            ClipItem::ActionPoint(a) => Some((raw_of(a.x)?, raw_of(a.y)?)),
            ClipItem::Light(l) => Some((raw_of(l.x)?, raw_of(l.y)?)),
        }
    }
    /// The item moved to raw position (u, v); things lose their slab parent and hero gate number.
    pub fn at(&self, u: i64, v: i64) -> ClipItem {
        let (x, y) = (Some(coord(u)), Some(coord(v)));
        match self.clone() {
            ClipItem::Thing(mut t) => {
                t.x = x;
                t.y = y;
                t.parent_tile = None;
                t.herogate_number = None;
                ClipItem::Thing(t)
            }
            ClipItem::ActionPoint(mut a) => {
                a.x = x;
                a.y = y;
                a.number = None;
                ClipItem::ActionPoint(a)
            }
            ClipItem::Light(mut l) => {
                l.x = x;
                l.y = y;
                l.parent_tile = None;
                ClipItem::Light(l)
            }
        }
    }
    fn turned(&self, t: Transform) -> ClipItem {
        match self.clone() {
            ClipItem::Thing(mut th) => {
                if let Some(o) = th.orientation {
                    th.orientation = Some(t.angle(o));
                }
                if th.door_orientation.is_some() && t.swaps_axes() {
                    th.door_orientation = Some(1 - th.door_orientation.unwrap_or(0).clamp(0, 1));
                }
                ClipItem::Thing(th)
            }
            x => x,
        }
    }
}

impl Clipboard {
    /// Copy the rectangle between `a` and `b` (inclusive, clipped to the map).
    pub fn copy(doc: &Document, a: (i32, i32), b: (i32, i32)) -> Option<Clipboard> {
        let d = doc.dims();
        let x0 = a.0.min(b.0).max(0);
        let y0 = a.1.min(b.1).max(0);
        let x1 = a.0.max(b.0).min(d.w as i32 - 1);
        let y1 = a.1.max(b.1).min(d.h as i32 - 1);
        if x0 > x1 || y0 > y1 {
            return None;
        }
        let mut cells = vec![];
        for y in y0..=y1 {
            for x in x0..=x1 {
                let s = doc.state_at(x, y)?;
                cells.push((s.kind, s.owner));
            }
        }
        // the items on the block, except what the slabs create themselves (`ParentTile`)
        let r = kfx_map::things::StlRect {
            x0: 3 * x0,
            y0: 3 * y0,
            x1: 3 * x1 + 2,
            y1: 3 * y1 + 2,
        };
        let items = doc
            .doc
            .items_in(r)
            .into_iter()
            .filter_map(|it| Self::clip_item(doc, it))
            .filter(|c| match c {
                ClipItem::Thing(t) => {
                    t.parent_tile.is_none() && t.subtype != Some(kfx_map::things::OBJ_SPINNING_KEY)
                }
                ClipItem::Light(l) => l.parent_tile.is_none(),
                _ => true,
            });
        let (ou, ov) = (x0 as i64 * 768, y0 as i64 * 768);
        let items = items
            .filter_map(|c| c.pos().map(|(u, v)| (u - ou, v - ov, c)))
            .collect();
        Some(Clipboard {
            w: x1 - x0 + 1,
            h: y1 - y0 + 1,
            cells,
            items,
        })
    }

    fn clip_item(doc: &Document, it: ItemRef) -> Option<ClipItem> {
        Some(match it {
            ItemRef::Thing(i) => ClipItem::Thing(doc.doc.things.items.get(i)?.clone()),
            ItemRef::ActionPoint(i) => {
                ClipItem::ActionPoint(doc.doc.action_points.items.get(i)?.clone())
            }
            ItemRef::Light(i) => ClipItem::Light(doc.doc.lights.items.get(i)?.clone()),
        })
    }

    /// Copy items alone (no slabs), relative to the subtile holding the first of them.
    pub fn copy_items(doc: &Document, items: &[ItemRef]) -> Option<Clipboard> {
        let list: Vec<ClipItem> = items
            .iter()
            .filter_map(|&it| Self::clip_item(doc, it))
            .collect();
        let (mu, mv) = list
            .iter()
            .filter_map(ClipItem::pos)
            .fold((i64::MAX, i64::MAX), |(a, b), (u, v)| (a.min(u), b.min(v)));
        if mu == i64::MAX {
            return None;
        }
        let (ou, ov) = (mu.div_euclid(256) * 256, mv.div_euclid(256) * 256);
        let items = list
            .into_iter()
            .filter_map(|c| c.pos().map(|(u, v)| (u - ou, v - ov, c)))
            .collect();
        Some(Clipboard {
            w: 0,
            h: 0,
            cells: vec![],
            items,
        })
    }

    /// Raw size of the area the items live in.
    fn raw_size(&self) -> (i64, i64) {
        if self.w > 0 {
            (self.w as i64 * 768, self.h as i64 * 768)
        } else {
            let (mu, mv) = self
                .items
                .iter()
                .fold((0, 0), |(a, b), (u, v, _)| (a.max(*u), b.max(*v)));
            (
                (mu.div_euclid(256) + 1) * 256,
                (mv.div_euclid(256) + 1) * 256,
            )
        }
    }

    fn transform_items(&self, t: Transform) -> Vec<(i64, i64, ClipItem)> {
        let (wr, hr) = self.raw_size();
        let (nw, nh) = if t.swaps_axes() { (hr, wr) } else { (wr, hr) };
        self.items
            .iter()
            .map(|(u, v, c)| {
                let (a, b) = t.raw(wr, hr, *u, *v);
                (a.min(nw - 1), b.min(nh - 1), c.turned(t))
            })
            .collect()
    }

    pub fn get(&self, x: i32, y: i32) -> (u16, u8) {
        self.cells[(y * self.w + x) as usize]
    }

    /// Rotate 90 degrees clockwise (as seen with north up).
    pub fn rotate(&self) -> Clipboard {
        let (w, h) = (self.h, self.w);
        let mut cells = Vec::with_capacity(self.cells.len());
        for y in 0..h {
            for x in 0..w {
                // new (x, y) comes from old (y, H-1-x)
                cells.push(self.get(y, self.h - 1 - x));
            }
        }
        Clipboard {
            w,
            h,
            cells,
            items: self.transform_items(Transform::Rot90),
        }
    }

    /// Mirror left-right.
    pub fn flip(&self) -> Clipboard {
        let mut cells = Vec::with_capacity(self.cells.len());
        for y in 0..self.h {
            for x in 0..self.w {
                cells.push(self.get(self.w - 1 - x, y));
            }
        }
        Clipboard {
            w: self.w,
            h: self.h,
            cells,
            items: self.transform_items(Transform::FlipX),
        }
    }

    /// Top-left slab when the block is centred on `at`.
    pub fn origin(&self, at: (i32, i32)) -> (i32, i32) {
        (at.0 - (self.w - 1) / 2, at.1 - (self.h - 1) / 2)
    }

    /// The items when the block is stamped centred on slab `at` (items-only clipboards: at
    /// subtile `stl`), at their new raw positions.
    pub fn stamp_items(&self, at: (i32, i32), stl: (i32, i32)) -> Vec<ClipItem> {
        let (ou, ov) = if self.w > 0 {
            let (ox, oy) = self.origin(at);
            (ox as i64 * 768, oy as i64 * 768)
        } else {
            (stl.0 as i64 * 256, stl.1 as i64 * 256)
        };
        self.items
            .iter()
            .map(|(u, v, c)| c.at(ou + u, ov + v))
            .collect()
    }

    /// (x, y, kind, owner) of every cell when stamped centred on `at`.
    pub fn stamp(&self, at: (i32, i32)) -> Vec<(i32, i32, u16, u8)> {
        let (ox, oy) = self.origin(at);
        let mut v = vec![];
        for y in 0..self.h {
            for x in 0..self.w {
                let (k, o) = self.get(x, y);
                v.push((ox + x, oy + y, k, o));
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected8(v: &[(i32, i32)]) -> bool {
        v.windows(2)
            .all(|w| (w[0].0 - w[1].0).abs() <= 1 && (w[0].1 - w[1].1).abs() <= 1)
    }

    #[test]
    fn brushes() {
        assert_eq!(brush_cells(1, false), vec![(0, 0)]);
        assert_eq!(brush_cells(1, true), vec![(0, 0)]);
        assert_eq!(brush_cells(3, false).len(), 9);
        // a round brush of size 3 is a plus shape
        let r3 = brush_cells(3, true);
        assert_eq!(r3.len(), 5);
        assert!(!r3.contains(&(1, 1)) && r3.contains(&(0, 1)));
        assert_eq!(brush_cells(2, false), vec![(0, 0), (1, 0), (0, 1), (1, 1)]);
        for n in 1..=9 {
            let s = brush_cells(n, false);
            let r = brush_cells(n, true);
            assert_eq!(s.len(), (n * n) as usize);
            assert!(r.len() <= s.len() && r.iter().all(|c| s.contains(c)));
            // symmetric for odd sizes
            if n % 2 == 1 {
                assert!(r
                    .iter()
                    .all(|&(x, y)| r.contains(&(-x, y)) && r.contains(&(y, x))));
                assert!(r.contains(&(0, 0)));
            }
        }
        assert_eq!(brush_cells(20, false).len(), 81);
    }

    #[test]
    fn lines_are_gap_free() {
        for &(a, b) in &[
            ((0, 0), (10, 3)),
            ((5, 5), (-3, 12)),
            ((0, 0), (0, 0)),
            ((2, 9), (2, -4)),
            ((0, 0), (7, 7)),
        ] {
            let l = line(a, b);
            assert_eq!(l[0], a);
            assert_eq!(*l.last().unwrap(), b);
            assert!(connected8(&l), "{a:?}->{b:?}: {l:?}");
            assert_eq!(l.len() as i32, (b.0 - a.0).abs().max((b.1 - a.1).abs()) + 1);
        }
    }

    #[test]
    fn fast_drag_interpolates() {
        // two mouse samples 20 slabs apart: every slab in between is painted
        let brush = brush_cells(1, false);
        let s = stroke_cells((0, 0), (20, 5), &brush);
        assert_eq!(s.len(), 21);
        assert!(connected8(&s));
        // a 3x3 brush leaves a 3 wide band without holes
        let b3 = brush_cells(3, false);
        let s = stroke_cells((0, 0), (12, 0), &b3);
        for x in -1..=13 {
            for y in -1..=1 {
                assert!(s.contains(&(x, y)), "({x},{y})");
            }
        }
        assert_eq!(s.len(), 15 * 3, "no duplicates");
    }

    #[test]
    fn boxes() {
        assert_eq!(rect_cells((3, 3), (1, 1), true).len(), 9);
        let o = rect_cells((0, 0), (4, 3), false);
        assert_eq!(o.len(), 2 * 5 + 2 * 2);
        assert!(!o.contains(&(2, 1)));
        assert_eq!(rect_cells((2, 2), (2, 2), false), vec![(2, 2)]);
    }

    #[test]
    fn circles() {
        assert_eq!(circle_cells((5, 5), 0, true), vec![(5, 5)]);
        let f = circle_cells((0, 0), 4, true);
        let o = circle_cells((0, 0), 4, false);
        assert!(o.iter().all(|c| f.contains(c)));
        assert!(f.contains(&(4, 0)) && f.contains(&(0, -4)) && !f.contains(&(4, 4)));
        assert!(!o.contains(&(0, 0)) && o.contains(&(4, 0)));
        // the ring is closed: every interior cell's 4-neighbours stay in the disc
        for &(x, y) in f.iter().filter(|c| !o.contains(c)) {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                assert!(f.contains(&(x + dx, y + dy)));
            }
        }
        assert_eq!(circle_radius((0, 0), (3, 4)), 5);
    }

    #[test]
    fn clipboard_rotate_flip() {
        // 3x2 block
        // a b c
        // d e f
        let c = Clipboard {
            w: 3,
            h: 2,
            cells: vec![(1, 0), (2, 0), (3, 0), (4, 1), (5, 1), (6, 1)],
            items: vec![],
        };
        let r = c.rotate();
        assert_eq!((r.w, r.h), (2, 3));
        // clockwise: d a / e b / f c
        assert_eq!(
            r.cells.iter().map(|x| x.0).collect::<Vec<_>>(),
            vec![4, 1, 5, 2, 6, 3]
        );
        assert_eq!(c.rotate().rotate().rotate().rotate(), c);
        let f = c.flip();
        assert_eq!(
            f.cells.iter().map(|x| x.0).collect::<Vec<_>>(),
            vec![3, 2, 1, 6, 5, 4]
        );
        assert_eq!(f.flip(), c);
        // flip then rotate twice = vertical flip
        let v = c.flip().rotate().rotate();
        assert_eq!(
            v.cells.iter().map(|x| x.0).collect::<Vec<_>>(),
            vec![4, 5, 6, 1, 2, 3]
        );
        let st = c.stamp((10, 10));
        assert_eq!(st[0], (9, 10, 1, 0));
        assert_eq!(st[5], (11, 11, 6, 1));
    }

    /// Items rotate and flip with the block (positions stay inside, facings turn).
    #[test]
    fn clipboard_items_follow() {
        let t = FxThing {
            x: Some(StlCoord::new(0, 128)),
            y: Some(StlCoord::new(1, 128)),
            orientation: Some(0),
            ..Default::default()
        };
        let c = Clipboard {
            w: 2,
            h: 1,
            cells: vec![(1, 0), (2, 0)],
            items: vec![(128, 384, ClipItem::Thing(t))],
        };
        let r = c.rotate();
        let (u, v, ClipItem::Thing(rt)) = &r.items[0] else {
            panic!()
        };
        // 2x1 slabs (1536x768 raw) turned clockwise: (128, 384) -> (768 - 384, 128)
        assert_eq!((*u, *v), (384, 128));
        assert_eq!(rt.orientation, Some(512));
        let f = c.flip();
        assert_eq!((f.items[0].0, f.items[0].1), (1536 - 128, 384));
        assert_eq!(c.rotate().rotate().rotate().rotate(), c);
        let st = c.stamp_items((10, 10), (0, 0));
        let ClipItem::Thing(s) = &st[0] else { panic!() };
        // origin slab (10, 10): raw 7680 + 128
        assert_eq!(s.x, Some(StlCoord::new(30, 128)));
    }

    #[test]
    fn flood_fill_bounds() {
        let Some((_, mut d)) = crate::document::tests::load_shipped(1) else {
            return;
        };
        let dims = d.dims();
        // carve a closed 5x4 lava pool, fill inside it
        d.begin_stroke("pool");
        let ring: Vec<_> = rect_cells((20, 20), (26, 25), false)
            .into_iter()
            .map(|(x, y)| (x, y, crate::document::Edit::Place { kind: 0, owner: 5 }))
            .collect();
        let inner: Vec<_> = rect_cells((21, 21), (25, 24), true)
            .into_iter()
            .map(|(x, y)| (x, y, crate::document::Edit::Place { kind: 12, owner: 5 }))
            .collect();
        d.apply(&ring);
        d.apply(&inner);
        d.end_stroke();
        let f = flood_fill(&d, (23, 22), |_, _| true);
        assert_eq!(f.len(), 5 * 4);
        assert!(f
            .iter()
            .all(|&(x, y)| (21..=25).contains(&x) && (21..=24).contains(&y)));
        // the outer rock ring of the map: a fill started on it stays in the map
        let all = flood_fill(&d, (0, 0), |_, _| true);
        assert!(all
            .iter()
            .all(|&(x, y)| x >= 0 && y >= 0 && x < dims.w as i32 && y < dims.h as i32));
        assert!(!all.is_empty());
        // blocked border: nothing on the outer ring
        let inner_only = flood_fill(&d, (0, 0), |x, y| {
            x > 0 && y > 0 && x < dims.w as i32 - 1 && y < dims.h as i32 - 1
        });
        assert!(inner_only.is_empty());
        let none = flood_fill(&d, (-1, 3), |_, _| true);
        assert!(none.is_empty());
    }
}
