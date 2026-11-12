//! Fairness report for maps with two or more keepers.
//!
//! Each metric is compared across keepers as a relative spread `(best - worst) / best` (0 =
//! equal, 1 = one keeper has nothing); the imbalance score is the weighted mean of the spreads.
//! The weights are a judgement call (documented on [`METRICS`]): gold and distance to the
//! nearest enemy matter most in a skirmish.
use crate::economy::Economy;
use crate::heroes::HeroPressure;
use crate::nav::INF;
use crate::reach::{Endpoint, KeeperFields};
use crate::sites::{owner_name, Sites, ThingKind};
use crate::terrain::Terrain;

/// One keeper's row.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerRow {
    pub player: u8,
    /// Claimable slabs reached first.
    pub area: usize,
    /// Gold: assigned gold slabs' gold + loose gold.
    pub gold: u64,
    pub gold_slabs: usize,
    pub gems: usize,
    /// Portals reached first, and the turns to the nearest portal.
    pub portals: usize,
    pub nearest_portal: u32,
    /// Dig turns to the map centre slab (or the nearest reachable slab around it).
    pub centre: u32,
    /// Turns until first contact with the nearest enemy (both digging towards each other).
    pub nearest_enemy: u32,
    pub creatures: usize,
    pub creature_levels: i64,
    pub creature_score: i64,
    /// Hero sources reaching this keeper within 10 % of their nearest keeper, their total score
    /// (groups) and the turns of the fastest one.
    pub hero_sources: usize,
    pub hero_score: i64,
    pub hero_turns: u32,
}

/// A metric's spread.
#[derive(Clone, Debug, PartialEq)]
pub struct Difference {
    pub metric: &'static str,
    pub spread: f32,
    pub weight: f32,
    /// Plain words: "gold: RED 12288, BLUE 6144".
    pub text: String,
}

/// The report.
#[derive(Clone, Debug, PartialEq)]
pub struct Fairness {
    pub rows: Vec<PlayerRow>,
    /// 0 = perfectly even.
    pub score: f32,
    /// Largest contributions first.
    pub differences: Vec<Difference>,
}

/// `(name, weight, more is better)`.
pub const METRICS: [(&str, f32, bool); 9] = [
    ("gold", 3.0, true),
    ("area", 1.5, true),
    ("gems", 1.5, true),
    ("portals", 1.0, true),
    ("distance to nearest portal", 1.5, false),
    ("distance to nearest enemy", 2.0, true),
    ("distance to the centre", 0.5, false),
    ("pre-placed creatures", 1.5, true),
    ("hero threat", 1.0, false),
];

fn value(r: &PlayerRow, m: &str) -> Option<f64> {
    let d = |v: u32| (v != INF).then_some(v as f64);
    Some(match m {
        "gold" => r.gold as f64,
        "area" => r.area as f64,
        "gems" => r.gems as f64,
        "portals" => r.portals as f64,
        "distance to nearest portal" => d(r.nearest_portal)?,
        "distance to nearest enemy" => d(r.nearest_enemy)?,
        "distance to the centre" => d(r.centre)?,
        "pre-placed creatures" => r.creature_score as f64,
        "hero threat" => r.hero_score as f64 + r.hero_sources as f64,
        _ => return None,
    })
}

/// Build the report. `contacts[(a, b)]` = first-contact turns.
pub fn fairness(
    t: &Terrain,
    sites: &Sites,
    eco: &Economy,
    fields: &[KeeperFields],
    contacts: &[(u8, u8, u32)],
    pressure: &[HeroPressure],
    groups: &[crate::heroes::HeroGroup],
) -> Option<Fairness> {
    if fields.len() < 2 {
        return None;
    }
    // the slabs around the continuous centre ((w-1)/2, (h-1)/2), a set every symmetry maps to
    // itself, grown until every keeper reaches one of them
    let mut centre_cells = vec![];
    for r in 0..t.w.max(t.h) {
        centre_cells.clear();
        for y in 0..t.h {
            for x in 0..t.w {
                if (2 * x - (t.w - 1)).abs() <= 2 * r + 1 && (2 * y - (t.h - 1)).abs() <= 2 * r + 1
                {
                    centre_cells.push((x, y));
                }
            }
        }
        if fields
            .iter()
            .all(|f| f.dig.nearest(&centre_cells).is_some())
        {
            break;
        }
    }
    let mut rows = vec![];
    for (f, pe) in fields.iter().zip(&eco.players) {
        let p = f.player;
        let mut row = PlayerRow {
            player: p,
            area: pe.claimable,
            gold: pe.gold_value + pe.loose_gold,
            gold_slabs: pe.gold_slabs,
            gems: pe.gems,
            portals: pe.own_portals,
            nearest_portal: pe.portals.first().map(|d| d.turns).unwrap_or(INF),
            centre: f.dig.nearest(&centre_cells).map(|e| e.1).unwrap_or(INF),
            nearest_enemy: contacts
                .iter()
                .filter(|c| c.0 == p || c.1 == p)
                .map(|c| c.2)
                .min()
                .unwrap_or(INF),
            creatures: 0,
            creature_levels: 0,
            creature_score: 0,
            hero_sources: 0,
            hero_score: 0,
            hero_turns: INF,
        };
        for s in &sites.things {
            if let (true, ThingKind::Creature { level, score, .. }) = (s.owner == p, &s.kind) {
                row.creatures += 1;
                row.creature_levels += level;
                row.creature_score += score;
            }
        }
        // a source threatens every keeper it reaches about as fast (10 %) as its nearest one (ties
        // on symmetric maps count for all of them)
        for hp in pressure {
            let Some(best) = hp.per_keeper.first().map(|e| e.1) else {
                continue;
            };
            let Some(&(_, mine)) = hp.per_keeper.iter().find(|e| e.0 == p) else {
                continue;
            };
            if (mine as u64) * 100 <= best as u64 * 110 {
                row.hero_sources += 1;
                row.hero_turns = row.hero_turns.min(mine);
                if let Endpoint::HeroGroup { index } = hp.source {
                    row.hero_score += groups.get(index).map(|g| g.score).unwrap_or(0);
                }
            }
        }
        rows.push(row);
    }
    let mut differences = vec![];
    for (m, wgt, more_better) in METRICS {
        let vals: Vec<(u8, Option<f64>)> = rows.iter().map(|r| (r.player, value(r, m))).collect();
        let known: Vec<(u8, f64)> = vals
            .iter()
            .filter_map(|(p, v)| v.map(|v| (*p, v)))
            .collect();
        let missing: Vec<u8> = vals
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(p, _)| *p)
            .collect();
        let fmt = |v: f64| {
            if m.starts_with("distance") {
                format!("{:.0}s", v / 20.0)
            } else {
                format!("{v:.0}")
            }
        };
        let (spread, text) = if !missing.is_empty() && !known.is_empty() {
            let names: Vec<&str> = missing.iter().map(|&p| owner_name(p)).collect();
            (
                1.0,
                format!("{m}: {} cannot reach it at all", names.join(", ")),
            )
        } else if known.len() < 2 {
            (0.0, String::new())
        } else {
            let hi = known
                .iter()
                .cloned()
                .fold((0u8, f64::MIN), |a, b| if b.1 > a.1 { b } else { a });
            let lo = known
                .iter()
                .cloned()
                .fold((0u8, f64::MAX), |a, b| if b.1 < a.1 { b } else { a });
            let s = if hi.1 <= 0.0 {
                0.0
            } else {
                (hi.1 - lo.1) / hi.1
            };
            let (best, worst) = if more_better { (hi, lo) } else { (lo, hi) };
            (
                s,
                format!(
                    "{m}: {} {} vs {} {}",
                    owner_name(best.0),
                    fmt(best.1),
                    owner_name(worst.0),
                    fmt(worst.1)
                ),
            )
        };
        differences.push(Difference {
            metric: m,
            spread: spread as f32,
            weight: wgt,
            text,
        });
    }
    let total_w: f32 = differences.iter().map(|d| d.weight).sum();
    let score = differences.iter().map(|d| d.spread * d.weight).sum::<f32>() / total_w.max(1e-6);
    differences.sort_by(|a, b| {
        (b.spread * b.weight)
            .total_cmp(&(a.spread * a.weight))
            .then(a.metric.cmp(b.metric))
    });
    Some(Fairness {
        rows,
        score,
        differences,
    })
}

impl Fairness {
    /// The `n` largest differences in plain words (only those with a spread).
    pub fn top(&self, n: usize) -> Vec<String> {
        self.differences
            .iter()
            .filter(|d| d.spread > 0.0)
            .take(n)
            .map(|d| format!("{} ({:.0}% apart)", d.text, d.spread * 100.0))
            .collect()
    }
}
