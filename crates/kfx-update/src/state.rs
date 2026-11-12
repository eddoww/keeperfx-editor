//! What the update check remembers between runs (`update-state.json` in the editor's
//! configuration folder): when it last asked GitHub, the ETag and body of the last answer, and
//! until when it must not ask again (rate limit, back-off after failures).

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Automatic checks happen at most this often.
pub const AUTO_INTERVAL_SECS: u64 = 24 * 3600;
/// First back-off after a failed check; doubled per further failure, up to a day.
pub const BACKOFF_BASE_SECS: u64 = 15 * 60;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CheckState {
    /// Unix seconds of the last answered request (200 or 304).
    pub last_check: Option<u64>,
    /// ETag of the cached release list (sent as `If-None-Match`).
    pub etag: Option<String>,
    /// No request before this time (Unix seconds).
    pub backoff_until: Option<u64>,
    /// Failed checks in a row (for the exponential back-off).
    pub failures: u32,
    /// The last error, for the Updates dialog.
    pub last_error: Option<String>,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `until` as "in N minutes" / "in N hours" from now.
pub fn describe_time(until: u64) -> String {
    let now = now_secs();
    let d = until.saturating_sub(now);
    if d < 90 {
        format!("in {d} s")
    } else if d < 90 * 60 {
        format!("in {} min", d.div_ceil(60))
    } else {
        format!("in {} h", d.div_ceil(3600))
    }
}

impl CheckState {
    /// An automatic check is due: a day after the last one and not during a back-off.
    pub fn auto_due(&self, now: u64) -> bool {
        self.last_check
            .is_none_or(|t| now >= t.saturating_add(AUTO_INTERVAL_SECS) || now < t)
            && !self.backing_off(now)
    }

    pub fn backing_off(&self, now: u64) -> bool {
        self.backoff_until.is_some_and(|u| now < u)
    }

    /// Record an answered request.
    pub fn succeeded(&mut self, now: u64, etag: Option<String>) {
        self.last_check = Some(now);
        if etag.is_some() {
            self.etag = etag;
        }
        self.failures = 0;
        self.backoff_until = None;
        self.last_error = None;
    }

    /// Record a failed request: back off exponentially (15 min, 30 min, ... up to a day), or
    /// until `until` when the server said when to come back.
    pub fn failed(&mut self, now: u64, err: &Error, until: Option<u64>) {
        self.failures = self.failures.saturating_add(1);
        let exp = BACKOFF_BASE_SECS
            .saturating_mul(1u64 << (self.failures - 1).min(10))
            .min(AUTO_INTERVAL_SECS);
        let at = until.unwrap_or(now + exp).max(now + 60);
        self.backoff_until = Some(at.min(now + AUTO_INTERVAL_SECS));
        self.last_error = Some(err.to_string());
    }
}

/// Rate-limit information of a GitHub answer. Returns when the next request may be made, if the
/// answer says to wait: a `429` or a `403` with no requests left (`x-ratelimit-remaining: 0`,
/// until `x-ratelimit-reset`), or a `retry-after` header (seconds).
pub fn rate_limit_until(
    status: u16,
    header: &dyn Fn(&str) -> Option<String>,
    now: u64,
) -> Option<u64> {
    let num = |n: &str| header(n).and_then(|v| v.trim().parse::<u64>().ok());
    if let Some(s) = num("retry-after") {
        if status == 403 || status == 429 || status == 503 {
            return Some(now + s.max(1));
        }
    }
    let exhausted = num("x-ratelimit-remaining") == Some(0);
    if (status == 403 || status == 429) && (exhausted || status == 429) {
        // the reset time, or an hour when the answer does not say
        let reset = num("x-ratelimit-reset").filter(|r| *r > now);
        return Some(reset.unwrap_or(now + 3600));
    }
    None
}

/// The files of the update state in a folder.
#[derive(Clone, Debug)]
pub struct StateFiles {
    pub dir: PathBuf,
}

impl StateFiles {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        StateFiles { dir: dir.into() }
    }
    fn state_path(&self) -> PathBuf {
        self.dir.join("update-state.json")
    }
    /// The body of the last release list (served again on `304 Not Modified`).
    pub fn cache_path(&self) -> PathBuf {
        self.dir.join("update-releases.json")
    }
    /// Downloads in progress and finished (resumable `.part` files).
    pub fn downloads_dir(&self) -> PathBuf {
        self.dir.join("update-downloads")
    }

    pub fn load(&self) -> CheckState {
        std::fs::read(self.state_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, s: &CheckState) -> Result<()> {
        let b = serde_json::to_vec_pretty(s)
            .map_err(|e| Error::parse("update state", e.to_string()))?;
        write_atomic(&self.state_path(), &b)
    }

    pub fn save_cache(&self, body: &[u8]) -> Result<()> {
        write_atomic(&self.cache_path(), body)
    }

    pub fn load_cache(&self) -> Option<Vec<u8>> {
        std::fs::read(self.cache_path()).ok()
    }
}

/// Write a small file through a temporary file and a rename.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| Error::io(d, e))?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::io(path, e)
    })
}
