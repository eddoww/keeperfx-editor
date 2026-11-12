//! Crash-safe file writes.
//!
//! Every file the editors write goes through here, so a failure part-way (disk full, a
//! permission error, a crash, the game or another tool holding a file) never leaves a truncated
//! file or a map that is half old and half new.
//!
//! * [`atomic_write`]: one file. The bytes go to a temporary file in the same folder, which is
//!   flushed to disk and then renamed over the target (a rename inside one folder replaces the
//!   target in one step on every supported file system).
//! * [`SaveTransaction`]: several files of one folder (a level). Nothing on disk changes until
//!   every new file is fully written as a temporary file; then the targets are replaced in the
//!   order given (so the caller can put the file that switches the game to a format last), and only
//!   then are the files to delete removed. Each replaced or removed file is first kept as a hidden
//!   backup (a hard link, or a copy where the file system has no links); when any step fails the
//!   backups are put back and the new files removed, so the folder holds exactly the old files
//!   again. After success the backups are deleted.
//!
//! Temporary and backup names start with `.` and contain `.kfx-new-` / `.kfx-old-`; a crash in
//! the middle can leave such files behind, never a damaged target.
//!
//! # Letter case
//! A written file ends up with exactly the name asked for, on every kind of file system:
//! * case-sensitive (Linux, some macOS volumes): `B` and `b` are two files; after writing `b` the
//!   stale `B` is removed with the other removals (it is backed up like them).
//! * case-insensitive (macOS by default, Windows): `b` and `B` are the same directory entry, so
//!   replacing `b` replaces `B`'s content, and the entry may keep the old spelling `B` (macOS
//!   does). It is then renamed to `b` (in two steps, through a scratch name, since a rename that
//!   only changes case is ignored by some file systems). Whether two names are the same entry is
//!   decided by file identity ([`TxFs::same_file`]) and the folder listing, never by comparing
//!   names, so the file just written is never taken for a stale variant.
//!
//! A failed transaction puts the old spelling back too.
//!
//! The file operations go through [`TxFs`] so tests can inject a failure at any step
//! ([`FaultFs`]).
use crate::{Error, Result};
use std::cell::Cell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The file operations a [`SaveTransaction`] performs.
pub trait TxFs {
    fn create_dir_all(&self, dir: &Path) -> std::io::Result<()>;
    /// File names (not paths) in `dir`.
    fn list(&self, dir: &Path) -> std::io::Result<Vec<String>>;
    fn exists(&self, path: &Path) -> bool;
    /// Whether an existing file is marked read-only (it is then not replaced: a rename would
    /// ignore the mark on some systems).
    fn is_readonly(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly())
    }
    /// Create `path` (which must not exist), write `bytes` and flush them to the disk.
    fn write_new(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()>;
    /// Make `to` (which must not exist) hold the current content of `from`.
    fn backup(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    /// Rename `from` to `to`, replacing `to`.
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn remove(&self, path: &Path) -> std::io::Result<()>;
    /// Flush the folder entry changes to the disk where the platform supports it.
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()>;
    /// Whether two existing paths name the same file (the same directory entry or hard links
    /// of one file). `false` when either does not exist or the platform cannot tell.
    fn same_file(&self, a: &Path, b: &Path) -> bool {
        same_file(a, b)
    }
}

/// [`TxFs::same_file`] of [`RealFs`]: device and inode on Unix; elsewhere (Windows) the final
/// paths the system resolves both to, which on a case-insensitive volume differ at most in case.
pub fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(x), Ok(y)) => x
                .to_string_lossy()
                .eq_ignore_ascii_case(&y.to_string_lossy()),
            _ => false,
        }
    }
}

/// [`TxFs`] over `std::fs`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RealFs;

impl TxFs for RealFs {
    fn create_dir_all(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)
    }
    fn list(&self, dir: &Path) -> std::io::Result<Vec<String>> {
        let mut v = vec![];
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            v.push(e.file_name().to_string_lossy().into_owned());
        }
        v.sort();
        Ok(v)
    }
    fn exists(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }
    fn write_new(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()
    }
    fn backup(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        if std::fs::hard_link(from, to).is_ok() {
            return Ok(());
        }
        let bytes = std::fs::read(from)?;
        self.write_new(to, &bytes)
    }
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }
    fn remove(&self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            match std::fs::File::open(dir)?.sync_all() {
                // some file systems (network shares, FAT on macOS) cannot flush a folder: their
                // renames are as durable as they get, which is not an error of the save
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
                    ) =>
                {
                    Ok(())
                }
                r => r,
            }
        }
        #[cfg(not(unix))]
        {
            // Windows cannot open a folder as a file; its renames are journaled by NTFS.
            let _ = dir;
            Ok(())
        }
    }
}

/// [`RealFs`] that fails its `fail_at`-th step (0-based; steps are `write_new`, `backup`,
/// `rename`, `remove` and `sync_dir`). A failing `write_new` first writes half of the bytes, like
/// a full disk. Counts every step, so a test can run a transaction once without a failure to
/// learn how many steps it has ([`FaultFs::steps`]).
#[derive(Debug, Default)]
pub struct FaultFs {
    pub fail_at: Option<usize>,
    count: Cell<usize>,
    failed: Cell<bool>,
}

impl FaultFs {
    pub fn new(fail_at: Option<usize>) -> Self {
        FaultFs {
            fail_at,
            count: Cell::new(0),
            failed: Cell::new(false),
        }
    }
    /// Steps performed so far.
    pub fn steps(&self) -> usize {
        self.count.get()
    }
    /// Whether the injected failure happened.
    pub fn failed(&self) -> bool {
        self.failed.get()
    }
    fn step(&self) -> std::io::Result<()> {
        let n = self.count.get();
        self.count.set(n + 1);
        if Some(n) == self.fail_at && !self.failed.get() {
            self.failed.set(true);
            return Err(std::io::Error::other(format!(
                "injected failure at step {n}"
            )));
        }
        Ok(())
    }
}

impl TxFs for FaultFs {
    fn create_dir_all(&self, dir: &Path) -> std::io::Result<()> {
        RealFs.create_dir_all(dir)
    }
    fn list(&self, dir: &Path) -> std::io::Result<Vec<String>> {
        RealFs.list(dir)
    }
    fn exists(&self, path: &Path) -> bool {
        RealFs.exists(path)
    }
    fn write_new(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if let Err(e) = self.step() {
            let _ = RealFs.write_new(path, &bytes[..bytes.len() / 2]);
            return Err(e);
        }
        RealFs.write_new(path, bytes)
    }
    fn backup(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.step()?;
        RealFs.backup(from, to)
    }
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.step()?;
        RealFs.rename(from, to)
    }
    fn remove(&self, path: &Path) -> std::io::Result<()> {
        self.step()?;
        RealFs.remove(path)
    }
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
        self.step()?;
        RealFs.sync_dir(dir)
    }
    fn same_file(&self, a: &Path, b: &Path) -> bool {
        RealFs.same_file(a, b)
    }
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn scratch_name(name: &str, kind: &str) -> String {
    format!(
        ".{name}.kfx-{kind}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// Whether `name` is a temporary or backup file left by an interrupted transaction.
pub fn is_scratch_name(name: &str) -> bool {
    name.starts_with('.') && (name.contains(".kfx-new-") || name.contains(".kfx-old-"))
}

#[derive(Clone, Debug)]
enum Op {
    Write { name: String, bytes: Vec<u8> },
    Remove { name: String },
}

/// What [`SaveTransaction::commit`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Committed {
    /// Files written (final paths), in order.
    pub written: Vec<PathBuf>,
    /// Files removed.
    pub removed: Vec<PathBuf>,
    /// Backups or temporary files that could not be deleted after the save succeeded.
    pub leftovers: Vec<PathBuf>,
}

/// A set of file writes and removals in one folder that happen completely or not at all (see
/// the module docs).
#[derive(Clone, Debug)]
pub struct SaveTransaction {
    dir: PathBuf,
    ops: Vec<Op>,
}

enum Undo {
    /// Put the backup back over the target.
    Restore { backup: PathBuf, target: PathBuf },
    /// Remove a file that did not exist before.
    Delete(PathBuf),
}

impl SaveTransaction {
    pub fn new(dir: &Path) -> Self {
        SaveTransaction {
            dir: dir.to_path_buf(),
            ops: vec![],
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write file `name` (a plain file name in the folder). Writes are applied in the order they
    /// are added; a second write of the same name replaces the first. Files whose names differ from
    /// `name` only in case are removed after all writes.
    pub fn write(&mut self, name: impl Into<String>, bytes: Vec<u8>) -> &mut Self {
        let name = name.into();
        self.ops.retain(
            |o| !matches!(o, Op::Write { name: n, .. } | Op::Remove { name: n } if *n == name),
        );
        self.ops.push(Op::Write { name, bytes });
        self
    }

    /// Remove every file named `name` ignoring case, after all writes (removals keep their
    /// order). A later write of the same name cancels the removal.
    pub fn remove(&mut self, name: impl Into<String>) -> &mut Self {
        let name = name.into();
        if !self
            .ops
            .iter()
            .any(|o| matches!(o, Op::Write { name: n, .. } if n.eq_ignore_ascii_case(&name)))
        {
            self.ops
                .retain(|o| !matches!(o, Op::Remove { name: n } if n.eq_ignore_ascii_case(&name)));
            self.ops.push(Op::Remove { name });
        }
        self
    }

    /// Names written, in order, with their bytes.
    pub fn writes(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.ops.iter().filter_map(|o| match o {
            Op::Write { name, bytes } => Some((name.as_str(), bytes.as_slice())),
            _ => None,
        })
    }

    /// Names removed (as given).
    pub fn removals(&self) -> impl Iterator<Item = &str> {
        self.ops.iter().filter_map(|o| match o {
            Op::Remove { name } => Some(name.as_str()),
            _ => None,
        })
    }

    /// Move the write of `name` to the end (it is applied last).
    pub fn write_last(&mut self, name: &str) -> &mut Self {
        if let Some(i) = self
            .ops
            .iter()
            .position(|o| matches!(o, Op::Write { name: n, .. } if n == name))
        {
            let op = self.ops.remove(i);
            let at = self
                .ops
                .iter()
                .rposition(|o| matches!(o, Op::Write { .. }))
                .map_or(0, |j| j + 1);
            self.ops.insert(at, op);
        }
        self
    }

    pub fn commit(self) -> Result<Committed> {
        self.commit_with(&RealFs)
    }

    /// [`SaveTransaction::commit`] through `fs`.
    pub fn commit_with(self, fs: &dyn TxFs) -> Result<Committed> {
        let dir = self.dir.clone();
        fs.create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let writes: Vec<(&String, &Vec<u8>)> = self
            .ops
            .iter()
            .filter_map(|o| match o {
                Op::Write { name, bytes } => Some((name, bytes)),
                _ => None,
            })
            .collect();
        // 0. a target marked read-only is a permission error, as with an in-place write
        for (name, _) in &writes {
            let target = dir.join(name.as_str());
            if fs.exists(&target) && fs.is_readonly(&target) {
                return Err(Error::io(
                    &target,
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "the file is read-only (nothing was changed)",
                    ),
                ));
            }
        }
        // 1. every new file as a temporary file; nothing visible changes yet
        let mut temps: Vec<PathBuf> = vec![];
        for (name, bytes) in &writes {
            if name.is_empty() || name.contains(['/', '\\']) {
                cleanup(fs, &temps);
                return Err(Error::io(
                    dir.join(name.as_str()),
                    std::io::Error::other("not a plain file name"),
                ));
            }
            let tmp = dir.join(scratch_name(name, "new"));
            if let Err(e) = fs.write_new(&tmp, bytes) {
                temps.push(tmp.clone());
                cleanup(fs, &temps);
                return Err(Error::io(dir.join(name.as_str()), e));
            }
            temps.push(tmp);
        }
        // 2. replace the targets in order, keeping each old file as a backup
        let mut undo: Vec<Undo> = vec![];
        let mut out = Committed::default();
        let fail = |undo: Vec<Undo>, temps: &[PathBuf], path: &Path, e: std::io::Error| -> Error {
            let problems = rollback(fs, undo);
            cleanup(fs, temps);
            let _ = fs.sync_dir(&dir);
            if problems.is_empty() {
                Error::io(
                    path,
                    std::io::Error::new(e.kind(), format!("{e} (nothing was changed)")),
                )
            } else {
                Error::io(
                    path,
                    std::io::Error::new(
                        e.kind(),
                        format!(
                            "{e}; restoring the previous files also failed: {}",
                            problems.join("; ")
                        ),
                    ),
                )
            }
        };
        let listing = fs.list(&dir).unwrap_or_default();
        for ((name, _), tmp) in writes.iter().zip(temps.iter()) {
            let target = dir.join(name.as_str());
            if fs.exists(&target) {
                let bak = dir.join(scratch_name(name, "old"));
                if let Err(e) = fs.backup(&target, &bak) {
                    let _ = fs.remove(&bak);
                    return Err(fail(undo, &temps, &target, e));
                }
                // on a case-insensitive file system the entry may be spelled differently: a
                // rollback restores that spelling
                let old_name =
                    entry_name(fs, &dir, name, &listing).unwrap_or_else(|| name.to_string());
                undo.push(Undo::Restore {
                    backup: bak,
                    target: dir.join(old_name),
                });
            } else {
                undo.push(Undo::Delete(target.clone()));
            }
            if let Err(e) = fs.rename(tmp, &target) {
                return Err(fail(undo, &temps, &target, e));
            }
            // the replaced entry kept its old spelling (case-insensitive file system): rename it
            if let Err(e) = fix_case(fs, &dir, name) {
                return Err(fail(undo, &temps, &target, e));
            }
            out.written.push(target);
        }
        // 3. removals: the files asked for, then other-case variants of the written names (only
        // when the folder lists them as separate entries, i.e. a case-sensitive file system)
        let listing = fs.list(&dir).unwrap_or_default();
        let mut gone: Vec<PathBuf> = vec![];
        for o in &self.ops {
            if let Op::Remove { name } = o {
                for n in listing
                    .iter()
                    .filter(|n| n.eq_ignore_ascii_case(name) && !is_scratch_name(n))
                {
                    gone.push(dir.join(n));
                }
            }
        }
        for (name, _) in &writes {
            if !listing.iter().any(|n| n == *name) {
                continue;
            }
            // a variant that is the written file itself (one entry seen under two spellings)
            // is never removed: identity decides, not the name
            let written = dir.join(name.as_str());
            for n in listing
                .iter()
                .filter(|n| n.eq_ignore_ascii_case(name) && n != name && !is_scratch_name(n))
            {
                if !fs.same_file(&dir.join(n), &written) {
                    gone.push(dir.join(n));
                }
            }
        }
        gone.dedup();
        for p in gone {
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let bak = dir.join(scratch_name(&name, "old"));
            if let Err(e) = fs.rename(&p, &bak) {
                return Err(fail(undo, &temps, &p, e));
            }
            undo.push(Undo::Restore {
                backup: bak,
                target: p.clone(),
            });
            out.removed.push(p);
        }
        if let Err(e) = fs.sync_dir(&dir) {
            return Err(fail(undo, &temps, &dir, e));
        }
        // 4. done: drop the backups
        for u in undo {
            if let Undo::Restore { backup, .. } = u {
                // one retry: a virus scanner or indexer may hold the file for a moment
                if fs.remove(&backup).is_err() && fs.remove(&backup).is_err() && fs.exists(&backup)
                {
                    out.leftovers.push(backup);
                }
            }
        }
        Ok(out)
    }
}

/// The listed entry that `name` opens in `dir`: `name` itself when listed, else its only other-case
/// spelling when that is the same file (a case-insensitive file system), else `None` (absent).
fn entry_name(fs: &dyn TxFs, dir: &Path, name: &str, listing: &[String]) -> Option<String> {
    if listing.iter().any(|n| n == name) {
        return Some(name.to_string());
    }
    let variants: Vec<&String> = listing
        .iter()
        .filter(|n| n.eq_ignore_ascii_case(name) && !is_scratch_name(n))
        .collect();
    match variants.as_slice() {
        [v] if fs.same_file(&dir.join(v.as_str()), &dir.join(name)) => Some(v.to_string()),
        _ => None,
    }
}

/// Make the entry that `name` opens in `dir` be spelled `name`, when a case-insensitive file
/// system kept another spelling. Two renames through a scratch name; when the second fails the
/// first is undone. Nothing to do when `name` is listed as is or does not exist.
fn fix_case(fs: &dyn TxFs, dir: &Path, name: &str) -> std::io::Result<()> {
    let listing = fs.list(dir)?;
    let Some(current) = entry_name(fs, dir, name, &listing) else {
        return Ok(());
    };
    if current == name {
        return Ok(());
    }
    let from = dir.join(&current);
    let via = dir.join(scratch_name(name, "old"));
    fs.rename(&from, &via)?;
    if let Err(e) = fs.rename(&via, &dir.join(name)) {
        if fs.rename(&via, &from).is_err() {
            return Err(std::io::Error::new(
                e.kind(),
                format!("{e}; the file is left as {}", via.display()),
            ));
        }
        return Err(e);
    }
    Ok(())
}

fn cleanup(fs: &dyn TxFs, temps: &[PathBuf]) {
    for t in temps {
        if fs.exists(t) {
            let _ = fs.remove(t);
        }
    }
}

/// Undo in reverse order; returns what could not be undone.
fn rollback(fs: &dyn TxFs, undo: Vec<Undo>) -> Vec<String> {
    let mut problems = vec![];
    for u in undo.into_iter().rev() {
        match u {
            Undo::Restore { backup, target } => {
                if let Err(e) = fs.rename(&backup, &target) {
                    problems.push(format!(
                        "{} could not be restored from {}: {e}",
                        target.display(),
                        backup.display()
                    ));
                } else {
                    if fs.exists(&backup) {
                        // renaming a hard link onto the same file does nothing (POSIX rename)
                        let _ = fs.remove(&backup);
                    }
                    // put the old spelling back (case-insensitive file system)
                    if let (Some(dir), Some(name)) = (target.parent(), target.file_name()) {
                        if let Err(e) = fix_case(fs, dir, &name.to_string_lossy()) {
                            problems.push(format!(
                                "{} could not be renamed back: {e}",
                                target.display()
                            ));
                        }
                    }
                }
            }
            Undo::Delete(p) => {
                if fs.exists(&p) {
                    if let Err(e) = fs.remove(&p) {
                        problems.push(format!("{}: {e}", p.display()));
                    }
                }
            }
        }
    }
    problems
}

/// Write one file crash-safely (see the module docs). Creates the parent folder.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let (dir, name) = split(path)?;
    let mut tx = SaveTransaction::new(&dir);
    tx.write(name, bytes.to_vec());
    match tx.commit() {
        Ok(_) => Ok(()),
        Err(Error::Io { source, .. }) => Err(source),
        Err(e) => Err(std::io::Error::other(e.to_string())),
    }
}

/// Remove one file crash-safely: like a [`SaveTransaction`] removal (no error when absent).
pub fn atomic_remove(path: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    std::fs::remove_file(path)
}

fn split(path: &Path) -> std::io::Result<(PathBuf, String)> {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| std::io::Error::other(format!("{}: not a file path", path.display())))?;
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    Ok((dir, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = crate::testutil::scratch_dir(&format!(
            "kfx-formats/kfx-atomic-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Whether the file system of `d` ignores case (probe files in `d` itself).
    fn case_insensitive(d: &Path) -> bool {
        let p = d.join(".case-probe");
        std::fs::write(&p, b"x").unwrap();
        let r = d.join(".CASE-PROBE").exists();
        std::fs::remove_file(&p).unwrap();
        r
    }

    fn snapshot(d: &Path) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn single_write_replaces() {
        let d = tmp("single");
        let p = d.join("a.txt");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(snapshot(&d), vec![("a.txt".into(), b"two".to_vec())]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Every step of a transaction failing in turn leaves exactly the old or the new files and no
    /// scratch files.
    #[test]
    fn every_failure_point_is_all_or_nothing() {
        let d = tmp("faults");
        let setup = || {
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("a"), b"old a").unwrap();
            std::fs::write(d.join("B"), b"old B upper").unwrap();
            std::fs::write(d.join("gone"), b"old gone").unwrap();
        };
        let tx = || {
            let mut t = SaveTransaction::new(&d);
            t.write("a", b"new a".to_vec())
                .write("b", b"new b".to_vec())
                .write("c", b"new c".to_vec())
                .remove("GONE");
            t
        };
        setup();
        let old = snapshot(&d);
        let insensitive = case_insensitive(&d);
        // the old file system state: `B` is its own file, or the entry `b` opens
        assert_eq!(
            old.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(),
            ["B", "a", "gone"]
        );
        let f = FaultFs::new(None);
        tx().commit_with(&f).unwrap();
        let new = snapshot(&d);
        // on both kinds of file system the result is spelled as written: case-sensitive, the
        // stale `B` is removed; case-insensitive, the replaced entry `B` is renamed to `b`
        assert_eq!(
            new,
            vec![
                ("a".into(), b"new a".to_vec()),
                ("b".into(), b"new b".to_vec()),
                ("c".into(), b"new c".to_vec())
            ],
            "case-insensitive: {insensitive}"
        );
        let steps = f.steps();
        assert!(steps >= 8, "{steps}");
        for k in 0..steps {
            setup();
            let f = FaultFs::new(Some(k));
            let r = tx().commit_with(&f);
            let now = snapshot(&d);
            assert!(f.failed(), "step {k} not reached");
            // a failed step before the end restores the old files; a failure while deleting the
            // backups after a complete save is retried and does not undo the save
            match r {
                Ok(_) => assert_eq!(now, new, "step {k}"),
                Err(_) => assert_eq!(now, old, "step {k}: not the old state"),
            }
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Writing `name` where only another spelling exists keeps one file, with the new bytes and
    /// the new spelling, on the real file system of the temp folder (whatever its kind).
    #[test]
    fn writing_over_another_spelling_renames_it() {
        let d = tmp("case");
        std::fs::write(d.join("MAP00001.SLB"), b"old").unwrap();
        let mut t = SaveTransaction::new(&d);
        t.write("map00001.slb", b"new".to_vec());
        let c = t.commit().unwrap();
        assert!(c.leftovers.is_empty());
        assert_eq!(snapshot(&d), vec![("map00001.slb".into(), b"new".to_vec())]);
        if case_insensitive(&d) {
            assert!(
                c.removed.is_empty(),
                "the written file was taken for a stale variant: {:?}",
                c.removed
            );
        } else {
            assert_eq!(c.removed, vec![d.join("MAP00001.SLB")]);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An in-memory case-insensitive, case-preserving folder (macOS / Windows), so the case
    /// handling is exercised on every platform. `keep_spelling`: a rename onto an existing entry
    /// keeps that entry's spelling (APFS) instead of taking the new one.
    struct CiFs {
        /// lowercase name -> (spelling, file id, bytes)
        files: std::cell::RefCell<std::collections::BTreeMap<String, (String, u64, Vec<u8>)>>,
        next_id: Cell<u64>,
        keep_spelling: bool,
        fault: FaultFs,
    }

    impl CiFs {
        fn new(keep_spelling: bool, fail_at: Option<usize>, files: &[(&str, &[u8])]) -> CiFs {
            let fs = CiFs {
                files: Default::default(),
                next_id: Cell::new(1),
                keep_spelling,
                fault: FaultFs::new(fail_at),
            };
            for (n, b) in files {
                let id = fs.id();
                fs.files
                    .borrow_mut()
                    .insert(n.to_lowercase(), (n.to_string(), id, b.to_vec()));
            }
            fs
        }
        fn id(&self) -> u64 {
            let i = self.next_id.get();
            self.next_id.set(i + 1);
            i
        }
        fn key(p: &Path) -> String {
            p.file_name().unwrap().to_string_lossy().to_lowercase()
        }
        fn state(&self) -> Vec<(String, Vec<u8>)> {
            self.files
                .borrow()
                .values()
                .map(|(n, _, b)| (n.clone(), b.clone()))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect()
        }
        fn missing(p: &Path) -> std::io::Error {
            std::io::Error::new(std::io::ErrorKind::NotFound, p.display().to_string())
        }
    }

    impl TxFs for CiFs {
        fn create_dir_all(&self, _: &Path) -> std::io::Result<()> {
            Ok(())
        }
        fn list(&self, _: &Path) -> std::io::Result<Vec<String>> {
            Ok(self.files.borrow().values().map(|e| e.0.clone()).collect())
        }
        fn exists(&self, p: &Path) -> bool {
            self.files.borrow().contains_key(&Self::key(p))
        }
        fn is_readonly(&self, _: &Path) -> bool {
            false
        }
        fn write_new(&self, p: &Path, bytes: &[u8]) -> std::io::Result<()> {
            let fail = self.fault.step();
            if self.exists(p) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    p.display().to_string(),
                ));
            }
            let id = self.id();
            let n = if fail.is_err() {
                bytes.len() / 2
            } else {
                bytes.len()
            };
            self.files.borrow_mut().insert(
                Self::key(p),
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    id,
                    bytes[..n].to_vec(),
                ),
            );
            fail
        }
        fn backup(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            self.fault.step()?;
            let e = self
                .files
                .borrow()
                .get(&Self::key(from))
                .cloned()
                .ok_or_else(|| Self::missing(from))?;
            // a hard link: same id and bytes under another name
            self.files.borrow_mut().insert(
                Self::key(to),
                (
                    to.file_name().unwrap().to_string_lossy().into_owned(),
                    e.1,
                    e.2,
                ),
            );
            Ok(())
        }
        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            self.fault.step()?;
            let mut f = self.files.borrow_mut();
            let (fk, tk) = (Self::key(from), Self::key(to));
            let mut e = f.remove(&fk).ok_or_else(|| Self::missing(from))?;
            let to_name = to.file_name().unwrap().to_string_lossy().into_owned();
            e.0 = match f.get(&tk) {
                // onto another existing entry: APFS keeps that entry's spelling
                Some(old) if self.keep_spelling => old.0.clone(),
                _ => to_name,
            };
            f.insert(tk, e);
            Ok(())
        }
        fn remove(&self, p: &Path) -> std::io::Result<()> {
            self.fault.step()?;
            self.files
                .borrow_mut()
                .remove(&Self::key(p))
                .map(|_| ())
                .ok_or_else(|| Self::missing(p))
        }
        fn sync_dir(&self, _: &Path) -> std::io::Result<()> {
            self.fault.step()
        }
        fn same_file(&self, a: &Path, b: &Path) -> bool {
            let f = self.files.borrow();
            matches!((f.get(&Self::key(a)), f.get(&Self::key(b))), (Some(x), Some(y)) if x.1 == y.1)
        }
    }

    /// The transaction of `every_failure_point_is_all_or_nothing` on simulated case-insensitive
    /// folders, with both rename behaviours, failing at every step in turn.
    #[test]
    fn case_insensitive_folders_all_or_nothing() {
        let old: &[(&str, &[u8])] = &[
            ("a", b"old a"),
            ("B", b"old B upper"),
            ("gone", b"old gone"),
        ];
        let tx = || {
            let mut t = SaveTransaction::new(Path::new("ci"));
            t.write("a", b"new a".to_vec())
                .write("b", b"new b".to_vec())
                .write("c", b"new c".to_vec())
                .remove("GONE");
            t
        };
        let old_state: Vec<(String, Vec<u8>)> = old
            .iter()
            .map(|(n, b)| (n.to_string(), b.to_vec()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let new_state: Vec<(String, Vec<u8>)> = vec![
            ("a".into(), b"new a".to_vec()),
            ("b".into(), b"new b".to_vec()),
            ("c".into(), b"new c".to_vec()),
        ];
        for keep_spelling in [true, false] {
            let fs = CiFs::new(keep_spelling, None, old);
            let c = tx().commit_with(&fs).unwrap();
            assert_eq!(fs.state(), new_state, "keep_spelling {keep_spelling}");
            assert!(c.leftovers.is_empty());
            let steps = fs.fault.steps();
            for k in 0..steps {
                let fs = CiFs::new(keep_spelling, Some(k), old);
                let r = tx().commit_with(&fs);
                assert!(fs.fault.failed(), "step {k} not reached");
                match r {
                    Ok(_) => assert_eq!(
                        fs.state(),
                        new_state,
                        "keep_spelling {keep_spelling} step {k}"
                    ),
                    Err(e) => assert_eq!(
                        fs.state(),
                        old_state,
                        "keep_spelling {keep_spelling} step {k}: {e}"
                    ),
                }
            }
        }
    }
}
