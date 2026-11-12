//! Shared error type.
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Malformed or unsupported data. `what` names the format ("slb", "rnc", "tngfx", ...).
    #[error("{what}: {msg}")]
    Format { what: &'static str, msg: String },
}

impl Error {
    pub fn format(what: &'static str, msg: impl Into<String>) -> Self {
        Error::Format {
            what,
            msg: msg.into(),
        }
    }
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
