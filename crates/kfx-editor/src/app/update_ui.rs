//! Updates in the window (`kfx-update` through `crate::updates`): the first-run notice about
//! update checks, the daily check, the banner, Help > Updates (versions, release notes, Update
//! now / Later / Skip this version, progress, roll back), Preferences > Updates, the health
//! confirmation of a freshly installed version, and the notice about a crash of the last run.
//!
//! Nothing is sent to GitHub before the first-run notice has been answered, and never when
//! automatic checks are off (except "Check now"). Installing goes through the usual "unsaved
//! changes" question, then the files are swapped and the editor starts again on the open map.

use super::*;
use crate::updates::{self, UpdateSettings};
use kfx_prefab::i18n::{trd, trf};
use kfx_update::client::{CheckOutcome, Update};
use kfx_update::install::{
    confirm_healthy, forget_health, on_start, rollback, rollback_version, Blocker, FsProbe, Layout,
    RealFs, StartState,
};
use kfx_update::updater::{Installed, Status, Worker};
use kfx_update::Channel;
use std::ffi::OsString;

/// The new version counts as working once its window has run this long.
const HEALTHY_AFTER: Duration = Duration::from_secs(10);

/// Starts a program (the new version, or the installer). Tests record instead.
pub type Launcher = fn(&Path, &[OsString]) -> Result<(), String>;

fn launch(exe: &Path, args: &[OsString]) -> Result<(), String> {
    kfx_update::install::relaunch(exe, args).map_err(|e| e.to_string())
}

/// What a restart is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartFor {
    Install,
    Rollback,
}

pub struct UpdateUi {
    /// Checks and downloads (created on first use).
    pub worker: Option<Worker>,
    pub layout: Result<Layout, String>,
    /// Why this copy cannot update itself (a link to the release page instead).
    pub blocker: Option<Blocker>,
    /// The newer version found by the last check.
    pub found: Option<Update>,
    pub banner: bool,
    pub dialog: bool,
    /// The first-run notice is open.
    pub notice: bool,
    /// The result of the last check or step, for the dialog.
    pub message: Option<(bool, String)>,
    /// The verified package of `found`.
    pub downloaded: Option<PathBuf>,
    /// Install as soon as the download is ready (Update now).
    install_when_ready: bool,
    /// Install when the editor closes (automatic installs).
    pub install_on_exit: bool,
    /// The last check was started by the user (its result is reported).
    manual: bool,
    pub health: StartState,
    started: Instant,
    pub crash_log: Option<PathBuf>,
    pub launcher: Launcher,
    /// What the editor started last (tests).
    pub launched: Option<(PathBuf, Vec<OsString>)>,
}

impl Default for UpdateUi {
    fn default() -> Self {
        let layout = updates::layout();
        UpdateUi {
            worker: None,
            blocker: updates::blocker(&layout),
            layout,
            found: None,
            banner: false,
            dialog: false,
            notice: false,
            message: None,
            downloaded: None,
            install_when_ready: false,
            install_on_exit: false,
            manual: false,
            health: StartState::Normal,
            started: Instant::now(),
            crash_log: None,
            launcher: launch,
            launched: None,
        }
    }
}

impl UpdateUi {
    fn worker(&mut self) -> Option<&Worker> {
        if self.worker.is_none() {
            self.worker = updates::updater().map(Worker::new);
        }
        self.worker.as_ref()
    }
}

impl EditorApp {
    /// At the start of a persistent session: the health of a fresh update, leftovers of an
    /// earlier update, the crash notice, then the first-run notice or the daily check.
    pub(super) fn updates_startup(&mut self) {
        let Some(dir) = updates::state_dir() else {
            return;
        };
        self.updates.health = on_start(&dir, crate::VERSION);
        if let Ok(l) = &self.updates.layout {
            kfx_update::install::cleanup_at_start(l);
        }
        if self.updates.health == StartState::Normal {
            if let Some(w) = self.updates.worker() {
                w.updater().clean_downloads();
            }
        }
        if let StartState::Unconfirmed { failed, .. } = &self.updates.health {
            if *failed >= 2 {
                // earlier starts of this version never got as far as the window
                self.updates.dialog = true;
            }
        }
        self.updates.crash_log = crate::crash::unseen(&dir);
        self.updates_begin();
    }

    /// The first-run notice, or the automatic check when it is due.
    pub fn updates_begin(&mut self) {
        let s = &self.settings.updates;
        if !s.notice_acknowledged {
            self.updates.notice = true;
        } else if s.auto_check {
            self.check_updates(false);
        }
    }

    /// Ask GitHub for releases (a manual check ignores the daily interval).
    pub fn check_updates(&mut self, manual: bool) {
        let req = updates::request(&self.settings.updates, manual);
        self.updates.manual = manual;
        if manual {
            self.updates.message = None;
        }
        if let Some(w) = self.updates.worker() {
            w.check(req);
        }
    }

    /// Start downloading the update found, installing it when ready if `install`.
    pub fn download_update(&mut self, install: bool) {
        let (Some(up), Ok(layout)) = (self.updates.found.clone(), self.updates.layout.clone())
        else {
            return;
        };
        self.updates.install_when_ready = install;
        if let Some(p) = self.updates.downloaded.clone() {
            self.update_downloaded(p);
            return;
        }
        self.updates.message = None;
        if let Some(w) = self.updates.worker() {
            w.download(up, layout);
        }
    }

    fn update_downloaded(&mut self, p: PathBuf) {
        self.updates.downloaded = Some(p);
        if self.updates.install_when_ready {
            self.updates.install_when_ready = false;
            self.request(Pending::Restart(RestartFor::Install));
        } else if self.settings.updates.auto_install {
            self.updates.install_on_exit = true;
            if let Some(u) = &self.updates.found {
                self.status = trf(
                    "kfx_editor.version_v_is_ready_it_is_installed_when_you_close",
                    "Version {v} is ready: it is installed when you close the editor",
                    &[("v", &u.version)],
                );
            }
        }
    }

    /// The map to open again after a restart.
    fn restart_map(&self) -> Option<PathBuf> {
        self.doc
            .as_ref()
            .filter(|_| self.doc_on_disk)
            .map(|d| d.slb_path())
    }

    /// Install the downloaded update (or roll back) and start the editor again. Called once
    /// unsaved changes are saved or discarded.
    pub(super) fn restart_for(&mut self, what: RestartFor) {
        let map = self.restart_map();
        let Ok(layout) = self.updates.layout.clone() else {
            return;
        };
        let result = match what {
            RestartFor::Install => self.install_downloaded(&layout, map.as_deref()),
            RestartFor::Rollback => rollback(&layout, &RealFs)
                .map(|_| {
                    if let Some(d) = updates::state_dir() {
                        forget_health(&d);
                    }
                    let args = map.iter().map(|m| m.clone().into_os_string()).collect();
                    (layout.exe.clone(), args)
                })
                .map_err(|e| e.to_string()),
        };
        match result {
            Ok((program, args)) => {
                // settings first: the new process reads them at start
                self.on_exit();
                match (self.updates.launcher)(&program, &args) {
                    Ok(()) => {
                        self.updates.launched = Some((program, args));
                        self.allow_close = true;
                        self.quit_requested = true;
                    }
                    Err(e) => {
                        self.updates.message = Some((
                            true,
                            trf(
                                "kfx_editor.installed_but_could_not_start_it_e_start_the_editor",
                                "Installed, but could not start it ({e}); start the editor \
                                again",
                                &[("e", &e)],
                            ),
                        ));
                    }
                }
            }
            Err(e) => {
                self.updates.dialog = true;
                self.updates.message = Some((
                    true,
                    trf(
                        "kfx_editor.the_update_failed_e_this_version_keeps_working",
                        "The update failed: {e}. This version keeps working.",
                        &[("e", &e)],
                    ),
                ));
            }
        }
    }

    /// Install the verified package: the program to start next and its arguments.
    fn install_downloaded(
        &mut self,
        layout: &Layout,
        map: Option<&Path>,
    ) -> Result<(PathBuf, Vec<OsString>), String> {
        let (Some(up), Some(file)) = (self.updates.found.clone(), self.updates.downloaded.take())
        else {
            return Err(trd("kfx_editor.nothing_downloaded", "nothing downloaded").into());
        };
        let w = self.updates.worker().ok_or_else(|| {
            trd(
                "kfx_editor.no_configuration_folder",
                "no configuration folder",
            )
        })?;
        match w
            .updater()
            .install(&up, layout, &file, map, &FsProbe, &RealFs)
            .map_err(|e| e.to_string())?
        {
            Installed::Swapped { exe } => Ok((
                exe,
                map.iter()
                    .map(|m| m.to_path_buf().into_os_string())
                    .collect(),
            )),
            Installed::RunSetup { program, args } => Ok((program, args)),
        }
    }

    /// The editor closes: an update downloaded for "install automatically" goes in now (without
    /// starting the editor again).
    pub(super) fn updates_on_exit(&mut self) {
        if !std::mem::take(&mut self.updates.install_on_exit) {
            return;
        }
        let Ok(layout) = self.updates.layout.clone() else {
            return;
        };
        match self.install_downloaded(&layout, None) {
            Ok((program, mut args)) => {
                // the installer must not start the editor again; a swap needs nothing more
                if program != layout.exe {
                    args.retain(|a| a != "/RELAUNCH");
                    let _ = (self.updates.launcher)(&program, &args);
                }
            }
            Err(e) => eprintln!("kfx-editor: the update was not installed: {e}"),
        }
    }

    /// Every frame: the worker's results, the health confirmation, and the update windows.
    pub(super) fn updates_ui(&mut self, ctx: &egui::Context) {
        self.updates_poll(ctx);
        if let StartState::Unconfirmed { health, .. } = &self.updates.health {
            if self.persist && self.updates.started.elapsed() >= HEALTHY_AFTER {
                if let Some(d) = updates::state_dir() {
                    confirm_healthy(&d, health);
                }
                self.updates.health = StartState::Normal;
            } else if self.persist {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }
        self.update_notice_ui(ctx);
        self.update_banner_ui(ctx);
        self.updates_window(ctx);
        self.crash_notice_ui(ctx);
    }

    fn updates_poll(&mut self, ctx: &egui::Context) {
        let Some(w) = &self.updates.worker else {
            return;
        };
        let st = w.status();
        if w.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        if st == Status::Idle {
            return;
        }
        w.reset();
        let manual = self.updates.manual;
        match st {
            Status::Checked(Ok(CheckOutcome::Available(up))) => {
                self.updates.banner = true;
                if self.updates.found.as_ref().map(|f| &f.version) != Some(&up.version) {
                    self.updates.downloaded = None;
                }
                let v = up.version.clone();
                self.updates.found = Some(*up);
                self.updates.message = None;
                self.status = trf(
                    "kfx_editor.version_v_of_the_editor_is_available_help_updates",
                    "Version {v} of the editor is available (Help > Updates)",
                    &[("v", &v)],
                );
                if self.settings.updates.auto_install && self.updates.blocker.is_none() {
                    self.download_update(false);
                }
            }
            Status::Checked(Ok(CheckOutcome::UpToDate { latest })) => {
                let text = trf(
                    "kfx_editor.this_version_v_is_the_newest_channel_c",
                    "This version ({v}) is the newest on the {c} channel",
                    &[
                        ("v", &crate::VERSION),
                        ("c", &channel_name(self.settings.updates.channel)),
                    ],
                );
                let _ = latest;
                if manual {
                    self.status = text.clone();
                }
                self.updates.message = Some((false, text));
            }
            Status::Checked(Ok(CheckOutcome::NotDue)) => {}
            Status::Checked(Err(e)) | Status::Failed(e) => {
                let text = trf(
                    "kfx_editor.update_check_failed_e",
                    "Update check failed: {e}",
                    &[("e", &e)],
                );
                if manual || self.updates.dialog {
                    self.status = text.clone();
                }
                self.updates.install_when_ready = false;
                self.updates.message = Some((true, text));
            }
            Status::Cancelled => {
                self.updates.install_when_ready = false;
                self.updates.message = Some((
                    false,
                    trd("kfx_editor.download_cancelled", "Download cancelled").into(),
                ));
            }
            Status::Downloaded(p) => self.update_downloaded(p),
            Status::Idle | Status::Checking | Status::Downloading { .. } => {}
        }
    }

    fn update_notice_ui(&mut self, ctx: &egui::Context) {
        // after the KeeperFX folder question of the first start, not on top of it
        if !self.updates.notice || self.first_run.open {
            return;
        }
        let mut answer = None;
        egui::Window::new(trd("kfx_editor.update_checks", "Update checks"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, EVec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(trd(
                    "kfx_editor.the_editor_can_look_for_new_versions_by_itself",
                    "The editor can look for new versions by itself: once a day it asks \
                    GitHub (api.github.com) for the list of releases of the editor. The request \
                    carries no personal data beyond what any web request has (your IP address \
                    and the editor's version). Nothing has been sent yet.",
                ));
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.updates_are_only_installed_when_you_say_so_this",
                        "Updates are only installed when you say so. This can be changed at any \
                        time in Preferences > Updates.",
                    ))
                    .color(TEXT_DIM),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(trd(
                            "kfx_editor.check_for_updates_once_a_day",
                            "Check for updates once a day",
                        ))
                        .clicked()
                    {
                        answer = Some(true);
                    }
                    if ui
                        .button(trd(
                            "kfx_editor.turn_update_checks_off",
                            "Turn update checks off",
                        ))
                        .clicked()
                    {
                        answer = Some(false);
                    }
                });
            });
        if let Some(on) = answer {
            self.updates.notice = false;
            self.settings.updates.notice_acknowledged = true;
            self.settings.updates.auto_check = on;
            self.save_settings();
            if on {
                self.check_updates(false);
            }
        }
    }

    fn update_banner_ui(&mut self, ctx: &egui::Context) {
        if !self.updates.banner || self.updates.dialog {
            return;
        }
        let Some(up) = &self.updates.found else {
            return;
        };
        let v = up.version.to_string();
        let mut act = None;
        egui::Area::new(egui::Id::unique("update-banner"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, EVec2::new(-14.0, 64.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .stroke(egui::Stroke::new(1.0, GOLD_DIM))
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.label(
                            RichText::new(trf(
                                "kfx_editor.version_v_of_the_editor_is_available",
                                "Version {v} of the editor is available",
                                &[("v", &v)],
                            ))
                            .color(GOLD),
                        );
                        if self.updates.install_on_exit {
                            ui.label(
                                RichText::new(trd(
                                    "kfx_editor.it_is_installed_when_you_close_the_editor",
                                    "It is installed when you close the editor.",
                                ))
                                .small(),
                            );
                        }
                        ui.horizontal(|ui| {
                            if ui.button(trd("kfx_editor.details", "Details…")).clicked() {
                                act = Some(0);
                            }
                            if ui.button(trd("kfx_editor.later", "Later")).clicked() {
                                act = Some(1);
                            }
                            if ui
                                .button(trd("kfx_editor.skip_this_version", "Skip this version"))
                                .clicked()
                            {
                                act = Some(2);
                            }
                        });
                    });
            });
        match act {
            Some(0) => self.updates.dialog = true,
            Some(1) => self.updates.banner = false,
            Some(2) => self.skip_found_version(),
            _ => {}
        }
    }

    fn skip_found_version(&mut self) {
        if let Some(u) = self.updates.found.take() {
            let v = u.version.to_string();
            if !self.settings.updates.skipped.contains(&v) {
                self.settings.updates.skipped.push(v);
            }
            self.save_settings();
        }
        self.updates.banner = false;
        self.updates.install_on_exit = false;
        self.updates.downloaded = None;
    }

    /// Help > Updates.
    fn updates_window(&mut self, ctx: &egui::Context) {
        if !self.updates.dialog {
            return;
        }
        let mut open = true;
        let mut act: Option<&'static str> = None;
        let progress = self.updates.worker.as_ref().and_then(|w| match w.status() {
            Status::Downloading { done, total } => Some((done, total)),
            _ => None,
        });
        let checking = self
            .updates
            .worker
            .as_ref()
            .is_some_and(|w| w.status() == Status::Checking);
        egui::Window::new(trd("kfx_editor.updates", "Updates"))
            .open(&mut open)
            .resizable(true)
            .default_size([560.0, 460.0])
            .show(ctx, |ui| {
                ui.label(trf(
                    "kfx_editor.this_version_v_channel_c",
                    "This version: {v} · channel: {c}",
                    &[
                        ("v", &crate::VERSION),
                        ("c", &channel_name(self.settings.updates.channel)),
                    ],
                ));
                if let StartState::Unconfirmed { health, failed } = &self.updates.health {
                    if *failed >= 2 {
                        ui.colored_label(
                            Color32::from_rgb(240, 160, 80),
                            trf(
                                "kfx_editor.this_version_did_not_start_properly_n_times",
                                "This version did not start properly {n} times since the update.",
                                &[("n", failed)],
                            ),
                        );
                    }
                    ui.label(trf(
                        "kfx_editor.updated_from_p_the_previous_version_is_kept_until",
                        "Updated from {p}. The previous version is kept until this one has run \
                        for a moment.",
                        &[("p", &health.previous)],
                    ));
                }
                let rb = self.updates.layout.as_ref().ok().and_then(rollback_version);
                if let Some(p) = &rb {
                    if ui
                        .button(trf(
                            "kfx_editor.roll_back_to_p",
                            "Roll back to {p}",
                            &[("p", p)],
                        ))
                        .on_hover_text(trd(
                            "kfx_editor.puts_the_previous_version_back_and_starts_it",
                            "Puts the previous version back and starts it",
                        ))
                        .clicked()
                    {
                        act = Some("rollback");
                    }
                }
                ui.separator();
                match &self.updates.found {
                    Some(up) => {
                        ui.label(
                            RichText::new(trf(
                                "kfx_editor.new_version_v",
                                "New version: {v}",
                                &[("v", &up.version)],
                            ))
                            .color(GOLD)
                            .strong(),
                        );
                        ui.label(
                            RichText::new(updates::release_page(up))
                                .small()
                                .color(TEXT_DIM),
                        );
                        ui.label(
                            RichText::new(trd("kfx_editor.release_notes", "Release notes"))
                                .strong(),
                        );
                        let notes = updates::notes_text(up.release.body.as_deref().unwrap_or(""));
                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.label(notes);
                            });
                        ui.separator();
                        if let Some((done, total)) = progress {
                            ui.add(
                                egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                                    .show_percentage(),
                            );
                            if ui.button(trd("kfx_editor.cancel", "Cancel")).clicked() {
                                act = Some("cancel");
                            }
                        } else if let Some(b) = &self.updates.blocker {
                            ui.colored_label(
                                Color32::from_rgb(240, 160, 80),
                                trf(
                                    "kfx_editor.this_copy_cannot_update_itself_b",
                                    "This copy cannot update itself: {b}",
                                    &[("b", b)],
                                ),
                            );
                            ui.horizontal(|ui| {
                                if ui
                                    .button(trd(
                                        "kfx_editor.open_the_download_page",
                                        "Open the download page",
                                    ))
                                    .clicked()
                                {
                                    act = Some("page");
                                }
                                if ui
                                    .button(trd(
                                        "kfx_editor.skip_this_version",
                                        "Skip this version",
                                    ))
                                    .clicked()
                                {
                                    act = Some("skip");
                                }
                            });
                        } else {
                            ui.horizontal(|ui| {
                                if ui
                                    .button(
                                        RichText::new(trd("kfx_editor.update_now", "Update now"))
                                            .strong(),
                                    )
                                    .on_hover_text(trd(
                                        "kfx_editor.downloads_checks_and_installs_the_new_version",
                                        "Downloads, checks and installs the new version, then \
                                        starts it again on the open map (unsaved changes are \
                                        asked about first)",
                                    ))
                                    .clicked()
                                {
                                    act = Some("now");
                                }
                                if ui.button(trd("kfx_editor.later", "Later")).clicked() {
                                    act = Some("later");
                                }
                                if ui
                                    .button(trd(
                                        "kfx_editor.skip_this_version",
                                        "Skip this version",
                                    ))
                                    .clicked()
                                {
                                    act = Some("skip");
                                }
                            });
                        }
                    }
                    None => {
                        ui.label(if checking {
                            trd("kfx_editor.checking", "Checking…")
                        } else {
                            trd(
                                "kfx_editor.no_newer_version_is_known",
                                "No newer version is known.",
                            )
                        });
                    }
                }
                if let Some((err, m)) = &self.updates.message {
                    ui.colored_label(
                        if *err {
                            Color32::from_rgb(240, 160, 80)
                        } else {
                            TEXT_DIM
                        },
                        m,
                    );
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !checking && progress.is_none(),
                            egui::Button::new(trd("kfx_editor.check_now", "Check now")),
                        )
                        .clicked()
                    {
                        act = Some("check");
                    }
                    if ui
                        .button(trd("kfx_editor.preferences_d", "Preferences…"))
                        .clicked()
                    {
                        act = Some("prefs");
                    }
                });
                ui.label(
                    RichText::new(trd(
                        "kfx_editor.downloads_are_checked_against_the_sha_256_sums",
                        "Downloads are checked against the SHA-256 sums published with the \
                        release on GitHub. They are not signed: see the manual, Updates.",
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
            });
        if !open {
            self.updates.dialog = false;
        }
        match act {
            Some("rollback") => self.request(Pending::Restart(RestartFor::Rollback)),
            Some("cancel") => {
                if let Some(w) = &self.updates.worker {
                    w.cancel();
                }
            }
            Some("page") => {
                if let Some(u) = &self.updates.found {
                    let page = updates::release_page(u);
                    if let Err(e) = super::workshop_win::open_url(&page) {
                        self.status = e;
                    }
                }
            }
            Some("skip") => {
                self.skip_found_version();
                self.updates.dialog = false;
            }
            Some("now") => self.download_update(true),
            Some("later") => {
                self.updates.dialog = false;
                self.updates.banner = false;
            }
            Some("check") => self.check_updates(true),
            Some("prefs") => {
                self.show_prefs = true;
                self.prefs_tab = "Updates".into();
            }
            _ => {}
        }
    }

    /// The last run ended in a crash: say where the log is.
    fn crash_notice_ui(&mut self, ctx: &egui::Context) {
        let Some(log) = self.updates.crash_log.clone() else {
            return;
        };
        let mut close = false;
        egui::Window::new(trd(
            "kfx_editor.the_editor_closed_unexpectedly",
            "The editor closed unexpectedly",
        ))
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.set_max_width(480.0);
            ui.label(trd(
                "kfx_editor.last_time_the_editor_stopped_because_of_an_error",
                "Last time, the editor stopped because of an error. Unsaved work may be in \
                    the crash recovery (it is offered when a map is opened). A crash log was \
                    written to:",
            ));
            ui.add(
                egui::Label::new(RichText::new(log.display().to_string()).monospace())
                    .selectable(true),
            );
            ui.label(trd(
                "kfx_editor.please_attach_it_to_a_bug_report",
                "Please attach it to a bug report (Help > About lists where to report).",
            ));
            ui.horizontal(|ui| {
                if ui
                    .button(trd("kfx_editor.open_the_folder", "Open the folder"))
                    .clicked()
                {
                    if let Some(d) = log.parent() {
                        if let Err(e) = crate::files::open_folder(d) {
                            self.status = e;
                        }
                    }
                }
                if ui.button(trd("kfx_editor.close", "Close")).clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.updates.crash_log = None;
            if let Some(d) = updates::state_dir() {
                crate::crash::mark_seen(&d);
            }
        }
    }

    /// Preferences > Updates.
    pub(super) fn prefs_updates(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new(trd("kfx_editor.updates", "Updates")).color(GOLD));
        let s: &mut UpdateSettings = &mut self.settings.updates;
        ui.checkbox(
            &mut s.auto_check,
            trd(
                "kfx_editor.check_for_updates_automatically",
                "Check for updates automatically",
            ),
        )
        .on_hover_text(trd(
            "kfx_editor.once_a_day_the_editor_asks_github_for_its_releases",
            "Once a day the editor asks GitHub (api.github.com) for its list of releases",
        ));
        ui.horizontal(|ui| {
            ui.label(trd("kfx_editor.channel", "Channel"));
            for (c, label) in [
                (Channel::PreRelease, channel_name(Channel::PreRelease)),
                (Channel::Stable, channel_name(Channel::Stable)),
            ] {
                ui.radio_value(&mut s.channel, c, label);
            }
        });
        ui.label(
            RichText::new(trd(
                "kfx_editor.every_release_so_far_is_a_pre_release_on_the_stable",
                "Every release so far is a pre-release: on the stable channel no update is \
                offered until a release is marked stable.",
            ))
            .small()
            .color(TEXT_DIM),
        );
        ui.checkbox(
            &mut s.auto_install,
            trd(
                "kfx_editor.download_and_install_automatically",
                "Download and install automatically",
            ),
        )
        .on_hover_text(trd(
            "kfx_editor.off_you_are_asked_first_on_a_new_version_is_downloaded",
            "Off: you are asked first. On: a new version is downloaded in the background and \
            installed when you close the editor",
        ));
        if !s.skipped.is_empty() {
            ui.horizontal(|ui| {
                ui.label(trf(
                    "kfx_editor.skipped_versions_v",
                    "Skipped versions: {v}",
                    &[("v", &s.skipped.join(", "))],
                ));
                if ui
                    .small_button(trd("kfx_editor.forget", "Forget"))
                    .clicked()
                {
                    s.skipped.clear();
                }
            });
        }
        if ui
            .button(trd("kfx_editor.check_now", "Check now"))
            .clicked()
        {
            self.updates.dialog = true;
            self.check_updates(true);
        }
        if let Some(b) = &self.updates.blocker {
            ui.label(
                RichText::new(trf(
                    "kfx_editor.this_copy_cannot_update_itself_b",
                    "This copy cannot update itself: {b}",
                    &[("b", b)],
                ))
                .small()
                .color(TEXT_DIM),
            );
        }
        if let Some(st) = updates::state_dir().map(kfx_update::state::StateFiles::new) {
            let state = st.load();
            if let Some(t) = state.last_check {
                let ago = kfx_update::state::now_secs().saturating_sub(t) / 60;
                ui.label(
                    RichText::new(trf(
                        "kfx_editor.last_check_n_minutes_ago",
                        "Last check: {n} minutes ago",
                        &[("n", &ago)],
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
            }
            if let Some(e) = state.last_error {
                ui.label(RichText::new(e).small().color(TEXT_DIM));
            }
        }
    }
}

pub fn channel_name(c: Channel) -> String {
    match c {
        Channel::PreRelease => trd("kfx_editor.channel_pre_release", "pre-release").into(),
        Channel::Stable => trd("kfx_editor.channel_stable", "stable").into(),
    }
}
