//! Reading downloaded archives safely.
//!
//! Supported: ZIP (`zip` crate), 7z (`sevenz-rust2`), and RAR through an external `7z`/`7zz`/
//! `7za` found on `PATH` (there is no pure-Rust RAR decoder; without the tool RAR is reported as
//! unsupported). Nothing from an archive is ever executed. Entry names are normalised and
//! checked ([`safe_rel_path`]): absolute paths, drive letters, `..` components and NUL are
//! rejected, symbolic links are never extracted, and [`Limits`] bound entry count, per-file and
//! total decompressed size and the compression ratio (zip bombs).

use crate::error::{Error, Result};
use std::io::Read;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    SevenZ,
    Rar,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_entries: usize,
    pub max_file: u64,
    pub max_total: u64,
    /// Total decompressed / archive size above which an archive over 64 MiB unpacked is refused.
    pub max_ratio: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_entries: 20_000,
            max_file: 512 << 20,
            max_total: 2 << 30,
            max_ratio: 1_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Normalised relative path with `/` separators (empty only for rejected entries).
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Why the entry may not be extracted (zip-slip, absolute path, link, ...).
    pub problem: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Listing {
    pub format: Format,
    pub entries: Vec<Entry>,
    /// Size of the archive file.
    pub archive_size: u64,
}

impl Listing {
    pub fn files(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| !e.is_dir && e.problem.is_none())
    }
    pub fn problems(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|e| e.problem.clone())
            .collect()
    }
    pub fn total_size(&self) -> u64 {
        self.files().map(|e| e.size).sum()
    }
}

pub fn detect_format(head: &[u8]) -> Format {
    if head.starts_with(b"PK\x03\x04")
        || head.starts_with(b"PK\x05\x06")
        || head.starts_with(b"PK\x07\x08")
    {
        Format::Zip
    } else if head.starts_with(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]) {
        Format::SevenZ
    } else if head.starts_with(b"Rar!\x1a\x07") {
        Format::Rar
    } else {
        Format::Unknown
    }
}

pub fn detect_file(path: &Path) -> Result<Format> {
    let mut f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut head = [0u8; 8];
    let n = f.read(&mut head).map_err(|e| Error::io(path, e))?;
    Ok(detect_format(&head[..n]))
}

/// Normalise an archive entry name into a safe relative path, or say why not.
pub fn safe_rel_path(raw: &str) -> std::result::Result<String, String> {
    if raw.contains('\0') {
        return Err(format!("{raw:?}: NUL in name"));
    }
    let n = raw.replace('\\', "/");
    if n.starts_with('/') {
        return Err(format!("{raw}: absolute path"));
    }
    let b = n.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return Err(format!("{raw}: drive letter"));
    }
    let mut parts = vec![];
    for c in n.split('/') {
        match c {
            "" | "." => {}
            ".." => return Err(format!("{raw}: parent directory component (zip-slip)")),
            c if c.chars().any(|ch| ch.is_control()) => {
                return Err(format!("{raw}: control character in name"))
            }
            c => parts.push(c),
        }
    }
    if parts.is_empty() {
        return Err(format!("{raw}: empty name"));
    }
    Ok(parts.join("/"))
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

fn mk_entry(raw: &str, size: u64, is_dir: bool, symlink: bool) -> Entry {
    match safe_rel_path(raw) {
        Ok(p) => Entry {
            path: p,
            size,
            is_dir,
            is_symlink: symlink,
            problem: symlink.then(|| format!("{raw}: symbolic link")),
        },
        Err(e) => Entry {
            path: String::new(),
            size,
            is_dir,
            is_symlink: symlink,
            problem: Some(e),
        },
    }
}

/// List an archive (no content is read). Unsafe entries are reported with `problem` set.
pub fn list(path: &Path, limits: &Limits) -> Result<Listing> {
    let archive_size = std::fs::metadata(path)
        .map_err(|e| Error::io(path, e))?
        .len();
    let format = detect_file(path)?;
    let entries = match format {
        Format::Zip => {
            let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
            let mut z = zip::ZipArchive::new(f).map_err(|e| archive_err(path, e.to_string()))?;
            if z.len() > limits.max_entries {
                return Err(Error::Unsafe(format!(
                    "{}: {} entries (limit {})",
                    path.display(),
                    z.len(),
                    limits.max_entries
                )));
            }
            let mut v = vec![];
            for i in 0..z.len() {
                let e = z
                    .by_index_raw(i)
                    .map_err(|e| archive_err(path, format!("entry {i}: {e}")))?;
                let symlink = e.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000);
                v.push(mk_entry(e.name(), e.size(), e.is_dir(), symlink));
            }
            v
        }
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
                    mk_entry(e.name(), e.size(), e.is_directory(), link)
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
                    v.push(mk_entry(&n, size, dir, link));
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
        Format::Unknown => return Err(archive_err(path, "not a ZIP, 7z or RAR archive")),
    };
    let l = Listing {
        format,
        entries,
        archive_size,
    };
    check_limits(path, &l, limits)?;
    Ok(l)
}

fn check_limits(path: &Path, l: &Listing, limits: &Limits) -> Result<()> {
    let total = l.total_size();
    if let Some(e) = l.files().find(|e| e.size > limits.max_file) {
        return Err(Error::Unsafe(format!(
            "{}: {} is {} bytes unpacked (limit {})",
            path.display(),
            e.path,
            e.size,
            limits.max_file
        )));
    }
    if total > limits.max_total {
        return Err(Error::Unsafe(format!(
            "{}: {} bytes unpacked (limit {})",
            path.display(),
            total,
            limits.max_total
        )));
    }
    if total > 64 << 20 && l.archive_size > 0 && total / l.archive_size.max(1) > limits.max_ratio {
        return Err(Error::Unsafe(format!(
            "{}: compression ratio {}:1 looks like a zip bomb",
            path.display(),
            total / l.archive_size.max(1)
        )));
    }
    Ok(())
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
    let mut total = 0u64;
    let mut take = |name: &str,
                    declared: u64,
                    r: &mut dyn Read,
                    sink: &mut dyn FnMut(&str, Vec<u8>) -> Result<()>|
     -> Result<()> {
        let cap = declared.min(limits.max_file);
        let mut buf = vec![];
        r.take(cap + 1)
            .read_to_end(&mut buf)
            .map_err(|e| archive_err(path, format!("{name}: {e}")))?;
        if buf.len() as u64 > cap {
            return Err(Error::Unsafe(format!(
                "{}: {name} is larger than its declared size ({declared})",
                path.display()
            )));
        }
        total += buf.len() as u64;
        if total > limits.max_total {
            return Err(Error::Unsafe(format!(
                "{}: unpacked data exceeds {} bytes",
                path.display(),
                limits.max_total
            )));
        }
        sink(name, buf)
    };
    match listing.format {
        Format::Zip => {
            let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
            let mut z = zip::ZipArchive::new(f).map_err(|e| archive_err(path, e.to_string()))?;
            for (i, ent) in listing.entries.iter().enumerate() {
                if ent.is_dir || ent.problem.is_some() || !wanted(&ent.path) {
                    continue;
                }
                let mut zf = z
                    .by_index(i)
                    .map_err(|e| archive_err(path, format!("{}: {e}", ent.path)))?;
                take(&ent.path, ent.size, &mut zf, sink)?;
            }
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
