//! End to end on Linux: a copy of the built `kfx-editor` in a scratch install folder updates
//! itself from a mock GitHub (`kfx-editor update --restart MAP`), the new "version" (a small
//! shell script) replaces it, and the restarted process is the new file, started on the map.
//!
//! Uses the debug-build test hooks `KFX_UPDATE_TEST_API` and `KFX_UPDATE_TEST_ALLOW_DEV`
//! (`src/updates.rs`); release builds ignore them, so the test does nothing there.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "../../kfx-update/tests/support/mod.rs"]
mod support;

use std::process::Command;
use std::time::{Duration, Instant};
use support::*;

fn serve_release(srv: &MockServer, script: &[u8], scratch: &Scratch) {
    let name = "kfx-editor-v99.0.0-linux-x86_64.tar.gz";
    let pkg = scratch.join(name);
    make_package(
        &pkg,
        "kfx-editor-v99.0.0-linux-x86_64",
        "kfx-editor",
        script,
        false,
    );
    let bytes = std::fs::read(&pkg).unwrap();
    let sums = format!("{}  {name}\n", sha256(&bytes));
    let list = serde_json::json!([{
        "tag_name": "v99.0.0",
        "body": "the future",
        "prerelease": true,
        "html_url": "https://github.com/eddoww/keeperfx-editor/releases/tag/v99.0.0",
        "assets": [
            {"name": name, "size": bytes.len(),
             "browser_download_url": srv.url(&format!("/dl/{name}"))},
            {"name": "SHA256SUMS.txt", "size": sums.len(),
             "browser_download_url": srv.url("/dl/SHA256SUMS.txt")},
        ],
    }]);
    srv.put(
        "/repos/eddoww/keeperfx-editor/releases?per_page=30",
        "application/json",
        serde_json::to_vec(&list).unwrap(),
    );
    srv.put(&format!("/dl/{name}"), "application/octet-stream", bytes);
    srv.put("/dl/SHA256SUMS.txt", "text/plain", sums.into_bytes());
}

#[test]
fn a_copy_of_the_editor_updates_itself_and_restarts_as_the_new_file() {
    if !cfg!(debug_assertions) {
        return;
    }
    let s = Scratch::new("editor-e2e");
    let srv = MockServer::start();
    let marker = s.join("restarted.txt");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"new kfx-editor 99.0.0\" \"$0\" \"$@\" > '{}'\n",
        marker.display()
    );
    serve_release(&srv, script.as_bytes(), &s);

    let install = s.join("install");
    std::fs::create_dir_all(&install).unwrap();
    let exe = install.join("kfx-editor");
    let built = std::fs::read(env!("CARGO_BIN_EXE_kfx-editor")).unwrap();
    std::fs::write(&exe, &built).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(install.join("README.md"), b"old").unwrap();
    let map = s.join("level/map00001.slb");
    std::fs::create_dir_all(map.parent().unwrap()).unwrap();
    std::fs::write(&map, b"").unwrap();
    let run = |args: &[&str]| {
        Command::new(&exe)
            .args(args)
            .env("KFX_UPDATE_TEST_API", srv.url(""))
            .env("KFX_UPDATE_TEST_ALLOW_DEV", "1")
            .env("KFX_EDITOR_CONFIG_DIR", s.join("config"))
            .output()
            .unwrap()
    };

    // --version and --check
    let v = run(&["--version"]);
    assert_eq!(
        String::from_utf8_lossy(&v.stdout).lines().next(),
        Some(format!("kfx-editor {}", env!("CARGO_PKG_VERSION")).as_str())
    );
    let c = run(&["update", "--check"]);
    let out = String::from_utf8_lossy(&c.stdout);
    assert_eq!(c.status.code(), Some(10), "{out}");
    assert!(out.contains("latest:  99.0.0"), "{out}");

    // the update itself, then the restart on the map
    let u = run(&["update", "--restart", map.to_str().unwrap()]);
    assert!(
        u.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&u.stdout),
        String::from_utf8_lossy(&u.stderr)
    );
    let end = Instant::now() + Duration::from_secs(20);
    while !marker.exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    let seen = std::fs::read_to_string(&marker).expect("the new version was started");
    let lines: Vec<&str> = seen.lines().collect();
    let canon = std::fs::canonicalize(&exe).unwrap();
    assert_eq!(lines[0], "new kfx-editor 99.0.0");
    assert_eq!(
        std::path::Path::new(lines[1]),
        canon.as_path(),
        "the same path, new file"
    );
    assert_eq!(
        std::path::Path::new(lines[2]),
        std::fs::canonicalize(&map).unwrap().as_path(),
        "started on the map"
    );
    assert_eq!(std::fs::read(&exe).unwrap(), script.as_bytes());
    assert_eq!(
        std::fs::read(install.join("README.md")).unwrap(),
        b"new readme"
    );
    // the previous version is kept for a roll back
    let rb = install.join(".kfx-editor-previous");
    assert_eq!(std::fs::read(rb.join("kfx-editor")).unwrap(), built);
    assert_eq!(
        std::fs::read_to_string(rb.join(".version")).unwrap(),
        env!("CARGO_PKG_VERSION")
    );
}
