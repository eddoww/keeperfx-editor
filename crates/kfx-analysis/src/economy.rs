//! Gold, gems, portals, claimable area and pre-built rooms per keeper.
//!
//! Slabs are given to the keeper whose imps reach them first ([`Mover::Dig`] distance from the
//! hearts, "nearest heart"); a slab is *contested* when another keeper reaches it within 15 % (plus
//! two slabs of walking). Gold values follow the layered rules:
//! * a gold slab holds `GoldHeld` of its kind (1024 for `GOLD` in the shipped terrain.cfg) and
//!   is gone once dug (`instance_dig` src/creature_instances.c:700, `mine_out_block`);
//! * gems (`GEMS`, indestructible) never run out: each hit yields
//!   `damage * GoldHeld / block_health` gold (`calculate_gold_digged_out_of_slab_with_single_hit`
//!   src/thing_stats.c:1174: 1 * 174 / 20 = 8 gold per hit shipped), reported as a rate;
//! * loose gold objects use `gold_object_typical_value` (src/thing_objects.c:2106) unless the thing
//!   has its own `GoldValue`.
//!
//! Room capacity uses the room's `TotalCapacity` function at full efficiency
//! (`ROOM_EFFICIENCY_MAX`, src/room_data.c:443-500, src/room_treasure.c:40): an upper bound, real
//! efficiency depends on the walls around the room.
use crate::nav::{Mover, INF};
use crate::reach::KeeperFields;
use crate::rules::Rules;
use crate::sites::{Sites, ThingKind};
use crate::terrain::{Class, Terrain};
use kfx_map::gamedata::GameData;

/// No keeper.
pub const NOBODY: u8 = 255;

/// A pre-built room.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomSummary {
    pub room: u16,
    pub name: String,
    pub owner: u8,
    pub slabs: usize,
    pub centre: (i32, i32),
    /// `TotalCapacity` function name of terrain.cfg (`slabs_all_wth_effcncy`, ...).
    pub capacity_func: String,
    /// Capacity at full efficiency (items / creatures; for gold storage: hoard sizes).
    pub capacity: u32,
    /// Gold storable (treasuries: slabs x `GoldPerHoard`).
    pub gold_capacity: u32,
}

/// A portal as seen from one keeper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortalDist {
    /// Index into [`Sites::portals`].
    pub portal: usize,
    /// Dig distance in turns ([`INF`] when unreachable).
    pub turns: u32,
}

/// One keeper's resources.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerEconomy {
    pub player: u8,
    /// Gold slabs assigned to this keeper (nearest heart) and their gold.
    pub gold_slabs: usize,
    pub gold_value: u64,
    /// Gem slabs assigned, and the gold per hit one imp gets from one of them.
    pub gems: usize,
    pub gem_gold_per_hit: u32,
    /// Loose gold objects assigned (gold).
    pub loose_gold: u64,
    /// Gold slabs assigned but contested.
    pub contested_gold_slabs: usize,
    /// Gold slabs this keeper can reach at all (by digging).
    pub reachable_gold_slabs: usize,
    pub reachable_gems: usize,
    /// Every portal by dig distance, nearest first.
    pub portals: Vec<PortalDist>,
    /// Portals assigned to this keeper (nearest heart).
    pub own_portals: usize,
    /// Slabs assigned to this keeper that can become its floor (open or diggable, not
    /// liquid).
    pub claimable: usize,
    /// Pre-built rooms owned by this keeper.
    pub rooms: Vec<usize>,
}

/// The economy of a map.
#[derive(Clone, Debug, PartialEq)]
pub struct Economy {
    /// Keeper reaching each slab first ([`NOBODY`] when none).
    pub owner_of: Vec<u8>,
    pub contested: Vec<bool>,
    pub players: Vec<PlayerEconomy>,
    pub total_gold_slabs: usize,
    pub total_gold_value: u64,
    pub total_gems: usize,
    pub unreachable_gold_slabs: usize,
    pub rooms: Vec<RoomSummary>,
}

fn capacity(func: &str, slabs: u32, wealth_sizes: u32) -> u32 {
    // full efficiency (ROOM_EFFICIENCY_MAX = 256)
    match func.to_ascii_lowercase().as_str() {
        "slabs_all_only" | "slabs_all_wth_effcncy" | "slabs_no_min_wth_effcncy" => slabs.max(1),
        "slabs_div2_wth_effcncy" => (slabs >> 1).max(1),
        "slabs_div2_nomin_effcncy" => slabs >> 1,
        "slabs_mul2_wth_effcncy" => (slabs << 1).max(1),
        "slabs_pow2_wth_effcncy" => slabs.max(1),
        "gold_slabs_wth_effcncy" | "gold_slabs_full" => slabs * wealth_sizes,
        "gold_slabs_div2" => slabs * wealth_sizes / 2,
        _ => 0,
    }
}

/// Pre-built rooms (connected slabs of one room kind and owner), heart and portal included.
pub fn rooms(t: &Terrain, gd: &GameData, r: &Rules) -> Vec<RoomSummary> {
    // `get_wealth_size_types_count`: the treasure hoard object models (5 shipped)
    let wealth = gd
        .object_names()
        .iter()
        .filter(|(m, _)| kfx_map::things::object_genre_is(gd, *m as i64, "TREASURE_HOARD"))
        .count()
        .max(1) as u32;
    crate::sites::room_groups(t, |_| true)
        .into_iter()
        .map(|(room, owner, cells)| {
            let cfg = gd.rooms.get(room as usize);
            let name = cfg.map(|c| c.name.clone()).unwrap_or_default();
            let func = cfg
                .and_then(|c| c.raw.get("TotalCapacity"))
                .and_then(|v| v.split_whitespace().next())
                .unwrap_or("")
                .to_string();
            let used = cfg
                .and_then(|c| c.raw.get("UsedCapacity"))
                .unwrap_or("")
                .to_ascii_lowercase();
            let n = cells.len() as u32;
            let gold_capacity = if used.contains("gold_hoardes_in_room") {
                n * r.gold_per_hoard
            } else {
                0
            };
            RoomSummary {
                room,
                name,
                owner,
                slabs: cells.len(),
                centre: crate::sites::group_centre(&cells),
                capacity: capacity(&func, n, wealth),
                capacity_func: func,
                gold_capacity,
            }
        })
        .collect()
}

/// Compute the economy from the keepers' fields.
pub fn economy(
    t: &Terrain,
    sites: &Sites,
    gd: &GameData,
    r: &Rules,
    fields: &[KeeperFields],
) -> Economy {
    let n = t.len();
    let mut owner_of = vec![NOBODY; n];
    let mut contested = vec![false; n];
    for i in 0..n {
        let mut best = (INF, NOBODY);
        let mut second = INF;
        for f in fields {
            let d = f.dig.dist[i];
            if d < best.0 {
                second = best.0;
                best = (d, f.player);
            } else if d < second {
                second = d;
            }
        }
        if best.0 != INF {
            // an exact tie belongs to nobody (it is contested), so symmetric maps stay even
            owner_of[i] = if second == best.0 { NOBODY } else { best.1 };
            contested[i] = second != INF
                && (second as u64) * 100 <= (best.0 as u64) * 115 + 2 * r.walk_turns as u64 * 100;
        }
    }
    let rooms = rooms(t, gd, r);
    let mut players: Vec<PlayerEconomy> = fields
        .iter()
        .map(|f| PlayerEconomy {
            player: f.player,
            gold_slabs: 0,
            gold_value: 0,
            gems: 0,
            gem_gold_per_hit: 0,
            loose_gold: 0,
            contested_gold_slabs: 0,
            reachable_gold_slabs: 0,
            reachable_gems: 0,
            portals: vec![],
            own_portals: 0,
            claimable: 0,
            rooms: rooms
                .iter()
                .enumerate()
                .filter(|(_, rm)| rm.owner == f.player)
                .map(|(k, _)| k)
                .collect(),
        })
        .collect();
    let mut eco = Economy {
        owner_of,
        contested,
        players: vec![],
        total_gold_slabs: 0,
        total_gold_value: 0,
        total_gems: 0,
        unreachable_gold_slabs: 0,
        rooms,
    };
    // gems are reachable when a neighbour is: they are dug from the side, never entered
    let gem_dist = |f: &KeeperFields, x: i32, y: i32| -> u32 {
        [(0, -1), (1, 0), (0, 1), (-1, 0)]
            .iter()
            .map(|(dx, dy)| f.dig.at(x + dx, y + dy))
            .min()
            .unwrap_or(INF)
    };
    for i in 0..n {
        let c = &t.cells[i];
        let (x, y) = t.xy(i);
        match c.class {
            Class::Gold => {
                eco.total_gold_slabs += 1;
                eco.total_gold_value += c.gold.max(0) as u64;
                if eco.owner_of[i] == NOBODY {
                    eco.unreachable_gold_slabs += 1;
                }
                for (p, f) in players.iter_mut().zip(fields) {
                    if f.dig.dist[i] != INF {
                        p.reachable_gold_slabs += 1;
                    }
                    if eco.owner_of[i] == p.player {
                        p.gold_slabs += 1;
                        p.gold_value += c.gold.max(0) as u64;
                        if eco.contested[i] {
                            p.contested_gold_slabs += 1;
                        }
                    }
                }
            }
            Class::Gems => {
                eco.total_gems += 1;
                // owner: nearest by a neighbour
                let mut best = (INF, NOBODY);
                let mut tie = false;
                for f in fields {
                    let d = gem_dist(f, x, y);
                    if d < best.0 {
                        best = (d, f.player);
                        tie = false;
                    } else if d == best.0 && d != INF {
                        tie = true;
                    }
                }
                if tie {
                    best.1 = NOBODY;
                }
                for (p, f) in players.iter_mut().zip(fields) {
                    if gem_dist(f, x, y) != INF {
                        p.reachable_gems += 1;
                    }
                    if best.1 == p.player {
                        p.gems += 1;
                        p.gem_gold_per_hit = r.gold_per_hit(c.gold, c.health);
                    }
                }
            }
            _ => {}
        }
        let o = eco.owner_of[i];
        if o != NOBODY
            && !matches!(
                c.class,
                Class::Water | Class::Lava | Class::Rock | Class::Gems
            )
        {
            if let Some(p) = players.iter_mut().find(|p| p.player == o) {
                p.claimable += 1;
            }
        }
    }
    for s in &sites.things {
        if let ThingKind::Gold { value } = s.kind {
            let o = eco.owner_of[t.idx(s.slab.0, s.slab.1)];
            if let Some(p) = players.iter_mut().find(|p| p.player == o) {
                p.loose_gold += value as u64;
            }
        }
    }
    for (p, f) in players.iter_mut().zip(fields) {
        let mut v: Vec<PortalDist> = sites
            .portals
            .iter()
            .enumerate()
            .map(|(k, po)| PortalDist {
                portal: k,
                turns: f.dig.nearest(&po.cells).map(|e| e.1).unwrap_or(INF),
            })
            .collect();
        v.sort_by_key(|d| (d.turns, d.portal));
        p.portals = v;
        debug_assert!(matches!(f.dig.mover, Mover::Dig { .. }));
    }
    for po in &sites.portals {
        let i = t.idx(po.centre.0, po.centre.1);
        let o = eco.owner_of[i];
        if let Some(p) = players.iter_mut().find(|p| p.player == o) {
            p.own_portals += 1;
        }
    }
    eco.players = players;
    eco
}
