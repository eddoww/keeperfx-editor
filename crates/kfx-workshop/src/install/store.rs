//! The [`Store`] of install manifests and backups, and what an install can open afterwards.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    /// Replaced an existing file; the old content is in the store under `backup`.
    pub backup: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadOrderRecord {
    pub existed_before: bool,
    pub backup: Option<String>,
    pub post_sha256: String,
    /// (stage, name) lines this install added.
    pub added: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledLevel {
    pub dir: String,
    pub number: u32,
    pub name: String,
    pub renumbered_from: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub kind: String,
    pub install_root: String,
    pub item: Option<ItemRef>,
    pub archive_sha256: String,
    pub installed_at: u64,
    pub files: Vec<InstalledFile>,
    /// Folders this install created, shallow first.
    pub dirs: Vec<String>,
    pub load_order: Option<LoadOrderRecord>,
    pub levels: Vec<InstalledLevel>,
    pub campaigns: Vec<String>,
}

/// The editor's own record of installs (outside the game folder).
#[derive(Clone, Debug)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: dir.into() }
    }
    fn manifest_path(&self, id: &str) -> PathBuf {
        self.dir.join("manifests").join(format!("{id}.json"))
    }
    pub fn backup_dir(&self, id: &str) -> PathBuf {
        self.dir.join("backups").join(id)
    }
    pub(super) fn staging_dir(&self, id: &str) -> PathBuf {
        self.dir.join("staging").join(id)
    }
    pub fn list(&self) -> Vec<Manifest> {
        let mut v: Vec<Manifest> = std::fs::read_dir(self.dir.join("manifests"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect();
        v.sort_by(|a, b| (a.installed_at, &a.id).cmp(&(b.installed_at, &b.id)));
        v
    }
    pub fn load(&self, id: &str) -> Result<Manifest> {
        let p = self.manifest_path(id);
        let b = std::fs::read(&p).map_err(|e| Error::io(&p, e))?;
        serde_json::from_slice(&b).map_err(|e| Error::Other(format!("{}: {e}", p.display())))
    }
    pub fn save(&self, m: &Manifest) -> Result<()> {
        let p = self.manifest_path(&m.id);
        std::fs::create_dir_all(p.parent().unwrap_or(&self.dir)).map_err(|e| Error::io(&p, e))?;
        let json = serde_json::to_vec_pretty(m).map_err(|e| Error::Other(e.to_string()))?;
        atomic_write(&p, &json).map_err(|e| Error::io(&p, e))
    }
    pub(super) fn remove(&self, id: &str) {
        let _ = std::fs::remove_file(self.manifest_path(id));
        let _ = std::fs::remove_dir_all(self.backup_dir(id));
        let _ = std::fs::remove_dir_all(self.staging_dir(id));
    }
    pub(super) fn fresh_id(&self, name: &str) -> String {
        let base = format!("{}-{}", slug(name), now_secs());
        if !self.manifest_path(&base).exists() {
            return base;
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|c| !self.manifest_path(c).exists())
            .unwrap_or(base)
    }
}

/// Where to open an installed level or campaign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    /// The `.slb` of a level.
    Level {
        slb: PathBuf,
        number: u32,
        name: String,
    },
    Campaign {
        cfg: PathBuf,
    },
}

pub fn open_targets(install: &Path, m: &Manifest) -> Vec<OpenTarget> {
    let mut v = vec![];
    for l in &m.levels {
        let want = format!("map{:05}.slb", l.number);
        if let Some(f) = m.files.iter().find(|f| {
            split_dir(&f.path).0.eq_ignore_ascii_case(&l.dir)
                && split_dir(&f.path).1.eq_ignore_ascii_case(&want)
        }) {
            v.push(OpenTarget::Level {
                slb: install.join(&f.path),
                number: l.number,
                name: l.name.clone(),
            });
        }
    }
    for c in &m.campaigns {
        if let Some(f) = m.files.iter().find(|f| f.path.eq_ignore_ascii_case(c)) {
            v.push(OpenTarget::Campaign {
                cfg: install.join(&f.path),
            });
        }
    }
    v
}
