mod common;
use common::*;
use kfx_map::autotile::SlabRect;
use kfx_map::validate::{validate, Severity};
use kfx_prefab::*;

/// A blank earth map with a heart for players 0, 1 and 2 in the top-left corner.
fn host_map(gd: &kfx_map::gamedata::GameData) -> kfx_map::mapdoc::MapDoc {
    let mut doc = blank(gd, 70, 60);
    let heart = gd.slab_id("HEART_PEDESTAL").unwrap() as u16;
    for (i, owner) in [0u8, 1, 2].into_iter().enumerate() {
        for y in 3..6usize {
            for x in 3 + i * 5..6 + i * 5 {
                doc.place_slab(gd, x, y, heart, owner).unwrap();
            }
        }
    }
    doc.apply_slab_objects(
        gd,
        SlabRect {
            x0: 0,
            y0: 0,
            x1: 20,
            y1: 10,
        },
    );
    doc
}

#[test]
fn every_starter_prefab_applies_and_validates() {
    let Some(gd) = gd() else { return };
    let all = starter::build_all(gd).unwrap();
    assert!(all.len() >= 20, "{} prefabs", all.len());
    let mut bad = vec![];
    for (id, p) in &all {
        for o in [
            Orientation::default(),
            Orientation {
                rotation: Rotation::R90,
                flip: true,
            },
            Orientation {
                rotation: Rotation::R180,
                flip: false,
            },
            Orientation {
                rotation: Rotation::R270,
                flip: false,
            },
        ] {
            let mut doc = host_map(gd);
            let opts = ApplyOptions {
                orientation: o,
                owners: OwnerMap::with_first(&[0]),
                ..Default::default()
            };
            let (_, rep) = apply(&mut doc, gd, p, (25, 20), &opts).unwrap();
            assert!(!rep.has_missing(), "{id}: {}", rep.summary());
            for i in validate(&doc, gd)
                .into_iter()
                .filter(|i| i.severity >= Severity::Warning)
            {
                bad.push(format!(
                    "{id} {o:?}: {:?} {} {}",
                    i.severity, i.code, i.message
                ));
            }
        }
    }
    println!("{} starter prefabs; problems: {}", all.len(), bad.len());
    for b in &bad {
        println!("  {b}");
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
