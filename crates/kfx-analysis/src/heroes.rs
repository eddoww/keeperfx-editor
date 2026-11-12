//! Hero pressure: where hero gates and pre-placed hero groups go and what they pass.
//!
//! Heroes walk to the keepers and break their doors; a party with a tunneller digs through
//! plain earth (not gold, walls or rock, which it walks around: the tunneller's wallhug uses
//! `SlbAtFlg_Filled|SlbAtFlg_Valuable` as the blocking set, src/ariadne_wallhug.c:1673). Gates are
//! scripted, so for a gate both a walking party and a tunnelling one are computed.
use crate::nav::{DistanceField, Mover, INF};
use crate::reach::{Endpoint, Route};
use crate::rules::Rules;
use crate::sites::{Sites, ThingKind, HEROES};
use crate::terrain::Terrain;
use kfx_map::gamedata::GameData;

/// Pre-placed heroes standing together (chains of hero creatures at most 3 slabs apart).
#[derive(Clone, Debug, PartialEq)]
pub struct HeroGroup {
    /// Indices into [`Sites::things`].
    pub members: Vec<usize>,
    pub cells: Vec<(i32, i32)>,
    pub score: i64,
    pub levels: i64,
    /// A member is a `DIGGING_CREATURE` (tunneller).
    pub tunneller: bool,
}

/// Where one hero source goes.
#[derive(Clone, Debug, PartialEq)]
pub struct HeroPressure {
    pub source: Endpoint,
    /// Walking only (no tunneller): route to the nearest keeper heart.
    pub walk: Option<Route>,
    /// With a tunneller (gates) / as the group is (groups with a tunneller).
    pub tunnel: Option<Route>,
    /// Keeper reached first (by the walking route, else the tunnelling one).
    pub target: Option<u8>,
    /// Turns to that keeper's heart.
    pub turns: u32,
    /// Every keeper this source reaches, with the turns (walking when possible, else
    /// tunnelling), nearest first.
    pub per_keeper: Vec<(u8, u32)>,
}

fn is_digger(gd: &GameData, model: i64) -> bool {
    usize::try_from(model)
        .ok()
        .and_then(|m| gd.creature_config_of_model(m))
        .is_some_and(|c| {
            c.attributes
                .properties
                .iter()
                .any(|p| p == "DIGGING_CREATURE")
        })
}

/// Group the hero creatures.
pub fn hero_groups(gd: &GameData, sites: &Sites) -> Vec<HeroGroup> {
    let heroes: Vec<usize> = sites
        .things
        .iter()
        .enumerate()
        .filter(|(_, s)| s.owner == HEROES && matches!(s.kind, ThingKind::Creature { .. }))
        .map(|(k, _)| k)
        .collect();
    let mut group = vec![usize::MAX; heroes.len()];
    let mut out = vec![];
    for a in 0..heroes.len() {
        if group[a] != usize::MAX {
            continue;
        }
        let g = out.len();
        group[a] = g;
        let mut stack = vec![a];
        let mut members = vec![];
        while let Some(i) = stack.pop() {
            members.push(heroes[i]);
            let p = sites.things[heroes[i]].slab;
            for b in 0..heroes.len() {
                let q = sites.things[heroes[b]].slab;
                if group[b] == usize::MAX && (p.0 - q.0).abs() <= 3 && (p.1 - q.1).abs() <= 3 {
                    group[b] = g;
                    stack.push(b);
                }
            }
        }
        members.sort();
        let mut cells: Vec<(i32, i32)> = members.iter().map(|&k| sites.things[k].slab).collect();
        cells.sort();
        cells.dedup();
        let (mut score, mut levels, mut tunneller) = (0, 0, false);
        for &k in &members {
            if let ThingKind::Creature {
                score: s,
                level,
                model,
                ..
            } = &sites.things[k].kind
            {
                score += s;
                levels += level;
                tunneller |= is_digger(gd, *model);
            }
        }
        out.push(HeroGroup {
            members,
            cells,
            score,
            levels,
            tunneller,
        });
    }
    out
}

fn heart_targets(sites: &Sites) -> Vec<(u8, Vec<(i32, i32)>)> {
    sites
        .keepers()
        .into_iter()
        .map(|p| (p, sites.heart_cells(p)))
        .collect()
}

fn keeper_turns(
    sites: &Sites,
    walk: &DistanceField,
    tunnel: Option<&DistanceField>,
) -> Vec<(u8, u32)> {
    let mut v: Vec<(u8, u32)> = heart_targets(sites)
        .into_iter()
        .filter_map(|(p, cells)| {
            let d = walk
                .nearest(&cells)
                .map(|e| e.1)
                .or_else(|| tunnel.and_then(|f| f.nearest(&cells).map(|e| e.1)))?;
            Some((p, d))
        })
        .collect();
    v.sort_by_key(|e| (e.1, e.0));
    v
}

fn best_route(
    t: &Terrain,
    sites: &Sites,
    field: &DistanceField,
    from: Endpoint,
) -> Option<(u8, Route)> {
    let mut best: Option<(u8, Route)> = None;
    for (p, cells) in heart_targets(sites) {
        if let Some(r) = Route::build(t, sites, field, from, Endpoint::Heart { owner: p }, &cells) {
            if best.as_ref().is_none_or(|b| r.turns < b.1.turns) {
                best = Some((p, r));
            }
        }
    }
    best
}

fn pressure(
    t: &Terrain,
    r: &Rules,
    sites: &Sites,
    from: Endpoint,
    starts: &[(i32, i32)],
    tunnel_too: bool,
) -> HeroPressure {
    let walk_f = DistanceField::compute(t, r, Mover::Hero { tunnel: false }, starts);
    let walk = best_route(t, sites, &walk_f, from);
    let tunnel_f =
        tunnel_too.then(|| DistanceField::compute(t, r, Mover::Hero { tunnel: true }, starts));
    let tunnel = tunnel_f
        .as_ref()
        .and_then(|f| best_route(t, sites, f, from));
    let per_keeper = keeper_turns(sites, &walk_f, tunnel_f.as_ref());
    let pick = walk.as_ref().or(tunnel.as_ref());
    HeroPressure {
        source: from,
        target: pick.map(|p| p.0),
        turns: pick.map(|p| p.1.turns).unwrap_or(INF),
        walk: walk.map(|w| w.1),
        tunnel: tunnel.map(|w| w.1),
        per_keeper,
    }
}

/// Pressure of every gate and every hero group.
pub fn hero_pressure(
    t: &Terrain,
    r: &Rules,
    sites: &Sites,
    groups: &[HeroGroup],
) -> Vec<HeroPressure> {
    let mut out = vec![];
    for g in &sites.gates {
        out.push(pressure(
            t,
            r,
            sites,
            Endpoint::Gate { number: g.number },
            &[g.slab],
            true,
        ));
    }
    for (k, g) in groups.iter().enumerate() {
        let mut p = pressure(
            t,
            r,
            sites,
            Endpoint::HeroGroup { index: k },
            &g.cells,
            g.tunneller,
        );
        if !g.tunneller {
            p.tunnel = None;
        }
        out.push(p);
    }
    out
}
