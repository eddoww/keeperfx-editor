//! KeeperFX Workshop client, installer and packager.
//!
//! * [`client`]: typed, cached, rate-limited, read-only access to <https://keeperfx.net/workshop>.
//! * [`archive`], [`classify`], [`install`]: inspect a downloaded archive, plan where its content
//!   goes in a KeeperFX install, apply the plan transactionally and uninstall it again.
//! * [`package`]: gather a level or campaign into a zip laid out the way the game and the
//!   Workshop expect (offline only; uploading needs the user's own browser login).
//! * [`ui`]: a host-independent egui widget tying it together.

pub mod archive;
pub mod classify;
pub mod client;
pub mod error;
pub mod html;
pub mod install;
pub mod model;
pub mod package;
pub mod ui;
pub mod util;

pub use error::{Error, Result};
