//! Locating and loading files of a KeeperFX install (case-insensitive, RNC-aware).
use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// Find `name` directly inside `dir`, matching case-insensitively.
pub fn find_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_file())
        .map(|e| e.path())
}

/// Resolve `rel` (slash separated) under `root`, each component case-insensitive.
pub fn resolve(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        let exact = cur.join(part);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let found = std::fs::read_dir(&cur).ok()?.flatten().find(|e| {
            e.file_name().to_string_lossy().eq_ignore_ascii_case(part)
                && (i + 1 == parts.len() || e.path().is_dir())
        })?;
        cur = found.path();
    }
    Some(cur)
}

/// A KeeperFX install directory (the one holding `data/`, `fxdata/`, `creatrs/`, ...).
#[derive(Clone, Debug)]
pub struct Install {
    pub root: PathBuf,
}

impl Install {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Install { root: root.into() }
    }

    /// Case-insensitive path of `rel` below the install root, if it exists.
    pub fn path(&self, rel: &str) -> Option<PathBuf> {
        resolve(&self.root, rel)
    }

    /// The `data/` directory.
    pub fn data_dir(&self) -> PathBuf {
        self.path("data").unwrap_or_else(|| self.root.join("data"))
    }

    /// The `fxdata/` directory.
    pub fn fxdata_dir(&self) -> PathBuf {
        self.path("fxdata")
            .unwrap_or_else(|| self.root.join("fxdata"))
    }

    /// Load `rel` through `kfx_formats::rnc::load_file` (unpacks RNC if packed).
    pub fn load(&self, rel: &str) -> Result<Vec<u8>> {
        let p = self.path(rel).ok_or_else(|| {
            Error::format(
                "install",
                format!("{rel}: not found under {}", self.root.display()),
            )
        })?;
        Ok(kfx_formats::rnc::load_file(&p)?)
    }
}

/// Load a file by path through the RNC-aware loader.
pub fn load_file(path: &Path) -> Result<Vec<u8>> {
    Ok(kfx_formats::rnc::load_file(path)?)
}
