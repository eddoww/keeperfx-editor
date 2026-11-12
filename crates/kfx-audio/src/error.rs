//! Error type of the crate.
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Malformed data.
    #[error("{what}: {msg}")]
    Format { what: &'static str, msg: String },
    /// Well formed but a codec / feature this crate does not decode.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A name, id or file that does not resolve.
    #[error("not found: {0}")]
    NotFound(String),
    /// The audio output failed.
    #[error("playback: {0}")]
    Playback(String),
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

impl From<kfx_formats::Error> for Error {
    fn from(e: kfx_formats::Error) -> Self {
        match e {
            kfx_formats::Error::Io { path, source } => Error::Io { path, source },
            kfx_formats::Error::Format { what, msg } => Error::Format { what, msg },
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
