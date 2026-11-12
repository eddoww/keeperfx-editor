//! Custom (fake) slabs, the sidecar file and the `.une` custom slab format of the Unearth editor.
use kfx_formats::level_bin::{Column, MapDims};
use kfx_formats::testutil;
use kfx_map::autotile::{kinds, SlabRect};
use kfx_map::gamedata::{pack_folders, ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns};
use kfx_map::patch::DocState;
use kfx_map::sidecar::{self, CustomObject, CustomObjectKind, CustomSlab, LockedRegion, Note};
use kfx_map::transform::{Symmetry, SymmetryMode, Transform};
use kfx_map::unearth;
use std::path::Path;

fn gd() -> Option<GameData> {
    let root = testutil::install_dir()?;
    Some(GameData::load(&ConfigLayers::for_level(&root, None, None, None, None)).unwrap())
}

fn base(gd: &GameData) -> MapDoc {
    let mut doc = MapDoc::new(Path::new("/tmp"), 1, MapDims::new(16, 16), 0, gd);
    for y in 1..15 {
        for x in 1..15 {
            doc.place_slab(gd, x, y, kinds::CLAIMED as u16, 0).unwrap();
        }
    }
    doc
}

fn pillar(gd: &GameData) -> CustomSlab {
    let mut cols = [Column::default(); 9];
    for (i, c) in cols.iter_mut().enumerate() {
        c.cubes[0] = 1;
        c.cubes[1] = 2 + (i as u16 % 3);
        c.floor_texture = 5 + i as u16;
    }
    let _ = gd;
    let mut s = CustomSlab::new("pillar", kinds::CLAIMED as u16, cols);
    s.wibble = [0, 1, 2, 0, 1, 2, 0, 1, 2];
    s
}

#[test]
fn custom_slab_survives_neighbours_and_regeneration() {
    let Some(gd) = gd() else { return };
    let mut doc = base(&gd);
    let id = doc.edit_sidecar(|s| s.add_custom_slab(pillar(&gd)));
    assert_eq!(id, 1000);
    doc.place_custom_slab(&gd, 8, 8, id, 0).unwrap();
    let def = doc.sidecar.custom_slab(id).unwrap().clone();
    let check = |doc: &MapDoc, what: &str| {
        for i in 0..9 {
            let k = doc.dims.stl_index(24 + i % 3, 24 + i / 3);
            assert!(
                kfx_map::column::column_equivalent(&doc.columns[k], &def.columns[i]),
                "{what}: column {i}"
            );
            assert_eq!(doc.wibble[k], def.wibble[i], "{what}: wibble {i}");
        }
        assert_eq!(doc.kinds[doc.dims.slab_index(8, 8)], kinds::CLAIMED as u16);
    };
    check(&doc, "placed");
    // walls and rooms around, then a regeneration of the whole map and of its columns
    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1), (1, 1), (-1, -1)] {
        let k = gd.slab_id("DRAPE_WALL").unwrap() as u16;
        doc.place_slab_with_things(&gd, (8 + dx) as usize, (8 + dy) as usize, k, 0)
            .unwrap();
    }
    check(&doc, "after neighbours");
    doc.regenerate_columns(&gd);
    check(&doc, "after regenerate_columns");
    let dec = kfx_map::decorate::Decorator::new(&gd, 3);
    doc.regenerate_columns_decorated(&gd, &dec);
    check(&doc, "after decorated regeneration");
    // a saved map with SaveColumns::Regenerate keeps it as well
    let dir =
        kfx_formats::testutil::scratch_dir(&format!("kfx-map/kfx_custom_{}", std::process::id()));
    doc.save(&gd, &dir, SaveColumns::Regenerate).unwrap();
    let again = MapDoc::load(&dir, 1, doc.dims, &gd).unwrap();
    assert_eq!(again.custom_cells, doc.custom_cells);
    assert_eq!(again.sidecar, doc.sidecar);
    for i in 0..9 {
        let k = again.dims.stl_index(24 + i % 3, 24 + i / 3);
        assert_eq!(
            again.columns[k].cubes, def.columns[i].cubes,
            "reloaded column {i}"
        );
    }
    // placing a normal slab over it clears the custom id
    doc.place_slab(&gd, 8, 8, kinds::CLAIMED as u16, 0).unwrap();
    assert_eq!(doc.custom_cells[doc.dims.slab_index(8, 8)], 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn sidecar_round_trip_and_removal() {
    let Some(gd) = gd() else { return };
    let mut doc = base(&gd);
    let mut def = pillar(&gd);
    def.objects.push(CustomObject {
        kind: CustomObjectKind::Light {
            intensity: 30,
            radius: 1280,
        },
        dx: 384,
        dy: 384,
        z: 512,
        orientation: 0,
    });
    def.objects.push(CustomObject {
        kind: CustomObjectKind::EffectGen {
            model: 1,
            range: 256,
        },
        dx: 100,
        dy: 200,
        z: 0,
        orientation: 512,
    });
    def.liquid = Some(0);
    let id = doc.edit_sidecar(|s| {
        let id = s.add_custom_slab(def);
        s.decoration.seed = Some(99);
        s.decoration.enabled = false;
        s.decoration.disabled_families = vec!["DIRT".into()];
        let mut sym = Symmetry::new(SymmetryMode::FourWay);
        sym.owners[1][0] = 1;
        s.symmetry = Some(sym);
        s.locked.push(LockedRegion {
            x0: 1,
            y0: 2,
            x1: 3,
            y1: 4,
            note: "hands off".into(),
        });
        s.notes.push(Note {
            x: 5,
            y: 6,
            text: "heart goes \"here\"\nsecond line".into(),
            color: Some("#ff8800".into()),
        });
        id
    });
    doc.place_custom_slab(&gd, 4, 4, id, 1).unwrap();
    doc.place_custom_slab(&gd, 9, 3, id, 0).unwrap();
    let dir =
        kfx_formats::testutil::scratch_dir(&format!("kfx-map/kfx_sidecar_{}", std::process::id()));
    doc.save(&gd, &dir, SaveColumns::Current).unwrap();
    let path = sidecar::sidecar_path(&dir, 1);
    let text = std::fs::read_to_string(&path).unwrap();
    println!("sidecar is {} bytes", text.len());
    let again = MapDoc::load(&dir, 1, doc.dims, &gd).unwrap();
    assert!(
        again.warnings.iter().all(|w| !w.contains("sidecar")),
        "{:?}",
        again.warnings
    );
    assert_eq!(again.sidecar, doc.sidecar);
    assert_eq!(again.custom_cells, doc.custom_cells);
    assert!(again.sidecar.locked_at(2, 3).is_some() && again.sidecar.locked_at(5, 5).is_none());
    assert_eq!(again.sidecar.notes_at(5, 6).count(), 1);
    // the game never sees it: no other map file mentions it, and nothing named .kfxed is a game
    // file
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with(".kfxed.toml"));
    // a map without custom data has no sidecar and a stale one is removed
    let mut plain = again;
    plain.sidecar = Default::default();
    plain.custom_cells.iter_mut().for_each(|c| *c = 0);
    plain.save(&gd, &dir, SaveColumns::Current).unwrap();
    assert!(!path.exists());
    // a broken sidecar is reported, not fatal, and the map is read-only until it is discarded
    // (saving would replace it)
    std::fs::write(&path, "version = [").unwrap();
    let mut broken = MapDoc::load(&dir, 1, plain.dims, &gd).unwrap();
    assert!(
        broken
            .warnings
            .iter()
            .any(|w| w.contains("kfxed.toml") && w.contains("could not be read")),
        "{:?}",
        broken.warnings
    );
    assert!(broken.read_only_reason().is_some());
    assert!(broken.save(&gd, &dir, SaveColumns::Current).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "version = [");
    broken.discard_unreadable();
    broken.save(&gd, &dir, SaveColumns::Current).unwrap();
    assert!(!path.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn custom_slab_objects_follow_the_slab() {
    let Some(gd) = gd() else { return };
    let mut doc = base(&gd);
    let mut def = pillar(&gd);
    // a torch object on the centre subtile; its subtile must not be blocking (floor-only column)
    def.columns[4] = kfx_map::column::canonical_column(
        &Column {
            floor_texture: 3,
            ..Default::default()
        },
        true,
    );
    def.objects.push(CustomObject {
        kind: CustomObjectKind::Object { model: 2 },
        dx: 384,
        dy: 384,
        z: 256,
        orientation: 0,
    });
    let id = doc.edit_sidecar(|s| s.add_custom_slab(def));
    doc.place_custom_slab(&gd, 6, 6, id, 2).unwrap();
    doc.apply_slab_objects(&gd, SlabRect::single(6, 6).grow(1));
    let slab_no = kfx_map::autotile::slab_number(doc.dims, 6, 6) as i64;
    let mine: Vec<_> = doc
        .things
        .items
        .iter()
        .filter(|t| t.parent_tile == Some(slab_no))
        .collect();
    assert_eq!(mine.len(), 1, "one custom object");
    assert_eq!(mine[0].subtype, Some(2));
    assert_eq!(mine[0].owner, Some(2));
    assert_eq!((mine[0].x.unwrap().stl, mine[0].y.unwrap().stl), (19, 19));
    // replacing the slab removes it again
    doc.place_slab_with_things(&gd, 6, 6, kinds::CLAIMED as u16, 0)
        .unwrap();
    assert!(doc
        .things
        .items
        .iter()
        .all(|t| t.parent_tile != Some(slab_no)));
}

#[test]
fn patches_cover_custom_slabs_and_sidecar() {
    let Some(gd) = gd() else { return };
    let mut doc = base(&gd);
    let start = DocState::of(&doc);
    let (id, p1) = doc.record_tracked(|d| d.edit_sidecar(|s| s.add_custom_slab(pillar(&gd))));
    let (_, p2) = doc.record_tracked(|d| {
        d.place_custom_slab(&gd, 3, 3, id, 0).unwrap();
        d.edit_sidecar(|s| {
            s.notes.push(Note {
                x: 3,
                y: 3,
                text: "x".into(),
                color: None,
            })
        });
    });
    let after = DocState::of(&doc);
    assert!(!p1.is_empty() && !p2.is_empty());
    p2.revert(&mut doc);
    assert!(doc.custom_cells.iter().all(|&c| c == 0));
    assert!(doc.sidecar.notes.is_empty() && doc.sidecar.custom_slab(id).is_some());
    p1.revert(&mut doc);
    assert_eq!(DocState::of(&doc), start);
    p1.apply(&mut doc);
    p2.apply(&mut doc);
    assert_eq!(DocState::of(&doc), after);
    // the snapshot recorder agrees
    let mut doc2 = base(&gd);
    doc2.record(|d| d.edit_sidecar(|s| s.add_custom_slab(pillar(&gd))));
    let (_, q) = doc2.record(|d| {
        d.place_custom_slab(&gd, 3, 3, id, 0).unwrap();
        d.edit_sidecar(|s| {
            s.notes.push(Note {
                x: 3,
                y: 3,
                text: "x".into(),
                color: None,
            })
        });
    });
    assert_eq!(q, p2);
}

#[test]
fn transforms_move_custom_slabs() {
    let Some(gd) = gd() else { return };
    let mut doc = base(&gd);
    let id = doc.edit_sidecar(|s| s.add_custom_slab(pillar(&gd)));
    doc.place_custom_slab(&gd, 2, 3, id, 0).unwrap();
    doc.transform_map(&gd, Transform::FlipX);
    let i = doc.dims.slab_index(13, 3);
    let nid = doc.custom_cells[i];
    assert_ne!(nid, 0, "the custom slab moved with the map");
    let old = doc.sidecar.custom_slab(id).unwrap().clone();
    let new = doc.sidecar.custom_slab(nid).unwrap().clone();
    // column 0 (north-west) of the original is column 2 (north-east) of the mirrored one
    assert_eq!(new.columns[2], old.columns[0]);
    assert_eq!(new.columns[0], old.columns[2]);
    let k = doc.dims.stl_index(3 * 13 + 2, 3 * 3);
    assert!(kfx_map::column::column_equivalent(
        &doc.columns[k],
        &new.columns[2]
    ));
    // flipping back finds the original definition again
    doc.transform_map(&gd, Transform::FlipX);
    assert_eq!(doc.custom_cells[doc.dims.slab_index(2, 3)], id);
    // resize keeps the cell with its slab
    doc.resize(&gd, 3, 0, 0, 0, kinds::ROCK as u16, 5).unwrap();
    assert_eq!(doc.custom_cells[doc.dims.slab_index(5, 3)], id);
}

#[test]
fn custom_slabs_cfg_parsing() {
    let t = "[slab12]\nname=\"x\"\n[slab1000]\nname=\"fancy\"\nrecognized_as=11\nliquid_type=0\n\
        cubes0=[ 1, 2, 3 ]\nfloor0=5\ncubes1=[ 4 ]\nfloor1=6\n\n[slab1001]\nname=\"b\"\n";
    let d = unearth::parse_custom_slabs_cfg(t);
    assert_eq!(d.len(), 2);
    assert_eq!(d[0].id, 1000);
    assert_eq!(d[0].name, "fancy");
    assert_eq!(d[0].recognized_as, Some(11));
    assert_eq!(d[0].cubes, vec![vec![1, 2, 3], vec![4]]);
    assert_eq!(d[0].floor, vec![5, 6]);
    assert_eq!(d[1].id, 1001);
}

#[test]
fn import_une_with_and_without_cfg() {
    let Some(gd) = gd() else { return };
    let mut doc = base(&gd);
    let mut une = vec![0u16; 256];
    une[3 * 16 + 4] = 1000;
    une[5 * 16 + 5] = 1000;
    une[7 * 16 + 7] = 1001;
    let bytes = unearth::write_une(&une);
    let cfg = "[slab1000]\nname=\"fancy\"\nrecognized_as=11\ncubes0=[1]\ncubes1=[1]\ncubes2=[1]\n\
        cubes3=[1]\ncubes4=[1,2]\ncubes5=[1]\ncubes6=[1]\ncubes7=[1]\ncubes8=[1]\nfloor0=1\n\
        floor1=1\nfloor2=1\nfloor3=1\nfloor4=7\nfloor5=1\nfloor6=1\nfloor7=1\nfloor8=1\n";
    let start = DocState::of(&doc);
    let (r, patch) = doc.record_tracked(|d| d.import_une(&gd, &bytes, Some(cfg)).unwrap());
    assert_eq!(r.cells, 3);
    assert_eq!(
        r.definitions,
        vec![
            (1000, unearth::UneSource::CustomSlabsCfg),
            (1001, unearth::UneSource::MapColumns)
        ]
    );
    assert_eq!(
        doc.sidecar.custom_slab(1000).unwrap().columns[4].cubes[..2],
        [1, 2]
    );
    assert_eq!(
        doc.sidecar.custom_slab(1000).unwrap().columns[4].floor_texture,
        7
    );
    assert_eq!(doc.sidecar.custom_slab(1000).unwrap().name, "fancy");
    assert_eq!(
        doc.sidecar.custom_slab(1001).unwrap().recognized_as,
        kinds::CLAIMED as u16
    );
    assert_eq!(doc.export_une(), bytes);
    patch.revert(&mut doc);
    assert_eq!(DocState::of(&doc), start);
    assert!(unearth::parse_une(&bytes[..10], doc.dims).is_err());
}

/// Every shipped `.une` loads (MapDoc::load imports it) and survives save + reload.
#[test]
fn shipped_une_files_import() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let (mut files, mut cells, mut defs, mut empty, mut saved_ok) = (0, 0, 0, 0, 0);
    for (levels, cfgs, crtr) in pack_folders(&root) {
        let gd = GameData::load(&ConfigLayers::for_level(
            &root,
            cfgs.as_deref(),
            Some(&levels),
            crtr.as_deref(),
            None,
        ))
        .unwrap();
        for e in std::fs::read_dir(&levels).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            let Some(n) = name
                .strip_prefix("map")
                .and_then(|n| n.strip_suffix(".une"))
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let Some(slb) = kfx_formats::level_bin::find_file(&levels, n, "slb")
                .and_then(|p| kfx_formats::rnc::load_file(&p).ok())
            else {
                continue;
            };
            let Some(dims) = MapDims::from_slb_len(slb.len()) else {
                continue;
            };
            files += 1;
            let doc = MapDoc::load(&levels, n, dims, &gd).unwrap();
            let c = doc.custom_cells.iter().filter(|&&c| c != 0).count();
            if c == 0 {
                empty += 1;
            }
            cells += c;
            defs += doc.sidecar.custom_slabs.len();
            for &id in doc.custom_cells.iter().filter(|&&c| c != 0) {
                assert!(
                    doc.sidecar.custom_slab(id).is_some(),
                    "map {n}: id {id} undefined"
                );
            }
            let dir = kfx_formats::testutil::scratch_dir(&format!(
                "kfx-map/kfx_une_{}_{n}",
                std::process::id()
            ));
            if c > 0 {
                doc.save_sidecar(&dir).unwrap_or_else(|_| {
                    std::fs::create_dir_all(&dir).unwrap();
                    doc.save_sidecar(&dir).unwrap()
                });
                let (sc, cells2, _) = sidecar::load(&sidecar::sidecar_path(&dir, n), dims).unwrap();
                assert_eq!(sc, doc.sidecar);
                assert_eq!(cells2, doc.custom_cells);
                saved_ok += 1;
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }
    println!(
        "[une] {files} shipped .une files: {cells} fake slab cells, {defs} definitions \
        rebuilt from the map's columns, {empty} files without any fake slab, {saved_ok} sidecars \
        round-tripped"
    );
}
