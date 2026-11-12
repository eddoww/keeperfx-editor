//! Game harness run: a NEW campaign created by this crate, with two copied levels, starts its
//! first level in the real game with no new log errors compared with the original level.
//!
//! Needs `KFX_GAME_TEST=1` and the test install (skipped otherwise). The game runs only through
//! the offscreen harness of `kfx-map` (`crates/kfx-map/tests/support/game.rs`, included here by
//! path): SDL offscreen video, dummy audio, SIGTERM after a timeout, one game at a time (the
//! same lock folder). The rig is a private install of symbolic links in the test scratch
//! folder (`campaign-ui/inst-game`); the install is never written.
#![cfg(unix)]
mod common;
#[path = "../../kfx-map/tests/support/game.rs"]
mod game;

use common::*;
use game::{GameRun, LogLine};
use kfx_campaign_ui::pack::{AddLevel, LevelSource};
use kfx_campaign_ui::{create_pack, validate, NewPack, Severity};
use kfx_formats::levelinfo::LevelKind;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const LOCK_DIR: &str = "/tmp/kfx-map-gametest.lock";
const SECS: u64 = 25;

/// Same protocol as `support/game.rs`: a lock folder holding the owner's pid.
struct DirLock;

impl DirLock {
    fn acquire(timeout: Duration) -> DirLock {
        let t = Instant::now();
        loop {
            match std::fs::create_dir(LOCK_DIR) {
                Ok(()) => {
                    let _ = std::fs::write(
                        Path::new(LOCK_DIR).join("pid"),
                        std::process::id().to_string(),
                    );
                    return DirLock;
                }
                Err(_) => {
                    let pid = std::fs::read_to_string(Path::new(LOCK_DIR).join("pid"))
                        .unwrap_or_default();
                    let alive =
                        !pid.trim().is_empty() && Path::new("/proc").join(pid.trim()).exists();
                    if !alive && t.elapsed() > Duration::from_secs(2) {
                        let _ = std::fs::remove_dir_all(LOCK_DIR);
                        continue;
                    }
                    assert!(t.elapsed() < timeout, "another game run holds {LOCK_DIR}");
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(LOCK_DIR);
    }
}

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn terminate(&mut self) {
        let Some(mut c) = self.0.take() else { return };
        let _ = Command::new("kill")
            .args(["-TERM", &c.id().to_string()])
            .status();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if let Ok(Some(_)) = c.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = c.kill();
        let _ = c.wait();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// A private install: everything symlinked except what the game writes (`keeperfx`, its cfg and
/// log, saves, replays, screenshots) and `campgns` / `levels` (real folders of symlinks, so the
/// new campaign can be added).
fn make_rig(install: &Path) -> PathBuf {
    let dir = private_install("game").expect("install");
    for n in [
        "keeperfx",
        "keeperfx.cfg",
        "keeperfx.log",
        "replays",
        "save",
        "scrshots",
    ] {
        let p = dir.join(n);
        if std::fs::symlink_metadata(&p).is_ok() {
            std::fs::remove_file(&p).unwrap();
        }
    }
    std::fs::copy(install.join("keeperfx"), dir.join("keeperfx")).unwrap();
    for d in ["replays", "save", "scrshots"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    // windowed 640x480, no cursor capture, as the kfx-map rig does
    let cfg = std::fs::read_to_string(install.join("keeperfx.cfg")).unwrap();
    let set = [
        ("FRONTEND_RES", "640x480w32 640x480w32 640x480w32"),
        ("INGAME_RES", "640x480w32"),
        ("CAPTURE_CURSOR", "OFF"),
        ("CURSOR_EDGE_CAMERA_PANNING", "OFF"),
        ("ATMOSPHERIC_SOUNDS", "OFF"),
    ];
    let mut out = String::new();
    let mut seen = vec![];
    for line in cfg.lines() {
        let key = line.split('=').next().unwrap_or("").trim();
        match set.iter().find(|(k, _)| *k == key) {
            Some((k, v)) => {
                out.push_str(&format!("{k}={v}\n"));
                seen.push(*k);
            }
            None => out.push_str(&format!("{line}\n")),
        }
    }
    for (k, v) in set {
        if !seen.contains(&k) {
            out.push_str(&format!("{k}={v}\n"));
        }
    }
    std::fs::write(dir.join("keeperfx.cfg"), out).unwrap();
    dir
}

/// The level number is part of some messages (`map00011.lgtfx`): compare them without it.
fn normalized(mut run: GameRun, level: u32) -> GameRun {
    let from = format!("map{level:05}");
    for l in &mut run.lines {
        l.raw = l.raw.replace(&from, "mapNNNNN");
        l.msg = l.msg.replace(&from, "mapNNNNN");
    }
    run
}

fn run(rig: &Path, campaign: &str, level: u32) -> GameRun {
    let _l = DirLock::acquire(Duration::from_secs(900));
    let log = rig.join("keeperfx.log");
    let _ = std::fs::remove_file(&log);
    let out = std::fs::File::create(rig.join("stdout.txt")).unwrap();
    let err = out.try_clone().unwrap();
    let child = Command::new("./keeperfx")
        .args([
            "-nointro",
            "-nosound",
            "-campaign",
            campaign,
            "-level",
            &level.to_string(),
        ])
        .current_dir(rig)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env("SDL_VIDEODRIVER", "offscreen")
        .env("SDL_AUDIODRIVER", "dummy")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .expect("spawn keeperfx");
    let mut guard = ChildGuard(Some(child));
    let start = Instant::now();
    let mut exited_early = false;
    loop {
        if let Ok(Some(_)) = guard.0.as_mut().unwrap().try_wait() {
            exited_early = true;
            break;
        }
        if start.elapsed() >= Duration::from_secs(SECS) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !exited_early {
        guard.terminate();
    }
    drop(guard);
    let text = std::fs::read(&log)
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    let lines: Vec<LogLine> = text.lines().map(LogLine::parse).collect();
    let started = lines
        .iter()
        .any(|l| l.raw.starts_with(&format!("Started level {level} ")));
    let turn = lines.iter().find_map(|l| {
        let i = l.raw.find("\"turn\":")?;
        l.raw[i + 7..]
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    });
    let crashed = lines
        .iter()
        .any(|l| l.raw.contains("Failure signal") && !l.raw.contains("Termination"));
    let stdout = std::fs::read_to_string(rig.join("stdout.txt")).unwrap_or_default();
    normalized(
        GameRun {
            lines,
            started,
            turn,
            exited_early,
            crashed,
            stdout,
        },
        level,
    )
}

#[test]
fn new_campaign_with_two_copied_levels_starts_in_the_game() {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: game test skipped");
        return;
    }
    let Some(install) = kfx_formats::testutil::install_dir() else {
        return;
    };
    let rig = make_rig(&install);
    // the new campaign: template, land view copied from the original campaign, two levels
    let mut spec = NewPack::campaign("kfxcamp", "Editor made campaign");
    spec.land_from = Some(rig.join("campgns/keeporig.cfg"));
    let mut pack = create_pack(&rig, &spec).unwrap();
    let orig = kfx_formats::campaign::Campaign::load(&rig.join("campgns/keeporig.cfg")).unwrap();
    for (src, new) in [(1u32, 11u32), (2, 12)] {
        let mut s = LevelSource::new(&rig.join("campgns/keeporig"), src);
        s.info = orig.map_info(src);
        let mut req = AddLevel::new(s, LevelKind::Single);
        req.number = Some(new);
        pack.add_level(&req).unwrap();
    }
    pack.save().unwrap();
    let errors: Vec<_> = validate(&pack)
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    println!("cfg:\n{}", String::from_utf8_lossy(&pack.cfg.to_bytes()));

    // baseline: the original campaign's level, same game, same rig
    let base = run(&rig, "keeporig", 1);
    println!("[keeporig 1] {}", base.summary());
    assert!(
        base.started && !base.crashed,
        "the baseline run must start: {}",
        base.summary()
    );
    let first = run(&rig, "kfxcamp", 11);
    println!("[kfxcamp 11] {}", first.summary());
    for l in first.problems() {
        println!("[kfxcamp 11]   {}", l.raw);
    }
    let new = first.new_problems_vs(&base);
    for l in &new {
        println!("[kfxcamp 11] NEW vs original: {l}");
    }
    assert!(
        first.started,
        "level 11 of the new campaign did not start:\n{}",
        first
            .lines
            .iter()
            .map(|l| l.raw.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(!first.crashed && !first.exited_early);
    assert!(
        first.lines.iter().any(|l| l.raw.contains("kfxcamp")),
        "the game names the new campaign in its log"
    );
    assert!(new.is_empty(), "new log errors: {new:?}");

    let base2 = run(&rig, "keeporig", 2);
    let second = run(&rig, "kfxcamp", 12);
    println!(
        "[keeporig 2] {}\n[kfxcamp 12] {}",
        base2.summary(),
        second.summary()
    );
    let new2 = second.new_problems_vs(&base2);
    for l in &new2 {
        println!("[kfxcamp 12] NEW vs original: {l}");
    }
    assert!(second.started && !second.crashed, "level 12 did not start");
    assert!(new2.is_empty(), "new log errors: {new2:?}");
    cleanup(&rig);
}
