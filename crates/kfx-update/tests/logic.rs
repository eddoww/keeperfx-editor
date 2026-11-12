//! Pure logic: versions, channels, skipped versions, assets per platform, checksums, rate limits
//! and back-off, URL policy, install layouts and blockers, the installer command line.
use kfx_update::client::{outcome, CheckOutcome, CheckRequest};
use kfx_update::install::*;
use kfx_update::platform::{select_asset, PackageKind, Platform};
use kfx_update::release::*;
use kfx_update::state::*;
use kfx_update::transport::{resolve_location, UrlPolicy};
use kfx_update::verify::Sums;
use kfx_update::Error;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn rel(tag: &str, pre: bool) -> Release {
    Release {
        tag_name: tag.into(),
        name: None,
        body: None,
        draft: false,
        prerelease: pre,
        html_url: String::new(),
        published_at: None,
        assets: vec![],
    }
}

fn v(s: &str) -> semver::Version {
    s.parse().unwrap()
}

fn live() -> Vec<Release> {
    parse_releases(
        &std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/releases-live.json"
        ))
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn tags_and_versions() {
    assert_eq!(parse_version("v0.1.1"), Some(v("0.1.1")));
    assert_eq!(parse_version("0.1.1"), Some(v("0.1.1")));
    assert_eq!(parse_version(" V1.2.3-rc.1 "), Some(v("1.2.3-rc.1")));
    assert_eq!(parse_version("v1.2"), Some(v("1.2.0")));
    for bad in [
        "nightly",
        "v1",
        "1.2.3.4",
        "v1.x",
        "",
        "v",
        "latest",
        "1..2",
        "v1.2.3 beta",
    ] {
        assert_eq!(parse_version(bad), None, "{bad:?}");
    }
    assert!(!is_dev_version("0.1.1"));
    assert!(is_dev_version("0.2.0-dev"));
    assert!(is_dev_version("0.2.0+dirty"));
    assert!(is_dev_version("garbage"));
}

#[test]
fn selection_by_channel_semver_and_skips() {
    let list = vec![
        rel("v0.1.1", true),
        rel("v0.3.0", true),
        rel("v0.2.10", false),
        rel("v0.2.9", false),
        rel("nightly", true),
        rel("v0.4.0", false),
    ];
    let mut list = list;
    list[5].draft = true; // drafts never count
    let cur = v("0.1.1");
    let pick = |ch, skip: &[&str]| {
        let skip: Vec<String> = skip.iter().map(|s| s.to_string()).collect();
        select(&list, &cur, ch, &skip).map(|(r, _)| r.tag_name.clone())
    };
    assert_eq!(pick(Channel::PreRelease, &[]).as_deref(), Some("v0.3.0"));
    // stable: only releases GitHub does not mark as pre-releases; 0.2.10 > 0.2.9 by semver
    assert_eq!(pick(Channel::Stable, &[]).as_deref(), Some("v0.2.10"));
    // skipping the newest offers nothing (older ones are superseded); tags with or without v
    assert_eq!(pick(Channel::PreRelease, &["0.3.0"]), None);
    assert_eq!(pick(Channel::Stable, &["v0.2.10"]), None);
    // the current version is not newer than itself
    assert_eq!(select(&list, &v("0.3.0"), Channel::PreRelease, &[]), None);
    // a semver pre-release of a newer version is newer than the current release
    let rc = vec![rel("v0.2.0-rc.1", true)];
    assert!(select(&rc, &cur, Channel::PreRelease, &[]).is_some());
    assert!(select(&rc, &v("0.2.0"), Channel::PreRelease, &[]).is_none());
    // the default channel is pre-release while every release is one
    assert_eq!(Channel::default(), Channel::PreRelease);
    assert_eq!(
        serde_json::to_string(&Channel::PreRelease).unwrap(),
        "\"pre-release\""
    );
}

#[test]
fn the_recorded_live_answer_parses() {
    let l = live();
    assert_eq!(l.len(), 2);
    assert!(l.iter().all(|r| r.prerelease && !r.draft));
    let req = |cur: &str, ch| CheckRequest {
        current: v(cur),
        channel: ch,
        skipped: vec![],
        platform: Platform::from_names("linux", "x86_64"),
        manual: true,
    };
    match outcome(&l, &req("0.1.0", Channel::PreRelease)) {
        CheckOutcome::Available(u) => {
            assert_eq!(u.version, v("0.1.1"));
            assert_eq!(
                u.archive.unwrap().name,
                "kfx-editor-v0.1.1-linux-x86_64.tar.gz"
            );
            assert_eq!(u.sums.unwrap().size, 414);
            assert!(u.setup.is_none());
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(
        outcome(&l, &req("0.1.1", Channel::PreRelease)),
        CheckOutcome::UpToDate {
            latest: Some(v("0.1.1"))
        }
    );
    // on the stable channel no release is offered at all today
    assert_eq!(
        outcome(&l, &req("0.1.0", Channel::Stable)),
        CheckOutcome::UpToDate { latest: None }
    );
}

#[test]
fn asset_per_platform() {
    let l = live();
    let r = &l[0];
    let name = |os, arch, kind| {
        select_asset(r, Platform::from_names(os, arch), kind).map(|a| a.name.clone())
    };
    use PackageKind::*;
    assert_eq!(
        name("linux", "x86_64", Archive).as_deref(),
        Some("kfx-editor-v0.1.1-linux-x86_64.tar.gz")
    );
    assert_eq!(
        name("windows", "x86_64", Archive).as_deref(),
        Some("kfx-editor-v0.1.1-windows-x86_64.zip")
    );
    assert_eq!(
        name("macos", "aarch64", Archive).as_deref(),
        Some("kfx-editor-v0.1.1-macos-arm64.tar.gz")
    );
    assert_eq!(
        name("macos", "x86_64", Archive).as_deref(),
        Some("kfx-editor-v0.1.1-macos-x86_64.tar.gz")
    );
    assert_eq!(
        name("windows", "x86_64", WindowsSetup),
        None,
        "0.1.1 has no installer"
    );
    assert_eq!(name("linux", "aarch64", Archive), None);
    assert_eq!(name("freebsd", "x86_64", Archive), None);
    assert_eq!(name("linux", "x86_64", WindowsSetup), None);
    // an installer next to the zip: each kind finds its own
    let mut r2 = rel("v0.2.0", true);
    for n in [
        "kfx-editor-v0.2.0-windows-x86_64.zip",
        "kfx-editor-v0.2.0-windows-x86_64-setup.exe",
    ] {
        r2.assets.push(Asset {
            name: n.into(),
            size: 1,
            browser_download_url: String::new(),
            digest: None,
        });
    }
    let w = Platform::from_names("windows", "x86_64");
    assert_eq!(
        select_asset(&r2, w, WindowsSetup).unwrap().name,
        "kfx-editor-v0.2.0-windows-x86_64-setup.exe"
    );
    assert_eq!(
        select_asset(&r2, w, Archive).unwrap().name,
        "kfx-editor-v0.2.0-windows-x86_64.zip"
    );
    // a tag spelt differently in the file name is still found when it is the only candidate
    r2.tag_name = "0.2.0".into();
    assert_eq!(
        select_asset(&r2, w, Archive).unwrap().name,
        "kfx-editor-v0.2.0-windows-x86_64.zip"
    );
}

#[test]
fn checksums() {
    let real = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/SHA256SUMS-v0.1.1.txt"
    ))
    .unwrap();
    let s = Sums::parse(&real).unwrap();
    assert_eq!(s.len(), 4);
    assert_eq!(
        s.get("kfx-editor-v0.1.1-windows-x86_64.zip"),
        Some("a2d8462e4b803060adc8bc8ca508f2cc7c6498ef66d252812ee780d5fa0ce5fa")
    );
    // the published digest of the API agrees with the sums file
    let l = live();
    for a in &l[0].assets {
        if let Some(h) = s.get(&a.name) {
            assert_eq!(a.digest.as_deref(), Some(format!("sha256:{h}").as_str()));
        }
    }
    let h = "A".repeat(64);
    // binary mode marker, CRLF, upper case, comments and blank lines
    let s = Sums::parse(&format!("# sums\r\n\r\n{h} *x.zip\r\n")).unwrap();
    assert_eq!(s.get("x.zip"), Some("a".repeat(64).as_str()));
    for bad in [
        "abc  x.zip",
        &format!("{h}x.zip"),
        &format!("{h}  "),
        &format!("{h}  ../x.zip"),
        &format!("{}  x.zip", "g".repeat(64)),
        &format!("{h}  x.zip\n{}  x.zip", "b".repeat(64)),
    ] {
        assert!(Sums::parse(bad).is_err(), "{bad:?}");
    }
    // the same line twice is fine
    assert!(Sums::parse(&format!("{h}  x\n{h}  x\n")).is_ok());
}

#[test]
fn rate_limits_and_backoff() {
    let now = 1_000_000;
    let hdr = |pairs: &'static [(&'static str, &'static str)]| {
        move |n: &str| {
            pairs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(n))
                .map(|(_, v)| v.to_string())
        }
    };
    let exhausted = hdr(&[
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-reset", "1003600"),
    ]);
    assert_eq!(rate_limit_until(403, &exhausted, now), Some(1_003_600));
    // a 403 for another reason is not a rate limit
    let fine = hdr(&[("x-ratelimit-remaining", "12")]);
    assert_eq!(rate_limit_until(403, &fine, now), None);
    assert_eq!(rate_limit_until(200, &exhausted, now), None);
    // secondary limits: retry-after
    let retry = hdr(&[("retry-after", "120")]);
    assert_eq!(rate_limit_until(429, &retry, now), Some(now + 120));
    assert_eq!(rate_limit_until(403, &retry, now), Some(now + 120));
    // 429 without headers: an hour
    assert_eq!(rate_limit_until(429, &hdr(&[]), now), Some(now + 3600));
    // a reset in the past is not trusted
    let stale = hdr(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "5")]);
    assert_eq!(rate_limit_until(403, &stale, now), Some(now + 3600));

    let mut s = CheckState::default();
    assert!(s.auto_due(now));
    let err = Error::Network {
        url: "u".into(),
        message: "m".into(),
    };
    s.failed(now, &err, None);
    assert_eq!(s.backoff_until, Some(now + BACKOFF_BASE_SECS));
    assert!(!s.auto_due(now + 60));
    s.failed(now, &err, None);
    assert_eq!(s.backoff_until, Some(now + 2 * BACKOFF_BASE_SECS));
    for _ in 0..20 {
        s.failed(now, &err, None);
    }
    assert_eq!(
        s.backoff_until,
        Some(now + AUTO_INTERVAL_SECS),
        "capped at a day"
    );
    assert!(s.last_error.is_some());
    s.succeeded(now, Some("e".into()));
    assert_eq!((s.failures, s.backoff_until), (0, None));
    assert_eq!(s.etag.as_deref(), Some("e"));
    // a 304 keeps the ETag
    s.succeeded(now, None);
    assert_eq!(s.etag.as_deref(), Some("e"));
    assert!(!s.auto_due(now + 3600));
    assert!(s.auto_due(now + AUTO_INTERVAL_SECS));
    // a clock that went backwards does not block checks forever
    assert!(s.auto_due(now - 10));
}

#[test]
fn url_policy() {
    let p = UrlPolicy::github();
    for ok in [
        "https://api.github.com/repos/eddoww/keeperfx-editor/releases",
        "https://github.com/eddoww/keeperfx-editor/releases/download/v0.1.1/x.zip",
        "https://release-assets.githubusercontent.com/github-production-release-asset/1/2?sig=x",
        "https://objects.githubusercontent.com/x",
        "https://API.GitHub.com:443/x",
    ] {
        assert!(p.check(ok).is_ok(), "{ok}");
    }
    for bad in [
        "http://api.github.com/x",
        "https://evil.com/x",
        "https://github.com.evil.com/x",
        "https://user@github.com/x",
        "https://github.com:8443/x",
        "ftp://github.com/x",
        "github.com/x",
        "https:///x",
        "file:///etc/passwd",
    ] {
        assert!(p.check(bad).is_err(), "{bad}");
    }
    let t = UrlPolicy::local_test_server("127.0.0.1:4000");
    assert!(t.check("http://127.0.0.1:4000/x").is_ok());
    assert!(t.check("http://127.0.0.1:4001/x").is_err());
    assert!(t.check("https://api.github.com/x").is_err());
    assert_eq!(
        resolve_location("https://github.com/a/b", "/c?d").as_deref(),
        Some("https://github.com/c?d")
    );
    assert_eq!(
        resolve_location("https://github.com/a", "//evil.com/x"),
        None
    );
    assert_eq!(resolve_location("https://github.com/a", "relative"), None);
}

/// A file system that exists only in the test.
struct FakeProbe {
    files: HashSet<PathBuf>,
    read_only: HashSet<PathBuf>,
    env: Vec<(&'static str, &'static str)>,
}

impl FakeProbe {
    fn new(files: &[&str]) -> Self {
        FakeProbe {
            files: files.iter().map(PathBuf::from).collect(),
            read_only: HashSet::new(),
            env: vec![],
        }
    }
}

impl Probe for FakeProbe {
    fn exists(&self, p: &Path) -> bool {
        self.files.contains(p)
    }
    fn writable(&self, dir: &Path) -> Result<(), String> {
        if self.read_only.contains(dir) {
            Err("Permission denied (os error 13)".into())
        } else {
            Ok(())
        }
    }
    fn env(&self, name: &str) -> Option<OsString> {
        self.env
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| OsString::from(v))
    }
}

fn build() -> BuildInfo {
    BuildInfo {
        version: "0.1.1".into(),
        debug: false,
    }
}

#[test]
fn layouts() {
    let mac = Platform::from_names("macos", "aarch64");
    let win = Platform::from_names("windows", "x86_64");
    let lin = Platform::from_names("linux", "x86_64");
    let none = FakeProbe::new(&[]);
    let l = detect(
        Path::new("/Applications/KeeperFX Map Editor.app/Contents/MacOS/kfx-editor"),
        mac,
        &none,
    );
    assert_eq!(
        l.kind,
        LayoutKind::MacBundle {
            app: "/Applications/KeeperFX Map Editor.app".into()
        }
    );
    assert_eq!(l.root, Path::new("/Applications"));
    // the bare executable of the macOS archive is a portable install
    let l = detect(Path::new("/Users/u/kfx/kfx-editor"), mac, &none);
    assert_eq!(l.kind, LayoutKind::Portable);
    // Windows: the installer's marker decides
    let exe = Path::new("C:/Users/u/AppData/Local/Programs/KeeperFX Map Editor/kfx-editor.exe");
    assert_eq!(detect(exe, win, &none).kind, LayoutKind::Portable);
    let marked =
        FakeProbe::new(&["C:/Users/u/AppData/Local/Programs/KeeperFX Map Editor/setup-install"]);
    assert_eq!(detect(exe, win, &marked).kind, LayoutKind::WindowsSetup);
    // the marker means nothing on Linux
    let l = detect(
        Path::new("/opt/kfx/kfx-editor"),
        lin,
        &FakeProbe::new(&["/opt/kfx/setup-install"]),
    );
    assert_eq!(l.kind, LayoutKind::Portable);
    assert_eq!(l.root, Path::new("/opt/kfx"));
}

#[test]
fn blockers() {
    let lin = Platform::from_names("linux", "x86_64");
    let mac = Platform::from_names("macos", "x86_64");
    let probe = FakeProbe::new(&[]);
    let l = detect(Path::new("/home/u/kfx/kfx-editor"), lin, &probe);
    assert_eq!(eligibility(&l, &build(), lin, &probe), Ok(()));
    let dev = |version: &str, debug| BuildInfo {
        version: version.into(),
        debug,
    };
    assert_eq!(
        eligibility(&l, &dev("0.1.1", true), lin, &probe),
        Err(Blocker::DevBuild)
    );
    assert_eq!(
        eligibility(&l, &dev("0.2.0-dev", false), lin, &probe),
        Err(Blocker::DevBuild)
    );
    let arm = Platform::from_names("linux", "aarch64");
    assert!(matches!(
        eligibility(&l, &build(), arm, &probe),
        Err(Blocker::UnsupportedPlatform(_))
    ));
    // package manager markers: beside the binary, or in <prefix>/share/kfx-editor
    let m = FakeProbe::new(&["/home/u/kfx/managed-install"]);
    assert!(matches!(
        eligibility(&l, &build(), lin, &m),
        Err(Blocker::Managed(_))
    ));
    let usr = detect(Path::new("/usr/bin/kfx-editor"), lin, &probe);
    let m = FakeProbe::new(&["/usr/share/kfx-editor/managed-install"]);
    assert_eq!(
        eligibility(&usr, &build(), lin, &m),
        Err(Blocker::Managed(
            "/usr/share/kfx-editor/managed-install".into()
        ))
    );
    // not writable
    let mut ro = FakeProbe::new(&[]);
    ro.read_only.insert("/home/u/kfx".into());
    assert!(matches!(
        eligibility(&l, &build(), lin, &ro),
        Err(Blocker::NotWritable { .. })
    ));
    // AppImage, translocated and mounted macOS apps
    let mut ai = FakeProbe::new(&[]);
    ai.env.push(("APPIMAGE", "/home/u/kfx.AppImage"));
    assert!(matches!(
        eligibility(&l, &build(), lin, &ai),
        Err(Blocker::ReadOnlyImage(_))
    ));
    for p in [
        "/private/var/folders/x/AppTranslocation/ABC/d/KeeperFX Map Editor.app/Contents/MacOS/kfx-editor",
        "/Volumes/KeeperFX Map Editor/KeeperFX Map Editor.app/Contents/MacOS/kfx-editor",
    ] {
        let l = detect(Path::new(p), mac, &probe);
        assert!(matches!(eligibility(&l, &build(), mac, &probe), Err(Blocker::ReadOnlyImage(_))), "{p}");
    }
    // a bundle needs its parent writable (the bundle is renamed)
    let l = detect(
        Path::new("/Applications/K.app/Contents/MacOS/kfx-editor"),
        mac,
        &probe,
    );
    let mut ro = FakeProbe::new(&[]);
    ro.read_only.insert("/Applications".into());
    assert!(matches!(
        eligibility(&l, &build(), mac, &ro),
        Err(Blocker::NotWritable { .. })
    ));
    // every blocker explains itself
    assert!(Blocker::DevBuild.to_string().contains("development build"));
}

#[test]
fn installer_command_line() {
    let (p, a) = setup_command(
        Path::new("C:/cfg/update-downloads/v0.2.0/kfx-editor-v0.2.0-windows-x86_64-setup.exe"),
        Some(Path::new("C:/maps/my levels/map00001.slb")),
    );
    assert!(p.ends_with("kfx-editor-v0.2.0-windows-x86_64-setup.exe"));
    let a: Vec<String> = a.iter().map(|s| s.to_string_lossy().into_owned()).collect();
    assert_eq!(
        a,
        [
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/RELAUNCH",
            "/OPEN=C:/maps/my levels/map00001.slb"
        ]
    );
    let (_, a) = setup_command(Path::new("s.exe"), None);
    assert_eq!(a.len(), 4);
    // the installer script reads the same switches and writes the same marker
    let iss = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packaging/windows/installer.iss"
    ))
    .unwrap();
    for needle in [
        "RELAUNCH",
        "{param:OPEN|}",
        SETUP_MARKER,
        ROLLBACK_DIR,
        STAGING_DIR,
    ] {
        assert!(iss.contains(needle), "installer.iss lacks {needle}");
    }
}
