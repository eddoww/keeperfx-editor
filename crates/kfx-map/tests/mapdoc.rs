//! MapDoc load / edit / save on a real map of the test install (skipped when absent).
use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::{rnc, testutil};
use kfx_map::autotile::{kinds, MapGen, SlabView};
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns, ThingFormat};
use std::path::PathBuf;

fn setup() -> Option<(PathBuf, GameData)> {
    let root = testutil::install_dir()?;
    let camp = testutil::resolve_ci(&root, "campgns/keeporig")?;
    let cfgs = testutil::resolve_ci(&root, "levels/classic_cfgs");
    let layers = ConfigLayers::for_level(&root, cfgs.as_deref(), Some(&camp), None, Some(1));
    Some((camp, GameData::load(&layers).unwrap()))
}

fn dims_of(dir: &std::path::Path, n: u32) -> MapDims {
    let slb = rnc::load_file(&level_bin::find_file(dir, n, "slb").unwrap()).unwrap();
    MapDims::from_slb_len(slb.len()).unwrap()
}

#[test]
fn incremental_edits_equal_full_regeneration() {
    let Some((dir, gd)) = setup() else { return };
    let mut doc = MapDoc::load(&dir, 1, dims_of(&dir, 1), &gd).unwrap();
    assert_eq!(doc.loaded_format, ThingFormat::Legacy);
    doc.regenerate_columns(&gd);
    // A deterministic scatter of edits, including map borders and liquids.
    let edits: [(usize, usize, u16, u8); 10] = [
        (0, 0, kinds::WATER as u16, 5),
        (1, 0, kinds::CLAIMED as u16, 0),
        (40, 40, 16, 0),
        (41, 40, 16, 0),
        (42, 41, kinds::WALLDRAPE as u16, 1),
        (84, 84, kinds::LAVA as u16, 5),
        (83, 84, kinds::PATH as u16, 5),
        (20, 30, 42, 2),
        (21, 30, kinds::EARTH as u16, 5),
        (10, 10, kinds::GOLD as u16, 5),
    ];
    for (x, y, k, o) in edits {
        let r = doc.set_slab(&gd, x, y, k).unwrap();
        assert!(r.x0 <= x as i32 && r.x1 >= x as i32);
        doc.set_owner(&gd, x, y, o).unwrap();
        doc.place_slab(&gd, x, (y + 2).min(84), kinds::EARTH as u16, 5)
            .unwrap();
    }
    let full = MapGen::regenerate_all(&doc.view(&gd), &gd);
    let d = doc.dims;
    for y in 0..d.stl_h() {
        for x in 0..d.stl_w() {
            let k = d.stl_index(x, y);
            assert_eq!(doc.columns[k], full.columns[k], "subtile ({x},{y})");
            assert_eq!(doc.wibble[k], full.wibble[k], "wibble ({x},{y})");
        }
    }
}

#[test]
fn save_and_reload() {
    let Some((dir, gd)) = setup() else { return };
    let dims = dims_of(&dir, 1);
    let mut doc = MapDoc::load(&dir, 1, dims, &gd).unwrap();
    let out =
        kfx_formats::testutil::scratch_dir(&format!("kfx-map/kfx_map_save_{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    // A stale legacy file must disappear, an existing upper-case file must be replaced.
    std::fs::write(out.join("MAP00001.TNG"), [0u8, 0]).unwrap();
    std::fs::write(out.join("MAP00001.SLB"), [0u8; 4]).unwrap();
    let r = doc.set_slab(&gd, 30, 30, kinds::CLAIMED as u16).unwrap();
    let (removed, added) = doc.apply_slab_objects(&gd, r);
    println!("slab objects around (30,30): removed {removed}, added {added}");
    doc.save(&gd, &out, SaveColumns::Current).unwrap();
    for ext in [
        "slb", "own", "wlb", "inf", "wib", "clm", "dat", "tngfx", "aptfx", "lgtfx",
    ] {
        assert!(
            out.join(level_bin::file_name(1, ext)).is_file(),
            "{ext} written"
        );
    }
    assert!(level_bin::find_file(&out, 1, "tng").is_none());
    // by listing, not `exists()`: on a case-insensitive file system that would find map00001.slb
    let names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!names.iter().any(|n| n == "MAP00001.SLB"), "{names:?}");
    assert!(names.iter().any(|n| n == "map00001.slb"), "{names:?}");
    let back = MapDoc::load(&out, 1, dims, &gd).unwrap();
    assert_eq!(back.loaded_format, ThingFormat::Fx);
    assert_eq!(back.kinds, doc.kinds);
    assert_eq!(back.owners, doc.owners);
    assert_eq!(back.wlb, doc.wlb);
    assert_eq!(back.texture_id, doc.texture_id);
    assert_eq!(back.things.items.len(), doc.things.items.len());
    assert_eq!(
        back.action_points.items.len(),
        doc.action_points.items.len()
    );
    assert_eq!(back.lights.items.len(), doc.lights.items.len());
    for k in 0..dims.stl_grid() {
        let (a, b) = (&back.columns[k], &doc.columns[k]);
        assert_eq!(
            (a.cubes, a.floor_texture, a.solidmask, a.orient),
            (b.cubes, b.floor_texture, b.solidmask, b.orient)
        );
    }
    // The regenerated save fits the game's column pool too.
    doc.save(&gd, &out, SaveColumns::Regenerate).unwrap();
    let clm =
        level_bin::Clm::parse(&std::fs::read(out.join("map00001.clm")).unwrap(), dims).unwrap();
    println!(
        "regenerated map00001: {} columns in .clm",
        clm.columns.len()
    );
    let v = back.view(&gd);
    assert_eq!(v.kind(30, 30), Some(kinds::CLAIMED));
    std::fs::remove_dir_all(&out).unwrap();
}

/// `place_slabs` is `place_slab` in a loop (same kinds, owners, `.wlb`, columns, wibble and
/// changed rectangle) on a large, mixed batch: every kind of the configuration, all owners,
/// liquids next to bridges, rooms next to earth (the torch-dirt restyling), slabs placed twice
/// and the map borders. A slab outside the map stops both the same way.
#[test]
fn place_slabs_equals_place_slab_loop() {
    let Some((dir, gd)) = setup() else { return };
    let base = MapDoc::load(&dir, 1, dims_of(&dir, 1), &gd).unwrap();
    let d = base.dims;
    let nk = gd.slab_count() as u64;
    // a fixed pseudo-random sequence (xorshift), biased towards earth so restyling happens
    let mut s = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut batch: Vec<(usize, usize, u16, u8)> = vec![];
    for _ in 0..6000 {
        let r = next();
        let (x, y) = ((r % d.w as u64) as usize, ((r >> 16) % d.h as u64) as usize);
        let kind = if (r >> 32) % 3 == 0 {
            kinds::EARTH as u16
        } else {
            ((r >> 40) % nk) as u16
        };
        batch.push((x, y, kind, ((r >> 56) % 6) as u8));
    }
    for x in 0..d.w as usize {
        batch.push((x, 0, kinds::CLAIMED as u16, 0));
        batch.push((x, d.h as usize - 1, kinds::WATER as u16, 5));
    }
    let t0 = std::time::Instant::now();
    let mut one = base.clone();
    let mut union: Option<kfx_map::autotile::SlabRect> = None;
    for &(x, y, k, o) in &batch {
        let r = one.place_slab(&gd, x, y, k, o).unwrap();
        union = Some(union.map_or(r, |u| u.union(r)));
    }
    let t_loop = t0.elapsed();
    let t0 = std::time::Instant::now();
    let mut bulk = base.clone();
    let r = bulk.place_slabs(&gd, batch.iter().copied()).unwrap();
    let t_bulk = t0.elapsed();
    eprintln!(
        "{} slabs on {}x{}: place_slab loop {t_loop:?}, place_slabs {t_bulk:?}",
        batch.len(),
        d.w,
        d.h
    );
    assert_eq!(r, union);
    assert_eq!(bulk.kinds, one.kinds);
    assert_eq!(bulk.owners, one.owners);
    assert_eq!(bulk.wlb, one.wlb);
    assert_eq!(bulk.custom_cells, one.custom_cells);
    assert!(bulk.columns == one.columns, "columns differ");
    assert_eq!(bulk.wibble, one.wibble);
    assert_eq!(bulk.place_slabs(&gd, std::iter::empty()).unwrap(), None);
    // an error part-way: the slabs before it are placed, the ones after are not
    let bad = [
        (3usize, 3usize, kinds::LAVA as u16, 5u8),
        (d.w as usize, 1, 0, 0),
        (4, 3, kinds::LAVA as u16, 5),
    ];
    let mut a = base.clone();
    assert!(a.place_slabs(&gd, bad).is_err());
    let mut b = base.clone();
    b.place_slab(&gd, 3, 3, kinds::LAVA as u16, 5).unwrap();
    assert_eq!(a.kinds, b.kinds);
    assert!(a.columns == b.columns);
}
