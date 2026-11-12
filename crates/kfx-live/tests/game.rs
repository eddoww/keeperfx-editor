//! The live link against the real game (`KFX_GAME_TEST=1`; offscreen video, dummy audio, one game
//! at a time through the lock of `crates/kfx-map/tests/support/game.rs`; the API listens on
//! 127.0.0.1 on a random high port). Skipped otherwise.
//!
//! The rig of the harness copies `keeperfx.cfg` into a private folder; this test then switches
//! `API_ENABLED` on and sets `API_PORT` there (the editor uses `-cfg API_ENABLED=ON -cfg
//! API_PORT=n` instead, see `crate::proto`). The game is started by `Rig::run` (SIGTERM at the
//! timeout) on a second thread, the client drives it from this one. The game runs without `-alex`,
//! so console commands that need cheat mode are expected to fail and the test uses script commands
//! for the effects.
#![cfg(unix)]
// the game harness of kfx-map's tests (it allows its own unused parts)
#[path = "../../kfx-map/tests/support/game.rs"]
mod harness;

use harness::Rig;
use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::levelinfo::Lof;
use kfx_formats::{rnc, testutil};
use kfx_live::actions::{self, LuaChannel};
use kfx_live::client::{Config, LiveClient, LiveEvent};
use kfx_live::logtail::{parse_things, LogTail};
use kfx_live::proto::{ApiError, Player, Request};
use std::path::Path;
use std::time::{Duration, Instant};

const SECS: u64 = 18;

fn enabled() -> Option<std::path::PathBuf> {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: game tests skipped");
        return None;
    }
    testutil::install_dir()
}

fn dims_of(dir: &Path, n: u32) -> MapDims {
    if let Some(l) = level_bin::find_file(dir, n, "lof").and_then(|p| Lof::load(&p).ok()) {
        let (w, h) = l.info().mapsize_or_default();
        return MapDims::new(w as u16, h as u16);
    }
    MapDims::from_slb_len(
        rnc::load_file(&level_bin::find_file(dir, n, "slb").unwrap())
            .unwrap()
            .len(),
    )
    .unwrap()
}

/// A random high port that is free right now.
fn free_port() -> u16 {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64
        ^ (std::process::id() as u64) << 16;
    for i in 0..200u64 {
        let port = 20000 + ((seed.wrapping_mul(2654435761).wrapping_add(i * 7919)) % 40000) as u16;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free port");
}

fn set_cfg_key(cfg: &str, key: &str, value: &str) -> String {
    let mut out = String::new();
    let mut seen = false;
    for line in cfg.lines() {
        if line.split('=').next().map(str::trim) == Some(key) {
            out.push_str(&format!("{key}={value}\n"));
            seen = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !seen {
        out.push_str(&format!("{key}={value}\n"));
    }
    out
}

fn call(c: &LiveClient, what: &str, r: Request) -> kfx_live::Reply {
    let rep = c.call(r).unwrap_or_else(|e| panic!("{what}: {e}"));
    println!(
        "[call] {what}: success={} data={:?} error={:?}",
        rep.success, rep.data, rep.error
    );
    rep
}

#[test]
fn live_link_with_the_real_game() {
    let Some(install) = enabled() else { return };
    let rig = Rig::new(&install, "live-api");
    let dir = install.join("multiplayer/classic");
    let n = 55;
    let dims = dims_of(&dir, n);
    rig.install_level(1, &dir, n, dims.w, dims.h, None);
    let port = free_port();
    let cfg_path = rig.dir.join("keeperfx.cfg");
    let cfg = std::fs::read_to_string(&cfg_path).unwrap();
    let cfg = set_cfg_key(
        &set_cfg_key(&cfg, "API_ENABLED", "ON"),
        "API_PORT",
        &port.to_string(),
    );
    std::fs::write(&cfg_path, cfg).unwrap();
    println!("API on 127.0.0.1:{port}, map {}x{}", dims.w, dims.h);

    let mut transcript: Vec<String> = vec![];
    let mut events: Vec<String> = vec![];
    let mut things = vec![];
    let run = std::thread::scope(|s| {
        let game = s.spawn(|| rig.run(1, SECS));
        let t0 = Instant::now();
        let mut ccfg = Config::localhost(port);
        ccfg.log_wire = true;
        ccfg.reconnect_interval = Duration::from_millis(300);
        ccfg.request_timeout = Duration::from_secs(8);
        let c = LiveClient::spawn(ccfg);
        // wait for the handshake: the game answers once its main loop polls the socket
        let end = Instant::now() + Duration::from_secs(40);
        while c.status().kfx_version.is_none() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(100));
        }
        let version = c.status().kfx_version.clone();
        println!(
            "handshake after {:.1} s: KeeperFX {version:?}",
            t0.elapsed().as_secs_f64()
        );
        assert!(
            version.is_some(),
            "no answer from the game: {:?}",
            c.status()
        );

        // subscriptions
        assert!(
            call(
                &c,
                "subscribe GAME_SAVED",
                Request::SubscribeEvent("GAME_SAVED".into())
            )
            .success
        );
        assert!(
            call(
                &c,
                "subscribe GAME_STARTED",
                Request::SubscribeEvent("GAME_STARTED".into())
            )
            .success
        );
        assert!(
            call(
                &c,
                "subscribe_var MONEY",
                Request::SubscribeVar {
                    var: "MONEY".into(),
                    player: Some(Player::P0)
                }
            )
            .success
        );
        let r = call(
            &c,
            "unknown var",
            Request::SubscribeVar {
                var: "NOT_A_VARIABLE".into(),
                player: None,
            },
        );
        assert_eq!(r.error, Some(ApiError::UnknownVar));

        // level and turn
        let r = call(&c, "get_level_info", Request::GetLevelInfo);
        let li = kfx_live::LevelInfo::from_value(r.data.as_ref().expect("level data")).unwrap();
        println!("level info: {li:?}");
        assert_eq!(li.level_number, 1);
        assert_eq!((li.mapsize_x, li.mapsize_y), (dims.w as i64, dims.h as i64));
        let t1 = call(&c, "game_turn 1", Request::GetCurrentGameInfo)
            .data
            .unwrap()["game_turn"]
            .as_i64()
            .unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        let t2 = call(&c, "game_turn 2", Request::GetCurrentGameInfo)
            .data
            .unwrap()["game_turn"]
            .as_i64()
            .unwrap();
        assert!(t2 > t1, "the game advances: {t1} -> {t2}");
        let v1 = call(
            &c,
            "read_var GAME_TURN",
            actions::read_var(&Player::P0, "GAME_TURN"),
        )
        .int()
        .unwrap();
        assert!(v1 >= t2);

        // an observable effect: gold
        let money0 = call(
            &c,
            "read_var MONEY",
            actions::read_var(&Player::P0, "MONEY"),
        )
        .int()
        .unwrap();
        assert!(
            call(
                &c,
                "ADD_GOLD_TO_PLAYER",
                actions::add_gold(&Player::P0, 1234)
            )
            .success
        );
        let money1 = call(
            &c,
            "read_var MONEY",
            actions::read_var(&Player::P0, "MONEY"),
        )
        .int()
        .unwrap();
        println!("MONEY {money0} -> {money1}");
        assert!(
            money1 >= money0 + 1234 - 1,
            "gold added: {money0} -> {money1}"
        );

        // Lua: an expression evaluated in the game, result brought back through a scratch flag
        let mut lua = LuaChannel::default();
        let eval = lua.eval_int("6*7").unwrap();
        assert!(call(&c, "RUN_LUA_CODE PLAYER0.FLAG7=(6*7)", eval).success);
        assert_eq!(
            call(&c, "read_var FLAG7", lua.result_request()).int(),
            Some(42)
        );
        // flags through set_var
        assert!(call(&c, "set_var FLAG6", actions::set_flag(&Player::P0, 6, 5)).success);
        assert_eq!(
            call(
                &c,
                "read_var FLAG6",
                actions::read_var(&Player::P0, "FLAG6")
            )
            .int(),
            Some(5)
        );
        // the whole-flag table (see the documented 1024 byte limit)
        let r = call(&c, "get_all_player_flags", Request::GetAllPlayerFlags);
        println!(
            "get_all_player_flags: success={} error={:?} bytes={}",
            r.success,
            r.error,
            r.data.as_ref().map(|d| d.to_string().len()).unwrap_or(0)
        );

        // errors
        assert_eq!(
            call(
                &c,
                "unknown action",
                Request::Raw(serde_json::json!({"action": "teleport_pigs"}))
            )
            .error,
            Some(ApiError::UnknownAction)
        );
        assert_eq!(
            call(
                &c,
                "bad script line",
                actions::script("THIS_IS_NOT_A_COMMAND(1)")
            )
            .error,
            Some(ApiError::FailedToExecuteMapCommand)
        );
        let r = call(
            &c,
            "zoomto.subtilecoord (needs cheats)",
            actions::zoom_to_subtile(100, 100),
        );
        println!("zoomto without -alex: {:?}", r.error);
        assert_eq!(r.error, Some(ApiError::FailedToExecuteConsoleCommand));
        // camera jump that does not need cheats: the player's own heart
        assert!(
            call(
                &c,
                "ZOOM_TO_LOCATION heart",
                actions::script("ZOOM_TO_LOCATION(PLAYER0,PLAYER0)")
            )
            .success
        );
        // game speed
        assert!(call(&c, "fps 20", actions::set_speed(20)).success);

        // things through Lua + the game log
        let mut tail = LogTail::new(rig.dir.join("keeperfx.log"));
        for class in ["Creature", "Object"] {
            for req in lua.dump_things(class).unwrap() {
                assert!(call(&c, &format!("dump {class}"), req).success);
            }
        }
        let end = Instant::now() + Duration::from_secs(5);
        let mut lines = vec![];
        while things_empty(&lines) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(200));
            lines.extend(tail.poll());
        }
        things = parse_things(&lines);
        println!(
            "things from the log: {} (first: {:?})",
            things.len(),
            things.first()
        );
        println!("Lua string pool used: {} bytes", lua.pool_used);

        // an event: saving the game sends GAME_SAVED
        assert!(call(&c, "game.save 1", actions::save_game(1)).success);
        let end = Instant::now() + Duration::from_secs(8);
        while Instant::now() < end && !events.iter().any(|e| e.contains("GAME_SAVED")) {
            for ev in c.poll() {
                collect(&mut events, &mut transcript, ev);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        for ev in c.poll() {
            collect(&mut events, &mut transcript, ev);
        }
        println!("events: {events:?}");
        assert!(
            events.iter().any(|e| e.contains("GAME_SAVED")),
            "no GAME_SAVED event: {events:?}"
        );
        assert!(
            events.iter().any(|e| e.contains("VAR_UPDATE")),
            "MONEY changed, a VAR_UPDATE should have come: {events:?}"
        );

        // pause: the game refuses commands while paused and cannot be resumed through the API
        assert!(call(&c, "pause", actions::pause()).success);
        std::thread::sleep(Duration::from_millis(500));
        let paused_read = c.call_timeout(
            actions::read_var(&Player::P0, "GAME_TURN"),
            Duration::from_secs(4),
        );
        println!("read_var while paused: {paused_read:?}");
        let r = c.call_timeout(
            actions::script("ADD_GOLD_TO_PLAYER(PLAYER0,1)"),
            Duration::from_secs(4),
        );
        println!("map_command while paused: {r:?}");
        if let Ok(r) = &r {
            assert_eq!(r.error, Some(ApiError::GameIsPaused));
        }

        // clean disconnect
        for ev in c.poll() {
            collect(&mut events, &mut transcript, ev);
        }
        c.shutdown();
        game.join().unwrap()
    });
    println!("--- exchanged messages (client -> game / game -> client) ---");
    for t in &transcript {
        println!("{t}");
    }
    println!("--- game log lines about the API ---");
    for l in run.lines.iter().filter(|l| {
        l.raw.contains("API")
            || l.raw.contains("Client connected")
            || l.raw.contains("Received message from client")
    }) {
        println!("{}", l.raw);
    }
    println!("{}", run.summary());
    assert!(run.started, "the level started");
    assert!(!run.crashed, "the game did not crash");
    assert!(run
        .lines
        .iter()
        .any(|l| l.raw.contains("API server active")));
    assert!(run.lines.iter().any(|l| l.raw.contains("Client connected")));
    assert!(
        run.lines
            .iter()
            .any(|l| l.raw.contains("API connection closed")),
        "the game saw a clean disconnect"
    );
    let _ = things;
}

fn things_empty(lines: &[String]) -> bool {
    parse_things(lines).is_empty()
}

fn collect(events: &mut Vec<String>, transcript: &mut Vec<String>, ev: LiveEvent) {
    match ev {
        LiveEvent::Event(e) => events.push(format!(
            "{} {}",
            e.name,
            e.data
                .map(|d| d.to_string())
                .or(e
                    .var
                    .map(|v| format!("{} {} {}", v.player, v.name, v.value)))
                .unwrap_or_default()
        )),
        LiveEvent::Wire { sent, text } => {
            transcript.push(format!("{} {text}", if sent { "->" } else { "<-" }))
        }
        _ => {}
    }
}
