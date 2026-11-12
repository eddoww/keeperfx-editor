//! `kfx-editor update`: the self-update on the command line.

use crate::updates;
use kfx_update::client::CheckOutcome;
use kfx_update::install::{relaunch, FsProbe, RealFs};
use kfx_update::updater::Installed;
use kfx_update::Channel;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

pub const USAGE: &str = "\
updates (ask GitHub for the releases of eddoww/keeperfx-editor; see docs/manual/updates.md):
  kfx-editor update --check [--channel stable|pre-release]
                                         print this version and the newest one; exit 0 when up
                                         to date, 10 when an update is available, 1 on errors
  kfx-editor update [--channel ..] [--restart [MAP]]
                                         download, verify and install the newest version;
                                         --restart starts it afterwards (on MAP)
  kfx-editor --version                   print the version";

/// Exit code of `update --check` when a newer version exists.
pub const EXIT_UPDATE_AVAILABLE: i32 = 10;

struct Args {
    check: bool,
    channel: Option<Channel>,
    restart: Option<Option<PathBuf>>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        check: false,
        channel: None,
        restart: None,
    };
    let mut it = args.iter().peekable();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--check" => a.check = true,
            "--channel" => {
                a.channel = Some(match it.next().map(String::as_str) {
                    Some("stable") => Channel::Stable,
                    Some("pre-release") | Some("prerelease") => Channel::PreRelease,
                    o => {
                        return Err(format!(
                            "--channel: expected stable or pre-release, got {o:?}"
                        ))
                    }
                })
            }
            "--restart" => {
                let map = it.next_if(|n| !n.starts_with('-')).map(PathBuf::from);
                a.restart = Some(map);
            }
            o => return Err(format!("update: unknown argument {o:?}")),
        }
    }
    Ok(a)
}

pub fn run(args: &[String]) -> i32 {
    match run_inner(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("kfx-editor: {e}");
            1
        }
    }
}

fn run_inner(args: &[String]) -> Result<i32, String> {
    let a = parse(args)?;
    let settings = crate::settings::Settings::load().updates;
    let mut s = settings.clone();
    if let Some(c) = a.channel {
        s.channel = c;
    }
    let u = updates::updater().ok_or("no configuration folder for the update state")?;
    let req = updates::request(&s, true);
    println!(
        "current: {} (channel {})",
        crate::VERSION,
        s.channel.as_str()
    );
    let outcome = u
        .client
        .check(&u.files, &req, kfx_update::state::now_secs())
        .map_err(|e| e.to_string())?;
    let up = match outcome {
        CheckOutcome::Available(up) => up,
        CheckOutcome::UpToDate { latest } => {
            println!(
                "latest:  {} (up to date)",
                latest
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "none".into())
            );
            return Ok(0);
        }
        // a manual check is always due; kept for completeness
        CheckOutcome::NotDue => return Ok(0),
    };
    println!("latest:  {} ({})", up.version, updates::release_page(&up));
    if a.check {
        return Ok(EXIT_UPDATE_AVAILABLE);
    }
    let layout = updates::layout();
    if let Some(b) = updates::blocker(&layout) {
        return Err(format!(
            "not updating: {b}\ndownload it from {}",
            updates::release_page(&up)
        ));
    }
    let layout = layout?;
    let mut last = 0u64;
    let mut progress = |done: u64, total: u64| {
        let pct = done * 100 / total.max(1);
        if pct != last || done == total {
            last = pct;
            eprint!(
                "\rdownloading {} {pct:3}% ({done} of {total} bytes)",
                up.version
            );
            let _ = std::io::stderr().flush();
        }
    };
    let file = u
        .download(&up, &layout, &mut progress, &AtomicBool::new(false))
        .map_err(|e| e.to_string())?;
    eprintln!("\nverified {}", file.display());
    let map = a.restart.clone().flatten();
    let map = map.map(|m| std::fs::canonicalize(&m).unwrap_or(m));
    match u
        .install(&up, &layout, &file, map.as_deref(), &FsProbe, &RealFs)
        .map_err(|e| e.to_string())?
    {
        Installed::Swapped { exe } => {
            println!("installed {} in {}", up.version, layout.root.display());
            // from kfx-editor-cli.exe, start the window program beside it
            let gui = exe.with_file_name(kfx_update::platform::Platform::current().exe_name());
            let exe = if gui.is_file() { gui } else { exe };
            if a.restart.is_some() {
                let args: Vec<std::ffi::OsString> =
                    map.into_iter().map(|m| m.into_os_string()).collect();
                relaunch(&exe, &args).map_err(|e| e.to_string())?;
                println!("started {}", exe.display());
            }
        }
        Installed::RunSetup { program, mut args } => {
            if a.restart.is_none() {
                args.retain(|x| x != "/RELAUNCH");
            }
            relaunch(&program, &args).map_err(|e| e.to_string())?;
            println!(
                "the installer of {} is running in the background",
                up.version
            );
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn arguments() {
        let x = parse(&a(&["--check", "--channel", "stable"])).unwrap();
        assert!(x.check && x.channel == Some(Channel::Stable) && x.restart.is_none());
        let x = parse(&a(&["--restart", "m.slb"])).unwrap();
        assert_eq!(x.restart, Some(Some(PathBuf::from("m.slb"))));
        let x = parse(&a(&["--restart", "--channel", "pre-release"])).unwrap();
        assert_eq!(x.restart, Some(None));
        assert!(parse(&a(&["--channel", "nightly"])).is_err());
        assert!(parse(&a(&["--frobnicate"])).is_err());
    }
}
