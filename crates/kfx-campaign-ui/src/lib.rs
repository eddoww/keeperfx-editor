//! Campaign and map pack editor for KeeperFX.
//!
//! * [`pack`]: the model ([`Pack`]) and its operations: level lists, bonus links, adding existing
//!   levels (with renumbering), level info, settings.
//! * [`files`]: create / rename / duplicate / package.
//! * [`texts`]: the `[strings]` / `[speech]` language tables.
//! * [`validate`]: every rule with the KeeperFX C source it comes from, and the safe fixes.
//! * [`landview`]: decoding of land view pictures.
//! * [`ui`]: the egui widget ([`CampaignEditor`]); no windowing or rendering backend.
pub mod files;
pub mod landview;
pub mod pack;
pub mod texts;
pub mod ui;
pub mod validate;

pub use files::{
    create_pack, duplicate_pack, install_levels, package, rename_pack, NewPack, PackageOptions,
    PackageReport, SourceEntry,
};
pub use pack::{AddLevel, Added, LevelEntry, LevelSource, Pack, SingleSlot};
pub use texts::Texts;
pub use ui::{CampaignEditor, Event};
pub use validate::{validate, Fix, Issue, Severity, Target};
