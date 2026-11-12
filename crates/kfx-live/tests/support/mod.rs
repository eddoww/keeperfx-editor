//! A mock of the game's API server that reproduces the framing and limits of `src/api.c`:
//! one client at a time (a second one is accepted and closed), one non-blocking `recv` of at most
//! 4095 bytes per tick, one trailing `\n` removed, objects cut by brace depth, replies of at most
//! 1023 bytes (`FAILED_TO_CREATE_JSON` otherwise), `INVALID_JSON_IN_PACKET` for an unterminated
//! object, responses with sorted members (`ack` first), `NOT_IN_LOCAL_GAME`, `GAME_IS_PAUSED`,
//! variable subscriptions compared once per tick.
// Shared by several test binaries; each uses a subset.
#![allow(dead_code)]
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
pub struct MockState {
    pub local_game: bool,
    pub paused: bool,
    pub turn: i64,
    /// (player index, var) -> value
    pub vars: HashMap<(i32, String), i64>,
    /// Everything the mock received (decoded objects as text), in order.
    pub received: Vec<String>,
    pub map_commands: Vec<String>,
    pub console_commands: Vec<String>,
    pub connections: u32,
    pub rejected: u32,
    pub closed_by_client: u32,
    pub pending_events: Vec<(String, Option<Value>)>,
    /// When true the mock reads requests but never answers (a game that hangs / is busy).
    pub mute: bool,
    pub sub_events: Vec<String>,
    pub sub_vars: Vec<(i32, String, i64)>,
    /// Swallow the next N complete replies (for timeout tests).
    pub kfx_version: String,
}

pub struct Mock {
    pub port: u16,
    pub state: Arc<Mutex<MockState>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

fn player_of(v: &Value) -> i32 {
    match v.get("player") {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) as i32,
        Some(Value::String(s)) => s
            .strip_prefix("PLAYER")
            .and_then(|n| n.parse().ok())
            .unwrap_or(-1),
        _ => 0,
    }
}

impl Mock {
    pub fn start() -> Mock {
        Self::start_on(0)
    }

    pub fn start_on(port: u16) -> Mock {
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind mock");
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(MockState {
            local_game: true,
            kfx_version: "1.3.1-mock".into(),
            ..Default::default()
        }));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        let join = std::thread::spawn(move || serve(listener, st, sp));
        Mock {
            port,
            state,
            stop,
            join: Some(join),
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    pub fn set_var(&self, player: i32, var: &str, v: i64) {
        self.with(|s| {
            s.vars.insert((player, var.to_string()), v);
        });
    }

    pub fn emit(&self, name: &str, data: Option<Value>) {
        self.with(|s| s.pending_events.push((name.to_string(), data)));
    }

    pub fn stop(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn dump(v: &Value) -> String {
    serde_json::to_string(v).unwrap()
}

/// `api_ok` / `api_err` / `api_return_data`: the game's JSON writer sorts members by key (`ack`,
/// `data`/`error`, `success`), which is what serde_json's default map does; 1023 byte limit.
fn reply(ack: Option<&Value>, success: bool, key: Option<(&str, Value)>) -> String {
    let mut m = Map::new();
    if let Some(a) = ack {
        m.insert("ack".into(), a.clone());
    }
    m.insert("success".into(), json!(success));
    if let Some((k, v)) = key {
        m.insert(k.into(), v);
    }
    let s = dump(&Value::Object(m));
    if s.len() > 1023 {
        return reply(ack, false, Some(("error", json!("FAILED_TO_CREATE_JSON"))));
    }
    s
}

fn err(ack: Option<&Value>, code: &str) -> String {
    reply(ack, false, Some(("error", json!(code))))
}

fn handle(obj: &str, st: &mut MockState, out: &mut Vec<String>) {
    st.received.push(obj.to_string());
    let Ok(v) = serde_json::from_str::<Value>(obj) else {
        out.push(err(None, "INVALID_JSON"));
        return;
    };
    if !v.is_object() {
        out.push(err(None, "INVALID_JSON_OBJECT"));
        return;
    }
    let ack = v.get("ack");
    let Some(action) = v.get("action").and_then(Value::as_str) else {
        out.push(err(ack, "MISSING_ACTION"));
        return;
    };
    let action = action.to_ascii_lowercase();
    let player = player_of(&v);
    let var = v.get("var").and_then(Value::as_str).unwrap_or("");
    let known_var = |n: &str| {
        n == "MONEY"
            || n == "GAME_TURN"
            || n.starts_with("FLAG")
            || n.starts_with("TIMER")
            || n.starts_with("CAMPAIGN_FLAG")
    };
    match action.as_str() {
        "get_kfx_info" => {
            return out.push(reply(
                ack,
                true,
                Some(("data", json!({"kfx_version": st.kfx_version}))),
            ))
        }
        "subscribe_var" | "unsubscribe_var" => {
            if var.is_empty() {
                return out.push(err(ack, "MISSING_VAR"));
            }
            if !known_var(var) {
                return out.push(err(ack, "UNKNOWN_VAR"));
            }
            let cur = *st.vars.get(&(player, var.to_string())).unwrap_or(&0);
            st.sub_vars.retain(|s| !(s.0 == player && s.1 == var));
            if action == "subscribe_var" {
                st.sub_vars.push((player, var.to_string(), cur));
            }
            return out.push(reply(ack, true, None));
        }
        "subscribe_event" | "unsubscribe_event" => {
            let Some(e) = v
                .get("event")
                .and_then(Value::as_str)
                .filter(|e| !e.is_empty())
            else {
                return out.push(err(ack, "MISSING_EVENT"));
            };
            if e.len() > 256 {
                return out.push(err(ack, "STRING_TOO_LONG"));
            }
            st.sub_events.retain(|x| x != e);
            if action == "subscribe_event" {
                st.sub_events.push(e.to_string());
            }
            return out.push(reply(ack, true, None));
        }
        "unsubscribe_all" => {
            st.sub_events.clear();
            st.sub_vars.clear();
            return out.push(reply(ack, true, None));
        }
        _ => {}
    }
    if !st.local_game {
        return out.push(err(ack, "NOT_IN_LOCAL_GAME"));
    }
    match action.as_str() {
        "map_command" | "console_command" => {
            if st.paused {
                return out.push(err(ack, "GAME_IS_PAUSED"));
            }
            let Some(c) = v.get("command").and_then(Value::as_str) else {
                return out.push(err(ack, "MISSING_COMMAND"));
            };
            if action == "map_command" {
                st.map_commands.push(c.to_string());
                if let Some(rest) = c
                    .strip_prefix("ADD_GOLD_TO_PLAYER(PLAYER0,")
                    .and_then(|r| r.strip_suffix(')'))
                {
                    let n: i64 = rest.parse().unwrap_or(0);
                    *st.vars.entry((0, "MONEY".into())).or_default() += n;
                }
                if c.starts_with("BOGUS") {
                    return out.push(err(ack, "FAILED_TO_EXECUTE_MAP_COMMAND"));
                }
                out.push(reply(ack, true, None));
            } else {
                let c = c.strip_prefix('!').unwrap_or(c);
                st.console_commands.push(c.to_string());
                if c == "pause" {
                    st.paused = true;
                }
                if c.starts_with("zoomto.") {
                    return out.push(err(ack, "FAILED_TO_EXECUTE_CONSOLE_COMMAND"));
                }
                out.push(reply(ack, true, None));
            }
        }
        "read_var" => {
            if var.is_empty() {
                return out.push(err(ack, "MISSING_VAR"));
            }
            if !known_var(var) {
                return out.push(err(ack, "UNKNOWN_VAR"));
            }
            let val = if var == "GAME_TURN" {
                st.turn
            } else {
                *st.vars.get(&(player, var.to_string())).unwrap_or(&0)
            };
            out.push(reply(ack, true, Some(("data", json!(val)))));
        }
        "set_var" => {
            if var.is_empty() {
                return out.push(err(ack, "MISSING_VAR"));
            }
            if !known_var(var) {
                return out.push(err(ack, "UNKNOWN_VAR"));
            }
            if !(var.starts_with("FLAG") || var.starts_with("CAMPAIGN_FLAG")) {
                return out.push(err(ack, "UNABLE_TO_SET_VAR"));
            }
            let Some(n) = v.get("value").and_then(Value::as_i64) else {
                return out.push(err(ack, "VALUE_MUST_BE_INT"));
            };
            st.vars.insert((player, var.to_string()), n);
            out.push(reply(ack, true, None));
        }
        "get_current_game_info" => out.push(reply(
            ack,
            true,
            Some(("data", json!({"game_turn": st.turn}))),
        )),
        "get_level_info" | "get_map_info" => out.push(reply(
            ack,
            true,
            Some((
                "data",
                json!({
                    "level_name": "Mock",
                    "level_number": 1,
                    "players": 1,
                    "mapsize_x": 85,
                    "mapsize_y": 85,
                    "is_multiplayer": false,
                    "campaign": {
                        "campaign_name": "c",
                        "campaign_display_name": "C",
                        "campaign_fname": "c",
                        "is_map_pack": false
                    }
                }),
            )),
        )),
        "get_all_player_flags" => {
            // the C reads the flags of the request's player for every entry (src/api.c:1375)
            let mut all = Map::new();
            for p in [
                "PLAYER0",
                "PLAYER1",
                "PLAYER2",
                "PLAYER3",
                "PLAYER4",
                "PLAYER5",
                "PLAYER6",
                "PLAYER_GOOD",
                "PLAYER_NEUTRAL",
            ] {
                let mut f = Map::new();
                for i in 0..8 {
                    f.insert(
                        format!("FLAG{i}"),
                        json!(st
                            .vars
                            .get(&(player, format!("FLAG{i}")))
                            .copied()
                            .unwrap_or(0)),
                    );
                }
                all.insert(p.into(), Value::Object(f));
            }
            out.push(reply(ack, true, Some(("data", Value::Object(all)))));
        }
        _ => out.push(err(ack, "UNKNOWN_ACTION")),
    }
}

fn serve(
    listener: TcpListener,
    state: Arc<Mutex<MockState>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    let mut active: Option<TcpStream> = None;
    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(4)); // one "game turn"
                                                      // accept
        if let Ok((s, _)) = listener.accept() {
            let mut st = state.lock().unwrap();
            if active.is_some() {
                st.rejected += 1; // accepted and closed
                drop(s);
            } else {
                s.set_nonblocking(true).unwrap();
                st.connections += 1;
                active = Some(s);
            }
        }
        let mut st = state.lock().unwrap();
        st.turn += 1;
        let Some(sock) = active.as_mut() else {
            continue;
        };
        let mut buf = [0u8; 4095];
        let mut out: Vec<String> = vec![];
        let mut closed = false;
        match sock.read(&mut buf) {
            Ok(0) => closed = true,
            Ok(n) => {
                // strlen: stop at NUL; strip one trailing newline
                let mut text = &buf[..n];
                if let Some(p) = text.iter().position(|&b| b == 0) {
                    text = &text[..p];
                }
                if text.last() == Some(&b'\n') {
                    text = &text[..text.len() - 1];
                }
                let (mut depth, mut start) = (0i32, usize::MAX);
                let mut objs = vec![];
                for (i, &b) in text.iter().enumerate() {
                    if b == b'{' {
                        if depth == 0 {
                            start = i;
                        }
                        depth += 1;
                    } else if b == b'}' {
                        depth -= 1;
                        if depth == 0 && start != usize::MAX {
                            objs.push(String::from_utf8_lossy(&text[start..=i]).to_string());
                            start = usize::MAX;
                        }
                    }
                }
                for o in objs {
                    if st.mute {
                        st.received.push(o);
                    } else {
                        handle(&o, &mut st, &mut out);
                    }
                }
                if depth > 0 {
                    out.push(err(None, "INVALID_JSON_IN_PACKET"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => closed = true,
        }
        if closed {
            st.closed_by_client += 1;
            st.sub_events.clear();
            st.sub_vars.clear();
            active = None;
            continue;
        }
        // events (4096 byte buffer), only when subscribed
        let evs: Vec<_> = st.pending_events.drain(..).collect();
        for (name, data) in evs {
            if st.sub_events.contains(&name) && !st.mute {
                let mut m = Map::new();
                m.insert("event".into(), json!(name));
                if let Some(d) = data {
                    m.insert("data".into(), d);
                }
                out.push(dump(&Value::Object(m)));
            }
        }
        // variable subscriptions (api_check_var_update)
        let mut updates = vec![];
        let turn = st.turn;
        let vars = st.vars.clone();
        for s in st.sub_vars.iter_mut() {
            let cur = if s.1 == "GAME_TURN" {
                turn
            } else {
                *vars.get(&(s.0, s.1.clone())).unwrap_or(&0)
            };
            if cur != s.2 {
                s.2 = cur;
                updates.push(json!({
                    "event": "VAR_UPDATE",
                    "var": {"player": format!("PLAYER{}", s.0), "name": s.1, "value": cur}
                }));
            }
        }
        if !st.mute {
            out.extend(updates.iter().map(dump));
        }
        drop(st);
        for line in out {
            let mut l = line.into_bytes();
            l.push(b'\n');
            if let Some(s) = active.as_mut() {
                let _ = s.write_all(&l);
            }
        }
    }
}
