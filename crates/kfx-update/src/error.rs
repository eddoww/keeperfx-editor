//! Errors of the update check, download and installation.

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request itself failed (DNS, TLS, timeout, connection reset, ...).
    #[error("network error for {url}: {message}")]
    Network { url: String, message: String },
    /// The server answered with an unexpected HTTP status.
    #[error("HTTP {status} for {url}")]
    Http { status: u16, url: String },
    /// GitHub's rate limit is exhausted (or a request failed recently): no request is made
    /// before `until` (Unix seconds).
    #[error("GitHub asks to wait: no update check before {}", crate::state::describe_time(*until))]
    RateLimited { until: u64 },
    /// A URL outside the pinned hosts, or not HTTPS.
    #[error("refused to contact {0}: only HTTPS to GitHub's release hosts is allowed")]
    UrlNotAllowed(String),
    /// An answer did not have the expected shape.
    #[error("could not understand {what}: {detail}")]
    Parse { what: &'static str, detail: String },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Archive(#[from] kfx_archive::Error),
    /// The release has no `SHA256SUMS.txt`, or it does not list the asset.
    #[error("the release has no checksum for {0}: refusing to install it")]
    MissingChecksum(String),
    /// The downloaded file does not match the published checksum.
    #[error(
        "{name}: checksum mismatch (expected {expected}, got {actual}): the download was discarded"
    )]
    ChecksumMismatch {
        name: String,
        expected: String,
        actual: String,
    },
    /// The downloaded file does not have the size the release lists.
    #[error("{name}: {actual} bytes downloaded, the release lists {expected}")]
    SizeMismatch {
        name: String,
        expected: u64,
        actual: u64,
    },
    /// A signature check ([`crate::verify::Verifier`]) refused the release.
    #[error("signature check failed: {0}")]
    Signature(String),
    /// The release has no download for this platform.
    #[error("release {tag} has no download for {platform}")]
    NoAsset { tag: String, platform: String },
    /// Self-update is not possible for this installation; only a notice is shown.
    #[error("{0}")]
    Blocked(crate::install::Blocker),
    /// The downloaded package does not have the expected layout.
    #[error("unexpected package layout: {0}")]
    Package(String),
    /// Replacing the files failed; the previous version was put back.
    #[error("installing the update failed ({0}); the current version was kept")]
    Swap(String),
    #[error("cancelled")]
    Cancelled,
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Error {
        Error::Io {
            path: path.into(),
            source,
        }
    }
    pub fn parse(what: &'static str, detail: impl Into<String>) -> Error {
        Error::Parse {
            what,
            detail: detail.into(),
        }
    }
    /// Retrying later may help (network trouble, server errors, rate limits).
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Network { .. } | Error::RateLimited { .. } => true,
            Error::Http { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }
}
