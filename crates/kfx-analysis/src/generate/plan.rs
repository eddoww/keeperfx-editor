//! The plan of a generated map: slab kinds and owners per slab, keeper bases, doors and things,
//! with the symmetry ([`Sym`]) that maps every feature to its images.
use super::*;

/// A door to place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanDoor {
    pub slab: (i32, i32),
    pub model: i64,
    pub owner: u8,
    pub locked: bool,
}

/// What a planned thing is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanThingKind {
    Creature { model: i64, level: i64 },
    Object { model: i64 },
    HeroGate { number: i64 },
}

/// A thing to place at a slab centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanThing {
    pub kind: PlanThingKind,
    pub owner: u8,
    pub slab: (i32, i32),
}

/// A keeper's base.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Base {
    pub owner: u8,
    pub heart: (i32, i32),
    pub portal: Option<(i32, i32)>,
    /// Direction from the map centre, in turns (for river arms).
    pub angle: f64,
}

/// The layout of a map before it becomes a document.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub w: i32,
    pub h: i32,
    pub kinds: Vec<u16>,
    pub owners: Vec<u8>,
    pub doors: Vec<PlanDoor>,
    pub things: Vec<PlanThing>,
    pub bases: Vec<Base>,
    pub sym: Sym,
    /// Cells a later stamp must not change (base, portal, fortress interior).
    pub locked: Vec<bool>,
}

/// Symmetry of a plan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sym {
    pub mode: Option<SymmetryMode>,
    /// `n > 1`: radial symmetry of order n.
    pub radial: u32,
    pub phase: f64,
    pub w: i32,
    pub h: i32,
}

impl Sym {
    pub(super) fn transforms(&self) -> Vec<Transform> {
        match self.mode {
            Some(SymmetryMode::NorthSouth) => vec![Transform::FlipY],
            Some(SymmetryMode::EastWest) => vec![Transform::FlipX],
            Some(SymmetryMode::FourWay) => {
                vec![Transform::FlipX, Transform::FlipY, Transform::Rot180]
            }
            Some(SymmetryMode::Rot2) => vec![Transform::Rot180],
            Some(SymmetryMode::Rot4) => {
                vec![Transform::Rot90, Transform::Rot180, Transform::Rot270]
            }
            None => vec![],
        }
    }
    /// Orbit size of a general point.
    pub fn order(&self) -> usize {
        if self.radial > 1 {
            self.radial as usize
        } else {
            1 + self.transforms().len()
        }
    }
    pub(super) fn centre(&self) -> (f64, f64) {
        ((self.w - 1) as f64 / 2.0, (self.h - 1) as f64 / 2.0)
    }
    /// Images of a continuous slab position (itself first), in orbit order.
    pub fn point_images(&self, p: (f64, f64)) -> Vec<(f64, f64)> {
        let (w, h) = ((self.w - 1) as f64, (self.h - 1) as f64);
        let mut out = vec![p];
        if self.radial > 1 {
            let (cx, cy) = self.centre();
            for k in 1..self.radial {
                let (c, s) = cos_sin(k as f64 / self.radial as f64);
                let (dx, dy) = (p.0 - cx, p.1 - cy);
                out.push((cx + dx * c - dy * s, cy + dx * s + dy * c));
            }
            return out;
        }
        for t in self.transforms() {
            out.push(match t {
                Transform::FlipX => (w - p.0, p.1),
                Transform::FlipY => (p.0, h - p.1),
                Transform::Rot90 => (h - p.1, p.0),
                Transform::Rot180 => (w - p.0, h - p.1),
                Transform::Rot270 => (p.1, w - p.0),
            });
        }
        out
    }
    /// Angle (turns) of the image `k` of a direction at angle `a`.
    pub(super) fn angle_image(&self, a: f64, k: usize) -> f64 {
        if self.radial > 1 {
            return a + k as f64 / self.radial as f64;
        }
        if k == 0 {
            return a;
        }
        match self.transforms()[k - 1] {
            Transform::FlipX => 0.5 - a,
            Transform::FlipY => -a,
            Transform::Rot90 => a + 0.25,
            Transform::Rot180 => a + 0.5,
            Transform::Rot270 => a + 0.75,
        }
    }
    /// The representative of slab (x, y) for noise (exact modes: the smallest image; radial:
    /// rotated back into the first sector).
    pub fn rep(&self, x: i32, y: i32) -> (i32, i32) {
        if self.radial > 1 {
            let (cx, cy) = self.centre();
            let (dx, dy) = (x as f64 - cx, y as f64 - cy);
            let mut best = (f64::MIN, 0u32);
            for k in 0..self.radial {
                let (c, s) = cos_sin(self.phase + k as f64 / self.radial as f64);
                let d = dx * c + dy * s;
                if d > best.0 {
                    best = (d, k);
                }
            }
            let (c, s) = cos_sin(best.1 as f64 / self.radial as f64);
            return (round(cx + dx * c + dy * s), round(cy - dx * s + dy * c));
        }
        let mut best = (x, y);
        for (a, b) in self.point_images((x as f64, y as f64)).into_iter().skip(1) {
            let p = (round(a), round(b));
            if (p.1, p.0) < (best.1, best.0) {
                best = p;
            }
        }
        best
    }
}

/// Slab kinds the generator uses (from terrain.cfg names, falling back to the hard-coded ids).
#[derive(Clone, Debug)]
pub struct Kinds {
    pub rock: u16,
    pub gold: u16,
    pub earth: u16,
    pub path: u16,
    pub claimed: u16,
    pub lava: u16,
    pub water: u16,
    pub gems: u16,
    pub heart: u16,
    pub portal: u16,
    pub walls: Vec<u16>,
}

pub(super) fn slab(gd: &GameData, name: &str, fallback: u8) -> u16 {
    gd.slab_id(name)
        .map(|k| k as u16)
        .unwrap_or(fallback as u16)
}

pub(super) fn room_slab(gd: &GameData, room: &str) -> Option<u16> {
    gd.room_id(room)
        .map(|r| gd.rooms[r].assigned_slab as u16)
        .filter(|&k| k > 0)
}

impl Kinds {
    pub fn new(gd: &GameData) -> Kinds {
        Kinds {
            rock: slab(gd, "HARD", kinds::ROCK),
            gold: slab(gd, "GOLD", kinds::GOLD),
            earth: slab(gd, "DIRT", kinds::EARTH),
            path: slab(gd, "PATH", kinds::PATH),
            claimed: slab(gd, "PRETTY_PATH", kinds::CLAIMED),
            lava: slab(gd, "LAVA", kinds::LAVA),
            water: slab(gd, "WATER", kinds::WATER),
            gems: slab(gd, "GEMS", kinds::GEMS),
            heart: room_slab(gd, "DUNGEON_HEART").unwrap_or(slab(gd, "HEART_PEDESTAL", 0)),
            portal: room_slab(gd, "ENTRANCE").unwrap_or(slab(gd, "ENTRANCE_ZONE", 0)),
            walls: [
                "DRAPE_WALL",
                "TWINS_WALL",
                "WOMAN_WALL",
                "PAIR_WALL",
                "TORCH_WALL",
            ]
            .iter()
            .filter_map(|n| gd.slab_id(n).map(|k| k as u16))
            .collect(),
        }
    }
}

/// A shape in slab coordinates (continuous).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Shape {
    Disk {
        c: (f64, f64),
        r: f64,
    },
    /// A disk with a noisy edge.
    Blob {
        c: (f64, f64),
        r: f64,
        seed: u64,
    },
    Capsule {
        a: (f64, f64),
        b: (f64, f64),
        r: f64,
    },
    Rect {
        a: (f64, f64),
        b: (f64, f64),
    },
}

impl Plan {
    pub(super) fn new(w: i32, h: i32, fill: u16, sym: Sym) -> Plan {
        let n = (w * h) as usize;
        Plan {
            w,
            h,
            kinds: vec![fill; n],
            owners: vec![PLAYER_NEUTRAL; n],
            doors: vec![],
            things: vec![],
            bases: vec![],
            sym,
            locked: vec![false; n],
        }
    }
    pub(super) fn idx(&self, x: i32, y: i32) -> usize {
        (y * self.w + x) as usize
    }
    pub(super) fn inside(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.w && y < self.h
    }
    /// Inside, off the border ring.
    pub(super) fn interior(&self, x: i32, y: i32) -> bool {
        x >= 1 && y >= 1 && x < self.w - 1 && y < self.h - 1
    }
    pub fn kind(&self, x: i32, y: i32) -> u16 {
        self.kinds[self.idx(x, y)]
    }
    pub(super) fn set(&mut self, x: i32, y: i32, k: u16, o: u8) {
        if self.interior(x, y) {
            let i = self.idx(x, y);
            if !self.locked[i] {
                self.kinds[i] = k;
                self.owners[i] = o;
            }
        }
    }
    pub(super) fn force(&mut self, x: i32, y: i32, k: u16, o: u8) {
        if self.interior(x, y) {
            let i = self.idx(x, y);
            self.kinds[i] = k;
            self.owners[i] = o;
        }
    }

    /// Cells of a shape (clipped to the map).
    pub(super) fn cells(&self, s: Shape) -> Vec<(i32, i32)> {
        let (x0, y0, x1, y1) = match s {
            Shape::Disk { c, r } | Shape::Blob { c, r, .. } => {
                (c.0 - r - 1.0, c.1 - r - 1.0, c.0 + r + 1.0, c.1 + r + 1.0)
            }
            Shape::Capsule { a, b, r } => (
                a.0.min(b.0) - r - 1.0,
                a.1.min(b.1) - r - 1.0,
                a.0.max(b.0) + r + 1.0,
                a.1.max(b.1) + r + 1.0,
            ),
            Shape::Rect { a, b } => (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)),
        };
        let (x0, y0, x1, y1) = (
            (x0 as i32 - 1).max(0),
            (y0 as i32 - 1).max(0),
            (x1 as i32 + 1).min(self.w - 1),
            (y1 as i32 + 1).min(self.h - 1),
        );
        let mut out = vec![];
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (fx, fy) = (x as f64, y as f64);
                let inside = match s {
                    Shape::Disk { c, r } => {
                        (fx - c.0) * (fx - c.0) + (fy - c.1) * (fy - c.1) <= r * r
                    }
                    Shape::Blob { c, r, seed } => {
                        let (rx, ry) = self.sym.rep(x, y);
                        let n = fractal(seed, rx as i64, ry as i64, 3) as f64 / 65535.0;
                        let rr = r * (0.7 + 0.6 * n);
                        (fx - c.0) * (fx - c.0) + (fy - c.1) * (fy - c.1) <= rr * rr
                    }
                    Shape::Capsule { a, b, r } => {
                        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                        let len2 = dx * dx + dy * dy;
                        let t = if len2 <= 0.0 {
                            0.0
                        } else {
                            (((fx - a.0) * dx + (fy - a.1) * dy) / len2).clamp(0.0, 1.0)
                        };
                        let (px, py) = (a.0 + t * dx, a.1 + t * dy);
                        (fx - px) * (fx - px) + (fy - py) * (fy - py) <= r * r
                    }
                    Shape::Rect { a, b } => {
                        fx >= a.0.min(b.0) - 1e-9
                            && fx <= a.0.max(b.0) + 1e-9
                            && fy >= a.1.min(b.1) - 1e-9
                            && fy <= a.1.max(b.1) + 1e-9
                    }
                };
                if inside {
                    out.push((x, y));
                }
            }
        }
        out
    }

    pub(super) fn image_shapes(&self, s: Shape) -> Vec<Shape> {
        match s {
            Shape::Disk { c, r } => self
                .sym
                .point_images(c)
                .into_iter()
                .map(|c| Shape::Disk { c, r })
                .collect(),
            Shape::Blob { c, r, seed } => self
                .sym
                .point_images(c)
                .into_iter()
                .map(|c| Shape::Blob { c, r, seed })
                .collect(),
            Shape::Capsule { a, b, r } => {
                let (ia, ib) = (self.sym.point_images(a), self.sym.point_images(b));
                ia.into_iter()
                    .zip(ib)
                    .map(|(a, b)| Shape::Capsule { a, b, r })
                    .collect()
            }
            Shape::Rect { a, b } => {
                let (ia, ib) = (self.sym.point_images(a), self.sym.point_images(b));
                ia.into_iter()
                    .zip(ib)
                    .map(|(a, b)| Shape::Rect { a, b })
                    .collect()
            }
        }
    }

    /// Cells of a shape and all its images.
    pub(super) fn stamp_cells(&self, s: Shape) -> Vec<(i32, i32)> {
        let mut v: Vec<(i32, i32)> = self
            .image_shapes(s)
            .into_iter()
            .flat_map(|s| self.cells(s))
            .collect();
        v.sort_by_key(|&(x, y)| (y, x));
        v.dedup();
        v
    }

    /// Set every cell of the shape and its images for which `keep(current kind)` holds.
    pub(super) fn stamp(&mut self, s: Shape, k: u16, o: u8, keep: impl Fn(u16) -> bool) {
        for (x, y) in self.stamp_cells(s) {
            if keep(self.kind(x, y)) {
                self.set(x, y, k, o);
            }
        }
    }

    /// Integer slab images of a slab (itself first, duplicates removed).
    pub(super) fn slab_images(&self, p: (i32, i32)) -> Vec<(i32, i32)> {
        let mut v: Vec<(i32, i32)> = vec![];
        for (a, b) in self.sym.point_images((p.0 as f64, p.1 as f64)) {
            let q = (round(a), round(b));
            if !v.contains(&q) {
                v.push(q);
            }
        }
        v
    }

    /// Copy every non-keeper slab from its representative (exact modes).
    pub(super) fn symmetrize(&mut self) {
        if self.sym.mode.is_none() {
            return;
        }
        let src_k = self.kinds.clone();
        let src_o = self.owners.clone();
        for y in 0..self.h {
            for x in 0..self.w {
                let (rx, ry) = self.sym.rep(x, y);
                let (i, j) = (self.idx(x, y), self.idx(rx, ry));
                if !sites::is_keeper(src_o[i]) && !sites::is_keeper(src_o[j]) {
                    self.kinds[i] = src_k[j];
                    self.owners[i] = src_o[j];
                }
            }
        }
    }

    pub(super) fn open_at(&self, gd: &GameData, x: i32, y: i32) -> bool {
        self.inside(x, y) && crate::terrain::classify(gd, self.kind(x, y) as usize).is_open()
    }
}
