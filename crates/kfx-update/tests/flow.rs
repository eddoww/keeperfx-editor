//! The whole update against a mock GitHub on the loopback interface: check, download (through
//! a redirect, with resume), verify, install into a scratch "install folder" holding a copy of
//! this test executable, and failure injection at each step.
mod support;
use kfx_update::client::*;
use kfx_update::install::*;
use kfx_update::platform::Platform;
use kfx_update::state::StateFiles;
use kfx_update::transport::UrlPolicy;
use kfx_update::updater::*;
use kfx_update::verify::HttpsOnly;
use kfx_update::{Channel, Error};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use support::*;

const NEW_EXE: &[u8] = b"the new kfx-editor executable";

/// The platform the packages are made for: this one when releases exist for it.
fn platform() -> Platform {
    let p = Platform::current();
    if p.release_name().is_some() {
        p
    } else {
        Platform::from_names("linux", "x86_64")
    }
}

struct World {
    srv: MockServer,
    scratch: Scratch,
    platform: Platform,
    /// The archive of v0.2.0 for the platform.
    archive_name: String,
    archive: Vec<u8>,
}

impl World {
    /// A mock GitHub with the recorded live releases plus a v0.2.0 pre-release for this
    /// platform. `sums` decides the SHA256SUMS.txt content (`None`: no sums file).
    fn new(tag: &str, sums: Option<&dyn Fn(&str, &str) -> String>) -> World {
        let srv = MockServer::start();
        let scratch = Scratch::new(tag);
        let platform = platform();
        let win = platform.os == kfx_update::platform::Os::Windows;
        let suffix = platform
            .asset_suffix(kfx_update::platform::PackageKind::Archive)
            .unwrap();
        let archive_name = format!("kfx-editor-v0.2.0{suffix}");
        let top = format!("kfx-editor-v0.2.0-{}", platform.release_name().unwrap());
        let pkg = scratch.join(&archive_name);
        make_package(&pkg, &top, platform.exe_name(), NEW_EXE, win);
        let archive = std::fs::read(&pkg).unwrap();
        let mut assets = vec![serde_json::json!({
            "name": archive_name,
            "size": archive.len(),
            "browser_download_url": srv.url(&format!("/dl/v0.2.0/{archive_name}")),
        })];
        srv.put(
            &format!("/assets/{archive_name}"),
            "application/octet-stream",
            archive.clone(),
        );
        srv.redirect(
            &format!("/dl/v0.2.0/{archive_name}"),
            &srv.url(&format!("/assets/{archive_name}")),
        );
        if let Some(f) = sums {
            let text = f(&sha256(&archive), &archive_name);
            assets.push(serde_json::json!({
                "name": "SHA256SUMS.txt",
                "size": text.len(),
                "browser_download_url": srv.url("/dl/v0.2.0/SHA256SUMS.txt"),
            }));
            srv.put("/assets/SHA256SUMS.txt", "text/plain", text.into_bytes());
            srv.redirect("/dl/v0.2.0/SHA256SUMS.txt", "/assets/SHA256SUMS.txt");
        }
        let mut list: Vec<serde_json::Value> =
            serde_json::from_str(&live_fixture(&srv.url(""))).unwrap();
        list.insert(
            0,
            serde_json::json!({
                "tag_name": "v0.2.0",
                "name": "KeeperFX Map Editor v0.2.0",
                "body": "Release notes\n\n* new things",
                "draft": false,
                "prerelease": true,
                "html_url": "https://github.com/eddoww/keeperfx-editor/releases/tag/v0.2.0",
                "assets": assets,
            }),
        );
        srv.put(
            "/repos/eddoww/keeperfx-editor/releases?per_page=30",
            "application/json",
            serde_json::to_vec(&list).unwrap(),
        );
        World {
            srv,
            scratch,
            platform,
            archive_name,
            archive,
        }
    }

    fn updater(&self) -> Updater {
        let cfg = ClientConfig {
            repo: DEFAULT_REPO.into(),
            api_base: self.srv.url(""),
            user_agent: "kfx-update-test".into(),
            policy: UrlPolicy::local_test_server(&self.srv.addr),
        };
        Updater {
            client: Client::new(cfg),
            files: StateFiles::new(self.scratch.join("config")),
            platform: self.platform,
            build: BuildInfo {
                version: "0.1.1".into(),
                debug: false,
            },
            verifier: Box::new(HttpsOnly),
        }
    }

    /// An install folder with a copy of this test executable as the editor, the shipped files
    /// and a file of the user's.
    fn install(&self) -> Layout {
        let root = self.scratch.join("install");
        std::fs::create_dir_all(root.join("docs/manual")).unwrap();
        let exe = root.join(self.platform.exe_name());
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        std::fs::write(root.join("README.md"), b"old readme").unwrap();
        std::fs::write(root.join("docs/manual/old.md"), b"old manual").unwrap();
        std::fs::write(root.join("my-notes.txt"), b"mine").unwrap();
        detect(&exe, self.platform, &FsProbe)
    }

    fn request(&self, manual: bool) -> CheckRequest {
        CheckRequest {
            current: "0.1.1".parse().unwrap(),
            channel: Channel::PreRelease,
            skipped: vec![],
            platform: self.platform,
            manual,
        }
    }

    fn check(&self, u: &Updater) -> Update {
        match u
            .client
            .check(&u.files, &self.request(true), kfx_update::state::now_secs())
            .unwrap()
        {
            CheckOutcome::Available(up) => *up,
            o => panic!("expected an update, got {o:?}"),
        }
    }
}

fn good_sums(hash: &str, name: &str) -> String {
    format!("{}  other-file.zip\n{hash}  {name}\n", "0".repeat(64))
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

fn this_exe() -> Vec<u8> {
    std::fs::read(std::env::current_exe().unwrap()).unwrap()
}

#[test]
fn check_download_verify_install() {
    let w = World::new("full", Some(&good_sums));
    let u = w.updater();
    let layout = w.install();
    let up = w.check(&u);
    assert_eq!(up.version.to_string(), "0.2.0");
    assert_eq!(up.archive.as_ref().unwrap().name, w.archive_name);
    assert!(up.release.body.as_deref().unwrap().contains("new things"));
    let mut last = (0, 0);
    let file = u
        .download(&up, &layout, &mut |d, t| last = (d, t), &no_cancel())
        .unwrap();
    assert_eq!(last, (w.archive.len() as u64, w.archive.len() as u64));
    assert_eq!(std::fs::read(&file).unwrap(), w.archive);
    // the request went through the redirect, with the User-Agent
    let log = w.srv.log();
    assert!(log.iter().any(|s| s.path.starts_with("/dl/v0.2.0/")));
    assert!(log
        .iter()
        .all(|s| s.headers.get("user-agent").map(String::as_str) == Some("kfx-update-test")));
    let map = w.scratch.join("map00001.slb");
    let done = u
        .install(&up, &layout, &file, Some(&map), &FsProbe, &RealFs)
        .unwrap();
    assert_eq!(
        done,
        Installed::Swapped {
            exe: layout.exe.clone()
        }
    );
    let root = &layout.root;
    assert_eq!(std::fs::read(&layout.exe).unwrap(), NEW_EXE);
    assert_eq!(
        std::fs::read(root.join("README.md")).unwrap(),
        b"new readme"
    );
    assert!(root.join("docs/manual/updates.md").is_file());
    assert!(
        !root.join("docs/manual/old.md").exists(),
        "docs replaced as a whole"
    );
    assert_eq!(std::fs::read(root.join("my-notes.txt")).unwrap(), b"mine");
    // the rollback copy
    let rb = root.join(ROLLBACK_DIR);
    assert_eq!(
        std::fs::read(rb.join(w.platform.exe_name())).unwrap(),
        this_exe()
    );
    assert_eq!(std::fs::read(rb.join("README.md")).unwrap(), b"old readme");
    assert_eq!(rollback_version(&layout).as_deref(), Some("0.1.1"));
    assert!(!root.join(STAGING_DIR).exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = std::fs::metadata(&layout.exe).unwrap().permissions().mode();
        assert_eq!(m & 0o111, 0o111, "the new executable is executable");
    }
    // first start of the new version: unconfirmed, then confirmed
    let state = u.files.dir.clone();
    match on_start(&state, "0.2.0") {
        StartState::Unconfirmed { health, failed } => {
            assert_eq!(failed, 0);
            assert_eq!(health.previous, "0.1.1");
            // a second start that also never got confirmed
            assert!(matches!(
                on_start(&state, "0.2.0"),
                StartState::Unconfirmed { failed: 1, .. }
            ));
            confirm_healthy(&state, &health);
        }
        s => panic!("{s:?}"),
    }
    assert!(
        !rb.exists(),
        "the rollback copy goes once the new version is healthy"
    );
    assert_eq!(on_start(&state, "0.2.0"), StartState::Normal);
}

#[test]
fn rollback_restores_the_previous_version() {
    let w = World::new("rollback", Some(&good_sums));
    let u = w.updater();
    let layout = w.install();
    let before = snapshot(&layout.root);
    let up = w.check(&u);
    let file = u
        .download(&up, &layout, &mut |_, _| {}, &no_cancel())
        .unwrap();
    u.install(&up, &layout, &file, None, &FsProbe, &RealFs)
        .unwrap();
    assert_eq!(rollback(&layout, &RealFs).unwrap(), "0.1.1");
    cleanup_at_start(&layout);
    let d = diff(&snapshot(&layout.root), &before);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn truncated_download_resumes_and_cancel_keeps_the_part() {
    let w = World::new("resume", Some(&good_sums));
    let u = w.updater();
    let layout = w.install();
    let up = w.check(&u);
    w.srv.faults(Faults {
        truncate_at: Some(w.archive.len() / 3),
        ..Faults::default()
    });
    let e = u
        .download(&up, &layout, &mut |_, _| {}, &no_cancel())
        .unwrap_err();
    assert!(matches!(e, Error::Network { .. }), "{e}");
    // the server is fine again: the next attempt asks only for the rest
    w.srv.faults(Faults::default());
    w.srv.clear_log();
    let file = u
        .download(&up, &layout, &mut |_, _| {}, &no_cancel())
        .unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), w.archive);
    let ranged: Vec<_> = w
        .srv
        .log()
        .into_iter()
        .filter_map(|s| s.headers.get("range").cloned())
        .collect();
    assert_eq!(
        ranged.len(),
        2,
        "one Range request per redirect hop: {ranged:?}"
    );
    assert!(ranged[0].starts_with("bytes=") && ranged[0] != "bytes=0-");
    // cancel: the partial file is kept
    std::fs::remove_file(&file).unwrap();
    let cancel = AtomicBool::new(true);
    let e = u
        .download(&up, &layout, &mut |_, _| {}, &cancel)
        .unwrap_err();
    assert!(matches!(e, Error::Cancelled));
    assert!(kfx_update::client::part_path(&file).exists());
    // a server that ignores Range: the download starts again from zero and still verifies
    w.srv.faults(Faults {
        no_ranges: true,
        ..Faults::default()
    });
    let file = u
        .download(&up, &layout, &mut |_, _| {}, &no_cancel())
        .unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), w.archive);
}

#[test]
fn wrong_hash_or_missing_sums_refuse_and_leave_the_install_intact() {
    let wrong = |_: &str, name: &str| format!("{}  {name}\n", "ab".repeat(32));
    for (tag, sums) in [
        ("wrong", Some(&wrong as &dyn Fn(&str, &str) -> String)),
        ("nosums", None),
        (
            "unlisted",
            Some(
                &(|h: &str, _: &str| format!("{h}  some-other-file.tar.gz\n"))
                    as &dyn Fn(&str, &str) -> String,
            ),
        ),
        (
            "garbage",
            Some(
                &(|_: &str, _: &str| "not a sums file\n".to_string())
                    as &dyn Fn(&str, &str) -> String,
            ),
        ),
    ] {
        let w = World::new(tag, sums);
        let u = w.updater();
        let layout = w.install();
        let before = snapshot(&layout.root);
        let up = w.check(&u);
        let e = u
            .download(&up, &layout, &mut |_, _| {}, &no_cancel())
            .unwrap_err();
        match tag {
            "wrong" => assert!(matches!(e, Error::ChecksumMismatch { .. }), "{e}"),
            "garbage" => assert!(matches!(e, Error::Parse { .. }), "{e}"),
            _ => assert!(matches!(e, Error::MissingChecksum(_)), "{tag}: {e}"),
        }
        // nothing left in the downloads, nothing changed in the install
        let dl = u.files.downloads_dir();
        let leftovers: Vec<_> = walk(&dl).into_iter().filter(|p| p.is_file()).collect();
        assert!(leftovers.is_empty(), "{tag}: {leftovers:?}");
        let d = diff(&snapshot(&layout.root), &before);
        assert!(d.is_empty(), "{tag}: {d:?}");
    }
}

#[test]
fn a_corrupted_package_is_refused_by_the_size_check() {
    let w = World::new("size", Some(&good_sums));
    let u = w.updater();
    let layout = w.install();
    let mut up = w.check(&u);
    // the release lists a different size than the file has
    up.archive.as_mut().unwrap().size += 7;
    let e = u
        .download(&up, &layout, &mut |_, _| {}, &no_cancel())
        .unwrap_err();
    assert!(
        matches!(e, Error::Network { .. } | Error::SizeMismatch { .. }),
        "{e}"
    );
}

#[test]
fn interrupted_swap_leaves_the_original_intact() {
    /// Fails the n-th rename.
    struct FailAt(std::sync::atomic::AtomicUsize, usize);
    impl FsOps for FailAt {
        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            let i = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if i == self.1 {
                return Err(std::io::Error::other("injected failure"));
            }
            std::fs::rename(from, to)
        }
    }
    let w = World::new("swapfail", Some(&good_sums));
    let u = w.updater();
    let layout = w.install();
    let up = w.check(&u);
    let file = u
        .download(&up, &layout, &mut |_, _| {}, &no_cancel())
        .unwrap();
    let before = snapshot(&layout.root);
    // the package has 4 top-level items (exe, README, LICENSE, docs): up to 7 renames
    for n in 0..7 {
        let kept = w.scratch.join(&format!("pkg-{n}"));
        std::fs::copy(&file, &kept).unwrap();
        let e = u
            .install(
                &up,
                &layout,
                &kept,
                None,
                &FsProbe,
                &FailAt(Default::default(), n),
            )
            .unwrap_err();
        assert!(matches!(e, Error::Swap(_)), "step {n}: {e}");
        cleanup_at_start(&layout);
        let d = diff(&snapshot(&layout.root), &before);
        assert!(d.is_empty(), "step {n}: {d:?}");
    }
}

#[cfg(unix)]
#[test]
fn unwritable_folder_is_a_blocker_not_a_failure() {
    use std::os::unix::fs::PermissionsExt;
    let w = World::new("ro", Some(&good_sums));
    let u = w.updater();
    let layout = w.install();
    let up = w.check(&u);
    let file = u
        .download(&up, &layout, &mut |_, _| {}, &no_cancel())
        .unwrap();
    let before = snapshot(&layout.root);
    std::fs::set_permissions(&layout.root, std::fs::Permissions::from_mode(0o555)).unwrap();
    if FsProbe.writable(&layout.root).is_ok() {
        // running as root: permissions do not apply
        std::fs::set_permissions(&layout.root, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let e = u
        .install(&up, &layout, &file, None, &FsProbe, &RealFs)
        .unwrap_err();
    std::fs::set_permissions(&layout.root, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(e, Error::Blocked(Blocker::NotWritable { .. })),
        "{e}"
    );
    let d = diff(&snapshot(&layout.root), &before);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn dev_builds_and_managed_installs_never_install() {
    let w = World::new("blocked", Some(&good_sums));
    let mut u = w.updater();
    let layout = w.install();
    let up = w.check(&u);
    let file = u
        .download(&up, &layout, &mut |_, _| {}, &no_cancel())
        .unwrap();
    let before = snapshot(&layout.root);
    u.build.debug = true;
    let e = u
        .install(&up, &layout, &file, None, &FsProbe, &RealFs)
        .unwrap_err();
    assert!(matches!(e, Error::Blocked(Blocker::DevBuild)), "{e}");
    u.build.debug = false;
    std::fs::write(layout.root.join(MANAGED_MARKER), b"").unwrap();
    let e = u
        .install(&up, &layout, &file, None, &FsProbe, &RealFs)
        .unwrap_err();
    assert!(matches!(e, Error::Blocked(Blocker::Managed(_))), "{e}");
    std::fs::remove_file(layout.root.join(MANAGED_MARKER)).unwrap();
    let d = diff(&snapshot(&layout.root), &before);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn etag_rate_limit_and_daily_interval() {
    let w = World::new("etag", Some(&good_sums));
    w.srv.state.lock().unwrap().etag = Some("W/\"abc\"".into());
    let u = w.updater();
    let now = kfx_update::state::now_secs();
    // first automatic check: asks
    let o = u.client.check(&u.files, &w.request(false), now).unwrap();
    assert!(matches!(o, CheckOutcome::Available(_)));
    assert_eq!(w.srv.log().len(), 1);
    // within a day: no request at all
    let o = u
        .client
        .check(&u.files, &w.request(false), now + 3600)
        .unwrap();
    assert_eq!(o, CheckOutcome::NotDue);
    assert_eq!(w.srv.log().len(), 1);
    // "Check now" asks with the ETag and gets 304: the cached list answers
    let o = u
        .client
        .check(&u.files, &w.request(true), now + 3600)
        .unwrap();
    assert!(matches!(o, CheckOutcome::Available(_)));
    let log = w.srv.log();
    assert_eq!(log.len(), 2);
    assert_eq!(
        log[1].headers.get("if-none-match").map(String::as_str),
        Some("W/\"abc\"")
    );
    assert_eq!(
        log[1].headers.get("accept").map(String::as_str),
        Some("application/vnd.github+json")
    );
    // rate limited: the reset time is remembered and no request is made before it
    let reset = now + 9000;
    w.srv.faults(Faults {
        rate_limited_until: Some(reset),
        ..Faults::default()
    });
    let e = u
        .client
        .check(&u.files, &w.request(true), now + 7200)
        .unwrap_err();
    assert!(matches!(e, Error::RateLimited { .. }), "{e}");
    let st = u.files.load();
    assert_eq!(st.backoff_until, Some(reset));
    let n = w.srv.log().len();
    let e = u
        .client
        .check(&u.files, &w.request(true), now + 7300)
        .unwrap_err();
    assert!(matches!(e, Error::RateLimited { .. }));
    assert_eq!(w.srv.log().len(), n, "no request while backing off");
}

#[test]
fn the_policy_is_enforced_on_every_hop() {
    let w = World::new("policy", Some(&good_sums));
    // the real GitHub policy refuses the plain-HTTP mock server outright
    let cfg = ClientConfig {
        api_base: w.srv.url(""),
        ..ClientConfig::github("kfx-update-test")
    };
    let c = Client::new(cfg);
    let mut st = Default::default();
    let e = c
        .fetch_releases(&StateFiles::new(w.scratch.join("cfg")), &mut st, 0)
        .unwrap_err();
    assert!(matches!(e, Error::UrlNotAllowed(_)), "{e}");
    assert!(w.srv.log().is_empty());
    // a redirect to another host is refused
    let u = w.updater();
    let layout = w.install();
    let up = w.check(&u);
    w.srv.redirect(
        &format!("/dl/v0.2.0/{}", w.archive_name),
        "http://203.0.113.9/evil.tar.gz",
    );
    let e = u
        .download(&up, &layout, &mut |_, _| {}, &no_cancel())
        .unwrap_err();
    assert!(matches!(e, Error::UrlNotAllowed(_)), "{e}");
}

/// Replace the executable while a copy of it runs (the case that needs the rename dance on
/// Windows, where a running `.exe` cannot be overwritten). Runs on every platform; on the
/// Windows CI job it exercises the Windows behaviour.
#[test]
fn replaces_a_running_executable() {
    let w = World::new("running", Some(&good_sums));
    let u = w.updater();
    let layout = w.install();
    let up = w.check(&u);
    let file = u
        .download(&up, &layout, &mut |_, _| {}, &no_cancel())
        .unwrap();
    // run the copy: this test binary, told to run only the helper below, which sleeps
    let mut child = std::process::Command::new(&layout.exe)
        .args([
            "--ignored",
            "--exact",
            "sleeping_helper",
            "--test-threads=1",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    #[cfg(windows)]
    {
        // the point of the exercise: overwriting a running executable fails on Windows
        assert!(std::fs::write(&layout.exe, b"x").is_err());
    }
    let r = u.install(&up, &layout, &file, None, &FsProbe, &RealFs);
    let _ = child.kill();
    let _ = child.wait();
    r.unwrap();
    assert_eq!(std::fs::read(&layout.exe).unwrap(), NEW_EXE);
    assert_eq!(
        std::fs::read(layout.root.join(ROLLBACK_DIR).join(w.platform.exe_name())).unwrap(),
        this_exe()
    );
    // the old executable is free again now: the rollback folder can go
    let state = u.files.dir.clone();
    if let StartState::Unconfirmed { health, .. } = on_start(&state, "0.2.0") {
        confirm_healthy(&state, &health);
    }
    assert!(!layout.root.join(ROLLBACK_DIR).exists());
}

/// Not a test: the process [`replaces_a_running_executable`] keeps running.
#[test]
#[ignore]
fn sleeping_helper() {
    std::thread::sleep(std::time::Duration::from_secs(20));
}

fn walk(d: &Path) -> Vec<PathBuf> {
    let mut v = vec![];
    if let Ok(rd) = std::fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                v.extend(walk(&p));
            }
            v.push(p);
        }
    }
    v
}
