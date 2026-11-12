//! Against the real GitHub (ignored by default; one API request and two downloads, about 25 MB):
//! the newest release's Linux archive is downloaded through GitHub's redirect, verified against
//! its SHA256SUMS.txt and staged into a scratch install folder, which checks that the published
//! archives have the layout the updater expects.
//!
//! ```sh
//! cargo test -p kfx-update --test live -- --ignored
//! ```
mod support;
use kfx_update::client::*;
use kfx_update::install::*;
use kfx_update::platform::Platform;
use kfx_update::state::StateFiles;
use kfx_update::updater::Updater;
use kfx_update::verify::HttpsOnly;
use kfx_update::Channel;
use std::sync::atomic::AtomicBool;
use support::Scratch;

#[test]
#[ignore]
fn live_download_verify_and_stage_the_linux_archive() {
    let s = Scratch::new("live");
    let linux = Platform::from_names("linux", "x86_64");
    let u = Updater {
        client: Client::new(ClientConfig::github(&ClientConfig::user_agent_for(
            "0.0.0-live-test",
        ))),
        files: StateFiles::new(s.join("state")),
        platform: linux,
        build: BuildInfo {
            version: "0.0.0".into(),
            debug: false,
        },
        verifier: Box::new(HttpsOnly),
    };
    let req = CheckRequest {
        current: semver::Version::new(0, 0, 0),
        channel: Channel::PreRelease,
        skipped: vec![],
        platform: linux,
        manual: true,
    };
    let up = match u
        .client
        .check(&u.files, &req, kfx_update::state::now_secs())
        .unwrap()
    {
        CheckOutcome::Available(up) => *up,
        o => panic!("{o:?}"),
    };
    eprintln!("newest release: {}", up.release.tag_name);
    let root = s.join("install");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("kfx-editor"), b"old").unwrap();
    let layout = detect(&root.join("kfx-editor"), linux, &FsProbe);
    let file = u
        .download(&up, &layout, &mut |_, _| {}, &AtomicBool::new(false))
        .unwrap();
    let staged = stage(&layout, linux, &file, &up.version.to_string()).unwrap();
    eprintln!("package items: {:?}", staged.items);
    assert!(staged.items.iter().any(|i| i == "kfx-editor"));
    assert!(staged.items.iter().any(|i| i == "docs"));
}
