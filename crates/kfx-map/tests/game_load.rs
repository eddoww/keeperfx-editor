//! Loads maps written by [`MapDoc::save`] in the real game and compares the game's log with
//! the log of the original map. Needs `KFX_GAME_TEST=1` and the test install (skipped
//! otherwise). See `support/game.rs` for how the game is run. The rig is unix only.
#![cfg(unix)]
mod support;

use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::{rnc, testutil};
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns};
use std::path::{Path, PathBuf};
use support::game::{Dump, GameRun, Rig};

const SECS: u64 = 25;

fn enabled() -> Option<PathBuf> {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: game load tests skipped");
        return None;
    }
    testutil::install_dir()
}

fn dims_of(dir: &Path, n: u32) -> MapDims {
    let slb = rnc::load_file(&level_bin::find_file(dir, n, "slb").unwrap()).unwrap();
    MapDims::from_slb_len(slb.len()).unwrap()
}

/// Game data as the game sees a level of the personal pack: fxdata + `map%05d.*` overrides.
fn pack_gd(install: &Path, rig: &Rig, n: u32) -> GameData {
    GameData::load(&ConfigLayers::for_level(
        install,
        None,
        Some(&rig.pack_dir()),
        None,
        Some(n),
    ))
    .unwrap()
}

fn report(tag: &str, run: &GameRun) {
    println!("[{tag}] {}", run.summary());
    for l in run.problems() {
        println!("[{tag}]   {}", l.raw);
    }
}

fn check_vs(tag: &str, run: &GameRun, base: &GameRun) -> Vec<String> {
    report(tag, run);
    assert!(run.started, "{tag}: level did not start");
    assert!(!run.crashed, "{tag}: game crashed");
    assert!(!run.exited_early, "{tag}: game exited before the timeout");
    let new = run.new_problems_vs(base);
    for l in &new {
        println!("[{tag}] NEW vs original: {l}");
    }
    new
}

/// What the game holds after loading (Lua dump at turn 2) versus the document. Returns
/// mismatch descriptions; prints a summary.
/// Thing counts are compared with the `base` run's dump when given (the game itself creates
/// things at load, e.g. script creatures), else with the document.
fn compare_dump(
    tag: &str,
    doc: &MapDoc,
    gd: &GameData,
    run: &GameRun,
    base: Option<&GameRun>,
) -> Vec<String> {
    let Some(d) = run.dump() else {
        return vec![format!(
            "{tag}: no KFXDUMP in the log (Lua dump did not run)"
        )];
    };
    let mut out = vec![];
    if !d.complete {
        out.push(format!("{tag}: Lua dump incomplete"));
    }
    let dims = doc.dims;
    let view = doc.view(gd);
    let (mut kind_bad, mut own_bad) = (0, 0);
    for y in 0..dims.h as i32 {
        for x in 0..dims.w as i32 {
            let i = dims.slab_index(x as usize, y as usize);
            let Some((gk, go)) = d.slabs.get(i) else {
                continue;
            };
            let k = kfx_map::autotile::SlabView::kind(&view, x, y).unwrap_or(0) as usize;
            let name = gd.slab_name(k).unwrap_or("?");
            if !name.eq_ignore_ascii_case(gk) {
                kind_bad += 1;
                if kind_bad <= 10 {
                    out.push(format!("{tag}: slab ({x},{y}) game {gk} doc {name}"));
                }
            }
            if *go != doc.owners[i] as i64 {
                own_bad += 1;
                if own_bad <= 10 {
                    out.push(format!(
                        "{tag}: slab ({x},{y}) owner game {go} doc {}",
                        doc.owners[i]
                    ));
                }
            }
        }
    }
    let mut h_bad = 0;
    let mut h_total = 0;
    for (y, row) in d.heights.iter().enumerate() {
        for (x, &g) in row.iter().enumerate() {
            let c = &doc.columns[dims.stl_index(x, y)];
            let ours = if c.solidmask == 0 {
                0
            } else {
                c.cubes.iter().position(|&q| q == 0).unwrap_or(8) as u8
            };
            h_total += 1;
            if g != ours {
                h_bad += 1;
                if h_bad <= 10 {
                    out.push(format!(
                        "{tag}: subtile ({x},{y}) floor height game {g} doc {ours}"
                    ));
                }
            }
        }
    }
    let class_of = |c: &str| match c {
        "Object" => 1,
        "EffectGen" => 7,
        "Door" => 9,
        "Trap" => 8,
        "Creature" => 5,
        _ => -1,
    };
    let mut counts = String::new();
    let bd = base.and_then(|b| b.dump());
    for (c, n) in &d.counts {
        let id = class_of(c);
        let ours = doc
            .things
            .items
            .iter()
            .filter(|t| t.class.as_ref().and_then(|k| k.id()) == Some(id))
            .count();
        let theirs = bd
            .as_ref()
            .and_then(|b| b.counts.iter().find(|(k, _)| k == c).map(|x| x.1));
        counts += &format!(
            " {c} game {n} doc {ours}{};",
            theirs.map_or(String::new(), |t| format!(" original-run {t}"))
        );
        if let Some(t) = theirs {
            if *n != t {
                out.push(format!(
                    "{tag}: {c} count game {n}, original map in the game {t}"
                ));
            }
        }
    }
    if let Some(b) = &bd {
        // positions of objects / effect generators / doors / traps (creatures move)
        let key = |t: &(String, String, i64, i64, i64)| (t.0.clone(), t.1.clone(), t.2, t.3, t.4);
        let mut pool: Vec<_> = b
            .things
            .iter()
            .filter(|t| t.0 != "Creature")
            .map(key)
            .collect();
        let mut extra = 0;
        for t in d.things.iter().filter(|t| t.0 != "Creature") {
            match pool.iter().position(|p| *p == key(t)) {
                Some(i) => {
                    pool.swap_remove(i);
                }
                None => {
                    extra += 1;
                    if extra <= 10 {
                        out.push(format!("{tag}: thing {t:?} not in the original run"));
                    }
                }
            }
        }
        for t in pool.iter().take(10) {
            out.push(format!("{tag}: thing {t:?} of the original run missing"));
        }
        counts += &format!(
            " non-creature things differing from the original run: +{extra} -{};",
            pool.len()
        );
    }
    println!(
        "[{tag}] dump: {} slabs ({kind_bad} kind and {own_bad} owner mismatches), {h_total} \
            subtile floor heights ({h_bad} mismatches);{counts}",
        d.slabs.len()
    );
    let _: &Dump = &d;
    out
}

/// a + b: shipped legacy maps re-saved with current and regenerated columns.
#[test]
fn resaved_shipped_maps_load() {
    let Some(install) = enabled() else { return };
    let rig = Rig::new(&install, "resave");
    let camp = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    let mut failures = vec![];
    for src_level in [1u32, 10] {
        let dims = dims_of(&camp, src_level);
        let script = std::fs::read(level_bin::find_file(&camp, src_level, "txt").unwrap()).unwrap();
        let script = String::from_utf8_lossy(&script).to_string();
        let n = 1;
        rig.install_level(n, &camp, src_level, dims.w, dims.h, None);
        rig.install_dump(n, dims.w, dims.h);
        let base = rig.run(n, SECS);
        report(&format!("keeporig {src_level} original"), &base);
        assert!(base.started && !base.crashed, "original map did not run");
        let gd = pack_gd(&install, &rig, n);
        let doc = MapDoc::load(&camp, src_level, dims, &gd).unwrap();
        for m in compare_dump(
            &format!("keeporig {src_level} original"),
            &doc,
            &gd,
            &base,
            None,
        ) {
            println!("  {m}");
        }
        for which in [SaveColumns::Current, SaveColumns::Regenerate] {
            let stage = rig.dir.join(format!("stage-{src_level}-{which:?}"));
            let _ = std::fs::remove_dir_all(&stage);
            doc.save(&gd, &stage, which).unwrap();
            rig.install_level(n, &stage, src_level, dims.w, dims.h, Some(&script));
            rig.install_dump(n, dims.w, dims.h);
            let run = rig.run(n, SECS);
            let tag = format!("keeporig {src_level} {which:?}");
            let new = check_vs(&tag, &run, &base);
            let saved = MapDoc::load(&stage, src_level, dims, &gd).unwrap();
            failures.extend(compare_dump(&tag, &saved, &gd, &run, Some(&base)));
            failures.extend(new.into_iter().map(|l| format!("{tag}: {l}")));
        }
    }
    assert!(failures.is_empty(), "new log problems: {failures:#?}");
}

const NEW_SCRIPT: &str = "LEVEL_VERSION(1)\nSET_GENERATE_SPEED(500)\nSTART_MONEY(PLAYER0,20000)\n\
    START_MONEY(PLAYER1,20000)\nCOMPUTER_PLAYER(PLAYER1,0)\nMAX_CREATURES(PLAYER0,30)\n\
    MAX_CREATURES(PLAYER1,30)\nADD_CREATURE_TO_POOL(FLY,10)\n\
    CREATURE_AVAILABLE(ALL_PLAYERS,FLY,1,0)\nROOM_AVAILABLE(ALL_PLAYERS,TREASURE,1,1)\n\
    ROOM_AVAILABLE(ALL_PLAYERS,LAIR,1,1)\nROOM_AVAILABLE(ALL_PLAYERS,GARDEN,1,1)\n\
    MAGIC_AVAILABLE(ALL_PLAYERS,POWER_IMP,1,1)\n";

fn kind(gd: &GameData, name: &str) -> u16 {
    gd.slab_id(name).unwrap_or_else(|| panic!("slab {name}")) as u16
}

/// A dug, claimed area with every room kind, liquids with bridges, door slabs and walls of
/// several owners, built with `place_slab` (+ doors with `place_slab_with_things`), then
/// `apply_slab_objects` over the area.
fn build_area(doc: &mut MapDoc, gd: &GameData, ox: usize, oy: usize, heart_owners: &[u8]) {
    use kfx_map::autotile::SlabRect;
    let (w, h) = (30usize, 30usize);
    for y in oy..oy + h {
        for x in ox..ox + w {
            let edge = x == ox || y == oy || x == ox + w - 1 || y == oy + h - 1;
            let (k, o) = if edge {
                // walls of three owners, one per side group
                let o = if y == oy {
                    0
                } else if x == ox {
                    1
                } else if y == oy + h - 1 {
                    2
                } else {
                    3
                };
                (
                    kind(
                        gd,
                        [
                            "DRAPE_WALL",
                            "TORCH_WALL",
                            "TWINS_WALL",
                            "WOMAN_WALL",
                            "PAIR_WALL",
                        ][(x + y) % 5],
                    ),
                    o,
                )
            } else {
                (kind(gd, "PRETTY_PATH"), 0)
            };
            doc.place_slab(gd, x, y, k, o).unwrap();
        }
    }
    let rooms = [
        "TREASURY_AREA",
        "BOOK_SHELVES",
        "LAIR_AREA",
        "HATCHERY",
        "TEMPLE_POOL",
        "GRAVE_AREA",
        "PRISON_AREA",
        "TORTURE_AREA",
        "WORKSHOP_AREA",
        "TRAINING_AREA",
        "GUARD_AREA",
        "BARRACK_AREA",
    ];
    for (i, r) in rooms.iter().enumerate() {
        let (bx, by) = (ox + 2 + (i % 4) * 7, oy + 2 + (i / 4) * 6);
        let owner = if i % 3 == 2 { 1 } else { 0 };
        for y in by..by + 4 {
            for x in bx..bx + 5 {
                doc.place_slab(gd, x, y, kind(gd, r), owner).unwrap();
            }
        }
    }
    // hearts
    for (n, &o) in heart_owners.iter().enumerate() {
        let (bx, by) = (ox + 2 + n * 8, oy + 20);
        for y in by..by + 3 {
            for x in bx..bx + 3 {
                doc.place_slab(gd, x, y, kind(gd, "HEART_PEDESTAL"), o)
                    .unwrap();
            }
        }
    }
    // lava and water strips with bridges
    for (row, liquid) in [(oy + 25, "LAVA"), (oy + 27, "WATER")] {
        for x in ox + 1..ox + w - 1 {
            doc.place_slab(gd, x, row, kind(gd, liquid), 5).unwrap();
        }
        for x in ox + 10..ox + 14 {
            doc.place_slab(gd, x, row, kind(gd, "BRIDGE_FRAME"), 0)
                .unwrap();
            let i = doc.dims.slab_index(x, row);
            doc.wlb[i] = if liquid == "LAVA" { 1 } else { 2 };
        }
    }
    // doors in the top wall (walls east and west) and the left wall (walls north and south)
    let all = SlabRect {
        x0: ox as i32 - 1,
        y0: oy as i32 - 1,
        x1: (ox + w) as i32,
        y1: (oy + h) as i32,
    };
    doc.regenerate_rect(gd, all.clip(doc.dims));
    for (i, door) in ["DOOR_WOODEN", "DOOR_BRACE", "DOOR_STEEL", "DOOR_MAGIC"]
        .iter()
        .enumerate()
    {
        doc.place_slab_with_things(gd, ox + 4 + 5 * i, oy, kind(gd, door), 0)
            .unwrap();
        doc.place_slab_with_things(gd, ox, oy + 4 + 5 * i, kind(gd, door), 1)
            .unwrap();
    }
    // lock one door
    if let Some(d) = doc.door_on_slab(ox as i32 + 4, oy as i32) {
        let mut t = doc.things.items[d].clone();
        t.door_locked = Some(1);
        doc.update_thing(gd, d, t);
    }
    let (rm, add) = doc.apply_slab_objects(gd, all.clip(doc.dims));
    println!("apply_slab_objects over the edited area: removed {rm}, added {add}");
}

fn run_doc(
    rig: &Rig,
    gd: &GameData,
    doc: &MapDoc,
    which: SaveColumns,
    tag: &str,
    base: &GameRun,
) -> Vec<String> {
    let stage = rig.dir.join(format!("stage-{tag}").replace(' ', "_"));
    let _ = std::fs::remove_dir_all(&stage);
    doc.save(gd, &stage, which).unwrap();
    let n = 2;
    rig.install_level(
        n,
        &stage,
        doc.level,
        doc.dims.w,
        doc.dims.h,
        Some(NEW_SCRIPT),
    );
    rig.install_dump(n, doc.dims.w, doc.dims.h);
    let run = rig.run(n, SECS);
    let mut out: Vec<String> = check_vs(tag, &run, base)
        .into_iter()
        .map(|l| format!("{tag}: {l}"))
        .collect();
    let saved = MapDoc::load(&stage, doc.level, doc.dims, gd).unwrap();
    out.extend(compare_dump(tag, &saved, gd, &run, None));
    if let Some(d) = run.dump() {
        let mut by: std::collections::BTreeMap<(String, String), usize> = Default::default();
        for t in &d.things {
            *by.entry((t.0.clone(), t.1.clone())).or_default() += 1;
        }
        println!("[{tag}] things in the game: {by:?}");
    }
    out
}

/// c: slab edits on a shipped map; d: new 60x40 and 170x170 maps.
#[test]
fn edited_and_new_maps_load() {
    let Some(install) = enabled() else { return };
    let rig = Rig::new(&install, "edits");
    let camp = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    let dims = dims_of(&camp, 1);
    rig.install_level(1, &camp, 1, dims.w, dims.h, None);
    let base = rig.run(1, SECS);
    report("baseline keeporig 1", &base);
    let gd = pack_gd(&install, &rig, 2);
    let only = std::env::var("KFX_GAME_ONLY").ok();
    let want = |t: &str| only.as_deref().is_none_or(|o| t.contains(o));
    let mut failures = vec![];
    if want("edited") {
        let mut doc = MapDoc::load(&camp, 1, dims, &gd).unwrap();
        doc.level = 2;
        build_area(&mut doc, &gd, 50, 50, &[1]);
        failures.extend(run_doc(
            &rig,
            &gd,
            &doc,
            SaveColumns::Current,
            "c edited keeporig 1",
            &base,
        ));
    }
    for (w, h) in [(60u16, 40u16), (170, 170)] {
        let tag = format!("d new {w}x{h}");
        if !want(&tag) {
            continue;
        }
        let mut doc = MapDoc::new(&rig.pack_dir(), 2, MapDims::new(w, h), 0, &gd);
        for y in 1..h as usize - 1 {
            for x in 1..w as usize - 1 {
                doc.place_slab(&gd, x, y, kind(&gd, "DIRT"), 5).unwrap();
            }
        }
        build_area(&mut doc, &gd, 3, 3, &[0, 1]);
        if w == h {
            // decorated columns
            let dec = kfx_map::decorate::Decorator::new(
                &gd,
                kfx_map::decorate::Decorator::seed_for_level(2),
            );
            doc.regenerate_columns_decorated(&gd, &dec);
        } else {
            // a quarter turn: 60x40 becomes 40x60
            doc.transform_map(&gd, kfx_map::transform::Transform::Rot90);
        }
        for i in kfx_map::validate::validate(&doc, &gd) {
            println!(
                "[{tag}] validate: {:?} {} {:?} {}",
                i.severity, i.code, i.at, i.message
            );
        }
        let tag = format!("{tag}{}", if w == h { " decorated" } else { " rotated" });
        failures.extend(run_doc(&rig, &gd, &doc, SaveColumns::Current, &tag, &base));
    }
    assert!(failures.is_empty(), "problems: {failures:#?}");
}

fn thing_is_slab_attached(t: &kfx_formats::level_fx::FxThing) -> bool {
    t.parent_tile.is_some()
}

/// Doors placed with `MapDoc::place_door` (task: slab objects around a door are refreshed):
/// the game must hold exactly the objects / effect generators / doors the document holds.
#[test]
fn placed_doors_match_game_objects() {
    let Some(install) = enabled() else { return };
    let rig = Rig::new(&install, "doors");
    let camp = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    let dims = dims_of(&camp, 1);
    rig.install_level(1, &camp, 1, dims.w, dims.h, None);
    let base = rig.run(1, SECS);
    assert!(base.started && !base.crashed, "original map did not run");
    let gd = pack_gd(&install, &rig, 2);
    let mut doc = MapDoc::new(&rig.pack_dir(), 2, MapDims::new(40, 30), 0, &gd);
    for y in 1..29usize {
        for x in 1..39usize {
            doc.place_slab(&gd, x, y, kind(&gd, "DIRT"), 5).unwrap();
        }
    }
    for y in 3..27usize {
        for x in 3..37usize {
            doc.place_slab(&gd, x, y, kind(&gd, "PRETTY_PATH"), 0)
                .unwrap();
        }
    }
    // a heart so the keeper exists, and two wall lines (vertical x=12, horizontal y=14)
    for y in 5..8usize {
        for x in 5..8usize {
            doc.place_slab(&gd, x, y, kind(&gd, "HEART_PEDESTAL"), 0)
                .unwrap();
        }
    }
    let walls = [
        "TORCH_WALL",
        "DRAPE_WALL",
        "TWINS_WALL",
        "WOMAN_WALL",
        "PAIR_WALL",
    ];
    for y in 3..27usize {
        doc.place_slab(&gd, 12, y, kind(&gd, walls[y % 5]), 0)
            .unwrap();
    }
    for x in 13..37usize {
        doc.place_slab(&gd, x, 14, kind(&gd, walls[x % 5]), 0)
            .unwrap();
    }
    doc.apply_slab_objects(
        &gd,
        kfx_map::autotile::SlabRect {
            x0: 0,
            y0: 0,
            x1: 39,
            y1: 29,
        },
    );
    let mut n = 0;
    for (i, (x, y)) in [
        (12usize, 6usize),
        (12, 9),
        (12, 20),
        (16, 14),
        (22, 14),
        (30, 14),
        (12, 24),
        (12, 25),
    ]
    .into_iter()
    .enumerate()
    {
        let model = 1 + (i % 4) as i64;
        doc.place_door(&gd, x, y, model, 0, i % 3 == 0).unwrap();
        n += 1;
    }
    println!(
        "placed {n} doors; things in the document: {}",
        doc.things.items.len()
    );
    for i in kfx_map::validate::validate(&doc, &gd) {
        println!(
            "[doors] validate: {:?} {} {:?} {}",
            i.severity, i.code, i.at, i.message
        );
    }
    let stage = rig.dir.join("stage-doors");
    let _ = std::fs::remove_dir_all(&stage);
    doc.save(&gd, &stage, SaveColumns::Current).unwrap();
    rig.install_level(2, &stage, 2, 40, 30, Some(NEW_SCRIPT));
    rig.install_dump(2, 40, 30);
    let run = rig.run(2, SECS);
    report("doors", &run);
    assert!(run.started && !run.crashed && !run.exited_early);
    let d = run.dump().expect("dump");
    assert!(d.complete);
    // document side: (class, owner, stl x, stl y) of every non-creature thing; model names via the
    // game's dump
    let class_name = |c: i64| match c {
        1 => "Object",
        7 => "EffectGen",
        9 => "Door",
        8 => "Trap",
        _ => "",
    };
    // doors the validator says cannot stand (and their keys) are destroyed by the game when it runs
    let doomed: Vec<(i32, i32)> = kfx_map::validate::validate(&doc, &gd)
        .iter()
        .filter(|i| i.code == "door-cannot-stand")
        .filter_map(|i| match i.at {
            kfx_map::validate::Location::Item(kfx_map::things::ItemRef::Thing(n)) => {
                let t = &doc.things.items[n];
                kfx_map::things::position(t.x, t.y).map(|(x, y)| (x.div_euclid(3), y.div_euclid(3)))
            }
            _ => None,
        })
        .collect();
    println!("[doors] validator: door-cannot-stand on slabs {doomed:?}");
    assert_eq!(
        doomed.len(),
        2,
        "the adjacent pair (12,24) (12,25) must be flagged, nothing else"
    );
    let mut ours: Vec<(String, i64, i64, i64)> = doc
        .things
        .items
        .iter()
        .filter(|t| {
            kfx_map::things::position(t.x, t.y).is_none_or(|(x, y)| {
                !doomed.contains(&(x.div_euclid(3), y.div_euclid(3))) || thing_is_slab_attached(t)
            })
        })
        .filter_map(|t| {
            let c = class_name(t.class.as_ref()?.id()?);
            let (x, y) = kfx_map::things::position(t.x, t.y)?;
            (!c.is_empty()).then(|| (c.to_string(), t.owner.unwrap_or(-1), x as i64, y as i64))
        })
        .collect();
    let mut theirs: Vec<(String, i64, i64, i64)> = d
        .things
        .iter()
        .filter(|t| t.0 != "Creature")
        .map(|t| (t.0.clone(), t.2, t.3, t.4))
        .collect();
    ours.sort();
    theirs.sort();
    let (mut only_ours, mut only_game) = (vec![], vec![]);
    let mut pool = theirs.clone();
    for o in &ours {
        match pool.iter().position(|p| p == o) {
            Some(i) => {
                pool.swap_remove(i);
            }
            None => only_ours.push(o.clone()),
        }
    }
    only_game.extend(pool);
    let by_model = |things: &[(String, String, i64, i64, i64)]| {
        let mut m: std::collections::BTreeMap<(String, String), usize> = Default::default();
        for t in things {
            *m.entry((t.0.clone(), t.1.clone())).or_default() += 1;
        }
        m
    };
    println!("[doors] game objects by model: {:?}", by_model(&d.things));
    println!(
        "[doors] document non-creature things {} / game {}; only in the document: \
        {only_ours:?}; only in the game: {only_game:?}",
        ours.len(),
        theirs.len()
    );
    for l in run.new_problems_vs(&base) {
        println!("[doors] NEW vs original: {l}");
    }
    assert!(
        only_ours.is_empty() && only_game.is_empty(),
        "document and game disagree on the objects"
    );
}
