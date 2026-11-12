//! Offscreen screenshots of each tab, from cached data only: the HTTP cache named by
//! `KFX_WORKSHOP_CACHE` (filled by `live_prime`) and the Workshop survey named by
//! `KFX_WORKSHOP_SURVEY` (item pages and files, see `survey.rs`). The PNGs are written to the
//! test scratch folder (`workshop/screens`). Skipped when either is unset or absent.
mod common;
use common::*;
use egui::vec2;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use kfx_workshop::install::*;
use kfx_workshop::model::*;
use kfx_workshop::package::*;
use kfx_workshop::ui::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct TestHost {
    root: PathBuf,
    subjects: Vec<(String, Subject)>,
    opened: Vec<String>,
    clipboard: Vec<String>,
    urls: Vec<String>,
}

impl Host for TestHost {
    fn install_root(&self) -> Option<PathBuf> {
        Some(self.root.clone())
    }
    fn open_in_editor(&mut self, t: &OpenTarget) {
        self.opened.push(format!("{t:?}"));
    }
    fn copy_to_clipboard(&mut self, text: &str) {
        self.clipboard.push(text.to_string());
    }
    fn open_url(&mut self, url: &str) {
        self.urls.push(url.to_string());
    }
    fn publish_subjects(&self) -> Vec<(String, Subject)> {
        self.subjects.clone()
    }
    fn validator_issues(&self, _: &Subject) -> Vec<Issue> {
        vec![Issue::new(
            Severity::Warning,
            "validator: slab 12,40 has no wall next to the lava",
        )]
    }
}

struct S {
    ws: WorkshopState,
    host: TestHost,
}

fn wait(h: &mut Harness<'static, S>, what: &str, cond: impl Fn(&S) -> bool) {
    let t = Instant::now();
    loop {
        h.run_steps(2);
        if cond(h.state()) {
            h.run_steps(3);
            return;
        }
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn shot(h: &mut Harness<'static, S>, dir: &Path, name: &str) -> PathBuf {
    h.run_steps(3);
    let img = h.render().expect("render");
    let p = dir.join(format!("{name}.png"));
    img.save(&p).unwrap();
    println!(
        "screenshot {} ({}x{})",
        p.display(),
        img.width(),
        img.height()
    );
    p
}

#[test]
fn screenshots_of_every_tab() {
    let (Some(cache), Some(survey)) = (
        std::env::var_os("KFX_WORKSHOP_CACHE").map(PathBuf::from),
        std::env::var_os("KFX_WORKSHOP_SURVEY").map(PathBuf::from),
    ) else {
        eprintln!("KFX_WORKSHOP_CACHE / KFX_WORKSHOP_SURVEY unset; skipped");
        return;
    };
    if !cache.join("http").is_dir() || !survey.join("pages").is_dir() {
        eprintln!("caches absent; skipped");
        return;
    }
    // the screenshots need egui_kittest's wgpu renderer (CI runners may have no adapter)
    if std::panic::catch_unwind(egui_kittest::wgpu::WgpuTestRenderer::new).is_err() {
        eprintln!("no wgpu adapter; skipped");
        return;
    }
    let shots = kfx_formats::testutil::scratch_dir("workshop/screens");
    let s = Scratch::new("screens");
    // a private install with a few levels to package
    let root = s.join("install");
    private_install(&root);
    let work = s.join("work");
    std::fs::create_dir_all(&work).unwrap();
    for (n, b) in level_files(1500, true) {
        std::fs::write(work.join(n), b).unwrap();
    }
    std::fs::write(work.join("map01500.kfxed.toml"), b"[notes]\n").unwrap();
    std::fs::write(work.join("map01500.rules.cfg"), b"[game]\n").unwrap();
    let host = TestHost {
        root: root.clone(),
        subjects: vec![(
            "Level 1500 (work folder)".into(),
            Subject::Level {
                folder: work.clone(),
                number: 1500,
            },
        )],
        opened: vec![],
        clipboard: vec![],
        urls: vec![],
    };
    let cfg = WorkshopConfig {
        cache_dir: cache.clone(),
        store_dir: s.join("store"),
        seed_dirs: vec![survey.clone()],
        offline: true,
    };
    let mut ws = WorkshopState::new(cfg);
    ws.set_query(BrowseQuery {
        category: Some(Category::Map),
        ..Default::default()
    });
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1280.0, 800.0))
        .with_accessibility_check(false)
        .build_ui_state(|ui, s: &mut S| s.ws.show(ui, &mut s.host), S { ws, host });

    // 1. browse grid with thumbnails
    wait(&mut h, "the browse page", |s| s.ws.browse_page().is_some());
    std::thread::sleep(Duration::from_millis(800)); // thumbnails decode on the worker
    h.run_steps(10);
    let items = h.state().ws.browse_page().unwrap().items.len();
    assert_eq!(items, 50, "the cached first page of Map / highest rated");
    shot(&mut h, &shots, "1-browse-grid");

    // 2. details of a creature with screenshots (page from the survey cache, images from the cache)
    h.state_mut()
        .ws
        .open_item(647, "maiden-of-the-nest", "Maiden of the Nest");
    wait(&mut h, "item 647", |s| s.ws.detail().is_some());
    std::thread::sleep(Duration::from_millis(500));
    h.run_steps(10);
    let d = h.state().ws.detail().unwrap().clone();
    assert_eq!(d.title, "Maiden of the Nest");
    assert!(!d.files.is_empty() && !d.images.is_empty());
    shot(&mut h, &shots, "2-details");

    // 3. download (from the survey seed) and the install plan window
    h.state_mut()
        .ws
        .open_item(164, "my-pet-dungeon", "My Pet Dungeon");
    wait(&mut h, "item 164", |s| {
        s.ws.detail().is_some_and(|d| d.id == 164)
    });
    let f = h
        .state()
        .ws
        .detail()
        .unwrap()
        .latest_file()
        .unwrap()
        .clone();
    h.state_mut().ws.start_download(&f);
    wait(&mut h, "the install plan", |s| {
        s.ws.pending_plan().is_some()
    });
    {
        let p = h.state().ws.pending_plan().unwrap();
        println!(
            "plan for item 164: {:?}\n{}",
            p.kind,
            p.describe().join("\n")
        );
    }
    shot(&mut h, &shots, "3-install-plan");

    // 4. confirm: the installed window with the open button
    h.get_by_label("Install").click();
    wait(&mut h, "the install", |s| s.ws.last_install.is_some());
    shot(&mut h, &shots, "4-installed");
    assert!(!h.state().ws.installed().is_empty());
    let id = h.state().ws.installed()[0].id.clone();

    // 5. Installed tab with an update offer
    h.state_mut().ws.last_install = None;
    h.state_mut().ws.tab = Tab::Installed;
    let newer = FileEntry {
        id: 99999,
        name: "my-pet-dungeon-v2.zip".into(),
        date: "2026-11-01".into(),
        size_label: "80.00KiB".into(),
        ..Default::default()
    };
    h.state_mut().ws.set_update_status(
        &id,
        Ok(UpdateStatus::Available {
            file: newer,
            reason:
                "newer file my-pet-dungeon-v2.zip (2026-11-01, 80.00KiB) than the installed one"
                    .into(),
        }),
    );
    h.run_steps(3);
    shot(&mut h, &shots, "5-installed-tab");

    // 6. Publish tab: gather and pre-flight
    h.state_mut().ws.tab = Tab::Publish;
    h.run_steps(3);
    h.get_by_label("Gather files and run pre-flight").click();
    wait(&mut h, "the package plan", |_| true);
    std::thread::sleep(Duration::from_millis(300));
    h.run_steps(8);
    shot(&mut h, &shots, "6-publish");

    // 7. uninstall through the UI restores the install
    h.state_mut().ws.tab = Tab::Installed;
    h.run_steps(3);
    h.get_by_label("Uninstall").click();
    h.run_steps(3);
    h.get_by_label("Yes, uninstall").click();
    wait(&mut h, "the uninstall", |s| s.ws.installed().is_empty());
    shot(&mut h, &shots, "7-after-uninstall");
}
