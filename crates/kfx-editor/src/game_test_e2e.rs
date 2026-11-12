//! End to end: a level made entirely by the editor's headless mode, played by the real game.
//!
//! 1. `--actions` (run in-process through `headless::run`): the constrained generator makes a
//!    one-keeper map with a hero gate, two starter prefabs are placed (the *Ambush corridor* with
//!    its script fragment, and a neutral den), the Events editor adds a hero wave from hero gate 1
//!    at `GAME_TURN >= WAVE_TURN`, and the level is saved straight into the harness's personal
//!    map pack. `validate` and `script-check` (the CLI subcommands) must report no errors.
//! 2. A baseline: a shipped level run the same way (same play path, API on), for the log
//!    comparison.
//! 3. The generated level, started through the editor's play path with the game's API on
//!    (`-cfg API_ENABLED=ON -cfg API_PORT=n`), offscreen; the Live link panel connects, reads its
//!    default watches (MONEY, FLAG0, TOTAL_CREATURES of PLAYER0), and the test polls GAME_TURN and
//!    PLAYER_GOOD's TOTAL_CREATURES through the panel's client until the wave has come. Then the
//!    creature positions (Lua into the game log) show the heroes near hero gate 1.
//!
//! Needs `KFX_GAME_TEST=1` and the test install; one game at a time (the harness's lock
//! directory), each game under a hard timeout and killed when it ends (also on panic). The rig is
//! deleted at the end unless `KFX_E2E_KEEP=1`; `KFX_GAME_RIG_ROOT` moves it off /tmp.
// the game harness shared with the kfx-map tests; these tests use a subset of it
#[path = "../../kfx-map/tests/support/game.rs"]
#[allow(dead_code)]
mod game;

use crate::settings::Settings;
use crate::ui_tests::Ui;
use game::{GameRun, LogLine, Rig};
use kfx_formats::testutil;
use kfx_live::model::Watch;
use kfx_live::proto::Player;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const LOCK_DIR: &str = "/tmp/kfx-map-gametest.lock";
/// The wave's turn (the game runs 20 turns a second: 15 s after the level starts).
const WAVE_TURN: i64 = 300;
/// Hard limit of one game run.
const GAME_TIMEOUT: Duration = Duration::from_secs(75);
const BASELINE_SECS: u64 = 25;
const LEVEL: u32 = 1;
const BASE_LEVEL: u32 = 2;

/// The harness's lock directory (same protocol as `game.rs`).
struct Lock;

impl Lock {
    fn acquire() -> Lock {
        let t = Instant::now();
        loop {
            if std::fs::create_dir(LOCK_DIR).is_ok() {
                let _ = std::fs::write(
                    Path::new(LOCK_DIR).join("pid"),
                    std::process::id().to_string(),
                );
                return Lock;
            }
            let pid = std::fs::read_to_string(Path::new(LOCK_DIR).join("pid")).unwrap_or_default();
            let alive = !pid.trim().is_empty() && Path::new("/proc").join(pid.trim()).exists();
            if !alive && t.elapsed() > Duration::from_secs(2) {
                let _ = std::fs::remove_dir_all(LOCK_DIR);
                continue;
            }
            assert!(
                t.elapsed() < Duration::from_secs(600),
                "another game run holds {LOCK_DIR}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(LOCK_DIR);
    }
}

/// SIGTERM, then SIGKILL after 5 s, when dropped.
struct Game(Option<Child>);

impl Game {
    fn stop(&mut self) {
        let Some(mut c) = self.0.take() else { return };
        let _ = Command::new("kill")
            .args(["-TERM", &c.id().to_string()])
            .status();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if let Ok(Some(_)) = c.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = c.kill();
        let _ = c.wait();
    }
}

impl Drop for Game {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Deletes the rig when dropped (unless `KFX_E2E_KEEP=1`).
struct RigCleanup(PathBuf);

impl Drop for RigCleanup {
    fn drop(&mut self) {
        if std::env::var("KFX_E2E_KEEP").ok().as_deref() == Some("1") {
            println!("rig kept at {}", self.0.display());
            return;
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
    l.local_addr().unwrap().port()
}

/// Start the game offscreen (SDL offscreen video, dummy audio, no display variables).
fn spawn(l: &crate::playtest::Launch, out: &Path) -> Game {
    let _ = std::fs::remove_file(l.cwd.join("keeperfx.log"));
    let f = std::fs::File::create(out).unwrap();
    let child = Command::new(&l.program)
        .args(&l.args)
        .current_dir(&l.cwd)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("SDL_VIDEODRIVER", "offscreen")
        .env("SDL_AUDIODRIVER", "dummy")
        .stdin(Stdio::null())
        .stdout(f.try_clone().unwrap())
        .stderr(f)
        .spawn()
        .expect("start the game");
    println!("game pid {}", child.id());
    Game(Some(child))
}

/// The game log as a harness run.
fn read_run(dir: &Path, level: u32) -> GameRun {
    let text = std::fs::read(dir.join("keeperfx.log"))
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    let lines: Vec<LogLine> = text.lines().map(LogLine::parse).collect();
    let started = lines
        .iter()
        .any(|l| l.raw.starts_with(&format!("Started level {level} ")));
    let turn = lines.iter().find_map(|l| {
        let i = l.raw.find("\"turn\":")?;
        l.raw[i + 7..]
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    });
    let crashed = lines
        .iter()
        .any(|l| l.raw.contains("Failure signal") && !l.raw.contains("Termination"));
    GameRun {
        lines,
        started,
        turn,
        exited_early: false,
        crashed,
        stdout: String::new(),
    }
}

/// Error / warning lines of `run` not in `base` (multiset difference without the varying
/// "[player] " prefix, level numbers made equal).
fn new_problems(run: &GameRun, base: &GameRun) -> Vec<String> {
    let norm = |l: &LogLine| {
        let m = l
            .msg
            .replace(&format!("map{BASE_LEVEL:05}"), "mapN")
            .replace(&format!("map{LEVEL:05}"), "mapN")
            .replace(&format!("level {BASE_LEVEL}"), "level N")
            .replace(&format!("level {LEVEL}"), "level N");
        (l.level, l.func.clone(), m)
    };
    let mut pool: Vec<_> = base.problems().into_iter().map(norm).collect();
    let mut out = vec![];
    for l in run.problems() {
        match pool.iter().position(|p| *p == norm(l)) {
            Some(i) => {
                pool.swap_remove(i);
            }
            None => out.push(l.raw.clone()),
        }
    }
    out
}

fn wait(u: &mut Ui, deadline: Instant, mut f: impl FnMut(&mut Ui) -> bool) -> bool {
    while Instant::now() < deadline {
        u.steps(1);
        if f(u) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn value(u: &mut Ui, player: Player, var: &str) -> Option<i64> {
    let c = u.app().live.conn.as_ref()?;
    c.panel
        .values
        .get(&Watch {
            player,
            var: var.into(),
        })?
        .value
}

/// A blocking read through the Live panel's client.
fn read(u: &mut Ui, player: &Player, var: &str) -> Option<i64> {
    let c = u.app().live.conn.as_ref()?;
    c.panel
        .client
        .call_timeout(
            kfx_live::actions::read_var(player, var),
            Duration::from_secs(3),
        )
        .ok()?
        .int()
}

/// The `--actions` script of the level (written next to the rig's editor settings).
fn actions(pack: &Path, shot: &Path) -> serde_json::Value {
    serde_json::json!([
        {"new_map": {"folder": pack.display().to_string(), "level": LEVEL, "w": 60, "h": 60,
                     "generate": true, "seed": 4242, "players": 1, "imps": 4, "hero_gates": 1,
                     "style": "open_cave", "script": "dk", "name": "E2E wave"}},
        {"prefabs": true},
        {"prefab_select": "Ambush corridor", "prefab_rotate": 0, "prefab_owners": [0]},
        {"prefab_place": [5, 15]},
        {"prefab_select": "Neutral den: spiders"},
        {"prefab_place": [45, 10]},
        {"events": "open"},
        {"events_add_wave": {"party": "E2E_WAVE",
                             "creatures": [["KNIGHT", 2, 0], ["ARCHER", 1], ["ARCHER", 1]],
                             "owner": "PLAYER_GOOD", "spawn": {"hero_gate": 1}, "turn": WAVE_TURN}},
        {"events_print": true},
        {"save": true},
        {"print": true},
        {"screenshot": shot.display().to_string(), "ui": true}
    ])
}

#[test]
fn game_e2e_generated_level_prefabs_and_wave() {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: skipped");
        return;
    }
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let rig = Rig::new(&install, "editor-e2e");
    let _cleanup = RigCleanup(rig.dir.clone());
    let pack = rig.pack_dir();

    // ---- 1. the level, made headlessly
    let work = rig.dir.join("e2e-work");
    std::fs::create_dir_all(work.join("config")).unwrap();
    std::env::set_var("KFX_EDITOR_CONFIG_DIR", work.join("config"));
    let shot = std::env::var_os("KFX_E2E_SHOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.join("level.png"));
    let json = serde_json::to_string_pretty(&actions(&pack, &shot)).unwrap();
    println!("--- actions JSON ---\n{json}");
    let file = work.join("e2e.json");
    std::fs::write(&file, &json).unwrap();
    let start_map = testutil::resolve_ci(&rig.dir, "campgns/keeporig/map00001.slb").unwrap();
    let args: Vec<String> = [
        "--screenshot",
        &work.join("final.png").display().to_string(),
        "--map",
        &start_map.display().to_string(),
        "--actions",
        &file.display().to_string(),
        "--size",
        "1600x1000",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let o = crate::headless::Options::parse(&args).unwrap();
    crate::headless::run(&o).expect("the headless actions run");
    let slb = pack.join(format!("map{LEVEL:05}.slb"));
    assert!(slb.is_file(), "level saved");
    // the pack lists its levels from `.lif` files; the editor's `.lof` is kept as written
    std::fs::write(
        pack.join(format!("map{LEVEL:05}.lif")),
        format!("{LEVEL}, E2E wave\n"),
    )
    .unwrap();
    let mut files: Vec<String> = std::fs::read_dir(&pack)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    println!("--- saved files: {files:?}");
    let script = std::fs::read_to_string(pack.join(format!("map{LEVEL:05}.txt"))).unwrap();
    println!("--- map{LEVEL:05}.txt ---\n{script}");
    println!(
        "--- map{LEVEL:05}.lof ---\n{}",
        std::fs::read_to_string(pack.join(format!("map{LEVEL:05}.lof"))).unwrap_or_default()
    );
    assert!(
        script.contains("REM --- prefab: Ambush corridor ---"),
        "the prefab's script fragment"
    );
    assert!(
        script.contains(&format!("IF(PLAYER0,GAME_TURN >= {WAVE_TURN})")),
        "the wave"
    );
    assert!(
        script.contains("ADD_PARTY_TO_LEVEL(PLAYER_GOOD,E2E_WAVE,-1,1)"),
        "the wave from hero gate 1"
    );
    let slb_s = slb.display().to_string();
    println!("--- kfx-editor validate {slb_s}");
    let v = crate::cli::run(&["validate".to_string(), slb_s.clone()]);
    println!("--- kfx-editor script-check {slb_s}");
    let s = crate::cli::run(&["script-check".to_string(), slb_s.clone()]);
    println!("validate exit {v}, script-check exit {s}");
    assert_eq!((v, s), (0, 0), "validate / script-check found errors");

    // ---- 2. baseline: a shipped level through the same play path, API on
    let camp = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    rig.install_level(BASE_LEVEL, &camp, 1, 85, 85, None);
    let port = free_port();
    let mut settings = Settings {
        install: Some(rig.dir.clone()),
        ..Default::default()
    };
    settings.play.exe = Some(rig.dir.join("keeperfx"));
    settings.play.no_sound = true;
    settings.audiolive.api_on_play = true;
    settings.audiolive.port = port;
    let mut u = Ui::with_settings("e2e-game", settings, eframe::egui::vec2(1400.0, 900.0));
    let _lock = Lock::acquire();
    u.open(&pack.join(format!("map{BASE_LEVEL:05}.slb")));
    let (l, _) = u
        .app()
        .play_launch()
        .expect("the baseline level can be played");
    let l = u.app().live_prepare_launch(l);
    println!("baseline launch: {}", l.display());
    let mut g = spawn(&l, &rig.dir.join("stdout-base.txt"));
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(BASELINE_SECS) {
        if let Some(c) = g.0.as_mut() {
            if let Ok(Some(_)) = c.try_wait() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    g.stop();
    let base = read_run(&rig.dir, BASE_LEVEL);
    println!("[baseline keeporig 1] {}", base.summary());
    for l in base.problems() {
        println!("[baseline]   {}", l.raw);
    }

    // ---- 3. the generated level, with the Live link panel
    u.open(&slb);
    let (l, target) = u.app().play_launch().expect("the level can be played");
    let l = u.app().live_prepare_launch(l);
    println!("target {target:?}");
    println!("launch: {}", l.display());
    assert!(l
        .args
        .windows(2)
        .any(|w| w[0] == "-cfg" && w[1] == "API_ENABLED=ON"));
    let mut g = spawn(&l, &rig.dir.join("stdout.txt"));
    let t0 = Instant::now();
    let deadline = t0 + GAME_TIMEOUT;
    u.app().live_game_launched(&l);
    let connected = wait(&mut u, deadline, |u| {
        u.app()
            .live
            .conn
            .as_ref()
            .is_some_and(|c| c.panel.client.status().kfx_version.is_some())
    });
    assert!(
        connected,
        "the panel connected: {:?}",
        u.app().live.conn.as_ref().map(|c| c.panel.client.status())
    );
    println!(
        "connected after {:.1} s: KeeperFX {:?}",
        t0.elapsed().as_secs_f64(),
        u.app()
            .live
            .conn
            .as_ref()
            .unwrap()
            .panel
            .client
            .status()
            .kfx_version
    );
    let good = Player::Name("PLAYER_GOOD".into());
    u.app()
        .live
        .conn
        .as_mut()
        .unwrap()
        .panel
        .add_watch(good.clone(), "TOTAL_CREATURES");
    assert!(
        wait(&mut u, deadline, |u| {
            value(u, Player::P0, "MONEY").is_some()
                && value(u, Player::P0, "FLAG0").is_some()
                && value(u, Player::P0, "TOTAL_CREATURES").is_some()
                && value(u, Player::Name("PLAYER_GOOD".into()), "TOTAL_CREATURES").is_some()
        }),
        "watched values read"
    );
    let reads = format!(
        "MONEY {:?} FLAG0 {:?} TOTAL_CREATURES {:?} | PLAYER_GOOD TOTAL_CREATURES {:?}",
        value(&mut u, Player::P0, "MONEY"),
        value(&mut u, Player::P0, "FLAG0"),
        value(&mut u, Player::P0, "TOTAL_CREATURES"),
        value(&mut u, good.clone(), "TOTAL_CREATURES")
    );
    println!("Live panel watches: {reads}");
    if let Some(li) = &u.app().live.conn.as_ref().unwrap().panel.level {
        println!("level info: {li:?}");
    }

    // poll the turn and the heroes until the wave has come
    let mut samples: Vec<(i64, i64)> = vec![];
    let mut before: Option<i64> = None;
    let mut spawned: Option<(i64, i64)> = None;
    while Instant::now() < deadline {
        u.steps(1);
        let (Some(turn), Some(n)) = (
            read(&mut u, &Player::P0, "GAME_TURN"),
            read(&mut u, &good, "TOTAL_CREATURES"),
        ) else {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        if samples.last().map(|s| s.1) != Some(n) || samples.len() % 10 == 0 {
            println!("  turn {turn}: PLAYER_GOOD TOTAL_CREATURES {n}");
        }
        samples.push((turn, n));
        if turn < WAVE_TURN {
            before = Some(before.map_or(n, |b: i64| b.max(n)));
        } else if n > before.unwrap_or(0) {
            spawned = Some((turn, n));
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let last_before = samples.iter().filter(|s| s.0 < WAVE_TURN).last().copied();
    println!(
        "last sample before turn {WAVE_TURN}: {last_before:?}; first sample with the wave: \
        {spawned:?}"
    );
    // the heroes' positions (Lua into the game log, the map overlay's data)
    u.app().live_refresh_positions();
    let got = wait(&mut u, deadline, |u| {
        u.app()
            .live
            .conn
            .as_ref()
            .is_some_and(|c| !c.panel.things.is_empty())
    });
    let heroes: Vec<_> = u
        .app()
        .live
        .conn
        .as_ref()
        .map(|c| {
            c.panel
                .things
                .iter()
                .filter(|t| t.owner == 4)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    println!(
        "positions read: {got}; PLAYER_GOOD creatures: {}",
        heroes
            .iter()
            .map(|t| format!("{} at stl ({},{})", t.model, t.stl_x, t.stl_y))
            .collect::<Vec<_>>()
            .join(", ")
    );
    {
        let c = u.app().live.conn.as_ref().unwrap();
        println!("--- Live panel log ---");
        for e in c
            .panel
            .log
            .iter()
            .filter(|e| e.kind != kfx_live::model::LogKind::Wire)
        {
            println!("{:?} {} {}", e.kind, e.tag, e.text);
        }
    }
    u.app().live_disconnect();
    std::thread::sleep(Duration::from_millis(300));
    g.stop();
    drop(_lock);
    let run = read_run(&rig.dir, LEVEL);
    println!(
        "[e2e level] {} (game ran {:.1} s)",
        run.summary(),
        t0.elapsed().as_secs_f64()
    );
    println!(
        "--- game log: errors / warnings of the generated level (classified against the \
        baseline) ---"
    );
    let new: Vec<String> = new_problems(&run, &base);
    for l in run.problems() {
        println!(
            "  [{}] {}",
            if new.contains(&l.raw) {
                "NEW"
            } else {
                "also in baseline"
            },
            l.raw
        );
    }
    println!("--- game log: level / API / script lines ---");
    for l in run.lines.iter().filter(|l| {
        [
            "Started level",
            "API",
            "Client connected",
            "party",
            "PARTY",
            "E2E_WAVE",
            "AMBUSH",
            "script",
            "Script",
        ]
        .iter()
        .any(|k| l.raw.contains(k))
    }) {
        println!("  {}", l.raw);
    }
    // the game prefixes its log lines with the game turn ("[302] ..."): the heroes in the log
    println!(
        "--- game log: the heroes (Lua position dump) and the reads around turn {WAVE_TURN} ---"
    );
    let turn_of = |raw: &str| {
        raw.strip_prefix('[')
            .and_then(|r| r.split(']').next())
            .and_then(|t| t.parse::<i64>().ok())
    };
    for l in run.lines.iter().filter(|l| {
        let near = turn_of(&l.raw).is_some_and(|t| (WAVE_TURN - 15..WAVE_TURN + 15).contains(&t));
        (l.raw.contains("KFXLIVE THING Creature") && l.raw.contains(" 4 "))
            || (near && l.raw.contains("read_var"))
    }) {
        println!("  {}", l.raw);
    }
    assert!(run.started, "the level started");
    assert!(!run.crashed, "the game crashed");
    assert!(
        run.lines
            .iter()
            .any(|l| l.raw.contains("API server active")),
        "the API was on"
    );
    let (turn, n) = spawned.expect("the wave did not come before the timeout");
    assert!(
        turn >= WAVE_TURN && turn < WAVE_TURN + 60,
        "the wave came at turn {turn}, expected right after {WAVE_TURN}"
    );
    assert!(
        n - before.unwrap_or(0) == 3,
        "three heroes: {before:?} -> {n}"
    );
    assert!(new.is_empty(), "new log problems: {new:?}");
}
