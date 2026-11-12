//! Updates through the real UI, with GitHub replaced by an in-memory transport (no network):
//! the first-run notice, the check it allows, the banner, the Updates dialog with the release
//! notes, Update now (download, checksum, swap into a scratch install folder, restart of the new
//! version recorded instead of started), and Skip this version. Needs a wgpu adapter, not an
//! install.
use crate::settings::Settings;
use crate::ui_tests::{gpu_available, Ui};
use egui_kittest::kittest::Queryable;
use kfx_formats::testutil;
use kfx_update::client::{Client, ClientConfig};
use kfx_update::install::{detect, BuildInfo, FsProbe, ROLLBACK_DIR};
use kfx_update::platform::{Os, PackageKind, Platform};
use kfx_update::state::StateFiles;
use kfx_update::transport::{Response, Transport};
use kfx_update::updater::{Updater, Worker};
use kfx_update::verify::{sha256_bytes, HttpsOnly};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NEW_EXE: &[u8] = b"kfx-editor 9.0.0";
const NOTES: &str =
    "## What's new\n\n* **Faster** saving\n* see [the manual](https://example.com/m)";

/// GitHub in memory: URL -> (status, headers, body).
struct MemGitHub {
    routes: HashMap<String, (u16, Vec<(String, String)>, Vec<u8>)>,
    log: Mutex<Vec<String>>,
}

impl Transport for MemGitHub {
    fn get(&self, url: &str, _h: &[(&str, String)]) -> kfx_update::Result<Response> {
        self.log.lock().unwrap().push(url.to_string());
        let (status, headers, body) =
            self.routes
                .get(url)
                .cloned()
                .unwrap_or((404, vec![], b"not found".to_vec()));
        Ok(Response {
            status,
            headers,
            body: Box::new(std::io::Cursor::new(body)),
        })
    }
}

fn platform() -> Platform {
    let p = Platform::current();
    if p.release_name().is_some() {
        p
    } else {
        Platform::from_names("linux", "x86_64")
    }
}

/// The package of 9.0.0 for the platform, as the release workflow lays it out.
fn package(p: Platform) -> Vec<u8> {
    let top = format!("kfx-editor-v9.0.0-{}", p.release_name().unwrap());
    let files: [(String, &[u8], u32); 2] = [
        (format!("{top}/{}", p.exe_name()), NEW_EXE, 0o755),
        (format!("{top}/README.md"), b"new readme", 0o644),
    ];
    let mut out = vec![];
    if p.os == Os::Windows {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        for (n, b, m) in &files {
            let o = zip::write::SimpleFileOptions::default().unix_permissions(*m);
            z.start_file(n.as_str(), o).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    } else {
        let gz = flate2::write::GzEncoder::new(&mut out, flate2::Compression::fast());
        let mut t = tar::Builder::new(gz);
        for (n, b, m) in &files {
            let mut h = tar::Header::new_gnu();
            h.set_size(b.len() as u64);
            h.set_mode(*m);
            t.append_data(&mut h, n, *b).unwrap();
        }
        t.into_inner().unwrap().finish().unwrap();
    }
    out
}

/// A scratch install folder, the worker on the in-memory GitHub, wired into the editor.
struct World {
    root: PathBuf,
    exe: PathBuf,
    gh: Arc<MemGitHub>,
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn world(tag: &str) -> World {
    let p = platform();
    let root = testutil::scratch_dir(&format!("kfx-editor/updates-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let install = root.join("install");
    std::fs::create_dir_all(&install).unwrap();
    let exe = install.join(p.exe_name());
    std::fs::write(&exe, b"kfx-editor old").unwrap();
    std::fs::write(install.join("README.md"), b"old readme").unwrap();
    let pkg = package(p);
    let name = format!(
        "kfx-editor-v9.0.0{}",
        p.asset_suffix(PackageKind::Archive).unwrap()
    );
    let sums = format!("{}  {name}\n", sha256_bytes(&pkg));
    let dl = "https://github.com/eddoww/keeperfx-editor/releases/download/v9.0.0";
    let blob = "https://release-assets.githubusercontent.com/github-production-release-asset/1";
    let releases = serde_json::json!([{
        "tag_name": "v9.0.0",
        "name": "KeeperFX Map Editor v9.0.0",
        "body": NOTES,
        "draft": false,
        "prerelease": true,
        "html_url": "https://github.com/eddoww/keeperfx-editor/releases/tag/v9.0.0",
        "assets": [
            {"name": name, "size": pkg.len(), "browser_download_url": format!("{dl}/{name}")},
            {"name": "SHA256SUMS.txt", "size": sums.len(),
             "browser_download_url": format!("{dl}/SHA256SUMS.txt")},
        ],
    }]);
    let redirect = |to: String| (302, vec![("Location".to_string(), to)], vec![]);
    let mut routes = HashMap::new();
    routes.insert(
        "https://api.github.com/repos/eddoww/keeperfx-editor/releases?per_page=30".to_string(),
        (200, vec![], serde_json::to_vec(&releases).unwrap()),
    );
    routes.insert(format!("{dl}/{name}"), redirect(format!("{blob}/{name}")));
    routes.insert(format!("{blob}/{name}"), (200, vec![], pkg));
    routes.insert(
        format!("{dl}/SHA256SUMS.txt"),
        redirect(format!("{blob}/SHA256SUMS.txt")),
    );
    routes.insert(
        format!("{blob}/SHA256SUMS.txt"),
        (200, vec![], sums.into_bytes()),
    );
    World {
        exe,
        root,
        gh: Arc::new(MemGitHub {
            routes,
            log: Mutex::new(vec![]),
        }),
    }
}

fn wire(u: &mut Ui, w: &World) {
    let p = platform();
    let updater = Updater {
        client: Client::with_transport(ClientConfig::github("kfx-editor-test"), w.gh.clone()),
        files: StateFiles::new(w.root.join("state")),
        platform: p,
        build: BuildInfo {
            version: crate::VERSION.into(),
            debug: false,
        },
        verifier: Box::new(HttpsOnly),
    };
    let app = u.app();
    app.updates.worker = Some(Worker::new(updater));
    app.updates.layout = Ok(detect(&w.exe, p, &FsProbe));
    app.updates.blocker = None;
    app.updates.launcher = record;
}

fn record(_: &Path, _: &[OsString]) -> Result<(), String> {
    Ok(())
}

fn until(u: &mut Ui, secs: u64, mut f: impl FnMut(&mut Ui) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        u.steps(1);
        if f(u) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn editor(tag: &str) -> Option<Ui> {
    if !gpu_available() {
        return None;
    }
    let mut u = Ui::with_settings(tag, Settings::default(), egui::vec2(1400.0, 900.0));
    u.app().hide_first_run();
    u.steps(1);
    Some(u)
}

#[test]
fn ui_update_notice_banner_dialog_and_install() {
    let Some(mut u) = editor("updates") else {
        return;
    };
    let w = world("install");
    wire(&mut u, &w);

    // first run: the notice, and nothing sent before it is answered
    u.app().updates_begin();
    u.steps(4);
    assert!(u.has("Check for updates once a day"));
    assert!(u.has("Turn update checks off"));
    u.shot("notice");
    u.steps(5);
    assert!(w.gh.log.lock().unwrap().is_empty(), "nothing sent yet");
    u.click("Check for updates once a day");
    assert!(u.app().settings.updates.notice_acknowledged);
    assert!(u.app().settings.updates.auto_check);

    // the check finds 9.0.0: banner
    let banner = "Version 9.0.0 of the editor is available";
    assert!(until(&mut u, 20, |u| u.has(banner)), "the banner shows");
    u.shot("banner");
    assert_eq!(
        w.gh.log.lock().unwrap().as_slice(),
        ["https://api.github.com/repos/eddoww/keeperfx-editor/releases?per_page=30"]
    );

    // the dialog: versions, notes as plain text
    u.click("Details…");
    u.steps(2);
    assert!(u.has("New version: 9.0.0"));
    u.shot("dialog");
    assert!(
        u.h.query_all_by_label_contains("• Faster saving")
            .next()
            .is_some(),
        "the release notes, Markdown reduced to text"
    );
    assert!(u
        .h
        .query_all_by_label_contains("the manual (https://example.com/m)")
        .next()
        .is_some());

    // Update now: download, verify, swap, restart (recorded)
    u.click("Update now");
    assert!(
        until(&mut u, 30, |u| u.app().updates.launched.is_some()),
        "installed and restarted: {:?}",
        u.app().updates.message
    );
    let (program, args) = u.app().updates.launched.clone().unwrap();
    assert_eq!(program, w.exe);
    assert!(args.is_empty(), "no map was open");
    assert_eq!(std::fs::read(&w.exe).unwrap(), NEW_EXE);
    assert_eq!(
        std::fs::read(
            w.exe
                .parent()
                .unwrap()
                .join(ROLLBACK_DIR)
                .join(platform().exe_name())
        )
        .unwrap(),
        b"kfx-editor old"
    );
}

#[test]
fn ui_update_skip_and_turn_off() {
    let Some(mut u) = editor("updates-skip") else {
        return;
    };
    let w = world("skip");
    wire(&mut u, &w);
    // "Turn update checks off": no request at all
    u.app().updates_begin();
    u.steps(4);
    u.click("Turn update checks off");
    u.steps(5);
    assert!(!u.app().settings.updates.auto_check);
    assert!(u.app().settings.updates.notice_acknowledged);
    assert!(w.gh.log.lock().unwrap().is_empty());
    u.app().updates_begin();
    u.steps(3);
    assert!(
        !u.has("Check for updates once a day"),
        "the notice is not shown again"
    );
    assert!(w.gh.log.lock().unwrap().is_empty(), "checks are off");

    // Check now still works; Skip this version hides it and remembers it
    u.menu("Help", "Check for updates…");
    let banner = "New version: 9.0.0";
    assert!(until(&mut u, 20, |u| u.has(banner)));
    u.click("Skip this version");
    u.steps(2);
    assert_eq!(u.app().settings.updates.skipped, ["9.0.0"]);
    assert!(!u.has("Version 9.0.0 of the editor is available"));
    assert!(u.app().updates.found.is_none());
}
