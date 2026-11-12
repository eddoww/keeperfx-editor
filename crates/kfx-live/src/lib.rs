//! Live link between the editor and a running KeeperFX game.
//!
//! * [`proto`]: the game's TCP JSON API, documented from the C code (read this first), typed
//!   requests, replies and events.
//! * [`client`]: a client on its own thread with channels, reconnect and timeouts.
//! * [`actions`]: requests for play-test helpers (gold, reveal, rooms, powers, camera, speed, Lua)
//!   and the honest list of what the API can and cannot do.
//! * [`logtail`]: reading Lua `print` output from the game log (thing positions).
//! * [`model`] and [`ui`] (feature `ui`): the panel state and its egui widget.
pub mod actions;
pub mod client;
pub mod logtail;
pub mod model;
pub mod proto;
#[cfg(feature = "ui")]
pub mod ui;

pub use client::{ClientError, Config, ConnState, LiveClient, LiveEvent, RequestId, Status};
pub use model::LivePanel;
pub use proto::{ApiError, GameEvent, Incoming, LevelInfo, Player, Reply, Request};
