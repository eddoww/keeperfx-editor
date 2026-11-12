//! The live link against the real game, started from the editor's play path: the level is opened
//! in the editor, `play_launch` + `live_prepare_launch` build the command line (with
//! `-cfg API_ENABLED=ON -cfg API_PORT=<port>`, the user's keeperfx.cfg untouched), the game runs
//! offscreen (SDL offscreen video, dummy audio) in the private copy of the install made by the
//! `kfx-map` harness, `live_game_launched` connects the Live link panel, and the panel reads
//! variables, sets a flag and gets creature positions through the game log.
//!
//! Needs `KFX_GAME_TEST=1` and the test install; one game at a time (the harness's lock
//! directory), hard timeout, the game is killed when the test ends (also on panic).
// the game harness shared with the kfx-map tests; these tests use a subset of it
#[path = "../../kfx-map/tests/support/game.rs"]
#[allow(dead_code)]
mod game;

use crate::settings::Settings;
use crate::ui_tests::Ui;
use kfx_formats::testutil;
use kfx_live::model::Watch;
use kfx_live::proto::Player;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const LOCK_DIR: &str = "/tmp/kfx-map-gametest.lock";
const HARD_TIMEOUT: Duration = Duration::from_secs(60);

/// The harness's lock directory (same protocol as `game.rs`: the directory holds the owner pid).
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

impl Drop for Game {
    fn drop(&mut self) {
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

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
    l.local_addr().unwrap().port()
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

fn value(u: &mut Ui, var: &str) -> Option<i64> {
    let c = u.app().live.conn.as_ref()?;
    c.panel
        .values
        .get(&Watch {
            player: Player::P0,
            var: var.into(),
        })?
        .value
}

#[test]
fn game_live_link_from_the_play_path() {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: skipped");
        return;
    }
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let rig = game::Rig::new(&install, "editor-live");
    let camp = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    rig.install_level(1, &camp, 1, 85, 85, None);
    let port = free_port();
    let mut settings = Settings {
        install: Some(rig.dir.clone()),
        ..Default::default()
    };
    settings.play.exe = Some(rig.dir.join("keeperfx"));
    settings.audiolive.api_on_play = true;
    settings.audiolive.port = port;
    let mut u = Ui::with_settings(
        "audiolive-game",
        settings,
        eframe::egui::vec2(1400.0, 900.0),
    );
    u.open(&rig.pack_dir().join("map00001.slb"));

    // the editor's play path
    let (l, target) = u.app().play_launch().expect("the level can be played");
    let l = u.app().live_prepare_launch(l);
    println!("target {target:?}");
    println!("launch: {}", l.display());
    assert!(l
        .args
        .windows(2)
        .any(|w| w[0] == "-cfg" && w[1] == "API_ENABLED=ON"));
    assert!(l
        .args
        .windows(2)
        .any(|w| w[0] == "-cfg" && w[1] == format!("API_PORT={port}")));
    let cfg_before = std::fs::read(rig.dir.join("keeperfx.cfg")).unwrap();

    let _lock = Lock::acquire();
    let _ = std::fs::remove_file(l.cwd.join("keeperfx.log"));
    let out = std::fs::File::create(rig.dir.join("stdout.txt")).unwrap();
    let child = Command::new(&l.program)
        .args(&l.args)
        .current_dir(&l.cwd)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("SDL_VIDEODRIVER", "offscreen")
        .env("SDL_AUDIODRIVER", "dummy")
        .stdin(Stdio::null())
        .stdout(out.try_clone().unwrap())
        .stderr(out)
        .spawn()
        .expect("start the game");
    println!("game pid {}", child.id());
    let mut g = Game(Some(child));
    let t0 = Instant::now();
    let deadline = t0 + HARD_TIMEOUT;
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
        "the panel connected to the game: {:?}",
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

    // the default watches (MONEY, FLAG0, TOTAL_CREATURES) are read on connect
    assert!(
        wait(&mut u, deadline, |u| value(u, "MONEY").is_some()
            && value(u, "FLAG0").is_some()
            && value(u, "TOTAL_CREATURES").is_some()),
        "watched values read"
    );
    println!(
        "read_var: MONEY {:?} FLAG0 {:?} TOTAL_CREATURES {:?}",
        value(&mut u, "MONEY"),
        value(&mut u, "FLAG0"),
        value(&mut u, "TOTAL_CREATURES")
    );
    // set a flag (set_var, no cheats) and watch it change
    {
        let c = u.app().live.conn.as_mut().unwrap();
        c.panel.add_watch(Player::P0, "FLAG6");
        c.panel
            .quick("set FLAG6", kfx_live::actions::set_flag(&Player::P0, 6, 5));
        c.panel.refresh_watches();
    }
    assert!(
        wait(&mut u, deadline, |u| value(u, "FLAG6") == Some(5)),
        "FLAG6 = 5 read back: {:?}",
        value(&mut u, "FLAG6")
    );
    println!("FLAG6 after set_var: {:?}", value(&mut u, "FLAG6"));
    // creature positions through Lua and the game log (the map overlay's data)
    u.app().live_refresh_positions();
    let got = wait(&mut u, deadline, |u| {
        u.app()
            .live
            .conn
            .as_ref()
            .is_some_and(|c| !c.panel.things.is_empty())
    });
    {
        let c = u.app().live.conn.as_ref().unwrap();
        println!(
            "positions: {} (first {:?}), Lua pool used {} bytes",
            c.panel.things.len(),
            c.panel.things.first(),
            c.panel.lua.pool_used
        );
    }
    assert!(got, "creature positions from the game log");
    u.app().settings.audiolive.live_open = true;
    u.steps(3);
    u.shot("panel");
    u.app().settings.audiolive.live_open = false;
    let heart = u
        .app()
        .doc
        .as_ref()
        .unwrap()
        .doc
        .things
        .items
        .iter()
        .find(|t| t.owner == Some(0) && t.x.is_some())
        .and_then(|t| Some((t.x?.stl as i32 / 3, t.y?.stl as i32 / 3)));
    if let Some((x, y)) = heart {
        u.look_at(x, y, 0.5);
    }
    u.shot("overlay");
    {
        let c = u.app().live.conn.as_ref().unwrap();
        println!("--- panel log ---");
        for e in &c.panel.log {
            println!("{:?} {} {}", e.kind, e.tag, e.text);
        }
    }
    u.app().live_disconnect();
    std::thread::sleep(Duration::from_millis(500));
    drop(g.0.take().map(|c| Game(Some(c))));
    let log = std::fs::read_to_string(l.cwd.join("keeperfx.log")).unwrap_or_default();
    println!("--- game log lines about the API ---");
    for line in log.lines().filter(|l| {
        l.contains("API")
            || l.contains("Client connected")
            || l.contains("Started level")
            || l.contains("CLI (-cfg)")
    }) {
        println!("{line}");
    }
    assert!(
        log.lines().any(|l| l.contains("API server active")),
        "the API was on"
    );
    assert!(
        log.lines().any(|l| l.starts_with("Started level 1 ")),
        "the level started"
    );
    assert_eq!(
        std::fs::read(rig.dir.join("keeperfx.cfg")).unwrap(),
        cfg_before,
        "keeperfx.cfg not touched"
    );
}
