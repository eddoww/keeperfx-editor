//! Hand-built maps with known answers (skipped without the test install).
mod common;

use kfx_analysis::analysis::Analysis;
use kfx_analysis::nav::INF;
use kfx_analysis::reach::Tier;
use kfx_analysis::rules::Rules;
use kfx_analysis::terrain::Class;
use kfx_formats::level_bin::MapDims;
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_map::things::{self, class};
use kfx_map::transform::SymmetryMode;

fn k(gd: &GameData, n: &str) -> u16 {
    gd.slab_id(n).unwrap_or_else(|| panic!("{n}")) as u16
}

/// A `w x h` rock map; `rows` paints it: one char per slab.
/// `#` rock, `.` earth, `g` gold, `*` gems, `_` path, `H` heart (owner by digit before), `P`
/// portal, `~` water, `=` lava, `W` wall, `D` wooden door, `c` claimed floor; a digit sets the
/// owner of the following slabs of that row (5 = neutral).
fn paint(gd: &GameData, rows: &[&str]) -> MapDoc {
    let h = rows.len() as u16;
    let w = rows[0].len() as u16;
    let mut doc = MapDoc::new(
        std::path::Path::new("/nonexistent"),
        1,
        MapDims::new(w, h),
        k(gd, "HARD"),
        gd,
    );
    for (y, row) in rows.iter().enumerate() {
        let mut owner = 5u8;
        let mut x = 0usize;
        for ch in row.chars() {
            if let Some(d) = ch.to_digit(10) {
                owner = d as u8;
                // a digit takes a slab too: it is claimed floor of that owner
                let i = doc.dims.slab_index(x, y);
                doc.kinds[i] = k(gd, "PRETTY_PATH");
                doc.owners[i] = owner;
                x += 1;
                continue;
            }
            let kind = match ch {
                '#' => "HARD",
                '.' => "DIRT",
                'g' => "GOLD",
                '*' => "GEMS",
                '_' => "PATH",
                'H' => "HEART_PEDESTAL",
                'P' => "ENTRANCE_ZONE",
                '~' => "WATER",
                '=' => "LAVA",
                'W' => "TORCH_WALL",
                'D' => "DOOR_WOODEN",
                'c' => "PRETTY_PATH",
                _ => panic!("{ch}"),
            };
            let i = doc.dims.slab_index(x, y);
            doc.kinds[i] = k(gd, kind);
            doc.owners[i] = if matches!(ch, '#' | '.' | 'g' | '*' | '_' | 'P' | '~' | '=') {
                5
            } else {
                owner
            };
            x += 1;
        }
        assert_eq!(x, w as usize, "row {y} width");
    }
    doc
}

fn gd() -> Option<GameData> {
    common::base_gd()
}

#[test]
fn portal_needs_digging_and_costs_match_the_rules() {
    let Some(gd) = gd() else { return };
    let doc = paint(
        &gd,
        &[
            "####################",
            "#0ccccc.......PPP..#",
            "#0cHHHc.......PPP..#",
            "#0cHHHc.......PPP..#",
            "#0cHHHc.......PPP..#",
            "#0ccccc............#",
            "####################",
        ],
    );
    let a = Analysis::of_doc(&doc, &gd);
    let r = Rules::from_game(&gd);
    assert_eq!(a.sites.keepers(), vec![0]);
    let f = a.keeper(0).unwrap();
    // from the heart's east column (5, y) through claimed (6) and 7 earth slabs to the portal (14)
    let expect =
        r.walk_turns + 7 * (r.walk_turns + r.dig_turns(r.block_health[0], false)) + r.walk_turns;
    assert_eq!(f.dig.at(14, 3), expect, "dig distance to the portal");
    assert_eq!(f.tier(14, 3), Tier::Dig);
    assert_eq!(f.tier(6, 3), Tier::Walk);
    assert_eq!(f.walk.at(14, 3), INF);
    assert_eq!(
        a.economy.players[0].portals[0].turns,
        f.dig.nearest(&a.sites.portals[0].cells).unwrap().1
    );
    // shipped rules: 8 turns a slab, dirt 2 hits of 8 turns
    if r == Rules::defaults() {
        assert_eq!(expect, 8 + 7 * 24 + 8);
    }
    assert!(a.fairness.is_none());
}

#[test]
fn walls_gold_gems_lava_and_doors() {
    let Some(gd) = gd() else { return };
    let doc = paint(
        &gd,
        &[
            "##########################",
            "#0cccc..gg..1WWWW...ccccc#",
            "#0cHHc..gg..1W..W...cHHc1#",
            "#0cHHc..*...1W..W...cHHc1#",
            "#0cccc......1WWWW...ccccc#",
            "#............============#",
            "#............=~~~~~~~~~=.#",
            "#............============#",
            "##########################",
        ],
    );
    // the second heart is player 1's: re-own its claimed floor and heart slabs
    let mut doc = doc;
    for y in 1..5 {
        for x in 20..25 {
            let i = doc.dims.slab_index(x, y);
            doc.owners[i] = 1;
        }
    }
    let a = Analysis::of_doc(&doc, &gd);
    assert_eq!(a.sites.keepers(), vec![0, 1]);
    let (f0, f1) = (a.keeper(0).unwrap(), a.keeper(1).unwrap());
    // player 1's walls: player 0 cannot dig them, player 1 can
    assert_eq!(a.terrain.at(13, 1).class, Class::Wall);
    assert_eq!(
        f0.dig.at(14, 2),
        INF,
        "the walled room of player 1 is closed to player 0"
    );
    assert_ne!(f1.dig.at(14, 2), INF);
    // the lava strip: beyond it only by crossing
    assert_eq!(f0.tier(14, 6), Tier::Cross);
    // gold: 4 slabs of GoldHeld each, nearest to player 0
    let g0 = a.economy.players.iter().find(|p| p.player == 0).unwrap();
    assert_eq!(g0.gold_slabs, 4);
    assert_eq!(
        g0.gold_value,
        4 * gd.slab(k(&gd, "GOLD") as usize).gold_held as u64
    );
    assert_eq!(g0.gems, 1);
    let r = Rules::from_game(&gd);
    let gs = gd.slab(k(&gd, "GEMS") as usize);
    assert_eq!(
        g0.gem_gold_per_hit,
        r.gold_per_hit(gs.gold_held, r.health(gs.block_health_index))
    );
    if r == Rules::defaults() {
        assert_eq!(g0.gem_gold_per_hit, 8, "174 * 1 / 20");
    }
    // player 0 cannot dig to player 1 (only through 1's walls or the lava), but player 1 can
    // dig out through its own walls, so they meet
    assert_eq!(a.routes.len(), 1);
    assert!(
        matches!(
            a.routes[0].mover,
            kfx_analysis::nav::Mover::Siege { player: 0 }
        ),
        "only by breaking walls: {:?}",
        a.routes[0].mover
    );
    assert!(a.routes[0].passes.count(Class::Wall) >= 1);
    assert_eq!(a.contacts.len(), 1);
    assert!(a.contacts[0].slab.0 <= 13, "{:?}", a.contacts[0]);
    let fr = a.fairness.as_ref().unwrap();
    assert_eq!(fr.rows.len(), 2);
    assert!(fr.score > 0.0, "asymmetric map");
    println!("fairness {:.3}: {:?}", fr.score, fr.top(4));
}

#[test]
fn own_locked_door_blocks_own_creatures_enemy_door_is_broken() {
    let Some(gd) = gd() else { return };
    let mut doc = paint(
        &gd,
        &[
            "##############",
            "#0cccc0W###__#",
            "#0cHHc0D_____#",
            "#0cHHc0W###__#",
            "#0cccc0W######",
            "##############",
        ],
    );
    // a door thing on the door slab, locked, owner 0
    let mut t = things::new_thing(
        class::DOOR,
        1,
        0,
        things::centred(3 * 7 + 1),
        things::centred(3 * 2 + 1),
        kfx_formats::level_fx::StlCoord::new(1, 128),
    );
    t.door_locked = Some(1);
    doc.things.items.push(t);
    let a = Analysis::of_doc(&doc, &gd);
    let f = a.keeper(0).unwrap();
    assert!(a.terrain.at(7, 2).locked);
    assert_eq!(f.walk.at(9, 2), INF, "own locked door blocks walking");
    // heroes on the far side break it
    let hero = kfx_analysis::nav::DistanceField::compute(
        &a.terrain,
        &a.rules,
        kfx_analysis::nav::Mover::Hero { tunnel: false },
        &[(11, 2)],
    );
    assert_ne!(hero.at(4, 2), INF, "heroes break the keeper's door");
    let walk = hero.at(4, 2);
    assert!(walk > 8 * a.rules.walk_turns, "the door costs time: {walk}");
}

#[test]
fn mirror_symmetry_detected_and_breaks_listed() {
    let Some(gd) = gd() else { return };
    let rows = [
        "################",
        "#0ccc.gg..gg.1c#",
        "#0HHc......1cHc#",
        "#....~~..~~....#",
        "#..*........*..#",
        "################",
    ];
    // make it an exact east-west mirror: player 1 side is the mirror of player 0's
    let mut doc = paint(&gd, &rows);
    let w = doc.dims.w as usize;
    for y in 0..doc.dims.h as usize {
        for x in 0..w / 2 {
            let (i, j) = (doc.dims.slab_index(x, y), doc.dims.slab_index(w - 1 - x, y));
            doc.kinds[j] = doc.kinds[i];
            doc.owners[j] = if doc.owners[i] == 0 { 1 } else { doc.owners[i] };
        }
    }
    let a = Analysis::of_doc(&doc, &gd);
    let ew = a
        .symmetry
        .iter()
        .find(|c| c.mode == SymmetryMode::EastWest)
        .unwrap();
    assert!(ew.terrain_exact(), "{:?}", ew.slab_breaks);
    assert!(!a
        .symmetry
        .iter()
        .find(|c| c.mode == SymmetryMode::NorthSouth)
        .unwrap()
        .terrain_exact());
    let fr = a.fairness.as_ref().unwrap();
    assert!(fr.score < 1e-6, "mirror map is fair: {:?}", fr.differences);
    // break one slab
    let i = doc.dims.slab_index(6, 4);
    doc.kinds[i] = k(&gd, "GOLD");
    let b = Analysis::of_doc(&doc, &gd);
    let ew = b
        .symmetry
        .iter()
        .find(|c| c.mode == SymmetryMode::EastWest)
        .unwrap();
    assert_eq!(ew.slab_breaks, vec![(6, 4), (9, 4)]);
}

#[test]
fn incremental_update_matches_full() {
    let Some(gd) = gd() else { return };
    let mut doc = paint(
        &gd,
        &[
            "################",
            "#0ccc....#...1c#",
            "#0HHc....#..1cH#",
            "#........#.....#",
            "################",
        ],
    );
    let mut a = Analysis::of_doc(&doc, &gd);
    assert!(a.contacts.is_empty(), "the rock wall separates them");
    for y in 1..4 {
        let i = doc.dims.slab_index(9, y);
        doc.kinds[i] = k(&gd, "DIRT");
    }
    a.update(
        &doc,
        &gd,
        kfx_map::autotile::SlabRect {
            x0: 9,
            y0: 1,
            x1: 9,
            y1: 3,
        },
    );
    let full = Analysis::of_doc(&doc, &gd);
    assert_eq!(a.terrain, full.terrain);
    assert_eq!(a.contacts, full.contacts);
    assert_eq!(a.routes, full.routes);
    assert_eq!(a.contacts.len(), 1);
}
