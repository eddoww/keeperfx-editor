//! Private install copies for tests: the shipped install is only ever read.
// Shared by several test binaries; each uses a subset.
#![allow(dead_code)]
use std::path::{Path, PathBuf};

/// The scratch folder of this crate's tests.
pub fn cache() -> PathBuf {
    kfx_formats::testutil::scratch_dir("campaign-ui")
}

/// A fresh install folder under the scratch folder: every entry of the real install is a symlink,
/// except `campgns`, `levels` and `multiplayer`, which are real folders holding symlinks, so new
/// packs can be created in them without touching the real install. Returns `None` without an
/// install or where symbolic links cannot be made (Windows without the symlink privilege).
pub fn private_install(name: &str) -> Option<PathBuf> {
    let real = kfx_formats::testutil::install_dir()?;
    let dir = cache().join(format!("inst-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for e in std::fs::read_dir(&real).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let link = dir.join(&n);
        if ["campgns", "levels", "multiplayer"]
            .iter()
            .any(|g| g.eq_ignore_ascii_case(&n))
        {
            std::fs::create_dir_all(&link).unwrap();
            for c in std::fs::read_dir(e.path()).unwrap().flatten() {
                if let Err(err) =
                    kfx_formats::testutil::symlink(&c.path(), &link.join(c.file_name()))
                {
                    eprintln!("skipped: no symbolic links ({err})");
                    return None;
                }
            }
        } else if let Err(err) = kfx_formats::testutil::symlink(&e.path(), &link) {
            eprintln!("skipped: no symbolic links ({err})");
            return None;
        }
    }
    Some(dir)
}

pub fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// Leftover scratch / backup files of interrupted transactions below `dir`.
pub fn scratch_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let md = std::fs::symlink_metadata(&p).unwrap();
            if md.file_type().is_symlink() {
                continue;
            }
            if p.is_dir() {
                walk(&p, out);
            } else if kfx_formats::atomic::is_scratch_name(
                &p.file_name().unwrap().to_string_lossy(),
            ) {
                out.push(p);
            }
        }
    }
    walk(dir, &mut out);
    out
}
