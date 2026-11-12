//! Document operations: things API, transforms, validation, patches (skipped without the
//! test install).
use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::level_fx::StlCoord;
use kfx_formats::{rnc, testutil};
use kfx_map::autotile::{kinds, SlabRect};
use kfx_map::decorate::Decorator;
use kfx_map::gamedata::{pack_folders, ConfigLayers, GameData};
use kfx_map::mapdoc::MapDoc;
use kfx_map::patch::{DocState, Patch};
use kfx_map::things::{self, class, door_slab_kinds, ItemRef, StlRect};
use kfx_map::transform::{Symmetry, SymmetryMode, Transform};
use kfx_map::validate::{validate, Severity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn base_gd() -> Option<(PathBuf, GameData)> {
    let root = testutil::install_dir()?;
    let gd = GameData::load(&ConfigLayers::for_level(&root, None, None, None, None)).unwrap();
    Some((root, gd))
}

fn slab(gd: &GameData, n: &str) -> u16 {
    gd.slab_id(n).unwrap() as u16
}

/// A small dug map with walls, for edits.
fn test_map(gd: &GameData, w: u16, h: u16) -> MapDoc {
    let mut doc = MapDoc::new(Path::new("/tmp"), 1, MapDims::new(w, h), 0, gd);
    for y in 1..h as usize - 1 {
        for x in 1..w as usize - 1 {
            doc.place_slab(gd, x, y, kinds::EARTH as u16, 5).unwrap();
        }
    }
    for y in 3..h as usize - 3 {
        for x in 3..w as usize - 3 {
            doc.place_slab(gd, x, y, kinds::CLAIMED as u16, 0).unwrap();
        }
    }
    doc
}

#[test]
fn door_slab_kind_follows_orientation_in_shipped_maps() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let (mut ok, mut bad, mut angle_agree, mut angle_total) = (0, 0, 0, 0);
    for (levels, cfgs, crtr) in pack_folders(&root) {
        let mut nums: Vec<u32> = std::fs::read_dir(&levels)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                n.strip_prefix("map")?.strip_suffix(".slb")?.parse().ok()
            })
            .collect();
        nums.sort();
        nums.dedup();
        let gd = GameData::load(&ConfigLayers::for_level(
            &root,
            cfgs.as_deref(),
            Some(&levels),
            crtr.as_deref(),
            None,
        ))
        .unwrap();
        for n in nums {
            let slb = rnc::load_file(&level_bin::find_file(&levels, n, "slb").unwrap()).unwrap();
            let Some(dims) = MapDims::from_slb_len(slb.len()) else {
                continue;
            };
            let Ok(doc) = MapDoc::load(&levels, n, dims, &gd) else {
                continue;
            };
            for t in &doc.things.items {
                if things::thing_class(t) != Some(class::DOOR) {
                    continue;
                }
                let Some((sx, sy)) = things::position(t.x, t.y) else {
                    continue;
                };
                let k = doc.kinds[dims.slab_index((sx / 3) as usize, (sy / 3) as usize)];
                let o = t.door_orientation.unwrap_or(0).clamp(0, 1) as usize;
                match t.subtype.and_then(|m| door_slab_kinds(&gd, m)) {
                    Some(ks) if ks[o] == k => ok += 1,
                    _ => bad += 1,
                }
                if let Some(a) = doc.door_angle(sx / 3, sy / 3) {
                    angle_total += 1;
                    angle_agree += (a as usize == o) as usize;
                }
            }
        }
    }
    println!(
        "door things whose slab kind is slbkind[orientation]: {ok} of {}; walls-based angle \
        agrees with the file: {angle_agree} of {angle_total}",
        ok + bad
    );
    // the game re-places the slab from the thing (src/thing_doors.c:140), so files may disagree;
    // with the order swapped this would be close to 0%
    assert!(
        ok > 0 && bad * 10 < ok,
        "door slab kind / orientation mapping wrong"
    );
    assert!(
        angle_agree * 10 >= angle_total * 9,
        "door angle rule disagrees with shipped maps"
    );
}

#[test]
fn doors_hero_gates_action_points() {
    let Some((_, gd)) = base_gd() else { return };
    let mut doc = test_map(&gd, 20, 20);
    // a wall pair east / west of (10, 5): door gets orientation 0
    doc.place_slab(&gd, 9, 5, slab(&gd, "DRAPE_WALL"), 0)
        .unwrap();
    doc.place_slab(&gd, 11, 5, slab(&gd, "DRAPE_WALL"), 0)
        .unwrap();
    let before = DocState::of(&doc);
    let (i, p) = doc.place_door(&gd, 10, 5, 1, 0, true).unwrap();
    let t = doc.things.items[i].clone();
    assert_eq!(t.door_orientation, Some(0));
    let ks = door_slab_kinds(&gd, 1).unwrap();
    assert_eq!(ks, [slab(&gd, "DOOR_WOODEN2"), slab(&gd, "DOOR_WOODEN")]);
    assert_eq!(doc.kinds[doc.dims.slab_index(10, 5)], ks[0]);
    let keys = doc.things_at(31, 16).len();
    assert_eq!(keys, 2, "door + key");
    // removing the door gives claimed path back and drops the key
    let p2 = doc.remove_thing(&gd, doc.door_on_slab(10, 5).unwrap());
    assert_eq!(doc.kinds[doc.dims.slab_index(10, 5)], kinds::CLAIMED as u16);
    assert!(doc.things_at(31, 16).is_empty());
    p2.revert(&mut doc);
    p.revert(&mut doc);
    assert_eq!(DocState::of(&doc), before);
    // a door slab placed between north / south walls gets orientation 1 and DOOR_WOODEN
    doc.place_slab(&gd, 5, 9, slab(&gd, "DRAPE_WALL"), 0)
        .unwrap();
    doc.place_slab(&gd, 5, 11, slab(&gd, "DRAPE_WALL"), 0)
        .unwrap();
    doc.place_slab_with_things(&gd, 5, 10, slab(&gd, "DOOR_WOODEN2"), 0)
        .unwrap();
    let d = doc.door_on_slab(5, 10).unwrap();
    assert_eq!(doc.things.items[d].door_orientation, Some(1));
    assert_eq!(doc.kinds[doc.dims.slab_index(5, 10)], ks[1]);
    // a non-door slab over it removes the door
    doc.place_slab_with_things(&gd, 5, 10, kinds::CLAIMED as u16, 0)
        .unwrap();
    assert!(doc.door_on_slab(5, 10).is_none());
    // hero gates and action points number themselves
    let hg = things::hero_gate_model(&gd).unwrap();
    for k in 0..3 {
        let t = things::new_thing(
            class::OBJECT,
            hg,
            4,
            things::centred(20 + k),
            things::centred(20),
            StlCoord::new(1, 0),
        );
        doc.add_thing(&gd, t);
    }
    let nums: Vec<_> = doc
        .things
        .items
        .iter()
        .filter(|t| things::is_hero_gate(&gd, t))
        .map(|t| t.herogate_number)
        .collect();
    assert_eq!(nums, vec![Some(1), Some(2), Some(3)]);
    let gate2 = doc
        .things
        .items
        .iter()
        .position(|t| t.herogate_number == Some(2))
        .unwrap();
    doc.remove_thing(&gd, gate2);
    let t = things::new_thing(
        class::OBJECT,
        hg,
        4,
        things::centred(30),
        things::centred(30),
        StlCoord::new(1, 0),
    );
    let (gi, _) = doc.add_thing(&gd, t);
    assert_eq!(
        doc.things.items[gi].herogate_number,
        Some(2),
        "smallest free number"
    );
    doc.things.items[gi].herogate_number = Some(9);
    doc.renumber_hero_gates(&gd);
    let mut nums: Vec<_> = doc
        .things
        .items
        .iter()
        .filter(|t| things::is_hero_gate(&gd, t))
        .filter_map(|t| t.herogate_number)
        .collect();
    nums.sort();
    assert_eq!(nums, vec![1, 2, 3]);
    for _ in 0..3 {
        doc.add_action_point(
            things::centred(10),
            things::centred(10),
            StlCoord::new(2, 0),
            None,
        );
    }
    doc.remove_action_point(0);
    let (ai, _) = doc.add_action_point(
        things::centred(12),
        things::centred(12),
        StlCoord::new(2, 0),
        None,
    );
    assert_eq!(doc.action_points.items[ai].number, Some(1));
    let items = doc.items_in(StlRect {
        x0: 9,
        y0: 9,
        x1: 13,
        y1: 13,
    });
    assert_eq!(
        items
            .iter()
            .filter(|i| matches!(i, ItemRef::ActionPoint(_)))
            .count(),
        3
    );
}

#[test]
fn ownership_follows_slab() {
    let Some((_, gd)) = base_gd() else { return };
    let mut doc = test_map(&gd, 20, 20);
    let lib = slab(&gd, "BOOK_SHELVES");
    for y in 5..8 {
        for x in 5..8 {
            doc.place_slab(&gd, x, y, lib, 0).unwrap();
        }
    }
    doc.apply_slab_objects(
        &gd,
        SlabRect {
            x0: 4,
            y0: 4,
            x1: 8,
            y1: 8,
        },
    );
    let book = gd
        .object_names()
        .into_iter()
        .map(|(i, _)| i as i64)
        .find(|&m| things::object_genre_is(&gd, m, "SPELLBOOK"))
        .unwrap();
    doc.add_thing(
        &gd,
        things::new_thing(
            class::OBJECT,
            book,
            0,
            things::centred(19),
            things::centred(19),
            StlCoord::new(1, 0),
        ),
    );
    let before = DocState::of(&doc);
    let p = doc.set_slab_owner_with_things(&gd, 6, 6, 2).unwrap();
    let on: Vec<_> = doc
        .things_on_slab(6, 6)
        .into_iter()
        .map(|i| doc.things.items[i].clone())
        .collect();
    assert!(
        on.iter()
            .any(|t| t.subtype == Some(book) && t.owner == Some(2)),
        "spellbook re-owned"
    );
    p.revert(&mut doc);
    assert_eq!(DocState::of(&doc), before);
}

#[test]
fn transforms_round_trip() {
    let Some((_, gd)) = base_gd() else { return };
    let mut doc = test_map(&gd, 23, 17);
    doc.place_slab(&gd, 4, 5, slab(&gd, "TREASURY_AREA"), 1)
        .unwrap();
    doc.add_thing(
        &gd,
        things::new_thing(
            class::CREATURE,
            1,
            0,
            StlCoord::new(13, 40),
            StlCoord::new(16, 200),
            StlCoord::new(1, 0),
        ),
    );
    let mut t = things::new_thing(
        class::TRAP,
        1,
        0,
        things::centred(30),
        things::centred(20),
        StlCoord::new(1, 0),
    );
    t.orientation = Some(300);
    doc.add_thing(&gd, t);
    doc.add_light(
        StlCoord::new(7, 9),
        StlCoord::new(8, 0),
        StlCoord::new(3, 0),
        StlCoord::new(5, 0),
        32,
    );
    doc.add_action_point(
        things::centred(40),
        things::centred(10),
        StlCoord::new(1, 0),
        None,
    );
    let key = |d: &MapDoc| {
        (
            d.dims,
            d.kinds.clone(),
            d.owners.clone(),
            d.things.items.clone(),
            d.lights.items.clone(),
            d.action_points.items.clone(),
        )
    };
    let orig = key(&doc);
    for _ in 0..4 {
        doc.transform_map(&gd, Transform::Rot90);
    }
    assert_eq!(key(&doc), orig, "four quarter turns");
    doc.transform_map(&gd, Transform::FlipX);
    assert_ne!(key(&doc).1, orig.1);
    doc.transform_map(&gd, Transform::FlipX);
    doc.transform_map(&gd, Transform::FlipY);
    doc.transform_map(&gd, Transform::FlipY);
    assert_eq!(key(&doc), orig, "double flips");
    let r = SlabRect {
        x0: 2,
        y0: 2,
        x1: 9,
        y1: 9,
    };
    let p = doc.transform_rect(&gd, r, Transform::Rot90).unwrap();
    for _ in 0..3 {
        doc.transform_rect(&gd, r, Transform::Rot90).unwrap();
    }
    assert_eq!(key(&doc), orig, "rect quarter turns");
    assert!(doc
        .transform_rect(
            &gd,
            SlabRect {
                x0: 0,
                y0: 0,
                x1: 3,
                y1: 1
            },
            Transform::Rot90
        )
        .is_err());
    let _ = p;
    // resize: grow left / top, then shrink back
    let p = doc.resize(&gd, 3, 2, 1, 0, 0, 5).unwrap();
    assert_eq!((doc.dims.w, doc.dims.h), (27, 19));
    assert_eq!(
        doc.kinds[doc.dims.slab_index(4 + 3, 5 + 2)],
        slab(&gd, "TREASURY_AREA")
    );
    doc.resize(&gd, -3, -2, -1, 0, 0, 5).unwrap();
    assert_eq!(key(&doc), orig, "resize there and back");
    let _ = p;
    // symmetry
    let s = Symmetry::new(SymmetryMode::EastWest);
    assert_eq!(
        s.slab_edits(doc.dims, 0, 3, 11, 0),
        vec![(0, 3, 11, 0), (22, 3, 11, 0)]
    );
}

/// Tiny deterministic PRNG for the property test.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

#[test]
fn random_edit_sequences_revert_to_original() {
    let Some((_, gd)) = base_gd() else { return };
    let dec = Decorator::new(&gd, 3);
    let kinds_pool: Vec<u16> = [
        "HARD",
        "DIRT",
        "PATH",
        "PRETTY_PATH",
        "LAVA",
        "WATER",
        "TREASURY_AREA",
        "BOOK_SHELVES",
        "DRAPE_WALL",
        "DOOR_WOODEN",
        "DOOR_STEEL2",
        "BRIDGE_FRAME",
        "GOLD",
        "HEART_PEDESTAL",
    ]
    .iter()
    .map(|n| slab(&gd, n))
    .collect();
    for seed in 1..=12u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut doc = test_map(&gd, 16 + rng.below(6) as u16, 14 + rng.below(6) as u16);
        let orig = DocState::of(&doc);
        let mut journal: Vec<Patch> = vec![];
        for _ in 0..60 {
            let (w, h) = (doc.dims.w as u64, doc.dims.h as u64);
            let (x, y) = (rng.below(w) as usize, rng.below(h) as usize);
            let p = match rng.below(14) {
                0 | 1 => {
                    let k = kinds_pool[rng.below(kinds_pool.len() as u64) as usize];
                    doc.place_slab_with_things(&gd, x, y, k, rng.below(6) as u8)
                        .unwrap()
                }
                2 => doc
                    .set_slab_owner_with_things(&gd, x, y, rng.below(9) as u8)
                    .unwrap(),
                3 => {
                    let c = [
                        class::CREATURE,
                        class::OBJECT,
                        class::TRAP,
                        class::EFFECT_GEN,
                    ][rng.below(4) as usize];
                    let t = things::new_thing(
                        c,
                        1 + rng.below(5) as i64,
                        rng.below(6) as u8,
                        StlCoord::new(3 * x as i32 + 1, rng.below(256) as i32),
                        StlCoord::new(3 * y as i32, 0),
                        StlCoord::new(1, 0),
                    );
                    doc.add_thing(&gd, t).1
                }
                4 if !doc.things.items.is_empty() => {
                    doc.remove_thing(&gd, rng.below(doc.things.items.len() as u64) as usize)
                }
                5 if !doc.things.items.is_empty() => {
                    let i = rng.below(doc.things.items.len() as u64) as usize;
                    doc.move_thing(
                        &gd,
                        i,
                        things::centred(3 * x as i32),
                        things::centred(3 * y as i32 + 2),
                        None,
                    )
                }
                6 => {
                    doc.add_action_point(
                        things::centred(3 * x as i32),
                        things::centred(3 * y as i32),
                        StlCoord::new(1, 0),
                        None,
                    )
                    .1
                }
                7 => {
                    doc.add_light(
                        things::centred(3 * x as i32),
                        things::centred(3 * y as i32),
                        StlCoord::new(2, 0),
                        StlCoord::new(4, 0),
                        20,
                    )
                    .1
                }
                8 if !doc.lights.items.is_empty() => {
                    doc.remove_light(rng.below(doc.lights.items.len() as u64) as usize)
                }
                9 if !doc.action_points.items.is_empty() => doc.renumber_action_points(),
                10 => {
                    doc.place_door(
                        &gd,
                        x,
                        y,
                        1 + rng.below(4) as i64,
                        rng.below(4) as u8,
                        rng.below(2) == 0,
                    )
                    .unwrap()
                    .1
                }
                11 => {
                    let t = [
                        Transform::FlipX,
                        Transform::FlipY,
                        Transform::Rot90,
                        Transform::Rot180,
                        Transform::Rot270,
                    ][rng.below(5) as usize];
                    if rng.below(2) == 0 {
                        doc.transform_map(&gd, t)
                    } else {
                        let s = rng.below(5) as i32;
                        let r = SlabRect {
                            x0: x as i32,
                            y0: y as i32,
                            x1: x as i32 + s,
                            y1: y as i32 + s,
                        };
                        if r.x1 < w as i32 && r.y1 < h as i32 {
                            doc.transform_rect(&gd, r, t).unwrap()
                        } else {
                            Patch::new()
                        }
                    }
                }
                12 => {
                    let e = |r: &mut Rng| r.below(5) as i32 - 2;
                    let (l, t, r, b) = (e(&mut rng), e(&mut rng), e(&mut rng), e(&mut rng));
                    doc.resize(&gd, l, t, r, b, 2, 5).unwrap_or_default()
                }
                _ => {
                    let r = SlabRect::single(x as i32, y as i32).grow(1);
                    doc.record(|d| d.decorate(&dec, r)).1
                }
            };
            journal.push(p);
        }
        let end = DocState::of(&doc);
        for p in journal.iter().rev() {
            p.revert(&mut doc);
        }
        assert_eq!(DocState::of(&doc), orig, "seed {seed}: undo all");
        for p in &journal {
            p.apply(&mut doc);
        }
        assert_eq!(DocState::of(&doc), end, "seed {seed}: redo all");
        let size: usize = journal.iter().map(|p| p.size_hint()).sum();
        println!(
            "seed {seed}: {} patches, ~{} KiB",
            journal.len(),
            size / 1024
        );
    }
}

#[test]
fn validate_shipped_and_broken_maps() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let mut codes: BTreeMap<(Severity, &str), usize> = BTreeMap::new();
    let mut maps = 0;
    let mut solid: BTreeMap<String, usize> = BTreeMap::new();
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
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            let Some(num) = n
                .strip_prefix("map")
                .and_then(|s| s.strip_suffix(".slb"))
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let slb = rnc::load_file(&e.path()).unwrap();
            let Some(dims) = MapDims::from_slb_len(slb.len()) else {
                continue;
            };
            let Ok(doc) = MapDoc::load(&levels, num, dims, &gd) else {
                continue;
            };
            maps += 1;
            for i in validate(&doc, &gd) {
                if i.code == "thing-in-solid" {
                    let w: Vec<&str> = i.message.split(' ').collect();
                    *solid.entry(format!("{} {}", w[1], w[3])).or_default() += 1;
                }
                *codes.entry((i.severity, i.code)).or_default() += 1;
            }
        }
    }
    println!("validate over {maps} shipped maps:");
    for ((s, c), n) in &codes {
        println!("  {s:?} {c}: {n}");
    }
    println!("  thing-in-solid by class/model: {solid:?}");
    // a broken map
    let Some((_, gd)) = base_gd() else { return };
    let mut doc = test_map(&gd, 12, 12);
    doc.place_slab(&gd, 6, 6, slab(&gd, "DOOR_WOODEN"), 0)
        .unwrap();
    doc.things.items.push(things::new_thing(
        class::CREATURE,
        999,
        12,
        things::centred(1),
        things::centred(1),
        StlCoord::new(1, 0),
    ));
    for _ in 0..2 {
        doc.action_points
            .items
            .push(kfx_formats::level_fx::FxActionPoint {
                number: Some(3),
                x: Some(things::centred(20)),
                y: Some(things::centred(20)),
                ..Default::default()
            });
    }
    let issues = validate(&doc, &gd);
    for i in &issues {
        println!(
            "  broken map: {:?} {} {:?}: {}",
            i.severity, i.code, i.at, i.message
        );
    }
    let has = |c: &str| issues.iter().any(|i| i.code == c);
    for c in [
        "door-slab-without-door",
        "thing-model-unknown",
        "thing-owner-range",
        "thing-in-solid",
        "action-point-duplicate",
        "keeper-without-heart",
    ] {
        assert!(has(c), "{c} not reported");
    }
}

/// Slab-attached items (torches, slab lights, ...) as a sorted, comparable list.
fn attached_items(doc: &MapDoc) -> Vec<String> {
    let mut v: Vec<String> = doc
        .things
        .items
        .iter()
        .filter(|t| t.parent_tile.is_some())
        .map(|t| {
            format!(
                "T {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
                t.class, t.subtype, t.owner, t.x, t.y, t.z, t.parent_tile
            )
        })
        .chain(
            doc.lights
                .items
                .iter()
                .filter(|l| l.parent_tile.is_some())
                .map(|l| {
                    format!(
                        "L {:?} {:?} {:?} {:?} {:?} {:?}",
                        l.x, l.y, l.z, l.range, l.intensity, l.parent_tile
                    )
                }),
        )
        .collect();
    v.sort();
    v
}

/// `place_door` over a wall slab removes the wall's torch and refreshes the torches of the
/// walls around: it leaves exactly what `place_slab_with_things` of the door
/// kind leaves (the game removes the attached objects of a re-placed slab, src/map_blocks.c:590).
#[test]
fn place_door_refreshes_slab_objects_like_place_slab() {
    let Some((_, gd)) = base_gd() else { return };
    let wall_kinds = [
        slab(&gd, "TORCH_WALL"),
        slab(&gd, "DRAPE_WALL"),
        slab(&gd, "TWINS_WALL"),
        slab(&gd, "TORCH_WALL"),
    ];
    let mut base = test_map(&gd, 20, 20);
    for y in 3..17usize {
        base.place_slab(&gd, 9, y, wall_kinds[y % 4], 0).unwrap();
        base.place_slab(&gd, 9, y, wall_kinds[y % 4], 0).unwrap();
    }
    let r = SlabRect {
        x0: 0,
        y0: 0,
        x1: 19,
        y1: 19,
    };
    base.apply_slab_objects(&gd, r);
    let torches_before = attached_items(&base);
    assert!(
        !torches_before.is_empty(),
        "the test wall must carry slab objects"
    );
    let door_kind = slab(&gd, "DOOR_WOODEN2");
    let mut changed_anything = false;
    for y in [4usize, 5, 6, 9, 12] {
        let mut a = base.clone();
        let (i, p) = a.place_door(&gd, 9, y, 1, 0, false).unwrap();
        assert_eq!(
            things::thing_class(&a.things.items[i]),
            Some(class::DOOR),
            "returned index is the door"
        );
        let mut b = base.clone();
        b.place_slab_with_things(&gd, 9, y, door_kind, 0).unwrap();
        assert_eq!(
            attached_items(&a),
            attached_items(&b),
            "door at y={y}: slab objects differ from place_slab_with_things"
        );
        // nothing stale is left on the door's own slab
        let slab_no = kfx_map::autotile::slab_number(a.dims, 9, y as i32) as i64;
        assert!(
            !a.things
                .items
                .iter()
                .any(|t| t.parent_tile == Some(slab_no) && thing_class_is_object(t)),
            "door at y={y}: slab objects of the replaced wall remain"
        );
        // re-applying the objects of the neighbourhood changes nothing: the patch left them current
        let mut c = a.clone();
        c.apply_slab_objects(&gd, SlabRect::single(9, y as i32).grow(1));
        assert_eq!(
            attached_items(&a),
            attached_items(&c),
            "door at y={y}: objects not up to date"
        );
        changed_anything |= attached_items(&a) != torches_before;
        // undo restores everything
        let mut u = a.clone();
        p.revert(&mut u);
        assert_eq!(DocState::of(&u), DocState::of(&base), "door at y={y}: undo");
    }
    assert!(changed_anything);
}

fn thing_class_is_object(t: &kfx_formats::level_fx::FxThing) -> bool {
    things::thing_class(t) == Some(class::OBJECT)
}
