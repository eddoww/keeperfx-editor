//! Live link with a running game (`kfx-live`): the game's TCP JSON API (`API_ENABLED` /
//! `API_PORT`), the Live link panel (events, watched flags / timers / variables, console, quick
//! actions, in-game positions), the in-game creature positions drawn on the map, and moving the
//! game camera to the editor's selection.
//!
//! Play / Save and play turn the API on for that game only when the preference asks for it: the
//! game reads `-cfg KEY=VALUE` lines after `keeperfx.cfg` and the mods' copies
//! (`main.cpp:1968-1985`, `src/config_keeperfx.c:1301-1302`), so the editor passes
//! `-cfg API_ENABLED=ON -cfg API_PORT=<port>` and never edits the user's `keeperfx.cfg`.
use super::*;
use crate::playtest::Launch;
use kfx_live::actions;
use kfx_live::client::{Config, LiveClient};
use kfx_live::model::{LivePanel, LogKind, Watch, WatchValue};
use kfx_live::proto::Player;
use kfx_prefab::i18n::{trd, trf, tx, Tx};
use std::net::ToSocketAddrs;

/// What the panel says the API cannot do (from `kfx_live::actions`).
pub const LIMITS: &[Tx] = &[
    tx(
        "kfx_editor.an_edited_script_cannot_be_pushed_into_a_running",
        "An edited script cannot be pushed into a running game: save it and restart \
        the level to run a changed mapNNNNN.txt.",
    ),
    tx(
        "kfx_editor.the_game_camera_goes_to_action_points_hero_gates",
        "The game camera goes to action points, hero gates and hearts without \
        cheats; any other place needs the game started with -alex.",
    ),
    tx(
        "kfx_editor.the_api_refuses_commands_while_the_game_is_pause",
        "The API refuses commands while the game is paused, so it cannot unpause it.",
    ),
    tx(
        "kfx_editor.only_flags_and_a_few_counters_can_be_written_oth",
        "Only flags and a few counters can be written; other script variables are \
        read-only.",
    ),
    tx(
        "kfx_editor.positions_come_from_lua_printing_into_keeperfx_l",
        "Positions come from Lua printing into keeperfx.log; the first refresh uses \
        about 200 bytes of the level's 8192 byte script string pool and each later one about 25 \
        (none through the console with cheats).",
    ),
    tx(
        "kfx_editor.the_game_listens_on_127_0_0_1_only_for_one_clien",
        "The game listens on 127.0.0.1 only, for one client at a time.",
    ),
];

/// A connection and its panel.
pub struct LiveConn {
    pub panel: LivePanel,
    pub ui: kfx_live::ui::UiState,
    /// The address connected to.
    pub addr: String,
    /// `keeperfx.log` of the launched game: followed once the game answers (it rewrites the log
    /// when it starts).
    pending_log: Option<PathBuf>,
    last_refresh: Option<Instant>,
}

/// State of the live link.
pub struct Live {
    pub conn: Option<LiveConn>,
    /// Text of the address field.
    pub addr: String,
    /// Refresh the creature positions every 10 s while connected.
    pub auto_refresh: bool,
    pub message: Option<String>,
}

impl Default for Live {
    fn default() -> Self {
        Live {
            conn: None,
            addr: String::new(),
            auto_refresh: false,
            message: None,
        }
    }
}

fn parse_addr(s: &str, default_port: u16) -> Result<std::net::SocketAddr, String> {
    let s = s.trim();
    let s = if s.is_empty() {
        format!("127.0.0.1:{default_port}")
    } else if s.contains(':') {
        s.to_string()
    } else {
        format!("{s}:{default_port}")
    };
    s.to_socket_addrs()
        .map_err(|e| format!("{s}: {e}"))?
        .next()
        .ok_or_else(|| format!("{s}: no address"))
}

/// The `-cfg` arguments that turn the game's API on.
pub fn api_args(port: u16) -> [String; 4] {
    [
        "-cfg".into(),
        "API_ENABLED=ON".into(),
        "-cfg".into(),
        format!("API_PORT={port}"),
    ]
}

impl EditorApp {
    /// Connect the panel to `addr` (`host:port`, a bare port or empty = 127.0.0.1 and the port of
    /// the preferences). Replaces an existing connection to another address.
    pub fn live_connect(&mut self, addr: &str) -> Result<(), String> {
        let sa = parse_addr(addr, self.settings.audiolive.port)?;
        let text = sa.to_string();
        if self.live.conn.as_ref().is_some_and(|c| c.addr == text) {
            return Ok(());
        }
        self.live_disconnect();
        let mut cfg = Config::localhost(sa.port());
        cfg.addr = sa;
        let mut panel = LivePanel::new(LiveClient::spawn(cfg));
        // (GAME_TURN would report a change every turn)
        for v in ["MONEY", "FLAG0", "TOTAL_CREATURES"] {
            panel.add_watch(Player::P0, v);
        }
        self.live.conn = Some(LiveConn {
            panel,
            ui: Default::default(),
            addr: text.clone(),
            pending_log: None,
            last_refresh: None,
        });
        self.live.addr = text;
        self.settings.audiolive.live_open = true;
        Ok(())
    }

    pub fn live_disconnect(&mut self) {
        if let Some(c) = self.live.conn.take() {
            c.panel.client.shutdown();
        }
    }

    /// The launch with the API arguments added when the preference asks for them.
    pub fn live_prepare_launch(&self, mut l: Launch) -> Launch {
        if self.settings.audiolive.api_on_play
            && !l
                .args
                .iter()
                .any(|a| a.to_ascii_uppercase().starts_with("API_ENABLED"))
        {
            l.args.extend(api_args(self.settings.audiolive.port));
        }
        l
    }

    /// The game was started: connect to it (when its API is on and the preference says so).
    pub fn live_game_launched(&mut self, l: &Launch) {
        if !l
            .args
            .iter()
            .any(|a| a.eq_ignore_ascii_case("API_ENABLED=ON"))
            || !self.settings.audiolive.auto_connect
        {
            return;
        }
        let port = l
            .args
            .iter()
            .find_map(|a| {
                a.strip_prefix("API_PORT=")
                    .and_then(|p| p.parse::<u16>().ok())
            })
            .unwrap_or(self.settings.audiolive.port);
        match self.live_connect(&format!("127.0.0.1:{port}")) {
            Ok(()) => {
                if let Some(c) = self.live.conn.as_mut() {
                    c.pending_log = Some(l.cwd.join("keeperfx.log"));
                    c.panel.push_log(
                        LogKind::Info,
                        "editor",
                        trf(
                            "kfx_editor.game_started_with_the_api_on_port_port_connectin",
                            "game started with the API on port {port}: connecting",
                            &[("port", &port)],
                        ),
                    );
                }
            }
            Err(e) => self.live.message = Some(e),
        }
    }

    /// Per frame: drain the client when the panel is hidden, follow the game log once connected,
    /// auto refresh of the positions.
    pub(super) fn live_tick(&mut self) {
        let auto = self.live.auto_refresh;
        let Some(c) = self.live.conn.as_mut() else {
            return;
        };
        if !self.settings.audiolive.live_open {
            c.panel.update();
        }
        if c.panel.client.is_connected() {
            if let Some(p) = c.pending_log.take() {
                c.panel.set_log_path(Some(p.clone()));
                c.ui.log_path = p.display().to_string();
            }
            let due = c
                .last_refresh
                .is_none_or(|t| t.elapsed() > Duration::from_secs(10));
            if auto && due && c.panel.lua.pool_used < 6000 {
                c.panel.request_things("Creature");
                c.last_refresh = Some(Instant::now());
            }
        }
        self.ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Ask the game for the creature positions (Lua into the game log).
    pub fn live_refresh_positions(&mut self) {
        if let Some(c) = self.live.conn.as_mut() {
            if c.panel.log_path.is_none() {
                if let Some(d) = &self.game_log.game_dir {
                    c.panel.set_log_path(Some(d.join("keeperfx.log")));
                    c.ui.log_path = d.join("keeperfx.log").display().to_string();
                }
            }
            c.panel.request_things("Creature");
            c.last_refresh = Some(Instant::now());
        }
    }

    /// The game camera to the selected action point or hero gate (`ZOOM_TO_LOCATION`, no cheats),
    /// else to the player's heart.
    pub fn live_camera_to_selection(&mut self) -> Result<String, String> {
        let d = self.doc.as_ref().ok_or("no map")?;
        let mut loc: Option<(i32, String)> = None;
        for it in &self.selected {
            match it {
                ItemRef::ActionPoint(i) => {
                    if let Some(n) = d.doc.action_points.items.get(*i).and_then(|a| a.number) {
                        loc = Some((
                            n as i32,
                            trf(
                                "kfx_editor.action_point_n",
                                "action point {n}",
                                &[("n", &n)],
                            ),
                        ));
                        break;
                    }
                }
                ItemRef::Thing(i) => {
                    if let Some(n) = d
                        .doc
                        .things
                        .items
                        .get(*i)
                        .filter(|t| kfx_map::things::is_hero_gate(&d.gd, t))
                        .and_then(|t| t.herogate_number)
                    {
                        loc = Some((
                            -(n as i32),
                            trf("kfx_editor.hero_gate_n", "hero gate {n}", &[("n", &n)]),
                        ));
                        break;
                    }
                }
                _ => {}
            }
        }
        let c = self.live.conn.as_mut().ok_or("not connected")?;
        let p = c.panel.quick_player.clone();
        let (req, label) = match loc {
            Some((n, label)) => (actions::zoom_to_action_point(&p, n), label),
            None => (
                actions::script(&format!(
                    "ZOOM_TO_LOCATION({},{})",
                    p.script_name(),
                    p.script_name()
                )),
                trf("kfx_editor.p_heart", "{p} heart", &[("p", &p.name())]),
            ),
        };
        c.panel.quick(
            &trf(
                "kfx_editor.camera_to_label",
                "camera to {label}",
                &[("label", &label)],
            ),
            req,
        );
        Ok(label)
    }

    /// The game camera to the centre of the editor's view (`zoomto.subtilecoord`, needs cheats).
    pub fn live_camera_to_view(&mut self) {
        let t = self.camera.target;
        if let Some(c) = self.live.conn.as_mut() {
            let (x, y) = (t.x.max(0.0) as i32, t.z.max(0.0) as i32);
            c.panel.quick(
                &trf(
                    "kfx_editor.camera_to_subtile_x_y",
                    "camera to subtile {x},{y}",
                    &[("x", &x), ("y", &y)],
                ),
                actions::zoom_to_subtile(x, y),
            );
        }
    }

    /// Recorded panel content for documentation screenshots (`--actions` `live_demo`): log lines,
    /// watched values and creature positions as a real session produced them.
    pub fn live_demo(&mut self) {
        if self.live.conn.is_none() {
            let _ = self.live_connect("127.0.0.1:1");
        }
        let Some(c) = self.live.conn.as_mut() else {
            return;
        };
        let p = &mut c.panel;
        p.push_log(
            LogKind::Info,
            "connected",
            "connected, KeeperFX 1.3.1 (recorded)",
        );
        p.push_log(
            LogKind::Info,
            "level",
            "level 1 \"Editor test 1\" (85x85 slabs)",
        );
        p.push_log(LogKind::Event, "GAME_STARTED", "");
        p.push_log(LogKind::VarUpdate, "VAR_UPDATE", "PLAYER0 MONEY = 21234");
        p.push_log(LogKind::Reply, "map_command", "ok: add 1234 gold");
        p.push_log(LogKind::Event, "GAME_SAVED", "");
        p.push_log(
            LogKind::Error,
            "console_command",
            "FAILED_TO_EXECUTE_CONSOLE_COMMAND (needs \
            cheat mode: start the game with the -alex command line option)",
        );
        for (var, v) in [("MONEY", 21234), ("GAME_TURN", 812), ("FLAG0", 3)] {
            let w = Watch {
                player: Player::P0,
                var: var.into(),
            };
            if !p.watches.contains(&w) {
                p.watches.push(w.clone());
            }
            p.values.insert(
                w,
                WatchValue {
                    value: Some(v),
                    previous: None,
                    updated: None,
                    error: None,
                },
            );
        }
        let lines: Vec<String> = [
            "KFXLIVE THING Creature IMP 0 121 128",
            "KFXLIVE THING Creature IMP 0 127 136",
            "KFXLIVE THING Creature IMP 0 133 125",
            "KFXLIVE THING Creature IMP 0 118 138",
            "KFXLIVE THING Creature FAIRY 4 140 130",
            "KFXLIVE THING Creature DWARFA 4 143 133",
            "KFXLIVE THING Creature IMP 1 196 190",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        p.things = kfx_live::logtail::parse_things(&lines);
    }

    /// The Live link window.
    pub(super) fn live_ui(&mut self, ctx: &egui::Context) {
        if !self.settings.audiolive.live_open {
            return;
        }
        let mut open = true;
        let mut connect: Option<String> = None;
        let mut disconnect = false;
        let mut refresh = false;
        let mut cam_sel = false;
        let mut cam_view = false;
        egui::Window::new(trd("kfx_editor.live_link", "Live link"))
            .id(egui::Id::unique("live-link"))
            .open(&mut open)
            .default_size([820.0, 560.0])
            .show(ctx, |ui| {
                let s = &mut self.settings.audiolive;
                ui.horizontal_wrapped(|ui| {
                    ui.label(trd("kfx_editor.game", "Game"));
                    if self.live.addr.is_empty() {
                        self.live.addr = format!("127.0.0.1:{}", s.port);
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.live.addr)
                            .desired_width(150.0)
                            .hint_text("127.0.0.1:5599"),
                    );
                    if self.live.conn.is_some() {
                        if ui
                            .button(trd("kfx_editor.disconnect", "Disconnect"))
                            .clicked()
                        {
                            disconnect = true;
                        }
                    } else if ui.button(trd("kfx_editor.connect", "Connect")).clicked() {
                        connect = Some(self.live.addr.clone());
                    }
                    ui.separator();
                    ui.checkbox(
                        &mut s.api_on_play,
                        trd(
                            "kfx_editor.enable_the_game_api_when_playing",
                            "Enable the game API when playing",
                        ),
                    )
                    .on_hover_text(trd(
                        "kfx_editor.play_and_save_and_play_add_cfg_api_enabled_on_cf",
                        "Play and Save and play add -cfg API_ENABLED=ON -cfg \
                        API_PORT=<port> to the game's command line. Your keeperfx.cfg is not \
                        changed.",
                    ));
                    ui.label(trd("kfx_editor.port", "port"));
                    ui.add(egui::DragValue::new(&mut s.port).range(1024..=65535));
                    ui.checkbox(
                        &mut s.auto_connect,
                        trd(
                            "kfx_editor.connect_when_the_game_starts",
                            "Connect when the game starts",
                        ),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    let connected = self
                        .live
                        .conn
                        .as_ref()
                        .is_some_and(|c| c.panel.client.is_connected());
                    ui.checkbox(
                        &mut s.overlay,
                        trd(
                            "kfx_editor.creature_positions_on_the_map",
                            "Creature positions on the map",
                        ),
                    );
                    if ui
                        .add_enabled(
                            connected,
                            egui::Button::new(trd(
                                "kfx_editor.refresh_positions",
                                "Refresh positions",
                            )),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.asks_the_game_lua_to_print_every_creature_s_posi",
                            "Asks the game (Lua) to print every creature's position into \
                            keeperfx.log",
                        ))
                        .clicked()
                    {
                        refresh = true;
                    }
                    ui.checkbox(
                        &mut self.live.auto_refresh,
                        trd("kfx_editor.every_10_s", "every 10 s"),
                    );
                    if let Some(c) = &self.live.conn {
                        ui.label(
                            RichText::new(trf(
                                "kfx_editor.things_positions_lua_pool_used_pool_used_8192_by",
                                "{things} positions, Lua pool used {pool_used} / 8192 bytes",
                                &[
                                    ("things", &c.panel.things.len()),
                                    ("pool_used", &c.panel.lua.pool_used),
                                ],
                            ))
                            .small()
                            .color(TEXT_DIM),
                        );
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            connected,
                            egui::Button::new(trd(
                                "kfx_editor.game_camera_to_the_selection",
                                "Game camera to the selection",
                            )),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.the_selected_action_point_or_hero_gate_else_your",
                            "The selected action point or hero gate, else your heart \
                            (ZOOM_TO_LOCATION, no cheats needed)",
                        ))
                        .clicked()
                    {
                        cam_sel = true;
                    }
                    if ui
                        .add_enabled(
                            connected,
                            egui::Button::new(trd(
                                "kfx_editor.game_camera_to_the_view_centre",
                                "Game camera to the view centre",
                            )),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.needs_the_game_started_with_alex",
                            "Needs the game started with -alex \
                            (Preferences > Play > cheats)",
                        ))
                        .clicked()
                    {
                        cam_view = true;
                    }
                });
                egui::CollapsingHeader::new(trd(
                    "kfx_editor.what_the_live_link_cannot_do",
                    "What the live link cannot do",
                ))
                .id_salt("live-limits")
                .show(ui, |ui| {
                    for l in LIMITS {
                        ui.label(RichText::new(*l).small());
                    }
                });
                if let Some(m) = &self.live.message {
                    ui.colored_label(Color32::LIGHT_RED, m);
                }
                ui.separator();
                match self.live.conn.as_mut() {
                    Some(c) => kfx_live::ui::show(ui, &mut c.panel, &mut c.ui),
                    None => {
                        ui.label(trd(
                            "kfx_editor.not_connected_turn_on_enable_the_game_api_when_p",
                            "Not connected. Turn on \"Enable the game API when playing\" \
                            and press Play, or connect to a game whose keeperfx.cfg has \
                            API_ENABLED=ON.",
                        ));
                    }
                }
            });
        self.settings.audiolive.live_open = open;
        if disconnect {
            self.live_disconnect();
        }
        if let Some(a) = connect {
            if let Err(e) = self.live_connect(&a) {
                self.live.message = Some(e);
            } else {
                self.live.message = None;
            }
        }
        if refresh {
            self.live_refresh_positions();
        }
        if cam_sel {
            match self.live_camera_to_selection() {
                Ok(l) => {
                    self.status = trf(
                        "kfx_editor.game_camera_to_l",
                        "Game camera to {l}",
                        &[("l", &l)],
                    )
                }
                Err(e) => self.status = e,
            }
        }
        if cam_view {
            self.live_camera_to_view();
        }
    }

    /// The in-game positions over the map (3D and 2D panes).
    pub(super) fn live_overlay(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        vp: Vec2,
        ppp: f32,
    ) {
        if !self.settings.audiolive.overlay {
            return;
        }
        let Some(c) = &self.live.conn else { return };
        let to_ui = |p: Vec2| rect.min + EVec2::new(p.x / ppp, p.y / ppp);
        let labels = self.camera.zoom > 0.3;
        for t in c.panel.things.iter().filter(|t| t.class == "Creature") {
            let (x, y) = (t.stl_x.max(0) as usize, t.stl_y.max(0) as usize);
            let h = self.terrain.top_height(x, y) as f32;
            let Some(p) = self
                .camera
                .project(Vec3::new(x as f32 + 0.5, h + 0.3, y as f32 + 0.5), vp)
            else {
                continue;
            };
            let at = to_ui(p);
            if !rect.contains(at) {
                continue;
            }
            let col = crate::assets::PLAYERS
                .get(t.owner.clamp(0, 8) as usize)
                .map(|p| Color32::from_rgb(p.1[0], p.1[1], p.1[2]))
                .unwrap_or(Color32::WHITE);
            let pts = vec![
                at + EVec2::new(0.0, -7.0),
                at + EVec2::new(6.0, 0.0),
                at + EVec2::new(0.0, 7.0),
                at + EVec2::new(-6.0, 0.0),
            ];
            painter.add(egui::Shape::convex_polygon(
                pts,
                col,
                egui::Stroke::new(1.5, Color32::BLACK),
            ));
            if labels {
                let g = painter.layout_no_wrap(
                    t.model.clone(),
                    egui::FontId::proportional(10.0),
                    Color32::WHITE,
                );
                let r = egui::Rect::from_min_size(
                    at + EVec2::new(8.0, -6.0),
                    g.size() + EVec2::new(4.0, 1.0),
                );
                painter.rect_filled(r, 2.0, Color32::from_black_alpha(170));
                painter.galley(r.min + EVec2::new(2.0, 0.5), g, Color32::WHITE);
            }
        }
    }
}
