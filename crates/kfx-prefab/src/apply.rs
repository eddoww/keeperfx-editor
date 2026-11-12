//! Applying a prefab to a document, and previewing it without touching the document.
//!
//! [`resolve`] turns the names of a prefab into the numbers of one map's configuration (slab
//! kinds, models, cubes) and reports what the configuration lacks, with the substitute / skip
//! decision taken for each missing name. [`apply`] does the edit as one [`Patch`] (undo = one
//! step): slabs through the document API (`place_slab`, custom slabs through the sidecar), then
//! the slab objects of the area (torches, furniture) re-created by `apply_slab_objects`, then
//! things (doors through `add_thing`, so the door slab, its key and the slab objects around
//! follow), action points and lights. Action points and hero gates get the smallest free
//! numbers; the script fragments come back rewritten for those numbers. [`preview`] gives the
//! same cells and icons without mutating anything, for a paste ghost.
use crate::model::*;
use crate::names::Names;
use crate::script::{self, Substitutions};
use crate::transform::Orientation;
use crate::{Error, Result};
use kfx_formats::level_bin::Column;
use kfx_formats::level_fx::{FxFlag, FxLight, StlCoord};
use kfx_map::autotile::{self, SlabRect, PLAYER_NEUTRAL};
use kfx_map::gamedata::GameData;
use kfx_map::mapdoc::MapDoc;
use kfx_map::patch::Patch;
use kfx_map::sidecar::{CustomObject, CustomObjectKind, CustomSlab};
use kfx_map::things::{self, ItemRef};

/// How the prefab's slabs combine with what is on the map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MaskMode {
    /// Every slab of the prefab (not masked) overwrites the map.
    #[default]
    Replace,
    /// Slabs of the prefab that are plain rock / earth count as empty: the map keeps what it has.
    SkipEmpty,
    /// Only land on rock / earth of the map: existing rooms, floor, liquid and doors stay.
    KeepExisting,
}

/// What to do with names the target configuration does not define.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MissingPolicy {
    /// Leave the slab / thing out.
    #[default]
    Skip,
    /// Use the built-in fallback (slab `PATH`, object `BARREL`) where there is one, else skip.
    Substitute,
}

/// What kind of name is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MissingKind {
    Slab,
    Creature,
    Object,
    Trap,
    Door,
    EffectGen,
    Cube,
}

impl MissingKind {
    pub fn label(self) -> &'static str {
        match self {
            MissingKind::Slab => "slab",
            MissingKind::Creature => "creature",
            MissingKind::Object => "object",
            MissingKind::Trap => "trap",
            MissingKind::Door => "door",
            MissingKind::EffectGen => "effect generator",
            MissingKind::Cube => "cube",
        }
    }
    fn of_class(c: ThingClass) -> MissingKind {
        match c {
            ThingClass::Object => MissingKind::Object,
            ThingClass::Creature => MissingKind::Creature,
            ThingClass::EffectGen => MissingKind::EffectGen,
            ThingClass::Trap => MissingKind::Trap,
            ThingClass::Door => MissingKind::Door,
        }
    }
}

/// An explicit substitution chosen by the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Substitution {
    pub kind: MissingKind,
    pub name: String,
    /// The name to use instead; `None` = leave it out.
    pub with: Option<String>,
}

/// Slot letter -> player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerMap {
    /// Players for slots A..H.
    pub slots: [u8; 8],
    /// The player "neutral" cells and things get.
    pub neutral: u8,
}

impl Default for OwnerMap {
    fn default() -> Self {
        OwnerMap {
            slots: [0, 1, 2, 3, 6, 7, 8, 0],
            neutral: PLAYER_NEUTRAL,
        }
    }
}

impl OwnerMap {
    /// Slot A = `a`, B = `b`..., the rest as in the default.
    pub fn with_first(players: &[u8]) -> OwnerMap {
        let mut m = OwnerMap::default();
        for (i, &p) in players.iter().take(8).enumerate() {
            m.slots[i] = p;
        }
        m
    }
    pub fn map(&self, o: Owner) -> u8 {
        match o {
            Owner::Neutral => self.neutral,
            Owner::Slot(s) => self.slots[(s as usize).min(7)],
            Owner::Fixed(n) => n,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ApplyOptions {
    pub orientation: Orientation,
    pub mask: MaskMode,
    pub owners: OwnerMap,
    pub missing: MissingPolicy,
    pub substitutions: Vec<Substitution>,
    pub things: bool,
    pub action_points: bool,
    pub lights: bool,
    /// Remove the things / action points / lights that stand on the slabs the prefab writes.
    pub clear_items: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        ApplyOptions {
            orientation: Orientation::default(),
            mask: MaskMode::Replace,
            owners: OwnerMap::default(),
            missing: MissingPolicy::Skip,
            substitutions: vec![],
            things: true,
            action_points: true,
            lights: true,
            clear_items: true,
        }
    }
}

/// What happened to a missing name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Skipped,
    Substituted(String),
}

/// A name of the prefab the target configuration does not define.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingItem {
    pub kind: MissingKind,
    pub name: String,
    /// How many slabs / things use it.
    pub count: usize,
    pub resolution: Resolution,
}

/// Whether a number is an action point or a hero gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberKind {
    ActionPoint,
    HeroGate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Renumbered {
    pub kind: NumberKind,
    pub key: String,
    /// The number the prefab's item had where it was captured.
    pub was: Option<i64>,
    pub number: i64,
}

/// Result of [`apply`] / [`resolve`].
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub missing: Vec<MissingItem>,
    pub warnings: Vec<String>,
    /// Slabs written / left alone because of the mask mode / outside the map / in a locked region.
    pub slabs_written: usize,
    pub slabs_kept: usize,
    pub slabs_outside: usize,
    pub slabs_locked: usize,
    pub things_added: usize,
    pub things_skipped: usize,
    pub things_removed: usize,
    pub action_points_added: usize,
    pub lights_added: usize,
    pub renumbered: Vec<Renumbered>,
    /// The script fragments with their placeholders filled in.
    pub script_dk: Option<String>,
    pub script_lua: Option<String>,
    /// The rectangle the prefab covers (clipped to the map).
    pub rect: Option<SlabRect>,
}

impl Report {
    /// Whether anything of the prefab was left out because a name is missing.
    pub fn has_missing(&self) -> bool {
        !self.missing.is_empty()
    }
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} slabs written ({} kept, {} outside, {} locked), {} \
                things (+{} skipped, -{} removed), {} action points, {} lights",
            self.slabs_written,
            self.slabs_kept,
            self.slabs_outside,
            self.slabs_locked,
            self.things_added,
            self.things_skipped,
            self.things_removed,
            self.action_points_added,
            self.lights_added
        );
        for m in &self.missing {
            s.push_str(&format!(
                "; missing {} {} x{}: {}",
                m.kind.label(),
                m.name,
                m.count,
                match &m.resolution {
                    Resolution::Skipped => "skipped".to_string(),
                    Resolution::Substituted(w) => format!("replaced by {w}"),
                }
            ));
        }
        s
    }
}

/// A prefab resolved against one configuration.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// The prefab after rotation / flip.
    pub prefab: Prefab,
    /// Per cell: the slab to write (`None` = masked, or missing and skipped).
    pub cells: Vec<Option<ResolvedCell>>,
    /// Per thing: the model number (`None` = skipped).
    pub models: Vec<Option<i64>>,
    /// Per custom definition: its game form (`None` = a cube or model is missing).
    pub customs: Vec<Option<CustomSlab>>,
    pub report: Report,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCell {
    pub kind: u16,
    pub owner: u8,
    pub wlb: Option<u8>,
    pub slx: u8,
    /// Index into [`Resolved::customs`].
    pub custom: Option<usize>,
}

fn fallback(kind: MissingKind) -> Option<&'static str> {
    match kind {
        MissingKind::Slab => Some("PATH"),
        MissingKind::Object => Some("BARREL"),
        _ => None,
    }
}

/// The names of `prefab` the configuration does not define, as a list to show before applying
/// (no decision is taken: every item says [`Resolution::Skipped`]).
pub fn check_requirements(prefab: &Prefab, gd: &GameData) -> Vec<MissingItem> {
    let names = Names::new(gd);
    let mut out: Vec<MissingItem> = vec![];
    let mut add = |kind: MissingKind, name: &str, n: usize| match out
        .iter_mut()
        .find(|m| m.kind == kind && m.name.eq_ignore_ascii_case(name))
    {
        Some(m) => m.count += n,
        None => out.push(MissingItem {
            kind,
            name: name.to_string(),
            count: n,
            resolution: Resolution::Skipped,
        }),
    };
    for c in prefab.cells.iter().flatten() {
        if names.slab_id(&c.slab).is_none() {
            add(MissingKind::Slab, &c.slab, 1);
        }
    }
    for t in &prefab.things {
        if !names.has_model(t.class, &t.model) {
            add(MissingKind::of_class(t.class), &t.model, 1);
        }
    }
    for d in &prefab.custom_slabs {
        if names.slab_id(&d.recognized_as).is_none() {
            add(MissingKind::Slab, &d.recognized_as, 1);
        }
        for c in &d.columns {
            for q in &c.cubes {
                if cube_id(gd, q).is_none() {
                    add(MissingKind::Cube, q, 1);
                }
            }
        }
    }
    out.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
    out
}

fn cube_id(gd: &GameData, name: &str) -> Option<u16> {
    if let Some(n) = name.strip_prefix('#') {
        return n.parse().ok();
    }
    gd.cube_id(name).map(|i| i as u16)
}

fn def_to_custom(gd: &GameData, names: &Names, d: &CustomSlabDef) -> Option<CustomSlab> {
    let kind = names.slab_id(&d.recognized_as)?;
    let mut columns = [Column::default(); 9];
    for (i, c) in d.columns.iter().enumerate() {
        let mut col = Column {
            floor_texture: c.floor,
            ..Default::default()
        };
        for (j, q) in c.cubes.iter().enumerate() {
            col.cubes[j] = cube_id(gd, q)?;
        }
        columns[i] = col;
    }
    let mut cs = CustomSlab::new(&d.name, kind, columns);
    cs.liquid = d.liquid;
    cs.wibble = d.wibble;
    for o in &d.objects {
        let kind = match o.kind.as_str() {
            "object" => CustomObjectKind::Object {
                model: names.model_id(ThingClass::Object, &o.model)? as i16,
            },
            "effectgen" => CustomObjectKind::EffectGen {
                model: names.model_id(ThingClass::EffectGen, &o.model)? as i16,
                range: o.range,
            },
            "light" => CustomObjectKind::Light {
                intensity: o.intensity,
                radius: o.range,
            },
            _ => return None,
        };
        cs.objects.push(CustomObject {
            kind,
            dx: o.dx,
            dy: o.dy,
            z: o.z,
            orientation: o.orientation,
        });
    }
    Some(cs)
}

/// Resolve `prefab` (turned / flipped by `opts.orientation`) against `gd`.
pub fn resolve(prefab: &Prefab, gd: &GameData, opts: &ApplyOptions) -> Resolved {
    let names = Names::new(gd);
    let p = prefab.transformed(opts.orientation);
    let mut report = Report::default();
    let mut missing: Vec<MissingItem> = vec![];
    let mut decide =
        |kind: MissingKind, name: &str, exists: &dyn Fn(&str) -> bool| -> Option<String> {
            // explicit choice first, then the policy
            let explicit = opts
                .substitutions
                .iter()
                .find(|s| s.kind == kind && s.name.eq_ignore_ascii_case(name));
            let with = match explicit {
                Some(s) => s.with.clone().filter(|w| exists(w)),
                None if opts.missing == MissingPolicy::Substitute => {
                    fallback(kind).filter(|w| exists(w)).map(str::to_string)
                }
                None => None,
            };
            let res = match &with {
                Some(w) => Resolution::Substituted(w.clone()),
                None => Resolution::Skipped,
            };
            match missing
                .iter_mut()
                .find(|m| m.kind == kind && m.name.eq_ignore_ascii_case(name))
            {
                Some(m) => m.count += 1,
                None => missing.push(MissingItem {
                    kind,
                    name: name.to_string(),
                    count: 1,
                    resolution: res,
                }),
            }
            with
        };
    // custom slabs
    let customs: Vec<Option<CustomSlab>> = p
        .custom_slabs
        .iter()
        .map(|d| {
            let c = def_to_custom(gd, &names, d);
            if c.is_none() {
                for col in &d.columns {
                    for q in &col.cubes {
                        if cube_id(gd, q).is_none() {
                            decide(MissingKind::Cube, q, &|_| false);
                        }
                    }
                }
                if names.slab_id(&d.recognized_as).is_none() {
                    decide(MissingKind::Slab, &d.recognized_as, &|_| false);
                }
                report.warnings.push(format!(
                    "custom slab \"{}\" cannot be built: its slabs are skipped",
                    d.name
                ));
            }
            c
        })
        .collect();
    let slab_exists = |n: &str| names.slab_id(n).is_some();
    let mut cells: Vec<Option<ResolvedCell>> = p
        .cells
        .iter()
        .map(|c| {
            let c = c.as_ref()?;
            let custom = match &c.custom {
                Some(n) => {
                    let i = p.custom_slabs.iter().position(|d| &d.name == n)?;
                    customs[i].as_ref()?;
                    Some(i)
                }
                None => None,
            };
            let kind = match names.slab_id(&c.slab) {
                Some(k) => k,
                None => names.slab_id(&decide(MissingKind::Slab, &c.slab, &slab_exists)?)?,
            };
            Some(ResolvedCell {
                kind,
                owner: opts.owners.map(c.owner),
                wlb: c.wlb,
                slx: c.slx,
                custom,
            })
        })
        .collect();
    let models: Vec<Option<i64>> = p
        .things
        .iter()
        .map(|t| {
            if let Some(id) = names
                .has_model(t.class, &t.model)
                .then(|| names.model_id(t.class, &t.model))
                .flatten()
            {
                return Some(id);
            }
            let exists = |n: &str| names.has_model(t.class, n);
            let w = decide(MissingKind::of_class(t.class), &t.model, &exists)?;
            names.model_id(t.class, &w)
        })
        .collect();
    // a door's slab kind follows the door's orientation (`create_door` re-places it that way,
    // src/thing_doors.c:140), which a turn of the prefab may have changed
    for (t, m) in p.things.iter().zip(&models) {
        let (Some(m), ThingClass::Door) = (m, t.class) else {
            continue;
        };
        let (sx, sy) = t.slab();
        if sx < 0 || sy < 0 || sx >= p.w as i32 || sy >= p.h as i32 {
            continue;
        }
        if let (Some(Some(rc)), Some(kinds)) = (
            cells.get_mut((sy * p.w as i32 + sx) as usize),
            kfx_map::things::door_slab_kinds(gd, *m),
        ) {
            rc.kind = kinds[t.door_orientation.unwrap_or(0).clamp(0, 1) as usize];
        }
    }
    missing.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
    report.missing = missing;
    Resolved {
        prefab: p,
        cells,
        models,
        customs,
        report,
    }
}

/// Top-left slab that centres a prefab of the (oriented) size on `centre`, like the editor's
/// clipboard stamp (`(w - 1) / 2`).
pub fn origin_centred(w: u32, h: u32, centre: (i32, i32)) -> (i32, i32) {
    (centre.0 - (w as i32 - 1) / 2, centre.1 - (h as i32 - 1) / 2)
}

fn write_allowed(
    doc: Option<&MapDoc>,
    rc: &ResolvedCell,
    x: i32,
    y: i32,
    mode: MaskMode,
) -> Option<bool> {
    // Some(false) = kept because of the mode
    let empty_new = crate::capture::is_empty_kind(rc.kind) && rc.custom.is_none();
    match mode {
        MaskMode::Replace => Some(true),
        MaskMode::SkipEmpty => Some(!empty_new),
        MaskMode::KeepExisting => {
            let Some(d) = doc else { return Some(true) };
            if !autotile::in_map(d.dims, x, y) {
                return Some(true);
            }
            let i = d.dims.slab_index(x as usize, y as usize);
            Some(crate::capture::is_empty_kind(d.kinds[i]) && d.custom_cells[i] == 0)
        }
    }
}

/// Apply `prefab` with its (oriented) north-west corner on slab `at`. One undoable patch.
///
/// Errors when the prefab lies completely outside the map or the document is read-only.
pub fn apply(
    doc: &mut MapDoc,
    gd: &GameData,
    prefab: &Prefab,
    at: (i32, i32),
    opts: &ApplyOptions,
) -> Result<(Patch, Report)> {
    prefab.check()?;
    if let Some(why) = doc.read_only_reason() {
        return Err(Error::Refused(why));
    }
    let res = resolve(prefab, gd, opts);
    let (w, h) = (res.prefab.w as i32, res.prefab.h as i32);
    let full = SlabRect {
        x0: at.0,
        y0: at.1,
        x1: at.0 + w - 1,
        y1: at.1 + h - 1,
    };
    if full.clip(doc.dims).is_empty() {
        return Err(Error::Refused(
            "the prefab lies completely outside the map".into(),
        ));
    }
    let names = Names::new(gd);
    let (report, patch) = {
        let (r, p) = doc.record_tracked(|d| apply_inner(d, gd, &names, &res, at, opts));
        (r, p)
    };
    Ok((patch, report))
}

fn apply_inner(
    d: &mut MapDoc,
    gd: &GameData,
    names: &Names,
    res: &Resolved,
    at: (i32, i32),
    opts: &ApplyOptions,
) -> Report {
    let mut rep = res.report.clone();
    let p = &res.prefab;
    let (w, h) = (p.w as i32, p.h as i32);
    let full = SlabRect {
        x0: at.0,
        y0: at.1,
        x1: at.0 + w - 1,
        y1: at.1 + h - 1,
    };
    rep.rect = Some(full.clip(d.dims));
    // which cells get written
    let mut written = vec![false; (w * h) as usize];
    for cy in 0..h {
        for cx in 0..w {
            let i = (cy * w + cx) as usize;
            let Some(rc) = &res.cells[i] else { continue };
            let (x, y) = (at.0 + cx, at.1 + cy);
            if !autotile::in_map(d.dims, x, y) {
                rep.slabs_outside += 1;
                continue;
            }
            if d.sidecar.locked_at(x, y).is_some() {
                rep.slabs_locked += 1;
                continue;
            }
            if write_allowed(Some(d), rc, x, y, opts.mask) == Some(true) {
                written[i] = true;
            } else {
                rep.slabs_kept += 1;
            }
        }
    }
    let is_written = |x: i32, y: i32, written: &[bool]| {
        let (cx, cy) = (x - at.0, y - at.1);
        cx >= 0 && cy >= 0 && cx < w && cy < h && written[(cy * w + cx) as usize]
    };
    // the old items on the slabs that are overwritten
    if opts.clear_items {
        let mut gone: Vec<ItemRef> = vec![];
        for (i, t) in d.things.items.iter().enumerate() {
            if t.parent_tile.is_some() {
                continue; // slab objects are re-created below
            }
            if let Some((sx, sy)) = things::position(t.x, t.y) {
                if is_written(sx.div_euclid(3), sy.div_euclid(3), &written) {
                    gone.push(ItemRef::Thing(i));
                }
            }
        }
        for (i, a) in d.action_points.items.iter().enumerate() {
            if let Some((sx, sy)) = things::position(a.x, a.y) {
                if is_written(sx.div_euclid(3), sy.div_euclid(3), &written) {
                    gone.push(ItemRef::ActionPoint(i));
                }
            }
        }
        for (i, l) in d.lights.items.iter().enumerate() {
            if l.parent_tile.is_some() {
                continue;
            }
            if let Some((sx, sy)) = things::position(l.x, l.y) {
                if is_written(sx.div_euclid(3), sy.div_euclid(3), &written) {
                    gone.push(ItemRef::Light(i));
                }
            }
        }
        rep.things_removed = gone
            .iter()
            .filter(|g| matches!(g, ItemRef::Thing(_)))
            .count();
        if !gone.is_empty() {
            let _ = d.remove_items(gd, &gone);
        }
    }
    // custom definitions the written cells need -> ids in the document
    let mut custom_ids: Vec<Option<u16>> = vec![None; res.customs.len()];
    for (i, rc) in res.cells.iter().enumerate() {
        if let (Some(rc), true) = (rc, written[i]) {
            if let Some(ci) = rc.custom {
                if custom_ids[ci].is_none() {
                    if let Some(def) = &res.customs[ci] {
                        custom_ids[ci] = Some(d.edit_sidecar(|s| {
                            if let Some(e) = s.custom_slabs.iter().find(|e| same_custom(e, def)) {
                                e.id
                            } else {
                                let mut def = def.clone();
                                def.id = 0;
                                let base = def.name.clone();
                                let mut n = 2;
                                while s.custom_slabs.iter().any(|e| e.name == def.name) {
                                    def.name = format!("{base} ({n})");
                                    n += 1;
                                }
                                s.add_custom_slab(def)
                            }
                        }));
                    }
                }
            }
        }
    }
    // slabs, in cell order; runs of plain slabs go through one `place_slabs` batch. The `.wlb`
    // and `.slx` of a cell are set after its run: kinds never depend on them, and the columns
    // of every touched slab are regenerated below anyway.
    let mut run: Vec<(usize, usize, u16, u8)> = vec![];
    let mut run_extra: Vec<(usize, usize, Option<u8>, u8)> = vec![];
    let flush = |d: &mut MapDoc,
                 run: &mut Vec<(usize, usize, u16, u8)>,
                 run_extra: &mut Vec<(usize, usize, Option<u8>, u8)>| {
        // every written cell is inside the map, so the batch cannot stop part-way
        let _ = d.place_slabs(gd, run.drain(..));
        for (x, y, wlb, slx) in run_extra.drain(..) {
            if let Some(wlb) = wlb {
                let _ = d.set_wlb(x, y, wlb);
            }
            let _ = d.set_slx(x, y, slx);
        }
    };
    for cy in 0..h {
        for cx in 0..w {
            let i = (cy * w + cx) as usize;
            let Some(rc) = &res.cells[i] else { continue };
            if !written[i] {
                continue;
            }
            let (x, y) = ((at.0 + cx) as usize, (at.1 + cy) as usize);
            match rc.custom.and_then(|c| custom_ids[c]) {
                Some(id) => {
                    flush(d, &mut run, &mut run_extra);
                    let _ = d.place_custom_slab(gd, x, y, id, rc.owner);
                    if let Some(wlb) = rc.wlb {
                        let _ = d.set_wlb(x, y, wlb);
                    }
                    let _ = d.set_slx(x, y, rc.slx);
                }
                None => {
                    run.push((x, y, rc.kind, rc.owner));
                    run_extra.push((x, y, rc.wlb, rc.slx));
                }
            }
            rep.slabs_written += 1;
        }
    }
    flush(d, &mut run, &mut run_extra);
    if rep.slabs_written > 0 {
        let r = full.grow(1).clip(d.dims);
        d.regenerate_rect(gd, r);
        d.apply_slab_objects(gd, r);
    }
    // numbers for action points and hero gates
    let mut subs = Substitutions::default();
    for (slot, pl) in opts.owners.slots.iter().enumerate() {
        subs.player.insert(
            ((b'A' + slot as u8) as char).to_string(),
            script::player_text(*pl),
        );
    }
    let thing_cell_written = |t: &PrefabThing, written: &[bool]| -> bool {
        let (sx, sy) = t.slab();
        if sx < 0 || sy < 0 || sx >= w || sy >= h {
            return false;
        }
        let i = (sy * w + sx) as usize;
        written[i] || p.cells[i].is_none()
    };
    let (ox, oy) = (
        at.0 as i64 * RAW_PER_SLAB as i64,
        at.1 as i64 * RAW_PER_SLAB as i64,
    );
    let in_doc = |d: &MapDoc, rx: i64, ry: i64| {
        rx >= 0 && ry >= 0 && rx < d.dims.w as i64 * 768 && ry < d.dims.h as i64 * 768
    };
    if opts.action_points {
        for a in &p.action_points {
            let (rx, ry) = (ox + a.x as i64, oy + a.y as i64);
            if !in_doc(d, rx, ry) {
                rep.warnings.push(format!(
                    "action point {} lies outside the map: skipped",
                    a.key
                ));
                continue;
            }
            let Some(n) = d.free_action_point_number() else {
                rep.warnings
                    .push("no free action point number (255 in use): action points skipped".into());
                break;
            };
            d.add_action_point(coord(rx), coord(ry), coord(a.range as i64), Some(n));
            if !a.extra.is_empty() {
                let last = d.action_points.items.len() - 1;
                d.action_points.items[last].extra = a
                    .extra
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
            }
            subs.ap.insert(a.key.clone(), n);
            rep.action_points_added += 1;
            rep.renumbered.push(Renumbered {
                kind: NumberKind::ActionPoint,
                key: a.key.clone(),
                was: a.number,
                number: n,
            });
        }
    }
    if opts.things {
        for (ti, t) in p.things.iter().enumerate() {
            let Some(model) = res.models[ti] else {
                rep.things_skipped += 1;
                continue;
            };
            let (rx, ry) = (ox + t.x as i64, oy + t.y as i64);
            if !in_doc(d, rx, ry) || !thing_cell_written(t, &written) {
                rep.things_skipped += 1;
                continue;
            }
            let (sx, sy) = ((rx / 256) as i32, (ry / 256) as i32);
            // a door stands at a fixed height (`place_door`), everything else on the floor
            let z = if t.class == ThingClass::Door {
                256 + 128
            } else {
                ((d.floor_height(sx, sy) as i64 * 256) + t.above as i64).max(0)
            };
            let owner = opts.owners.map(t.owner);
            let mut th = things::new_thing(
                t.class.id(),
                model,
                owner,
                coord(rx),
                coord(ry),
                StlCoord::from_raw(z as u32),
            );
            th.orientation = t.orientation;
            if t.class == ThingClass::Creature {
                th.creature_level = Some(t.level.unwrap_or(1));
                th.creature_gold = t.gold;
                th.creature_initial_health = t.health;
                th.creature_name = t.name.clone();
            }
            if t.class == ThingClass::Door {
                th.door_orientation = Some(t.door_orientation.unwrap_or(0));
                th.door_locked = Some(t.door_locked.unwrap_or(false) as i64);
            }
            th.custom_box = t.custom_box;
            th.gold_value = t.gold_value;
            if let Some(r) = t.effect_range {
                th.effect_range = Some(coord(r as i64));
            }
            th.extra = t
                .extra
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if let Some(key) = &t.gate {
                th.herogate_number = d.free_hero_gate_number(gd);
                if th.herogate_number.is_none() {
                    rep.warnings
                        .push("no free hero gate number (255 in use): gate skipped".into());
                    rep.things_skipped += 1;
                    continue;
                }
                subs.gate
                    .insert(key.clone(), th.herogate_number.unwrap_or(0));
                rep.renumbered.push(Renumbered {
                    kind: NumberKind::HeroGate,
                    key: key.clone(),
                    was: None,
                    number: th.herogate_number.unwrap_or(0),
                });
            }
            d.add_thing(gd, th);
            rep.things_added += 1;
        }
    }
    if opts.lights {
        for l in &p.lights {
            let (rx, ry) = (ox + l.x as i64, oy + l.y as i64);
            if !in_doc(d, rx, ry) {
                continue;
            }
            let (sx, sy) = ((rx / 256) as i32, (ry / 256) as i32);
            let (bx, by) = (sx.div_euclid(3), sy.div_euclid(3));
            let cx = bx - at.0;
            let cy = by - at.1;
            if cx >= 0
                && cy >= 0
                && cx < w
                && cy < h
                && !written[(cy * w + cx) as usize]
                && p.cells[(cy * w + cx) as usize].is_some()
            {
                continue;
            }
            let z = ((d.floor_height(sx, sy) as i64 * 256) + l.above as i64).max(0);
            d.touch_lights();
            d.lights.items.push(FxLight {
                dynamic: Some(FxFlag::Bool(l.dynamic)),
                x: Some(coord(rx)),
                y: Some(coord(ry)),
                z: Some(StlCoord::from_raw(z as u32)),
                range: Some(coord(l.range as i64)),
                intensity: Some(l.intensity),
                parent_tile: None,
                extra: l
                    .extra
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            });
            rep.lights_added += 1;
        }
    }
    let _ = names;
    if let Some(s) = &p.scripts.dk {
        let (t, miss) = script::rewrite(s, &subs);
        for m in miss {
            rep.warnings.push(format!(
                "DK script: placeholder {{{{{}.{}}}}} has no value",
                m.kind, m.key
            ));
        }
        rep.script_dk = Some(t);
    }
    if let Some(s) = &p.scripts.lua {
        let (t, miss) = script::rewrite(s, &subs);
        for m in miss {
            rep.warnings.push(format!(
                "Lua script: placeholder {{{{{}.{}}}}} has no value",
                m.kind, m.key
            ));
        }
        rep.script_lua = Some(t);
    }
    rep
}

fn coord(raw: i64) -> StlCoord {
    StlCoord::new(raw.div_euclid(256) as i32, raw.rem_euclid(256) as i32)
}

fn same_custom(a: &CustomSlab, b: &CustomSlab) -> bool {
    a.name == b.name
        && a.recognized_as == b.recognized_as
        && a.liquid == b.liquid
        && a.columns == b.columns
        && a.wibble == b.wibble
        && a.objects == b.objects
}

// ---------------------------------------------------------------------------------------------
// Preview

/// What an icon of the ghost stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconKind {
    Creature,
    Object,
    Trap,
    Door,
    EffectGen,
    HeroGate,
    ActionPoint,
    Light,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewCell {
    /// Absolute slab position.
    pub x: i32,
    pub y: i32,
    pub slab: String,
    pub kind: u16,
    pub owner: u8,
    pub custom: bool,
    /// Whether applying would write this slab (false: kept because of the mask mode, a locked
    /// region, or outside the map).
    pub writes: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewIcon {
    pub kind: IconKind,
    /// Absolute position in slab units (fractions inside the slab).
    pub x: f32,
    pub y: f32,
    pub label: String,
    pub owner: u8,
}

/// The ghost of an application: no document is changed.
#[derive(Clone, Debug)]
pub struct Preview {
    pub origin: (i32, i32),
    pub size: (u32, u32),
    pub cells: Vec<PreviewCell>,
    pub icons: Vec<PreviewIcon>,
    pub report: Report,
}

/// Cells and icons of `prefab` at `at` (north-west corner). With a `doc` the mask mode and locked
/// regions are evaluated against it, without one every non-masked slab counts as written.
pub fn preview(
    prefab: &Prefab,
    gd: &GameData,
    doc: Option<&MapDoc>,
    at: (i32, i32),
    opts: &ApplyOptions,
) -> Preview {
    let res = resolve(prefab, gd, opts);
    let p = &res.prefab;
    let names = Names::new(gd);
    let mut cells = vec![];
    let mut report = res.report.clone();
    for cy in 0..p.h as i32 {
        for cx in 0..p.w as i32 {
            let i = (cy * p.w as i32 + cx) as usize;
            let Some(rc) = &res.cells[i] else { continue };
            let (x, y) = (at.0 + cx, at.1 + cy);
            let inside = doc.is_none_or(|d| autotile::in_map(d.dims, x, y));
            let locked = doc.is_some_and(|d| {
                autotile::in_map(d.dims, x, y) && d.sidecar.locked_at(x, y).is_some()
            });
            let writes = inside && !locked && write_allowed(doc, rc, x, y, opts.mask) == Some(true);
            if !inside {
                report.slabs_outside += 1;
            } else if locked {
                report.slabs_locked += 1;
            } else if !writes {
                report.slabs_kept += 1;
            } else {
                report.slabs_written += 1;
            }
            cells.push(PreviewCell {
                x,
                y,
                slab: names.slab_name(rc.kind),
                kind: rc.kind,
                owner: rc.owner,
                custom: rc.custom.is_some(),
                writes,
            });
        }
    }
    let mut icons = vec![];
    let to_slab = |u: i32, origin: i32| origin as f32 + u as f32 / RAW_PER_SLAB as f32;
    for (ti, t) in p.things.iter().enumerate() {
        if res.models[ti].is_none() {
            continue;
        }
        let kind = match t.class {
            ThingClass::Creature => IconKind::Creature,
            ThingClass::Object if t.gate.is_some() => IconKind::HeroGate,
            ThingClass::Object => IconKind::Object,
            ThingClass::Trap => IconKind::Trap,
            ThingClass::Door => IconKind::Door,
            ThingClass::EffectGen => IconKind::EffectGen,
        };
        icons.push(PreviewIcon {
            kind,
            x: to_slab(t.x, at.0),
            y: to_slab(t.y, at.1),
            label: t.model.clone(),
            owner: opts.owners.map(t.owner),
        });
    }
    for a in &p.action_points {
        icons.push(PreviewIcon {
            kind: IconKind::ActionPoint,
            x: to_slab(a.x, at.0),
            y: to_slab(a.y, at.1),
            label: a.key.clone(),
            owner: PLAYER_NEUTRAL,
        });
    }
    for l in &p.lights {
        icons.push(PreviewIcon {
            kind: IconKind::Light,
            x: to_slab(l.x, at.0),
            y: to_slab(l.y, at.1),
            label: "light".into(),
            owner: PLAYER_NEUTRAL,
        });
    }
    Preview {
        origin: at,
        size: (p.w, p.h),
        cells,
        icons,
        report,
    }
}
