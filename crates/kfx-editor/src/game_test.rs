//! The real game loads what the editor saves: a shipped map edited with the editor's own edit
//! paths (slab strokes with decoration, things / doors / action points / lights through the
//! journal, undo / redo in between), and optionally a map saved by the GUI (`--actions`), in
//! `KFX_EDITOR_GAME_MAP` (a `mapNNNNN.slb`). Needs `KFX_GAME_TEST=1` and the test install;
//! uses the game harness of `kfx-map` (`crates/kfx-map/tests/support/game.rs`: offscreen SDL,
//! hard timeout, one game at a time).
// the game harness shared with the kfx-map tests; these tests use a subset of it
#[path = "../../kfx-map/tests/support/game.rs"]
#[allow(dead_code)]
mod game;

use crate::document::{self, Document, Edit};
use game::{GameRun, Rig};
use kfx_formats::level_fx::StlCoord;
use kfx_formats::testutil;
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::MapDoc;
use kfx_map::things::{self, class};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

const SECS: u64 = 25;
const SCRIPT: &str = "LEVEL_VERSION(1)\nSET_GENERATE_SPEED(500)\nSTART_MONEY(PLAYER0,20000)\n\
    START_MONEY(PLAYER1,20000)\nCOMPUTER_PLAYER(PLAYER1,0)\nMAX_CREATURES(PLAYER0,30)\n\
    MAX_CREATURES(PLAYER1,30)\nROOM_AVAILABLE(ALL_PLAYERS,TREASURE,1,1)\n\
    ROOM_AVAILABLE(ALL_PLAYERS,LAIR,1,1)\nMAGIC_AVAILABLE(ALL_PLAYERS,POWER_IMP,1,1)\n";

fn class_name(c: u8) -> &'static str {
    match c {
        class::OBJECT => "Object",
        class::CREATURE => "Creature",
        class::EFFECT_GEN => "EffectGen",
        class::TRAP => "Trap",
        class::DOOR => "Door",
        _ => "?",
    }
}

/// Compare the document's things with the game's dump: every non-creature thing of the
/// document must be in the game at the same subtile with the same owner; creatures by class,
/// model name and owner (they may have walked). Returns (problems, game-only extras).
fn compare(tag: &str, doc: &MapDoc, gd: &GameData, run: &GameRun) -> (Vec<String>, Vec<String>) {
    let Some(d) = run.dump() else {
        return (
            vec![format!("{tag}: no KFXDUMP (Lua dump did not run)")],
            vec![],
        );
    };
    let mut problems = vec![];
    if !d.complete {
        problems.push(format!("{tag}: dump incomplete"));
    }
    let name = |t: &kfx_formats::level_fx::FxThing| {
        let m = t.subtype.unwrap_or(-1);
        let find = |l: Vec<(usize, String)>| {
            l.into_iter()
                .find(|(i, _)| *i as i64 == m)
                .map(|x| x.1)
                .unwrap_or_default()
        };
        match things::thing_class(t) {
            Some(class::CREATURE) => gd
                .creatures
                .get((m - 1).max(0) as usize)
                .cloned()
                .unwrap_or_default(),
            Some(class::OBJECT) => find(gd.object_names()),
            Some(class::TRAP) => find(gd.trap_names()),
            Some(class::DOOR) => find(gd.door_names()),
            Some(class::EFFECT_GEN) => find(gd.effect_generator_names()),
            _ => String::new(),
        }
    };
    let mut pool: Vec<(String, String, i64, i64, i64)> = d.things.clone();
    let mut missing = 0;
    for t in &doc.things.items {
        let Some(c) = things::thing_class(t) else {
            continue;
        };
        let (Some(x), Some(y)) = (t.x, t.y) else {
            continue;
        };
        let key_class = class_name(c).to_string();
        let model = name(t);
        let owner = t.owner.unwrap_or(-1);
        let pos = if c == class::CREATURE {
            pool.iter()
                .position(|g| g.0 == key_class && g.1.eq_ignore_ascii_case(&model) && g.2 == owner)
        } else {
            pool.iter().position(|g| {
                g.0 == key_class
                    && g.1.eq_ignore_ascii_case(&model)
                    && g.2 == owner
                    && g.3 == x.stl as i64
                    && g.4 == y.stl as i64
            })
        };
        // gold in a treasury is merged into hoards by the game when the level starts
        let gold = c == class::OBJECT
            && t.subtype.is_some_and(|m| {
                ["TREASURE_HOARD", "VALUABLE"]
                    .iter()
                    .any(|g| things::object_genre_is(gd, m, g))
                    || gd.object_genre(m as usize).is_some_and(|_| {
                        model.eq_ignore_ascii_case("GOLD") || model.eq_ignore_ascii_case("GOLDL")
                    })
            });
        match pos {
            Some(i) => {
                pool.swap_remove(i);
            }
            None if gold => println!(
                "[{tag}] {model} at ({},{}) not in the game as such (gold is \
                merged into treasury hoards)",
                x.stl, y.stl
            ),
            None => {
                missing += 1;
                if missing <= 15 {
                    problems.push(format!(
                        "{tag}: {key_class} {model} owner {owner} at ({},{}) \
                        not in the game",
                        x.stl, y.stl
                    ));
                }
            }
        }
    }
    if missing > 15 {
        problems.push(format!("{tag}: ... {} more missing", missing - 15));
    }
    let mut extra: BTreeMap<(String, String), usize> = BTreeMap::new();
    for g in &pool {
        *extra.entry((g.0.clone(), g.1.clone())).or_default() += 1;
    }
    let extras = extra
        .into_iter()
        .map(|((c, m), n)| format!("{c} {m} x{n}"))
        .collect();
    println!(
        "[{tag}] document things {}, game things {}, matched {}",
        doc.things.items.len(),
        d.things.len(),
        d.things.len() - pool.len()
    );
    (problems, extras)
}

fn report(tag: &str, run: &GameRun) {
    println!("[{tag}] {}", run.summary());
    for l in run.problems() {
        println!("[{tag}]   {}", l.raw);
    }
}

fn run_saved(
    rig: &Rig,
    n: u32,
    folder: &Path,
    level: u32,
    doc: &MapDoc,
    gd: &GameData,
    base: &GameRun,
    tag: &str,
) -> Vec<String> {
    rig.install_level(n, folder, level, doc.dims.w, doc.dims.h, Some(SCRIPT));
    rig.install_dump(n, doc.dims.w, doc.dims.h);
    let run = rig.run(n, SECS);
    report(tag, &run);
    let mut out = vec![];
    if !run.started {
        out.push(format!("{tag}: level did not start"));
    }
    if run.crashed || run.exited_early {
        out.push(format!("{tag}: game crashed / exited early"));
    }
    for l in run.new_problems_vs(base) {
        out.push(format!("{tag}: NEW log line vs the unedited map: {l}"));
    }
    let (p, extras) = compare(tag, doc, gd, &run);
    println!("[{tag}] things the game added itself: {extras:?}");
    out.extend(p);
    out
}

#[test]
fn game_loads_editor_output() {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: skipped");
        return;
    }
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let rig = Rig::new(&install, "editor-things");
    let camp = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    let loc = document::locate(&install, &camp, 15).unwrap();
    // the game plays a personal-pack level with the base configs + the map's own overrides
    let layers = ConfigLayers::for_level(&install, None, Some(&rig.pack_dir()), None, Some(2));
    let gd = Arc::new(GameData::load(&layers).unwrap());

    // baseline: the unedited map, saved untouched
    let base_dir = rig.dir.join("stage-base");
    let mut base_doc = Document::load(&loc, gd.clone()).unwrap();
    base_doc.save_to(&base_dir, 15).unwrap();
    rig.install_level(1, &base_dir, 15, loc.dims.w, loc.dims.h, Some(SCRIPT));
    let base = rig.run(1, SECS);
    report("baseline map15 resaved", &base);

    // edited: slabs (decorated), many things, doors, action points, lights, with undo / redo
    let mut d = Document::load(&loc, gd.clone()).unwrap();
    d.decorator.enabled = true;
    let claimed = gd.slab_id("PRETTY_PATH").unwrap() as u16;
    let lair = gd
        .room_id("LAIR")
        .map(|r| gd.rooms[r].assigned_slab as u16)
        .unwrap();
    d.begin_stroke("dig");
    let edits: Vec<(i32, i32, Edit)> = (8..20)
        .flat_map(|y| {
            (60..76).map(move |x| {
                (
                    x,
                    y,
                    Edit::Place {
                        kind: claimed,
                        owner: 0,
                    },
                )
            })
        })
        .collect();
    d.apply(&edits);
    let edits: Vec<(i32, i32, Edit)> = (10..14)
        .flat_map(|y| {
            (62..66).map(move |x| {
                (
                    x,
                    y,
                    Edit::Place {
                        kind: lair,
                        owner: 0,
                    },
                )
            })
        })
        .collect();
    d.apply(&edits);
    d.end_stroke();
    let names = [
        "IMP",
        "DRAGON",
        "BILE_DEMON",
        "TROLL",
        "DARK_MISTRESS",
        "SPIDER",
        "DRUID",
        "TIME_MAGE",
        "MAIDEN",
    ];
    let mut placed = 0;
    for (k, n) in names.iter().enumerate() {
        let Some(m) = gd.creature_id(n) else { continue };
        for j in 0..6 {
            let (sx, sy) = (3 * 66 + 1 + 3 * j, 3 * 15 + 1 + (k as i32 % 3));
            let mut t = things::new_thing(
                class::CREATURE,
                m as i64,
                0,
                things::centred(sx),
                things::centred(sy),
                d.doc.floor_z(sx, sy),
            );
            t.creature_level = Some(1 + (j as i64 * 2) % 10);
            t.orientation = Some((j as i64 * 256) % 2048);
            d.op("creature", |doc, g| doc.add_thing(g, t));
            placed += 1;
        }
    }
    for (k, o) in [
        "GOLD",
        "BARREL",
        "TEMPLE_STATUE",
        "FERN",
        "MUSHROOM_YELLOW_LUM",
        "CANDLESTCK",
    ]
    .iter()
    .enumerate()
    {
        let Some(m) = gd.object_id(o) else { continue };
        for j in 0..4 {
            let (sx, sy) = (3 * 61 + 1 + 3 * j, 3 * 17 + 1 - (k as i32 % 2));
            let t = things::new_thing(
                class::OBJECT,
                m as i64,
                0,
                StlCoord::new(sx, 64 * (k as i32 % 4)),
                things::centred(sy),
                d.doc.floor_z(sx, sy),
            );
            d.op("object", |doc, g| doc.add_thing(g, t));
            placed += 1;
        }
    }
    let trap = gd.trap_names().into_iter().find(|(i, _)| *i > 0).unwrap().0 as i64;
    for j in 0..4 {
        let (sx, sy) = (3 * 70 + 1 + 3 * j, 3 * 9 + 1);
        let t = things::new_thing(
            class::TRAP,
            trap,
            0,
            things::centred(sx),
            things::centred(sy),
            d.doc.floor_z(sx, sy),
        );
        d.op("trap", |doc, g| doc.add_thing(g, t));
    }
    // a corridor with a locked door through earth
    d.begin_stroke("corridor");
    let edits: Vec<(i32, i32, Edit)> = (76..82)
        .map(|x| {
            (
                x,
                12,
                Edit::Place {
                    kind: claimed,
                    owner: 0,
                },
            )
        })
        .collect();
    d.apply(&edits);
    d.end_stroke();
    let door = gd.door_names().into_iter().find(|(i, _)| *i > 0).unwrap().0 as i64;
    d.op("door", |doc, g| {
        doc.place_door(g, 79, 12, door, 0, true)
            .map(|(i, p)| (Some(i), p))
            .unwrap()
    });
    d.op("ap", |doc, _| {
        doc.add_action_point(
            things::centred(200),
            things::centred(40),
            StlCoord::new(3, 0),
            None,
        )
    });
    d.op("light", |doc, _| {
        doc.add_light(
            things::centred(195),
            things::centred(35),
            StlCoord::new(3, 0),
            StlCoord::new(6, 0),
            40,
        )
    });
    if let Some(m) = things::hero_gate_model(&gd) {
        let t = things::new_thing(
            class::OBJECT,
            m,
            4,
            things::centred(205),
            things::centred(50),
            d.doc.floor_z(205, 50),
        );
        d.op("gate", |doc, g| doc.add_thing(g, t));
    }
    // undo / redo in between
    d.undo();
    d.undo();
    d.redo();
    d.redo();
    println!(
        "edited map: {} things ({placed} placed), {} action points, {} lights, {} undo steps",
        d.doc.things.items.len(),
        d.doc.action_points.items.len(),
        d.doc.lights.items.len(),
        d.history().len()
    );
    for i in kfx_map::validate::validate(&d.doc, &gd)
        .iter()
        .filter(|i| i.severity != kfx_map::validate::Severity::Info)
    {
        println!("validate: {:?} {} {}", i.severity, i.code, i.message);
    }
    let out_dir = rig.dir.join("stage-edited");
    d.save_to(&out_dir, 15).unwrap();
    let saved = MapDoc::load(&out_dir, 15, d.dims(), &gd).unwrap();
    assert_eq!(saved.things.items.len(), d.doc.things.items.len());
    let mut failures = run_saved(&rig, 2, &out_dir, 15, &saved, &gd, &base, "edited map15");

    // a map saved by the GUI through --actions
    if let Ok(p) = std::env::var("KFX_EDITOR_GAME_MAP") {
        let p = std::path::PathBuf::from(p);
        let folder = p.parent().unwrap().to_path_buf();
        let level = document::map_number(&p).unwrap();
        let l = document::locate(&install, &folder, level).unwrap();
        let doc = MapDoc::load(&folder, level, l.dims, &gd).unwrap();
        failures.extend(run_saved(
            &rig,
            3,
            &folder,
            level,
            &doc,
            &gd,
            &base,
            "gui scene",
        ));
    }
    for f in &failures {
        println!("FAIL {f}");
    }
    assert!(failures.is_empty(), "{} problems", failures.len());
}

/// Play-testing through the editor's own command line (`playtest::build_launch`), run like the
/// harness does it: a private copy of the install, SDL offscreen video and dummy audio, a hard
/// timeout, the harness lock (one game at a time), the process always killed. A level whose DK
/// script is broken on purpose must show the script error in `keeperfx.log`, linked to its
/// script line by `playtest::parse_log`. The rig folder is kept for the Game log screenshot.
#[test]
fn play_launch_logs_script_error() {
    use crate::playtest;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: skipped");
        return;
    }
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let rig = Rig::new(&install, "editor-play");
    let src = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    let script = "LEVEL_VERSION(1)\nSTART_MONEY(PLAYER0,5000)\nFOO_BAR_COMMAND(PLAYER0,1)\n\
        MAX_CREATURES(PLAYER0,NOT_A_NUMBER)\n";
    rig.install_level(4, &src, 4, 85, 85, Some(script));
    let exe = rig.dir.join("keeperfx");
    let settings = crate::settings::PlaySettings {
        windowed: true,
        window_res: "640x480".into(),
        no_sound: true,
        ..Default::default()
    };
    let target = playtest::play_target(&rig.dir, &rig.pack_dir(), 4);
    let campaign = match &target {
        playtest::PlayTarget::Pack { campaign, .. } => campaign.clone(),
        t => panic!("not a pack: {t:?}"),
    };
    assert_eq!(campaign.to_ascii_lowercase(), "levels/personal");
    let l = playtest::build_launch(&settings, &exe, &campaign, 4, false);
    println!("launch: {}", l.display());
    // the harness lock: one game at a time
    let lock = std::path::Path::new("/tmp/kfx-map-gametest.lock");
    let t0 = Instant::now();
    while std::fs::create_dir(lock).is_err() {
        let pid = std::fs::read_to_string(lock.join("pid")).unwrap_or_default();
        if pid.trim().is_empty() || !std::path::Path::new("/proc").join(pid.trim()).exists() {
            let _ = std::fs::remove_dir_all(lock);
            continue;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(600),
            "another game holds the lock"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = std::fs::write(lock.join("pid"), std::process::id().to_string());
    let _ = std::fs::remove_file(rig.dir.join("keeperfx.log"));
    let mut child = Command::new(&l.program)
        .args(&l.args)
        .current_dir(&l.cwd)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("SDL_VIDEODRIVER", "offscreen")
        .env("SDL_AUDIODRIVER", "dummy")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn keeperfx");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if child.try_wait().ok().flatten().is_none() {
        let _ = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) && child.try_wait().ok().flatten().is_none() {
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
    }
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(lock);
    let text = playtest::read_log(&rig.dir).expect("keeperfx.log");
    assert!(text.contains("Started level 4 "), "the level did not start");
    let entries = playtest::parse_log(&text);
    let linked: Vec<_> = entries.iter().filter(|e| e.script.is_some()).collect();
    for e in &linked {
        println!("{:?} {:?}: {}", e.level, e.script, e.text);
    }
    assert!(
        linked
            .iter()
            .any(|e| e.script == Some((playtest::ScriptKind::Dk, 3))),
        "no error linked to line 3"
    );
    println!("rig kept at {}", rig.dir.display());
}

/// What the game makes of (1) a map saved with a custom slab, (2) a per-map rules
/// override written by the integrated configuration editor (its session over the level's layer
/// stack, `EditorFs`), (3) the legacy `.tng/.apt/.lgt` output. Each must start with no log line
/// the unedited map does not have.
#[test]
fn game_loads_custom_slab_rules_override_and_legacy_files() {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: skipped");
        return;
    }
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let rig = Rig::new(&install, "editor-custom-slab-config");
    let camp = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    let loc = document::locate(&install, &camp, 15).unwrap();
    let layers = ConfigLayers::for_level(&install, None, Some(&rig.pack_dir()), None, Some(2));
    let gd = Arc::new(GameData::load(&layers).unwrap());
    let base_dir = rig.dir.join("stage-base");
    let mut base_doc = Document::load(&loc, gd.clone()).unwrap();
    base_doc.save_to(&base_dir, 15).unwrap();
    rig.install_level(1, &base_dir, 15, loc.dims.w, loc.dims.h, Some(SCRIPT));
    let base = rig.run(1, SECS);
    report("baseline map15 resaved", &base);
    let mut failures: Vec<String> = vec![];
    let check = |failures: &mut Vec<String>, tag: &str, run: &GameRun| {
        report(tag, run);
        if !run.started {
            failures.push(format!("{tag}: level did not start"));
        }
        if run.crashed || run.exited_early {
            failures.push(format!("{tag}: game crashed / exited early"));
        }
        for l in run.new_problems_vs(&base) {
            failures.push(format!("{tag}: NEW log line vs the unedited map: {l}"));
        }
    };

    // (1) custom slabs: a gold slab's columns as a custom slab placed on a dug path
    let mut d = Document::load(&loc, gd.clone()).unwrap();
    d.verify_tracked = false;
    let gold = d
        .doc
        .kinds
        .iter()
        .position(|&k| k == kfx_map::autotile::kinds::GOLD as u16)
        .unwrap();
    let (gx, gy) = ((gold % 85) as i32, (gold / 85) as i32);
    let cols: [kfx_formats::level_bin::Column; 9] = std::array::from_fn(|k| {
        d.doc.columns[d
            .dims()
            .stl_index(3 * gx as usize + k % 3, 3 * gy as usize + k / 3)]
    });
    let id = d.doc.edit_sidecar(|s| {
        s.add_custom_slab(kfx_map::sidecar::CustomSlab::new(
            "gold seam",
            kfx_map::autotile::kinds::GOLD as u16,
            cols,
        ))
    });
    let claimed = gd.slab_id("PRETTY_PATH").unwrap() as u16;
    d.begin_stroke("dig");
    let edits: Vec<(i32, i32, Edit)> = (10..16)
        .flat_map(|y| {
            (62..70).map(move |x| {
                (
                    x,
                    y,
                    Edit::Place {
                        kind: claimed,
                        owner: 0,
                    },
                )
            })
        })
        .collect();
    d.apply(&edits);
    d.apply(&[
        (64, 12, Edit::Custom { id, owner: 5 }),
        (66, 13, Edit::Custom { id, owner: 5 }),
    ]);
    d.end_stroke();
    let dir = rig.dir.join("stage-custom");
    d.save_to(&dir, 15).unwrap();
    assert!(dir.join("map00015.kfxed.toml").is_file(), "sidecar written");
    let back = MapDoc::load(&dir, 15, d.dims(), &gd).unwrap();
    assert_eq!(
        back.custom_cells[d.dims().slab_index(64, 12)],
        id,
        "custom slab survives save + load"
    );
    rig.install_level(2, &dir, 15, loc.dims.w, loc.dims.h, Some(SCRIPT));
    let run = rig.run(2, SECS);
    check(&mut failures, "custom slab map", &run);

    // (2) a per-map rules override from the integrated config editor
    rig.install_level(3, &base_dir, 15, loc.dims.w, loc.dims.h, Some(SCRIPT));
    let lc = document::level_config(&rig.dir, &rig.pack_dir(), 3, &Default::default());
    let ui_layers = crate::app::config_ui_stack(&lc.stack, 3);
    let mut session = kfx_config_ui::ConfigSession::new(
        Arc::new(crate::app::EditorFsForTest::default()),
        ui_layers,
    );
    let map_layer = session
        .layers
        .index_of(&kfx_config_ui::LayerKind::Map(3))
        .expect("map layer");
    session.set_target(map_layer).unwrap();
    let gph = session
        .cfg_value("rules.cfg", "game", "GoldPerHoard")
        .unwrap();
    let new_gph = if gph.trim() == "2600" { "2400" } else { "2600" };
    session.set_cfg("rules.cfg", "game", "GoldPerHoard", new_gph);
    session.set_cfg("rules.cfg", "game", "PayDayGap", "9000");
    let saved = session.save().unwrap();
    println!("[rules override] written {saved:?} (GoldPerHoard {gph} -> {new_gph})");
    let text = std::fs::read_to_string(rig.pack_dir().join("map00003.rules.cfg")).unwrap();
    println!("[rules override] map00003.rules.cfg:\n{text}");
    assert!(text.contains("GoldPerHoard") && text.contains("PayDayGap"));
    let run = rig.run(3, SECS);
    check(&mut failures, "per-map rules override", &run);

    // (3) the legacy format
    let mut d = Document::load(&loc, gd.clone()).unwrap();
    d.verify_tracked = false;
    d.begin_stroke("dig");
    let edits: Vec<(i32, i32, Edit)> = (10..14)
        .flat_map(|y| {
            (62..68).map(move |x| {
                (
                    x,
                    y,
                    Edit::Place {
                        kind: claimed,
                        owner: 0,
                    },
                )
            })
        })
        .collect();
    d.apply(&edits);
    d.end_stroke();
    let rep = d.doc.legacy_report(&gd);
    println!(
        "[legacy] report: {} errors, warnings {:?}",
        rep.errors.len(),
        rep.warnings
    );
    let dir = rig.dir.join("stage-legacy");
    d.save_to_format(&dir, 15, true).unwrap();
    for ext in ["tng", "apt", "lgt"] {
        assert!(
            kfx_formats::level_bin::find_file(&dir, 15, ext).is_some(),
            "{ext} written"
        );
    }
    for ext in ["tngfx", "aptfx", "lgtfx"] {
        assert!(
            kfx_formats::level_bin::find_file(&dir, 15, ext).is_none(),
            "{ext} not written"
        );
    }
    // the legacy baseline: the shipped (legacy) map as it is
    rig.install_level(5, &camp, 15, loc.dims.w, loc.dims.h, Some(SCRIPT));
    let legacy_base = rig.run(5, SECS);
    report("baseline shipped legacy map15", &legacy_base);
    rig.install_level(4, &dir, 15, loc.dims.w, loc.dims.h, Some(SCRIPT));
    rig.install_dump(4, loc.dims.w, loc.dims.h);
    let run = rig.run(4, SECS);
    report("legacy format", &run);
    if !run.started || run.crashed || run.exited_early {
        failures.push("legacy format: did not start / crashed".into());
    }
    // compared with the shipped legacy map (the game warns about the missing FX files of any
    // legacy map; the level numbers differ, so compare without them)
    let strip = |l: &str| l.replace("map00004", "mapN").replace("map00005", "mapN");
    let shipped: Vec<String> = legacy_base
        .problems()
        .iter()
        .map(|l| strip(&l.raw))
        .collect();
    for l in run.problems() {
        if !shipped.contains(&strip(&l.raw)) {
            failures.push(format!(
                "legacy format: NEW log line vs the shipped legacy map: {}",
                l.raw
            ));
        }
    }
    let saved = MapDoc::load(&dir, 15, d.dims(), &gd).unwrap();
    let (p, _) = compare("legacy format", &saved, &gd, &run);
    failures.extend(p);
    for f in &failures {
        println!("FAIL {f}");
    }
    assert!(failures.is_empty(), "{} problems", failures.len());
}

/// A map with random doodads (objects on paths, effect generators on lava)
/// and a DK script whose setup block was written by the script wizard. The level must load
/// without new log problems, every doodad must exist in the game, and the wizard's values must
/// be in effect (PLAYER0's gold and creature limit read back through the Lua API).
#[test]
fn game_loads_doodads_and_script_wizard_block() {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: skipped");
        return;
    }
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let rig = Rig::new(&install, "editor-doodads-wizard");
    let camp = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    let loc = document::locate(&install, &camp, 15).unwrap();
    let layers = ConfigLayers::for_level(&install, None, Some(&rig.pack_dir()), None, Some(2));
    let gd = Arc::new(GameData::load(&layers).unwrap());
    // the script without PLAYER0's gold and limit: the wizard block sets them
    let script: String = SCRIPT
        .lines()
        .filter(|l| !l.contains("(PLAYER0,"))
        .map(|l| format!("{l}\n"))
        .collect();
    let base_dir = rig.dir.join("stage-base");
    let mut base_doc = Document::load(&loc, gd.clone()).unwrap();
    base_doc.save_to(&base_dir, 15).unwrap();
    rig.install_level(1, &base_dir, 15, loc.dims.w, loc.dims.h, Some(&script));
    let base = rig.run(1, SECS);
    report("baseline map15 resaved", &base);

    // doodads
    let mut d = Document::load(&loc, gd.clone()).unwrap();
    d.verify_tracked = false;
    let lava_fx = gd
        .effect_generator_names()
        .into_iter()
        .map(|(_, n)| n)
        .find(|n| n.to_ascii_uppercase().contains("LAVA"))
        .expect("a lava effect generator");
    let set = crate::doodads::Doodads {
        seed: Some(5),
        rules: vec![
            crate::doodads::DoodadRule {
                slab: "PATH".into(),
                model: "BARREL".into(),
                chance: 4.0,
                ..Default::default()
            },
            crate::doodads::DoodadRule {
                slab: "PRETTY_PATH".into(),
                model: "BARREL".into(),
                chance: 4.0,
                ..Default::default()
            },
            crate::doodads::DoodadRule {
                slab: "LAVA".into(),
                kind: crate::doodads::DoodadKind::Effect,
                model: lava_fx.clone(),
                chance: 3.0,
                ..Default::default()
            },
        ],
    };
    let before = d.doc.things.items.len();
    let count = |name: &str| {
        gd.slab_id(name).map_or(0, |k| {
            d.doc.kinds.iter().filter(|&&x| x as usize == k).count()
        })
    };
    println!(
        "[doodads] slabs: PATH {} PRETTY_PATH {} LAVA {}; problems {:?}",
        count("PATH"),
        count("PRETTY_PATH"),
        count("LAVA"),
        crate::doodads::problems(&set, &gd)
    );
    let (n, _) = d.apply_doodads(set);
    println!(
        "[doodads] {n} placed ({before} -> {} things; lava effect {lava_fx})",
        d.doc.things.items.len()
    );
    assert!(n > 10, "doodads placed: {n}");
    let dir = rig.dir.join("stage-doodads");
    d.save_to(&dir, 15).unwrap();
    assert!(
        crate::doodads::Doodads::load(&dir, 15).unwrap().rules.len() == 3,
        "rules saved in the sidecar"
    );

    // the wizard block
    let mut m = kfx_script_ui::wizard::WizardModel::read(&script);
    {
        let p0 = m.player_mut(kfx_script_ui::wizard::Player::P0);
        p0.start_money = Some(12345);
        p0.max_creatures = Some(17);
    }
    let plan = m.plan(script.as_bytes());
    let text = String::from_utf8(plan.apply_to(script.as_bytes())).unwrap();
    println!("[wizard] script:\n{text}");
    assert!(
        text.starts_with(&script[..script.find('\n').unwrap()]),
        "the rest of the script is kept"
    );
    rig.install_level(2, &dir, 15, loc.dims.w, loc.dims.h, Some(&text));
    let lua = format!(
        "{}\nlocal function kfx_wizard()\n  print(\"KFXDUMP WIZARD \" .. \
        tostring(PLAYER0.MONEY) .. \" \" .. \
        tostring(PLAYER0.max_creatures))\nend\nRegisterTimerEvent(kfx_wizard, 2, false)\n",
        game::dump_lua(loc.dims.w, loc.dims.h)
    );
    rig.write_level_file(2, "lua", &lua);
    let run = rig.run(2, SECS);
    report("doodads + wizard", &run);
    let mut failures = vec![];
    if !run.started || run.crashed || run.exited_early {
        failures.push("doodads + wizard: did not start / crashed".to_string());
    }
    for l in run.new_problems_vs(&base) {
        failures.push(format!(
            "doodads + wizard: NEW log line vs the unedited map: {l}"
        ));
    }
    let saved = MapDoc::load(&dir, 15, d.dims(), &gd).unwrap();
    let (p, _) = compare("doodads + wizard", &saved, &gd, &run);
    failures.extend(p);
    let wiz = run.lines.iter().find_map(|l| {
        l.raw
            .split("KFXDUMP WIZARD ")
            .nth(1)
            .map(|s| s.trim().to_string())
    });
    println!("[wizard] in the game: PLAYER0 MONEY / max_creatures = {wiz:?}");
    match wiz
        .as_deref()
        .map(|s| s.split_whitespace().collect::<Vec<_>>())
    {
        Some(v) if v.len() == 2 => {
            if v[0] != "12345" {
                failures.push(format!(
                    "wizard: PLAYER0 gold is {} in the game, not 12345",
                    v[0]
                ));
            }
            if v[1] != "17" {
                failures.push(format!(
                    "wizard: PLAYER0 creature limit is {} in the game, not 17",
                    v[1]
                ));
            }
        }
        _ => failures.push("wizard: no KFXDUMP WIZARD line in the log".into()),
    }
    for f in &failures {
        println!("FAIL {f}");
    }
    assert!(failures.is_empty(), "{} problems", failures.len());
}
