//! Protocol and client tests against a mock that reproduces the framing of `src/api.c`.
mod support;
use kfx_live::actions::{self, LuaChannel};
use kfx_live::client::{ClientError, Config, LiveClient, LiveEvent};
use kfx_live::logtail::{parse_things, LogTail};
use kfx_live::model::{ConsoleMode, LivePanel, LogKind};
use kfx_live::proto::{self, ApiError, Player, Request};
use serde_json::json;
use std::time::{Duration, Instant};
use support::Mock;

fn client_for(m: &Mock) -> LiveClient {
    let mut cfg = Config::localhost(m.port);
    cfg.reconnect_interval = Duration::from_millis(100);
    cfg.request_timeout = Duration::from_millis(800);
    cfg.log_wire = true;
    let c = LiveClient::spawn(cfg);
    assert!(
        c.wait_connected(Duration::from_secs(5)),
        "connect: {:?}",
        c.status()
    );
    // The connection is reported before the handshake reply with the game version arrives.
    let end = Instant::now() + Duration::from_secs(5);
    while c.status().kfx_version.is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    c
}

fn wait_for<T>(c: &LiveClient, secs: u64, mut f: impl FnMut(&LiveEvent) -> Option<T>) -> Option<T> {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Some(e) = c.poll_wait(Duration::from_millis(50)) {
            if let Some(r) = f(&e) {
                return Some(r);
            }
        }
    }
    None
}

#[test]
fn encoding_and_server_framing() {
    // minimal JSON, ack first, no spaces
    let b = proto::encode(
        &Request::ReadVar {
            var: "MONEY".into(),
            player: Some(Player::Index(0)),
        },
        Some(7),
    )
    .unwrap();
    assert!(!b.contains(&b' ') && !b.contains(&b'\n'));
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(
        v,
        json!({"ack":7,"action":"read_var","var":"MONEY","player":0})
    );
    let b = proto::encode(
        &Request::SetVar {
            var: "FLAG1".into(),
            player: Some(Player::Name("PLAYER2".into())),
            value: -5,
        },
        None,
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(
        v,
        json!({"action":"set_var","var":"FLAG1","player":"PLAYER2","value":-5})
    );
    // braces inside strings are escaped, so the server's brace counting sees one object
    let req = Request::MapCommand("RUN_LUA_CODE(\"local s='{' return s\")".into());
    let b = proto::encode(&req, Some(1)).unwrap();
    let text = String::from_utf8(b.clone()).unwrap();
    assert!(
        text.contains("\\u007b") && !text[1..text.len() - 1].contains('{'),
        "{text}"
    );
    let (objs, partial) = proto::server_split(&b);
    assert_eq!((objs.len(), partial), (1, false));
    // and JSON decoding gives the original text back
    let v: serde_json::Value = serde_json::from_slice(&objs[0]).unwrap();
    assert_eq!(v["command"], "RUN_LUA_CODE(\"local s='{' return s\")");
    // a raw unescaped brace in a string breaks the server framing
    let (objs, partial) = proto::server_split(br#"{"action":"map_command","command":"x{"}"#);
    assert_eq!((objs.len(), partial), (0, true));
    // several objects in one read, trailing newline removed
    let (objs, partial) = proto::server_split(b"{\"a\":1}{\"b\":{\"c\":2}}\n");
    assert_eq!((objs.len(), partial), (2, false));
    // NUL ends the text
    let (objs, _) = proto::server_split(b"{\"a\":1}\0{\"b\":2}");
    assert_eq!(objs.len(), 1);
    // oversize requests are refused instead of being cut by the 4095 byte buffer
    let big = Request::MapCommand("x".repeat(5000));
    assert!(matches!(
        proto::encode(&big, None),
        Err(proto::ProtoError::TooLong(_))
    ));
    // NUL
    assert!(proto::encode(&Request::MapCommand("a\0b".into()), None).is_err());
}

#[test]
fn parsing_server_lines() {
    use proto::Incoming::*;
    match proto::parse_incoming(r#"{"ack":3,"success":true,"data":42}"#).unwrap() {
        Reply(r) => assert_eq!((r.ack, r.success, r.int()), (Some(3), true, Some(42))),
        o => panic!("{o:?}"),
    }
    match proto::parse_incoming(r#"{"success":false,"error":"INVALID_JSON"}"#).unwrap() {
        Reply(r) => assert_eq!(
            (r.ack, r.success, r.error),
            (None, false, Some(ApiError::InvalidJson))
        ),
        o => panic!("{o:?}"),
    }
    match proto::parse_incoming(
        r#"{"event":"VAR_UPDATE","var":{"player":"PLAYER0","name":"MONEY","value":123}}"#,
    )
    .unwrap()
    {
        Event(e) => assert_eq!(e.var.unwrap().value, 123),
        o => panic!("{o:?}"),
    }
    match proto::parse_incoming(
        r#"{"event":"RESEARCH_COMPLETED","data":{"player":0,"kind_description":"SPELL"}}"#,
    )
    .unwrap()
    {
        Event(e) => assert_eq!(
            (e.name.as_str(), e.data.unwrap()["player"].as_i64()),
            ("RESEARCH_COMPLETED", Some(0))
        ),
        o => panic!("{o:?}"),
    }
    assert!(proto::parse_incoming("not json").is_err());
    assert!(matches!(proto::parse_incoming("[1]").unwrap(), Other(_)));
    assert_eq!(
        ApiError::from_code("GAME_IS_PAUSED"),
        ApiError::GameIsPaused
    );
    assert_eq!(ApiError::from_code("SOMETHING_NEW").code(), "SOMETHING_NEW");
}

#[test]
fn requests_and_replies() {
    let m = Mock::start();
    let c = client_for(&m);
    assert_eq!(c.status().kfx_version.as_deref(), Some("1.3.1-mock"));
    // data
    m.set_var(0, "MONEY", 500);
    let r = c.call(actions::read_var(&Player::P0, "MONEY")).unwrap();
    assert_eq!((r.success, r.int()), (true, Some(500)));
    // the effect of a command, then read back
    let r = c.call(actions::add_gold(&Player::P0, 1234)).unwrap();
    assert!(r.success);
    assert_eq!(
        c.call(actions::read_var(&Player::P0, "MONEY"))
            .unwrap()
            .int(),
        Some(1734)
    );
    assert_eq!(
        m.with(|s| s.map_commands.clone()),
        ["ADD_GOLD_TO_PLAYER(PLAYER0,1234)"]
    );
    // set_var / read_var with named player
    assert!(
        c.call(Request::SetVar {
            var: "FLAG3".into(),
            player: Some(Player::Name("PLAYER2".into())),
            value: 99
        })
        .unwrap()
        .success
    );
    let r = c
        .call(Request::ReadVar {
            var: "FLAG3".into(),
            player: Some(Player::Name("PLAYER2".into())),
        })
        .unwrap();
    assert_eq!(r.int(), Some(99));
    // typed errors
    let r = c
        .call(Request::ReadVar {
            var: "NOPE".into(),
            player: None,
        })
        .unwrap();
    assert_eq!(r.error, Some(ApiError::UnknownVar));
    let r = c
        .call(Request::SetVar {
            var: "MONEY".into(),
            player: None,
            value: 1,
        })
        .unwrap();
    assert_eq!(r.error, Some(ApiError::UnableToSetVar));
    let r = c.call(Request::MapCommand("BOGUS()".into())).unwrap();
    assert_eq!(r.error, Some(ApiError::FailedToExecuteMapCommand));
    let r = c
        .call(Request::Raw(json!({"action": "teleport_pigs"})))
        .unwrap();
    assert_eq!(r.error, Some(ApiError::UnknownAction));
    // level info typed
    let r = c.call(Request::GetLevelInfo).unwrap();
    let li = proto::LevelInfo::from_value(r.data.as_ref().unwrap()).unwrap();
    assert_eq!(
        (li.level_number, li.mapsize_x, li.campaign.is_map_pack),
        (1, 85, false)
    );
    // reply too big for the game's 1024 byte buffer (get_all_player_flags with big values)
    for i in 0..8 {
        m.set_var(0, &format!("FLAG{i}"), 1_000_000_000 + i);
    }
    let r = c.call(Request::GetAllPlayerFlags).unwrap();
    assert_eq!(r.error, Some(ApiError::FailedToCreateJson), "{r:?}");
    for i in 0..8 {
        m.set_var(0, &format!("FLAG{i}"), i);
    }
    let r = c.call(Request::GetAllPlayerFlags).unwrap();
    assert!(r.success);
    // the C bug: every player entry shows the request player's flags
    let d = r.data.unwrap();
    assert_eq!(d["PLAYER0"], d["PLAYER_NEUTRAL"]);
    // not in a local game: everything except the subscription family is refused, even unknown
    // actions
    m.with(|s| s.local_game = false);
    assert_eq!(
        c.call(Request::GetCurrentGameInfo).unwrap().error,
        Some(ApiError::NotInLocalGame)
    );
    assert_eq!(
        c.call(Request::Raw(json!({"action":"nonsense"})))
            .unwrap()
            .error,
        Some(ApiError::NotInLocalGame)
    );
    assert!(c.call(Request::GetKfxInfo).unwrap().success);
    assert!(c.call(Request::SubscribeEvent("X".into())).unwrap().success);
    // paused: commands refused, reads still answered
    m.with(|s| {
        s.local_game = true;
        s.paused = true;
    });
    assert_eq!(
        c.call(actions::pause()).unwrap().error,
        Some(ApiError::GameIsPaused)
    );
    assert_eq!(
        c.call(actions::script("ADD_GOLD_TO_PLAYER(PLAYER0,1)"))
            .unwrap()
            .error,
        Some(ApiError::GameIsPaused)
    );
    assert!(
        c.call(actions::read_var(&Player::P0, "MONEY"))
            .unwrap()
            .success
    );
    // long event name
    let r = c.call(Request::SubscribeEvent("E".repeat(300))).unwrap();
    assert_eq!(r.error, Some(ApiError::StringTooLong));
    // transcript sanity: acks are unique and echoed first
    let wire: Vec<String> = c
        .poll()
        .into_iter()
        .filter_map(|e| {
            if let LiveEvent::Wire { text, .. } = e {
                Some(text)
            } else {
                None
            }
        })
        .collect();
    assert!(
        wire.iter()
            .any(|w| w.starts_with(r#"{"ack":"#) && w.contains("get_kfx_info")),
        "{wire:?}"
    );
    assert!(
        wire.iter()
            .any(|w| w.starts_with(r#"{"ack":"#) && w.ends_with(r#""data":500,"success":true}"#)),
        "members sorted like the game's: {wire:?}"
    );
}

#[test]
fn braces_in_lua_survive_the_servers_brace_counting() {
    let m = Mock::start();
    let c = client_for(&m);
    let code = "local t = {1,2} x = '}' y = '{'";
    let req = actions::run_lua(code).unwrap();
    assert!(c.call(req).unwrap().success);
    assert_eq!(
        m.with(|s| s.map_commands.clone()),
        [format!("RUN_LUA_CODE(\"{code}\")")]
    );
    // double quotes cannot be passed through RUN_LUA_CODE
    assert!(actions::run_lua("print(\"x\")").is_err());
    assert!(actions::run_lua("a\nb").is_err());
}

#[test]
fn events_var_updates_and_subscriptions() {
    let m = Mock::start();
    let c = client_for(&m);
    assert!(
        c.call(Request::SubscribeEvent("RESEARCH_COMPLETED".into()))
            .unwrap()
            .success
    );
    assert!(
        c.call(Request::SubscribeVar {
            var: "MONEY".into(),
            player: Some(Player::P0)
        })
        .unwrap()
        .success
    );
    m.emit(
        "RESEARCH_COMPLETED",
        Some(json!({"player":0,"category":2,"kind":5,"kind_description":"SPELL","level_number":1})),
    );
    m.emit("NOT_SUBSCRIBED", None);
    let ev = wait_for(&c, 3, |e| {
        if let LiveEvent::Event(g) = e {
            Some(g.clone())
        } else {
            None
        }
    })
    .expect("an event");
    assert_eq!(ev.name, "RESEARCH_COMPLETED");
    assert_eq!(ev.data.unwrap()["kind_description"], "SPELL");
    m.set_var(0, "MONEY", 777);
    let ev = wait_for(&c, 3, |e| {
        if let LiveEvent::Event(g) = e {
            g.var.clone()
        } else {
            None
        }
    })
    .expect("var update");
    assert_eq!(
        (ev.player.as_str(), ev.name.as_str(), ev.value),
        ("PLAYER0", "MONEY", 777)
    );
    // unsubscribe: no more updates
    assert!(
        c.call(Request::UnsubscribeVar {
            var: "MONEY".into(),
            player: Some(Player::P0)
        })
        .unwrap()
        .success
    );
    m.set_var(0, "MONEY", 778);
    assert!(wait_for(&c, 1, |e| if let LiveEvent::Event(g) = e {
        g.var.clone()
    } else {
        None
    })
    .is_none());
    // NOT_SUBSCRIBED was never delivered
    assert!(c
        .poll()
        .iter()
        .all(|e| !matches!(e, LiveEvent::Event(g) if g.name == "NOT_SUBSCRIBED")));
}

#[test]
fn async_send_matches_replies_to_ids() {
    let m = Mock::start();
    let c = client_for(&m);
    m.set_var(0, "FLAG0", 11);
    m.set_var(0, "FLAG1", 22);
    let a = c.send(actions::read_var(&Player::P0, "FLAG0"));
    let b = c.send(actions::read_var(&Player::P0, "FLAG1"));
    let bad = c.send(Request::ReadVar {
        var: "NOPE".into(),
        player: None,
    });
    let mut got = std::collections::HashMap::new();
    wait_for(&c, 3, |e| {
        if let LiveEvent::Reply { id, result, .. } = e {
            got.insert(*id, result.clone().unwrap());
        }
        (got.len() == 3).then_some(())
    })
    .expect("three replies");
    assert_eq!(got[&a].int(), Some(11));
    assert_eq!(got[&b].int(), Some(22));
    assert_eq!(got[&bad].error, Some(ApiError::UnknownVar));
}

#[test]
fn game_absent_reconnect_and_resubscribe() {
    // no game: spawn never fails, requests fail fast
    let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut cfg = Config::localhost(port);
    cfg.reconnect_interval = Duration::from_millis(100);
    cfg.connect_timeout = Duration::from_millis(200);
    cfg.request_timeout = Duration::from_millis(800);
    let c = LiveClient::spawn(cfg);
    // the first failed attempt: ~immediate on Linux / macOS; Windows retries a refused localhost
    // connect until the connect timeout
    let end = Instant::now() + Duration::from_secs(3);
    while c.status().last_error.is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!c.is_connected());
    assert!(c.status().last_error.is_some());
    let t = Instant::now();
    assert_eq!(c.call(Request::GetKfxInfo), Err(ClientError::NotConnected));
    assert!(t.elapsed() < Duration::from_secs(2));
    // the game appears
    let m = Mock::start_on(port);
    assert!(c.wait_connected(Duration::from_secs(5)));
    assert!(
        c.call(Request::SubscribeEvent("GAME_SAVED".into()))
            .unwrap()
            .success
    );
    assert!(
        c.call(Request::SubscribeVar {
            var: "FLAG2".into(),
            player: Some(Player::P0)
        })
        .unwrap()
        .success
    );
    // ... and goes away
    m.stop();
    let end = Instant::now() + Duration::from_secs(3);
    while c.is_connected() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!c.is_connected(), "disconnect noticed");
    assert!(c
        .poll()
        .iter()
        .any(|e| matches!(e, LiveEvent::Disconnected { .. })));
    // ... and comes back: the client reconnects and subscribes again on its own
    let m = Mock::start_on(port);
    assert!(c.wait_connected(Duration::from_secs(5)));
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end && m.with(|s| s.sub_events.is_empty() || s.sub_vars.is_empty()) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        m.with(|s| (
            s.sub_events.clone(),
            s.sub_vars.iter().map(|v| v.1.clone()).collect::<Vec<_>>()
        )),
        (vec!["GAME_SAVED".to_string()], vec!["FLAG2".to_string()])
    );
    m.emit("GAME_SAVED", None);
    assert!(wait_for(&c, 3, |e| {
        matches!(e, LiveEvent::Event(g) if g.name == "GAME_SAVED").then_some(())
    })
    .is_some());
}

#[test]
fn timeout_second_client_and_clean_disconnect() {
    let m = Mock::start();
    let c = client_for(&m);
    // a second client is accepted and closed by the game
    let mut cfg = Config::localhost(m.port);
    cfg.reconnect_interval = Duration::from_secs(60);
    let c2 = LiveClient::spawn(cfg);
    std::thread::sleep(Duration::from_millis(500));
    assert!(m.with(|s| s.rejected) >= 1);
    assert!(!c2.is_connected() || c2.call(Request::GetKfxInfo).is_err());
    drop(c2);
    // a game that does not answer: the request times out, the next one still works
    m.with(|s| s.mute = true);
    assert_eq!(
        c.call(Request::GetCurrentGameInfo),
        Err(ClientError::Timeout)
    );
    m.with(|s| s.mute = false);
    assert!(c.call(Request::GetCurrentGameInfo).unwrap().success);
    assert!(c.status().timeouts >= 1);
    // clean disconnect: the game sees EOF (it logs "API connection closed" and drops the
    // subscriptions)
    assert!(
        c.call(Request::SubscribeEvent("WIN_GAME".into()))
            .unwrap()
            .success
    );
    let before = m.with(|s| s.closed_by_client);
    c.shutdown();
    let end = Instant::now() + Duration::from_secs(3);
    while m.with(|s| s.closed_by_client) == before && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(m.with(|s| s.closed_by_client), before + 1);
    assert!(m.with(|s| s.sub_events.is_empty()));
}

#[test]
fn panel_model_watch_console_and_filter() {
    let m = Mock::start();
    let c = client_for(&m);
    let mut p = LivePanel::new(c);
    m.set_var(0, "FLAG1", 5);
    p.add_watch(Player::P0, "flag1");
    p.add_watch(Player::P0, "FLAG1"); // duplicate ignored
    p.add_watch(Player::Index(2), "MONEY");
    assert_eq!(p.watches.len(), 2);
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end && p.values.values().any(|v| v.value.is_none()) {
        p.update();
        std::thread::sleep(Duration::from_millis(20));
    }
    let w1 = p.watches[0].clone();
    assert_eq!(p.values[&w1].value, Some(5));
    // a change is pushed
    m.set_var(0, "FLAG1", 6);
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end && p.values[&w1].value != Some(6) {
        p.update();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        (p.values[&w1].value, p.values[&w1].previous),
        (Some(6), Some(5))
    );
    assert!(p
        .log
        .iter()
        .any(|e| e.kind == LogKind::VarUpdate && e.text.contains("FLAG1 = 6")));
    // events are subscribed on connect and logged
    m.emit("GAME_SAVED", None);
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end && !p.log.iter().any(|e| e.tag == "GAME_SAVED") {
        p.update();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(p.log.iter().any(|e| e.tag == "GAME_SAVED"));
    assert_eq!(p.level.as_ref().map(|l| l.level_number), Some(1));
    // console: prefixes pick the mode, history works, errors explain themselves
    p.console_input = "!fps 40".into();
    p.submit_console();
    p.console_input = ">ADD_GOLD_TO_PLAYER(PLAYER0,10)".into();
    p.submit_console();
    p.console_input = "=print('x')".into();
    p.submit_console();
    p.console_mode = ConsoleMode::Console;
    p.console_input = "zoomto.subtilecoord 10 10".into();
    p.submit_console();
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end
        && !p
            .log
            .iter()
            .any(|e| e.kind == LogKind::Error && e.text.contains("-alex"))
    {
        p.update();
        std::thread::sleep(Duration::from_millis(20));
    }
    let consoles = m.with(|s| s.console_commands.clone());
    assert_eq!(consoles, ["fps 40", "zoomto.subtilecoord 10 10"]);
    let maps = m.with(|s| s.map_commands.clone());
    assert_eq!(
        maps,
        [
            "ADD_GOLD_TO_PLAYER(PLAYER0,10)",
            "RUN_LUA_CODE(\"print('x')\")"
        ]
    );
    assert!(
        p.log.iter().any(|e| e.kind == LogKind::Error
            && e.text.contains("FAILED_TO_EXECUTE_CONSOLE_COMMAND")
            && e.text.contains("-alex")),
        "{:?}",
        p.log.iter().map(|e| &e.text).collect::<Vec<_>>()
    );
    p.history_step(true);
    assert_eq!(p.console_input, "zoomto.subtilecoord 10 10");
    p.history_step(true);
    assert_eq!(p.console_input, "=print('x')");
    p.history_step(false);
    assert_eq!(p.console_input, "zoomto.subtilecoord 10 10");
    p.history_step(false);
    assert_eq!(p.console_input, "");
    // filter
    p.filter = "gold".into();
    assert!(p
        .filtered_log()
        .iter()
        .all(|e| e.tag.to_lowercase().contains("gold") || e.text.to_lowercase().contains("gold")));
    assert!(!p.filtered_log().is_empty());
    p.filter.clear();
    p.show.insert(LogKind::Error, false);
    assert!(p.filtered_log().iter().all(|e| e.kind != LogKind::Error));
    // remove a watch unsubscribes it
    p.remove_watch(&w1);
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end && m.with(|s| s.sub_vars.iter().any(|v| v.1 == "FLAG1")) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(m.with(|s| !s.sub_vars.iter().any(|v| v.1 == "FLAG1")));
}

#[test]
fn log_tail_and_thing_lines() {
    let dir = kfx_formats::testutil::scratch_dir("kfx-live/test-logtail");
    let path = dir.join("keeperfx.log");
    std::fs::write(&path, "old line\n").unwrap();
    let mut t = LogTail::new(&path);
    assert!(t.poll().is_empty(), "starts at the end");
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    write!(
        f,
        "[12] KFXLIVE THING Creature IMP 0 14 20\nnoise\n[13] KFXLIVE THING Creature DRAGON \
        1 40 41\npartial KFXLIVE THI"
    )
    .unwrap();
    let lines = t.poll();
    assert_eq!(lines.len(), 3, "the unfinished line waits");
    let things = parse_things(&lines);
    assert_eq!(things.len(), 2);
    assert_eq!(
        (
            things[0].model.as_str(),
            things[0].owner,
            things[0].stl_x,
            things[0].stl_y
        ),
        ("IMP", 0, 14, 20)
    );
    write!(
        f,
        "NG Object GOLD 5 1 2\nKFXLIVE THING Creature BROKEN x y\n"
    )
    .unwrap();
    let things = parse_things(&t.poll());
    assert_eq!(
        things.len(),
        1,
        "the completed line is a thing, the one with non numeric fields is not: {things:?}"
    );
    assert_eq!((things[0].model.as_str(), things[0].owner), ("GOLD", 5));
    // truncation (the game restarted) re-reads from the start
    std::fs::write(&path, "x\n").unwrap();
    assert_eq!(t.poll(), ["x"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn actions_build_the_documented_lines() {
    let p = Player::P0;
    assert_eq!(
        actions::add_gold(&p, 500),
        Request::MapCommand("ADD_GOLD_TO_PLAYER(PLAYER0,500)".into())
    );
    assert_eq!(
        actions::reveal_map(&p, (85, 85)),
        Request::MapCommand("REVEAL_MAP_RECT(PLAYER0,130,130,261,261)".into())
    );
    assert_eq!(
        actions::give_room(&p, "TREASURE").unwrap(),
        Request::MapCommand("ROOM_AVAILABLE(PLAYER0,TREASURE,1,1)".into())
    );
    assert!(
        actions::give_room(&p, "A,B)").is_err(),
        "no way to smuggle a second command in"
    );
    assert_eq!(
        actions::give_power(&Player::Index(2), "POWER_LIGHTNING").unwrap(),
        Request::MapCommand("MAGIC_AVAILABLE(PLAYER2,POWER_LIGHTNING,1,1)".into())
    );
    assert_eq!(
        actions::add_creature(&p, "DRAGON", "PLAYER0", 2, 5, 0).unwrap(),
        Request::MapCommand("ADD_CREATURE_TO_LEVEL(PLAYER0,DRAGON,PLAYER0,2,5,0)".into())
    );
    assert_eq!(
        actions::zoom_to_action_point(&p, 3),
        Request::MapCommand("ZOOM_TO_LOCATION(PLAYER0,3)".into())
    );
    assert_eq!(
        actions::zoom_to_subtile(10, 20),
        Request::ConsoleCommand {
            command: "zoomto.subtilecoord 10 20".into(),
            player: None
        }
    );
    assert_eq!(
        actions::set_speed(0),
        Request::ConsoleCommand {
            command: "fps 1".into(),
            player: None
        }
    );
    assert_eq!(actions::console("!pause"), actions::pause());
    assert!(
        actions::cheat_hint("lua print(1)").is_some() && actions::cheat_hint("pause").is_none()
    );
    // the Lua channel: the helper is defined once, calls are tiny, the pool use is counted
    let mut lua = LuaChannel::default();
    let first = lua.dump_things("Creature").unwrap();
    let second = lua.dump_things("Object").unwrap();
    assert_eq!((first.len(), second.len()), (2, 1));
    assert_eq!(
        second[0],
        Request::MapCommand("RUN_LUA_CODE(\"KfxLiveDump('Object')\")".into())
    );
    assert!(
        lua.pool_used < 500 && lua.pool_fraction() < 0.1,
        "{}",
        lua.pool_used
    );
    assert!(lua.dump_things("Creature'); os.exit() --").is_err());
    let e = lua.eval_int("6*7").unwrap();
    assert_eq!(
        e,
        Request::MapCommand("RUN_LUA_CODE(\"PLAYER0.FLAG7=(6*7)\")".into())
    );
    assert_eq!(
        lua.result_request(),
        Request::ReadVar {
            var: "FLAG7".into(),
            player: Some(Player::P0)
        }
    );
    let mut cheat = LuaChannel::default();
    cheat.use_console = true;
    assert!(matches!(
        &cheat.eval_int("1").unwrap(),
        Request::ConsoleCommand { command, .. } if command.starts_with("lua ")
    ));
    assert_eq!(cheat.pool_used, 0, "the console route costs no string pool");
}
