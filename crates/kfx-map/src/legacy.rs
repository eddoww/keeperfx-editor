//! Output in the original file formats: `.tng`, `.apt`, `.lgt` instead of `.tngfx`, `.aptfx`,
//! `.lgtfx`, for people who need maps the original game (or tools that only know the legacy files)
//! can open.
//!
//! The game picks the format by the presence of `.lgtfx` (`load_level_file`,
//! src/lvl_filesdk1.c:1440-1466), so a legacy save writes the three legacy files and removes the three
//! FX files ([`MapDoc::save_legacy`]).
//!
//! # What the legacy files cannot hold
//! Errors (nothing is written; [`LegacyReport::errors`]):
//! * map size other than 85x85: positions are `u16` 8.8 subtile coordinates (`struct
//!   LegacyInitThing`, src/lvl_filesdk1.c:89; the high byte is the subtile so 255 subtiles = 85 slabs
//!   at most) and the size is not stored in any legacy file (a map without `.lof` is
//!   `DEFAULT_MAP_SIZE` = 85, see `kfx_formats::campaign`).
//! * thing class, model or owner above 255 (one byte each), or without a number (only a
//!   `SubtypeName`), owner above 5 (the legacy loader discards the thing, src/thing_factory.c:135),
//!   coordinates beyond 16 bits, hero gate / custom box number above 255, action point number above
//!   65535, light intensity above 255.
//! * more things / action points / lights than the game loads (`THINGS_COUNT - 2`, 255,
//!   2047: src/lvl_filesdk1.c:756, 886, 1263; see [`crate::validate`]).
//!
//! Warnings (the map is written, but information is dropped): extended creature fields
//! (`CreatureName`, `CreatureGold`, `CreatureInitialHealth`; the legacy record has only the level
//! in `params[1]`, `thing_create_thing` src/thing_factory.c:99-190), `Orientation` (only the FX loader
//! reads it, src/thing_factory.c:301), `GoldValue` of gold piles (src/thing_factory.c:285), unknown extra
//! keys, more than 2048 columns (the size of the column table of the original Dungeon Keeper),
//! custom slabs (the sidecar is still written; the legacy files just see their columns and
//! `recognized_as` kind).
//!
//! # Bytes the game ignores
//! A legacy record holds bytes the game never reads for its class (`range` of an object,
//! `params` of a trap, a light's `flags`, ...). Loading a legacy map keeps them in the thing's
//! extra keys ([`LEGACY_RANGE_KEY`] and friends, not written to FX files), and
//! [`MapDoc::save_legacy`] writes them back, so an unedited record round-trips byte for byte.
//!
//! # Order
//! Records are written in the order of the document's lists, which is the order they were loaded
//! in; a legacy file loaded and saved again keeps its record order (items the editor removed or
//! added aside).
use crate::column::ColumnPool;
use crate::gamedata::GameData;
use crate::mapdoc::{MapDoc, SaveColumns};
use crate::things::{class, thing_class};
use crate::validate::{MAX_ACTION_POINTS, MAX_LIGHTS, MAX_MAP_THINGS};
use crate::{Error, Result};
use kfx_formats::atomic::SaveTransaction;
use kfx_formats::level_bin::{self, ActionPoint, Apt, Lgt, Light, Thing, Tng};
use std::collections::BTreeMap;
use std::path::Path;

/// Extra key under which a thing loaded from a legacy file keeps the bytes the game never reads
/// for its class (`range`; see [`unused_legacy_bytes`]), so a legacy save writes the record back
/// unchanged. The FX writer drops these keys.
pub const LEGACY_RANGE_KEY: &str = "LegacyRange";
/// Same for `params`: eight integers, only the unused positions are non-zero.
pub const LEGACY_PARAMS_KEY: &str = "LegacyParams";
/// Light: `flags` (the TOML loader ignores it, src/light_data.c:238) and the five unused fields
/// (`unused[0..3]`, `unused_10`, `[u16; 3]` + `u8`) as `[flags, u0, u1, u2, u10]`.
pub const LEGACY_LIGHT_KEY: &str = "LegacyLight";

/// Highest owner the legacy `.tng` loader keeps (`itng->owner > 5` is discarded,
/// `thing_create_thing` src/thing_factory.c:135).
pub const LEGACY_MAX_OWNER: i64 = 5;

/// Whether an extra key is one of the legacy residue keys.
pub fn is_legacy_key(k: &str) -> bool {
    k == LEGACY_RANGE_KEY || k == LEGACY_PARAMS_KEY || k == LEGACY_LIGHT_KEY
}

/// Which `range` / `params` bytes of a legacy record the game reads for `class`
/// (`thing_create_thing`, src/thing_factory.c:99-190): `(range used, params used)`. Object: `params[1]`
/// for hero gates and custom boxes only; creature: `params[1]`; effect generator: `range`; trap:
/// none; door: `params[0..2]`.
fn used_bytes(class: u8, object_param1: bool) -> (bool, [bool; 8]) {
    let mut p = [false; 8];
    match class {
        1 => p[1] = object_param1,
        5 => p[1] = true,
        9 => {
            p[0] = true;
            p[1] = true;
        }
        _ => {}
    }
    (class == 7, p)
}

/// The bytes of a legacy thing record the game does not read for its class: `(range, params)`
/// with the used positions zeroed.
pub fn unused_legacy_bytes(t: &Thing, object_param1: bool) -> (u16, [u8; 8]) {
    let (range_used, used) = used_bytes(t.class, object_param1);
    let mut p = t.params;
    for i in 0..8 {
        if used[i] {
            p[i] = 0;
        }
    }
    (if range_used { 0 } else { t.range }, p)
}

/// Store the unused bytes of legacy record `t` in `item.extra` (nothing when they are zero).
pub fn keep_legacy_residue(item: &mut kfx_formats::level_fx::FxThing, t: &Thing) {
    let (range, params) = unused_legacy_bytes(
        t,
        item.herogate_number.is_some() || item.custom_box.is_some(),
    );
    if range != 0 {
        item.extra
            .insert(LEGACY_RANGE_KEY.into(), toml::Value::Integer(range as i64));
    }
    if params.iter().any(|&b| b != 0) {
        item.extra.insert(
            LEGACY_PARAMS_KEY.into(),
            toml::Value::Array(
                params
                    .iter()
                    .map(|&b| toml::Value::Integer(b as i64))
                    .collect(),
            ),
        );
    }
}

/// Same for a light.
pub fn keep_light_residue(item: &mut kfx_formats::level_fx::FxLight, l: &Light) {
    let v = [
        l.flags as u16,
        l.unused[0],
        l.unused[1],
        l.unused[2],
        l.unused_10 as u16,
    ];
    if v.iter().any(|&b| b != 0) {
        item.extra.insert(
            LEGACY_LIGHT_KEY.into(),
            toml::Value::Array(v.iter().map(|&b| toml::Value::Integer(b as i64)).collect()),
        );
    }
}

fn ints(v: &toml::Value) -> Vec<i64> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_integer()).collect())
        .unwrap_or_default()
}

/// The map size the legacy format can express.
pub const LEGACY_MAP_SLABS: u16 = 85;
/// Column table size of the original game.
pub const LEGACY_COLUMN_LIMIT: usize = 2048;

/// What a legacy save loses or cannot do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyReport {
    /// The save is refused.
    pub errors: Vec<String>,
    /// Data dropped, the save goes ahead.
    pub warnings: Vec<String>,
}

fn add(m: &mut BTreeMap<&'static str, (usize, String)>, code: &'static str, what: String) {
    let e = m.entry(code).or_insert((0, what));
    e.0 += 1;
}

/// A thing as a legacy record plus the reasons it cannot be one.
fn thing_record(t: &kfx_formats::level_fx::FxThing) -> std::result::Result<Thing, &'static str> {
    let cls = t
        .class
        .as_ref()
        .and_then(|c| c.id())
        .ok_or("thing without a class number")?;
    if u8::try_from(cls).is_err() {
        return Err("thing class above 255");
    }
    match t.subtype {
        None => return Err("thing without a numeric model (only a SubtypeName)"),
        Some(m) if !(0..=255).contains(&m) => return Err("model above 255"),
        _ => {}
    }
    if !t.owner.is_some_and(|o| (0..=255).contains(&o)) {
        return Err("owner missing or above 255");
    }
    if t.owner.is_some_and(|o| o > LEGACY_MAX_OWNER) {
        // thing_create_thing, src/thing_factory.c:135
        return Err("owner above 5: the legacy loader discards such things");
    }
    if t.herogate_number
        .or(t.custom_box)
        .is_some_and(|n| !(0..=255).contains(&n))
    {
        return Err("hero gate / custom box number above 255");
    }
    let l = t
        .to_legacy()
        .ok_or("coordinate or index does not fit the legacy fields")?;
    let mut r = Thing {
        x: l.x,
        y: l.y,
        z: l.z,
        class: l.oclass,
        model: l.model,
        owner: l.owner,
        range: l.range,
        index: l.index,
        params: l.params,
    };
    // the bytes the game ignores come back as they were loaded
    let (range_used, used) = used_bytes(
        r.class,
        t.herogate_number.is_some() || t.custom_box.is_some(),
    );
    if let (false, Some(v)) = (
        range_used,
        t.extra.get(LEGACY_RANGE_KEY).and_then(|v| v.as_integer()),
    ) {
        r.range = v as u16;
    }
    if let Some(v) = t.extra.get(LEGACY_PARAMS_KEY) {
        for (i, b) in ints(v).into_iter().take(8).enumerate() {
            if !used[i] {
                r.params[i] = b as u8;
            }
        }
    }
    Ok(r)
}

impl MapDoc {
    /// The legacy `.tng` / `.apt` / `.lgt` of this document, and what cannot be expressed.
    pub fn legacy_files(&self, gd: &GameData) -> (Tng, Apt, Lgt, LegacyReport) {
        let mut rep = LegacyReport::default();
        let d = self.dims;
        if d.w != LEGACY_MAP_SLABS || d.h != LEGACY_MAP_SLABS {
            rep.errors.push(format!(
                "the map is {}x{} slabs; legacy files can only describe \
                {LEGACY_MAP_SLABS}x{LEGACY_MAP_SLABS} (8.8 subtile coordinates in one byte, no \
                size field)",
                d.w, d.h
            ));
        }
        let mut errs: BTreeMap<&'static str, (usize, String)> = BTreeMap::new();
        let mut warns: BTreeMap<&'static str, (usize, String)> = BTreeMap::new();
        let mut tng = Tng {
            things: vec![],
            declared_count: None,
            trailing: vec![],
        };
        for (i, t) in self.things.items.iter().enumerate() {
            match thing_record(t) {
                Ok(r) => tng.things.push(r),
                Err(why) => add(&mut errs, why, format!("thing {i}: {why}")),
            }
            let cls = thing_class(t);
            if cls == Some(class::CREATURE)
                && (t.creature_name.is_some()
                    || t.creature_gold.is_some()
                    || t.creature_initial_health.is_some())
            {
                add(
                    &mut warns,
                    "creature-extended",
                    format!(
                        "creature {i}: CreatureName / \
                    CreatureGold / CreatureInitialHealth are dropped"
                    ),
                );
            }
            if t.orientation.is_some() && cls != Some(class::DOOR) {
                add(
                    &mut warns,
                    "orientation",
                    format!("thing {i}: Orientation is dropped"),
                );
            }
            if t.gold_value.is_some() {
                add(
                    &mut warns,
                    "gold-value",
                    format!("thing {i}: GoldValue is dropped"),
                );
            }
            if t.extra.keys().any(|k| !is_legacy_key(k)) {
                add(
                    &mut warns,
                    "thing-extra",
                    format!(
                        "thing {i}: unknown keys {:?} are dropped",
                        t.extra
                            .keys()
                            .filter(|k| !is_legacy_key(k))
                            .collect::<Vec<_>>()
                    ),
                );
            }
        }
        let mut apt = Apt {
            points: vec![],
            declared_count: None,
            trailing: vec![],
        };
        for (i, a) in self.action_points.items.iter().enumerate() {
            match a.to_legacy() {
                Some(l) => apt.points.push(ActionPoint {
                    x: l.x,
                    y: l.y,
                    range: l.range,
                    number: l.num,
                }),
                None => add(
                    &mut errs,
                    "action-point",
                    format!(
                        "action point {i}: number or coordinate does not fit the legacy fields"
                    ),
                ),
            }
            if !a.extra.is_empty() {
                add(
                    &mut warns,
                    "action-point-extra",
                    format!("action point {i}: unknown keys are dropped"),
                );
            }
        }
        let mut lgt = Lgt {
            lights: vec![],
            declared_count: None,
            trailing: vec![],
        };
        for (i, l) in self.lights.items.iter().enumerate() {
            match l.to_legacy() {
                Some(r) => lgt.lights.push({
                    let k = l.extra.get(LEGACY_LIGHT_KEY).map(ints).unwrap_or_default();
                    let g = |i: usize| k.get(i).copied().unwrap_or(0) as u16;
                    Light {
                        radius: r.radius,
                        intensity: r.intensity,
                        flags: g(0) as u8,
                        unused: [g(1), g(2), g(3)],
                        x: r.x,
                        y: r.y,
                        z: r.z,
                        unused_10: g(4) as u8,
                        is_dynamic: r.is_dynamic,
                        attached_slb: r.attached_slb,
                    }
                }),
                None => add(
                    &mut errs,
                    "light",
                    format!(
                        "light {i}: intensity above 255 or \
                    coordinate does not fit the legacy fields"
                    ),
                ),
            }
            if l.extra.keys().any(|k| !is_legacy_key(k)) {
                add(
                    &mut warns,
                    "light-extra",
                    format!("light {i}: unknown keys are dropped"),
                );
            }
        }
        if self.things.items.len() > MAX_MAP_THINGS {
            rep.errors.push(format!(
                "{} things; the game loads at most {MAX_MAP_THINGS}",
                self.things.items.len()
            ));
        }
        if self.action_points.items.len() > MAX_ACTION_POINTS {
            rep.errors.push(format!(
                "{} action points; the game loads at most {MAX_ACTION_POINTS}",
                self.action_points.items.len()
            ));
        }
        if self.lights.items.len() > MAX_LIGHTS {
            rep.errors.push(format!(
                "{} lights; the game loads at most {MAX_LIGHTS}",
                self.lights.items.len()
            ));
        }
        for (_, (n, first)) in errs {
            rep.errors.push(if n > 1 {
                format!("{first} (and {} more of this kind)", n - 1)
            } else {
                first
            });
        }
        for (_, (n, first)) in warns {
            rep.warnings.push(if n > 1 {
                format!("{first} (and {} more of this kind)", n - 1)
            } else {
                first
            });
        }
        if let Ok((clm, _)) = ColumnPool::build(gd, d, &self.columns) {
            if clm.columns.len() > LEGACY_COLUMN_LIMIT {
                rep.warnings.push(format!(
                    "{} columns; the original game's column table holds {LEGACY_COLUMN_LIMIT}",
                    clm.columns.len()
                ));
            }
        }
        if self.custom_cells.iter().any(|&c| c != 0) {
            rep.warnings.push(
                "custom slabs: legacy files only see their columns and recognised \
                kind; the definitions stay in the sidecar"
                    .into(),
            );
        }
        (tng, apt, lgt, rep)
    }

    /// What [`MapDoc::save_legacy`] would complain about.
    pub fn legacy_report(&self, gd: &GameData) -> LegacyReport {
        self.legacy_files(gd).3
    }

    /// Write the map in the original formats: everything [`MapDoc::save`] writes except the FX
    /// files, plus `.tng .apt .lgt`; the stale `.lgtfx` (which switches the game to the FX files),
    /// then `.tngfx .aptfx` are removed after all writes. Nothing is written when the report has
    /// errors (the returned error lists them); otherwise the report with its warnings is returned.
    /// All or nothing, like [`MapDoc::save`].
    pub fn save_legacy(
        &self,
        gd: &GameData,
        folder: &Path,
        which: SaveColumns,
    ) -> Result<LegacyReport> {
        let (tx, report) = self.save_legacy_transaction(gd, folder, which)?;
        tx.commit()?;
        Ok(report)
    }

    /// The files [`MapDoc::save_legacy`] writes and removes, not yet applied.
    pub fn save_legacy_transaction(
        &self,
        gd: &GameData,
        folder: &Path,
        which: SaveColumns,
    ) -> Result<(SaveTransaction, LegacyReport)> {
        self.check_writable()?;
        let (tng, apt, lgt, report) = self.legacy_files(gd);
        if !report.errors.is_empty() {
            return Err(Error::Invalid(format!(
                "cannot save in the legacy format: {}",
                report.errors.join("; ")
            )));
        }
        let (mut bin, any_slx) = self.build_bin(gd, which)?;
        bin.tng = Some(tng);
        bin.apt = Some(apt);
        bin.lgt = Some(lgt);
        let mut tx = SaveTransaction::new(folder);
        for (ext, bytes) in bin.files() {
            tx.write(level_bin::file_name(self.level, ext), bytes);
        }
        self.sidecar_ops(&mut tx);
        for ext in ["lgtfx", "tngfx", "aptfx"] {
            tx.remove(level_bin::file_name(self.level, ext));
        }
        if !any_slx {
            tx.remove(level_bin::file_name(self.level, "slx"));
        }
        Ok((tx, report))
    }
}
