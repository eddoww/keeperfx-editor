//! Reachability tiers, per-thing reachability, isolated pockets and routes.
use crate::nav::{polyline, DistanceField, Mover, INF};
use crate::rules::Rules;
use crate::sites::{Sites, ThingKind};
use crate::terrain::{Class, Terrain};

/// How a slab can be reached from a keeper's hearts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Walking now.
    Walk = 1,
    /// After digging through what the keeper's imps may dig (and breaking enemy doors).
    Dig = 2,
    /// Only by also crossing lava (flying / lava-proof creatures, or a bridge).
    Cross = 3,
    /// Not at all.
    Never = 0,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Walk => "walk now",
            Tier::Dig => "after digging",
            Tier::Cross => "fly / cross lava",
            Tier::Never => "unreachable",
        }
    }
    pub fn from_dists(walk: u32, dig: u32, cross: u32) -> Tier {
        if walk != INF {
            Tier::Walk
        } else if dig != INF {
            Tier::Dig
        } else if cross != INF {
            Tier::Cross
        } else {
            Tier::Never
        }
    }
}

/// The three fields of one keeper.
#[derive(Clone, Debug, PartialEq)]
pub struct KeeperFields {
    pub player: u8,
    pub walk: DistanceField,
    pub dig: DistanceField,
    pub cross: DistanceField,
}

impl KeeperFields {
    pub fn compute(t: &Terrain, r: &Rules, player: u8, starts: &[(i32, i32)]) -> KeeperFields {
        KeeperFields {
            player,
            walk: DistanceField::compute(t, r, Mover::Walk { player }, starts),
            dig: DistanceField::compute(t, r, Mover::Dig { player }, starts),
            cross: DistanceField::compute(t, r, Mover::DigCross { player }, starts),
        }
    }
    pub fn tier(&self, x: i32, y: i32) -> Tier {
        Tier::from_dists(self.walk.at(x, y), self.dig.at(x, y), self.cross.at(x, y))
    }
    /// Per-slab tiers.
    pub fn tiers(&self) -> Vec<Tier> {
        (0..self.dig.dist.len())
            .map(|i| Tier::from_dists(self.walk.dist[i], self.dig.dist[i], self.cross.dist[i]))
            .collect()
    }
}

/// Reachability of one interesting thing from every keeper.
#[derive(Clone, Debug, PartialEq)]
pub struct ThingReach {
    /// Index into [`Sites::things`].
    pub site: usize,
    /// `(player, tier, dig distance in turns or INF)`.
    pub by_player: Vec<(u8, Tier, u32)>,
}

pub fn thing_reach(sites: &Sites, fields: &[KeeperFields]) -> Vec<ThingReach> {
    sites
        .things
        .iter()
        .enumerate()
        .map(|(i, s)| ThingReach {
            site: i,
            by_player: fields
                .iter()
                .map(|f| {
                    (
                        f.player,
                        f.tier(s.slab.0, s.slab.1),
                        f.dig.at(s.slab.0, s.slab.1),
                    )
                })
                .collect(),
        })
        .collect()
}

/// Open slabs no keeper and no hero gate can get to, even by digging or crossing lava.
#[derive(Clone, Debug, PartialEq)]
pub struct Pocket {
    pub cells: Vec<(i32, i32)>,
    /// Indices into [`Sites::things`] inside.
    pub things: Vec<usize>,
}

/// Connected open areas (4-neighbour) reached by none of `reached` (one flag per slab).
pub fn pockets(t: &Terrain, sites: &Sites, reached: &[bool]) -> Vec<Pocket> {
    let mut seen = vec![false; t.len()];
    let mut out = vec![];
    for i in 0..t.len() {
        if seen[i] || reached[i] || !t.cells[i].class.is_open() {
            continue;
        }
        let mut cells = vec![];
        let mut stack = vec![i];
        seen[i] = true;
        let mut touches = false;
        while let Some(j) = stack.pop() {
            let (x, y) = t.xy(j);
            cells.push((x, y));
            for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let (nx, ny) = (x + dx, y + dy);
                if !t.inside(nx, ny) {
                    continue;
                }
                let k = t.idx(nx, ny);
                if reached[k] && t.cells[k].class.is_open() {
                    touches = true;
                }
                if !seen[k] && !reached[k] && t.cells[k].class.is_open() {
                    seen[k] = true;
                    stack.push(k);
                }
            }
        }
        if touches {
            continue;
        }
        cells.sort_by_key(|&(x, y)| (y, x));
        let things = sites
            .things
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                cells
                    .binary_search_by_key(&(s.slab.1, s.slab.0), |&(x, y)| (y, x))
                    .is_ok()
            })
            .map(|(k, _)| k)
            .collect();
        out.push(Pocket { cells, things });
    }
    out
}

/// What a route goes through.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Passes {
    /// Slabs per class, in [`Class`] order of first appearance.
    pub classes: Vec<(Class, usize)>,
    /// Doors on the route: `(slab, owner, locked)`.
    pub doors: Vec<((i32, i32), u8, bool)>,
    /// Rooms entered: `(room id, owner, slabs)`.
    pub rooms: Vec<(u16, u8, usize)>,
    /// Interesting things within one slab of the route (indices into [`Sites::things`]).
    pub things: Vec<usize>,
}

impl Passes {
    pub fn of(t: &Terrain, sites: &Sites, route: &[(i32, i32)]) -> Passes {
        let mut p = Passes::default();
        for &(x, y) in route {
            let c = t.at(x, y);
            match p.classes.iter_mut().find(|e| e.0 == c.class) {
                Some(e) => e.1 += 1,
                None => p.classes.push((c.class, 1)),
            }
            if c.class == Class::Door {
                p.doors.push(((x, y), c.owner, c.locked));
            }
            if c.room != 0 {
                match p.rooms.iter_mut().find(|e| e.0 == c.room && e.1 == c.owner) {
                    Some(e) => e.2 += 1,
                    None => p.rooms.push((c.room, c.owner, 1)),
                }
            }
        }
        for (k, s) in sites.things.iter().enumerate() {
            if route
                .iter()
                .any(|&(x, y)| (x - s.slab.0).abs() <= 1 && (y - s.slab.1).abs() <= 1)
            {
                p.things.push(k);
            }
        }
        p
    }

    /// Count of slabs of a class.
    pub fn count(&self, c: Class) -> usize {
        self.classes
            .iter()
            .find(|e| e.0 == c)
            .map(|e| e.1)
            .unwrap_or(0)
    }

    /// Hero creatures near the route and their total score.
    pub fn heroes(&self, sites: &Sites) -> (usize, i64) {
        let mut n = 0;
        let mut s = 0;
        for &k in &self.things {
            if let (crate::sites::HEROES, ThingKind::Creature { score, .. }) =
                (sites.things[k].owner, &sites.things[k].kind)
            {
                n += 1;
                s += score;
            }
        }
        (n, s)
    }
}

/// An end of a route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Heart { owner: u8 },
    Gate { number: i64 },
    HeroGroup { index: usize },
    Slab(i32, i32),
}

/// A shortest route.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub from: Endpoint,
    pub to: Endpoint,
    /// Turns along the route for its mover.
    pub turns: u32,
    /// The mover; [`Mover::Siege`] when no route exists without breaking enemy walls.
    pub mover: Mover,
    /// Every slab, start first.
    pub slabs: Vec<(i32, i32)>,
    /// Corner points.
    pub polyline: Vec<(i32, i32)>,
    pub passes: Passes,
}

impl Route {
    /// The route of `field` to the nearest of `targets`, `None` when none is reached.
    pub fn build(
        t: &Terrain,
        sites: &Sites,
        field: &DistanceField,
        from: Endpoint,
        to: Endpoint,
        targets: &[(i32, i32)],
    ) -> Option<Route> {
        let ((x, y), turns) = field.nearest(targets)?;
        let slabs = field.route(x, y);
        Some(Route {
            from,
            to,
            turns,
            mover: field.mover,
            polyline: polyline(&slabs),
            passes: Passes::of(t, sites, &slabs),
            slabs,
        })
    }
}
