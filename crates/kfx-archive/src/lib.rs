//! Safe reading of downloaded archives: ZIP and gzip-compressed tar.
//!
//! Shared by the Workshop installer (`kfx-workshop`, which adds 7z and RAR on top) and the
//! self-updater (`kfx-update`). Nothing from an archive is ever executed. Entry names are
//! normalised and checked ([`safe_rel_path`]): absolute paths, drive letters, `..` components,
//! control characters and NUL are rejected; symbolic links, hard links and special files are
//! never extracted; [`Limits`] bound the entry count, the per-file and total unpacked size and the
//! compression ratio (zip bombs), and an entry that holds more data than its header declares
//! aborts the read.
//!
//! [`list`] inspects an archive, [`read_entries`] reads chosen entries into memory, and
//! [`extract_all`] writes every safe entry below a new, empty folder.

use std::io::Read;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The archive could not be read or its format is not supported.
    #[error("archive {path}: {detail}")]
    Archive { path: String, detail: String },
    /// An entry would escape the target folder, is a link, or exceeds a limit.
    #[error("unsafe archive: {0}")]
    Unsafe(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Error {
        Error::Io {
            path: path.into(),
            source,
        }
    }
    pub fn archive(path: &Path, detail: impl Into<String>) -> Error {
        Error::Archive {
            path: path.display().to_string(),
            detail: detail.into(),
        }
    }
}

/// Archive formats recognised by their first bytes. This crate reads [`Format::Zip`] and
/// [`Format::TarGz`]; the others are recognised so that callers can handle them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    /// gzip-compressed tar (`.tar.gz`, `.tgz`).
    TarGz,
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
    /// A symbolic link, hard link or special file (never extracted).
    pub is_symlink: bool,
    /// The archive marks the file executable (Unix mode bits).
    pub executable: bool,
    /// Why the entry may not be extracted (zip-slip, absolute path, link, ...).
    pub problem: Option<String>,
}

impl Entry {
    /// An entry from its raw archive name; unsafe names and links get a [`Entry::problem`].
    pub fn new(raw: &str, size: u64, is_dir: bool, symlink: bool) -> Entry {
        match safe_rel_path(raw) {
            Ok(p) => Entry {
                path: p,
                size,
                is_dir,
                is_symlink: symlink,
                executable: false,
                problem: symlink.then(|| format!("{raw}: symbolic link")),
            },
            Err(e) => Entry {
                path: String::new(),
                size,
                is_dir,
                is_symlink: symlink,
                executable: false,
                problem: Some(e),
            },
        }
    }
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
    } else if head.starts_with(&[0x1f, 0x8b]) {
        Format::TarGz
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

/// The entry count, sizes and compression ratio of a listing against `limits`.
pub fn check_limits(path: &Path, l: &Listing, limits: &Limits) -> Result<()> {
    if l.entries.len() > limits.max_entries {
        return Err(Error::Unsafe(format!(
            "{}: {} entries (limit {})",
            path.display(),
            l.entries.len(),
            limits.max_entries
        )));
    }
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

/// Bounded reading of entry contents: each entry at most its declared size (and
/// [`Limits::max_file`]), all entries together at most [`Limits::max_total`].
pub struct Budget<'a> {
    path: &'a Path,
    limits: &'a Limits,
    total: u64,
}

impl<'a> Budget<'a> {
    pub fn new(path: &'a Path, limits: &'a Limits) -> Self {
        Budget {
            path,
            limits,
            total: 0,
        }
    }

    /// Read one entry whose header declares `declared` bytes.
    pub fn read(&mut self, name: &str, declared: u64, r: &mut dyn Read) -> Result<Vec<u8>> {
        let cap = declared.min(self.limits.max_file);
        let mut buf = vec![];
        r.take(cap + 1)
            .read_to_end(&mut buf)
            .map_err(|e| Error::archive(self.path, format!("{name}: {e}")))?;
        if buf.len() as u64 > cap {
            return Err(Error::Unsafe(format!(
                "{}: {name} is larger than its declared size ({declared})",
                self.path.display()
            )));
        }
        self.total += buf.len() as u64;
        if self.total > self.limits.max_total {
            return Err(Error::Unsafe(format!(
                "{}: unpacked data exceeds {} bytes",
                self.path.display(),
                self.limits.max_total
            )));
        }
        Ok(buf)
    }
}

/// List a ZIP or tar.gz archive (no content is kept). Unsafe entries are reported with
/// `problem` set; an archive over the limits is an error.
pub fn list(path: &Path, limits: &Limits) -> Result<Listing> {
    let archive_size = std::fs::metadata(path)
        .map_err(|e| Error::io(path, e))?
        .len();
    let format = detect_file(path)?;
    let entries = match format {
        Format::Zip => list_zip(path, limits)?,
        Format::TarGz => list_tar_gz(path, limits)?,
        Format::SevenZ | Format::Rar => {
            return Err(Error::archive(
                path,
                "7z and RAR archives are not read here",
            ))
        }
        Format::Unknown => return Err(Error::archive(path, "not a ZIP or tar.gz archive")),
    };
    let l = Listing {
        format,
        entries,
        archive_size,
    };
    check_limits(path, &l, limits)?;
    Ok(l)
}

/// List the entries of a ZIP archive (used by [`list`]).
pub fn list_zip(path: &Path, limits: &Limits) -> Result<Vec<Entry>> {
    let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| Error::archive(path, e.to_string()))?;
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
            .map_err(|e| Error::archive(path, format!("entry {i}: {e}")))?;
        let mode = e.unix_mode();
        let symlink = mode.is_some_and(|m| m & 0o170000 == 0o120000);
        let mut ent = Entry::new(e.name(), e.size(), e.is_dir(), symlink);
        ent.executable = mode.is_some_and(|m| m & 0o111 != 0);
        v.push(ent);
    }
    Ok(v)
}

fn open_tar_gz(path: &Path) -> Result<tar::Archive<flate2::read::GzDecoder<std::fs::File>>> {
    let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    Ok(tar::Archive::new(flate2::read::GzDecoder::new(f)))
}

/// The entry of a tar header; `None` for metadata entries (PAX / GNU extension headers are
/// folded into the next entry by the `tar` crate and do not show up here).
fn tar_entry<R: Read>(e: &tar::Entry<R>) -> Entry {
    use tar::EntryType as T;
    let raw = String::from_utf8_lossy(&e.path_bytes()).into_owned();
    let h = e.header();
    let size = e.size();
    let kind = h.entry_type();
    let mut ent = match kind {
        T::Regular | T::Continuous => Entry::new(&raw, size, false, false),
        T::Directory => Entry::new(&raw, 0, true, false),
        T::Symlink => Entry::new(&raw, size, false, true),
        T::Link => {
            let mut x = Entry::new(&raw, size, false, true);
            if x.problem
                .as_deref()
                .is_some_and(|p| p.ends_with("symbolic link"))
            {
                x.problem = Some(format!("{raw}: hard link"));
            }
            x
        }
        other => {
            let mut x = Entry::new(&raw, size, false, true);
            x.problem = Some(format!("{raw}: special file ({other:?})"));
            x
        }
    };
    if std::str::from_utf8(&e.path_bytes()).is_err() && ent.problem.is_none() {
        ent.problem = Some(format!("{raw}: name is not UTF-8"));
    }
    ent.executable = h.mode().is_ok_and(|m| m & 0o111 != 0);
    ent
}

/// List the entries of a tar.gz archive (used by [`list`]). The limits are checked while
/// reading, so a bomb is refused before it is decompressed.
pub fn list_tar_gz(path: &Path, limits: &Limits) -> Result<Vec<Entry>> {
    let mut a = open_tar_gz(path)?;
    let mut v = vec![];
    let mut total = 0u64;
    let iter = a
        .entries()
        .map_err(|e| Error::archive(path, e.to_string()))?;
    for e in iter {
        let e = e.map_err(|e| Error::archive(path, e.to_string()))?;
        let ent = tar_entry(&e);
        if v.len() >= limits.max_entries {
            return Err(Error::Unsafe(format!(
                "{}: more than {} entries",
                path.display(),
                limits.max_entries
            )));
        }
        if !ent.is_dir && ent.problem.is_none() {
            if ent.size > limits.max_file {
                return Err(Error::Unsafe(format!(
                    "{}: {} is {} bytes unpacked (limit {})",
                    path.display(),
                    ent.path,
                    ent.size,
                    limits.max_file
                )));
            }
            total += ent.size;
            if total > limits.max_total {
                return Err(Error::Unsafe(format!(
                    "{}: more than {} bytes unpacked",
                    path.display(),
                    limits.max_total
                )));
            }
        }
        v.push(ent);
    }
    Ok(v)
}

/// Read the content of the wanted entries (by normalised path) of a ZIP or tar.gz archive.
/// Reading is bounded by `limits` and by each entry's declared size (a header that lies about
/// the size aborts). Entries with a `problem` are never read. `sink` receives `(path, bytes)`.
pub fn read_entries(
    path: &Path,
    wanted: &dyn Fn(&str) -> bool,
    limits: &Limits,
    sink: &mut dyn FnMut(&str, Vec<u8>) -> Result<()>,
) -> Result<()> {
    let listing = list(path, limits)?;
    read_listed(path, &listing, wanted, limits, sink)
}

/// [`read_entries`] for an archive already listed with [`list`].
pub fn read_listed(
    path: &Path,
    listing: &Listing,
    wanted: &dyn Fn(&str) -> bool,
    limits: &Limits,
    sink: &mut dyn FnMut(&str, Vec<u8>) -> Result<()>,
) -> Result<()> {
    let mut budget = Budget::new(path, limits);
    match listing.format {
        Format::Zip => {
            let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
            let mut z = zip::ZipArchive::new(f).map_err(|e| Error::archive(path, e.to_string()))?;
            for (i, ent) in listing.entries.iter().enumerate() {
                if ent.is_dir || ent.problem.is_some() || !wanted(&ent.path) {
                    continue;
                }
                let mut zf = z
                    .by_index(i)
                    .map_err(|e| Error::archive(path, format!("{}: {e}", ent.path)))?;
                let b = budget.read(&ent.path, ent.size, &mut zf)?;
                sink(&ent.path, b)?;
            }
        }
        Format::TarGz => {
            let mut a = open_tar_gz(path)?;
            let iter = a
                .entries()
                .map_err(|e| Error::archive(path, e.to_string()))?;
            // the second pass must see the same entries as the listing
            for (i, e) in iter.enumerate() {
                let mut e = e.map_err(|e| Error::archive(path, e.to_string()))?;
                let ent = tar_entry(&e);
                if listing.entries.get(i) != Some(&ent) {
                    return Err(Error::Unsafe(format!(
                        "{}: the archive changed while it was read",
                        path.display()
                    )));
                }
                if ent.is_dir || ent.problem.is_some() || !wanted(&ent.path) {
                    continue;
                }
                let b = budget.read(&ent.path, ent.size, &mut e)?;
                sink(&ent.path, b)?;
            }
        }
        _ => return Err(Error::archive(path, "not a ZIP or tar.gz archive")),
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

/// Extract every entry of a ZIP or tar.gz archive below `dest`, which must not exist yet (it
/// is created). Any unsafe entry (see the module documentation) refuses the whole archive
/// before anything is written; a file that would be written twice is refused too. Files the
/// archive marks executable get the executable bits on Unix. Returns the extracted file paths
/// (relative, `/` separators). On error `dest` may hold a partial extraction: the caller removes
/// it.
pub fn extract_all(path: &Path, dest: &Path, limits: &Limits) -> Result<Vec<String>> {
    let listing = list(path, limits)?;
    let problems = listing.problems();
    if !problems.is_empty() {
        return Err(Error::Unsafe(format!(
            "{}: {}",
            path.display(),
            problems.join("; ")
        )));
    }
    std::fs::create_dir(dest).map_err(|e| Error::io(dest, e))?;
    let exec: std::collections::HashSet<&str> = listing
        .files()
        .filter(|e| e.executable)
        .map(|e| e.path.as_str())
        .collect();
    for d in listing.entries.iter().filter(|e| e.is_dir) {
        make_dirs(dest, &d.path)?;
    }
    let mut written = vec![];
    read_listed(path, &listing, &|_| true, limits, &mut |name, bytes| {
        let target = dest.join(name);
        if let Some((parent, _)) = name.rsplit_once('/') {
            make_dirs(dest, parent)?;
        }
        write_new(&target, &bytes, exec.contains(name))?;
        written.push(name.to_string());
        Ok(())
    })?;
    Ok(written)
}

/// Create `rel` (a checked relative path) below `root`, refusing to follow anything that is not
/// a plain folder on the way.
fn make_dirs(root: &Path, rel: &str) -> Result<()> {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        p.push(part);
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => {
                return Err(Error::Unsafe(format!(
                    "{}: not a plain folder",
                    p.display()
                )))
            }
            Err(_) => std::fs::create_dir(&p).map_err(|e| Error::io(&p, e))?,
        }
    }
    Ok(())
}

fn write_new(target: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(if executable { 0o755 } else { 0o644 });
    }
    #[cfg(not(unix))]
    let _ = executable;
    let mut f = o.open(target).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            Error::Unsafe(format!(
                "{}: written twice by the archive",
                target.display()
            ))
        } else {
            Error::io(target, e)
        }
    })?;
    f.write_all(bytes).map_err(|e| Error::io(target, e))?;
    f.sync_all().map_err(|e| Error::io(target, e))
}
