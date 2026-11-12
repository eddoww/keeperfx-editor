//! The editable map document.
//!
//! # Loading (`load_level_file`, `src/lvl_filesdk1.c:1422-1490`)
//! The game loads `.dat .flg .clm`, then tries `.lgtfx`; when it loads, the level is in "FX
//! format" and `.aptfx` / `.tngfx` are used, otherwise the legacy `.lgt` / `.apt` / `.tng`
//! (src/lvl_filesdk1.c:1440-1466). [`MapDoc::load`] follows that rule and keeps things, action points
//! and lights in the FX models in memory either way.
//!
//! # Slab objects (torches, room furniture, slab lights)
//! The game creates slab objects only when it *places* a slab (`place_single_slab_type_on_map`
//! src/map_blocks.c:1374 and `dump_slab_on_map` src/map_blocks.c:1550). Loading a level does not place
//! slabs: `load_level_file` reads the precomputed columns and then creates exactly the things and
//! lights listed in the thing / light files (`load_tngfx_file` / `load_thing_file`,
//! `load_lgtfx_file` / `load_static_light_file`); `reinitialise_map_rooms` (src/room_data.c:1147) only
//! recomputes room statistics. The single exception is doors: `create_door` (src/thing_doors.c:140)
//! re-dumps the door slab, which would create the door slabset's objects (the shipped slabset has
//! none for doors).
//!
//! So a map file must contain its torches, hearts, room decorations and slab lights itself.
//! The game ties them to their slab through the parent index (`ParentTile`, the legacy thing
//! `index` / light `attached_slb`): `delete_attached_things_on_slab` (src/map_blocks.c:590) and
//! `delete_attached_lights_on_slab` (src/map_blocks.c:1045) remove them when the slab is re-placed.
//! [`MapDoc::apply_slab_objects`] does the same for the editor: it removes things/lights attached
//! to the slabs of a rectangle and writes the regenerated ones.
//!
//! # Files that are present but unreadable
//! A `.tngfx` / `.aptfx` / `.lgtfx` or sidecar that exists but cannot be read is not the same as
//! an absent one: saving would replace it with whatever the editor holds (nothing). Such files
//! are listed in [`MapDoc::unreadable`] with their bytes and the parse error, and the document is
//! read-only ([`MapDoc::read_only_reason`]; [`MapDoc::save`] refuses) until the caller explicitly
//! accepts losing them ([`MapDoc::discard_unreadable`]).
//!
//! # Saving
//! [`MapDoc::save`] / [`MapDoc::save_legacy`] build every file in memory and write them as one
//! [`SaveTransaction`]: nothing on disk changes unless every file could be written, and a failure
//! part-way restores the previous files (see `kfx_formats::atomic`). The file that switches the
//! game's format (`.lgtfx`) is written last; stale files are removed after all writes. Things the
//! game would reject when loading the FX files make the save fail
//! ([`MapDoc::game_rejected_things`]): one such thing makes the game refuse the whole level.
use crate::autotile::{
    self, MapGen, SlabObject, SlabObjectKind, SlabRect, SlabView, PLAYER_NEUTRAL,
};
use crate::column::{loaded_column, ColumnPool};
use crate::gamedata::GameData;
use crate::sidecar::{self, CustomObjectKind, CustomSlab, Sidecar};
use crate::{Error, Result};
use kfx_formats::atomic::SaveTransaction;
use kfx_formats::level_bin::{self, Column, Flg, Inf, LevelBin, MapDims, Own, Slb, Slx, Wib, Wlb};
use kfx_formats::level_fx::{
    AptFx, FxActionPoint, FxClass, FxFile, FxFlag, FxLight, FxThing, LegacyActionPointFields,
    LegacyLightFields, LegacyThingFields, LgtFx, ObjectRole, StlCoord, TngFx,
};
use std::path::{Path, PathBuf};

/// How things / action points / lights were stored when loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThingFormat {
    /// `.tngfx` / `.aptfx` / `.lgtfx`.
    Fx,
    /// `.tng` / `.apt` / `.lgt`.
    Legacy,
}

/// Which columns [`MapDoc::save`] writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveColumns {
    /// The document's current column grid: the loaded `.clm`/`.dat` with every edited area
    /// regenerated (the default; keeps the decorative variants of untouched areas).
    Current,
    /// Regenerate every slab before saving.
    Regenerate,
}

/// A map file that exists but could not be read (see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct UnreadablePart {
    /// The file as found.
    pub path: PathBuf,
    /// Extension after `mapNNNNN.` (`tngfx`, `aptfx`, `lgtfx`, `kfxed.toml`).
    pub ext: &'static str,
    /// What the file holds (`things`, ...), for messages.
    pub what: &'static str,
    /// The parse error (with line and column where the reader gives them).
    pub error: String,
    /// The file's bytes as loaded.
    pub raw: Vec<u8>,
}

/// The editable map.
#[derive(Clone, Debug)]
pub struct MapDoc {
    pub folder: PathBuf,
    pub level: u32,
    pub dims: MapDims,
    /// Slab kinds as stored in `.slb` (u16). The game turns values above the slab count into
    /// rock when loading; [`MapDoc::view`] applies that.
    pub kinds: Vec<u16>,
    /// Slab owners (from `.own` subtile (3x+2, 3y+2), see [`Own::slab_owner`]).
    pub owners: Vec<u8>,
    /// `.wlb` per slab.
    pub wlb: Vec<u8>,
    /// Whether `.wlb` came from a file (else it was derived like `initialise_map_wlb_auto`).
    pub wlb_from_file: bool,
    /// `.slx` texture style per slab (0 = map default).
    pub slx: Vec<u8>,
    pub texture_id: u8,
    pub inf_trailing: Vec<u8>,
    pub things: TngFx,
    pub action_points: AptFx,
    pub lights: LgtFx,
    pub loaded_format: ThingFormat,
    /// Current column of every subtile, `(3W+1) x (3H+1)`.
    pub columns: Vec<Column>,
    /// Current wibble of every subtile.
    pub wibble: Vec<u8>,
    /// Whether the loaded map had `.clm` + `.dat` (else the grid was generated at load).
    pub had_columns: bool,
    /// Loaded `.own`, kept for the extra row/column bytes.
    pub own_loaded: Option<Own>,
    /// Loaded `.flg` (written back only when it existed).
    pub flg: Option<Flg>,
    /// Problems found while loading.
    pub warnings: Vec<String>,
    /// Custom slab id per slab (0 = a normal slab), see [`crate::sidecar`]. Saved in the
    /// sidecar; the slab's `.slb` kind is the definition's `recognized_as`.
    pub custom_cells: Vec<u16>,
    /// Editor-only data stored in `mapNNNNN.kfxed.toml` next to the map.
    pub sidecar: Sidecar,
    /// Files present but unreadable when loaded; while not empty (and not discarded) the
    /// document cannot be saved.
    pub unreadable: Vec<UnreadablePart>,
    /// The caller accepted that saving replaces the unreadable files.
    pub unreadable_discarded: bool,
    /// The folder's `.une` (custom slab ids of the Unearth editor) was readable when loaded:
    /// saving keeps it in step with the custom slabs (a `.une` that could not be read is left
    /// untouched).
    pub une_sync: bool,
    /// Change journal of [`MapDoc::record_tracked`] (none outside it).
    pub(crate) journal: Option<Box<crate::patch::Journal>>,
}

/// [`SlabView`] over a document, with the game's slab kind normalisation.
pub struct DocView<'a> {
    pub doc: &'a MapDoc,
    pub slab_count: usize,
}

impl SlabView for DocView<'_> {
    fn dims(&self) -> MapDims {
        self.doc.dims
    }
    fn kind(&self, x: i32, y: i32) -> Option<u8> {
        autotile::in_map(self.doc.dims, x, y).then(|| {
            let n = self.doc.kinds[self.doc.dims.slab_index(x as usize, y as usize)] as usize;
            // load_map_slab_file (src/lvl_filesdk1.c:1209): `n > slab_types_count` becomes rock.
            if n > self.slab_count {
                0
            } else {
                n as u8
            }
        })
    }
    fn owner(&self, x: i32, y: i32) -> Option<u8> {
        autotile::in_map(self.doc.dims, x, y)
            .then(|| self.doc.owners[self.doc.dims.slab_index(x as usize, y as usize)])
    }
    fn wlb(&self, x: i32, y: i32) -> u8 {
        if autotile::in_map(self.doc.dims, x, y) {
            self.doc.wlb[self.doc.dims.slab_index(x as usize, y as usize)]
        } else {
            0
        }
    }
}

fn object_role(gd: &GameData, model: u8) -> ObjectRole {
    // object_is_hero_gate / thing_is_custom_special_box (src/thing_objects.c:426, 356)
    match gd
        .object_genre(model as usize)
        .map(|g| g.trim().to_ascii_uppercase())
    {
        Some(g) if g == "HEROGATE" => ObjectRole::HeroGate,
        Some(g) if g == "SPECIALBOX" && !matches!(model, 86..=93 | 170..=173) => {
            ObjectRole::CustomBox
        }
        _ => ObjectRole::Plain,
    }
}

fn coord(raw: i32) -> StlCoord {
    StlCoord::new(raw >> 8, raw & 0xFF)
}

impl MapDoc {
    pub fn view<'a>(&'a self, gd: &GameData) -> DocView<'a> {
        DocView {
            doc: self,
            slab_count: gd.slab_count(),
        }
    }

    /// Load level `level` from `folder` with the given dims (from the campaign / LOF, or
    /// [`MapDims::from_slb_len`]).
    pub fn load(folder: &Path, level: u32, dims: MapDims, gd: &GameData) -> Result<MapDoc> {
        let bin = LevelBin::load_with_dims(folder, level, Some(dims))?;
        let mut warnings = vec![];
        let slb = bin.slb.clone().ok_or_else(|| {
            Error::Invalid(format!("map{level:05}: no .slb in {}", folder.display()))
        })?;
        let n = dims.slabs();
        let owners: Vec<u8> = match &bin.own {
            Some(o) => (0..n)
                .map(|i| {
                    o.slab_owner(dims, i % dims.w as usize, i / dims.w as usize)
                        .unwrap_or(PLAYER_NEUTRAL)
                })
                .collect(),
            None => {
                warnings.push("no .own file; all slabs neutral".into());
                vec![PLAYER_NEUTRAL; n]
            }
        };
        // Things / action points / lights.
        let fx_path = |ext: &str| level_bin::find_file(folder, level, ext);
        let mut unreadable: Vec<UnreadablePart> = vec![];
        let mut bad = |ext: &'static str,
                       what: &'static str,
                       p: &Path,
                       e: &dyn std::fmt::Display,
                       warnings: &mut Vec<String>| {
            let raw = std::fs::read(p).unwrap_or_default();
            warnings.push(format!(
                "{} could not be read ({e}); the map is read-only until you \
                choose to discard its {what}",
                p.display()
            ));
            unreadable.push(UnreadablePart {
                path: p.to_path_buf(),
                ext,
                what,
                error: e.to_string(),
                raw,
            });
        };
        let lgtfx = fx_path("lgtfx").map(|p| (LgtFx::load(&p), p));
        for w in lgtfx
            .iter()
            .filter_map(|(r, _)| r.as_ref().ok())
            .flat_map(|l| l.warnings.iter())
        {
            warnings.push(format!("lgtfx: {w}"));
        }
        let (things, action_points, lights, fmt) = match lgtfx {
            Some((Ok(l), _)) => {
                let t = match fx_path("tngfx").map(|p| (TngFx::load(&p), p)) {
                    Some((Ok(t), _)) => {
                        warnings.extend(t.warnings.iter().map(|w| format!("tngfx: {w}")));
                        t
                    }
                    Some((Err(e), p)) => {
                        bad("tngfx", "things", &p, &e, &mut warnings);
                        TngFx::new()
                    }
                    None => {
                        warnings.push("FX format (lgtfx) but no tngfx".into());
                        TngFx::new()
                    }
                };
                let a = match fx_path("aptfx").map(|p| (AptFx::load(&p), p)) {
                    Some((Ok(a), _)) => {
                        warnings.extend(a.warnings.iter().map(|w| format!("aptfx: {w}")));
                        a
                    }
                    Some((Err(e), p)) => {
                        bad("aptfx", "action points", &p, &e, &mut warnings);
                        AptFx::new()
                    }
                    None => AptFx::new(),
                };
                (t, a, l, ThingFormat::Fx)
            }
            other => {
                if let Some((Err(e), p)) = other {
                    // the game uses the legacy files then (src/lvl_filesdk1.c:1440); the FX files
                    // still hold the author's work, so they must not be overwritten silently
                    warnings.push(format!(
                        "lgtfx did not load; the game uses the legacy \
                        .lgt/.tng/.apt files, and so does the editor's view"
                    ));
                    bad(
                        "lgtfx",
                        "lights (and the FX things and action points, which the game \
                        ignores without a readable .lgtfx)",
                        &p,
                        &e,
                        &mut warnings,
                    );
                }
                let mut t = TngFx::new();
                for th in bin.tng.iter().flat_map(|t| t.things.iter()) {
                    let l = LegacyThingFields {
                        x: th.x,
                        y: th.y,
                        z: th.z,
                        oclass: th.class,
                        model: th.model,
                        owner: th.owner,
                        range: th.range,
                        index: th.index,
                        params: th.params,
                    };
                    let mut item = FxThing::from_legacy(&l, object_role(gd, th.model));
                    crate::legacy::keep_legacy_residue(&mut item, th);
                    t.items.push(item);
                }
                let mut a = AptFx::new();
                for p in bin.apt.iter().flat_map(|a| a.points.iter()) {
                    a.items
                        .push(FxActionPoint::from_legacy(&LegacyActionPointFields {
                            x: p.x,
                            y: p.y,
                            range: p.range,
                            num: p.number,
                        }));
                }
                let mut lg = LgtFx::new();
                for l in bin.lgt.iter().flat_map(|l| l.lights.iter()) {
                    let mut item = FxLight::from_legacy(&LegacyLightFields {
                        radius: l.radius,
                        intensity: l.intensity,
                        flags: l.flags,
                        x: l.x,
                        y: l.y,
                        z: l.z,
                        is_dynamic: l.is_dynamic,
                        attached_slb: l.attached_slb,
                    });
                    crate::legacy::keep_light_residue(&mut item, l);
                    lg.items.push(item);
                }
                (t, a, lg, ThingFormat::Legacy)
            }
        };
        let mut doc = MapDoc {
            folder: folder.to_path_buf(),
            level,
            dims,
            kinds: slb.kinds.clone(),
            owners,
            wlb: vec![0; n],
            wlb_from_file: false,
            slx: bin
                .slx
                .as_ref()
                .map(|s| s.values.clone())
                .unwrap_or_else(|| vec![0; n]),
            texture_id: bin.inf.as_ref().map_or(0, |i| i.texture_id),
            inf_trailing: bin
                .inf
                .as_ref()
                .map(|i| i.trailing.clone())
                .unwrap_or_default(),
            things,
            action_points,
            lights,
            loaded_format: fmt,
            columns: vec![],
            wibble: vec![],
            had_columns: false,
            own_loaded: bin.own.clone(),
            flg: bin.flg.clone(),
            warnings,
            custom_cells: vec![0; n],
            sidecar: Sidecar::default(),
            unreadable,
            unreadable_discarded: false,
            une_sync: false,
            journal: None,
        };
        if let Some(p) = level_bin::find_file(folder, level, "kfxed.toml") {
            match sidecar::load(&p, dims) {
                Ok((sc, cells, w)) => {
                    doc.sidecar = sc;
                    doc.custom_cells = cells;
                    doc.warnings.extend(w);
                }
                Err(e) => {
                    let raw = std::fs::read(&p).unwrap_or_default();
                    doc.warnings.push(format!(
                        "{} could not be read ({e}); the map is read-only \
                        until you choose to discard its editor data",
                        p.display()
                    ));
                    doc.unreadable.push(UnreadablePart {
                        path: p.clone(),
                        ext: "kfxed.toml",
                        what: "editor data (custom slabs, notes, locks)",
                        error: e.to_string(),
                        raw,
                    });
                }
            }
        }
        // .wlb: load_map_wlb_file validation (src/lvl_filesdk1.c:1141), else initialise_map_wlb_auto.
        match &bin.wlb {
            Some(w) => {
                doc.wlb_from_file = true;
                let mut fixes = 0;
                for i in 0..n {
                    let v = w.values[i];
                    let view = doc.view(gd);
                    let kind = view
                        .kind((i % dims.w as usize) as i32, (i / dims.w as usize) as i32)
                        .unwrap_or(0);
                    let sk = autotile::slab_kind_from_wlb(gd, v);
                    let ok = v == 0
                        || match sk {
                            None => false,
                            Some(k) => {
                                gd.slab(kind as usize).wlb_type == autotile::WLB_BRIDGE
                                    || kind as usize == k
                            }
                        };
                    doc.wlb[i] = if ok { v } else { 0 };
                    if !ok {
                        fixes += 1;
                    }
                }
                if fixes > 0 {
                    doc.warnings.push(format!(
                        "WLB file is muddled - fixed values for {fixes} tiles"
                    ));
                }
            }
            None => {
                let w: Vec<u8> = {
                    let view = doc.view(gd);
                    (0..n)
                        .map(|i| {
                            autotile::auto_wlb(
                                &view,
                                gd,
                                (i % dims.w as usize) as i32,
                                (i / dims.w as usize) as i32,
                            )
                        })
                        .collect()
                };
                doc.wlb = w;
            }
        }
        // Columns: as loaded, or generated when the map has none.
        match (&bin.clm, &bin.dat) {
            (Some(clm), Some(dat)) => {
                doc.had_columns = true;
                doc.columns = (0..dims.stl_grid())
                    .map(|i| loaded_column(clm, dat, i))
                    .collect();
                doc.wibble = match &bin.wib {
                    Some(w) => w.values.clone(),
                    None => vec![0; dims.stl_grid()],
                };
            }
            _ => {
                doc.warnings.push("no .clm/.dat; columns generated".into());
                doc.columns = vec![Column::default(); dims.stl_grid()];
                doc.wibble = vec![0; dims.stl_grid()];
                doc.regenerate_columns(gd);
            }
        }
        if let Some(p) = level_bin::find_file(folder, level, "une") {
            let bytes = std::fs::read(&p).map_err(|e| Error::io(&p, e));
            let readable = bytes.as_ref().map_err(|e| e.to_string()).and_then(|b| {
                crate::unearth::parse_une(b, dims)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            });
            match readable {
                Ok(()) => doc.une_sync = true,
                Err(e) => doc.warnings.push(format!(
                    "{} could not be read ({e}); it is left untouched when saving",
                    p.display()
                )),
            }
            if doc.une_sync
                && doc.sidecar.is_empty()
                && doc.custom_cells.iter().all(|&c| c == 0)
                && !doc.unreadable.iter().any(|u| u.ext == "kfxed.toml")
            {
                // a map with a `.une` and no sidecar yet: take its custom slab ids
                match bytes.and_then(|b| doc.import_une(gd, &b, None)) {
                    Ok(r) if r.cells > 0 => doc.warnings.push(format!(
                        "imported {} custom slab cells ({} definitions) from {}",
                        r.cells,
                        r.definitions.len(),
                        p.display()
                    )),
                    Ok(_) => {}
                    Err(e) => {
                        doc.une_sync = false;
                        doc.warnings.push(format!(
                            "{} could not be imported ({e}); it is left untouched when saving",
                            p.display()
                        ));
                    }
                }
            }
        }
        Ok(doc)
    }

    /// A new map of `kind` slabs (normally rock), neutral, with generated columns.
    pub fn new(folder: &Path, level: u32, dims: MapDims, kind: u16, gd: &GameData) -> MapDoc {
        let n = dims.slabs();
        let mut doc = MapDoc {
            folder: folder.to_path_buf(),
            level,
            dims,
            kinds: vec![kind; n],
            owners: vec![PLAYER_NEUTRAL; n],
            wlb: vec![gd.slab(kind as usize).wlb_type; n],
            wlb_from_file: false,
            slx: vec![0; n],
            texture_id: 0,
            inf_trailing: vec![],
            things: TngFx::new(),
            action_points: AptFx::new(),
            lights: LgtFx::new(),
            loaded_format: ThingFormat::Fx,
            columns: vec![Column::default(); dims.stl_grid()],
            wibble: vec![0; dims.stl_grid()],
            had_columns: false,
            own_loaded: None,
            flg: None,
            warnings: vec![],
            custom_cells: vec![0; n],
            sidecar: Sidecar::default(),
            unreadable: vec![],
            unreadable_discarded: false,
            une_sync: false,
            journal: None,
        };
        doc.regenerate_columns(gd);
        doc
    }

    /// Why the document cannot be saved, `None` when it can: files that exist but could not be
    /// read would be replaced (see the module docs).
    pub fn read_only_reason(&self) -> Option<String> {
        if self.unreadable.is_empty() || self.unreadable_discarded {
            return None;
        }
        let parts: Vec<String> = self
            .unreadable
            .iter()
            .map(|u| format!("{} ({}): {}", u.path.display(), u.what, u.error))
            .collect();
        Some(format!(
            "read-only: saving would replace files the editor could not read: {}",
            parts.join("; ")
        ))
    }

    /// Accept that saving replaces the unreadable files with what the document holds (their
    /// bytes stay in [`MapDoc::unreadable`] so the caller can back them up).
    pub fn discard_unreadable(&mut self) {
        self.unreadable_discarded = true;
    }

    fn whole(&self) -> SlabRect {
        SlabRect {
            x0: 0,
            y0: 0,
            x1: self.dims.w as i32 - 1,
            y1: self.dims.h as i32 - 1,
        }
    }

    /// Generate the columns / wibble of `rect` into the current grid.
    pub fn regenerate_rect(&mut self, gd: &GameData, rect: SlabRect) {
        let mut gen = MapGen::empty(self.dims);
        self.regenerate_rect_with(gd, rect, &mut gen);
    }

    /// [`MapDoc::regenerate_rect`] with a reusable scratch generation `gen` (of this document's
    /// dims). The subtiles of `rect` in `gen` are reset first, so the result is the same as with a
    /// fresh [`MapGen::empty`]: subtiles the generator leaves unresolved get the default column.
    fn regenerate_rect_with(&mut self, gd: &GameData, rect: SlabRect, gen: &mut MapGen) {
        let rect = rect.clip(self.dims);
        for (x, y) in rect.iter() {
            for i in 0..9 {
                gen.columns[self
                    .dims
                    .stl_index(3 * x as usize + i % 3, 3 * y as usize + i / 3)] =
                    Default::default();
            }
        }
        gen.regenerate_rect(&self.view(gd), gd, rect);
        for (x, y) in rect.iter() {
            let custom = self.custom_slab_at(x, y).cloned();
            let owner = self.owners[self.dims.slab_index(x as usize, y as usize)];
            for i in 0..9 {
                let k = self
                    .dims
                    .stl_index(3 * x as usize + i % 3, 3 * y as usize + i / 3);
                self.j_stl(k);
                match &custom {
                    // custom slabs keep their own columns: a neighbour never regenerates them
                    Some(c) => {
                        self.columns[k] = autotile::owner_column(gd, &c.columns[i], owner);
                        self.wibble[k] = c.wibble[i];
                    }
                    None => {
                        self.columns[k] = gen.columns[k];
                        self.wibble[k] = gen.wibble[k];
                    }
                }
            }
        }
    }

    /// The custom slab definition at slab (x, y), if the slab is custom and defined.
    pub fn custom_slab_at(&self, x: i32, y: i32) -> Option<&CustomSlab> {
        if !autotile::in_map(self.dims, x, y) {
            return None;
        }
        match self.custom_cells[self.dims.slab_index(x as usize, y as usize)] {
            0 => None,
            id => self.sidecar.custom_slab(id),
        }
    }

    /// Edit the sidecar (custom slab definitions, editor settings); journaled for
    /// [`MapDoc::record_tracked`].
    pub fn edit_sidecar<R>(&mut self, f: impl FnOnce(&mut Sidecar) -> R) -> R {
        self.touch_props();
        f(&mut self.sidecar)
    }

    /// Place custom slab `id` at (x, y): the slab kind becomes `recognized_as`, `.wlb` the
    /// definition's liquid (or the kind's `WlbType`), and the nine columns are the definition's
    /// (recoloured for `owner`). The 3x3 block around is regenerated; the slab itself is kept by
    /// later regenerations. Errors when the id is not defined.
    pub fn place_custom_slab(
        &mut self,
        gd: &GameData,
        x: usize,
        y: usize,
        id: u16,
        owner: u8,
    ) -> Result<SlabRect> {
        let i = self.check(x, y)?;
        let def = self
            .sidecar
            .custom_slab(id)
            .ok_or_else(|| Error::Invalid(format!("custom slab {id} is not defined")))?
            .clone();
        self.j_slab(i);
        self.kinds[i] = def.recognized_as;
        self.owners[i] = owner;
        let wlb = def
            .liquid
            .unwrap_or_else(|| gd.slab(def.recognized_as as usize).wlb_type);
        if def.liquid.is_some()
            || gd.slab(def.recognized_as as usize).wlb_type != autotile::WLB_BRIDGE
        {
            self.wlb[i] = wlb;
        }
        self.custom_cells[i] = id;
        Ok(self.refresh_around(gd, x as i32, y as i32))
    }

    /// Regenerate every slab's columns (drops the loaded decorative variants).
    pub fn regenerate_columns(&mut self, gd: &GameData) {
        let r = self.whole();
        self.regenerate_rect(gd, r);
    }

    /// The full generation (columns, wibble, unfiltered objects) for the current layout.
    pub fn generate(&self, gd: &GameData) -> MapGen {
        MapGen::regenerate_all(&self.view(gd), gd)
    }

    fn check(&self, x: usize, y: usize) -> Result<usize> {
        if x >= self.dims.w as usize || y >= self.dims.h as usize {
            return Err(Error::Invalid(format!(
                "slab ({x},{y}) outside {}x{}",
                self.dims.w, self.dims.h
            )));
        }
        Ok(self.dims.slab_index(x, y))
    }

    /// Set a slab kind exactly as given and regenerate the 3x3 block around it. Non-bridge
    /// kinds also set `.wlb` to their `WlbType` (`place_slab_columns`, src/map_blocks.c:766).
    /// Returns the slabs whose columns changed.
    pub fn set_slab(&mut self, gd: &GameData, x: usize, y: usize, kind: u16) -> Result<SlabRect> {
        let i = self.check(x, y)?;
        self.j_slab(i);
        self.kinds[i] = kind;
        self.custom_cells[i] = 0;
        let s = gd.slab(kind as usize);
        if s.wlb_type != autotile::WLB_BRIDGE {
            self.wlb[i] = s.wlb_type;
        }
        Ok(self.refresh_around(gd, x as i32, y as i32))
    }

    /// Set a slab owner and regenerate the 3x3 block around it.
    pub fn set_owner(&mut self, gd: &GameData, x: usize, y: usize, owner: u8) -> Result<SlabRect> {
        let i = self.check(x, y)?;
        self.j_slab(i);
        self.owners[i] = owner;
        Ok(self.refresh_around(gd, x as i32, y as i32))
    }

    /// Place a slab the way `place_slab_type_on_map_f` (src/map_blocks.c:1646) changes kinds: plain
    /// earth becomes torch dirt where `alter_rock_style` says so, and the 8 neighbours' earth is
    /// re-styled too (for ground/room the C uses the torch grid only, src/map_blocks.c:1712-1725).
    pub fn place_slab(
        &mut self,
        gd: &GameData,
        x: usize,
        y: usize,
        kind: u16,
        owner: u8,
    ) -> Result<SlabRect> {
        let mut gen = MapGen::empty(self.dims);
        self.place_slab_with(gd, x, y, kind, owner, &mut gen)
    }

    /// [`MapDoc::place_slab`] for many slabs, in the order given: the same per-slab rule (kind
    /// restyling, the 8 neighbours' earth, the 3x3 regeneration after each slab), so the result is
    /// identical to calling `place_slab` in a loop, but one scratch generation is allocated for the
    /// whole batch instead of a full-map one per slab.
    ///
    /// Returns the union of the slabs whose columns changed (`None` for an empty batch). A slab
    /// outside the map is an error like with `place_slab`; the slabs before it stay placed.
    pub fn place_slabs(
        &mut self,
        gd: &GameData,
        slabs: impl IntoIterator<Item = (usize, usize, u16, u8)>,
    ) -> Result<Option<SlabRect>> {
        let mut gen: Option<MapGen> = None;
        let mut changed: Option<SlabRect> = None;
        for (x, y, kind, owner) in slabs {
            let gen = gen.get_or_insert_with(|| MapGen::empty(self.dims));
            let r = self.place_slab_with(gd, x, y, kind, owner, gen)?;
            changed = Some(changed.map_or(r, |c| c.union(r)));
        }
        Ok(changed)
    }

    fn place_slab_with(
        &mut self,
        gd: &GameData,
        x: usize,
        y: usize,
        kind: u16,
        owner: u8,
        gen: &mut MapGen,
    ) -> Result<SlabRect> {
        let i = self.check(x, y)?;
        let (xi, yi) = (x as i32, y as i32);
        let k = autotile::alter_rock_style(&self.view(gd), gd, kind as u8, xi, yi) as u16;
        let k = if kind > u8::MAX as u16 { kind } else { k };
        self.j_slab(i);
        self.kinds[i] = k;
        self.owners[i] = owner;
        self.custom_cells[i] = 0;
        let s = gd.slab(k as usize);
        if s.wlb_type != autotile::WLB_BRIDGE {
            self.wlb[i] = s.wlb_type;
        }
        let cat = s.category;
        for (dx, dy) in autotile::AROUND_EIGHT {
            let (sx, sy) = (xi + dx, yi + dy);
            if !autotile::in_map(self.dims, sx, sy) {
                continue;
            }
            let j = self.dims.slab_index(sx as usize, sy as usize);
            let nk = self.kinds[j];
            if nk != autotile::kinds::EARTH as u16 {
                continue;
            }
            let new = if cat == crate::gamedata::category::ROOM_INTERIOR
                || cat == crate::gamedata::category::FORTIFIED_GROUND
            {
                if autotile::torch_flags_for_slab(&self.view(gd), sx, sy) == 0 {
                    autotile::kinds::EARTH
                } else {
                    autotile::kinds::TORCHDIRT
                }
            } else {
                autotile::alter_rock_style(&self.view(gd), gd, nk as u8, sx, sy)
            };
            self.j_slab(j);
            self.kinds[j] = new as u16;
        }
        let r = SlabRect::single(xi, yi).grow(1).clip(self.dims);
        self.regenerate_rect_with(gd, r, gen);
        Ok(r)
    }

    fn refresh_around(&mut self, gd: &GameData, x: i32, y: i32) -> SlabRect {
        let r = SlabRect::single(x, y).grow(1).clip(self.dims);
        self.regenerate_rect(gd, r);
        r
    }

    pub fn set_wlb(&mut self, x: usize, y: usize, v: u8) -> Result<SlabRect> {
        let i = self.check(x, y)?;
        self.j_slab(i);
        self.wlb[i] = v;
        Ok(SlabRect::single(x as i32, y as i32))
    }

    pub fn set_slx(&mut self, x: usize, y: usize, v: u8) -> Result<SlabRect> {
        let i = self.check(x, y)?;
        self.j_slab(i);
        self.slx[i] = v;
        Ok(SlabRect::single(x as i32, y as i32))
    }

    pub fn set_texture_id(&mut self, id: u8) {
        self.touch_props();
        self.texture_id = id;
    }

    /// Slab objects the game would create for the slabs of `rect`, filtered with the current
    /// columns (`BLOCKING` test of `place_slab_object`).
    pub fn slab_objects(&self, gd: &GameData, rect: SlabRect) -> Vec<SlabObject> {
        let v = self.view(gd);
        let rect = rect.clip(self.dims);
        let mut out = vec![];
        for (x, y) in rect.iter() {
            let objects = match self.custom_slab_at(x, y) {
                Some(c) => self.custom_slab_objects(c, x, y),
                None => autotile::slab_columns(&v, gd, x, y).objects,
            };
            for o in objects {
                if !autotile::subtile_blocking(&self.columns, &v, gd, o.check_stl.0, o.check_stl.1)
                {
                    out.push(o);
                }
            }
        }
        out
    }

    /// The objects of custom slab `c` standing at slab (x, y), as slab objects.
    fn custom_slab_objects(&self, c: &CustomSlab, x: i32, y: i32) -> Vec<SlabObject> {
        let owner = self.owners[self.dims.slab_index(x as usize, y as usize)];
        let parent_slab = autotile::slab_number(self.dims, x, y);
        c.objects
            .iter()
            .map(|o| {
                let (ax, ay) = (768 * x + o.dx, 768 * y + o.dy);
                SlabObject {
                    kind: match o.kind {
                        CustomObjectKind::Object { model } => SlabObjectKind::Object { model },
                        CustomObjectKind::EffectGen { model, range } => {
                            SlabObjectKind::EffectGen { model, range }
                        }
                        CustomObjectKind::Light { intensity, radius } => {
                            SlabObjectKind::Light { intensity, radius }
                        }
                    },
                    x: ax,
                    y: ay,
                    z: o.z,
                    owner,
                    orientation: o.orientation,
                    parent_slab,
                    slabset_id: 0,
                    stl_id: 0,
                    check_stl: (ax >> 8, ay >> 8),
                }
            })
            .collect()
    }

    /// Replace the slab-attached things and lights of `rect` by the regenerated slab objects,
    /// like the game when it re-places those slabs: objects / effect generators whose parent is
    /// one of the slabs (and that stand within one subtile of it, src/map_blocks.c:590-612) and
    /// lights attached to it (src/map_blocks.c:1045-1090) are removed, then the slab objects are
    /// added with `ParentTile` set. Returns how many items were removed and added.
    pub fn apply_slab_objects(&mut self, gd: &GameData, rect: SlabRect) -> (usize, usize) {
        let rect = rect.clip(self.dims);
        let d = self.dims;
        self.touch_things();
        self.touch_lights();
        let within = |x: i32, y: i32, sx: i32, sy: i32, lo: i32, hi: i32| {
            let (bx, by) = (3 * sx, 3 * sy);
            x >= bx + lo && x <= bx + hi && y >= by + lo && y <= by + hi
        };
        let mut removed = 0;
        let before = self.things.items.len();
        self.things.items.retain(|t| {
            let attached = matches!(t.class.as_ref().and_then(|c| c.id()), Some(1) | Some(7));
            let (Some(p), Some(x), Some(y)) = (t.parent_tile, t.x, t.y) else {
                return true;
            };
            if !attached {
                return true;
            }
            !rect.iter().any(|(sx, sy)| {
                p == autotile::slab_number(d, sx, sy) as i64 && within(x.stl, y.stl, sx, sy, -1, 3)
            })
        });
        removed += before - self.things.items.len();
        let before = self.lights.items.len();
        self.lights.items.retain(|l| {
            let (Some(p), Some(x), Some(y)) = (l.parent_tile, l.x, l.y) else {
                return true;
            };
            !rect.iter().any(|(sx, sy)| {
                p == autotile::slab_number(d, sx, sy) as i64 && within(x.stl, y.stl, sx, sy, -1, 4)
            })
        });
        removed += before - self.lights.items.len();
        let objs = self.slab_objects(gd, rect);
        let added = objs.len();
        for o in objs {
            match o.kind {
                SlabObjectKind::Light { intensity, radius } => self.lights.items.push(FxLight {
                    dynamic: Some(FxFlag::Bool(false)),
                    x: Some(coord(o.x)),
                    y: Some(coord(o.y)),
                    z: Some(coord(o.z)),
                    range: Some(coord(radius)),
                    intensity: Some(intensity as i64),
                    parent_tile: Some(o.parent_slab as i64),
                    ..Default::default()
                }),
                SlabObjectKind::Object { model } => self.things.items.push(FxThing {
                    class: Some(FxClass::from_id(1)),
                    subtype: Some(model as i64),
                    owner: Some(o.owner as i64),
                    x: Some(coord(o.x)),
                    y: Some(coord(o.y)),
                    z: Some(coord(o.z)),
                    parent_tile: Some(o.parent_slab as i64),
                    orientation: (o.orientation != 0).then_some(o.orientation as i64),
                    ..Default::default()
                }),
                SlabObjectKind::EffectGen { model, range } => self.things.items.push(FxThing {
                    class: Some(FxClass::from_id(7)),
                    subtype: Some(model as i64),
                    owner: Some(o.owner as i64),
                    x: Some(coord(o.x)),
                    y: Some(coord(o.y)),
                    z: Some(coord(o.z)),
                    parent_tile: Some(o.parent_slab as i64),
                    effect_range: Some(coord(range)),
                    ..Default::default()
                }),
            }
        }
        (removed, added)
    }

    /// The map files shared by FX and legacy output (everything but things / action points /
    /// lights) and whether a `.slx` is needed.
    pub(crate) fn build_bin(&self, gd: &GameData, which: SaveColumns) -> Result<(LevelBin, bool)> {
        let d = self.dims;
        let (columns, wibble) = match which {
            SaveColumns::Current => (self.columns.clone(), self.wibble.clone()),
            SaveColumns::Regenerate => {
                let mut c = self.clone();
                c.regenerate_columns(gd);
                (c.columns, c.wibble)
            }
        };
        let (clm, dat) = ColumnPool::build(gd, d, &columns)?;
        let mut own = vec![PLAYER_NEUTRAL; d.stl_grid()];
        if let Some(o) = &self.own_loaded {
            for (i, b) in own.iter_mut().enumerate() {
                *b = o.owners.get(i).copied().unwrap_or(PLAYER_NEUTRAL);
            }
        }
        for y in 0..d.h as usize {
            for x in 0..d.w as usize {
                let ow = self.owners[d.slab_index(x, y)];
                for i in 0..9 {
                    own[d.stl_index(3 * x + i % 3, 3 * y + i / 3)] = ow;
                }
            }
        }
        let any_slx = self.slx.iter().any(|&v| v != 0);
        let bin = LevelBin {
            number: self.level,
            dims: Some(d),
            slb: Some(Slb {
                kinds: self.kinds.clone(),
                trailing: vec![],
            }),
            own: Some(Own {
                owners: own,
                trailing: vec![],
            }),
            dat: Some(dat),
            clm: Some(clm),
            wib: Some(Wib {
                values: wibble,
                trailing: vec![],
            }),
            wlb: Some(Wlb {
                values: self.wlb.clone(),
                trailing: vec![],
            }),
            flg: self.flg.clone(),
            inf: Some(Inf {
                texture_id: self.texture_id,
                trailing: self.inf_trailing.clone(),
            }),
            slx: any_slx.then(|| Slx {
                values: self.slx.clone(),
                trailing: vec![],
            }),
            tng: None,
            apt: None,
            lgt: None,
        };
        Ok((bin, any_slx))
    }

    /// Write the map into `folder` (normally [`MapDoc::folder`]): `.slb .own .wlb .inf .wib .clm
    /// .dat`, `.slx` when any value is non-zero (a stale one is removed otherwise), `.flg` when the
    /// map had one, and always `.tngfx .aptfx .lgtfx` (each with `[common]`; `.lgtfx` is what
    /// switches the game to the FX files, so it is written last). Stale `.tng/.apt/.lgt` are
    /// removed only after everything else was written. All or nothing: see the module docs.
    pub fn save(&self, gd: &GameData, folder: &Path, which: SaveColumns) -> Result<()> {
        self.save_transaction(gd, folder, which)?.commit()?;
        Ok(())
    }

    /// The files [`MapDoc::save`] writes and removes, not yet applied (callers add their own
    /// files, e.g. the `.lof`, and commit once).
    pub fn save_transaction(
        &self,
        gd: &GameData,
        folder: &Path,
        which: SaveColumns,
    ) -> Result<SaveTransaction> {
        self.check_writable()?;
        let rejected = self.game_rejected_things(gd);
        if !rejected.is_empty() {
            let list: Vec<String> = rejected
                .iter()
                .take(5)
                .map(|(i, why)| format!("thing {i}: {why}"))
                .collect();
            return Err(Error::Invalid(format!(
                "not saved: the game refuses to load a level with {} thing(s) it cannot create \
                    ({}{}); fix or remove them",
                rejected.len(),
                list.join("; "),
                if rejected.len() > 5 { "; ..." } else { "" }
            )));
        }
        let (bin, any_slx) = self.build_bin(gd, which)?;
        let mut tx = SaveTransaction::new(folder);
        for (ext, bytes) in bin.files() {
            tx.write(level_bin::file_name(self.level, ext), bytes);
        }
        let tngfx = strip_legacy(
            &self.things,
            |t| &mut t.extra,
            |t| t.extra.keys().any(|k| crate::legacy::is_legacy_key(k)),
        );
        let lgtfx = strip_legacy(
            &self.lights,
            |l| &mut l.extra,
            |l| l.extra.keys().any(|k| crate::legacy::is_legacy_key(k)),
        );
        tx.write(
            level_bin::file_name(self.level, "tngfx"),
            tngfx.to_game_bytes()?,
        );
        tx.write(
            level_bin::file_name(self.level, "aptfx"),
            self.action_points.to_game_bytes()?,
        );
        self.sidecar_ops(&mut tx);
        tx.write(
            level_bin::file_name(self.level, "lgtfx"),
            lgtfx.to_game_bytes()?,
        );
        if !any_slx {
            tx.remove(level_bin::file_name(self.level, "slx"));
        }
        for ext in ["tng", "apt", "lgt"] {
            tx.remove(level_bin::file_name(self.level, ext));
        }
        Ok(tx)
    }

    /// `Err` while the document is read-only ([`MapDoc::read_only_reason`]).
    pub fn check_writable(&self) -> Result<()> {
        match self.read_only_reason() {
            Some(r) => Err(Error::Invalid(r)),
            None => Ok(()),
        }
    }

    /// Things the game's FX loader refuses, each of which makes the game fail the whole level
    /// (`load_tngfx_file` returns failure, src/lvl_filesdk1.c:1469; `thing_create_thing_adv`
    /// src/thing_factory.c:217-412): no or an unknown `ThingType` or a class the loader does not create
    /// (only Object, Creature, EffectGen, Trap, Door and the classes 10 / 11), no `Subtype`, a
    /// `SubtypeStringID` that is not a string or not a known name, no or a too high `Ownership`
    /// (above `PLAYERS_COUNT`). Also more things than the game can create.
    pub fn game_rejected_things(&self, gd: &GameData) -> Vec<(usize, String)> {
        let mut out = vec![];
        for (i, t) in self.things.items.iter().enumerate() {
            if let Some(why) = crate::validate::fx_thing_rejection(t, gd) {
                out.push((i, why));
            }
        }
        out
    }

    /// Remove the things of [`MapDoc::game_rejected_things`] (an explicit user choice);
    /// returns how many were removed.
    pub fn drop_game_rejected_things(&mut self, gd: &GameData) -> usize {
        let bad: std::collections::HashSet<usize> = self
            .game_rejected_things(gd)
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        if bad.is_empty() {
            return 0;
        }
        self.touch_things();
        let mut i = 0;
        self.things.items.retain(|_| {
            let keep = !bad.contains(&i);
            i += 1;
            keep
        });
        bad.len()
    }

    /// Write `mapNNNNN.kfxed.toml` (custom slabs and editor settings) into `folder`, or remove
    /// a stale one when there is nothing to store; keeps a readable `.une` in step.
    pub fn save_sidecar(&self, folder: &Path) -> Result<()> {
        self.check_writable()?;
        let mut tx = SaveTransaction::new(folder);
        self.sidecar_ops(&mut tx);
        tx.commit()?;
        Ok(())
    }

    /// Sidecar / `.une` writes and removals for a save into `tx`.
    pub(crate) fn sidecar_ops(&self, tx: &mut SaveTransaction) {
        // keep a `.une` that was readable in step so the map stays usable in the Unearth editor;
        // one the editor could not read is left as it is
        if self.une_sync {
            tx.write(level_bin::file_name(self.level, "une"), self.export_une());
        }
        if self.sidecar.is_empty() && self.custom_cells.iter().all(|&c| c == 0) {
            tx.remove(level_bin::file_name(self.level, "kfxed.toml"));
        } else {
            tx.write(
                level_bin::file_name(self.level, "kfxed.toml"),
                sidecar::to_toml(&self.sidecar, self.dims, &self.custom_cells).into_bytes(),
            );
        }
    }
}

/// The file without the legacy residue keys (they are only for `save_legacy`); no copy when
/// there is none.
fn strip_legacy<'a, T>(
    f: &'a FxFile<T>,
    extra: impl Fn(&mut T) -> &mut kfx_formats::level_fx::Extra,
    has: impl Fn(&T) -> bool,
) -> std::borrow::Cow<'a, FxFile<T>>
where
    T: kfx_formats::level_fx::FxItem,
    FxFile<T>: Clone,
{
    if !f.items.iter().any(has) {
        return std::borrow::Cow::Borrowed(f);
    }
    let mut c = f.clone();
    for i in c.items.iter_mut() {
        extra(i).retain(|k, _| !crate::legacy::is_legacy_key(k));
    }
    std::borrow::Cow::Owned(c)
}
