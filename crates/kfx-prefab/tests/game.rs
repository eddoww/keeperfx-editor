//! Every starter prefab applied to one map, loaded in the real game through the offscreen
//! harness of `kfx-map` (`KFX_GAME_TEST=1`; one game at a time, also across test processes; hard
//! timeout). The game's own log is compared with the log of a plain map with the same script.
#![cfg(unix)]
mod common;
#[path = "../../kfx-map/tests/support/game.rs"]
mod game;

use game::Rig;
use kfx_formats::level_bin::MapDims;
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns};
use kfx_prefab::*;

const SECS: u64 = 30;

const SCRIPT: &str = "LEVEL_VERSION(1)\nSET_GENERATE_SPEED(500)\n\
    START_MONEY(PLAYER0,20000)\nMAX_CREATURES(PLAYER0,30)\n\
    ADD_CREATURE_TO_POOL(FLY,10)\nCREATURE_AVAILABLE(ALL_PLAYERS,FLY,1,0)\n\
    ROOM_AVAILABLE(ALL_PLAYERS,TREASURE,1,1)\nROOM_AVAILABLE(ALL_PLAYERS,LAIR,1,1)\n\
    ROOM_AVAILABLE(ALL_PLAYERS,GARDEN,1,1)\n\
    MAGIC_AVAILABLE(ALL_PLAYERS,POWER_IMP,1,1)\n";

#[test]
fn all_starter_prefabs_load_in_the_game() {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: game test skipped");
        return;
    }
    let Some(install) = kfx_formats::testutil::install_dir() else {
        return;
    };
    let rig = Rig::new(&install, "prefab-starters");
    let n = 3u32;
    let gd = GameData::load(&ConfigLayers::for_level(
        &install,
        None,
        Some(&rig.pack_dir()),
        None,
        Some(n),
    ))
    .unwrap();
    let starters = starter::build_all(&gd).unwrap();

    // ---- baseline: the same map without prefabs except the player's heart
    let (w, h) = (130u16, 110u16);
    let mut base = common::blank(&gd, w, h);
    base.level = n;
    base.folder = rig.pack_dir();
    let heart = &starters
        .iter()
        .find(|(id, _)| id == "start-compact")
        .unwrap()
        .1;
    apply(&mut base, &gd, heart, (3, 3), &ApplyOptions::default()).unwrap();
    let stage = rig.dir.join("stage-base");
    base.save(&gd, &stage, SaveColumns::Current).unwrap();
    rig.install_level(n, &stage, n, w, h, Some(SCRIPT));
    rig.install_dump(n, w, h);
    let base_run = rig.run(n, SECS);
    println!("[baseline] {}", base_run.summary());
    for l in base_run.problems() {
        println!("[baseline]   {}", l.raw);
    }
    assert!(
        base_run.started && !base_run.crashed && !base_run.exited_early,
        "baseline did not run"
    );

    // ---- all starter prefabs on one map, in a grid
    let mut doc = MapDoc::new(&rig.pack_dir(), n, MapDims::new(w, h), 0, &gd);
    let dirt = gd.slab_id("DIRT").unwrap() as u16;
    for y in 1..h as usize - 1 {
        for x in 1..w as usize - 1 {
            doc.place_slab(&gd, x, y, dirt, 5).unwrap();
        }
    }
    let mut script = SCRIPT.to_string();
    let (cols, cw, ch) = (5usize, 24i32, 17i32);
    let mut summary = vec![];
    for (i, (id, p)) in starters.iter().enumerate() {
        let at = (3 + (i % cols) as i32 * cw, 3 + (i / cols) as i32 * ch);
        // three starts for three keepers, everything else belongs to player 0
        let first = match id.as_str() {
            "start-gateway" => 1,
            "start-fortress" => 2,
            _ => 0,
        };
        let o = ApplyOptions {
            owners: OwnerMap::with_first(&[first]),
            ..Default::default()
        };
        let (_, rep) = apply(&mut doc, &gd, p, at, &o).unwrap();
        assert!(!rep.has_missing(), "{id}: {}", rep.summary());
        if let Some(s) = &rep.script_dk {
            script = script::append_fragment(&script, s, &p.meta.name, script::ScriptKind::Dk);
        }
        summary.push(format!("{id:<22} at {at:?}: {}", rep.summary()));
    }
    for s in &summary {
        println!("{s}");
    }
    let issues: Vec<_> = kfx_map::validate::validate(&doc, &gd)
        .into_iter()
        .filter(|i| i.severity >= kfx_map::validate::Severity::Warning)
        .collect();
    println!("validate: {} warnings/errors", issues.len());
    for i in &issues {
        println!("  {:?} {} {}", i.severity, i.code, i.message);
    }
    assert!(
        issues
            .iter()
            .all(|i| i.severity != kfx_map::validate::Severity::Error),
        "{issues:?}"
    );
    println!(
        "combined map: {} things, {} action points, {} lights",
        doc.things.items.len(),
        doc.action_points.items.len(),
        doc.lights.items.len()
    );
    let stage = rig.dir.join("stage-combined");
    doc.save(&gd, &stage, SaveColumns::Current).unwrap();
    rig.install_level(n, &stage, n, w, h, Some(&script));
    rig.install_dump(n, w, h);
    let run = rig.run(n, SECS);
    println!("[combined] {}", run.summary());
    for l in run.problems() {
        println!("[combined]   {}", l.raw);
    }
    assert!(
        run.started && !run.crashed && !run.exited_early,
        "combined map: {}",
        run.summary()
    );
    let new = run.new_problems_vs(&base_run);
    for l in &new {
        println!("[combined] NEW vs baseline: {l}");
    }
    assert!(new.is_empty(), "new log problems: {new:#?}");

    // ---- what the game holds equals the document
    let d = run.dump().expect("Lua dump");
    assert!(d.complete);
    let view = doc.view(&gd);
    let (mut bad_kind, mut bad_owner) = (0, 0);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = doc.dims.slab_index(x as usize, y as usize);
            let (gk, go) = &d.slabs[i];
            let k = kfx_map::autotile::SlabView::kind(&view, x, y).unwrap_or(0) as usize;
            if !gd.slab_name(k).unwrap_or("?").eq_ignore_ascii_case(gk) {
                bad_kind += 1;
            }
            if *go != doc.owners[i] as i64 {
                bad_owner += 1;
            }
        }
    }
    let count = |c: &str| {
        d.counts
            .iter()
            .find(|(k, _)| k == c)
            .map(|x| x.1)
            .unwrap_or(0)
    };
    let doc_count = |cls: i64| {
        doc.things
            .items
            .iter()
            .filter(|t| t.class.as_ref().and_then(|k| k.id()) == Some(cls))
            .count()
    };
    println!(
        "game dump: {}x{} slabs ({bad_kind} kind and {bad_owner} \
            owner mismatches); objects game {} doc {}; doors game {} doc {}; traps game {} \
            doc {}; creatures game {} doc {}; effect generators game {} doc {}",
        w,
        h,
        count("Object"),
        doc_count(1),
        count("Door"),
        doc_count(9),
        count("Trap"),
        doc_count(8),
        count("Creature"),
        doc_count(5),
        count("EffectGen"),
        doc_count(7)
    );
    assert_eq!((bad_kind, bad_owner), (0, 0));
    // things the game holds that the file does not list, by class and model name (the game
    // creates some itself, e.g. what rooms make when the level starts): the baseline map has
    // the same kind of extras, so they are compared with it
    let names = names::Names::new(&gd);
    let extras = |dump: &game::Dump, doc: &MapDoc| {
        let mut m: std::collections::BTreeMap<(String, String), i64> = Default::default();
        for t in &dump.things {
            *m.entry((t.0.clone(), t.1.to_ascii_uppercase()))
                .or_default() += 1;
        }
        for t in &doc.things.items {
            let Some(c) = t
                .class
                .as_ref()
                .and_then(|c| c.id())
                .and_then(|c| model::ThingClass::from_id(c as u8))
            else {
                continue;
            };
            let model = names
                .model_name(c, t.subtype.unwrap_or(-1))
                .to_ascii_uppercase();
            *m.entry((c.name().to_string(), model)).or_default() -= 1;
        }
        m.retain(|_, v| *v != 0);
        m
    };
    let base_dump = base_run.dump().expect("baseline dump");
    let (eb, ec) = (extras(&base_dump, &base), extras(&d, &doc));
    println!("game minus document, baseline map: {eb:?}");
    println!("game minus document, combined map: {ec:?}");
    assert_eq!(
        count("Door"),
        doc_count(9),
        "every door survived loading (door_can_stand)"
    );
    assert_eq!(count("Trap"), doc_count(8));
    assert!(
        count("Creature") >= doc_count(5),
        "creatures of the document exist in the game"
    );
    // every difference is something the game makes from the map itself; show which
    let only_game: Vec<_> = ec
        .iter()
        .filter(|(k, v)| **v > 0 && eb.get(*k).copied().unwrap_or(0) != **v)
        .collect();
    let only_doc: Vec<_> = ec.iter().filter(|(_, v)| **v < 0).collect();
    println!(
        "extra in the game beyond the baseline's: {only_game:?}; \
            listed in the file but missing in the game: {only_doc:?}"
    );
    assert!(
        only_doc.is_empty(),
        "things of the file the game did not create: {only_doc:?}"
    );
    let _ = std::fs::remove_dir_all(&rig.dir);
}
