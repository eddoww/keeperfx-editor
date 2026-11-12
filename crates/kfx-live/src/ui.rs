//! egui panel for the live link: connection status, event log with filter, watch table of flags /
//! timers / variables per player, console with history, quick actions, thing positions.
//!
//! ```ignore
//! let mut panel = LivePanel::new(LiveClient::spawn(Config::localhost(5599)));
//! // every frame:
//! kfx_live::ui::show(ui, &mut panel);
//! ```
use crate::actions;
use crate::client::ConnState;
use crate::model::{ConsoleMode, LivePanel, LogKind, COMMON_VARS};
use crate::proto::Player;
use egui::{Color32, RichText};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Events,
    Watch,
    Console,
    Actions,
    Things,
}

/// Persistent UI state (selected tab, text boxes).
#[derive(Default)]
pub struct UiState {
    pub tab: Tab,
    pub watch_var: String,
    pub gold: i32,
    pub room: String,
    pub power: String,
    pub creature: String,
    pub camera_ap: i32,
    pub camera_x: i32,
    pub camera_y: i32,
    pub speed: u32,
    pub slot: u32,
    pub log_path: String,
    pub thing_class: String,
}

fn kind_color(k: LogKind) -> Color32 {
    match k {
        LogKind::Info => Color32::from_gray(160),
        LogKind::Event => Color32::from_rgb(110, 190, 240),
        LogKind::VarUpdate => Color32::from_rgb(150, 210, 140),
        LogKind::Request => Color32::from_rgb(200, 180, 110),
        LogKind::Reply => Color32::from_rgb(170, 170, 210),
        LogKind::Error => Color32::from_rgb(230, 100, 90),
        LogKind::Wire => Color32::from_gray(120),
    }
}

fn kind_name(k: LogKind) -> &'static str {
    match k {
        LogKind::Info => "info",
        LogKind::Event => "events",
        LogKind::VarUpdate => "var updates",
        LogKind::Request => "requests",
        LogKind::Reply => "replies",
        LogKind::Error => "errors",
        LogKind::Wire => "wire",
    }
}

pub fn clock(t: SystemTime) -> String {
    let s = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
}

/// The whole panel. Call every frame.
pub fn show(ui: &mut egui::Ui, panel: &mut LivePanel, st: &mut UiState) {
    panel.update();
    status_line(ui, panel);
    ui.separator();
    ui.horizontal(|ui| {
        for (t, n) in [
            (Tab::Events, "Events"),
            (Tab::Watch, "Watch"),
            (Tab::Console, "Console"),
            (Tab::Actions, "Quick actions"),
            (Tab::Things, "Positions"),
        ] {
            ui.selectable_value(&mut st.tab, t, n);
        }
    });
    ui.separator();
    match st.tab {
        Tab::Events => events_tab(ui, panel),
        Tab::Watch => watch_tab(ui, panel, st),
        Tab::Console => console_tab(ui, panel),
        Tab::Actions => actions_tab(ui, panel, st),
        Tab::Things => things_tab(ui, panel, st),
    }
    if panel.client.is_connected() || !panel.log.is_empty() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    } else {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
    }
}

pub fn status_line(ui: &mut egui::Ui, panel: &LivePanel) {
    let s = panel.client.status();
    ui.horizontal(|ui| {
        let (col, text) = match s.state {
            ConnState::Connected => (Color32::from_rgb(90, 200, 110), "connected"),
            ConnState::Connecting => (Color32::from_rgb(230, 190, 80), "connecting"),
            ConnState::Disconnected => (Color32::from_rgb(200, 90, 80), "no game"),
        };
        ui.label(RichText::new("o").color(col).strong());
        ui.label(format!("{text} ({})", s.addr));
        if let Some(v) = &s.kfx_version {
            ui.weak(format!("KeeperFX {v}"));
        }
        if let Some(l) = &panel.level {
            ui.weak(format!(
                "level {} {}",
                l.level_number,
                l.level_name.clone().unwrap_or_default()
            ));
        }
        ui.weak(format!(
            "sent {} / replies {} / events {} / timeouts {} / queued {}",
            s.requests_sent, s.replies, s.events, s.timeouts, s.queued
        ));
        if s.state != ConnState::Connected {
            if let Some(e) = &s.last_error {
                ui.weak(e);
            }
            ui.weak(
                "The game's API is off unless keeperfx.cfg has API_ENABLED=ON (and API_PORT), \
                or the game was started with -cfg API_ENABLED=ON",
            );
        }
    });
}

fn events_tab(ui: &mut egui::Ui, panel: &mut LivePanel) {
    ui.horizontal(|ui| {
        ui.label("Filter");
        ui.add(egui::TextEdit::singleline(&mut panel.filter).desired_width(160.0));
        for k in [
            LogKind::Event,
            LogKind::VarUpdate,
            LogKind::Request,
            LogKind::Reply,
            LogKind::Error,
            LogKind::Info,
            LogKind::Wire,
        ] {
            let mut on = panel.show.get(&k).copied().unwrap_or(true);
            if ui.checkbox(&mut on, kind_name(k)).changed() {
                panel.show.insert(k, on);
            }
        }
        if ui.button("Clear").clicked() {
            panel.log.clear();
        }
    });
    let rows: Vec<_> = panel
        .filtered_log()
        .into_iter()
        .map(|e| (e.when, e.kind, e.tag.clone(), e.text.clone()))
        .collect();
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show_rows(ui, 16.0, rows.len(), |ui, range| {
            for (when, kind, tag, text) in &rows[range] {
                ui.horizontal(|ui| {
                    ui.weak(clock(*when));
                    ui.label(RichText::new(tag).color(kind_color(*kind)).monospace());
                    ui.label(RichText::new(text).monospace());
                });
            }
        });
}

/// The players a watch can be for: the keepers, and the heroes and neutral creatures (by name, as
/// the API takes them), so that a hero wave's creatures can be watched too.
pub fn watch_players() -> Vec<Player> {
    let mut v: Vec<Player> = (0..7).map(Player::Index).collect();
    v.push(Player::Name("PLAYER_GOOD".into()));
    v.push(Player::Name("PLAYER_NEUTRAL".into()));
    v
}

fn watch_tab(ui: &mut egui::Ui, panel: &mut LivePanel, st: &mut UiState) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("kfx_live_watch_player")
            .selected_text(panel.quick_player.name())
            .show_ui(ui, |ui| {
                for p in watch_players() {
                    let label = p.name();
                    ui.selectable_value(&mut panel.quick_player, p, label);
                }
            });
        ui.add(
            egui::TextEdit::singleline(&mut st.watch_var)
                .desired_width(140.0)
                .hint_text("FLAG0, MONEY, TIMER3, IMP…"),
        );
        if ui.button("Watch").clicked() {
            let v = std::mem::take(&mut st.watch_var);
            let p = panel.quick_player.clone();
            panel.add_watch(p, &v);
        }
        egui::ComboBox::from_id_salt("kfx_live_watch_common")
            .selected_text("Common")
            .show_ui(ui, |ui| {
                for v in COMMON_VARS {
                    if ui.selectable_label(false, *v).clicked() {
                        let p = panel.quick_player.clone();
                        panel.add_watch(p, v);
                    }
                }
            });
        if ui.button("Refresh").clicked() {
            panel.refresh_watches();
        }
    });
    ui.separator();
    // one column per player, one row per variable
    let mut players: Vec<Player> = vec![];
    let mut vars: Vec<String> = vec![];
    for w in &panel.watches {
        if !players.contains(&w.player) {
            players.push(w.player.clone());
        }
        if !vars.contains(&w.var) {
            vars.push(w.var.clone());
        }
    }
    players.sort();
    let mut remove = None;
    egui::Grid::new("kfx_live_watch_grid")
        .striped(true)
        .show(ui, |ui| {
            ui.label("");
            for p in &players {
                ui.label(RichText::new(p.name()).strong());
            }
            ui.end_row();
            for v in &vars {
                ui.label(RichText::new(v).monospace());
                for p in &players {
                    let w = crate::model::Watch {
                        player: p.clone(),
                        var: v.clone(),
                    };
                    match panel.values.get(&w) {
                        None => {
                            ui.weak("-");
                        }
                        Some(val) => {
                            let fresh = val.updated.is_some_and(|t| t.elapsed().as_millis() < 800);
                            let text = match (&val.error, val.value) {
                                (Some(e), _) => {
                                    RichText::new(e).color(Color32::from_rgb(230, 100, 90))
                                }
                                (None, Some(x)) => {
                                    let t = RichText::new(x.to_string()).monospace();
                                    if fresh {
                                        t.color(Color32::from_rgb(250, 220, 110))
                                    } else {
                                        t
                                    }
                                }
                                (None, None) => RichText::new("…").weak(),
                            };
                            let r = ui.label(text);
                            r.context_menu(|ui| {
                                if ui.button("Stop watching").clicked() {
                                    remove = Some(w.clone());
                                    ui.close();
                                }
                            });
                        }
                    }
                }
                ui.end_row();
            }
        });
    if let Some(w) = remove {
        panel.remove_watch(&w);
    }
    if panel.watches.is_empty() {
        // the panel subscribes to each watch (`subscribe_var`)
        ui.weak("Add variables to watch. Values update when the game reports a change.");
    }
}

fn console_tab(ui: &mut egui::Ui, panel: &mut LivePanel) {
    ui.horizontal(|ui| {
        for (m, n) in [
            (ConsoleMode::Console, "console"),
            (ConsoleMode::Script, "DK script"),
            (ConsoleMode::Lua, "Lua"),
        ] {
            ui.selectable_value(&mut panel.console_mode, m, n);
        }
        ui.checkbox(&mut panel.lua.use_console, "Lua via console (cheats)");
        ui.weak(format!(
            "Lua string pool used: {} / 8192 bytes",
            panel.lua.pool_used
        ));
    });
    let h = (ui.available_height() - 34.0).max(60.0);
    let rows: Vec<_> = panel
        .log
        .iter()
        .filter(|e| matches!(e.kind, LogKind::Request | LogKind::Reply | LogKind::Error))
        .map(|e| (e.kind, e.tag.clone(), e.text.clone()))
        .collect();
    egui::ScrollArea::vertical()
        .max_height(h)
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (k, tag, text) in rows {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tag).color(kind_color(k)).monospace());
                    ui.label(RichText::new(text).monospace());
                });
            }
        });
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut panel.console_input)
                .desired_width(ui.available_width() - 70.0)
                .hint_text("line (prefix ! console, > script, = Lua)"),
        );
        if r.has_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                panel.history_step(true);
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                panel.history_step(false);
            }
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Send").clicked() || enter {
            panel.submit_console();
            r.request_focus();
        }
    });
}

fn actions_tab(ui: &mut egui::Ui, panel: &mut LivePanel, st: &mut UiState) {
    let p = panel.quick_player.clone();
    ui.horizontal(|ui| {
        ui.label("Player");
        egui::ComboBox::from_id_salt("kfx_live_quick_player")
            .selected_text(p.name())
            .show_ui(ui, |ui| {
                for i in 0..7 {
                    ui.selectable_value(
                        &mut panel.quick_player,
                        Player::Index(i),
                        format!("PLAYER{i}"),
                    );
                }
            });
    });
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut st.gold).speed(10.0));
        if ui.button("Add gold").clicked() {
            let g = if st.gold == 0 { 1000 } else { st.gold };
            panel.quick(&format!("add {g} gold"), actions::add_gold(&p, g));
        }
        if ui.button("Reveal map").clicked() {
            let size = panel
                .level
                .as_ref()
                .map(|l| (l.mapsize_x as i32, l.mapsize_y as i32))
                .unwrap_or((85, 85));
            panel.quick("reveal map", actions::reveal_map(&p, size));
        }
    });
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("kfx_live_room")
            .selected_text(if st.room.is_empty() { "room" } else { &st.room })
            .show_ui(ui, |ui| {
                for r in actions::ROOMS {
                    ui.selectable_value(&mut st.room, r.to_string(), *r);
                }
            });
        if ui.button("Give room").clicked() && !st.room.is_empty() {
            if let Ok(r) = actions::give_room(&p, &st.room) {
                panel.quick(&format!("room {}", st.room), r);
            }
        }
        egui::ComboBox::from_id_salt("kfx_live_power")
            .selected_text(if st.power.is_empty() {
                "power"
            } else {
                &st.power
            })
            .show_ui(ui, |ui| {
                for r in actions::POWERS {
                    ui.selectable_value(&mut st.power, r.to_string(), *r);
                }
            });
        if ui.button("Give power").clicked() && !st.power.is_empty() {
            if let Ok(r) = actions::give_power(&p, &st.power) {
                panel.quick(&format!("power {}", st.power), r);
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut st.creature)
                .desired_width(110.0)
                .hint_text("IMP, DRAGON…"),
        );
        if ui.button("Add creature at heart").clicked() && !st.creature.is_empty() {
            let loc = p.script_name();
            if let Ok(r) =
                actions::add_creature(&p, &st.creature.to_ascii_uppercase(), &loc, 1, 1, 0)
            {
                panel.quick(&format!("add {}", st.creature), r);
            }
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Camera: action point");
        ui.add(egui::DragValue::new(&mut st.camera_ap));
        if ui.button("Jump").clicked() {
            panel.quick(
                &format!("camera to action point {}", st.camera_ap),
                actions::zoom_to_action_point(&p, st.camera_ap),
            );
        }
        ui.label("Subtile");
        ui.add(egui::DragValue::new(&mut st.camera_x));
        ui.add(egui::DragValue::new(&mut st.camera_y));
        if ui
            .button("Jump (cheats)")
            .on_hover_text("Needs the game started with -alex (cheats)")
            .clicked()
        {
            panel.quick(
                &format!("camera to {},{}", st.camera_x, st.camera_y),
                actions::zoom_to_subtile(st.camera_x, st.camera_y),
            );
        }
    });
    ui.horizontal(|ui| {
        if ui
            .button("Pause")
            .on_hover_text("The API cannot unpause: unpause in the game")
            .clicked()
        {
            panel.quick("pause", actions::pause());
        }
        if st.speed == 0 {
            st.speed = 20;
        }
        ui.add(
            egui::DragValue::new(&mut st.speed)
                .range(1..=500)
                .suffix(" turns/s"),
        );
        if ui.button("Set speed").clicked() {
            panel.quick(&format!("speed {}", st.speed), actions::set_speed(st.speed));
        }
        ui.add(
            egui::DragValue::new(&mut st.slot)
                .range(0..=99)
                .prefix("slot "),
        );
        if ui.button("Save").clicked() {
            panel.quick("save", actions::save_game(st.slot));
        }
        if ui.button("Load").clicked() {
            panel.quick("load", actions::load_game(st.slot));
        }
    });
    ui.weak(
        "Pushing an edited script is not possible through the API: restart the level to run a \
        changed mapNNNNN.txt.",
    );
}

fn things_tab(ui: &mut egui::Ui, panel: &mut LivePanel, st: &mut UiState) {
    ui.horizontal(|ui| {
        ui.label("Game log");
        let r = ui.add(
            egui::TextEdit::singleline(&mut st.log_path)
                .desired_width(300.0)
                .hint_text("path of keeperfx.log"),
        );
        if r.lost_focus() {
            panel.set_log_path(if st.log_path.is_empty() {
                None
            } else {
                Some(st.log_path.clone().into())
            });
        }
    });
    ui.horizontal(|ui| {
        for c in ["Creature", "Object", "Trap", "Door", "EffectGen"] {
            if ui.button(c).clicked() {
                st.thing_class = c.to_string();
                panel.request_things(c);
            }
        }
        ui.weak("Lua prints the positions into the game log; this panel reads them from there.");
    });
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("kfx_live_things")
                .striped(true)
                .show(ui, |ui| {
                    for t in &panel.things {
                        ui.label(&t.class);
                        ui.label(&t.model);
                        ui.label(format!("owner {}", t.owner));
                        ui.label(format!("{}, {}", t.stl_x, t.stl_y));
                        ui.end_row();
                    }
                });
        });
}
