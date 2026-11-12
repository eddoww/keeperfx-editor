//! Prefab library management: the folders a host points at and the prefabs in them.
//!
//! Three kinds of folder: the read-only starter set (shipped under `data/prefabs/`, also embedded
//! in the library: [`Library::add_embedded_starter`]), the user library (a folder in the platform
//! data dir, path supplied by the host) and a per-campaign library (a folder next to the
//! campaign). Every file is read with [`Prefab::from_toml`]; a file that does not parse is
//! listed in [`Library::problems`] and never replaced. All writes go through
//! `kfx_formats::atomic`.
use crate::model::{Prefab, FILE_EXT};
use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// Where a prefab comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Origin {
    Starter,
    User,
    Campaign,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Starter => "Starter",
            Origin::User => "User",
            Origin::Campaign => "Campaign",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Root {
    pub origin: Origin,
    pub path: PathBuf,
    pub read_only: bool,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub origin: Origin,
    /// `None` for the embedded starter set.
    pub path: Option<PathBuf>,
    pub prefab: Prefab,
    pub read_only: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Library {
    pub roots: Vec<Root>,
    pub entries: Vec<Entry>,
    /// Files that could not be read: `(path, message)`.
    pub problems: Vec<(PathBuf, String)>,
    embedded: bool,
}

/// `name` as a file stem: lower case ASCII letters and digits, other runs become `-`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "prefab".into()
    } else {
        out
    }
}

fn is_prefab_file(p: &Path) -> bool {
    p.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(FILE_EXT))
        && p.is_file()
}

impl Library {
    pub fn new() -> Library {
        Library::default()
    }

    /// Add a folder (created on first save when it does not exist). Call [`Library::scan`].
    pub fn add_root(&mut self, origin: Origin, path: impl Into<PathBuf>, read_only: bool) {
        let path = path.into();
        if !self.roots.iter().any(|r| r.path == path) {
            self.roots.push(Root {
                origin,
                path,
                read_only,
            });
        }
    }

    /// Include the starter set compiled into the library.
    pub fn add_embedded_starter(&mut self) {
        self.embedded = true;
    }

    /// Read every root again. Entries are ordered by origin, then name.
    pub fn scan(&mut self) {
        self.entries.clear();
        self.problems.clear();
        if self.embedded {
            for (name, text) in crate::starter_files::STARTER_FILES {
                match Prefab::from_toml(text) {
                    Ok(prefab) => self.entries.push(Entry {
                        origin: Origin::Starter,
                        path: None,
                        prefab,
                        read_only: true,
                    }),
                    Err(e) => self.problems.push((PathBuf::from(name), e.to_string())),
                }
            }
        }
        for r in self.roots.clone() {
            let Ok(rd) = std::fs::read_dir(&r.path) else {
                continue;
            };
            let mut files: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| is_prefab_file(p))
                .collect();
            files.sort();
            for f in files {
                match Prefab::load(&f) {
                    Ok(prefab) => {
                        // a folder starter file shadows the embedded copy of the same name
                        if r.origin == Origin::Starter {
                            self.entries.retain(|e| {
                                !(e.origin == Origin::Starter
                                    && e.path.is_none()
                                    && e.prefab.meta.name == prefab.meta.name)
                            });
                        }
                        self.entries.push(Entry {
                            origin: r.origin,
                            path: Some(f),
                            prefab,
                            read_only: r.read_only,
                        });
                    }
                    Err(e) => self.problems.push((f, e.to_string())),
                }
            }
        }
        self.entries.sort_by(|a, b| {
            (a.origin, a.prefab.meta.name.to_lowercase())
                .cmp(&(b.origin, b.prefab.meta.name.to_lowercase()))
        });
    }

    /// Indices of the entries matching `query` (case-insensitive words, all must occur in the
    /// name, description, tags or author) and carrying every tag in `tags`.
    pub fn search(&self, query: &str, tags: &[String]) -> Vec<usize> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                let m = &e.prefab.meta;
                let hay = format!(
                    "{} {} {} {}",
                    m.name,
                    m.description,
                    m.author,
                    m.tags.join(" ")
                )
                .to_lowercase();
                words.iter().all(|w| hay.contains(w))
                    && tags
                        .iter()
                        .all(|t| m.tags.iter().any(|x| x.eq_ignore_ascii_case(t)))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Every tag with the number of prefabs using it, most used first.
    pub fn all_tags(&self) -> Vec<(String, usize)> {
        let mut m: std::collections::BTreeMap<String, usize> = Default::default();
        for e in &self.entries {
            for t in &e.prefab.meta.tags {
                *m.entry(t.to_lowercase()).or_default() += 1;
            }
        }
        let mut v: Vec<(String, usize)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    fn root_for(&self, origin: Origin) -> Result<&Root> {
        self.roots
            .iter()
            .find(|r| r.origin == origin && !r.read_only)
            .ok_or_else(|| Error::Refused(format!("no writable {} library folder", origin.label())))
    }

    fn free_path(dir: &Path, stem: &str) -> PathBuf {
        let mut p = dir.join(format!("{stem}.{FILE_EXT}"));
        let mut n = 2;
        while p.exists() {
            p = dir.join(format!("{stem}-{n}.{FILE_EXT}"));
            n += 1;
        }
        p
    }

    /// Save `prefab` as a new file of the writable folder of `origin` (never replaces a file).
    /// Returns the index of the new entry.
    pub fn save_new(&mut self, origin: Origin, prefab: Prefab) -> Result<usize> {
        let dir = self.root_for(origin)?.path.clone();
        std::fs::create_dir_all(&dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
        let path = Self::free_path(&dir, &slug(&prefab.meta.name));
        prefab.save(&path)?;
        self.scan();
        self.entries
            .iter()
            .position(|e| e.path.as_deref() == Some(path.as_path()))
            .ok_or_else(|| Error::Io("saved prefab not found again".into()))
    }

    /// Replace the file of entry `i` with `prefab` (not for read-only entries).
    pub fn update(&mut self, i: usize, prefab: Prefab) -> Result<()> {
        let e = self
            .entries
            .get(i)
            .ok_or_else(|| Error::Invalid("no such prefab".into()))?;
        if e.read_only {
            return Err(Error::Refused("this prefab is read-only".into()));
        }
        let path = e
            .path
            .clone()
            .ok_or_else(|| Error::Refused("this prefab has no file".into()))?;
        prefab.save(&path)?;
        self.entries[i].prefab = prefab;
        Ok(())
    }

    /// Import a `.kfxprefab` file into the writable folder of `origin`: the file must parse; its
    /// bytes are copied unchanged (unknown keys survive). Returns the new entry's index.
    pub fn import_file(&mut self, src: &Path, origin: Origin) -> Result<usize> {
        let bytes = std::fs::read(src).map_err(|e| Error::Io(format!("{}: {e}", src.display())))?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|_| Error::Invalid(format!("{}: not UTF-8", src.display())))?;
        let p = Prefab::from_toml(&text)
            .map_err(|e| Error::Invalid(format!("{}: {e}", src.display())))?;
        let dir = self.root_for(origin)?.path.clone();
        std::fs::create_dir_all(&dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
        let stem = src
            .file_stem()
            .map(|s| slug(&s.to_string_lossy()))
            .unwrap_or_else(|| slug(&p.meta.name));
        let path = Self::free_path(&dir, &stem);
        kfx_formats::atomic::atomic_write(&path, &bytes)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        self.scan();
        self.entries
            .iter()
            .position(|e| e.path.as_deref() == Some(path.as_path()))
            .ok_or_else(|| Error::Io("imported prefab not found again".into()))
    }

    /// Write entry `i` to `dest` (file text of the loaded prefab; refuses to overwrite a
    /// different file without `overwrite`).
    pub fn export(&self, i: usize, dest: &Path, overwrite: bool) -> Result<()> {
        let e = self
            .entries
            .get(i)
            .ok_or_else(|| Error::Invalid("no such prefab".into()))?;
        if dest.exists() && !overwrite {
            return Err(Error::Refused(format!("{} exists", dest.display())));
        }
        e.prefab.save(dest)
    }

    /// Delete the file of entry `i` (user and campaign entries only).
    pub fn delete(&mut self, i: usize) -> Result<()> {
        let e = self
            .entries
            .get(i)
            .ok_or_else(|| Error::Invalid("no such prefab".into()))?;
        if e.read_only {
            return Err(Error::Refused("this prefab is read-only".into()));
        }
        let path = e
            .path
            .clone()
            .ok_or_else(|| Error::Refused("this prefab has no file".into()))?;
        std::fs::remove_file(&path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        self.entries.remove(i);
        Ok(())
    }
}

/// The folder of the per-campaign library next to a campaign's level folder.
pub fn campaign_folder(levels_dir: &Path) -> PathBuf {
    levels_dir.join("prefabs")
}
