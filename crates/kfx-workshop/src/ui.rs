//! The host-independent egui widget: Browse, Installed and Publish tabs.
//!
//! All blocking work (HTTP, hashing, archive reading, installing, zipping) runs on two worker
//! threads owned by [`WorkshopState`] (`net`: browse, item pages, images, update checks; `task`:
//! downloads, planning, applying, uninstalling, packaging). Every job carries a cancel flag.
//! The widget never touches the network on the UI thread.

use crate::archive::Limits;
use crate::classify::{self, Inspected};
use crate::client::{Client, ClientConfig, Downloaded, Fetched, Policy, Source};
use crate::install::{
    self, Action, Conflict, ItemRef, Manifest, OpenTarget, PackTarget, PlanOptions, Store,
    UpdateStatus,
};
use crate::model::*;
use crate::package::{self, Issue, PackageOptions, PackagePlan, Severity, Subject};
use egui::{vec2, Color32, RichText};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// What the widget needs from the application around it.
pub trait Host {
    /// The KeeperFX install to install into / package from.
    fn install_root(&self) -> Option<PathBuf>;
    /// Open an installed level or campaign in the editor.
    fn open_in_editor(&mut self, target: &OpenTarget);
    fn copy_to_clipboard(&mut self, text: &str);
    fn open_url(&mut self, url: &str);
    /// Levels and campaigns that can be packaged: (label, subject).
    fn publish_subjects(&self) -> Vec<(String, Subject)>;
    /// Validator findings for a subject (shown in the pre-flight).
    fn validator_issues(&self, subject: &Subject) -> Vec<Issue>;
}

#[derive(Clone, Debug)]
pub struct WorkshopConfig {
    /// Folder of the HTTP cache and the downloaded archives (the editor uses a `workshop`
    /// folder in the platform's cache directory).
    pub cache_dir: PathBuf,
    /// Folder of the install manifests and backups (the editor uses a `workshop` folder in the
    /// platform's data directory).
    pub store_dir: PathBuf,
    /// Read-only survey-layout folders consulted before the network.
    pub seed_dirs: Vec<PathBuf>,
    pub offline: bool,
}

/// Last-resort clean-up of text from the Workshop, archives or the disk before egui shows it.
/// Every printable character is kept: the editor gives egui the system's fallback fonts
/// (`kfx-fonts`), and a character that no font has is drawn as a box (egui's default missing-glyph
/// policy; only test harnesses panic on one). What is replaced by `?` is what would break the
/// layout or mislead: control characters other than newline and tab, and the bidirectional
/// overrides / isolates / marks that can make a file name read differently from what it is.
/// (The data itself is kept untouched.)
pub fn safe_text(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' | '\t' => c,
            c if c.is_control() => '?',
            '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => '?',
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod safe_text_tests {
    use super::safe_text;

    #[test]
    fn keeps_every_script_and_replaces_controls() {
        assert_eq!(safe_text("Карта 地图 ★ é\tok\n"), "Карта 地图 ★ é\tok\n");
        assert_eq!(safe_text("a\u{0}b\u{1b}[31m\u{85}"), "a?b?[31m?");
        assert_eq!(safe_text("evil\u{202e}txt.exe"), "evil?txt.exe");
    }
}

// ---------------------------------------------------------------- worker

enum Job {
    Browse {
        q: BrowseQuery,
        policy: Policy,
    },
    Item {
        id: u32,
        slug: String,
        policy: Policy,
    },
    Image {
        key: String,
        path: String,
        max_w: u32,
    },
    Updates {
        items: Vec<(String, u32, String)>,
    },
    Download {
        item: ItemRef,
        file: FileEntry,
        dest: PathBuf,
    },
    Plan {
        archive: PathBuf,
        install: PathBuf,
        opts: Box<PlanOptions>,
        insp: Option<Arc<Inspected>>,
    },
    Apply {
        install: PathBuf,
        plan: Box<install::InstallPlan>,
    },
    Uninstall {
        install: PathBuf,
        id: String,
        force: bool,
    },
    Gather {
        subject: Subject,
        opts: Box<PackageOptions>,
    },
    Package {
        plan: Box<PackagePlan>,
        dest: PathBuf,
        force: bool,
    },
}

enum Event {
    Browse {
        q: BrowseQuery,
        res: Result<Fetched<Page>, String>,
    },
    Item {
        id: u32,
        res: Result<Fetched<ItemDetail>, String>,
    },
    Image {
        key: String,
        res: Result<egui::ColorImage, String>,
    },
    Updates(Vec<(String, Result<UpdateStatus, String>)>),
    Progress(Progress),
    Downloaded {
        item: ItemRef,
        res: Result<Downloaded, String>,
    },
    Planned {
        res: Result<(Arc<Inspected>, Box<install::InstallPlan>), String>,
    },
    Applied {
        res: Result<Manifest, String>,
    },
    Uninstalled {
        id: String,
        res: Result<install::UninstallReport, String>,
    },
    Gathered {
        res: Result<Box<PackagePlan>, String>,
    },
    Packaged {
        res: Result<package::Packaged, String>,
    },
}

struct Worker {
    tx: Option<Sender<(Job, Arc<AtomicBool>)>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    fn spawn(
        name: &str,
        client: Arc<Client>,
        store: Store,
        events: Sender<Event>,
        ctx: Arc<Mutex<Option<egui::Context>>>,
    ) -> Worker {
        let (tx, rx): (Sender<(Job, Arc<AtomicBool>)>, Receiver<_>) = channel();
        let handle = std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                while let Ok((job, cancel)) = rx.recv() {
                    if cancel.load(Ordering::SeqCst) {
                        continue;
                    }
                    let emit = |e: Event| {
                        let _ = events.send(e);
                        if let Ok(g) = ctx.lock() {
                            if let Some(c) = g.as_ref() {
                                c.request_repaint();
                            }
                        }
                    };
                    run_job(&client, &store, job, &cancel, &emit);
                }
            })
            .ok();
        Worker {
            tx: Some(tx),
            handle,
        }
    }
    fn send(&self, job: Job, cancel: &Arc<AtomicBool>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send((job, cancel.clone()));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.tx = None; // ends the loop after the current job
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn decode_thumb(bytes: &[u8], max_w: u32) -> std::result::Result<egui::ColorImage, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let img = if img.width() > max_w {
        img.resize(max_w, max_w * 3 / 2, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

fn run_job(
    client: &Client,
    store: &Store,
    job: Job,
    cancel: &Arc<AtomicBool>,
    emit: &dyn Fn(Event),
) {
    let es = |e: crate::Error| e.to_string();
    match job {
        Job::Browse { q, policy } => {
            let res = client.browse(&q, policy).map_err(es);
            emit(Event::Browse { q, res });
        }
        Job::Item { id, slug, policy } => {
            let res = client.item(id, &slug, policy).map_err(es);
            emit(Event::Item { id, res });
        }
        Job::Image { key, path, max_w } => {
            let res = client
                .image(&path, Policy::PreferCache)
                .map_err(es)
                .and_then(|f| decode_thumb(&f.value, max_w));
            emit(Event::Image { key, res });
        }
        Job::Updates { items } => {
            let mut out = vec![];
            for (mid, item_id, slug) in items {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                let res = client
                    .item(item_id, &slug, Policy::Refresh)
                    .map_err(es)
                    .and_then(|d| {
                        store
                            .load(&mid)
                            .map(|m| install::check_update(&m, &d.value))
                            .map_err(es)
                    });
                out.push((mid, res));
            }
            emit(Event::Updates(out));
        }
        Job::Download { item, file, dest } => {
            let res = client
                .download(item.id, &file, &dest, &|p| emit(Event::Progress(p)), cancel)
                .map_err(es);
            emit(Event::Downloaded { item, res });
        }
        Job::Plan {
            archive,
            install,
            opts,
            insp,
        } => {
            let res = (|| -> crate::Result<(Arc<Inspected>, Box<install::InstallPlan>)> {
                let insp = match insp {
                    Some(i) => i,
                    None => Arc::new(classify::classify_archive(&archive, &Limits::default())?),
                };
                let plan = install::plan(&install, &archive, &insp, &opts)?;
                Ok((insp, Box::new(plan)))
            })()
            .map_err(es);
            emit(Event::Planned { res });
        }
        Job::Apply { install, plan } => {
            let res = install::apply(&install, &plan, store, &Limits::default()).map_err(es);
            emit(Event::Applied { res });
        }
        Job::Uninstall { install, id, force } => {
            let res = install::uninstall(&install, &id, store, force).map_err(es);
            emit(Event::Uninstalled { id, res });
        }
        Job::Gather { subject, opts } => {
            let res = package::gather(&subject, &opts).map(Box::new).map_err(es);
            emit(Event::Gathered { res });
        }
        Job::Package { plan, dest, force } => {
            let res = package::write_zip(&plan, &dest, force).map_err(es);
            emit(Event::Packaged { res });
        }
    }
}

// ---------------------------------------------------------------- state

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Browse,
    Installed,
    Publish,
}

enum Thumb {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

struct PlanUi {
    archive: PathBuf,
    insp: Arc<Inspected>,
    plan: Box<install::InstallPlan>,
    opts: PlanOptions,
    packs: Vec<String>,
    new_pack_name: String,
    pack_choice: usize, // 0 default, 1.. existing, last = new
}

#[derive(Default)]
struct PublishUi {
    subject_idx: usize,
    title: String,
    author: String,
    description: String,
    layout_idx: usize,
    include_sidecar: bool,
    plan: Option<Box<PackagePlan>>,
    zip_path: String,
    result: Option<package::Packaged>,
    force: bool,
}

pub struct WorkshopState {
    cfg: WorkshopConfig,
    client: Arc<Client>,
    store: Store,
    net: Worker,
    task: Worker,
    events: Receiver<Event>,
    ctx: Arc<Mutex<Option<egui::Context>>>,
    pub tab: Tab,
    query: BrowseQuery,
    search_text: String,
    page: Option<Page>,
    page_source: Option<Source>,
    loading_page: bool,
    started: bool,
    pub selected: Option<ItemSummary>,
    detail: Option<ItemDetail>,
    loading_detail: bool,
    thumbs: HashMap<String, Thumb>,
    pub status: String,
    pub error: Option<String>,
    download: Option<(ItemRef, Progress, Arc<AtomicBool>)>,
    planning: bool,
    plan_ui: Option<PlanUi>,
    pub last_install: Option<Manifest>,
    installed: Vec<Manifest>,
    updates: HashMap<String, Result<UpdateStatus, String>>,
    checking_updates: bool,
    confirm_uninstall: Option<String>,
    busy_install: bool,
    publish: PublishUi,
    cancels: Vec<Arc<AtomicBool>>,
}

impl WorkshopState {
    pub fn new(cfg: WorkshopConfig) -> Self {
        let mut cc = ClientConfig::new(cfg.cache_dir.clone());
        cc.seed_dirs = cfg.seed_dirs.clone();
        cc.offline = cfg.offline;
        let client = Arc::new(Client::new(cc));
        Self::with_client(cfg, client)
    }

    pub fn with_client(cfg: WorkshopConfig, client: Arc<Client>) -> Self {
        let store = Store::new(cfg.store_dir.clone());
        let (etx, erx) = channel();
        let ctx: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
        let net = Worker::spawn(
            "kfx-workshop-net",
            client.clone(),
            store.clone(),
            etx.clone(),
            ctx.clone(),
        );
        let task = Worker::spawn(
            "kfx-workshop-task",
            client.clone(),
            store.clone(),
            etx,
            ctx.clone(),
        );
        let installed = store.list();
        WorkshopState {
            query: BrowseQuery {
                page: 1,
                ..Default::default()
            },
            cfg,
            client,
            store,
            net,
            task,
            events: erx,
            ctx,
            tab: Tab::Browse,
            search_text: String::new(),
            page: None,
            page_source: None,
            loading_page: false,
            started: false,
            selected: None,
            detail: None,
            loading_detail: false,
            thumbs: HashMap::new(),
            status: String::new(),
            error: None,
            download: None,
            planning: false,
            plan_ui: None,
            last_install: None,
            installed,
            updates: HashMap::new(),
            checking_updates: false,
            confirm_uninstall: None,
            busy_install: false,
            publish: PublishUi {
                include_sidecar: false,
                ..Default::default()
            },
            cancels: vec![],
        }
    }

    /// Show an item's details (deep link from the host).
    pub fn open_item(&mut self, id: u32, slug: &str, title: &str) {
        self.tab = Tab::Browse;
        self.select(ItemSummary {
            id,
            slug: slug.to_string(),
            title: title.to_string(),
            ..Default::default()
        });
    }
    /// The details of the selected item once loaded.
    pub fn detail(&self) -> Option<&ItemDetail> {
        self.detail.as_ref()
    }
    /// Record an update-check result (the Installed tab shows it).
    pub fn set_update_status(&mut self, manifest_id: &str, status: Result<UpdateStatus, String>) {
        self.updates.insert(manifest_id.to_string(), status);
    }
    /// Reload the Installed list from the store.
    pub fn refresh_installed(&mut self) {
        self.installed = self.store.list();
    }
    /// The install plan waiting for confirmation, if any.
    pub fn pending_plan(&self) -> Option<&install::InstallPlan> {
        self.plan_ui.as_ref().map(|p| &*p.plan)
    }
    pub fn browse_page(&self) -> Option<&Page> {
        self.page.as_ref()
    }

    pub fn client(&self) -> &Arc<Client> {
        &self.client
    }
    pub fn installed(&self) -> &[Manifest] {
        &self.installed
    }
    pub fn is_busy(&self) -> bool {
        self.loading_page
            || self.loading_detail
            || self.download.is_some()
            || self.planning
            || self.busy_install
            || self.checking_updates
    }
    pub fn set_query(&mut self, q: BrowseQuery) {
        self.search_text = q.text.clone();
        self.query = q;
        self.started = false;
    }

    fn new_cancel(&mut self) -> Arc<AtomicBool> {
        let c = Arc::new(AtomicBool::new(false));
        self.cancels.retain(|x| Arc::strong_count(x) > 1);
        self.cancels.push(c.clone());
        c
    }

    /// Cancel everything in flight.
    pub fn cancel_all(&mut self) {
        for c in &self.cancels {
            c.store(true, Ordering::SeqCst);
        }
        self.cancels.clear();
        self.loading_page = false;
        self.loading_detail = false;
        self.download = None;
        self.planning = false;
        self.busy_install = false;
        self.checking_updates = false;
        self.status = "cancelled".into();
    }

    fn load_page(&mut self, policy: Policy) {
        self.loading_page = true;
        let c = self.new_cancel();
        self.net.send(
            Job::Browse {
                q: self.query.clone(),
                policy,
            },
            &c,
        );
    }

    fn select(&mut self, it: ItemSummary) {
        self.detail = None;
        self.loading_detail = true;
        let c = self.new_cancel();
        self.net.send(
            Job::Item {
                id: it.id,
                slug: it.slug.clone(),
                policy: Policy::PreferCache,
            },
            &c,
        );
        self.selected = Some(it);
    }

    fn want_image(&mut self, path: &str, max_w: u32) {
        if self.thumbs.contains_key(path) {
            return;
        }
        // keep a few in flight; the rest wait for a later frame
        let loading = self
            .thumbs
            .values()
            .filter(|t| matches!(t, Thumb::Loading))
            .count();
        if loading >= 8 {
            return;
        }
        self.thumbs.insert(path.to_string(), Thumb::Loading);
        let c = self.new_cancel();
        self.net.send(
            Job::Image {
                key: path.to_string(),
                path: path.to_string(),
                max_w,
            },
            &c,
        );
    }

    /// Download a file of the currently shown item, then plan its installation (the plan
    /// window opens when done).
    pub fn start_download(&mut self, f: &FileEntry) {
        let Some(d) = &self.detail else { return };
        let item = ItemRef::from_file(d.id, &d.title, &d.slug, f);
        let dest = self
            .cfg
            .cache_dir
            .join("downloads")
            .join(d.id.to_string())
            .join(f.id.to_string());
        let c = self.new_cancel();
        self.download = Some((item.clone(), Progress::default(), c.clone()));
        self.error = None;
        self.status = format!("downloading {}", f.name);
        self.task.send(
            Job::Download {
                item,
                file: f.clone(),
                dest,
            },
            &c,
        );
    }

    fn start_plan(
        &mut self,
        archive: PathBuf,
        install: PathBuf,
        opts: PlanOptions,
        insp: Option<Arc<Inspected>>,
    ) {
        self.planning = true;
        let c = self.new_cancel();
        self.task.send(
            Job::Plan {
                archive,
                install,
                opts: Box::new(opts),
                insp,
            },
            &c,
        );
    }

    fn poll(&mut self, host: &mut dyn Host) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                Event::Browse { q, res } => {
                    if q != self.query {
                        continue;
                    }
                    self.loading_page = false;
                    match res {
                        Ok(f) => {
                            self.page_source = Some(f.source);
                            self.page = Some(f.value);
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Event::Item { id, res } => {
                    if self.selected.as_ref().map(|s| s.id) != Some(id) {
                        continue;
                    }
                    self.loading_detail = false;
                    match res {
                        Ok(f) => self.detail = Some(f.value),
                        Err(e) => self.error = Some(e),
                    }
                }
                Event::Image { key, res } => {
                    let ctx = self.ctx.lock().ok().and_then(|g| g.clone());
                    let t = match (res, ctx) {
                        (Ok(img), Some(ctx)) => {
                            Thumb::Ready(ctx.load_texture(&key, img, egui::TextureOptions::LINEAR))
                        }
                        _ => Thumb::Failed,
                    };
                    self.thumbs.insert(key, t);
                }
                Event::Updates(v) => {
                    self.checking_updates = false;
                    for (id, r) in v {
                        self.updates.insert(id, r);
                    }
                }
                Event::Progress(p) => {
                    if let Some((_, pp, _)) = &mut self.download {
                        *pp = p;
                    }
                }
                Event::Downloaded { item, res } => {
                    self.download = None;
                    match res {
                        Ok(d) => {
                            self.status = format!(
                                "downloaded {} ({} bytes, sha256 {})",
                                item.file_name,
                                d.size,
                                &d.sha256[..12]
                            );
                            let Some(root) = host.install_root() else {
                                self.error = Some("no KeeperFX install is configured".into());
                                continue;
                            };
                            let mut opts = PlanOptions::new(&item.title);
                            opts.item = Some(item);
                            self.start_plan(d.path, root, opts, None);
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Event::Planned { res } => {
                    self.planning = false;
                    match res {
                        Ok((insp, plan)) => {
                            let root = host.install_root().unwrap_or_default();
                            let opts = match &self.plan_ui {
                                Some(p) if p.archive == plan.archive => p.opts.clone(),
                                _ => {
                                    let mut o = PlanOptions::new(&plan.name);
                                    o.item = plan.item.clone();
                                    o
                                }
                            };
                            let packs = install::existing_packs(&root);
                            let (nn, pc) = match &self.plan_ui {
                                Some(p) if p.archive == plan.archive => {
                                    (p.new_pack_name.clone(), p.pack_choice)
                                }
                                _ => (crate::util::slug(&plan.name), 0),
                            };
                            self.plan_ui = Some(PlanUi {
                                archive: plan.archive.clone(),
                                insp,
                                plan,
                                opts,
                                packs,
                                new_pack_name: nn,
                                pack_choice: pc,
                            });
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Event::Applied { res } => {
                    self.busy_install = false;
                    match res {
                        Ok(m) => {
                            self.status = format!("installed {} ({} files)", m.name, m.files.len());
                            self.installed = self.store.list();
                            self.last_install = Some(m);
                            self.plan_ui = None;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Event::Uninstalled { id, res } => {
                    self.busy_install = false;
                    match res {
                        Ok(r) => {
                            self.status = format!(
                                "uninstalled {id}: {} removed, {} restored, {} kept (modified)",
                                r.removed.len(),
                                r.restored.len(),
                                r.kept_modified.len()
                            );
                            self.installed = self.store.list();
                            self.updates.remove(&id);
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Event::Gathered { res } => match res {
                    Ok(p) => {
                        self.publish.zip_path = self
                            .cfg
                            .cache_dir
                            .join("packages")
                            .join(&p.zip_name)
                            .display()
                            .to_string();
                        self.publish.plan = Some(p);
                        self.publish.result = None;
                    }
                    Err(e) => self.error = Some(e),
                },
                Event::Packaged { res } => {
                    self.busy_install = false;
                    match res {
                        Ok(p) => {
                            self.status = format!(
                                "wrote {} ({} files, {})",
                                p.zip.display(),
                                p.files,
                                crate::util::format_size(p.size)
                            );
                            self.publish.result = Some(p);
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        }
    }

    /// Draw the widget.
    pub fn show(&mut self, ui: &mut egui::Ui, host: &mut dyn Host) {
        if let Ok(mut g) = self.ctx.lock() {
            if g.is_none() {
                *g = Some(ui.ctx().clone());
            }
        }
        self.poll(host);
        if !self.started {
            self.started = true;
            self.load_page(Policy::PreferCache);
        }
        egui::Panel::top("kfx-ws-top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Browse, "Browse");
                ui.selectable_value(
                    &mut self.tab,
                    Tab::Installed,
                    format!("Installed ({})", self.installed.len()),
                );
                ui.selectable_value(&mut self.tab, Tab::Publish, "Publish");
                ui.separator();
                let mut off = self.client.is_offline();
                if ui
                    .checkbox(&mut off, "Offline")
                    .on_hover_text("Use only the cache")
                    .changed()
                {
                    self.client.set_offline(off);
                }
                if self.is_busy() {
                    ui.spinner();
                    if ui.button("Cancel").clicked() {
                        self.cancel_all();
                    }
                }
                ui.label(RichText::new(safe_text(&self.status)).weak());
            });
            if let Some(e) = self.error.clone() {
                ui.horizontal(|ui| {
                    ui.colored_label(Color32::from_rgb(220, 90, 80), safe_text(&e));
                    if ui.small_button("Dismiss").clicked() {
                        self.error = None;
                    }
                });
            }
        });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Browse => self.browse_tab(ui, host),
            Tab::Installed => self.installed_tab(ui, host),
            Tab::Publish => self.publish_tab(ui, host),
        });
        self.plan_window(ui, host);
    }

    // ---- browse ----

    fn browse_tab(&mut self, ui: &mut egui::Ui, _host: &mut dyn Host) {
        let mut reload = None::<Policy>;
        ui.horizontal_wrapped(|ui| {
            let cat_label = self.query.category.map_or("All categories", |c| c.label());
            egui::ComboBox::from_id_salt("ws-cat")
                .selected_text(cat_label)
                .show_ui(ui, |ui| {
                    let mut ch = ui
                        .selectable_value(&mut self.query.category, None, "All categories")
                        .changed();
                    for c in Category::ALL {
                        ch |= ui
                            .selectable_value(&mut self.query.category, Some(c), c.label())
                            .changed();
                    }
                    if ch {
                        self.query.page = 1;
                        reload = Some(Policy::PreferCache);
                    }
                });
            egui::ComboBox::from_id_salt("ws-sort")
                .selected_text(self.query.sort.label())
                .show_ui(ui, |ui| {
                    let mut ch = false;
                    for s in Sort::ALL {
                        ch |= ui
                            .selectable_value(&mut self.query.sort, s, s.label())
                            .changed();
                    }
                    if ch {
                        self.query.page = 1;
                        reload = Some(Policy::PreferCache);
                    }
                });
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.search_text)
                    .hint_text("Search")
                    .desired_width(180.0),
            );
            if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                | ui.button("Search").clicked()
            {
                self.query.text = self.search_text.clone();
                self.query.page = 1;
                reload = Some(Policy::PreferCache);
            }
            if ui
                .button("Refresh")
                .on_hover_text("Ask the Workshop again (ignores the cache)")
                .clicked()
            {
                reload = Some(Policy::Refresh);
            }
            if let Some(p) = &self.page {
                ui.separator();
                if ui
                    .add_enabled(self.query.page() > 1, egui::Button::new("<"))
                    .clicked()
                {
                    self.query.page = self.query.page() - 1;
                    reload = Some(Policy::PreferCache);
                }
                ui.label(format!(
                    "page {} / {}",
                    self.query.page(),
                    p.total_pages.max(1)
                ));
                if ui
                    .add_enabled(self.query.page() < p.total_pages, egui::Button::new(">"))
                    .clicked()
                {
                    self.query.page = self.query.page() + 1;
                    reload = Some(Policy::PreferCache);
                }
                if let Some(s) = self.page_source {
                    ui.label(
                        RichText::new(match s {
                            Source::Network => "live",
                            Source::Cache => "cached",
                            Source::StaleCache => "stale cache",
                            Source::Seed => "survey cache",
                        })
                        .weak(),
                    );
                }
            }
        });
        if let Some(p) = reload {
            self.load_page(p);
        }
        ui.separator();
        egui::Panel::right("kfx-ws-details")
            .resizable(true)
            .default_size(380.0)
            .min_size(260.0)
            .show(ui, |ui| self.details_pane(ui));
        egui::CentralPanel::default().show(ui, |ui| self.grid(ui));
    }

    fn grid(&mut self, ui: &mut egui::Ui) {
        let Some(page) = self.page.clone() else {
            ui.label(if self.loading_page {
                "Loading…"
            } else {
                "Nothing loaded yet."
            });
            return;
        };
        if page.items.is_empty() {
            ui.label("No items match.");
            return;
        }
        let card_w = 168.0;
        let cols = ((ui.available_width() / (card_w + 8.0)).floor() as usize).max(1);
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("ws-grid")
                    .spacing(vec2(8.0, 8.0))
                    .show(ui, |ui| {
                        for (i, it) in page.items.iter().enumerate() {
                            let selected = self.selected.as_ref().is_some_and(|s| s.id == it.id);
                            let (rect, resp) =
                                ui.allocate_exact_size(vec2(card_w, 196.0), egui::Sense::click());
                            let vis = ui.visuals();
                            let fill = if selected {
                                vis.selection.bg_fill
                            } else if resp.hovered() {
                                vis.widgets.hovered.bg_fill
                            } else {
                                vis.faint_bg_color
                            };
                            ui.painter().rect_filled(rect, 4.0, fill);
                            ui.painter().rect_stroke(
                                rect,
                                4.0,
                                egui::Stroke::new(1.0, vis.widgets.noninteractive.bg_stroke.color),
                                egui::StrokeKind::Inside,
                            );
                            let img_rect = egui::Rect::from_min_size(
                                rect.min + vec2(4.0, 4.0),
                                vec2(card_w - 8.0, 120.0),
                            );
                            ui.painter()
                                .rect_filled(img_rect, 2.0, Color32::from_gray(40));
                            if let Some(t) = &it.thumbnail {
                                self.want_image(&t.clone(), 320);
                                if let Some(Thumb::Ready(tex)) = self.thumbs.get(t) {
                                    let sz = tex.size_vec2();
                                    let scale =
                                        (img_rect.width() / sz.x).min(img_rect.height() / sz.y);
                                    let r =
                                        egui::Rect::from_center_size(img_rect.center(), sz * scale);
                                    ui.painter().image(
                                        tex.id(),
                                        r,
                                        egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        Color32::WHITE,
                                    );
                                }
                            }
                            let mut text_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(egui::Rect::from_min_max(
                                        img_rect.left_bottom() + vec2(2.0, 4.0),
                                        rect.max - vec2(4.0, 2.0),
                                    ))
                                    .layout(egui::Layout::top_down(egui::Align::Min)),
                            );
                            // inside the scroll area's clip too (a card scrolled half out must not
                            // draw its text outside)
                            text_ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                            // the texts must not take the card's click (selectable labels would)
                            text_ui.style_mut().interaction.selectable_labels = false;
                            text_ui.add(
                                egui::Label::new(RichText::new(safe_text(&it.title)).strong())
                                    .truncate(),
                            );
                            text_ui.label(
                                RichText::new(it.category.map_or("", |c| c.label())).small(),
                            );
                            text_ui.horizontal(|ui| {
                                if let Some(r) = it.rating {
                                    ui.label(
                                        RichText::new(format!("{r:.1} / 5"))
                                            .color(Color32::from_rgb(230, 190, 60)),
                                    );
                                }
                                if let Some(d) = it.difficulty {
                                    ui.label(RichText::new(format!("diff {d:.1}")).small().weak());
                                }
                            });
                            text_ui.add(
                                egui::Label::new(
                                    RichText::new(safe_text(&it.submitter)).small().weak(),
                                )
                                .truncate(),
                            );
                            if resp.clicked() {
                                clicked = Some(it.clone());
                            }
                            if (i + 1) % cols == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
        if let Some(it) = clicked {
            self.select(it);
        }
    }

    fn details_pane(&mut self, ui: &mut egui::Ui) {
        let Some(sel) = self.selected.clone() else {
            ui.weak("Select an item to see its details.");
            return;
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.heading(safe_text(&sel.title));
                let Some(d) = self.detail.clone() else {
                    if self.loading_detail {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading details…");
                        });
                    }
                    return;
                };
                ui.label(safe_text(&format!(
                    "by {}",
                    if d.submitter.is_empty() {
                        &sel.submitter
                    } else {
                        &d.submitter
                    }
                )));
                egui::Grid::new("ws-info")
                    .num_columns(2)
                    .spacing(vec2(12.0, 2.0))
                    .show(ui, |ui| {
                        let row = |ui: &mut egui::Ui, k: &str, v: String| {
                            if !v.is_empty() {
                                ui.label(RichText::new(k).weak());
                                ui.label(safe_text(&v));
                                ui.end_row();
                            }
                        };
                        row(
                            ui,
                            "Category",
                            d.category.map_or(String::new(), |c| c.label().to_string()),
                        );
                        row(
                            ui,
                            "Rating",
                            d.rating.map_or(String::new(), |r| {
                                format!("{r:.2} / 5 ({} ratings)", d.rating_count.unwrap_or(0))
                            }),
                        );
                        row(
                            ui,
                            "Difficulty",
                            d.difficulty.map_or(String::new(), |r| {
                                format!("{r:.2} / 5 ({} ratings)", d.difficulty_count.unwrap_or(0))
                            }),
                        );
                        row(
                            ui,
                            "Compatibility",
                            d.min_game_version
                                .clone()
                                .map_or(String::new(), |v| format!("needs {v}")),
                        );
                        row(
                            ui,
                            "Original author",
                            d.original_author.clone().unwrap_or_default(),
                        );
                        row(ui, "Created", d.original_date.clone().unwrap_or_default());
                        row(ui, "Submitted", d.submitted.clone().unwrap_or_default());
                        row(ui, "Updated", d.updated.clone().unwrap_or_default());
                        row(ui, "Downloads", d.downloads.to_string());
                        for (k, v) in &d.extra {
                            row(ui, k, v.clone());
                        }
                    });
                if !d.images.is_empty() {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for im in d.images.iter().take(6).cloned().collect::<Vec<_>>() {
                            self.want_image(&im, 480);
                            match self.thumbs.get(&im) {
                                Some(Thumb::Ready(tex)) => {
                                    let sz = tex.size_vec2();
                                    let s = (170.0 / sz.x).min(120.0 / sz.y).min(1.0);
                                    ui.image((tex.id(), sz * s));
                                }
                                Some(Thumb::Loading) | None => {
                                    ui.spinner();
                                }
                                Some(Thumb::Failed) => {}
                            }
                        }
                    });
                }
                ui.add_space(6.0);
                ui.separator();
                ui.label(RichText::new("Files").strong());
                let mut dl = None;
                let busy = self.download.is_some() || self.planning;
                let latest = d.latest_file().map(|f| f.id);
                egui::Grid::new("ws-files")
                    .num_columns(5)
                    .spacing(vec2(10.0, 3.0))
                    .show(ui, |ui| {
                        for f in &d.files {
                            let name = RichText::new(safe_text(&f.name));
                            ui.label(if f.broken {
                                name.strikethrough().weak()
                            } else {
                                name
                            });
                            ui.label(safe_text(&f.size_label));
                            ui.label(safe_text(&f.date));
                            ui.label(if f.broken {
                                "broken".to_string()
                            } else if Some(f.id) == latest {
                                "latest".to_string()
                            } else {
                                String::new()
                            });
                            if ui
                                .add_enabled(
                                    !busy && !f.broken,
                                    egui::Button::new("Download & install"),
                                )
                                .clicked()
                            {
                                dl = Some(f.clone());
                            }
                            ui.end_row();
                        }
                    });
                if d.files.is_empty() {
                    ui.weak("The page lists no downloadable file.");
                }
                if let Some((item, p, _)) = &self.download {
                    let frac = if p.total > 0 {
                        p.done as f32 / p.total as f32
                    } else {
                        0.0
                    };
                    ui.add(egui::ProgressBar::new(frac).text(safe_text(&format!(
                        "{} {} / {}{}",
                        item.file_name,
                        crate::util::format_size(p.done),
                        if p.total > 0 {
                            crate::util::format_size(p.total)
                        } else {
                            "?".into()
                        },
                        if p.resumed_from > 0 { " (resumed)" } else { "" }
                    ))));
                }
                if let Some(f) = dl {
                    self.start_download(&f);
                }
                if !d.description.is_empty() {
                    ui.separator();
                    ui.label(RichText::new("Description").strong());
                    ui.label(safe_text(&d.description));
                }
                if !d.install_instructions.is_empty() {
                    ui.separator();
                    ui.label(RichText::new("Installation instructions (author)").strong());
                    ui.label(safe_text(&d.install_instructions));
                }
            });
    }

    // ---- install plan window ----

    fn plan_window(&mut self, ui: &mut egui::Ui, host: &mut dyn Host) {
        let mut replan = false;
        let mut apply = false;
        let mut close = false;
        let install_root = host.install_root();
        if let Some(pu) = &mut self.plan_ui {
            let mut open = true;
            egui::Window::new("Install plan")
                .open(&mut open)
                .collapsible(false)
                .default_width(560.0)
                .show(ui.ctx(), |ui| {
                    ui.label(
                        RichText::new(safe_text(&format!(
                            "{}: {}",
                            pu.plan.kind.label(),
                            pu.plan.name
                        )))
                        .strong(),
                    );
                    if let Some(w) = &pu.insp.class.wrapper {
                        ui.weak(safe_text(&format!("archive folder '{w}' is stripped")));
                    }
                    ui.horizontal(|ui| {
                        ui.label("If something already exists:");
                        for (c, l) in [
                            (Conflict::Renumber, "keep it, renumber / rename mine"),
                            (Conflict::Skip, "skip"),
                            (Conflict::Overwrite, "overwrite (backed up)"),
                        ] {
                            if ui.radio_value(&mut pu.opts.conflict, c, l).changed() {
                                replan = true;
                            }
                        }
                    });
                    if matches!(
                        pu.plan.kind,
                        classify::Kind::Level | classify::Kind::MapPack
                    ) && pu.insp.class.layout == classify::Layout::Loose
                    {
                        ui.horizontal(|ui| {
                            ui.label("Put the level in:");
                            let label = match pu.pack_choice {
                                0 => "default".to_string(),
                                n if n <= pu.packs.len() => {
                                    safe_text(&format!("levels/{}", pu.packs[n - 1]))
                                }
                                _ => safe_text(&format!("new pack '{}'", pu.new_pack_name)),
                            };
                            egui::ComboBox::from_id_salt("ws-pack")
                                .selected_text(label)
                                .show_ui(ui, |ui| {
                                    let mut ch = ui
                                        .selectable_value(&mut pu.pack_choice, 0, "Default")
                                        .changed();
                                    for (i, p) in pu.packs.clone().iter().enumerate() {
                                        ch |= ui
                                            .selectable_value(
                                                &mut pu.pack_choice,
                                                i + 1,
                                                safe_text(&format!("levels/{p}")),
                                            )
                                            .changed();
                                    }
                                    let n = pu.packs.len() + 1;
                                    ch |= ui
                                        .selectable_value(&mut pu.pack_choice, n, "New pack")
                                        .changed();
                                    if ch {
                                        replan = true;
                                    }
                                });
                            if pu.pack_choice > pu.packs.len()
                                && ui.text_edit_singleline(&mut pu.new_pack_name).lost_focus()
                            {
                                replan = true;
                            }
                        });
                        pu.opts.pack_target = match pu.pack_choice {
                            0 => None,
                            n if n <= pu.packs.len() => {
                                Some(PackTarget::Existing(pu.packs[n - 1].clone()))
                            }
                            _ => Some(PackTarget::New(pu.new_pack_name.clone())),
                        };
                    }
                    if !pu.plan.load_order.is_empty()
                        && ui
                            .checkbox(&mut pu.opts.enable_mods, "Enable in mods/load_order.cfg")
                            .changed()
                    {
                        replan = true;
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            for l in pu.plan.describe() {
                                let c = if l.contains("BLOCKED") {
                                    Some(Color32::from_rgb(230, 90, 80))
                                } else if l.contains("OVERWRITTEN") || l.contains("warning:") {
                                    Some(Color32::from_rgb(230, 190, 70))
                                } else {
                                    None
                                };
                                let l = safe_text(&l);
                                match c {
                                    Some(c) => ui.colored_label(c, l),
                                    None => ui.label(l),
                                };
                            }
                            egui::CollapsingHeader::new(format!(
                                "All {} files",
                                pu.plan.items.len()
                            ))
                            .show(ui, |ui| {
                                for i in &pu.plan.items {
                                    let tag = match i.action {
                                        Action::Create => "new",
                                        Action::Overwrite => "OVERWRITE",
                                        Action::Skip => "skip",
                                    };
                                    ui.label(
                                        RichText::new(safe_text(&format!("{tag:9} {}", i.dest)))
                                            .monospace()
                                            .small(),
                                    );
                                }
                            });
                        });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !pu.plan.is_blocked() && install_root.is_some(),
                                egui::Button::new("Install"),
                            )
                            .clicked()
                        {
                            apply = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            if !open {
                close = true;
            }
        }
        if replan {
            if let (Some(pu), Some(root)) = (&self.plan_ui, install_root.clone()) {
                let (a, o, i) = (pu.archive.clone(), pu.opts.clone(), pu.insp.clone());
                self.start_plan(a, root, o, Some(i));
            }
        }
        if apply {
            if let (Some(plan), Some(root)) =
                (self.plan_ui.as_ref().map(|p| p.plan.clone()), install_root)
            {
                self.busy_install = true;
                let c = self.new_cancel();
                self.task.send(
                    Job::Apply {
                        install: root,
                        plan,
                    },
                    &c,
                );
            }
        }
        if close {
            self.plan_ui = None;
        }
        // result of the last install: open buttons
        if let Some(m) = self.last_install.clone() {
            let mut keep = true;
            egui::Window::new("Installed")
                .open(&mut keep)
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.label(safe_text(&format!(
                        "{} installed: {} files.",
                        m.name,
                        m.files.len()
                    )));
                    if let Some(root) = host.install_root() {
                        for t in install::open_targets(&root, &m) {
                            match &t {
                                OpenTarget::Level { number, name, .. } => {
                                    if ui
                                        .button(format!(
                                            "Open level {number} ({name}) in the editor"
                                        ))
                                        .clicked()
                                    {
                                        host.open_in_editor(&t);
                                    }
                                }
                                OpenTarget::Campaign { cfg } => {
                                    if ui
                                        .button(format!(
                                            "Open campaign {} in the editor",
                                            cfg.file_name()
                                                .map(|n| n.to_string_lossy().into_owned())
                                                .unwrap_or_default()
                                        ))
                                        .clicked()
                                    {
                                        host.open_in_editor(&t);
                                    }
                                }
                            }
                        }
                    }
                    if m.load_order.is_some() {
                        ui.weak("mods/load_order.cfg was updated.");
                    }
                });
            if !keep {
                self.last_install = None;
            }
        }
    }

    // ---- installed ----

    fn installed_tab(&mut self, ui: &mut egui::Ui, host: &mut dyn Host) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.checking_updates && !self.installed.is_empty(),
                    egui::Button::new("Check for updates"),
                )
                .clicked()
            {
                let items: Vec<(String, u32, String)> = self
                    .installed
                    .iter()
                    .filter_map(|m| {
                        m.item
                            .as_ref()
                            .map(|i| (m.id.clone(), i.id, i.slug.clone()))
                    })
                    .collect();
                self.checking_updates = true;
                let c = self.new_cancel();
                self.net.send(Job::Updates { items }, &c);
            }
            if ui.button("Reload list").clicked() {
                self.installed = self.store.list();
            }
        });
        ui.separator();
        if self.installed.is_empty() {
            ui.weak("Nothing installed through the Workshop yet.");
            return;
        }
        let mut uninstall = None;
        let mut update = None;
        let mut open = None;
        let root = host.install_root();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for m in self.installed.clone() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(safe_text(&m.name)).strong());
                            ui.label(RichText::new(safe_text(&m.kind)).weak());
                            if let Some(i) = &m.item {
                                ui.label(
                                    RichText::new(safe_text(&format!(
                                        "item #{} file {} ({}, {})",
                                        i.id, i.file_name, i.file_size_label, i.file_date
                                    )))
                                    .small()
                                    .weak(),
                                );
                            }
                        });
                        ui.label(
                            RichText::new(format!(
                                "{} files, {} levels",
                                m.files.len(),
                                m.levels.len()
                            ))
                            .small(),
                        );
                        match self.updates.get(&m.id) {
                            Some(Ok(UpdateStatus::Available { file, reason })) => {
                                ui.colored_label(
                                    Color32::from_rgb(230, 190, 70),
                                    safe_text(&format!("Update available: {reason}")),
                                );
                                if ui.button("Update").clicked() {
                                    update = Some((m.clone(), file.clone()));
                                }
                            }
                            Some(Ok(UpdateStatus::UpToDate)) => {
                                ui.colored_label(Color32::from_rgb(110, 200, 120), "Up to date");
                            }
                            Some(Ok(UpdateStatus::Unknown(w))) => {
                                ui.weak(safe_text(&format!("Update status unknown: {w}")));
                            }
                            Some(Err(e)) => {
                                ui.colored_label(
                                    Color32::from_rgb(220, 90, 80),
                                    safe_text(&format!("Update check failed: {e}")),
                                );
                            }
                            None => {}
                        }
                        ui.horizontal(|ui| {
                            if let Some(root) = &root {
                                for t in install::open_targets(root, &m) {
                                    if let OpenTarget::Level { number, .. } = &t {
                                        if ui.small_button(format!("Open level {number}")).clicked()
                                        {
                                            open = Some(t.clone());
                                        }
                                    } else if ui.small_button("Open campaign").clicked() {
                                        open = Some(t.clone());
                                    }
                                }
                            }
                            if self.confirm_uninstall.as_deref() == Some(m.id.as_str()) {
                                ui.colored_label(
                                    Color32::from_rgb(230, 190, 70),
                                    "Remove these files and restore what they replaced?",
                                );
                                if ui.button("Yes, uninstall").clicked() {
                                    uninstall = Some(m.id.clone());
                                }
                                if ui.button("No").clicked() {
                                    self.confirm_uninstall = None;
                                }
                            } else if ui.small_button("Uninstall").clicked() {
                                self.confirm_uninstall = Some(m.id.clone());
                            }
                        });
                    });
                }
            });
        if let Some(t) = open {
            host.open_in_editor(&t);
        }
        if let (Some(id), Some(root)) = (uninstall, root.clone()) {
            self.confirm_uninstall = None;
            self.busy_install = true;
            let c = self.new_cancel();
            self.task.send(
                Job::Uninstall {
                    install: root,
                    id,
                    force: false,
                },
                &c,
            );
        }
        if let (Some((m, file)), Some(root)) = (update, root) {
            // update = uninstall the old version, then download and install the new one
            if let Some(i) = &m.item {
                match install::uninstall(&root, &m.id, &self.store, false) {
                    Ok(r) if r.kept_modified.is_empty() => {
                        self.installed = self.store.list();
                        self.selected = Some(ItemSummary {
                            id: i.id,
                            slug: i.slug.clone(),
                            title: i.title.clone(),
                            ..Default::default()
                        });
                        self.tab = Tab::Browse;
                        self.detail = Some(ItemDetail {
                            id: i.id,
                            slug: i.slug.clone(),
                            title: i.title.clone(),
                            files: vec![file.clone()],
                            ..Default::default()
                        });
                        self.start_download(&file);
                    }
                    Ok(r) => {
                        self.error = Some(format!(
                            "update stopped: {} file(s) were modified since the install \
                                ({}); uninstall manually",
                            r.kept_modified.len(),
                            r.kept_modified.join(", ")
                        ))
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }
    }

    // ---- publish ----

    fn publish_tab(&mut self, ui: &mut egui::Ui, host: &mut dyn Host) {
        let subjects = host.publish_subjects();
        ui.label(
            "Package a level or campaign into a zip for the Workshop. \
                Nothing is uploaded: uploading needs your own login in a browser.",
        );
        ui.separator();
        if subjects.is_empty() {
            ui.weak("No open level or campaign to package.");
            return;
        }
        let p = &mut self.publish;
        p.subject_idx = p.subject_idx.min(subjects.len() - 1);
        let mut gather = false;
        egui::Grid::new("ws-pub")
            .num_columns(2)
            .spacing(vec2(10.0, 4.0))
            .show(ui, |ui| {
                ui.label("Package");
                egui::ComboBox::from_id_salt("ws-pub-subject")
                    .selected_text(safe_text(&subjects[p.subject_idx].0))
                    .show_ui(ui, |ui| {
                        for (i, (l, _)) in subjects.iter().enumerate() {
                            ui.selectable_value(&mut p.subject_idx, i, safe_text(l));
                        }
                    });
                ui.end_row();
                ui.label("Title");
                ui.text_edit_singleline(&mut p.title);
                ui.end_row();
                ui.label("Author");
                ui.text_edit_singleline(&mut p.author);
                ui.end_row();
                ui.label("Description");
                ui.add(
                    egui::TextEdit::multiline(&mut p.description)
                        .desired_rows(3)
                        .desired_width(420.0),
                );
                ui.end_row();
                ui.label("Layout (levels)");
                egui::ComboBox::from_id_salt("ws-pub-layout")
                    .selected_text(
                        ["Automatic", "Flat files at the root", "levels/standard/…"][p.layout_idx],
                    )
                    .show_ui(ui, |ui| {
                        for (i, l) in ["Automatic", "Flat files at the root", "levels/standard/…"]
                            .iter()
                            .enumerate()
                        {
                            ui.selectable_value(&mut p.layout_idx, i, *l);
                        }
                    });
                ui.end_row();
                ui.label("Editor data");
                ui.checkbox(
                    &mut p.include_sidecar,
                    "Include mapNNNNN.kfxed.toml (custom slabs, notes, locks)",
                );
                ui.end_row();
            });
        if ui.button("Gather files and run pre-flight").clicked() {
            gather = true;
        }
        if gather {
            let subject = subjects[self.publish.subject_idx].1.clone();
            let mut opts = PackageOptions::new(&self.publish.title);
            opts.author = self.publish.author.clone();
            opts.description = self.publish.description.clone();
            opts.include_sidecar = self.publish.include_sidecar;
            opts.layout = match self.publish.layout_idx {
                1 => package::PackLayout::Flat,
                2 => package::PackLayout::Rooted("levels/standard".into()),
                _ => package::PackLayout::Auto,
            };
            opts.host_issues = host.validator_issues(&subject);
            let c = self.new_cancel();
            self.task.send(
                Job::Gather {
                    subject,
                    opts: Box::new(opts),
                },
                &c,
            );
        }
        let mut write = None::<bool>;
        let mut copy = false;
        let p = &mut self.publish;
        if let Some(plan) = &mut p.plan {
            ui.separator();
            let pf = &plan.preflight;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} error(s)", pf.count(Severity::Error))).color(
                        if pf.has_errors() {
                            Color32::from_rgb(230, 90, 80)
                        } else {
                            Color32::GRAY
                        },
                    ),
                );
                ui.label(
                    RichText::new(format!("{} warning(s)", pf.count(Severity::Warning)))
                        .color(Color32::from_rgb(230, 190, 70)),
                );
                ui.label(format!(
                    "{} in {} files",
                    crate::util::format_size(pf.total_size),
                    plan.included().count()
                ));
                for (r, s) in pf.by_role.iter().take(5) {
                    ui.label(
                        RichText::new(format!("{r} {}", crate::util::format_size(*s)))
                            .small()
                            .weak(),
                    );
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("ws-pub-issues")
                .max_height(120.0)
                .show(ui, |ui| {
                    for i in &pf.issues {
                        let (c, t) = match i.severity {
                            Severity::Error => (Color32::from_rgb(230, 90, 80), "error"),
                            Severity::Warning => (Color32::from_rgb(230, 190, 70), "warning"),
                            Severity::Info => (Color32::GRAY, "info"),
                        };
                        ui.colored_label(c, safe_text(&format!("{t}: {}", i.message)));
                    }
                });
            let mut changed = false;
            egui::ScrollArea::vertical()
                .id_salt("ws-pub-files")
                .max_height(200.0)
                .show(ui, |ui| {
                    for e in plan.entries.iter_mut().filter(|e| !e.path.is_empty()) {
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut e.included, "").changed() {
                                changed = true;
                            }
                            ui.label(RichText::new(safe_text(&e.path)).monospace().small());
                            ui.label(
                                RichText::new(safe_text(&format!(
                                    "{} {}",
                                    crate::util::format_size(e.size),
                                    e.note.clone().unwrap_or_default()
                                )))
                                .small()
                                .weak(),
                            );
                        });
                    }
                });
            if changed {
                plan.opts.excluded = plan
                    .entries
                    .iter()
                    .filter(|e| !e.included)
                    .map(|e| e.path.clone())
                    .collect();
                plan.refresh();
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Zip:");
                ui.add(egui::TextEdit::singleline(&mut p.zip_path).desired_width(420.0));
                ui.checkbox(&mut p.force, "Write even with errors");
                if ui.button("Write zip").clicked() {
                    write = Some(p.force);
                }
            });
            if let Some(r) = &p.result {
                ui.colored_label(
                    Color32::from_rgb(110, 200, 120),
                    safe_text(&format!(
                        "Wrote {} ({} files, {}, sha256 {}); the installer reads it back as: {}",
                        r.zip.display(),
                        r.files,
                        crate::util::format_size(r.size),
                        &r.sha256[..12],
                        r.kind.label()
                    )),
                );
                if ui
                    .button("Copy zip path and open the Workshop upload page")
                    .clicked()
                {
                    copy = true;
                }
                if ui.button("Copy upload form text").clicked() {
                    host.copy_to_clipboard(&package::upload_form_text(plan));
                }
            }
        }
        if let Some(force) = write {
            if let Some(plan) = self.publish.plan.clone() {
                self.busy_install = true;
                let c = self.new_cancel();
                self.task.send(
                    Job::Package {
                        plan,
                        dest: PathBuf::from(self.publish.zip_path.clone()),
                        force,
                    },
                    &c,
                );
            }
        }
        if copy {
            if let Some(r) = &self.publish.result {
                host.copy_to_clipboard(&r.zip.display().to_string());
                host.open_url(package::upload_page_url());
                self.status = "zip path copied; upload in your browser with your own login".into();
            }
        }
    }
}

impl Drop for WorkshopState {
    fn drop(&mut self) {
        for c in &self.cancels {
            c.store(true, Ordering::SeqCst);
        }
    }
}
