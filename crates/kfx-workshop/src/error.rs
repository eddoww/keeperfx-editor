//! Errors of the Workshop client, installer and packager.

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Offline mode (or no network) and the answer is not in the cache.
    #[error("offline: {0} is not in the cache")]
    Offline(String),
    /// The server answered with an HTTP error status.
    #[error("the Workshop answered HTTP {status} for {url}")]
    Http { status: u16, url: String },
    /// The request itself failed (DNS, TLS, timeout, ...).
    #[error("network error for {url}: {message}")]
    Network { url: String, message: String },
    /// A page or JSON answer did not have the expected shape.
    #[error("could not understand the Workshop answer ({what}): {detail}")]
    Parse { what: &'static str, detail: String },
    #[error("cancelled")]
    Cancelled,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// An archive could not be read or is not supported.
    #[error("archive {path}: {detail}")]
    Archive { path: String, detail: String },
    /// An archive entry would escape the target folder, is a link, or exceeds a limit.
    #[error("unsafe archive: {0}")]
    Unsafe(String),
    #[error("install plan: {0}")]
    Plan(String),
    #[error("{0}")]
    Other(String),
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
    /// True for errors where retrying later may help.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Network { .. } => true,
            Error::Http { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }
}

impl From<kfx_formats::Error> for Error {
    fn from(e: kfx_formats::Error) -> Error {
        Error::Other(e.to_string())
    }
}

impl From<kfx_archive::Error> for Error {
    fn from(e: kfx_archive::Error) -> Error {
        match e {
            kfx_archive::Error::Io { path, source } => Error::Io { path, source },
            kfx_archive::Error::Archive { path, detail } => Error::Archive { path, detail },
            kfx_archive::Error::Unsafe(s) => Error::Unsafe(s),
        }
    }
}
