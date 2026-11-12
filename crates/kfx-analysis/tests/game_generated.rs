//! A generated 4-keeper map saved with `MapDoc::save` loads in the real game with no new log
//! problems and with every keeper's heart. Needs `KFX_GAME_TEST=1` and the test install; uses the
//! offscreen game rig of `kfx-map` (one game at a time, hard timeout, the process is always
//! killed). Unix only.
#![cfg(unix)]
#[path = "../../kfx-map/tests/support/mod.rs"]
mod support;

use kfx_analysis::generate::{generate, GenParams, StarterScript, Style, SymmetryChoice};
use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::{rnc, testutil};
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns};
use support::game::Rig;

const SECS: u64 = 25;

/// A DK script from the generator's starter values (what a host would write).
fn dk_script(s: &StarterScript) -> String {
    let mut out = format!(
        "LEVEL_VERSION(1)\nSET_GENERATE_SPEED({})\n",
        s.generate_speed
    );
    for p in &s.players {
        out += &format!(
            "START_MONEY({},{})\nMAX_CREATURES({},{})\n",
            p.script_name, p.start_money, p.script_name, p.max_creatures
        );
        if let Some(c) = p.computer {
            out += &format!("COMPUTER_PLAYER({},{c})\n", p.script_name);
        }
    }
    for (c, n) in &s.creature_pool {
        out += &format!("ADD_CREATURE_TO_POOL({c},{n})\nCREATURE_AVAILABLE(ALL_PLAYERS,{c},1,0)\n");
    }
    for (r, now) in &s.rooms {
        out += &format!("ROOM_AVAILABLE(ALL_PLAYERS,{r},1,{})\n", *now as u8);
    }
    for (p, now) in &s.powers {
        out += &format!("MAGIC_AVAILABLE(ALL_PLAYERS,{p},1,{})\n", *now as u8);
    }
    out
}

#[test]
fn generated_four_keeper_map_loads_in_the_game() {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: skipped");
        return;
    }
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let rig = Rig::new(&install, "kfx-analysis-gen");
    // baseline: an original map, for the log lines every run has
    let camp = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    let slb = rnc::load_file(&level_bin::find_file(&camp, 1, "slb").unwrap()).unwrap();
    let d1 = MapDims::from_slb_len(slb.len()).unwrap();
    rig.install_level(1, &camp, 1, d1.w, d1.h, None);
    let base = rig.run(1, SECS);
    println!("[baseline] {}", base.summary());
    assert!(base.started && !base.crashed);

    let gd = GameData::load(&ConfigLayers::for_level(
        &install,
        None,
        Some(&rig.pack_dir()),
        None,
        Some(2),
    ))
    .unwrap();
    let stage = rig.dir.join("stage-gen");
    let p = GenParams {
        seed: 4242,
        width: 85,
        height: 85,
        players: 4,
        style: Style::Fortress,
        symmetry: SymmetryChoice::Auto,
        folder: stage.clone(),
        level: 2,
        ..Default::default()
    };
    let g = generate(&gd, &p).unwrap_or_else(|e| panic!("{e}"));
    println!(
        "generated: {:?} attempts {} imbalance {:?}; {} things",
        g.report.symmetry,
        g.report.attempts,
        g.report.imbalance,
        g.doc.things.items.len()
    );
    let _ = std::fs::remove_dir_all(&stage);
    g.doc.save(&gd, &stage, SaveColumns::Current).unwrap();
    let script = dk_script(&g.script);
    rig.install_level(2, &stage, 2, 85, 85, Some(&script));
    rig.install_dump(2, 85, 85);
    let run = rig.run(2, SECS);
    println!("[generated] {}", run.summary());
    for l in run.problems() {
        println!("[generated]   {}", l.raw);
    }
    let new = run.new_problems_vs(&base);
    for l in &new {
        println!("[generated] NEW vs original: {l}");
    }
    let dump = run.dump();
    let mut hearts = vec![];
    if let Some(d) = &dump {
        for t in &d.things {
            if t.0 == "Object" && t.1.eq_ignore_ascii_case("SOUL_CONTAINER") {
                hearts.push(t.2);
            }
        }
        let creatures = d.things.iter().filter(|t| t.0 == "Creature").count();
        println!(
            "[generated] dump complete {}: hearts of players {hearts:?}; {creatures} \
            creatures; counts {:?}",
            d.complete, d.counts
        );
        // the slab layer the game holds is the document's
        let saved = MapDoc::load(&stage, 2, MapDims::new(85, 85), &gd).unwrap();
        let mut bad = 0;
        for (i, (k, o)) in d.slabs.iter().enumerate() {
            let name = gd.slab_name(saved.kinds[i] as usize).unwrap_or("?");
            if !name.eq_ignore_ascii_case(k) || *o != saved.owners[i] as i64 {
                bad += 1;
            }
        }
        println!(
            "[generated] slab kind/owner mismatches game vs document: {bad} of {}",
            d.slabs.len()
        );
        assert_eq!(bad, 0);
    }
    let _ = std::fs::remove_dir_all(&rig.dir);
    assert!(run.started, "level did not start");
    assert!(!run.crashed, "game crashed");
    assert!(!run.exited_early, "game exited early");
    assert!(new.is_empty(), "new log problems: {new:#?}");
    hearts.sort();
    // dump owners are player ids (PLAYER0..3 = 0..3)
    assert_eq!(
        hearts,
        vec![0, 1, 2, 3],
        "every keeper has its heart in the game"
    );
}
