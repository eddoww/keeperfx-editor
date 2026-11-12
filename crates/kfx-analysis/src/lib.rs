//! Map analysis and constrained random map generation for KeeperFX maps.
//!
//! A pure logic crate: it reads a [`kfx_map::mapdoc::MapDoc`] with its
//! [`kfx_map::gamedata::GameData`] and produces data, per-slab layers and RGBA images a GUI can
//! draw; the generator produces documents through the document's own mutation API.
//!
//! * [`rules`]: rule values from the layered configs (walk / dig times, block health, gold).
//! * [`terrain`]: what each slab is for movement and digging.
//! * [`sites`]: hearts, hero gates, portals, interesting things.
//! * [`nav`]: movement profiles, distance fields (Dijkstra), routes.
//! * [`reach`]: reachability tiers, per-thing reachability, isolated pockets, route contents.
//! * [`economy`]: gold, gems, portals, claimable area, pre-built rooms per keeper.
//! * [`heroes`]: hero gates and pre-placed hero groups and where they go.
//! * [`fairness`]: per-keeper table and imbalance score.
//! * [`symmetry`]: which symmetry modes a map satisfies and what breaks them.
//! * [`analysis`]: all of the above in one value, with incremental update.
//! * [`layer`]: per-slab layers with legends and the top-down renderer.
//! * [`rng`], [`generate`]: the seeded constrained map generator.
//!
//! # Model and assumptions
//! * Slab classes come from `terrain.cfg` attributes only (see [`terrain`]); columns, room
//!   furniture and floor height steps inside a slab are ignored, one cell per slab.
//! * Costs are game turns for one imp (see [`nav`] and [`rules`]): walking `768 / BaseSpeed`
//!   (8), digging `ceil(block_health / dig damage) * DIG instance Time` (dirt 16, gold 160,
//!   own wall 80), doors `Health / 2`, enemy walls (siege routes only) 4x an own wall. Many
//!   imps dig in parallel in the game, so absolute times are pessimistic; they compare
//!   players and maps.
//! * Water is walkable by everyone, lava only by flying / lava-proof creatures (or with a
//!   bridge): reachability tier "cross".
//! * First contact is the best slab where two keepers digging towards each other meet: the
//!   slower side's time there.
//!
//! # Timings (release build)
//! Measured by the tests: all 274 shipped maps (85x85 and smaller up to 121x121) take
//! 4 ms on average, 13 ms at most for the whole analysis; a generated 170x170 map with 7
//! keepers 20-28 ms (1 to 4 hero gates; Dijkstra fields 18-20 ms). [`Analysis::update`]
//! re-classifies only the edited rectangle but recomputes every field (no incremental Dijkstra): it
//! costs the same as a full run, fast enough to rerun after each edit (debounced) on any map size.
pub mod analysis;
pub mod economy;
pub mod fairness;
pub mod generate;
pub mod heroes;
pub mod layer;
pub mod nav;
pub mod reach;
pub mod rng;
pub mod rules;
pub mod sites;
pub mod symmetry;
pub mod terrain;

pub use analysis::{Analysis, Options};
