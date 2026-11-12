// Shared by several test binaries; each uses a subset.
#![allow(dead_code)]
use kfx_formats::level_bin::MapDims;
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::MapDoc;
use std::sync::OnceLock;

/// The game data of the test install, `None` when there is none (the test is skipped).
pub fn gd() -> Option<&'static GameData> {
    static GD: OnceLock<Option<GameData>> = OnceLock::new();
    GD.get_or_init(|| {
        let install = kfx_formats::testutil::install_dir()?;
        GameData::load(&ConfigLayers::for_level(&install, None, None, None, None)).ok()
    })
    .as_ref()
}

/// A blank earth map: rock border, dirt inside.
pub fn blank(gd: &GameData, w: u16, h: u16) -> MapDoc {
    let mut doc = MapDoc::new(
        std::path::Path::new("/nonexistent"),
        1,
        MapDims::new(w, h),
        0,
        gd,
    );
    let dirt = gd.slab_id("DIRT").unwrap() as u16;
    for y in 1..h as usize - 1 {
        for x in 1..w as usize - 1 {
            doc.place_slab(gd, x, y, dirt, 5).unwrap();
        }
    }
    doc
}

use kfx_map::autotile::SlabRect;
use kfx_map::things::{self, class};

pub fn k(gd: &GameData, n: &str) -> u16 {
    gd.slab_id(n).unwrap() as u16
}

/// A small walled room with a heart, a door, a creature, an action point, a light and a hero gate.
pub fn source(gd: &GameData) -> MapDoc {
    let mut doc = blank(gd, 40, 30);
    for y in 5..=11usize {
        for x in 5..=13usize {
            let edge = x == 5 || x == 13 || y == 5 || y == 11;
            let kind = if edge {
                k(gd, "PAIR_WALL")
            } else {
                k(gd, "PRETTY_PATH")
            };
            doc.place_slab(gd, x, y, kind, if edge { 1 } else { 1 })
                .unwrap();
        }
    }
    for y in 7..=9usize {
        for x in 6..=8usize {
            doc.place_slab(gd, x, y, k(gd, "HEART_PEDESTAL"), 1)
                .unwrap();
        }
    }
    for x in 10..=12usize {
        doc.place_slab(gd, x, 10, k(gd, "TREASURY_AREA"), 1)
            .unwrap();
    }
    doc.regenerate_rect(
        gd,
        SlabRect {
            x0: 0,
            y0: 0,
            x1: 39,
            y1: 29,
        },
    );
    doc.place_door(gd, 9, 5, 3, 1, true).unwrap();
    doc.apply_slab_objects(
        gd,
        SlabRect {
            x0: 0,
            y0: 0,
            x1: 39,
            y1: 29,
        },
    );
    let imp = gd.creature_id("IMP").unwrap() as i64;
    let mut t = things::new_thing(
        class::CREATURE,
        imp,
        1,
        things::centred(11),
        things::centred(24),
        doc.floor_z(11, 24),
    );
    t.creature_level = Some(4);
    doc.add_thing(gd, t);
    doc.add_action_point(
        things::centred(32),
        things::centred(18),
        kfx_formats::level_fx::StlCoord::new(3, 0),
        Some(7),
    );
    doc.add_light(
        things::centred(28),
        things::centred(28),
        kfx_formats::level_fx::StlCoord::new(3, 0),
        kfx_formats::level_fx::StlCoord::new(5, 0),
        36,
    );
    let gate = things::hero_gate_model(gd).unwrap();
    let mut g = things::new_thing(
        class::OBJECT,
        gate,
        4,
        things::centred(30),
        things::centred(25),
        doc.floor_z(30, 25),
    );
    g.herogate_number = Some(9);
    doc.add_thing(gd, g);
    doc
}
