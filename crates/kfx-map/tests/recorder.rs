//! `MapDoc::record_tracked` against the snapshot-diff `MapDoc::record`: identical patches and
//! documents over random operation sequences, plus a benchmark on a 170x170 map.
use kfx_formats::level_bin::{Column, MapDims};
use kfx_formats::level_fx::StlCoord;
use kfx_formats::testutil;
use kfx_map::autotile::{kinds, SlabRect};
use kfx_map::decorate::Decorator;
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::MapDoc;
use kfx_map::patch::DocState;
use kfx_map::sidecar::{CustomSlab, Note};
use kfx_map::things::{self, class};
use kfx_map::transform::Transform;
use std::path::Path;
use std::time::Instant;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n.max(1)
    }
}

fn gd() -> Option<GameData> {
    let root = testutil::install_dir()?;
    Some(GameData::load(&ConfigLayers::for_level(&root, None, None, None, None)).unwrap())
}

fn dug_map(gd: &GameData, w: u16, h: u16) -> MapDoc {
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
fn tracked_equals_snapshot_over_random_sequences() {
    let Some(gd) = gd() else { return };
    let names = [
        "DIRT",
        "PRETTY_PATH",
        "DRAPE_WALL",
        "TORCH_WALL",
        "WATER",
        "LAVA",
        "HARD",
        "DOOR_WOODEN",
        "DOOR_STEEL",
        "TREASURY_AREA",
        "HEART_PEDESTAL",
        "LAIR_AREA",
        "BRIDGE_FRAME",
    ];
    let kind_ids: Vec<u16> = names
        .iter()
        .map(|n| gd.slab_id(n).unwrap_or_else(|| panic!("{n}")) as u16)
        .collect();
    let dec = Decorator::new(&gd, 7);
    let hg = things::hero_gate_model(&gd).unwrap();
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for seed in 0..6u64 {
        let mut rng = Rng(seed + 1);
        let mut doc = dug_map(&gd, 24, 20);
        // a custom slab to place
        let mut cols = [Column::default(); 9];
        for (i, c) in cols.iter_mut().enumerate() {
            c.cubes[0] = 1 + i as u16;
            c.floor_texture = 3;
        }
        let id = doc.edit_sidecar(|s| {
            s.add_custom_slab(CustomSlab::new("test", kinds::CLAIMED as u16, cols))
        });
        for _step in 0..160 {
            let (w, h) = (doc.dims.w as usize, doc.dims.h as usize);
            let (x, y) = (rng.below(w), rng.below(h));
            let kind = kind_ids[rng.below(kind_ids.len())];
            let owner = rng.below(6) as u8;
            let ntl = doc.things.items.len();
            let n = rng.below(40);
            let (a, b, c) = (rng.below(w * 3), rng.below(h * 3), rng.below(1000));
            let before = DocState::of(&doc);
            let op = n;
            let name = match op {
                0..=9 => {
                    doc.record_checked(|d| {
                        let _ = d.place_slab_with_things(&gd, x, y, kind, owner);
                    });
                    "place_slab_with_things"
                }
                10..=11 => {
                    doc.record_checked(|d| {
                        let _ = d.place_slab(&gd, x, y, kind, owner);
                    });
                    "place_slab"
                }
                12 => {
                    doc.record_checked(|d| {
                        let _ = d.set_slab_owner_with_things(&gd, x, y, owner);
                    });
                    "set_slab_owner_with_things"
                }
                13..=14 => {
                    doc.record_checked(|d| {
                        let _ = d.place_door(&gd, x, y, 1 + (c % 4) as i64, owner, c % 2 == 0);
                    });
                    "place_door"
                }
                15..=17 => {
                    doc.record_checked(|d| {
                        let m = 1 + (c % 5) as i64;
                        let t = things::new_thing(
                            class::CREATURE,
                            m,
                            owner,
                            things::centred(a as i32),
                            things::centred(b as i32),
                            StlCoord::new(1, 0),
                        );
                        d.add_thing(&gd, t);
                    });
                    "add_thing"
                }
                18..=19 if ntl > 0 => {
                    doc.record_checked(|d| {
                        d.remove_thing(&gd, c % ntl);
                    });
                    "remove_thing"
                }
                20 if ntl > 0 => {
                    doc.record_checked(|d| {
                        d.move_thing(
                            &gd,
                            c % ntl,
                            things::centred(a as i32),
                            things::centred(b as i32),
                            None,
                        );
                    });
                    "move_thing"
                }
                21 if ntl > 0 => {
                    doc.record_checked(|d| {
                        let mut t = d.things.items[c % ntl].clone();
                        t.owner = Some(owner as i64);
                        d.update_thing(&gd, c % ntl, t);
                    });
                    "update_thing"
                }
                22 => {
                    doc.record_checked(|d| {
                        d.add_action_point(
                            things::centred(a as i32),
                            things::centred(b as i32),
                            StlCoord::new(3, 0),
                            None,
                        );
                    });
                    "add_action_point"
                }
                23 => {
                    doc.record_checked(|d| {
                        d.remove_action_point(c % 4);
                        d.renumber_action_points();
                    });
                    "remove+renumber action points"
                }
                24 => {
                    doc.record_checked(|d| {
                        d.add_light(
                            things::centred(a as i32),
                            things::centred(b as i32),
                            StlCoord::new(2, 0),
                            StlCoord::new(5, 0),
                            40,
                        );
                    });
                    "add_light"
                }
                25 => {
                    doc.record_checked(|d| {
                        d.remove_light(c % 6);
                    });
                    "remove_light"
                }
                26 => {
                    doc.record_checked(|d| {
                        let t = things::new_thing(
                            class::OBJECT,
                            hg,
                            4,
                            things::centred(a as i32),
                            things::centred(b as i32),
                            StlCoord::new(1, 0),
                        );
                        d.add_thing(&gd, t);
                        d.renumber_hero_gates(&gd);
                    });
                    "hero gates"
                }
                27..=28 => {
                    doc.record_checked(|d| {
                        let r = SlabRect {
                            x0: x as i32 - 2,
                            y0: y as i32 - 2,
                            x1: x as i32 + 2,
                            y1: y as i32 + 1,
                        };
                        d.regenerate_rect_decorated(&gd, &dec, r);
                    });
                    "regenerate_rect_decorated"
                }
                29 => {
                    doc.record_checked(|d| {
                        d.decorate(
                            &dec,
                            SlabRect {
                                x0: 0,
                                y0: 0,
                                x1: 8,
                                y1: 8,
                            },
                        );
                    });
                    "decorate"
                }
                30 => {
                    doc.record_checked(|d| {
                        d.apply_slab_objects(&gd, SlabRect::single(x as i32, y as i32).grow(2));
                    });
                    "apply_slab_objects"
                }
                31 => {
                    doc.record_checked(|d| {
                        let _ = d.set_wlb(x, y, (c % 4) as u8);
                        let _ = d.set_slx(x, y, (c % 3) as u8);
                        d.set_texture_id((c % 5) as u8);
                    });
                    "wlb/slx/texture"
                }
                32..=33 => {
                    doc.record_checked(|d| {
                        let _ = d.place_custom_slab(&gd, x, y, id, owner);
                    });
                    "place_custom_slab"
                }
                34 => {
                    doc.record_checked(|d| {
                        d.edit_sidecar(|s| {
                            s.notes.push(Note {
                                x: x as i32,
                                y: y as i32,
                                text: format!("n{c}"),
                                color: None,
                            })
                        });
                    });
                    "edit_sidecar"
                }
                35 => {
                    doc.record_checked(|d| {
                        let r = SlabRect {
                            x0: 2,
                            y0: 2,
                            x1: 9,
                            y1: 9,
                        };
                        let _ = d.transform_rect(
                            &gd,
                            r,
                            [Transform::FlipX, Transform::Rot90, Transform::Rot180][c % 3],
                        );
                    });
                    "transform_rect"
                }
                36 if w < 36 && h < 36 => {
                    doc.record_checked(|d| {
                        let _ = d.resize(
                            &gd,
                            (c % 3) as i32 - 1,
                            1,
                            1,
                            (c % 2) as i32 - 1,
                            kinds::EARTH as u16,
                            5,
                        );
                    });
                    "resize"
                }
                37 => {
                    doc.record_checked(|d| {
                        d.transform_map(&gd, Transform::Rot90);
                    });
                    "transform_map"
                }
                _ => {
                    doc.record_checked(|d| {
                        let r = SlabRect {
                            x0: 0,
                            y0: 0,
                            x1: 4,
                            y1: 4,
                        };
                        d.regenerate_rect(&gd, r);
                    });
                    "regenerate_rect"
                }
            };
            *counts.entry(name).or_default() += 1;
            // the tracked patch (checked equal to the snapshot one inside record_checked) undoes
            let _ = before;
        }
    }
    println!(
        "operations run through record_checked (tracked == snapshot patch and document): \
        {counts:?}"
    );
    println!("total {}", counts.values().sum::<usize>());
}

/// The patch from `record_tracked` reverts and re-applies exactly.
#[test]
fn tracked_patches_revert() {
    let Some(gd) = gd() else { return };
    let mut doc = dug_map(&gd, 20, 20);
    let start = DocState::of(&doc);
    let mut patches = vec![];
    for i in 0..12usize {
        let (_, p) = doc.record_tracked(|d| {
            let _ = d.place_slab_with_things(
                &gd,
                4 + i % 5,
                5 + i / 5,
                gd.slab_id("TORCH_WALL").unwrap() as u16,
                0,
            );
        });
        patches.push(p);
    }
    let end = DocState::of(&doc);
    for p in patches.iter().rev() {
        p.revert(&mut doc);
    }
    assert_eq!(DocState::of(&doc), start);
    for p in &patches {
        p.apply(&mut doc);
    }
    assert_eq!(DocState::of(&doc), end);
}

#[test]
fn recorder_benchmark_170() {
    let Some(gd) = gd() else { return };
    let t = Instant::now();
    let mut doc = dug_map(&gd, 170, 170);
    println!("[bench] built the 170x170 map in {:?}", t.elapsed());
    let hg = things::hero_gate_model(&gd).unwrap();
    // some things so the lists have a realistic size
    for i in 0..3000usize {
        let t = things::new_thing(
            class::CREATURE,
            1 + (i % 10) as i64,
            0,
            things::centred(10 + (i % 400) as i32),
            things::centred(10 + (i / 400) as i32 * 3),
            StlCoord::new(1, 0),
        );
        doc.things.items.push(t);
    }
    let _ = hg;
    let walls = [
        gd.slab_id("TORCH_WALL").unwrap() as u16,
        gd.slab_id("PRETTY_PATH").unwrap() as u16,
    ];
    let n = 150;
    let mut results = vec![];
    for (label, tracked) in [
        ("snapshot (record)", false),
        ("tracked (record_tracked)", true),
    ] {
        let mut d = doc.clone();
        let t = Instant::now();
        for i in 0..n {
            let (x, y) = (20 + i % 100, 20 + (i * 7) % 100);
            let k = walls[i % 2];
            if tracked {
                d.record_tracked(|d| {
                    let _ = d.place_slab_with_things(&gd, x, y, k, 0);
                });
            } else {
                d.record(|d| {
                    let _ = d.place_slab_with_things(&gd, x, y, k, 0);
                });
            }
        }
        let per_slab = t.elapsed() / n as u32;
        // move_thing + decorate-like small column edits
        let t = Instant::now();
        for i in 0..n {
            let (x, y) = (20 + i % 100, 20 + (i * 7) % 100);
            let f = |d: &mut MapDoc| {
                d.set_wlb(x, y, 0).ok();
                d.set_owner(&gd, x, y, 1).ok();
            };
            if tracked {
                d.record_tracked(f);
            } else {
                d.record(f);
            }
        }
        let per_owner = t.elapsed() / n as u32;
        let t = Instant::now();
        for i in 0..n {
            let (x, y) = (20 + i % 100, 20 + (i * 7) % 100);
            let f = |d: &mut MapDoc| {
                let t = things::new_thing(
                    class::CREATURE,
                    1,
                    0,
                    things::centred(3 * x as i32),
                    things::centred(3 * y as i32),
                    StlCoord::new(1, 0),
                );
                d.add_thing(&gd, t);
            };
            if tracked {
                d.record_tracked(f);
            } else {
                d.record(f);
            }
        }
        let per_thing = t.elapsed() / n as u32;
        println!(
            "[bench] 170x170, {} things, {label}: place_slab_with_things {per_slab:?}/edit, \
            set_wlb+set_owner {per_owner:?}/edit, add_thing {per_thing:?}/edit",
            doc.things.items.len()
        );
        results.push((per_slab, per_owner));
    }
    assert!(
        results[1].1 < results[0].1,
        "tracked recording of a slab-only edit must be cheaper than a full clone"
    );
}
