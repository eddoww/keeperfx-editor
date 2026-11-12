//! Rotating and flipping a prefab (in memory, before it is applied).
//!
//! Same rules as `kfx_map::transform` for a rectangle of the map: slab cells move, positions move
//! exactly on the 1/256 subtile grid, `Orientation` angles turn with the prefab, door
//! orientation flips under quarter turns, custom slab columns / objects are moved to where the
//! turn puts them (cube faces are direction specific and are not rotated). The flip is applied
//! first (mirror east <-> west), then the clockwise rotation.
use crate::model::*;
use kfx_map::transform::Transform;

/// Clockwise rotation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rotation {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    pub const ALL: [Rotation; 4] = [Rotation::R0, Rotation::R90, Rotation::R180, Rotation::R270];
    pub fn degrees(self) -> u32 {
        match self {
            Rotation::R0 => 0,
            Rotation::R90 => 90,
            Rotation::R180 => 180,
            Rotation::R270 => 270,
        }
    }
    /// One more quarter turn clockwise.
    pub fn next(self) -> Rotation {
        match self {
            Rotation::R0 => Rotation::R90,
            Rotation::R90 => Rotation::R180,
            Rotation::R180 => Rotation::R270,
            Rotation::R270 => Rotation::R0,
        }
    }
}

/// Flip (east <-> west mirror) then rotation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Orientation {
    pub rotation: Rotation,
    pub flip: bool,
}

impl Orientation {
    pub fn is_identity(&self) -> bool {
        self.rotation == Rotation::R0 && !self.flip
    }
    /// The `kfx_map` transforms to apply in order.
    pub fn steps(&self) -> Vec<Transform> {
        let mut v = vec![];
        if self.flip {
            v.push(Transform::FlipX);
        }
        match self.rotation {
            Rotation::R0 => {}
            Rotation::R90 => v.push(Transform::Rot90),
            Rotation::R180 => v.push(Transform::Rot180),
            Rotation::R270 => v.push(Transform::Rot270),
        }
        v
    }
    /// Size of a `w x h` prefab after the orientation.
    pub fn size(&self, w: u32, h: u32) -> (u32, u32) {
        match self.rotation {
            Rotation::R90 | Rotation::R270 => (h, w),
            _ => (w, h),
        }
    }
}

impl Prefab {
    /// The prefab turned / flipped.
    pub fn transformed(&self, o: Orientation) -> Prefab {
        let mut p = self.clone();
        for t in o.steps() {
            p = p.transformed_by(t);
        }
        p
    }

    fn transformed_by(&self, t: Transform) -> Prefab {
        let (w, h) = (self.w as i64, self.h as i64);
        let (nw, nh) = if t.swaps_axes() { (h, w) } else { (w, h) };
        let mut out = self.clone();
        out.w = nw as u32;
        out.h = nh as u32;
        out.cells = vec![None; (nw * nh) as usize];
        // custom definitions: one transformed copy per original, named after the turn
        let mut renamed: std::collections::HashMap<String, String> = Default::default();
        let mut defs: Vec<CustomSlabDef> = vec![];
        for d in &self.custom_slabs {
            let n = transform_def(d, t);
            let name = if n == *d {
                d.name.clone()
            } else {
                let mut n = n;
                n.name = format!("{} [{t:?}]", d.name);
                n.name.clone()
            };
            renamed.insert(d.name.clone(), name.clone());
            let mut nd = transform_def(d, t);
            nd.name = name;
            if !defs.iter().any(|x| x.name == nd.name) {
                defs.push(nd);
            }
        }
        out.custom_slabs = defs;
        for y in 0..h {
            for x in 0..w {
                let (nx, ny) = t.slab(w, h, x, y);
                let mut c = self.cells[(y * w + x) as usize].clone();
                if let Some(c) = c.as_mut() {
                    if let Some(n) = &c.custom {
                        c.custom = renamed.get(n).cloned();
                    }
                }
                out.cells[(ny * nw + nx) as usize] = c;
            }
        }
        let (wr, hr) = (w * RAW_PER_SLAB as i64, h * RAW_PER_SLAB as i64);
        let (nwr, nhr) = if t.swaps_axes() { (hr, wr) } else { (wr, hr) };
        let map = |u: i32, v: i32| {
            let (a, b) = t.raw(wr, hr, u as i64, v as i64);
            (a.clamp(0, nwr - 1) as i32, b.clamp(0, nhr - 1) as i32)
        };
        for th in &mut out.things {
            let (a, b) = map(th.x, th.y);
            th.x = a;
            th.y = b;
            if let Some(o) = th.orientation {
                th.orientation = Some(t.angle(o));
            }
            if th.class == ThingClass::Door && t.swaps_axes() {
                th.door_orientation = th.door_orientation.map(|d| 1 - d.clamp(0, 1));
            }
        }
        for a in &mut out.action_points {
            let (x, y) = map(a.x, a.y);
            a.x = x;
            a.y = y;
        }
        for l in &mut out.lights {
            let (x, y) = map(l.x, l.y);
            l.x = x;
            l.y = y;
        }
        out
    }
}

/// Custom slab definition `d` after the transform `t` of the slab it stands on.
fn transform_def(d: &CustomSlabDef, t: Transform) -> CustomSlabDef {
    let mut out = d.clone();
    for y in 0..3i64 {
        for x in 0..3i64 {
            let (nx, ny) = t.slab(3, 3, x, y);
            let (a, b) = ((y * 3 + x) as usize, (ny * 3 + nx) as usize);
            out.columns[b] = d.columns[a].clone();
            out.wibble[b] = d.wibble[a];
        }
    }
    for o in &mut out.objects {
        let (a, b) = t.raw(768, 768, o.dx as i64, o.dy as i64);
        o.dx = a.clamp(0, 767) as i32;
        o.dy = b.clamp(0, 767) as i32;
        o.orientation = t.angle(o.orientation as i64) as i16;
    }
    out
}
