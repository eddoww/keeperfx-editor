//! State of the live panel, independent of any GUI: connection, event log with filter, watch table,
//! console with history, quick actions, thing positions. [`crate::ui`] draws it.
use crate::actions::{self, LuaChannel};
use crate::client::{ClientError, LiveClient, LiveEvent, RequestId};
use crate::logtail::{self, LiveThing, LogTail};
use crate::proto::{ApiError, LevelInfo, Player, Reply, Request};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

/// Events the game can send (see [`crate::proto`]); subscribed on connect.
pub const KNOWN_EVENTS: &[&str] = &[
    "GAME_STARTED",
    "GAME_ENDED",
    "CAMPAIGN_LOADED",
    "GAME_SAVED",
    "GAME_LOADED",
    "WIN_GAME",
    "LOSE_GAME",
    "SPECIAL_ACTIVATED",
    "RESEARCH_COMPLETED",
    "MANUFACTURE_COMPLETED",
    "ACTION_POINT",
];

/// Variables worth watching: the script flags and timers, gold and turn counters.
pub const COMMON_VARS: &[&str] = &[
    "MONEY",
    "GAME_TURN",
    "TOTAL_CREATURES",
    "TOTAL_DIGGERS",
    "FLAG0",
    "FLAG1",
    "FLAG2",
    "FLAG3",
    "FLAG4",
    "FLAG5",
    "FLAG6",
    "FLAG7",
    "TIMER0",
    "TIMER1",
    "TIMER2",
    "TIMER3",
    "TIMER4",
    "TIMER5",
    "TIMER6",
    "TIMER7",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogKind {
    Info,
    Event,
    VarUpdate,
    Request,
    Reply,
    Error,
    Wire,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub when: SystemTime,
    pub kind: LogKind,
    /// Event name / request action, for filtering.
    pub tag: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Watch {
    pub player: Player,
    pub var: String,
}

#[derive(Clone, Debug)]
pub struct WatchValue {
    pub value: Option<i64>,
    pub previous: Option<i64>,
    pub updated: Option<Instant>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleMode {
    /// A game console command (`pause`, `fps 40`, `game.save 1`).
    Console,
    /// One DK script command line (`ADD_GOLD_TO_PLAYER(PLAYER0,500)`).
    Script,
    /// Lua through `RUN_LUA_CODE` (or the console `lua` with cheats).
    Lua,
}

#[derive(Clone, Debug)]
enum Pending {
    Watch(Watch),
    Console(String),
    Level,
    Quick(String),
    Things,
}

pub struct LivePanel {
    pub client: LiveClient,
    pub log: VecDeque<LogEntry>,
    pub log_cap: usize,
    /// Substring filter on tag and text.
    pub filter: String,
    pub show: HashMap<LogKind, bool>,
    pub watches: Vec<Watch>,
    pub values: HashMap<Watch, WatchValue>,
    pub players: Vec<Player>,
    pub level: Option<LevelInfo>,
    pub console_input: String,
    pub console_mode: ConsoleMode,
    pub history: Vec<String>,
    history_pos: Option<usize>,
    pub quick_player: Player,
    pub lua: LuaChannel,
    pub things: Vec<LiveThing>,
    pub log_tail: Option<LogTail>,
    pub log_path: Option<PathBuf>,
    pub events: Vec<String>,
    pending: HashMap<RequestId, Pending>,
    connected_before: bool,
}

impl LivePanel {
    pub fn new(client: LiveClient) -> LivePanel {
        let mut show = HashMap::new();
        for k in [
            LogKind::Info,
            LogKind::Event,
            LogKind::VarUpdate,
            LogKind::Request,
            LogKind::Reply,
            LogKind::Error,
        ] {
            show.insert(k, true);
        }
        show.insert(LogKind::Wire, false);
        LivePanel {
            client,
            log: VecDeque::new(),
            log_cap: 2000,
            filter: String::new(),
            show,
            watches: vec![],
            values: HashMap::new(),
            players: vec![Player::Index(0)],
            level: None,
            console_input: String::new(),
            console_mode: ConsoleMode::Console,
            history: vec![],
            history_pos: None,
            quick_player: Player::Index(0),
            lua: LuaChannel::default(),
            things: vec![],
            log_tail: None,
            log_path: None,
            events: KNOWN_EVENTS.iter().map(|s| s.to_string()).collect(),
            pending: HashMap::new(),
            connected_before: false,
        }
    }

    pub fn push_log(&mut self, kind: LogKind, tag: impl Into<String>, text: impl Into<String>) {
        self.log.push_back(LogEntry {
            when: SystemTime::now(),
            kind,
            tag: tag.into(),
            text: text.into(),
        });
        while self.log.len() > self.log_cap {
            self.log.pop_front();
        }
    }

    /// Entries that pass the kind toggles and the text filter.
    pub fn filtered_log(&self) -> Vec<&LogEntry> {
        let q = self.filter.trim().to_ascii_lowercase();
        self.log
            .iter()
            .filter(|e| self.show.get(&e.kind).copied().unwrap_or(true))
            .filter(|e| {
                q.is_empty()
                    || e.tag.to_ascii_lowercase().contains(&q)
                    || e.text.to_ascii_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn add_watch(&mut self, player: Player, var: &str) {
        let w = Watch {
            player: player.clone(),
            var: var.trim().to_ascii_uppercase(),
        };
        if w.var.is_empty() || self.watches.contains(&w) {
            return;
        }
        if !self.players.contains(&player) {
            self.players.push(player);
        }
        self.values.insert(
            w.clone(),
            WatchValue {
                value: None,
                previous: None,
                updated: None,
                error: None,
            },
        );
        self.watches.push(w.clone());
        if self.client.is_connected() {
            self.subscribe_watch(&w);
        }
    }

    pub fn remove_watch(&mut self, w: &Watch) {
        self.watches.retain(|x| x != w);
        self.values.remove(w);
        if self.client.is_connected() {
            self.client.send(Request::UnsubscribeVar {
                var: w.var.clone(),
                player: Some(w.player.clone()),
            });
        }
    }

    fn subscribe_watch(&mut self, w: &Watch) {
        self.client.send(Request::SubscribeVar {
            var: w.var.clone(),
            player: Some(w.player.clone()),
        });
        let id = self.client.send(actions::read_var(&w.player, &w.var));
        self.pending.insert(id, Pending::Watch(w.clone()));
    }

    /// Read every watched variable again.
    pub fn refresh_watches(&mut self) {
        for w in self.watches.clone() {
            let id = self.client.send(actions::read_var(&w.player, &w.var));
            self.pending.insert(id, Pending::Watch(w));
        }
    }

    /// Send the text of the console line according to the mode (history, `!`/`=` prefixes switch
    /// the mode for that line).
    pub fn submit_console(&mut self) {
        let text = self.console_input.trim().to_string();
        self.console_input.clear();
        self.history_pos = None;
        if text.is_empty() {
            return;
        }
        if self.history.last() != Some(&text) {
            self.history.push(text.clone());
        }
        let (mode, body) = match text.chars().next() {
            Some('!') => (ConsoleMode::Console, text[1..].trim().to_string()),
            Some('=') => (ConsoleMode::Lua, text[1..].trim().to_string()),
            Some('>') => (ConsoleMode::Script, text[1..].trim().to_string()),
            _ => (self.console_mode, text.clone()),
        };
        self.run(mode, &body);
    }

    /// Run a line in a mode.
    pub fn run(&mut self, mode: ConsoleMode, body: &str) {
        let req = match mode {
            ConsoleMode::Console => Ok(actions::console(body)),
            ConsoleMode::Script => Ok(actions::script(body)),
            ConsoleMode::Lua => {
                if self.lua.use_console {
                    Ok(actions::run_lua_console(body))
                } else {
                    actions::run_lua(body)
                }
            }
        };
        match req {
            Ok(r) => {
                let label = body.to_string();
                self.push_log(LogKind::Request, r.action().to_string(), label.clone());
                let id = self.client.send(r);
                self.pending.insert(id, Pending::Console(label));
            }
            Err(e) => self.push_log(LogKind::Error, "console", e),
        }
    }

    /// Previous / next history line (arrow keys).
    pub fn history_step(&mut self, up: bool) {
        if self.history.is_empty() {
            return;
        }
        let n = self.history.len();
        let pos = match (self.history_pos, up) {
            (None, true) => n - 1,
            (None, false) => return,
            (Some(p), true) => p.saturating_sub(1),
            (Some(p), false) => {
                if p + 1 >= n {
                    self.history_pos = None;
                    self.console_input.clear();
                    return;
                }
                p + 1
            }
        };
        self.history_pos = Some(pos);
        self.console_input = self.history[pos].clone();
    }

    /// Send a quick action request and note it.
    pub fn quick(&mut self, label: &str, req: Request) {
        self.push_log(
            LogKind::Request,
            req.action().to_string(),
            label.to_string(),
        );
        let id = self.client.send(req);
        self.pending.insert(id, Pending::Quick(label.to_string()));
    }

    /// Ask the game to list the things of a class into its log (see [`crate::logtail`]); they show
    /// up in [`LivePanel::things`] once the log line arrives and [`LivePanel::update`] reads it.
    /// Needs `log_path`.
    pub fn request_things(&mut self, class: &str) {
        match self.lua.dump_things(class) {
            Ok(reqs) => {
                for r in reqs {
                    let id = self.client.send(r);
                    self.pending.insert(id, Pending::Things);
                }
            }
            Err(e) => self.push_log(LogKind::Error, "things", e),
        }
    }

    pub fn set_log_path(&mut self, path: Option<PathBuf>) {
        self.log_tail = path.as_ref().map(LogTail::new);
        self.log_path = path;
    }

    /// Handle everything that arrived; call once per UI frame.
    pub fn update(&mut self) {
        for ev in self.client.poll() {
            self.handle(ev);
        }
        if let Some(t) = &mut self.log_tail {
            let lines = t.poll();
            if lines.iter().any(|l| l.contains(logtail::MARKER)) {
                let new = logtail::parse_things(&lines);
                if !new.is_empty() {
                    // a dump replaces the previous list of its class
                    let classes: Vec<String> = new.iter().map(|t| t.class.clone()).collect();
                    self.things.retain(|t| !classes.contains(&t.class));
                    self.things.extend(new);
                }
            }
        }
    }

    fn handle(&mut self, ev: LiveEvent) {
        match ev {
            LiveEvent::Connected { kfx_version } => {
                self.push_log(
                    LogKind::Info,
                    "connected",
                    format!(
                        "connected, KeeperFX {}",
                        kfx_version.as_deref().unwrap_or("?")
                    ),
                );
                self.connected_before = true;
                let id = self.client.send(Request::GetLevelInfo);
                self.pending.insert(id, Pending::Level);
                for e in self.events.clone() {
                    self.client.send(Request::SubscribeEvent(e));
                }
                // subscribing twice is harmless ("also true if already subscribed"); the client
                // also re-subscribes on its own
                for w in self.watches.clone() {
                    self.subscribe_watch(&w);
                }
            }
            LiveEvent::Disconnected { reason } => {
                self.push_log(
                    LogKind::Info,
                    "disconnected",
                    format!("disconnected: {reason}"),
                );
                self.pending.clear();
            }
            LiveEvent::Event(e) => {
                if let Some(v) = &e.var {
                    let player = if let Some(n) = v
                        .player
                        .strip_prefix("PLAYER")
                        .and_then(|n| n.parse::<i32>().ok())
                    {
                        Player::Index(n)
                    } else {
                        Player::Name(v.player.clone())
                    };
                    let w = Watch {
                        player,
                        var: v.name.clone(),
                    };
                    if let Some(val) = self.values.get_mut(&w) {
                        // The same value can arrive twice (a pushed update and a poll):
                        // only a real change moves the old value to `previous`.
                        if val.value != Some(v.value) {
                            val.previous = val.value;
                        }
                        val.value = Some(v.value);
                        val.updated = Some(Instant::now());
                        val.error = None;
                    }
                    self.push_log(
                        LogKind::VarUpdate,
                        "VAR_UPDATE",
                        format!("{} {} = {}", v.player, v.name, v.value),
                    );
                } else {
                    let text = e.data.as_ref().map(|d| d.to_string()).unwrap_or_default();
                    self.push_log(LogKind::Event, e.name.clone(), text);
                }
            }
            LiveEvent::Reply {
                id,
                request,
                result,
            } => self.handle_reply(id, request, result),
            LiveEvent::LateReply(r) => self.push_log(
                LogKind::Reply,
                "late",
                match &r.data {
                    Some(d) => format!("late reply: {d}"),
                    None => "late reply".to_string(),
                },
            ),
            LiveEvent::Garbage(t) => self.push_log(LogKind::Error, "garbage", t),
            LiveEvent::Wire { sent, text } => {
                self.push_log(LogKind::Wire, if sent { "->" } else { "<-" }, text)
            }
        }
    }

    fn handle_reply(
        &mut self,
        id: RequestId,
        request: Request,
        result: Result<Reply, ClientError>,
    ) {
        let pending = self.pending.remove(&id);
        let tag = request.action().to_string();
        match result {
            Err(e) => {
                if let Some(Pending::Watch(w)) = &pending {
                    if let Some(v) = self.values.get_mut(w) {
                        v.error = Some(e.to_string());
                    }
                }
                self.push_log(LogKind::Error, tag, e.to_string());
            }
            Ok(r) => {
                if !r.success {
                    let err = r.error.clone().unwrap_or(ApiError::Other("?".into()));
                    let mut text = err.code().to_string();
                    let ex = err.explain();
                    if !ex.is_empty() {
                        text = format!("{text}: {ex}");
                    }
                    if let (
                        ApiError::FailedToExecuteConsoleCommand,
                        Request::ConsoleCommand { command, .. },
                    ) = (&err, &request)
                    {
                        if let Some(h) = actions::cheat_hint(command) {
                            text = format!("{text} ({h})");
                        }
                    }
                    if let Some(Pending::Watch(w)) = &pending {
                        if let Some(v) = self.values.get_mut(w) {
                            v.error = Some(err.code().to_string());
                        }
                    }
                    self.push_log(LogKind::Error, tag, text);
                    return;
                }
                match pending {
                    Some(Pending::Watch(w)) => {
                        if let Some(v) = self.values.get_mut(&w) {
                            let new = r.int();
                            if v.value != new {
                                v.previous = v.value;
                            }
                            v.value = new;
                            v.updated = Some(Instant::now());
                            v.error = None;
                        }
                    }
                    Some(Pending::Level) => {
                        self.level = r.data.as_ref().and_then(LevelInfo::from_value);
                        if let Some(l) = &self.level {
                            let name = match l.level_name.as_deref() {
                                Some(n) if !n.is_empty() => format!(" \"{n}\""),
                                _ => String::new(),
                            };
                            self.push_log(
                                LogKind::Info,
                                "level",
                                format!(
                                    "level {}{name} ({}x{} slabs)",
                                    l.level_number, l.mapsize_x, l.mapsize_y
                                ),
                            );
                        }
                    }
                    Some(Pending::Console(t)) | Some(Pending::Quick(t)) => {
                        let extra = r
                            .data
                            .as_ref()
                            .map(|d| format!(" -> {d}"))
                            .unwrap_or_default();
                        self.push_log(LogKind::Reply, tag, format!("ok: {t}{extra}"));
                    }
                    Some(Pending::Things) | None => {}
                }
            }
        }
    }

    /// Time since the last connect, for the status line.
    pub fn uptime(&self) -> Option<Duration> {
        self.client.status().connected_since.map(|t| t.elapsed())
    }
}
