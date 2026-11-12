//! The whole update: download and verify, then install (swap or installer), and a background
//! [`Worker`] for a GUI.

use crate::client::{CheckOutcome, CheckRequest, Client, Update};
use crate::install::{
    eligibility, record_installed, setup_command, stage, swap, BuildInfo, FsOps, Health, Layout,
    LayoutKind, Probe,
};
use crate::platform::{Platform, SUMS_NAME};
use crate::state::StateFiles;
use crate::verify::{check_file, Sums, Verifier};
use crate::{Error, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Largest `SHA256SUMS.txt` accepted.
const MAX_SUMS_BYTES: u64 = 64 << 10;

pub struct Updater {
    pub client: Client,
    pub files: StateFiles,
    pub platform: Platform,
    pub build: BuildInfo,
    pub verifier: Box<dyn Verifier>,
}

/// What [`Updater::install`] did, and what the caller does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    /// The files were replaced: start `exe` again (with the open map) and exit.
    Swapped { exe: PathBuf },
    /// Run the installer with these arguments and exit; it starts the editor again.
    RunSetup {
        program: PathBuf,
        args: Vec<OsString>,
    },
}

impl Updater {
    /// The file to download for `layout`: the installer for installer-based Windows installs,
    /// else the archive.
    pub fn asset_for<'a>(&self, update: &'a Update, layout: &Layout) -> Result<&'a crate::Asset> {
        let a = match layout.kind {
            LayoutKind::WindowsSetup => update.setup.as_ref(),
            _ => update.archive.as_ref(),
        };
        a.ok_or_else(|| Error::NoAsset {
            tag: update.release.tag_name.clone(),
            platform: self.platform.describe(),
        })
    }

    /// Download the package for `layout` and verify it against the release's `SHA256SUMS.txt`
    /// (passed through the [`Verifier`] first). A file that fails the check is deleted. Returns
    /// the verified file.
    pub fn download(
        &self,
        update: &Update,
        layout: &Layout,
        progress: &mut dyn FnMut(u64, u64),
        cancel: &AtomicBool,
    ) -> Result<PathBuf> {
        let asset = self.asset_for(update, layout)?;
        let sums_asset = update
            .sums
            .as_ref()
            .ok_or_else(|| Error::MissingChecksum(format!("{} ({SUMS_NAME})", asset.name)))?;
        let raw = self.client.fetch_small(sums_asset, MAX_SUMS_BYTES)?;
        self.verifier.verify_sums(&update.release, &raw, &mut |a| {
            self.client.fetch_small(a, MAX_SUMS_BYTES)
        })?;
        let text = String::from_utf8(raw).map_err(|_| Error::parse(SUMS_NAME, "not UTF-8"))?;
        let sums = Sums::parse(&text)?;
        if sums.get(&asset.name).is_none() {
            return Err(Error::MissingChecksum(asset.name.clone()));
        }
        let dir = self
            .files
            .downloads_dir()
            .join(safe_name(&update.release.tag_name));
        let dest = dir.join(safe_name(&asset.name));
        if dest.exists() && check_file(&sums, asset, &dest).is_ok() {
            progress(asset.size, asset.size);
            return Ok(dest);
        }
        let _ = std::fs::remove_file(&dest);
        self.client.download(asset, &dest, progress, cancel)?;
        if let Err(e) = check_file(&sums, asset, &dest) {
            let _ = std::fs::remove_file(&dest);
            let _ = std::fs::remove_file(crate::client::part_path(&dest));
            return Err(e);
        }
        Ok(dest)
    }

    /// Install a verified package. Refuses when [`eligibility`] says no. For a swap, the
    /// previous version stays in the rollback folder and an update record is written for the
    /// first start of the new version.
    pub fn install(
        &self,
        update: &Update,
        layout: &Layout,
        file: &Path,
        map: Option<&Path>,
        probe: &dyn Probe,
        fs: &dyn FsOps,
    ) -> Result<Installed> {
        eligibility(layout, &self.build, self.platform, probe).map_err(Error::Blocked)?;
        if layout.kind == LayoutKind::WindowsSetup {
            let (program, args) = setup_command(file, map);
            return Ok(Installed::RunSetup { program, args });
        }
        let version = update.version.to_string();
        let staged = stage(layout, self.platform, file, &version)?;
        swap(layout, &staged, &self.build.version, fs)?;
        let health = Health {
            version,
            previous: self.build.version.clone(),
            root: layout.root.clone(),
            starts: 0,
        };
        // without the record the update still works; only the rollback offer is lost
        let _ = record_installed(&self.files.dir, &health);
        let _ = std::fs::remove_file(file);
        Ok(Installed::Swapped {
            exe: layout.exe.clone(),
        })
    }

    /// Remove downloads of earlier updates (call at start when no update is in progress).
    pub fn clean_downloads(&self) {
        let _ = std::fs::remove_dir_all(self.files.downloads_dir());
    }
}

/// A file name from untrusted text: path separators and odd characters replaced.
fn safe_name(s: &str) -> String {
    let n: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if n.trim_matches('.').is_empty() {
        "_".into()
    } else {
        n
    }
}

/// What the background [`Worker`] is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    /// A check finished.
    Checked(std::result::Result<CheckOutcome, String>),
    Downloading {
        done: u64,
        total: u64,
    },
    /// The package is downloaded and verified.
    Downloaded(PathBuf),
    Failed(String),
    Cancelled,
}

/// Runs checks and downloads on a thread; a GUI polls [`Worker::status`] every frame.
pub struct Worker {
    updater: Arc<Updater>,
    status: Arc<Mutex<Status>>,
    cancel: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
}

impl Worker {
    pub fn new(updater: Updater) -> Worker {
        Worker {
            updater: Arc::new(updater),
            status: Arc::new(Mutex::new(Status::Idle)),
            cancel: Arc::new(AtomicBool::new(false)),
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn updater(&self) -> &Updater {
        &self.updater
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(Status::Idle)
    }

    /// Forget a finished result.
    pub fn reset(&self) {
        if !self.is_busy() {
            if let Ok(mut s) = self.status.lock() {
                *s = Status::Idle;
            }
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn start(
        &self,
        first: Status,
        job: impl FnOnce(&Updater, &Mutex<Status>, &AtomicBool) -> Status + Send + 'static,
    ) -> bool {
        if self.busy.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.cancel.store(false, Ordering::SeqCst);
        if let Ok(mut s) = self.status.lock() {
            *s = first;
        }
        let (u, st, c, b) = (
            self.updater.clone(),
            self.status.clone(),
            self.cancel.clone(),
            self.busy.clone(),
        );
        std::thread::spawn(move || {
            let end = job(&u, &st, &c);
            if let Ok(mut s) = st.lock() {
                *s = end;
            }
            b.store(false, Ordering::SeqCst);
        });
        true
    }

    /// Start a check. Returns false when the worker is busy.
    pub fn check(&self, req: CheckRequest) -> bool {
        self.start(Status::Checking, move |u, _, _| {
            Status::Checked(
                u.client
                    .check(&u.files, &req, crate::state::now_secs())
                    .map_err(|e| e.to_string()),
            )
        })
    }

    /// Start downloading and verifying `update` for `layout`.
    pub fn download(&self, update: Update, layout: Layout) -> bool {
        self.start(
            Status::Downloading { done: 0, total: 0 },
            move |u, st, cancel| {
                let mut progress = |done, total| {
                    if let Ok(mut s) = st.lock() {
                        *s = Status::Downloading { done, total };
                    }
                };
                match u.download(&update, &layout, &mut progress, cancel) {
                    Ok(p) => Status::Downloaded(p),
                    Err(Error::Cancelled) => Status::Cancelled,
                    Err(e) => Status::Failed(e.to_string()),
                }
            },
        )
    }
}
