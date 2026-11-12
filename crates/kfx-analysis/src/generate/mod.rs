//! Seeded, constrained random map generator.
//!
//! [`generate`] makes a whole map, [`generate_patch`] fills a rectangle of an existing one. A
//! map is first laid out as a [`Plan`] (slab kinds and owners, doors, things), checked with the
//! analyses of this crate ([`check_plan`]) and repaired (dig corridors, extra gold) or re-rolled
//! until the hard constraints hold, then turned into a [`MapDoc`] through the document API and
//! checked again (validator + analyses).
//!
//! # Hard constraints (per keeper)
//! * a heart in a claimed start area (by construction: 3x3 heart room in a 7x7 claimed square);
//! * a portal its imps can reach by digging;
//! * at least [`GenParams::min_gold`] gold reachable and closer to it than to anyone else;
//! * a dig route to every other keeper and, with heroes, to at least one hero gate;
//! * with two or more keepers, a fairness score ([`crate::fairness`]) at most
//!   [`GenParams::max_imbalance`];
//! * no validator error (`kfx_map::validate`).
//!
//! # Determinism
//! All randomness comes from [`crate::rng::Rng`] (SplitMix64) seeded from [`GenParams::seed`]
//! and the attempt number; noise is integer value noise; angles use [`crate::rng::cos_sin`], a
//! fixed polynomial; positions are computed with `+ - * / sqrt` only (IEEE 754 exact) and rounded
//! with [`crate::rng::round`]. No hash maps are iterated. So a seed gives the same map on every
//! platform.
//!
//! # Symmetry
//! With an exact [`SymmetryMode`], keeper bases are images of seed bases and every feature is
//! stamped at all its images; a final pass copies every non-keeper slab from its orbit
//! representative, so the terrain is exactly symmetric. `Radial` places `n` keepers by rotations of
//! `1/n` turn with the noise rotated too (close to, but not exactly, symmetric: the grid does not
//! rotate by thirds); the fairness check guards it.
//!
//! # Building the document
//! The plan is written into a new [`MapDoc`] slab layer (kinds, owners, `.wlb` like
//! [`MapDoc::set_slab`]), earth next to ground / rooms gets the game's torch-dirt rule
//! (`alter_rock_style`, src/map_blocks.c:1613, as [`MapDoc::place_slab`] does), then the columns are
//! regenerated (decorated with [`Decorator`] when asked), the slab objects (heart, torches, room
//! furniture) are created with [`MapDoc::apply_slab_objects`], doors are placed with
//! [`MapDoc::place_door`] and things with [`MapDoc::add_thing`]. The plan is written in one go
//! rather than with [`MapDoc::place_slabs`]: that applies the torch-dirt rule slab by slab in
//! placement order and regenerates undecorated 3x3 blocks after each slab, which the decorated
//! whole-rect regeneration here would overwrite anyway, and the one-pass torch rule over the
//! finished layout does not depend on an order.
use crate::analysis::{Analysis, Options};
use crate::rng::{cos_sin, fractal, round, Rng};
use crate::rules::Rules;
use crate::sites::{
    self, creature_score, Gate, Heart, Sites, ThingKind, ThingSite, HEROES, KEEPERS,
};
use crate::terrain::{Class, KindTable, Terrain};
use kfx_formats::level_bin::MapDims;
use kfx_map::autotile::{self, kinds, SlabRect, PLAYER_NEUTRAL};
use kfx_map::decorate::Decorator;
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_map::patch::Patch;
use kfx_map::things::{self as kthings, class};
use kfx_map::transform::{SymmetryMode, Transform};
use kfx_map::validate::{validate, Severity};
use std::path::PathBuf;

mod checks;
mod document;
mod features;
mod layout;
mod plan;

pub use checks::*;
pub use document::*;
use features::*;
use layout::*;
pub use plan::*;

/// Map style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    /// Earth with rock outcrops, gold veins, pools and open caverns.
    OpenCave,
    /// Rock-heavy, with pre-dug chambers joined by corridors.
    Chambers,
    /// Keepers on islands in a lava or water sea, joined by earth causeways.
    Islands,
    /// Rivers of lava or water between the keepers, with fords.
    River,
    /// A walled hero stronghold in the centre.
    Fortress,
}

pub const ALL_STYLES: [Style; 5] = [
    Style::OpenCave,
    Style::Chambers,
    Style::Islands,
    Style::River,
    Style::Fortress,
];

/// Symmetry wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymmetryChoice {
    /// The best exact mode for the keeper count (2: half turn, 4: quarter turn on square maps
    /// else four-way), else radial (3, 5, 6, 7: an exact mode would leave keepers of one orbit
    /// in different surroundings).
    Auto,
    /// No symmetry.
    None,
    /// Keepers and noise rotated by `1/n` turn (approximate).
    Radial,
    /// An exact mode; the keeper count must be a multiple of its orbit size (2 or 4).
    Mode(SymmetryMode),
}

/// Generator parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct GenParams {
    pub seed: u64,
    pub width: u16,
    pub height: u16,
    pub style: Style,
    /// Keepers 0..=7 (owners in [`KEEPERS`] order).
    pub players: u8,
    pub symmetry: SymmetryChoice,
    /// Gold amount 0..=100.
    pub richness: u8,
    /// Water and lava amounts 0..=100.
    pub water: u8,
    pub lava: u8,
    /// Hero gates and (fortress) hero garrison.
    pub heroes: bool,
    /// Hero gates wanted (0 = one orbit of the map centre).
    pub hero_gates: u8,
    /// Gem clusters.
    pub gems: u8,
    /// Neutral chambers (with a neutral room in every other one).
    pub neutral_rooms: u8,
    /// Neutral creatures spread over the chambers.
    pub neutral_creatures: u8,
    /// Special boxes spread over the chambers.
    pub special_boxes: u8,
    /// Imps per keeper.
    pub imps: u8,
    /// Minimum gold per keeper (0 = from the richness).
    pub min_gold: u64,
    pub max_imbalance: f32,
    pub max_attempts: u32,
    /// Randomise column decoration (see [`Decorator`]).
    pub decorate: bool,
    /// Folder and level number of the new document.
    pub folder: PathBuf,
    pub level: u32,
}

impl Default for GenParams {
    fn default() -> Self {
        GenParams {
            seed: 1,
            width: 85,
            height: 85,
            style: Style::OpenCave,
            players: 2,
            symmetry: SymmetryChoice::Auto,
            richness: 50,
            water: 20,
            lava: 20,
            heroes: true,
            hero_gates: 0,
            gems: 2,
            neutral_rooms: 2,
            neutral_creatures: 4,
            special_boxes: 2,
            imps: 4,
            min_gold: 0,
            max_imbalance: 0.25,
            max_attempts: 8,
            decorate: true,
            folder: PathBuf::from("."),
            level: 1,
        }
    }
}

impl GenParams {
    /// The minimum gold per keeper in effect.
    pub fn min_gold_value(&self) -> u64 {
        if self.min_gold > 0 {
            self.min_gold
        } else {
            4096 + self.richness.min(100) as u64 * 80
        }
    }
}

/// One player of the starter script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerSetup {
    pub owner: u8,
    /// `PLAYER0`.. `PLAYER6` (owners 6..8 are `PLAYER4`..`PLAYER6`).
    pub script_name: String,
    pub start_money: u32,
    pub max_creatures: u32,
    /// Computer player personality for all but the first keeper.
    pub computer: Option<u8>,
}

/// Starter script values, plain data for the host to turn into a script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StarterScript {
    pub generate_speed: u32,
    pub players: Vec<PlayerSetup>,
    /// `(creature, count)` for `ADD_CREATURE_TO_POOL`, all available.
    pub creature_pool: Vec<(String, u32)>,
    /// `(room, available now)`.
    pub rooms: Vec<(String, bool)>,
    /// `(power, available now)`.
    pub powers: Vec<(String, bool)>,
    /// Hero gate numbers on the map.
    pub hero_gates: Vec<i64>,
}

/// Script name of an owner.
pub fn script_player(o: u8) -> &'static str {
    match o {
        0 => "PLAYER0",
        1 => "PLAYER1",
        2 => "PLAYER2",
        3 => "PLAYER3",
        4 => "PLAYER_GOOD",
        6 => "PLAYER4",
        7 => "PLAYER5",
        8 => "PLAYER6",
        _ => "PLAYER_NEUTRAL",
    }
}

/// A constraint check result.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// What happened while generating.
#[derive(Clone, Debug, PartialEq)]
pub struct GenReport {
    pub attempts: u32,
    pub symmetry: Option<SymmetryMode>,
    pub radial: bool,
    pub checks: Vec<Check>,
    pub repairs: Vec<String>,
    pub imbalance: Option<f32>,
    pub warnings: Vec<String>,
    /// Microseconds.
    pub time_us: u64,
}

/// Why no map came out.
#[derive(Clone, Debug, PartialEq)]
pub struct GenFailure {
    pub reason: String,
    pub attempts: u32,
    /// Failed checks of every attempt (`attempt: check: detail`).
    pub failures: Vec<String>,
}

impl std::fmt::Display for GenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} after {} attempt(s)", self.reason, self.attempts)?;
        for l in self.failures.iter().take(12) {
            write!(f, "\n  {l}")?;
        }
        Ok(())
    }
}

/// A generated map.
#[derive(Clone, Debug)]
pub struct Generated {
    pub doc: MapDoc,
    pub plan: Plan,
    pub script: StarterScript,
    pub report: GenReport,
}

fn us(t: std::time::Instant) -> u64 {
    t.elapsed().as_micros() as u64
}

/// Generate a whole map (see the module docs).
pub fn generate(gd: &GameData, p: &GenParams) -> Result<Generated, GenFailure> {
    let t0 = std::time::Instant::now();
    let fail = |reason: String, attempts: u32, failures: Vec<String>| GenFailure {
        reason,
        attempts,
        failures,
    };
    if p.players as usize > KEEPERS.len() {
        return Err(fail(
            format!("{} keepers asked, at most {}", p.players, KEEPERS.len()),
            0,
            vec![],
        ));
    }
    let (w, h) = (
        p.width.clamp(20, kfx_map::transform::MAX_MAP_SLABS) as i32,
        p.height.clamp(20, kfx_map::transform::MAX_MAP_SLABS) as i32,
    );
    let ctx = Ctx {
        gd,
        k: Kinds::new(gd),
        p,
        border: true,
    };
    let rules = Rules::from_game(gd);
    let table = KindTable::new(gd, &rules);
    let mut failures = vec![];
    for attempt in 0..p.max_attempts.max(1) {
        let seed = crate::rng::mix(p.seed ^ (attempt as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut plan = match build_plan(&ctx, w, h, seed) {
            Ok(pl) => pl,
            Err(e) => return Err(fail(e, attempt + 1, failures)),
        };
        let mut repairs = vec![];
        let mut checks = vec![];
        for round in 0..4 {
            let (a, c) = check_plan(&plan, gd, &rules, &table, p);
            checks = c;
            if checks.iter().all(|c| c.ok) || round == 3 {
                break;
            }
            let done = repair(&mut plan, &ctx.k, &a, &checks, p, true);
            if done.is_empty() {
                break;
            }
            repairs.extend(done);
        }
        if checks.iter().any(|c| !c.ok) {
            for c in checks.iter().filter(|c| !c.ok) {
                failures.push(format!("attempt {attempt}: {}: {}", c.name, c.detail));
            }
            continue;
        }
        // the document
        let dims = MapDims::new(w as u16, h as u16);
        let mut doc = MapDoc::new(&p.folder, p.level, dims, ctx.k.rock, gd);
        let mut dec = Decorator::new(gd, seed);
        dec.enabled = p.decorate;
        write_plan(&mut doc, gd, &plan, 0, 0, &dec);
        let issues = validate(&doc, gd);
        let errors: Vec<String> = issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{} {}", i.code, i.message))
            .collect();
        let warnings: Vec<String> = issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .map(|i| format!("{} {}", i.code, i.message))
            .collect();
        if !errors.is_empty() {
            for e in errors {
                failures.push(format!("attempt {attempt}: validator: {e}"));
            }
            continue;
        }
        // final check on the document itself
        let a = Analysis::of_doc_with(
            &doc,
            gd,
            rules.clone(),
            table.clone(),
            Options {
                symmetry: plan.sym.mode.is_some(),
                heroes: true,
            },
        );
        let final_checks = constraint_checks(&a, p);
        if let Some(c) = final_checks.iter().find(|c| !c.ok) {
            failures.push(format!(
                "attempt {attempt}: document: {}: {}",
                c.name, c.detail
            ));
            continue;
        }
        let mut warn = warnings;
        if let Some(m) = plan.sym.mode {
            if !a.symmetry.iter().any(|c| c.mode == m && c.terrain_exact()) {
                warn.push(format!("terrain is not exactly {m:?} symmetric"));
            }
        }
        let script = starter_script(gd, &plan);
        let report = GenReport {
            attempts: attempt + 1,
            symmetry: plan.sym.mode,
            radial: plan.sym.radial > 1,
            checks: final_checks,
            repairs,
            imbalance: a.fairness.as_ref().map(|f| f.score),
            warnings: warn,
            time_us: us(t0),
        };
        return Ok(Generated {
            doc,
            plan,
            script,
            report,
        });
    }
    Err(fail(
        "no attempt met every constraint".into(),
        p.max_attempts.max(1),
        failures,
    ))
}

/// Fill rectangle `rect` of `doc` with generated terrain (and keepers when `p.players > 0`),
/// as one undoable [`Patch`]. Things standing in the rectangle are removed first. Keeper
/// constraints are checked on the rectangle's own plan; the patch is refused when the document gets
/// new validator errors.
pub fn generate_patch(
    doc: &mut MapDoc,
    gd: &GameData,
    rect: SlabRect,
    p: &GenParams,
) -> Result<(Patch, GenReport), GenFailure> {
    let t0 = std::time::Instant::now();
    let rect = rect.clip(doc.dims);
    let (w, h) = (rect.x1 - rect.x0 + 1, rect.y1 - rect.y0 + 1);
    if w < 8 || h < 8 {
        return Err(GenFailure {
            reason: format!("rectangle {w}x{h} too small (8x8 minimum)"),
            attempts: 0,
            failures: vec![],
        });
    }
    let ctx = Ctx {
        gd,
        k: Kinds::new(gd),
        p,
        border: false,
    };
    let rules = Rules::from_game(gd);
    let table = KindTable::new(gd, &rules);
    let errors_before = validate(doc, gd)
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let mut failures = vec![];
    for attempt in 0..p.max_attempts.max(1) {
        let seed = crate::rng::mix(p.seed ^ (attempt as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut plan = match build_plan(&ctx, w, h, seed) {
            Ok(pl) => pl,
            Err(e) => {
                return Err(GenFailure {
                    reason: e,
                    attempts: attempt + 1,
                    failures,
                })
            }
        };
        // the rectangle's edge is not the map border: keep the generated terrain there
        let checks = if p.players > 0 {
            let (a, c) = check_plan(&plan, gd, &rules, &table, p);
            repair(&mut plan, &ctx.k, &a, &c, p, false);
            check_plan(&plan, gd, &rules, &table, p).1
        } else {
            vec![]
        };
        if let Some(c) = checks.iter().find(|c| !c.ok) {
            failures.push(format!("attempt {attempt}: {}: {}", c.name, c.detail));
            continue;
        }
        let mut dec = Decorator::new(gd, seed);
        dec.enabled = p.decorate;
        let mut trial = doc.clone();
        let (_, patch) = trial.record_tracked(|d| {
            d.touch_things();
            let r = kthings::StlRect::of_slabs(rect);
            let doomed: Vec<usize> = d
                .things_in(r)
                .into_iter()
                .filter(|&i| d.things.items[i].parent_tile.is_none())
                .collect();
            for i in doomed.into_iter().rev() {
                let _ = d.remove_thing(gd, i);
            }
            write_plan(d, gd, &plan, rect.x0, rect.y0, &dec);
        });
        let errors_after = validate(&trial, gd)
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count();
        if errors_after > errors_before {
            failures.push(format!(
                "attempt {attempt}: {} new validator errors",
                errors_after - errors_before
            ));
            continue;
        }
        patch.apply(doc);
        let report = GenReport {
            attempts: attempt + 1,
            symmetry: plan.sym.mode,
            radial: plan.sym.radial > 1,
            checks,
            repairs: vec![],
            imbalance: None,
            warnings: vec![],
            time_us: us(t0),
        };
        return Ok((patch, report));
    }
    Err(GenFailure {
        reason: "no attempt met every constraint".into(),
        attempts: p.max_attempts.max(1),
        failures,
    })
}
