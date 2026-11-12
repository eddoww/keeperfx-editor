//! Decoration families derived from the test install's configs (skipped when absent).
use kfx_formats::testutil;
use kfx_map::decorate::{Decorator, Families};
use kfx_map::gamedata::{ConfigLayers, GameData};

fn gd() -> Option<GameData> {
    let root = testutil::install_dir()?;
    Some(GameData::load(&ConfigLayers::for_level(&root, None, None, None, None)).unwrap())
}

#[test]
fn families_from_configs() {
    let Some(gd) = gd() else { return };
    let f = Families::derive(&gd);
    for c in &f.cubes {
        println!(
            "cube family {:<24} {:?} extra {:?}",
            c.name, c.members, c.extra
        );
    }
    for c in &f.floors {
        println!(
            "floor family {:<24} {:?} extra {:?}",
            c.name, c.members, c.extra
        );
    }
    let has = |m: &[u16]| f.cubes.iter().any(|c| c.members == m);
    for m in [
        &[1u16, 2, 3][..],
        &[25, 26, 27],
        &[49, 50, 51],
        &[441, 442, 443, 444],
        &[126, 127, 128],
        &[72, 73, 74],
    ] {
        assert!(has(m), "family {m:?} not derived");
    }
    let path = f.cubes.iter().find(|c| c.members == [25, 26, 27]).unwrap();
    assert_eq!(path.extra, [28, 29]);
    assert!(f.floors.iter().any(|c| c.members == [546, 547]));
}

#[test]
fn decoration_keeps_shape_and_is_deterministic() {
    let Some(gd) = gd() else { return };
    let dec = Decorator::new(&gd, 7);
    let mut seen = 0;
    for c in gd.columnset.iter() {
        for (x, y) in [(0usize, 0usize), (5, 9), (100, 3)] {
            let d = dec.column(c, x, y);
            assert_eq!(d, dec.column(c, x, y));
            for z in 0..8 {
                assert_eq!(d.cubes[z] == 0, c.cubes[z] == 0);
            }
            assert_eq!(
                (d.solidmask, d.bitfields, d.orient),
                (c.solidmask, c.bitfields, c.orient)
            );
            seen += (d != *c) as usize;
        }
    }
    println!("{seen} decorated columnset columns differ from the plain one");
    assert!(seen > 0);
}
