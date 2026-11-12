//! Error type of the map crate.
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Format(#[from] kfx_formats::Error),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: TOML: {msg}")]
    Toml { path: PathBuf, msg: String },
    /// The map needs more distinct columns than the game's column pool holds.
    #[error(
        "column pool overflow: {needed} columns needed (map {map} + game-created {game}), \
        limit {limit}"
    )]
    PoolOverflow {
        needed: usize,
        map: usize,
        game: usize,
        limit: usize,
    },
    #[error("{0}")]
    Invalid(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
