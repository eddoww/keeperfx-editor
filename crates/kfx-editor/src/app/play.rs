//! Play-testing from the editor (F5 / Shift+F5) and the Game log panel.
use super::*;
use crate::playtest::{self, LogEntry, LogLevel, PlayTarget, ScriptKind};
use kfx_prefab::i18n::{trd, trf};
use std::sync::atomic::{AtomicBool, Ordering};

/// State of the game log panel and the last launch.
#[derive(Default)]
pub struct GameLog {
    pub open: bool,
    /// Folder of the game whose `keeperfx.log` is shown.
    pub game_dir: Option<PathBuf>,
    pub entries: Vec<LogEntry>,
    pub show_warnings: bool,
    pub only_scripts: bool,
    /// Set by the reaper thread when the launched game exits.
    pub done: Option<Arc<AtomicBool>>,
    pub last_launch: Option<String>,
    pub error: Option<String>,
}

/// A loose map was played: what to do.
pub struct LoosePrompt {
    pub dest: u32,
    pub name: String,
}

/// The recording of Play and record, in the game's folder.
pub const PACKET_FILE: &str = "kfx_editor_auto.pck";

impl EditorApp {
    /// The game executable: the preference, else the one detected in the install.
    pub fn game_exe(&self) -> Option<PathBuf> {
        self.settings
            .play
            .exe
            .clone()
            .filter(|p| p.is_file())
            .or_else(|| self.install().and_then(playtest::detect_exe))
    }

    /// The launch for the open level (saved files), or why it cannot be played.
    pub fn play_launch(&self) -> Result<(playtest::Launch, PlayTarget), String> {
        let d = self.doc.as_ref().ok_or("no map")?;
        if !self.doc_on_disk {
            return Err(trd("kfx_editor.save_the_map_first", "save the map first"));
        }
        let install = self.install().ok_or("no install")?;
        let exe = self.game_exe().ok_or(trd(
            "kfx_editor.no_game_executable_set_it_in_preferences_play",
            "no game executable: set it in Preferences > Play",
        ))?;
        let target = playtest::play_target(install, &d.doc.folder, d.doc.level);
        match &target {
            PlayTarget::Pack {
                campaign,
                multiplayer,
                ..
            } => {
                let mut l = playtest::build_launch(
                    &self.settings.play,
                    &exe,
                    campaign,
                    d.doc.level,
                    *multiplayer,
                );
                // record or replay the game's input packets (main.cpp:1822-1838)
                match self.packet_mode {
                    Some(true) => {
                        if !l.cwd.join(PACKET_FILE).is_file() {
                            return Err(trf(
                                "kfx_editor.no_recording_yet_packet_file_in_cwd_use_play_and",
                                "no recording yet ({PACKET_FILE} in {cwd}): use Play \
                                and record first",
                                &[("PACKET_FILE", &PACKET_FILE), ("cwd", &l.cwd.display())],
                            ));
                        }
                        l.args
                            .extend(["-packetload".to_string(), PACKET_FILE.to_string()]);
                    }
                    Some(false) => l
                        .args
                        .extend(["-packetsave".to_string(), PACKET_FILE.to_string()]),
                    None => {}
                }
                Ok((l, target))
            }
            PlayTarget::Loose => Err("loose map".into()),
        }
    }

    /// Play and record the input (`replay` false, `-packetsave`), or replay the last recording
    /// (`-packetload`); the right-click menu of Play.
    pub fn play_packet(&mut self, replay: bool) {
        self.packet_mode = Some(replay);
        self.play(false);
        self.packet_mode = None;
    }

    /// Play (F5) or Save & Play (Shift+F5).
    pub fn play(&mut self, save_first: bool) {
        if self.doc.is_none() {
            return;
        }
        if save_first
            || (self.settings.play.save_before_play && self.anything_dirty())
            || !self.doc_on_disk
        {
            self.do_save();
            if self.anything_dirty() || !self.doc_on_disk {
                return;
            }
        }
        match self.play_launch() {
            Ok((l, target)) => {
                if let PlayTarget::Pack {
                    listed: false,
                    kind,
                    ..
                } = &target
                {
                    if *kind != kfx_formats::campaign::PackKind::Campaign {
                        self.status = trd(
                            "kfx_editor.note_the_map_pack_does_not_list_this_level_no_lo",
                            "Note: the map pack does not list this level (no .lof / \
                            .lif); the game may not find it",
                        );
                    }
                }
                self.launch(l);
            }
            Err(e) if e == "loose map" => {
                let install = self.install().map(Path::to_path_buf).unwrap_or_default();
                let personal = kfx_gfx::install::resolve(&install, "levels/personal")
                    .unwrap_or_else(|| install.join("levels/personal"));
                let level = self.doc.as_ref().map(|d| d.doc.level).unwrap_or(1);
                let name = self
                    .doc
                    .as_ref()
                    .map(|d| {
                        trf(
                            "kfx_editor.map_level_editor_copy",
                            "map{level} (editor copy)",
                            &[("level", &format!("{:05}", d.doc.level))],
                        )
                    })
                    .unwrap_or_default();
                self.loose_prompt = Some(LoosePrompt {
                    dest: playtest::next_free_level(&personal, level.max(1)),
                    name,
                });
            }
            Err(e) => {
                self.status = trf("kfx_editor.cannot_play_e", "Cannot play: {e}", &[("e", &e)])
            }
        }
    }

    /// Play a map file that is not open (map browser).
    pub fn play_file(&mut self, slb: &Path) {
        let (Some(install), Some(level), Some(folder)) = (
            self.install().map(Path::to_path_buf),
            document::map_number(slb),
            slb.parent(),
        ) else {
            return;
        };
        let Some(exe) = self.game_exe() else {
            self.status = trd(
                "kfx_editor.no_game_executable_set_it_in_preferences_play",
                "no game executable: set it in Preferences > Play",
            );
            return;
        };
        match playtest::play_target(&install, folder, level) {
            PlayTarget::Pack {
                campaign,
                multiplayer,
                ..
            } => {
                let l = playtest::build_launch(
                    &self.settings.play,
                    &exe,
                    &campaign,
                    level,
                    multiplayer,
                );
                self.launch(l);
            }
            PlayTarget::Loose => {
                self.status = trd(
                    "kfx_editor.that_map_is_not_in_a_campaign_or_map_pack_open_i",
                    "That map is not in a campaign or map pack: open it and use Play to \
                    copy it into levels/personal",
                )
            }
        }
    }

    fn launch(&mut self, l: playtest::Launch) {
        let l = self.live_prepare_launch(l);
        let done = Arc::new(AtomicBool::new(false));
        match playtest::spawn_detached(&l, done.clone()) {
            Ok(pid) => {
                self.status = trf(
                    "kfx_editor.game_started_pid_pid_l",
                    "Game started (pid {pid}): {l}",
                    &[("pid", &pid), ("l", &l.display())],
                );
                self.game_log.game_dir = Some(l.cwd.clone());
                self.game_log.done = Some(done);
                self.game_log.last_launch = Some(l.display());
                self.live_game_launched(&l);
            }
            Err(e) => {
                self.status = trf(
                    "kfx_editor.could_not_start_the_game_e",
                    "Could not start the game: {e}",
                    &[("e", &e)],
                )
            }
        }
    }

    /// Copy the loose map into `levels/personal` and play it from there.
    fn play_copy(&mut self, dest: u32, name: &str) {
        let Some(d) = &self.doc else { return };
        let Some(install) = self.install().map(Path::to_path_buf) else {
            return;
        };
        let dims = d.dims();
        match playtest::copy_into_personal(
            &install,
            &d.doc.folder,
            d.doc.level,
            dest,
            name,
            (dims.w, dims.h),
        ) {
            Ok(pack) => {
                let Some(exe) = self.game_exe() else {
                    self.status = trd(
                        "kfx_editor.no_game_executable_set_it_in_preferences_play",
                        "no game executable: set it in Preferences > Play",
                    );
                    return;
                };
                let l = playtest::build_launch(
                    &self.settings.play,
                    &exe,
                    "levels/personal",
                    dest,
                    false,
                );
                self.status = trf(
                    "kfx_editor.copied_to_pack_as_map_dest",
                    "Copied to {pack} as map{dest}",
                    &[("pack", &pack.display()), ("dest", &format!("{:05}", dest))],
                );
                self.launch(l);
            }
            Err(e) => {
                self.status = trf("kfx_editor.copy_failed_e", "Copy failed: {e}", &[("e", &e)])
            }
        }
    }

    /// Read `keeperfx.log` of the game folder into the panel.
    pub fn refresh_game_log(&mut self) {
        let dir = self.game_log.game_dir.clone().or_else(|| {
            self.game_exe()
                .and_then(|e| e.parent().map(Path::to_path_buf))
        });
        let Some(dir) = dir else {
            self.game_log.error = Some(trd("kfx_editor.no_game_folder", "no game folder").into());
            return;
        };
        match playtest::read_log(&dir) {
            Some(t) => {
                self.game_log.entries = playtest::parse_log(&t);
                self.game_log.error = None;
            }
            None => {
                self.game_log.entries.clear();
                self.game_log.error = Some(trf(
                    "kfx_editor.no_keeperfx_log_in_dir",
                    "no keeperfx.log in {dir}",
                    &[("dir", &dir.display())],
                ));
            }
        }
        self.game_log.game_dir = Some(dir);
    }

    /// Per frame: the game ended -> show its log.
    pub(super) fn play_tick(&mut self) {
        if let Some(d) = &self.game_log.done {
            if d.load(Ordering::SeqCst) {
                self.game_log.done = None;
                self.refresh_game_log();
                self.game_log.open = true;
                let (e, w) = (
                    self.game_log
                        .entries
                        .iter()
                        .filter(|x| x.level == LogLevel::Error)
                        .count(),
                    self.game_log
                        .entries
                        .iter()
                        .filter(|x| x.level == LogLevel::Warning)
                        .count(),
                );
                self.status = trf(
                    "kfx_editor.the_game_ended_e_errors_w_warnings_in_keeperfx_l",
                    "The game ended: {e} errors, {w} warnings in keeperfx.log",
                    &[("e", &e), ("w", &w)],
                );
            } else {
                self.ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
    }

    /// Follow the script link of the `n`th linked log entry (as a click in the panel does).
    pub fn game_log_go(&mut self, n: usize) -> Result<(), String> {
        let (k, line) = self
            .game_log
            .entries
            .iter()
            .filter_map(|e| e.script)
            .nth(n)
            .ok_or("no such linked log line")?;
        let tab = self
            .scripts
            .as_ref()
            .map(|s| {
                if k == ScriptKind::Dk {
                    s.ws.dk_name()
                } else {
                    s.ws.lua_name()
                }
            })
            .ok_or("no scripts")?;
        self.reveal_script_line(&tab, line);
        Ok(())
    }

    pub(super) fn game_log_ui(&mut self, ctx: &egui::Context) {
        if !self.game_log.open {
            return;
        }
        let mut open = true;
        let mut go: Option<(ScriptKind, usize)> = None;
        let mut refresh = false;
        let (dk, lua) = self
            .scripts
            .as_ref()
            .map(|s| (s.ws.dk_name(), s.ws.lua_name()))
            .unwrap_or_default();
        egui::Window::new(trd("kfx_editor.game_log", "Game log"))
            .open(&mut open)
            .default_size([760.0, 420.0])
            .show(ctx, |ui| {
                let g = &mut self.game_log;
                ui.horizontal(|ui| {
                    if ui
                        .button(trd("kfx_editor.reload_3a4c", "⟳ Reload"))
                        .clicked()
                    {
                        refresh = true;
                    }
                    ui.checkbox(&mut g.show_warnings, trd("kfx_editor.warnings", "Warnings"));
                    ui.checkbox(
                        &mut g.only_scripts,
                        trd("kfx_editor.script_lines_only", "Script lines only"),
                    );
                    if let Some(d) = &g.game_dir {
                        ui.label(
                            RichText::new(format!("{}", d.join("keeperfx.log").display()))
                                .small()
                                .color(TEXT_DIM),
                        );
                    }
                });
                if let Some(l) = &g.last_launch {
                    ui.label(RichText::new(l).small().monospace().color(TEXT_DIM));
                }
                if g.done.is_some() {
                    ui.label(
                        RichText::new(trd(
                            "kfx_editor.the_game_is_running_its_log_is_shown_when_it_end",
                            "The game is running; its log is shown when it ends.",
                        ))
                        .color(GOLD),
                    );
                }
                if let Some(e) = &g.error {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
                ui.separator();
                let rows: Vec<&LogEntry> = g
                    .entries
                    .iter()
                    .filter(|e| {
                        (g.show_warnings || e.level == LogLevel::Error)
                            && (!g.only_scripts || e.script.is_some())
                    })
                    .collect();
                ui.label(
                    RichText::new(trf(
                        "kfx_editor.rows_shown_of_entries_errors_and_warnings",
                        "{rows} shown of {entries} errors and warnings",
                        &[("rows", &rows.len()), ("entries", &g.entries.len())],
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for e in rows {
                            let c = if e.level == LogLevel::Error {
                                Color32::from_rgb(240, 100, 90)
                            } else {
                                Color32::from_rgb(240, 190, 80)
                            };
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!("{:>5}", e.log_line))
                                        .monospace()
                                        .small()
                                        .color(TEXT_DIM),
                                );
                                match e.script {
                                    Some((k, line)) => {
                                        let file = if k == ScriptKind::Dk { &dk } else { &lua };
                                        if ui
                                            .link(
                                                RichText::new(format!("{file}:{line}")).monospace(),
                                            )
                                            .on_hover_text(trd(
                                                "kfx_editor.show_the_line_in_the_script_editor",
                                                "Show the line in the script editor",
                                            ))
                                            .clicked()
                                        {
                                            go = Some((k, line));
                                        }
                                    }
                                    None => {}
                                }
                                ui.label(RichText::new(&e.text).color(c));
                            });
                        }
                    });
            });
        self.game_log.open = open;
        if refresh {
            self.refresh_game_log();
        }
        if let Some((k, line)) = go {
            let tab = if k == ScriptKind::Dk { dk } else { lua };
            self.reveal_script_line(&tab, line);
        }
    }

    pub(super) fn loose_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(p) = &mut self.loose_prompt else {
            return;
        };
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("loose-play")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(
                RichText::new(trd(
                    "kfx_editor.play_a_map_outside_the_game_s_levels",
                    "Play a map outside the game's levels",
                ))
                .color(GOLD),
            );
            ui.label(trd(
                "kfx_editor.the_game_only_starts_levels_of_a_campaign_or_map",
                "The game only starts levels of a campaign or map pack: `-campaign` \
                names the pack and `-level` its level, read from the pack's levels folder \
                (campgns/, levels/ or multiplayer/). This map is in none of them.",
            ));
            ui.label(trd(
                "kfx_editor.it_can_be_copied_into_the_personal_levels_pack_l",
                "It can be copied into the \"Personal levels\" pack \
                (levels/personal) with a .lof file, and played from there. Your map stays where \
                it is; edits are not copied back.",
            ));
            egui::Grid::new("loose").num_columns(2).show(ui, |ui| {
                ui.label(trd(
                    "kfx_editor.level_number_in_levels_personal",
                    "Level number in levels/personal",
                ));
                ui.add(egui::DragValue::new(&mut p.dest).range(1..=32767));
                ui.end_row();
                ui.label(trd("kfx_editor.name", "Name"));
                ui.text_edit_singleline(&mut p.name);
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui
                    .button(trd("kfx_editor.copy_and_play", "Copy and play"))
                    .clicked()
                {
                    answer = Some(true);
                }
                if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                    answer = Some(false);
                }
            });
        });
        if let Some(a) = answer {
            let p = self.loose_prompt.take().unwrap();
            if a {
                self.play_copy(p.dest, &p.name);
            }
        }
    }
}
