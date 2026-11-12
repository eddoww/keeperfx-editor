//! Movement profiles and distance fields (Dijkstra over the slab grid, costs in game turns).
//!
//! Costs (see [`crate::rules::Rules`]): entering an open slab costs `walk_turns` (diagonal steps
//! 1.4x, only between open slabs whose two orthogonal neighbours are open too, so routes do not cut
//! wall corners); a slab that must be dug first costs `walk_turns + dig_turns` (orthogonal steps
//! only: imps dig the face they stand at); an enemy door costs `walk_turns + Health /
//! door_turns_per_health_div`. One imp is modelled: a keeper with many imps digs faster, so the
//! times compare maps and players, they are not a stopwatch.
use crate::rules::Rules;
use crate::terrain::{Class, Terrain};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Unreachable.
pub const INF: u32 = u32::MAX;

/// How something moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mover {
    /// A keeper's creatures walking: open safe slabs, own unlocked doors.
    Walk { player: u8 },
    /// A keeper's imps: walking plus digging what `player` may dig, enemy doors broken.
    Dig { player: u8 },
    /// [`Mover::Dig`] plus lava (flying or lava-proof creatures, or a bridge).
    DigCross { player: u8 },
    /// [`Mover::Dig`] plus other keepers' fortified walls, at four times the dig time of an own
    /// wall: imps cannot dig them (`subtile_is_diggable_for_player` src/map_data.c:830), it takes a
    /// Destroy Walls power or a possessed creature (`BREAK_NEUTRAL_WALLS`,
    /// src/creature_instances.c:933). Used for heart-to-heart routes when no plain dig route exists.
    Siege { player: u8 },
    /// Heroes: walking, keeper doors broken; `tunnel` adds digging through plain earth like a
    /// tunneller (`DIGGING_CREATURE`, wallhugs around `FILLED|VALUABLE` slabs,
    /// `get_next_position_and_angle_required_to_tunnel_creature_to` src/ariadne_wallhug.c:1629).
    Hero { tunnel: bool },
}

impl Mover {
    fn player(self) -> u8 {
        match self {
            Mover::Walk { player }
            | Mover::Dig { player }
            | Mover::DigCross { player }
            | Mover::Siege { player } => player,
            Mover::Hero { .. } => crate::sites::HEROES,
        }
    }

    /// Cost of entering `c` with an orthogonal step, `None` when it cannot.
    pub fn enter_cost(self, c: &crate::terrain::Cell, r: &Rules) -> Option<u32> {
        let p = self.player();
        let walk = r.walk_turns;
        match c.class {
            Class::Floor | Class::Room | Class::Water => Some(walk),
            Class::Lava => matches!(self, Mover::DigCross { .. }).then_some(walk),
            Class::Door => {
                if c.owner == p {
                    (!c.locked).then_some(walk)
                } else if matches!(self, Mover::Walk { .. }) {
                    None
                } else {
                    Some(walk + c.door_health / r.door_turns_per_health_div.max(1))
                }
            }
            Class::Earth | Class::Gold | Class::Wall => match self {
                Mover::Dig { .. } | Mover::DigCross { .. } | Mover::Siege { .. }
                    if Terrain::diggable_by(c, p) =>
                {
                    Some(walk + r.dig_turns(c.health, c.owner == p))
                }
                Mover::Siege { .. } if c.class == Class::Wall => {
                    Some(walk + 4 * r.dig_turns(c.health, true))
                }
                Mover::Hero { tunnel: true } if c.class == Class::Earth => {
                    Some(walk + r.dig_turns(c.health, false))
                }
                _ => None,
            },
            Class::Rock | Class::Gems => None,
        }
    }

    /// Whether `c` is entered without digging or breaking anything.
    fn open(self, c: &crate::terrain::Cell, r: &Rules) -> bool {
        match c.class {
            Class::Floor | Class::Room | Class::Water => true,
            Class::Lava => matches!(self, Mover::DigCross { .. }),
            Class::Door => self.enter_cost(c, r) == Some(r.walk_turns),
            _ => false,
        }
    }
}

/// Distances (game turns) from a set of start slabs, with the route back.
#[derive(Clone, Debug, PartialEq)]
pub struct DistanceField {
    pub w: i32,
    pub h: i32,
    pub mover: Mover,
    pub dist: Vec<u32>,
    /// Previous slab index on the best route (`u32::MAX` for starts and unreached slabs).
    pub prev: Vec<u32>,
}

const ORTHO: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
const DIAG: [(i32, i32); 4] = [(1, -1), (1, 1), (-1, 1), (-1, -1)];

impl DistanceField {
    /// Dijkstra from `starts` (slabs outside the map are ignored). Start slabs cost nothing even
    /// when solid.
    pub fn compute(t: &Terrain, r: &Rules, mover: Mover, starts: &[(i32, i32)]) -> DistanceField {
        let n = t.len();
        let mut dist = vec![INF; n];
        let mut prev = vec![u32::MAX; n];
        let mut heap = BinaryHeap::new();
        // entering costs, computed once
        let cost: Vec<u32> = t
            .cells
            .iter()
            .map(|c| mover.enter_cost(c, r).unwrap_or(INF))
            .collect();
        let open: Vec<bool> = t.cells.iter().map(|c| mover.open(c, r)).collect();
        for &(x, y) in starts {
            if t.inside(x, y) {
                let i = t.idx(x, y);
                if dist[i] != 0 {
                    dist[i] = 0;
                    heap.push(Reverse((0u32, i as u32)));
                }
            }
        }
        while let Some(Reverse((d, i))) = heap.pop() {
            let i = i as usize;
            if d > dist[i] {
                continue;
            }
            let (x, y) = t.xy(i);
            for (dx, dy) in ORTHO {
                let (nx, ny) = (x + dx, y + dy);
                if !t.inside(nx, ny) {
                    continue;
                }
                let j = t.idx(nx, ny);
                if cost[j] == INF {
                    continue;
                }
                let nd = d.saturating_add(cost[j]);
                if nd < dist[j] {
                    dist[j] = nd;
                    prev[j] = i as u32;
                    heap.push(Reverse((nd, j as u32)));
                }
            }
            if !open[i] && d != 0 {
                continue;
            }
            for (dx, dy) in DIAG {
                let (nx, ny) = (x + dx, y + dy);
                if !t.inside(nx, ny) {
                    continue;
                }
                let j = t.idx(nx, ny);
                if !open[j] || !open[t.idx(x + dx, y)] || !open[t.idx(x, y + dy)] {
                    continue;
                }
                let nd = d.saturating_add(cost[j] * 14 / 10);
                if nd < dist[j] {
                    dist[j] = nd;
                    prev[j] = i as u32;
                    heap.push(Reverse((nd, j as u32)));
                }
            }
        }
        DistanceField {
            w: t.w,
            h: t.h,
            mover,
            dist,
            prev,
        }
    }

    pub fn at(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return INF;
        }
        self.dist[(y * self.w + x) as usize]
    }

    /// The best of `targets`: `(slab, distance)`, `None` when none is reached.
    pub fn nearest(&self, targets: &[(i32, i32)]) -> Option<((i32, i32), u32)> {
        targets
            .iter()
            .map(|&p| (p, self.at(p.0, p.1)))
            .filter(|e| e.1 != INF)
            .min_by_key(|e| (e.1, e.0 .1, e.0 .0))
    }

    /// The route from the start to `(x, y)`, start first; empty when unreached.
    pub fn route(&self, x: i32, y: i32) -> Vec<(i32, i32)> {
        if self.at(x, y) == INF {
            return vec![];
        }
        let mut out = vec![(x, y)];
        let mut i = (y * self.w + x) as usize;
        let mut guard = 0;
        while self.prev[i] != u32::MAX && guard <= self.dist.len() {
            i = self.prev[i] as usize;
            out.push((i as i32 % self.w, i as i32 / self.w));
            guard += 1;
        }
        out.reverse();
        out
    }
}

/// A route as a polyline: the corner points of a slab route (collinear points dropped).
pub fn polyline(route: &[(i32, i32)]) -> Vec<(i32, i32)> {
    if route.len() < 3 {
        return route.to_vec();
    }
    let mut out = vec![route[0]];
    for k in 1..route.len() - 1 {
        let (a, b, c) = (route[k - 1], route[k], route[k + 1]);
        if (b.0 - a.0, b.1 - a.1) != (c.0 - b.0, c.1 - b.1) {
            out.push(b);
        }
    }
    out.push(route[route.len() - 1]);
    out
}

/// Cheapest `max(a, b)` over all slabs: the time both sides need when they dig towards each
/// other and meet at the best slab, with that slab. `None` when they cannot meet.
pub fn meeting(a: &DistanceField, b: &DistanceField) -> Option<((i32, i32), u32)> {
    let mut best: Option<(usize, u32)> = None;
    for i in 0..a.dist.len() {
        let (da, db) = (a.dist[i], b.dist[i]);
        if da == INF || db == INF {
            continue;
        }
        let m = da.max(db);
        if best.is_none_or(|(_, v)| m < v) {
            best = Some((i, m));
        }
    }
    best.map(|(i, m)| ((i as i32 % a.w, i as i32 / a.w), m))
}
