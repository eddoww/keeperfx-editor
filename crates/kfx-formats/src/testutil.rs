//! Helpers for tests and examples: the KeeperFX install used as test data, scratch folders,
//! case-insensitive path resolution and symbolic links.
use std::path::{Path, PathBuf};

/// The KeeperFX install used as test data: the folder named by the `KFX_INSTALL` environment
/// variable, or `None` when it is unset or not a folder (tests then skip).
pub fn install_dir() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("KFX_INSTALL")?);
    p.is_dir().then_some(p)
}

/// A scratch folder for tests and examples, `<root>/<name>` (created). The root is
/// `KFX_TEST_TMP` when set, else `target/tmp` of the build that made the running executable
/// (test and example executables live in `target/<profile>/deps` or `.../examples`), else the
/// system temporary folder.
pub fn scratch_dir(name: &str) -> PathBuf {
    let root = std::env::var_os("KFX_TEST_TMP")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(target_tmp)
        .unwrap_or_else(|| std::env::temp_dir().join("kfx-tests"));
    let dir = root.join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// `target/tmp` derived from the running executable's location, when it is a Cargo build.
fn target_tmp() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let profile = match dir.file_name()?.to_str()? {
        "deps" | "examples" => dir.parent()?,
        _ => dir,
    };
    let target = profile.parent()?;
    // `target/<profile>` or `target/<triple>/<profile>`
    let target = if target.join("CACHEDIR.TAG").is_file() {
        target
    } else {
        target
            .parent()
            .filter(|t| t.join("CACHEDIR.TAG").is_file())?
    };
    Some(target.join("tmp"))
}

/// Resolve `rel` under `base`, matching each path component case-insensitively.
pub fn resolve_ci(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = base.to_path_buf();
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        let exact = cur.join(part);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let found = std::fs::read_dir(&cur)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?;
        cur = found.path();
    }
    Some(cur)
}

/// A symbolic link `link` -> `target` (test installs link to the read-only real one). On
/// Windows this needs the symlink privilege (developer mode) and fails otherwise; callers skip.
pub fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no symbolic links on this platform",
        ))
    }
}
