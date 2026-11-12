//! Self-update of the KeeperFX map editor from the releases of its GitHub repository.
//!
//! No GUI here: blocking I/O meant to run on a worker thread ([`updater::Worker`]), and pure
//! logic that is tested on every platform.
//!
//! * [`client`]: the release list (`GET /repos/{owner}/{repo}/releases`) with ETag, daily
//!   interval, rate-limit and back-off handling ([`state`]); resumable, cancellable downloads.
//! * [`release`]: versions, channels and the choice of the update; [`platform`]: the asset for
//!   the running OS and processor.
//! * [`verify`]: `SHA256SUMS.txt`, SHA-256 of downloads and the [`verify::Verifier`] hook for a
//!   future signature check (the trust model is described there).
//! * [`install`]: the install layout (portable folder, macOS bundle, Windows installer), when
//!   self-update must not be attempted, the atomic swap with rollback, and the health record of
//!   a freshly installed version.
//! * [`transport`]: the wire, HTTPS only and pinned to GitHub's hosts ([`transport::UrlPolicy`]);
//!   tests inject a plain-HTTP policy for a local mock server.
//!
//! Archives are unpacked with the hardened extraction of `kfx-archive`.

pub mod client;
pub mod error;
pub mod install;
pub mod platform;
pub mod release;
pub mod state;
pub mod transport;
pub mod updater;
pub mod verify;

pub use error::{Error, Result};
pub use release::{Asset, Channel, Release};
