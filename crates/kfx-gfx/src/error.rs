//! Error type of the crate.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Failure reported by `kfx-formats` (I/O, RNC unpacking).
    #[error(transparent)]
    Formats(#[from] kfx_formats::Error),
    /// Malformed or unsupported art data. `what` names the format.
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
}

pub type Result<T> = std::result::Result<T, Error>;
