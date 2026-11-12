//! Campaign > Workshop…: the KeeperFX Workshop widget ([`kfx_workshop::ui::WorkshopState`]) in a
//! window, with the editor as its [`Host`]: installs go into the configured KeeperFX folder,
//! installed levels and campaigns open in the editor / campaign editor, the pre-flight of a
//! package gets the findings of the editor's validator (`kfx_map::validate` of the level, the
//! campaign checks of `kfx_campaign_ui` for a pack), URLs open in the system browser and text
//! goes to the clipboard.
//!
//! Network: nothing is asked of keeperfx.net before the user has read (once, remembered in the
//! settings) that the window does so; the widget then loads the first browse page when the
//! window is shown, everything else on a click. Headless runs and tests are always offline
//! unless a test hands in its own (mock) client.
use super::*;
use kfx_prefab::i18n::{trd, trf};
#[cfg(test)]
use kfx_workshop::client::Client;
use kfx_workshop::install::OpenTarget;
use kfx_workshop::package::{Issue as WsIssue, Severity as WsSeverity, Subject};
use kfx_workshop::ui::{Host, Tab as WsTab, WorkshopConfig, WorkshopState};

/// Open a web page in the system browser (not waited for). Only `http(s)` URLs; no shell is
/// involved (Windows: `rundll32 url.dll,FileProtocolHandler`).
pub fn open_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || url.chars().any(|c| c.is_control() || c == ' ')
    {
        return Err(trf(
            "kfx_editor.not_a_web_address_url",
            "not a web address: {url}",
            &[("url", &url)],
        ));
    }
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut c| {
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        })
        .map_err(|e| {
            trf(
                "kfx_editor.could_not_open_url_e",
                "could not open {url}: {e}",
                &[("url", &url), ("e", &e)],
            )
        })
}

fn tab_name(t: WsTab) -> &'static str {
    match t {
        WsTab::Browse => "browse",
        WsTab::Installed => "installed",
        WsTab::Publish => "publish",
    }
}

fn tab_of(s: &str) -> Option<WsTab> {
    Some(match s {
        "browse" => WsTab::Browse,
        "installed" => WsTab::Installed,
        "publish" => WsTab::Publish,
        _ => return None,
    })
}

/// Where the Workshop keeps its HTTP cache / downloads and its install records. With
/// `KFX_EDITOR_CONFIG_DIR` (tests, portable use) both live in that folder.
pub fn default_config(offline: bool) -> WorkshopConfig {
    let cfg = crate::settings::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let dirs = directories::ProjectDirs::from("net", "KeeperFX", "kfx-editor");
    let portable = std::env::var_os("KFX_EDITOR_CONFIG_DIR").is_some();
    let cache_dir = match &dirs {
        Some(d) if !portable => d.cache_dir().join("workshop"),
        _ => cfg.join("workshop-cache"),
    };
    let store_dir = match &dirs {
        Some(d) if !portable => d.data_dir().join("workshop"),
        _ => cfg.join("workshop"),
    };
    WorkshopConfig {
        cache_dir,
        store_dir,
        seed_dirs: vec![],
        offline,
    }
}

fn map_severity(s: kfx_map::validate::Severity) -> WsSeverity {
    match s {
        kfx_map::validate::Severity::Error => WsSeverity::Error,
        kfx_map::validate::Severity::Warning => WsSeverity::Warning,
        kfx_map::validate::Severity::Info => WsSeverity::Info,
    }
}

/// The editor seen by the Workshop widget. Requests that change the editor are collected and
/// carried out after the frame.
struct EditorHost<'a> {
    app: &'a EditorApp,
    ctx: egui::Context,
    install: Option<PathBuf>,
    base_gd: Option<Arc<GameData>>,
    open: Vec<OpenTarget>,
    urls: Vec<String>,
}

impl EditorHost<'_> {
    /// Validator findings of a level on disk (the open document when it is that level).
    fn level_issues(&self, folder: &Path, number: u32, out: &mut Vec<WsIssue>) {
        let tag = format!("map{number:05}");
        if let Some(d) = self
            .app
            .doc
            .as_ref()
            .filter(|d| d.doc.level == number && extras::same_dir(&d.doc.folder, folder))
        {
            if d.is_dirty() || self.app.scripts_dirty() {
                out.push(WsIssue::new(
                    WsSeverity::Warning,
                    trf(
                        "kfx_editor.tag_has_unsaved_changes_in_the_editor_the_packag",
                        "{tag} has unsaved changes in the editor: the package holds the \
                        files on disk",
                        &[("tag", &tag)],
                    ),
                ));
            }
            for i in kfx_map::validate::validate(&d.doc, &d.gd)
                .into_iter()
                .chain(document::extra_issues(&d.doc, &d.gd))
            {
                out.push(WsIssue::new(
                    map_severity(i.severity),
                    format!("{tag}: {}", i.message),
                ));
            }
            return;
        }
        let (Some(install), Some(gd)) = (&self.install, &self.base_gd) else {
            return;
        };
        let loaded = document::locate(install, folder, number).and_then(|loc| {
            kfx_map::mapdoc::MapDoc::load(folder, number, loc.dims, gd).map_err(|e| e.to_string())
        });
        match loaded {
            Ok(m) => {
                for i in kfx_map::validate::validate(&m, gd) {
                    out.push(WsIssue::new(
                        map_severity(i.severity),
                        format!("{tag}: {}", i.message),
                    ));
                }
            }
            Err(e) => out.push(WsIssue::new(
                WsSeverity::Warning,
                trf(
                    "kfx_editor.tag_could_not_be_checked_e",
                    "{tag} could not be checked: {e}",
                    &[("tag", &tag), ("e", &e)],
                ),
            )),
        }
    }
}

impl Host for EditorHost<'_> {
    fn install_root(&self) -> Option<PathBuf> {
        self.install.clone()
    }
    fn open_in_editor(&mut self, target: &OpenTarget) {
        self.open.push(target.clone());
    }
    fn copy_to_clipboard(&mut self, text: &str) {
        self.ctx.copy_text(text.to_string());
    }
    fn open_url(&mut self, url: &str) {
        self.urls.push(url.to_string());
    }
    fn publish_subjects(&self) -> Vec<(String, Subject)> {
        let mut v = vec![];
        if let Some(d) = self.app.doc.as_ref().filter(|_| self.app.doc_on_disk) {
            v.push((
                trf(
                    "kfx_editor.open_level_map_level_folder",
                    "Open level map{level} ({folder})",
                    &[
                        ("level", &format!("{:05}", d.doc.level)),
                        ("folder", &d.doc.folder.display()),
                    ],
                ),
                Subject::Level {
                    folder: d.doc.folder.clone(),
                    number: d.doc.level,
                },
            ));
        }
        let mut packs: Vec<(PathBuf, PathBuf)> = vec![];
        if let (Some(d), Some(install)) = (self.app.doc.as_ref(), &self.install) {
            if let Some((cfg, _)) = &d.pack_cfg {
                packs.push((install.clone(), cfg.clone()));
            }
        }
        if let Some(e) = &self.app.camp.editor {
            if !packs
                .iter()
                .any(|(_, c)| extras::same_dir(c, &e.pack.cfg_path))
            {
                packs.push((e.pack.install.clone(), e.pack.cfg_path.clone()));
            }
        }
        for (install_root, cfg) in packs {
            let name = cfg
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            v.push((
                trf(
                    "kfx_editor.campaign_map_pack_name",
                    "Campaign / map pack {name}",
                    &[("name", &name)],
                ),
                Subject::Campaign { install_root, cfg },
            ));
        }
        v
    }
    fn validator_issues(&self, subject: &Subject) -> Vec<WsIssue> {
        let mut out = vec![];
        match subject {
            Subject::Level { folder, number } => self.level_issues(folder, *number, &mut out),
            Subject::Campaign { install_root, cfg } => {
                let pack = match self
                    .app
                    .camp
                    .editor
                    .as_ref()
                    .filter(|e| extras::same_dir(&e.pack.cfg_path, cfg))
                {
                    Some(e) => {
                        if e.pack.is_dirty() || e.texts.is_dirty() {
                            out.push(WsIssue::new(
                                WsSeverity::Warning,
                                trd(
                                    "kfx_editor.the_campaign_has_unsaved_changes_in_the_campaign",
                                    "the campaign has unsaved changes in the campaign \
                                    editor: the package holds the files on disk",
                                ),
                            ));
                        }
                        Some(e.pack.clone())
                    }
                    None => kfx_campaign_ui::Pack::load(install_root, cfg).ok(),
                };
                let Some(pack) = pack else {
                    out.push(WsIssue::new(
                        WsSeverity::Error,
                        trf(
                            "kfx_editor.cfg_could_not_be_read",
                            "{cfg} could not be read",
                            &[("cfg", &cfg.display())],
                        ),
                    ));
                    return out;
                };
                for i in kfx_campaign_ui::validate(&pack) {
                    let s = match i.severity {
                        kfx_campaign_ui::Severity::Error => WsSeverity::Error,
                        kfx_campaign_ui::Severity::Warning => WsSeverity::Warning,
                        kfx_campaign_ui::Severity::Info => WsSeverity::Info,
                    };
                    out.push(WsIssue::new(s, i.message));
                }
                if let Some(dir) = pack.levels_dir() {
                    // the maps themselves (base configuration; at most 60 levels)
                    for e in pack.entries().iter().filter(|e| e.exists).take(60) {
                        self.level_issues(&dir, e.number, &mut out);
                    }
                }
            }
        }
        out
    }
}

impl EditorApp {
    /// Show or hide the Workshop window.
    pub fn show_workshop(&mut self, open: bool) {
        self.settings.layout.campaign.workshop_open = open;
    }

    /// The Workshop widget once the window has been used (tests).
    #[cfg(test)]
    pub fn workshop_state(&mut self) -> Option<&mut WorkshopState> {
        self.camp.workshop.as_mut()
    }

    fn make_workshop(&mut self) -> WorkshopState {
        let offline = self.settings.workshop.offline
            || (self.camp.dry_run && self.camp.workshop_client.is_none());
        let mut cfg = self
            .camp
            .workshop_config
            .clone()
            .unwrap_or_else(|| default_config(offline));
        cfg.offline |= offline;
        let mut ws = match self.camp.workshop_client.clone() {
            Some(c) => WorkshopState::with_client(cfg, c),
            None => WorkshopState::new(cfg),
        };
        if self.camp.dry_run && self.camp.workshop_client.is_none() {
            ws.client().set_offline(true);
        }
        if let Some(t) = tab_of(&self.settings.layout.campaign.workshop_tab) {
            ws.tab = t;
        }
        ws
    }

    /// Use this client (a mock transport in tests) when the Workshop state is made.
    #[cfg(test)]
    pub fn set_workshop_client(&mut self, cfg: WorkshopConfig, client: Arc<Client>) {
        self.camp.workshop = None;
        self.camp.workshop_config = Some(cfg);
        self.camp.workshop_client = Some(client);
        self.camp.dry_run = true;
    }

    pub(super) fn workshop_window(&mut self, ctx: &egui::Context) {
        if !self.settings.layout.campaign.workshop_open {
            return;
        }
        let mut open = true;
        if !self.settings.workshop.acknowledged {
            let mut go = false;
            let mut later = false;
            let mut offline = self.settings.workshop.offline;
            egui::Window::new(trd("kfx_editor.workshop", "Workshop"))
                .id(egui::Id::unique("kfx-workshop-notice"))
                .open(&mut open)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .collapsible(false)
                .resizable(false)
                .default_width(520.0)
                .show(ctx, |ui| {
                    ui.heading(
                        RichText::new(trd("kfx_editor.keeperfx_workshop", "KeeperFX Workshop"))
                            .color(GOLD),
                    );
                    ui.label(trd(
                        "kfx_editor.the_workshop_window_reads_keeperfx_net_the_item",
                        "The Workshop window reads keeperfx.net: the item lists, item \
                        pages, pictures and the files you choose to download.",
                    ));
                    ui.label(trd(
                        "kfx_editor.nothing_is_sent_but_these_requests_no_login_no_c",
                        "Nothing is sent but these requests: no login, no cookies, no \
                        upload. Publishing makes a zip and opens the upload page in your own \
                        browser.",
                    ));
                    ui.label(trd(
                        "kfx_editor.answers_are_kept_in_a_cache_so_items_you_have_se",
                        "Answers are kept in a cache, so items you have seen also work \
                        offline.",
                    ));
                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut offline,
                        trd(
                            "kfx_editor.offline_use_only_the_cache_never_the_network",
                            "Offline: use only the cache, never the network.",
                        ),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(trd("kfx_editor.continue", "Continue")).clicked() {
                            go = true;
                        }
                        if ui.button(trd("kfx_editor.not_now", "Not now")).clicked() {
                            later = true;
                        }
                    });
                });
            self.settings.workshop.offline = offline;
            if go {
                self.settings.workshop.acknowledged = true;
                self.save_settings();
            }
            if later || !open {
                self.settings.layout.campaign.workshop_open = false;
            }
            return;
        }
        if self.camp.workshop.is_none() {
            self.camp.workshop = Some(self.make_workshop());
        }
        let base_gd = self
            .install()
            .map(Path::to_path_buf)
            .and_then(|i| self.gamedata(&document::base_layers(&i)).ok());
        let Some(mut ws) = self.camp.workshop.take() else {
            return;
        };
        let mut host = EditorHost {
            app: self,
            ctx: ctx.clone(),
            install: self.install().map(Path::to_path_buf),
            base_gd,
            open: vec![],
            urls: vec![],
        };
        let max = ctx.content_rect().size() - egui::vec2(40.0, 80.0);
        let size =
            egui::vec2(1150.0f32.min(max.x), 720.0f32.min(max.y)).max(egui::vec2(400.0, 300.0));
        egui::Window::new(trd("kfx_editor.workshop", "Workshop"))
            .id(egui::Id::unique("kfx-workshop"))
            .open(&mut open)
            .default_size(size)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .min_size([420.0, 320.0])
            .show(ctx, |ui| {
                ui.set_min_size(ui.available_size());
                ws.show(ui, &mut host);
            });
        let EditorHost {
            open: targets,
            urls,
            ..
        } = host;
        self.settings.layout.campaign.workshop_tab = tab_name(ws.tab).to_string();
        if !self.camp.dry_run {
            self.settings.workshop.offline = ws.client().is_offline();
        }
        self.camp.workshop = Some(ws);
        if !open {
            self.settings.layout.campaign.workshop_open = false;
        }
        for u in urls {
            self.camp.opened_urls.push(u.clone());
            if self.camp.dry_run || cfg!(test) {
                self.status = trf(
                    "kfx_editor.would_open_u_in_the_browser",
                    "Would open {u} in the browser",
                    &[("u", &u)],
                );
            } else if let Err(e) = open_url(&u) {
                self.status = e;
            }
        }
        for t in targets {
            match t {
                OpenTarget::Level { slb, .. } => self.request(Pending::Open(slb)),
                OpenTarget::Campaign { cfg } => {
                    if let Err(e) = self.open_campaign_cfg(&cfg) {
                        self.status = e;
                    }
                }
            }
        }
    }
}

/// `--actions`: `{"workshop": {"open": true, "acknowledge": true, "tab": "installed",
/// "cache": "<dir>", "close": true}}`. Always offline.
pub fn run_actions(
    app: &mut EditorApp,
    obj: &serde_json::Map<String, serde_json::Value>,
    rel: &dyn Fn(&str) -> PathBuf,
) -> Result<(), String> {
    let Some(v) = obj.get("workshop").and_then(|v| v.as_object()) else {
        return Ok(());
    };
    app.camp.dry_run = true;
    if let Some(c) = v.get("cache").and_then(|x| x.as_str()) {
        // read an existing cache (offline): a private store so nothing of the user's changes
        let store = crate::settings::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("workshop-headless");
        app.camp.workshop_config = Some(WorkshopConfig {
            cache_dir: rel(c),
            store_dir: store,
            seed_dirs: vec![],
            offline: true,
        });
        app.camp.workshop = None;
    }
    if let Some(b) = v.get("acknowledge").and_then(|x| x.as_bool()) {
        app.settings.workshop.acknowledged = b;
    }
    if let Some(b) = v.get("open").and_then(|x| x.as_bool()) {
        app.show_workshop(b);
    }
    if let Some(t) = v.get("tab").and_then(|x| x.as_str()) {
        let tab = tab_of(t).ok_or_else(|| format!("unknown workshop tab {t}"))?;
        app.settings.layout.campaign.workshop_tab = t.to_string();
        if let Some(ws) = app.camp.workshop.as_mut() {
            ws.tab = tab;
        }
    }
    if let Some(q) = v.get("category").and_then(|x| x.as_str()) {
        let cat = kfx_workshop::model::Category::ALL
            .into_iter()
            .find(|c| c.label().eq_ignore_ascii_case(q))
            .ok_or_else(|| format!("unknown workshop category {q}"))?;
        if app.camp.workshop.is_none() {
            app.camp.workshop = Some(app.make_workshop());
        }
        if let Some(ws) = app.camp.workshop.as_mut() {
            ws.set_query(kfx_workshop::model::BrowseQuery {
                category: Some(cat),
                ..Default::default()
            });
        }
    }
    if v.get("close").and_then(|x| x.as_bool()) == Some(true) {
        app.show_workshop(false);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_are_opened() {
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("https://x y").is_err());
        assert!(open_url("javascript:alert(1)").is_err());
        assert!(open_url("https://keeperfx.net/\n").is_err());
    }
}
