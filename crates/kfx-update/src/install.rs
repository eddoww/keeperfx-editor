//! Where the editor is installed, whether it may update itself, and the swap of the files.
//!
//! # Layouts
//!
//! * **Portable folder** (Linux and Windows archives, the bare macOS executable): the executable
//!   with the README, licences, changelog and `docs/` beside it. Every top-level item of the new
//!   package replaces the item of the same name in the folder; anything else in the folder (the
//!   user's own files) is left alone.
//! * **macOS application bundle**: when the executable runs from `X.app/Contents/MacOS/`, the
//!   whole `X.app` is replaced by the bundle of the new package.
//! * **Windows installer**: a `setup-install` marker beside the executable (written by the
//!   Inno Setup installer). The updater runs the new `-setup.exe` silently instead of swapping
//!   files, so that the version recorded in Apps & features stays right (see
//!   [`setup_command`]).
//!
//! # The swap
//!
//! The new files are unpacked into a staging folder next to the installed ones (same file
//! system, so renames are atomic). Each installed item is renamed into the rollback folder
//! `.kfx-editor-previous`, then the new item is renamed into place. Renaming works for the
//! running executable on every platform, Windows included (a running `.exe` cannot be
//! overwritten or deleted, but it can be renamed). When any step fails, every step done so far is
//! undone in reverse order, leaving the installed version as it was.
//!
//! The rollback folder is kept until the new version has started cleanly once
//! ([`confirm_healthy`]); until then the Updates dialog offers to roll back ([`rollback`]).
//! A new version that does not start at all cannot do that: the manual way back is to move the
//! items of `.kfx-editor-previous` back into the folder (documented in the manual).

use crate::platform::{Os, Platform};
use crate::release::is_dev_version;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Marker beside the executable: installed by a package manager, never self-update.
pub const MANAGED_MARKER: &str = "managed-install";
/// Marker beside the executable: installed by the Windows installer.
pub const SETUP_MARKER: &str = "setup-install";
/// The previous version, kept until the new one has started cleanly.
pub const ROLLBACK_DIR: &str = ".kfx-editor-previous";
/// Unpacked update, and items rolled out of the way, before they are cleaned up.
pub const STAGING_DIR: &str = ".kfx-update-staging";
/// Version of the items in the rollback folder.
const ROLLBACK_VERSION_FILE: &str = ".version";
/// The items the update put in place (one per line), removed again by a rollback.
const ROLLBACK_ITEMS_FILE: &str = ".items";

/// How the running copy is installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutKind {
    /// The executable and its files in a folder.
    Portable,
    /// A macOS application bundle (the `.app` folder).
    MacBundle { app: PathBuf },
    /// Installed by the Windows installer.
    WindowsSetup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The running executable.
    pub exe: PathBuf,
    pub kind: LayoutKind,
    /// The folder whose items are replaced (the executable's folder, or the bundle's parent).
    pub root: PathBuf,
}

/// Why the editor will not update itself (a notice with a link is shown instead).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocker {
    /// A debug build or a version with a pre-release / build suffix.
    DevBuild,
    /// No release is built for this OS or processor.
    UnsupportedPlatform(String),
    /// Installed by a package manager (`managed-install` marker).
    Managed(PathBuf),
    /// The install folder cannot be written.
    NotWritable { path: PathBuf, reason: String },
    /// Running from a read-only disk image or a translocated / mounted copy.
    ReadOnlyImage(PathBuf),
    /// The location of the running executable is unknown.
    UnknownLocation(String),
}

impl std::fmt::Display for Blocker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Blocker::DevBuild => write!(f, "this is a development build; update it from source"),
            Blocker::UnsupportedPlatform(p) => {
                write!(f, "no release is built for this platform ({p})")
            }
            Blocker::Managed(m) => write!(
                f,
                "installed by a package manager ({}); update it with the package manager",
                m.display()
            ),
            Blocker::NotWritable { path, reason } => write!(
                f,
                "the install folder {} cannot be written ({reason}); download the new version by hand",
                path.display()
            ),
            Blocker::ReadOnlyImage(p) => write!(
                f,
                "running from a read-only location ({}); move the application to a folder you own first",
                p.display()
            ),
            Blocker::UnknownLocation(e) => {
                write!(f, "the location of the running program is unknown ({e})")
            }
        }
    }
}

/// File system questions behind a trait, so that the platform-specific detection is tested on
/// any platform.
pub trait Probe {
    fn exists(&self, p: &Path) -> bool;
    /// The folder can be written (by trying to create a file in it).
    fn writable(&self, dir: &Path) -> std::result::Result<(), String>;
    /// An environment variable (AppImage, ...).
    fn env(&self, name: &str) -> Option<OsString>;
}

/// The real file system.
pub struct FsProbe;

impl Probe for FsProbe {
    fn exists(&self, p: &Path) -> bool {
        p.exists()
    }
    fn writable(&self, dir: &Path) -> std::result::Result<(), String> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let t = dir.join(format!(".kfx-write-test-{}-{n}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&t)
        {
            Ok(_) => {
                let _ = std::fs::remove_file(&t);
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }
    fn env(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }
}

/// The layout of the copy whose executable is `exe` (canonical path).
pub fn detect(exe: &Path, platform: Platform, probe: &dyn Probe) -> Layout {
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    if platform.os == Os::MacOs {
        // X.app/Contents/MacOS/kfx-editor
        let macos = dir.file_name().is_some_and(|n| n == "MacOS");
        let contents = dir.parent();
        let app = contents.and_then(Path::parent);
        if let (true, Some(c), Some(app)) = (macos, contents, app) {
            let is_app = c.file_name().is_some_and(|n| n == "Contents")
                && app
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("app"));
            if is_app {
                return Layout {
                    exe: exe.to_path_buf(),
                    root: app.parent().map(Path::to_path_buf).unwrap_or_default(),
                    kind: LayoutKind::MacBundle {
                        app: app.to_path_buf(),
                    },
                };
            }
        }
    }
    let kind = if platform.os == Os::Windows && probe.exists(&dir.join(SETUP_MARKER)) {
        LayoutKind::WindowsSetup
    } else {
        LayoutKind::Portable
    };
    Layout {
        exe: exe.to_path_buf(),
        kind,
        root: dir,
    }
}

/// The package-manager marker of an install: beside the executable, or in
/// `<prefix>/share/kfx-editor/` for an executable in `<prefix>/bin` (where distribution
/// packages put it, so that `/usr/bin` holds only programs).
pub fn managed_marker(exe_dir: &Path, probe: &dyn Probe) -> Option<PathBuf> {
    let beside = exe_dir.join(MANAGED_MARKER);
    if probe.exists(&beside) {
        return Some(beside);
    }
    if exe_dir.file_name().is_some_and(|n| n == "bin") {
        let shared = exe_dir
            .parent()?
            .join("share/kfx-editor")
            .join(MANAGED_MARKER);
        if probe.exists(&shared) {
            return Some(shared);
        }
    }
    None
}

/// What the build is: decides [`Blocker::DevBuild`].
#[derive(Clone, Debug)]
pub struct BuildInfo {
    pub version: String,
    /// Built with `debug_assertions`.
    pub debug: bool,
}

/// Whether this copy may replace itself, or why not.
pub fn eligibility(
    layout: &Layout,
    build: &BuildInfo,
    platform: Platform,
    probe: &dyn Probe,
) -> std::result::Result<(), Blocker> {
    if build.debug || is_dev_version(&build.version) {
        return Err(Blocker::DevBuild);
    }
    if platform.release_name().is_none() {
        return Err(Blocker::UnsupportedPlatform(platform.describe()));
    }
    let exe_dir = layout.exe.parent().unwrap_or(&layout.root);
    if let Some(m) = managed_marker(exe_dir, probe) {
        return Err(Blocker::Managed(m));
    }
    // AppImage (read-only squashfs mount) and macOS App Translocation (a read-only random path)
    if probe.env("APPIMAGE").is_some() {
        return Err(Blocker::ReadOnlyImage(layout.exe.clone()));
    }
    let s = layout.exe.to_string_lossy();
    if s.contains("/AppTranslocation/") || (platform.os == Os::MacOs && s.starts_with("/Volumes/"))
    {
        return Err(Blocker::ReadOnlyImage(layout.exe.clone()));
    }
    // the Windows installer replaces the files itself (asking for elevation for an all-users
    // install), so only the other layouts need a writable folder
    let mut must_write = vec![];
    match &layout.kind {
        LayoutKind::WindowsSetup => {}
        LayoutKind::Portable => must_write.push(layout.root.clone()),
        LayoutKind::MacBundle { app } => {
            must_write.push(layout.root.clone());
            must_write.push(app.clone());
        }
    }
    for d in must_write {
        if let Err(reason) = probe.writable(&d) {
            return Err(
                if reason.contains("Read-only file system") || reason.contains("os error 30") {
                    Blocker::ReadOnlyImage(d)
                } else {
                    Blocker::NotWritable { path: d, reason }
                },
            );
        }
    }
    Ok(())
}

/// File operations of the swap behind a trait, so that tests can make any step fail.
pub trait FsOps {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
}

pub struct RealFs;

impl FsOps for RealFs {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }
}

/// An unpacked package ready to be swapped in: `items` are the names of the top-level items
/// below `dir` that replace the installed ones.
#[derive(Clone, Debug)]
pub struct Staged {
    pub dir: PathBuf,
    pub items: Vec<String>,
    pub version: String,
}

/// Unpack `archive` into the staging folder of `layout` and check that it is a package of the
/// editor: one top folder holding the executable (and, for a bundle, the `.app`).
pub fn stage(layout: &Layout, platform: Platform, archive: &Path, version: &str) -> Result<Staged> {
    let staging = layout.root.join(STAGING_DIR);
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| Error::io(&staging, e))?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| Error::io(&staging, e))?;
    let unpacked = staging.join("new");
    let limits = kfx_archive::Limits {
        max_entries: 5_000,
        max_file: 256 << 20,
        max_total: 1 << 30,
        max_ratio: 100,
    };
    if let Err(e) = kfx_archive::extract_all(archive, &unpacked, &limits) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e.into());
    }
    let fail = |msg: String| {
        let _ = std::fs::remove_dir_all(&staging);
        Error::Package(msg)
    };
    let tops: Vec<_> = std::fs::read_dir(&unpacked)
        .map_err(|e| Error::io(&unpacked, e))?
        .filter_map(|e| e.ok())
        .collect();
    let [top] = tops.as_slice() else {
        return Err(fail(format!(
            "{} top-level items, expected one folder",
            tops.len()
        )));
    };
    let top = top.path();
    if !top.is_dir() {
        return Err(fail("the top-level item is not a folder".into()));
    }
    let exe = top.join(platform.exe_name());
    if !exe.is_file() {
        return Err(fail(format!("no {} in the package", platform.exe_name())));
    }
    let items = match &layout.kind {
        LayoutKind::MacBundle { app } => {
            let new_app = std::fs::read_dir(&top)
                .map_err(|e| Error::io(&top, e))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "app") && p.is_dir())
                .ok_or_else(|| fail("no .app bundle in the package".into()))?;
            if !new_app
                .join("Contents/MacOS")
                .join(platform.exe_name())
                .is_file()
            {
                return Err(fail("the .app bundle has no executable".into()));
            }
            // the bundle takes the name of the installed one (the user may have renamed it)
            let name = app
                .file_name()
                .ok_or_else(|| fail("bundle without a name".into()))?;
            let target = top.join(name);
            if new_app != target {
                std::fs::rename(&new_app, &target).map_err(|e| Error::io(&target, e))?;
            }
            vec![name.to_string_lossy().into_owned()]
        }
        _ => {
            let mut v: Vec<String> = std::fs::read_dir(&top)
                .map_err(|e| Error::io(&top, e))?
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
    }
    Ok(Staged {
        dir: top,
        items,
        version: version.to_string(),
    })
}

/// One step done by [`swap`], undone in reverse order on failure.
enum Step {
    Moved { from: PathBuf, to: PathBuf },
}

/// Replace the installed items by the staged ones (see the module documentation). The
/// previous items go to the rollback folder, which records `current_version`. On failure every
/// step is undone and [`Error::Swap`] is returned.
pub fn swap(layout: &Layout, staged: &Staged, current_version: &str, fs: &dyn FsOps) -> Result<()> {
    let rollback = layout.root.join(ROLLBACK_DIR);
    if rollback.exists() {
        std::fs::remove_dir_all(&rollback).map_err(|e| Error::io(&rollback, e))?;
    }
    std::fs::create_dir_all(&rollback).map_err(|e| Error::io(&rollback, e))?;
    std::fs::write(rollback.join(ROLLBACK_VERSION_FILE), current_version)
        .map_err(|e| Error::io(&rollback, e))?;
    std::fs::write(rollback.join(ROLLBACK_ITEMS_FILE), staged.items.join("\n"))
        .map_err(|e| Error::io(&rollback, e))?;
    let mut done: Vec<Step> = vec![];
    let run = |from: PathBuf, to: PathBuf, done: &mut Vec<Step>| -> std::io::Result<()> {
        fs.rename(&from, &to)?;
        done.push(Step::Moved { from, to });
        Ok(())
    };
    let mut result = Ok(());
    for item in &staged.items {
        let installed = layout.root.join(item);
        if installed.symlink_metadata().is_ok() {
            if let Err(e) = run(installed.clone(), rollback.join(item), &mut done) {
                result = Err(format!("moving {} aside: {e}", installed.display()));
                break;
            }
        }
        if let Err(e) = run(staged.dir.join(item), installed.clone(), &mut done) {
            result = Err(format!("putting {} in place: {e}", installed.display()));
            break;
        }
    }
    if let Err(msg) = result {
        let mut undo_errors = vec![];
        for Step::Moved { from, to } in done.into_iter().rev() {
            if let Err(e) = fs.rename(&to, &from) {
                undo_errors.push(format!("{} -> {}: {e}", to.display(), from.display()));
            }
        }
        if undo_errors.is_empty() {
            let _ = std::fs::remove_dir_all(&rollback);
        }
        return Err(Error::Swap(if undo_errors.is_empty() {
            msg
        } else {
            format!(
                "{msg}; restoring failed too ({}): the previous files are in {}",
                undo_errors.join("; "),
                rollback.display()
            )
        }));
    }
    // the staging folder now only holds the empty package folder
    let _ = std::fs::remove_dir_all(layout.root.join(STAGING_DIR));
    Ok(())
}

/// The version kept in the rollback folder of `layout`, if there is one.
pub fn rollback_version(layout: &Layout) -> Option<String> {
    std::fs::read_to_string(layout.root.join(ROLLBACK_DIR).join(ROLLBACK_VERSION_FILE))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Put the previous version back: the items the update installed go to the staging folder
/// (removed at the next start) and the items of the rollback folder take their place. Same
/// all-or-nothing behaviour as [`swap`].
pub fn rollback(layout: &Layout, fs: &dyn FsOps) -> Result<String> {
    let rb = layout.root.join(ROLLBACK_DIR);
    let version = rollback_version(layout)
        .ok_or_else(|| Error::Swap("there is no previous version to go back to".into()))?;
    let out = layout.root.join(STAGING_DIR).join("rolled-back");
    if out.exists() {
        std::fs::remove_dir_all(&out).map_err(|e| Error::io(&out, e))?;
    }
    std::fs::create_dir_all(&out).map_err(|e| Error::io(&out, e))?;
    let mut items: Vec<String> = std::fs::read_dir(&rb)
        .map_err(|e| Error::io(&rb, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != ROLLBACK_VERSION_FILE && n != ROLLBACK_ITEMS_FILE)
        .collect();
    // items new in the update (no previous copy) are removed too; names are checked because
    // the list is read back from disk
    if let Ok(list) = std::fs::read_to_string(rb.join(ROLLBACK_ITEMS_FILE)) {
        items.extend(
            list.lines()
                .filter(|n| !n.is_empty() && *n != "." && *n != ".." && !n.contains(['/', '\\']))
                .map(str::to_string),
        );
    }
    items.sort();
    items.dedup();
    let mut done: Vec<(PathBuf, PathBuf)> = vec![];
    let mut failed = None;
    for item in &items {
        let installed = layout.root.join(item);
        if installed.symlink_metadata().is_ok() {
            let to = out.join(item);
            match fs.rename(&installed, &to) {
                Ok(()) => done.push((installed.clone(), to)),
                Err(e) => {
                    failed = Some(format!("{}: {e}", installed.display()));
                    break;
                }
            }
        }
        let previous = rb.join(item);
        if previous.symlink_metadata().is_err() {
            continue;
        }
        match fs.rename(&previous, &installed) {
            Ok(()) => done.push((previous, installed)),
            Err(e) => {
                failed = Some(format!("{item}: {e}"));
                break;
            }
        }
    }
    if let Some(msg) = failed {
        for (from, to) in done.into_iter().rev() {
            let _ = fs.rename(&to, &from);
        }
        return Err(Error::Swap(msg));
    }
    let _ = std::fs::remove_dir_all(&rb);
    Ok(version)
}

/// Tidy up at start: the staging folder of an interrupted or finished update, and stray
/// `*.old` files of older updaters. Errors are ignored (a file still locked by the exiting
/// previous process is removed next time).
pub fn cleanup_at_start(layout: &Layout) {
    let _ = std::fs::remove_dir_all(layout.root.join(STAGING_DIR));
    if let Ok(rd) = std::fs::read_dir(&layout.root) {
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "old") && p.is_file() {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

/// The record of an installed update that has not yet proven itself (`update-health.json` in
/// the configuration folder).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Health {
    /// The version that was installed.
    pub version: String,
    /// The version it replaced (in the rollback folder).
    pub previous: String,
    /// The install folder (the rollback folder is below it).
    pub root: PathBuf,
    /// Starts of the new version that did not reach [`confirm_healthy`].
    pub starts: u32,
}

/// What [`on_start`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartState {
    /// No update awaiting confirmation.
    Normal,
    /// This is the first start(s) after an update: confirm once the window runs. `failed`
    /// counts earlier starts that never got that far.
    Unconfirmed { health: Health, failed: u32 },
}

fn health_path(state_dir: &Path) -> PathBuf {
    state_dir.join("update-health.json")
}

/// Record an installed update (before the restart).
pub fn record_installed(state_dir: &Path, h: &Health) -> Result<()> {
    let b =
        serde_json::to_vec_pretty(h).map_err(|e| Error::parse("update health", e.to_string()))?;
    crate::state::write_atomic(&health_path(state_dir), &b)
}

/// Call once at every start of the editor window: counts the start of an unconfirmed update.
/// A record for another version (the user went back by hand) is dropped.
pub fn on_start(state_dir: &Path, version: &str) -> StartState {
    let p = health_path(state_dir);
    let Some(mut h) = std::fs::read(&p)
        .ok()
        .and_then(|b| serde_json::from_slice::<Health>(&b).ok())
    else {
        return StartState::Normal;
    };
    if h.version != version {
        let _ = std::fs::remove_file(&p);
        return StartState::Normal;
    }
    let failed = h.starts;
    h.starts += 1;
    let _ = record_installed(state_dir, &h);
    StartState::Unconfirmed { health: h, failed }
}

/// The new version started cleanly: forget the record and remove the rollback folder.
pub fn confirm_healthy(state_dir: &Path, h: &Health) {
    let _ = std::fs::remove_dir_all(h.root.join(ROLLBACK_DIR));
    let _ = std::fs::remove_file(health_path(state_dir));
}

/// Forget an update record (after a rollback).
pub fn forget_health(state_dir: &Path) {
    let _ = std::fs::remove_file(health_path(state_dir));
}

/// The silent command line of the Windows installer for an update: no wizard, no message
/// boxes, no reboot, and start the editor again afterwards (on `map` if given). The installer
/// script (`packaging/windows/installer.iss`) reads `/RELAUNCH` and `/OPEN=`.
pub fn setup_command(setup: &Path, map: Option<&Path>) -> (PathBuf, Vec<OsString>) {
    let mut args: Vec<OsString> = [
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART",
        "/RELAUNCH",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    if let Some(m) = map {
        let mut a = OsString::from("/OPEN=");
        a.push(m.as_os_str());
        args.push(a);
    }
    (setup.to_path_buf(), args)
}

/// Start `exe` with `args` as a new, independent process (the caller then exits).
pub fn relaunch(exe: &Path, args: &[OsString]) -> Result<()> {
    std::process::Command::new(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::io(exe, e))
}
