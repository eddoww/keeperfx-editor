//! Campaign > Workshop… through the real editor UI, with a mock transport (no request ever
//! reaches keeperfx.net): the first-use notice holds every request back until Continue, the
//! browse page and the item page come from the Workshop crate's test fixtures, "Download &
//! install" installs a level into a private install, the Installed tab lists it and Uninstall
//! removes it again. The private install lives in a scratch folder of the test build (the
//! shipped install is only linked for reading; its level folders are never written).
use crate::settings::Settings;
use crate::ui_tests::Ui;
use eframe::egui;
use egui_kittest::kittest::Queryable;
use kfx_workshop::client::{Client, ClientConfig, MockTransport};
use kfx_workshop::ui::WorkshopConfig;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kfx-workshop/tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// A scratch folder of the test build (`testutil::scratch_dir`), removed when dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Scratch {
        let p = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/campaign/{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An install the editor can load: every top-level entry of the real install is a symlink
/// (read only), except the pack folders, which are real, empty folders with a copy of
/// `levels/personal.cfg` (so installs land in this folder only).
#[cfg(unix)]
fn private_install(real: &Path, root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    for e in std::fs::read_dir(real).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if ["campgns", "levels", "multiplayer", "mods"]
            .iter()
            .any(|g| g.eq_ignore_ascii_case(&n))
        {
            continue;
        }
        std::os::unix::fs::symlink(e.path(), root.join(&n)).unwrap();
    }
    for d in ["campgns", "levels/personal", "multiplayer", "mods"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let cfg = kfx_formats::testutil::resolve_ci(real, "levels/personal.cfg")
        .map(|p| std::fs::read(p).unwrap());
    let cfg = cfg.unwrap_or_else(|| {
        b"[common]\nNAME = Personal levels\nLEVELS_LOCATION = levels/personal\n".to_vec()
    });
    std::fs::write(root.join("levels/personal.cfg"), cfg).unwrap();
}

/// A Workshop download: one free play level, loose files.
fn level_zip() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for ext in [
            "slb", "own", "clm", "dat", "tng", "apt", "lgt", "wib", "inf", "txt",
        ] {
            z.start_file(format!("map01500.{ext}"), o).unwrap();
            z.write_all(format!("{ext} of level 1500").as_bytes())
                .unwrap();
        }
        z.start_file("map01500.lif", o).unwrap();
        z.write_all(b"1500, Mock level\n").unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn wait(u: &mut Ui, what: &str, cond: impl Fn(&mut Ui) -> bool) {
    let t = Instant::now();
    loop {
        u.steps(2);
        if cond(u) {
            u.steps(3);
            return;
        }
        if t.elapsed() > Duration::from_secs(30) {
            u.shot(&format!("timeout-{what}"));
            panic!("timed out waiting for {what}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(unix)]
#[test]
fn ui_workshop_install_and_uninstall() {
    let Some(real) = kfx_formats::testutil::install_dir() else {
        return;
    };
    let s = Scratch::new("ws");
    let root = s.0.join("install");
    private_install(&real, &root);
    let mock = Arc::new(MockTransport::default());
    let browse = fixture("browse-creature-p1.html");
    mock.add("/workshop/browse?order_by=highest-rated", browse);
    // the card "Maiden of the Nest" answers with the item page of the fixture (a map pack page)
    mock.add(
        "/workshop/item/647/maiden-of-the-nest",
        fixture("item-10-mappack.html"),
    );
    for f in ["2038", "2036", "2032"] {
        mock.add(&format!("/workshop/download/10/{f}/brorc.zip"), level_zip());
    }
    mock.add("/workshop/download/10/325/Brorca.zip", level_zip());
    let mut cc = ClientConfig::new(s.0.join("cache"));
    cc.min_interval = Duration::from_millis(5);
    let client = Arc::new(Client::with_transport(cc, mock.clone()));
    let cfg = WorkshopConfig {
        cache_dir: s.0.join("cache"),
        store_dir: s.0.join("store"),
        seed_dirs: vec![],
        offline: false,
    };

    let mut u = Ui::with_settings(
        "workshop",
        Settings {
            install: Some(root.clone()),
            ..Default::default()
        },
        egui::vec2(1500.0, 950.0),
    );
    // `KFX_INSTALL` (which the test install comes from) overrides the saved setting at start-up:
    // switch to the private install the way the first-run dialog does
    u.app().use_install(&root);
    u.steps(2);
    assert_eq!(
        u.app().install(),
        Some(root.as_path()),
        "the private install loads"
    );
    u.app().set_workshop_client(cfg, client);

    // the first-use notice: nothing is requested before Continue
    u.menu("Campaign", "Workshop…");
    assert!(u.has("Continue"), "the notice is shown first");
    for _ in 0..5 {
        u.steps(2);
    }
    assert!(
        mock.log.lock().unwrap().is_empty(),
        "no request before the notice is acknowledged"
    );
    assert!(u.app().workshop_state().is_none());
    u.shot("notice");
    u.click("Continue");
    assert!(u.app().settings.workshop.acknowledged);

    // browse (fixture page)
    wait(&mut u, "the browse page", |u| {
        u.app()
            .workshop_state()
            .is_some_and(|w| w.browse_page().is_some())
    });
    assert_eq!(
        u.app()
            .workshop_state()
            .unwrap()
            .browse_page()
            .unwrap()
            .items
            .len(),
        5
    );
    u.shot("browse");
    u.click_first("Maiden of the Nest");
    wait(&mut u, "the item page", |u| {
        u.app()
            .workshop_state()
            .is_some_and(|w| w.detail().is_some())
    });
    assert!(!u
        .app()
        .workshop_state()
        .unwrap()
        .detail()
        .unwrap()
        .files
        .is_empty());
    u.shot("details");

    // download and install into the private install
    u.click_first("Download & install");
    wait(&mut u, "the install plan", |u| {
        u.app()
            .workshop_state()
            .is_some_and(|w| w.pending_plan().is_some())
    });
    u.shot("plan");
    u.click("Install");
    wait(&mut u, "the install", |u| {
        u.app()
            .workshop_state()
            .is_some_and(|w| w.last_install.is_some())
    });
    let installed = root.join("levels/personal/map01500.slb");
    assert!(
        installed.is_file(),
        "the level is in the configured KeeperFX folder"
    );
    assert!(
        u.has("Open level 1500 (Mock level) in the editor")
            || u.h.query_all_by_label_contains("Open level 1500").count() > 0
    );
    u.shot("installed");
    // close the result window, then the Installed tab lists the item
    u.app().workshop_state().unwrap().last_install = None;
    u.steps(2);
    u.click_first("Installed (1)");
    assert_eq!(
        u.app().settings.layout.campaign.workshop_tab,
        "installed",
        "the tab is kept with the layout"
    );
    assert_eq!(u.app().workshop_state().unwrap().installed().len(), 1);
    u.shot("installed-tab");
    u.click("Uninstall");
    u.click("Yes, uninstall");
    wait(&mut u, "the uninstall", |u| {
        u.app()
            .workshop_state()
            .is_some_and(|w| w.installed().is_empty())
    });
    assert!(!installed.exists(), "uninstall removed the level");
    u.shot("uninstalled");
    // every request went to the mock
    let log = mock.log.lock().unwrap().clone();
    assert!(
        log.iter()
            .all(|(url, _)| url.starts_with("https://keeperfx.net/")),
        "{log:?}"
    );
    assert!(
        log.iter()
            .any(|(url, _)| url.contains("/workshop/download/10/")),
        "{log:?}"
    );
    // the real install was not written: its level folders were never linked
    assert!(std::fs::symlink_metadata(root.join("levels"))
        .unwrap()
        .file_type()
        .is_dir());
}
