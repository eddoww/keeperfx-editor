//! File safety: crash-recovery snapshots, external-change fingerprints, the backups list
//! (restore, size-limited pruning), level display names, opening a folder in the file manager.
use crate::document::{level_files, timestamp};
use kfx_formats::level_bin;
use kfx_formats::levelinfo::Lof;
use kfx_prefab::i18n::trf;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

// ------------------------------------------------------------------------------ snapshots

/// What a crash-recovery snapshot belongs to (`snapshot.json` in its folder).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SnapshotInfo {
    /// The map's own folder and level number.
    pub folder: PathBuf,
    pub level: u32,
    /// UTC `YYYYMMDD-HHMMSS`.
    pub time: String,
    /// The map files in the snapshot are newer than the saved map.
    pub map_dirty: bool,
    /// Scripts (logical names) with unsaved edits stored in the snapshot.
    pub scripts: Vec<String>,
    /// The map had never been saved (a new map).
    pub new_map: bool,
}

/// A snapshot folder with its description.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub dir: PathBuf,
    pub info: SnapshotInfo,
}

/// Folder of the snapshot of a map: `<autosave>/mapNNNNN-<hash of the folder>`.
pub fn snapshot_dir(root: &Path, folder: &Path, level: u32) -> PathBuf {
    let canon = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let mut h: u64 = 0xcbf29ce484222325;
    for b in canon.to_string_lossy().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    root.join(format!("map{level:05}-{:016x}", h))
}

/// Replace the snapshot of a map. `write_map` writes the map files into the folder it is given
/// (as level `level`); `scripts` are (logical name, bytes) of the scripts with unsaved edits.
/// The new snapshot is complete in a temporary folder (files flushed to disk) before it
/// replaces the previous one, which is kept until the replacement succeeded.
pub fn write_snapshot(
    root: &Path,
    info: &SnapshotInfo,
    write_map: impl FnOnce(&Path) -> Result<(), String>,
    scripts: &[(String, Vec<u8>)],
) -> Result<PathBuf, String> {
    let dir = snapshot_dir(root, &info.folder, info.level);
    let tmp = dir.with_extension("tmp");
    let old = dir.with_extension("old");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let fill = || -> Result<(), String> {
        if info.map_dirty {
            write_map(&tmp)?;
        }
        for (name, bytes) in scripts {
            kfx_formats::atomic::atomic_write(&tmp.join(name), bytes)
                .map_err(|e| format!("{name}: {e}"))?;
        }
        let json = serde_json::to_string_pretty(info).map_err(|e| e.to_string())?;
        // snapshot.json last: a folder without it is not listed as a snapshot
        kfx_formats::atomic::atomic_write(&tmp.join("snapshot.json"), json.as_bytes())
            .map_err(|e| e.to_string())
    };
    if let Err(e) = fill() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&old);
    let had_old = dir.exists() && std::fs::rename(&dir, &old).is_ok();
    if let Err(e) = std::fs::rename(&tmp, &dir) {
        if had_old {
            let _ = std::fs::rename(&old, &dir);
        }
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("{}: {e}", dir.display()));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(dir)
}

/// Remove the snapshot of a map (after a save, or when its changes were discarded).
pub fn remove_snapshot(root: &Path, folder: &Path, level: u32) {
    let _ = std::fs::remove_dir_all(snapshot_dir(root, folder, level));
}

/// Every complete snapshot, newest first.
pub fn list_snapshots(root: &Path) -> Vec<Snapshot> {
    let mut v: Vec<Snapshot> = std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.extension().is_none())
                .filter_map(|dir| {
                    let t = std::fs::read_to_string(dir.join("snapshot.json")).ok()?;
                    let info: SnapshotInfo = serde_json::from_str(&t).ok()?;
                    Some(Snapshot { dir, info })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.info.time.cmp(&a.info.time));
    v
}

// ---------------------------------------------------------------------- external changes

/// Modification time and size of every file of a level (`mapNNNNN.*`), sorted by name; the
/// scripts can be left out (the script workspace tracks those itself).
pub fn fingerprint(
    folder: &Path,
    level: u32,
    with_scripts: bool,
) -> Vec<(String, Option<SystemTime>, u64)> {
    level_files(folder, level)
        .into_iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().to_ascii_lowercase();
            if !with_scripts && (name.ends_with(".txt") || name.ends_with(".lua")) {
                return None;
            }
            let m = std::fs::metadata(&p).ok()?;
            Some((name, m.modified().ok(), m.len()))
        })
        .collect()
}

// -------------------------------------------------------------------------------- backups

/// One backup folder (`<backups>/<timestamp>-mapNNNNN[-n]/`, see `document::backup_level`).
#[derive(Clone, Debug, PartialEq)]
pub struct Backup {
    pub dir: PathBuf,
    /// The map folder the files came from (`SOURCE.txt`).
    pub source: Option<PathBuf>,
    pub level: u32,
    /// UTC `YYYYMMDD-HHMMSS`.
    pub time: String,
    pub bytes: u64,
    pub files: Vec<String>,
}

fn dir_size(d: &Path) -> u64 {
    std::fs::read_dir(d)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// Every backup, newest first.
pub fn list_backups(root: &Path) -> Vec<Backup> {
    let mut v: Vec<Backup> = std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .filter_map(|dir| {
                    let name = dir.file_name()?.to_string_lossy().into_owned();
                    let i = name.find("-map")?;
                    let time = name[..i].to_string();
                    let level: u32 = name[i + 4..].split('-').next()?.parse().ok()?;
                    let source = std::fs::read_to_string(dir.join("SOURCE.txt"))
                        .ok()
                        .map(|s| PathBuf::from(s.trim()));
                    let mut files: Vec<String> = std::fs::read_dir(&dir)
                        .map(|rd| {
                            rd.flatten()
                                .map(|e| e.file_name().to_string_lossy().into_owned())
                                .filter(|n| n != "SOURCE.txt")
                                .collect()
                        })
                        .unwrap_or_default();
                    files.sort();
                    Some(Backup {
                        bytes: dir_size(&dir),
                        dir,
                        source,
                        level,
                        time,
                        files,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.time.cmp(&a.time).then(b.dir.cmp(&a.dir)));
    v
}

/// Remove the oldest backups until the folder holds at most `limit_mb` MiB (0 = no limit). The
/// newest backup is always kept. Returns the removed folders.
pub fn prune_backups(root: &Path, limit_mb: u32) -> Vec<PathBuf> {
    if limit_mb == 0 {
        return vec![];
    }
    let limit = limit_mb as u64 * 1024 * 1024;
    let list = list_backups(root);
    let mut total: u64 = list.iter().map(|b| b.bytes).sum();
    let mut removed = vec![];
    for b in list.iter().skip(1).rev() {
        if total <= limit {
            break;
        }
        if std::fs::remove_dir_all(&b.dir).is_ok() {
            total -= b.bytes;
            removed.push(b.dir.clone());
        }
    }
    removed
}

/// Put the files of a backup back into `dest` as level `level`: the level's other current files
/// are removed. All or nothing (`kfx_formats::atomic::SaveTransaction`); back the current files
/// up before calling this.
pub fn restore_backup(b: &Backup, dest: &Path) -> Result<usize, String> {
    let mut tx = kfx_formats::atomic::SaveTransaction::new(dest);
    let mut n = 0;
    for name in &b.files {
        let from = b.dir.join(name);
        if !from.is_file() {
            continue;
        }
        tx.write(
            name.clone(),
            std::fs::read(&from).map_err(|e| format!("{}: {e}", from.display()))?,
        );
        n += 1;
    }
    for f in level_files(dest, b.level) {
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if !b.files.iter().any(|x| x.eq_ignore_ascii_case(&name)) {
            tx.remove(name);
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(n)
}

// ---------------------------------------------------------------------- session backups

/// Copies of files kept before the editor first overwrites or deletes them in a session (level
/// string tables, the campaign's string tables and `.cfg`, `.lof` / `.lif`, scripts, config
/// overrides). The copies go to `<backups>/<timestamp>-files/` (one folder per session, with
/// `SOURCE.txt` listing where each came from). A copy that cannot be made is an error, and
/// callers then do not write. Clones share the state.
#[derive(Clone, Debug, Default)]
pub struct SessionBackups {
    inner: Arc<Mutex<BackupState>>,
}

#[derive(Debug, Default)]
struct BackupState {
    /// `None`: backups are off (tests and previews that never write user files).
    mode: Option<Option<PathBuf>>,
    done: HashSet<PathBuf>,
    folder: Option<PathBuf>,
}

impl SessionBackups {
    /// Backups into `root` (the editor's backups folder); `None` when the platform has no
    /// config folder: every backup then fails, and with it the write.
    pub fn new(root: Option<PathBuf>) -> Self {
        SessionBackups {
            inner: Arc::new(Mutex::new(BackupState {
                mode: Some(root),
                ..Default::default()
            })),
        }
    }

    /// No backups (tests).
    pub fn disabled() -> Self {
        Self::default()
    }

    fn key(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    /// Treat `path` as backed up (e.g. it is part of a level backup).
    pub fn mark(&self, path: &Path) {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        st.done.insert(Self::key(path));
    }

    /// Back `path` up unless it was this session (or it does not exist, or backups are off).
    /// Returns the copy when one was made.
    pub fn ensure(&self, path: &Path) -> Result<Option<PathBuf>, String> {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let root = match &st.mode {
            None => return Ok(None),
            Some(None) => {
                if !path.is_file() {
                    return Ok(None);
                }
                return Err(trf(
                    "kfx_editor.path_no_folder_for_backups_on_this_system_so_it",
                    "{path}: no folder for backups on this system, so it is not overwritten",
                    &[("path", &path.display())],
                ));
            }
            Some(Some(r)) => r.clone(),
        };
        let key = Self::key(path);
        if st.done.contains(&key) || !path.is_file() {
            return Ok(None);
        }
        let folder = match &st.folder {
            Some(f) => f.clone(),
            None => {
                let mut f = root.join(format!("{}-files", timestamp()));
                let mut n = 1;
                while f.exists() {
                    n += 1;
                    f = root.join(format!("{}-files-{n}", timestamp()));
                }
                f
            }
        };
        let name = path.display().to_string().replace(['/', '\\', ':'], "_");
        let copy = folder.join(&name);
        let bytes = std::fs::read(path).map_err(|e| {
            trf(
                "kfx_editor.backup_of_path_e",
                "backup of {path}: {e}",
                &[("path", &path.display()), ("e", &e)],
            )
        })?;
        kfx_formats::atomic::atomic_write(&copy, &bytes).map_err(|e| {
            trf(
                "kfx_editor.backup_of_path_to_copy_e",
                "backup of {path} to {copy}: {e}",
                &[
                    ("path", &path.display()),
                    ("copy", &copy.display()),
                    ("e", &e),
                ],
            )
        })?;
        let list = folder.join("SOURCE.txt");
        let mut text = std::fs::read_to_string(&list).unwrap_or_default();
        text.push_str(&format!("{name}\t{}\n", path.display()));
        kfx_formats::atomic::atomic_write(&list, text.as_bytes()).map_err(|e| {
            trf(
                "kfx_editor.backup_list_list_e",
                "backup list {list}: {e}",
                &[("list", &list.display()), ("e", &e)],
            )
        })?;
        st.folder = Some(folder);
        st.done.insert(key);
        Ok(Some(copy))
    }
}

/// `YYYYMMDD-HHMMSS` -> `YYYY-MM-DD HH:MM:SS UTC`.
pub fn pretty_time(t: &str) -> String {
    if t.len() >= 15 && t.as_bytes()[8] == b'-' {
        format!(
            "{}-{}-{} {}:{}:{} UTC",
            &t[0..4],
            &t[4..6],
            &t[6..8],
            &t[9..11],
            &t[11..13],
            &t[13..15]
        )
    } else {
        t.to_string()
    }
}

/// Now as `YYYYMMDD-HHMMSS` (UTC).
pub fn now() -> String {
    timestamp()
}

// ---------------------------------------------------------------------------- level names

/// The string resolver of a campaign / map pack `.cfg` (English, no mods) for `level` in
/// `levels_dir`: what the game answers for a `NAME_ID` / `NAME_TEXT_ID`
/// (`kfx_formats::strings`).
pub fn pack_strings(
    install: &Path,
    cfg: &Path,
    levels_dir: Option<&Path>,
    level: Option<u32>,
) -> Option<kfx_formats::strings::StringResolver> {
    let c = kfx_formats::campaign::Campaign::load(cfg).ok()?;
    let src = kfx_formats::strings::StringSources::resolve(
        install,
        &[],
        &c.strings(),
        levels_dir,
        level,
        kfx_formats::strings::Lang::English,
    )
    .ok()?;
    kfx_formats::strings::StringResolver::load(&src).ok()
}

/// A campaign's display name: `NAME`, else its `NAME_TEXT_ID` through the string tables.
pub fn campaign_name(install: &Path, cfg: &Path) -> Option<String> {
    let c = kfx_formats::campaign::Campaign::load(cfg).ok()?;
    let common = c.common();
    if let Some(n) = common.name.filter(|n| !n.trim().is_empty()) {
        return Some(n);
    }
    let id = common
        .name_text_id
        .and_then(|i| i.trim().parse::<i64>().ok())?;
    pack_strings(install, cfg, None, None)
        .map(|r| r.get(id))
        .filter(|s| !s.trim().is_empty())
}

/// A level's display name: its campaign / map pack entry (`NAME_TEXT`, or `NAME_ID` resolved
/// like the game's `get_string`: the level's own strings, then the campaign's, then the GUI
/// strings), else the `.lof` `NAME_TEXT` / `NAME_ID`, else the `.lif` name.
pub fn level_name(install: &Path, folder: &Path, level: u32) -> Option<String> {
    let cfg = crate::document::find_pack_cfg(install, folder, level);
    if let Some(cfg) = &cfg {
        if let Ok(c) = kfx_formats::campaign::Campaign::load(cfg) {
            if let Some(info) = c.map_info(level) {
                if let Some(n) = info.name_text.filter(|n| !n.trim().is_empty()) {
                    return Some(n);
                }
                if let Some(id) = info.name_id.and_then(|i| i.trim().parse::<i64>().ok()) {
                    if let Some(s) = pack_strings(install, cfg, Some(folder), Some(level))
                        .map(|r| r.get(id))
                        .filter(|s| !s.trim().is_empty())
                    {
                        return Some(s);
                    }
                }
            }
        }
        if let Ok(r) = kfx_formats::campaign::resolve(install, cfg) {
            if let Some(n) = r
                .levels
                .iter()
                .find(|l| l.number == level)
                .and_then(|l| l.name.clone())
                .filter(|n| !n.trim().is_empty())
            {
                return Some(n);
            }
        }
    }
    if let Some(info) = level_bin::find_file(folder, level, "lof")
        .and_then(|p| Lof::load(&p).ok())
        .map(|l| l.info())
    {
        if let Some(n) = info.name_text.filter(|n| !n.trim().is_empty()) {
            return Some(n);
        }
        if let (Some(id), Some(cfg)) = (
            info.name_id.and_then(|i| i.trim().parse::<i64>().ok()),
            &cfg,
        ) {
            if let Some(s) = pack_strings(install, cfg, Some(folder), Some(level))
                .map(|r| r.get(id))
                .filter(|s| !s.trim().is_empty())
            {
                return Some(s);
            }
        }
    }
    let lif = level_bin::find_file(folder, level, "lif")?;
    let t = kfx_formats::cfg::latin1_decode(&std::fs::read(lif).ok()?);
    t.lines().find_map(|l| {
        let (n, name) = l.split_once(',')?;
        (n.trim().parse::<u32>().ok()? == level).then(|| name.trim().to_string())
    })
}

// ---------------------------------------------------------------------------- open folder

/// Open a file with the system's default program (not waited for).
pub fn open_path(p: &Path) -> Result<(), String> {
    if !p.exists() {
        return Err(trf(
            "kfx_editor.p_does_not_exist",
            "{p} does not exist",
            &[("p", &p.display())],
        ));
    }
    let r = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(p)
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(p).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(p).spawn()
    };
    r.map(|_| ()).map_err(|e| {
        trf(
            "kfx_editor.could_not_open_p_e",
            "could not open {p}: {e}",
            &[("p", &p.display()), ("e", &e)],
        )
    })
}

/// Show a folder in the system's file manager (not waited for).
pub fn open_folder(dir: &Path) -> Result<(), String> {
    let prog = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(prog)
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut c| {
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        })
        .map_err(|e| format!("{prog}: {e}"))
}

/// The map to open for a dropped / given path: the path itself when it is a map file, else the
/// lowest-numbered `mapNNNNN.slb` of a folder. Returns the map and how many maps the folder has.
pub fn map_for_path(p: &Path) -> Option<(PathBuf, usize)> {
    if p.is_file() {
        return crate::document::map_number(p).map(|_| (p.to_path_buf(), 1));
    }
    let mut maps: Vec<(u32, PathBuf)> = std::fs::read_dir(p)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|f| f.extension().is_some_and(|x| x.eq_ignore_ascii_case("slb")))
        .filter_map(|f| crate::document::map_number(&f).map(|n| (n, f)))
        .collect();
    maps.sort();
    let n = maps.len();
    maps.into_iter().next().map(|(_, f)| (f, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/files_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn snapshots_roundtrip() {
        let root = tmp("snap");
        let info = SnapshotInfo {
            folder: root.join("lv"),
            level: 4,
            time: now(),
            map_dirty: true,
            scripts: vec!["map00004.txt".into()],
            new_map: false,
        };
        let dir = write_snapshot(
            &root,
            &info,
            |d| std::fs::write(d.join("map00004.slb"), [1, 2]).map_err(|e| e.to_string()),
            &[("map00004.txt".into(), b"REM x".to_vec())],
        )
        .unwrap();
        assert!(dir.join("map00004.slb").is_file() && dir.join("map00004.txt").is_file());
        let l = list_snapshots(&root);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].info, info);
        remove_snapshot(&root, &info.folder, 4);
        assert!(list_snapshots(&root).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn backups_list_restore_prune() {
        let root = tmp("bk");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("map00002.slb"), vec![7u8; 600_000]).unwrap();
        std::fs::write(src.join("map00002.txt"), "REM a").unwrap();
        let bdir = root.join("backups");
        let b1 = crate::document::backup_level(&src, 2, &bdir)
            .unwrap()
            .unwrap();
        std::fs::write(src.join("map00002.txt"), "REM b").unwrap();
        let b2 = crate::document::backup_level(&src, 2, &bdir)
            .unwrap()
            .unwrap();
        let l = list_backups(&bdir);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].level, 2);
        assert_eq!(l[0].source.as_deref(), Some(src.as_path()));
        assert!(l[0].files.contains(&"map00002.txt".to_string()));
        // restore the older one
        let old = l.iter().find(|b| b.dir == b1).unwrap();
        std::fs::write(src.join("map00002.lua"), "-- extra").unwrap();
        assert_eq!(restore_backup(old, &src).unwrap(), 2);
        assert_eq!(
            std::fs::read_to_string(src.join("map00002.txt")).unwrap(),
            "REM a"
        );
        assert!(!src.join("map00002.lua").exists());
        // 1 MiB limit: the older one goes, the newest stays
        let removed = prune_backups(&bdir, 1);
        assert_eq!(removed.len(), 1);
        assert!(b2.exists() && !b1.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A backup that cannot be made stops the write; a file is backed up once per session.
    #[test]
    fn session_backups_once_and_failures_are_errors() {
        let d = tmp("sessbk");
        let f = d.join("map00001.eng.dat");
        std::fs::write(&f, b"one").unwrap();
        let b = SessionBackups::new(Some(d.join("backups")));
        let copy = b.ensure(&f).unwrap().expect("a copy");
        assert_eq!(std::fs::read(&copy).unwrap(), b"one");
        std::fs::write(&f, b"two").unwrap();
        assert_eq!(b.ensure(&f).unwrap(), None, "once per session");
        assert!(b.ensure(&d.join("missing")).unwrap().is_none());
        // the backups folder cannot be created (a file is in the way)
        std::fs::write(d.join("blocker"), b"x").unwrap();
        let bad = SessionBackups::new(Some(d.join("blocker").join("backups")));
        assert!(bad.ensure(&f).is_err());
        assert!(
            SessionBackups::new(None).ensure(&f).is_err(),
            "no backups folder at all"
        );
        assert_eq!(SessionBackups::disabled().ensure(&f).unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn fingerprints_change() {
        let d = tmp("fp");
        std::fs::write(d.join("map00001.slb"), [1]).unwrap();
        std::fs::write(d.join("map00001.txt"), [1]).unwrap();
        let a = fingerprint(&d, 1, false);
        assert_eq!(a.len(), 1);
        std::fs::write(d.join("map00001.slb"), [1, 2]).unwrap();
        assert_ne!(a, fingerprint(&d, 1, false));
        assert_eq!(map_for_path(&d).unwrap().0, d.join("map00001.slb"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn shipped_level_names() {
        let Some(root) = kfx_formats::testutil::install_dir() else {
            return;
        };
        let camp = kfx_formats::testutil::resolve_ci(&root, "campgns/keeporig").unwrap();
        let n = level_name(&root, &camp, 1);
        println!("keeporig map00001: {n:?}");
        assert!(n.is_some());
    }
}
