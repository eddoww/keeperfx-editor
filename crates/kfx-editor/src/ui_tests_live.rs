//! The Live link panel through the real UI (`ui_tests` harness) against the mock game server of
//! `kfx-live` (same framing and limits as `src/api.c`): Level > Live link opens it, Connect reaches
//! the mock, an event the game sends shows in the Events tab, a watched variable updates; a launch
//! with "Enable the game API when playing" gets `-cfg API_ENABLED=ON -cfg API_PORT=<port>` and the
//! panel connects by itself; creature positions printed into the game log are drawn on the map;
//! "Game camera to the selection" sends `ZOOM_TO_LOCATION` for the selected action point.
// the mock game server shared with the kfx-live tests; this test uses a subset of it
#[path = "../../kfx-live/tests/support/mod.rs"]
#[allow(dead_code)]
mod support;

use crate::playtest::Launch;
use crate::ui_tests::{level_copy, Tmp, Ui};
use egui_kittest::kittest::Queryable;
use kfx_formats::testutil;
use kfx_map::things::ItemRef;
use std::io::Write;
use std::time::{Duration, Instant};
use support::Mock;

/// Run frames until `f` holds (at most `secs`).
fn until(u: &mut Ui, secs: u64, mut f: impl FnMut(&mut Ui) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        u.steps(1);
        if f(u) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn connected(u: &mut Ui) -> bool {
    u.app()
        .live
        .conn
        .as_ref()
        .is_some_and(|c| c.panel.client.is_connected())
}

#[test]
fn ui_live_panel_with_the_mock_game() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("audiolive-live") else {
        return;
    };
    let dir = level_copy(&inst, 1, "live");
    u.open(&dir.join("map00001.slb"));
    let m = Mock::start();
    m.set_var(0, "MONEY", 1000);

    // Level > Live link, address, Connect
    u.menu("Level", "Live link");
    assert!(u.app().settings.audiolive.live_open);
    assert!(
        u.has("What the live link cannot do"),
        "the honest limits are in the panel"
    );
    u.app().live.addr = format!("127.0.0.1:{}", m.port);
    u.click("Connect");
    assert!(until(&mut u, 10, connected), "connected to the mock");
    // the panel subscribes on connect; then the game sends an event and a variable changes
    assert!(
        until(&mut u, 10, |_| m.with(|s| s
            .sub_events
            .iter()
            .any(|e| e == "GAME_SAVED")
            && s.sub_vars.iter().any(|v| v.1 == "MONEY"))),
        "subscribed"
    );
    m.emit("GAME_SAVED", None);
    m.set_var(0, "MONEY", 2234);
    let seen = until(&mut u, 10, |u| {
        u.app()
            .live
            .conn
            .as_ref()
            .is_some_and(|c| c.panel.log.iter().any(|e| e.tag == "GAME_SAVED"))
    });
    assert!(seen, "GAME_SAVED arrived");
    u.steps(2);
    assert!(
        u.h.query_all_by_label("GAME_SAVED").next().is_some(),
        "the event is shown in the Events tab"
    );
    u.shot("events");
    assert!(
        until(&mut u, 10, |u| {
            let c = u.app().live.conn.as_ref().unwrap();
            c.panel
                .watches
                .iter()
                .find(|w| w.var == "MONEY")
                .and_then(|w| c.panel.values.get(w))
                .and_then(|v| v.value)
                == Some(2234)
        }),
        "the watched MONEY updated"
    );
    u.click_first("Watch");
    u.steps(2);
    assert!(
        u.h.query_all_by_label("2234").next().is_some(),
        "the watch table shows the value"
    );
    u.shot("watch");

    // jump the game camera to the selected action point (no cheats needed)
    let ap = u
        .app()
        .doc
        .as_ref()
        .unwrap()
        .doc
        .action_points
        .items
        .iter()
        .position(|a| a.number.is_some());
    if let Some(i) = ap {
        let n = u.app().doc.as_ref().unwrap().doc.action_points.items[i]
            .number
            .unwrap();
        u.app().selected.clear();
        u.app().selected.insert(ItemRef::ActionPoint(i));
        u.click("Game camera to the selection");
        let want = format!("ZOOM_TO_LOCATION(PLAYER0,{n})");
        assert!(
            until(&mut u, 10, |_| m
                .with(|s| s.map_commands.iter().any(|c| c == &want))),
            "{want} sent: {:?}",
            m.with(|s| s.map_commands.clone())
        );
    }

    // Play with the API on: the launch gets the -cfg lines, the panel reconnects by itself and
    // follows the game's log, whose creature positions are drawn on the map
    u.click("Disconnect");
    assert!(u.app().live.conn.is_none());
    let game = Tmp(kfx_formats::testutil::scratch_dir(&format!(
        "kfx-editor/uitest-live-game-{}",
        std::process::id()
    )));
    let _ = std::fs::create_dir_all(&game.0);
    std::fs::write(game.join("keeperfx.log"), "LOG CREATED\n").unwrap();
    u.app().settings.audiolive.api_on_play = true;
    u.app().settings.audiolive.port = m.port;
    let l = u.app().live_prepare_launch(Launch {
        program: game.join("keeperfx"),
        args: vec![
            "-campaign".into(),
            "keeporig".into(),
            "-level".into(),
            "1".into(),
        ],
        cwd: game.0.clone(),
    });
    let tail: Vec<&str> = l
        .args
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(String::as_str)
        .collect();
    assert_eq!(
        tail,
        [
            "-cfg",
            "API_ENABLED=ON",
            "-cfg",
            &format!("API_PORT={}", m.port)
        ]
    );
    u.app().live_game_launched(&l);
    assert!(
        until(&mut u, 10, connected),
        "auto connect after the launch"
    );
    assert!(
        until(&mut u, 5, |u| u
            .app()
            .live
            .conn
            .as_ref()
            .is_some_and(|c| c.panel.log_path.is_some())),
        "the game log is followed once connected"
    );
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(game.join("keeperfx.log"))
            .unwrap();
        writeln!(f, "Lua: KFXLIVE THING Creature IMP 0 121 128").unwrap();
        writeln!(f, "Lua: KFXLIVE THING Creature IMP 0 127 136").unwrap();
    }
    assert!(
        until(&mut u, 5, |u| u.app().live.conn.as_ref().is_some_and(|c| c
            .panel
            .things
            .len()
            == 2)),
        "positions read from the log"
    );
    u.app().settings.audiolive.live_open = false;
    u.look_at(42, 44, 0.6);
    u.shot("overlay");
    u.app().live_disconnect();
    drop(m);
}
