//! Reading downloaded archives safely.
//!
//! Supported: ZIP and tar.gz (through the shared `kfx-archive` crate, which also holds the name
//! checks and limits), 7z (`sevenz-rust2`), and RAR through an external `7z`/`7zz`/`7za` found on
//! `PATH` (there is no pure-Rust RAR decoder; without the tool RAR is reported as unsupported).
//! Nothing from an archive is ever executed. Entry names are normalised and checked
//! ([`safe_rel_path`]): absolute paths, drive letters, `..` components and NUL are rejected,
//! symbolic links are never extracted, and [`Limits`] bound entry count, per-file and total
//! decompressed size and the compression ratio (zip bombs).

use crate::error::{Error, Result};
use std::io::Read;
use std::path::Path;

pub use kfx_archive::{
    check_limits, detect_format, safe_rel_path, Budget, Entry, Format, Limits, Listing,
};

pub fn detect_file(path: &Path) -> Result<Format> {
    Ok(kfx_archive::detect_file(path)?)
}

fn archive_err(path: &Path, detail: impl Into<String>) -> Error {
    Error::Archive {
        path: path.display().to_string(),
        detail: detail.into(),
    }
}

fn external_7z() -> Option<&'static str> {
    ["7zz", "7z", "7za", "7zr"].into_iter().find(|c| {
        std::process::Command::new(c)
            .arg("--help")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    })
}

/// List an archive (no content is read). Unsafe entries are reported with `problem` set.
pub fn list(path: &Path, limits: &Limits) -> Result<Listing> {
    let archive_size = std::fs::metadata(path)
        .map_err(|e| Error::io(path, e))?
        .len();
    let format = detect_file(path)?;
    let entries = match format {
        Format::Zip => kfx_archive::list_zip(path, limits)?,
        Format::TarGz => kfx_archive::list_tar_gz(path, limits)?,
        Format::SevenZ => {
            let a =
                sevenz_rust2::Archive::open(path).map_err(|e| archive_err(path, e.to_string()))?;
            if a.files.len() > limits.max_entries {
                return Err(Error::Unsafe(format!(
                    "{}: {} entries (limit {})",
                    path.display(),
                    a.files.len(),
                    limits.max_entries
                )));
            }
            // FILE_ATTRIBUTE_REPARSE_POINT (0x400) or unix symlink in the high 16 bits
            a.files
                .iter()
                .map(|e| {
                    let attr = e.windows_attributes();
                    let link = attr & 0x400 != 0
                        || (attr & 0x8000 != 0 && (attr >> 16) & 0o170000 == 0o120000);
                    Entry::new(e.name(), e.size(), e.is_directory(), link)
                })
                .collect()
        }
        Format::Rar => {
            let tool = external_7z().ok_or_else(|| {
                archive_err(
                    path,
                    "RAR archives need an external 7z (7zz/7z/7za) on PATH; none \
                        found. Extract the archive by hand.",
                )
            })?;
            let out = std::process::Command::new(tool)
                .args(["l", "-slt", "-ba", "--"])
                .arg(path)
                .output()
                .map_err(|e| archive_err(path, e.to_string()))?;
            if !out.status.success() {
                return Err(archive_err(
                    path,
                    format!("{tool} could not list the archive"),
                ));
            }
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            let mut v = vec![];
            for block in text.split("\n\n") {
                let (mut name, mut size, mut dir, mut link) = (None, 0u64, false, false);
                for l in block.lines() {
                    if let Some(x) = l.strip_prefix("Path = ") {
                        name = Some(x.to_string());
                    } else if let Some(x) = l.strip_prefix("Size = ") {
                        size = x.trim().parse().unwrap_or(0);
                    } else if let Some(x) = l.strip_prefix("Folder = ") {
                        dir = x.trim() == "+";
                    } else if let Some(x) = l.strip_prefix("Attributes = ") {
                        link = x.trim_start().starts_with('l') || x.contains(" l");
                    } else if let Some(x) = l.strip_prefix("Symbolic Link = ") {
                        link = link || !x.trim().is_empty();
                    }
                }
                if let Some(n) = name {
                    v.push(Entry::new(&n, size, dir, link));
                }
            }
            if v.len() > limits.max_entries {
                return Err(Error::Unsafe(format!(
                    "{}: {} entries",
                    path.display(),
                    v.len()
                )));
            }
            v
        }
        Format::Unknown => return Err(archive_err(path, "not a ZIP, tar.gz, 7z or RAR archive")),
    };
    let l = Listing {
        format,
        entries,
        archive_size,
    };
    check_limits(path, &l, limits)?;
    Ok(l)
}

/// Read the content of the wanted entries (by normalised path). Reading is bounded by `limits`
/// and by each entry's declared size (a header that lies about the size aborts). Entries with a
/// `problem` are never read. `sink` receives `(path, bytes)`.
pub fn read_entries(
    path: &Path,
    wanted: &dyn Fn(&str) -> bool,
    limits: &Limits,
    sink: &mut dyn FnMut(&str, Vec<u8>) -> Result<()>,
) -> Result<()> {
    let listing = list(path, limits)?;
    let mut budget = Budget::new(path, limits);
    let mut take = |name: &str,
                    declared: u64,
                    r: &mut dyn Read,
                    sink: &mut dyn FnMut(&str, Vec<u8>) -> Result<()>|
     -> Result<()> {
        let b = budget.read(name, declared, r)?;
        sink(name, b)
    };
    match listing.format {
        Format::Zip | Format::TarGz => {
            // the sink's own error is kept and returned as is
            let mut own: Option<Error> = None;
            let r = kfx_archive::read_listed(path, &listing, wanted, limits, &mut |n, b| {
                sink(n, b).map_err(|e| {
                    own = Some(e);
                    kfx_archive::Error::Unsafe("stopped".into())
                })
            });
            if let Some(e) = own {
                return Err(e);
            }
            r?;
        }
        Format::SevenZ => {
            let mut r = sevenz_rust2::SevenZReader::open(path, sevenz_rust2::Password::empty())
                .map_err(|e| archive_err(path, e.to_string()))?;
            let mut by_name: std::collections::HashMap<String, &Entry> =
                std::collections::HashMap::new();
            // 7z entry names are normalised the same way
            for e in &listing.entries {
                by_name.insert(e.path.clone(), e);
            }
            let mut err: Option<Error> = None;
            let res = r.for_each_entries(|ent, rd| {
                if ent.is_directory() {
                    return Ok(true);
                }
                let drain = |rd: &mut dyn Read| {
                    // an unread entry of a solid block fails its checksum: consume it (bounded)
                    let _ = std::io::copy(&mut rd.take(limits.max_file + 1), &mut std::io::sink());
                };
                let Ok(p) = safe_rel_path(ent.name()) else {
                    drain(rd);
                    return Ok(true);
                };
                let Some(le) = by_name.get(&p) else {
                    drain(rd);
                    return Ok(true);
                };
                if le.problem.is_some() || !wanted(&p) {
                    drain(rd);
                    return Ok(true);
                }
                match take(&p, ent.size(), rd, sink) {
                    Ok(()) => Ok(true),
                    Err(e) => {
                        err = Some(e);
                        Ok(false)
                    }
                }
            });
            if let Some(e) = err {
                return Err(e);
            }
            res.map_err(|e| archive_err(path, e.to_string()))?;
        }
        Format::Rar => {
            let tool = external_7z().ok_or_else(|| archive_err(path, "no external 7z found"))?;
            for ent in &listing.entries {
                if ent.is_dir || ent.problem.is_some() || !wanted(&ent.path) {
                    continue;
                }
                let mut child = std::process::Command::new(tool)
                    .args(["e", "-so", "-y", "--"])
                    .arg(path)
                    .arg(&ent.path)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|e| archive_err(path, e.to_string()))?;
                let mut out = child
                    .stdout
                    .take()
                    .ok_or_else(|| archive_err(path, "no stdout"))?;
                let r = take(&ent.path, ent.size, &mut out, sink);
                let _ = child.kill();
                let _ = child.wait();
                r?;
            }
        }
        Format::Unknown => return Err(archive_err(path, "unknown archive format")),
    }
    Ok(())
}

/// Convenience: read one entry.
pub fn read_one(path: &Path, entry: &str, limits: &Limits) -> Result<Option<Vec<u8>>> {
    let mut found = None;
    read_entries(path, &|p| p == entry, limits, &mut |_, b| {
        found = Some(b);
        Ok(())
    })?;
    Ok(found)
}
