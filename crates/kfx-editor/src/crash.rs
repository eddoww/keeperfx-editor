//! Crash logs. The editor window is often started from a desktop shortcut, where nobody sees
//! standard error (and the Windows GUI executable has none), so a panic would vanish without a
//! trace. The panic hook writes a crash log into the `crashes/` folder of the configuration
//! folder; the next start of the window tells the user where it is (`app/updates.rs`).

use std::path::{Path, PathBuf};

/// The crash logs folder below the configuration folder.
pub fn crash_dir(config: &Path) -> PathBuf {
    config.join("crashes")
}

/// The file naming the crash log not yet shown to the user.
fn pending_file(config: &Path) -> PathBuf {
    crash_dir(config).join("unseen.txt")
}

/// Install the panic hook (the window only; command-line runs print the panic as usual). The
/// previous hook still runs, so the message also reaches standard error.
pub fn install_hook(config: Option<PathBuf>) {
    let Some(config) = config else {
        return;
    };
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed").to_string();
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        // only a panic of the main (window) thread ends the editor
        let _ = write_log(
            &config,
            &name,
            &payload,
            &location,
            &backtrace,
            name == "main",
        );
        previous(info);
    }));
}

/// Write a crash log; returns its path. `fatal` marks it to be shown at the next start.
pub fn write_log(
    config: &Path,
    thread: &str,
    message: &str,
    location: &str,
    backtrace: &str,
    fatal: bool,
) -> std::io::Result<PathBuf> {
    let dir = crash_dir(config);
    std::fs::create_dir_all(&dir)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("crash-{now}-{}.log", std::process::id()));
    let text = format!(
        "KeeperFX Map Editor {} crashed (Unix time {now})\nsystem: {} {}\nthread '{thread}' \
         panicked at {location}:\n{message}\n\nbacktrace:\n{backtrace}\n",
        crate::VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    std::fs::write(&path, text)?;
    if fatal {
        std::fs::write(pending_file(config), path.to_string_lossy().as_bytes())?;
    }
    keep_newest(&dir, 20);
    Ok(path)
}

/// The crash log of the last run, if it ended in a crash that was not shown yet.
pub fn unseen(config: &Path) -> Option<PathBuf> {
    let p = std::fs::read_to_string(pending_file(config)).ok()?;
    let p = PathBuf::from(p.trim());
    p.is_file().then_some(p)
}

/// The user has seen the notice.
pub fn mark_seen(config: &Path) {
    let _ = std::fs::remove_file(pending_file(config));
}

/// Remove all but the newest `n` crash logs.
fn keep_newest(dir: &Path, n: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .collect();
    logs.sort();
    if logs.len() > n {
        for p in &logs[..logs.len() - n] {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_and_the_unseen_marker() {
        let d =
            kfx_formats::testutil::scratch_dir(&format!("kfx-editor/crash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(unseen(&d), None);
        let p = write_log(&d, "worker", "boom", "x.rs:1:2", "bt", false).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("boom"));
        assert_eq!(
            unseen(&d),
            None,
            "a background thread is not a crash of the editor"
        );
        let p = write_log(&d, "main", "bang", "y.rs:3:4", "bt", true).unwrap();
        assert_eq!(unseen(&d), Some(p));
        mark_seen(&d);
        assert_eq!(unseen(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
