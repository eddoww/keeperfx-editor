//! The validation checks added in the second round: `door-cannot-stand`, `gold-in-treasury`,
//! reachability.
use kfx_formats::level_bin::MapDims;
use kfx_formats::level_fx::StlCoord;
use kfx_formats::testutil;
use kfx_map::autotile::kinds;
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::MapDoc;
use kfx_map::things::{self, class};
use kfx_map::validate::{validate, Severity};
use std::path::Path;

fn gd() -> Option<GameData> {
    let root = testutil::install_dir()?;
    Some(GameData::load(&ConfigLayers::for_level(&root, None, None, None, None)).unwrap())
}

fn slab(gd: &GameData, n: &str) -> u16 {
    gd.slab_id(n).unwrap() as u16
}

fn map(gd: &GameData) -> MapDoc {
    let mut doc = MapDoc::new(Path::new("/tmp"), 1, MapDims::new(30, 30), 0, gd);
    for y in 1..29 {
        for x in 1..29 {
            doc.place_slab(gd, x, y, kinds::EARTH as u16, 5).unwrap();
        }
    }
    for y in 3..27 {
        for x in 3..27 {
            doc.place_slab(gd, x, y, kinds::CLAIMED as u16, 0).unwrap();
        }
    }
    doc
}

fn count(doc: &MapDoc, gd: &GameData, code: &str) -> usize {
    validate(doc, gd).iter().filter(|i| i.code == code).count()
}

#[test]
fn door_cannot_stand_table() {
    // walls on E+W or N+S stand, one wall or all four do not
    use kfx_map::things::{door_angle_for_walls, BUILD_DOOR_ANGLE};
    assert_eq!(door_angle_for_walls([false, true, false, true]), Some(0));
    assert_eq!(door_angle_for_walls([true, false, true, false]), Some(1));
    assert_eq!(door_angle_for_walls([true, true, true, true]), None);
    assert_eq!(door_angle_for_walls([false, false, false, true]), None);
    assert_eq!(BUILD_DOOR_ANGLE.iter().filter(|&&a| a >= 0).count(), 6);
    let Some(gd) = gd() else { return };
    let mut doc = map(&gd);
    let wall = slab(&gd, "DRAPE_WALL");
    doc.place_slab(&gd, 9, 10, wall, 0).unwrap();
    doc.place_slab(&gd, 11, 10, wall, 0).unwrap();
    doc.place_door(&gd, 10, 10, 1, 0, false).unwrap();
    assert!(doc.door_can_stand(&gd, 10, 10));
    assert_eq!(count(&doc, &gd, "door-cannot-stand"), 0);
    // a second door right next to the first one: the neighbouring door is not a wall
    doc.place_slab(&gd, 12, 10, wall, 0).unwrap();
    doc.place_door(&gd, 11, 10, 1, 0, false).unwrap();
    assert!(!doc.door_can_stand(&gd, 11, 10));
    assert!(!doc.door_can_stand(&gd, 10, 10));
    let errs: Vec<_> = validate(&doc, &gd)
        .into_iter()
        .filter(|i| i.code == "door-cannot-stand")
        .collect();
    assert_eq!(errs.len(), 2);
    assert!(errs.iter().all(|i| i.severity == Severity::Error));
    // the edge of the map counts as wall
    let mut doc = map(&gd);
    doc.place_slab(&gd, 1, 0, slab(&gd, "DOOR_WOODEN"), 0)
        .unwrap();
    assert!(doc.door_stand_walls(&gd, 0, 0).is_some());
}

#[test]
fn loose_gold_in_a_treasury_is_noted() {
    let Some(gd) = gd() else { return };
    let mut doc = map(&gd);
    let gold = gd.object_id("GOLD").expect("GOLD object");
    for x in 5..8 {
        doc.place_slab(&gd, x, 5, slab(&gd, "TREASURY_AREA"), 0)
            .unwrap();
    }
    let put = |doc: &mut MapDoc, sx: i32, sy: i32| {
        let t = things::new_thing(
            class::OBJECT,
            gold as i64,
            5,
            things::centred(sx),
            things::centred(sy),
            StlCoord::new(1, 0),
        );
        doc.add_thing(&gd, t);
    };
    put(&mut doc, 16, 16); // 5,5 treasury, owned
    put(&mut doc, 40, 40); // claimed floor elsewhere
    assert_eq!(count(&doc, &gd, "gold-in-treasury"), 1);
    // a neutral treasury keeps its gold (src/room_treasure.c:97)
    doc.set_owner(&gd, 5, 5, 5).unwrap();
    assert_eq!(count(&doc, &gd, "gold-in-treasury"), 0);
}

#[test]
fn unreachable_pockets() {
    let Some(gd) = gd() else { return };
    let mut doc = map(&gd);
    let heart = gd.object_id("SOUL_CONTAINER").expect("heart object");
    // heart in the claimed area
    let h = things::new_thing(
        class::OBJECT,
        heart as i64,
        0,
        things::centred(10 * 3 + 1),
        things::centred(10 * 3 + 1),
        StlCoord::new(1, 0),
    );
    doc.add_thing(&gd, h);
    // a pocket (2x2) sealed with HARD rock, one with DIRT (diggable) walls
    let hard = slab(&gd, "HARD");
    for (ox, oy, rock) in [(15usize, 15usize, hard), (20, 20, slab(&gd, "DIRT"))] {
        for y in oy - 1..oy + 3 {
            for x in ox - 1..ox + 3 {
                doc.place_slab(&gd, x, y, rock, 5).unwrap();
            }
        }
        for y in oy..oy + 2 {
            for x in ox..ox + 2 {
                doc.place_slab(&gd, x, y, kinds::CLAIMED as u16, 0).unwrap();
            }
        }
    }
    let bat = 1i64;
    let in_pocket = |doc: &mut MapDoc, cls: u8, model: i64, sx: i32, sy: i32| {
        let z = doc.floor_z(sx, sy);
        let t = things::new_thing(cls, model, 5, things::centred(sx), things::centred(sy), z);
        doc.add_thing(&gd, t);
    };
    in_pocket(&mut doc, class::CREATURE, bat, 46, 46); // hard pocket
    in_pocket(&mut doc, class::CREATURE, bat, 61, 61); // dirt pocket: digging reaches it
    let trap = gd
        .trap_names()
        .into_iter()
        .next()
        .map(|(i, _)| i as i64)
        .unwrap();
    in_pocket(&mut doc, class::TRAP, trap, 47, 47);
    let issues = validate(&doc, &gd);
    let un: Vec<_> = issues
        .iter()
        .filter(|i| i.code == "creature-unreachable" || i.code == "thing-unreachable")
        .collect();
    assert_eq!(un.len(), 2, "{un:?}");
    assert!(un
        .iter()
        .any(|i| i.code == "creature-unreachable" && i.severity == Severity::Info));
    assert!(un
        .iter()
        .any(|i| i.code == "thing-unreachable" && i.severity == Severity::Warning));
    // no heart / hero gate: nothing is reported
    let mut empty = map(&gd);
    for y in 12..14 {
        for x in 12..14 {
            empty.place_slab(&gd, x, y, hard, 5).unwrap();
        }
    }
    assert!(kfx_map::validate::unreachable_things(&empty, &gd).is_empty());
}
