//! The only door to the file system: the host implements [`ConfigFs`] (and may add backups,
//! undo, sandboxing ...). Nothing in this crate touches files except through it.
//!
//! [`StdFs`] is a plain implementation over `std::fs`, [`MemFs`] an in-memory one for tests and
//! previews.
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// File access the editors need.
pub trait ConfigFs {
    /// Bytes of a file, `Ok(None)` when it does not exist.
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>>;
    /// Create or replace a file (the host creates parent folders as needed).
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Delete a file (no error when it does not exist).
    fn remove(&self, path: &Path) -> io::Result<()>;
    /// File names (not paths) in a folder; empty when the folder does not exist.
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<String>>;
}

/// Case-insensitive lookup of `dir/name` through `fs` (install file names have mixed case).
pub fn find_ci(fs: &dyn ConfigFs, dir: &Path, name: &str) -> Option<PathBuf> {
    let names = fs.list_dir(dir).ok()?;
    if names.iter().any(|n| n == name) {
        return Some(dir.join(name));
    }
    names
        .into_iter()
        .find(|n| n.eq_ignore_ascii_case(name))
        .map(|n| dir.join(n))
}

/// Read a file through `fs`, unpacking RNC when packed (`kfx_formats::rnc`).
pub fn read_unpacked(fs: &dyn ConfigFs, path: &Path) -> io::Result<Option<Vec<u8>>> {
    let Some(b) = fs.read(path)? else {
        return Ok(None);
    };
    if kfx_formats::rnc::is_rnc(&b) {
        return kfx_formats::rnc::unpack(&b)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()));
    }
    Ok(Some(b))
}

/// `std::fs` implementation.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdFs;

impl ConfigFs for StdFs {
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    /// Crash-safe: the bytes replace the file in one step (`kfx_formats::atomic`).
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        kfx_formats::atomic::atomic_write(path, bytes)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<String>> {
        match std::fs::read_dir(dir) {
            Ok(rd) => {
                let mut v: Vec<String> = rd
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                v.sort();
                Ok(v)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e),
        }
    }
}

/// In-memory file system, optionally falling through to a read-only lower one (so tests can
/// read the real install while every write stays in memory).
#[derive(Clone, Default)]
pub struct MemFs {
    files: Arc<Mutex<BTreeMap<PathBuf, Option<Vec<u8>>>>>,
    lower: Option<Arc<dyn ConfigFs + Send + Sync>>,
}

impl MemFs {
    pub fn new() -> Self {
        Self::default()
    }
    /// Reads fall through to `lower` for paths not written here.
    pub fn over(lower: impl ConfigFs + Send + Sync + 'static) -> Self {
        MemFs {
            files: Default::default(),
            lower: Some(Arc::new(lower)),
        }
    }
    /// Files written or removed in memory (`None` = removed).
    pub fn changes(&self) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        self.files.lock().unwrap().clone()
    }
}

impl ConfigFs for MemFs {
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        if let Some(v) = self.files.lock().unwrap().get(path) {
            return Ok(v.clone());
        }
        match &self.lower {
            Some(l) => l.read(path),
            None => Ok(None),
        }
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.files
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), Some(bytes.to_vec()));
        Ok(())
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        self.files.lock().unwrap().insert(path.to_path_buf(), None);
        Ok(())
    }
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<String>> {
        let mut names: Vec<String> = match &self.lower {
            Some(l) => l.list_dir(dir)?,
            None => vec![],
        };
        for (p, v) in self.files.lock().unwrap().iter() {
            if p.parent() != Some(dir) {
                continue;
            }
            let n = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            match v {
                Some(_) if !names.contains(&n) => names.push(n),
                None => names.retain(|x| x != &n),
                _ => {}
            }
        }
        names.sort();
        Ok(names)
    }
}
