//! Editor-only data stored next to a map: hand-built ("custom" / "fake") slabs and per-map
//! editor settings, in a TOML sidecar `mapNNNNN.kfxed.toml` that the game never reads (the
//! game only opens the `mapNNNNN.<ext>` files it knows; `load_level_file`,
//! `src/lvl_filesdk1.c:1422`).
//!
//! # Custom slabs
//! A custom slab is nine arbitrary columns (plus optional objects / lights) that stand in for a
//! slab. The `.slb` file holds [`CustomSlab::recognized_as`] (the slab kind the game and the
//! neighbours see), the `.clm` / `.dat` hold the nine columns, and the sidecar remembers *which*
//! slabs are custom and their definition, so that regenerating the neighbours (or the whole
//! map) keeps them: a custom slab is never regenerated, placing one writes its nine columns
//! and `recognized_as` into the slab layer. Both the definitions and the per-slab ids travel
//! with the map. The Unearth editor's "fake slabs" are the same idea, stored in its own
//! `custom_slabs.cfg` and a per-map `.une` file; [`crate::unearth`] imports and writes those.
//!
//! # Other settings
//! Decoration seed / toggles, symmetry, locked regions and notes pinned to slabs.
//!
//! The sidecar is part of the undoable document state (`Props` of [`crate::patch`]); the
//! per-slab custom ids are a slab layer (`MapDoc::custom_cells`).
use crate::column::{canonical_column, COLUMN_STACK_HEIGHT};
use crate::transform::{Symmetry, SymmetryMode, Transform};
use crate::{Error, Result};
use kfx_formats::level_bin::{Column, MapDims};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Custom slab ids start here (the first id of the `.une` format, see [`crate::unearth`]).
pub const FIRST_CUSTOM_ID: u16 = 1000;
/// Format version written to the sidecar.
pub const SIDECAR_VERSION: u32 = 1;

/// Path of the sidecar of level `level` in `folder`: `mapNNNNN.kfxed.toml`.
pub fn sidecar_path(folder: &Path, level: u32) -> PathBuf {
    folder.join(format!("map{level:05}.kfxed.toml"))
}

/// What a custom slab object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomObjectKind {
    Object {
        model: i16,
    },
    /// `range` in map coordinates (`EffectRange * 256`).
    EffectGen {
        model: i16,
        range: i32,
    },
    /// `radius` in map coordinates.
    Light {
        intensity: i16,
        radius: i32,
    },
}

/// An object, effect generator or light that belongs to a custom slab. It is created / removed
/// together with the slab like the slabset objects of a normal slab (`place_slab_object`,
/// src/map_blocks.c:1318; they are skipped when their subtile is `BLOCKING`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CustomObject {
    pub kind: CustomObjectKind,
    /// Offset from the slab's north-west corner in map coordinates (0..768).
    pub dx: i32,
    pub dy: i32,
    pub z: i32,
    pub orientation: i16,
}

/// A hand-built slab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomSlab {
    pub id: u16,
    pub name: String,
    /// The slab kind stored in `.slb` and seen by the game and the neighbours.
    pub recognized_as: u16,
    /// `.wlb` value; `None` = the `WlbType` of `recognized_as`.
    pub liquid: Option<u8>,
    /// The nine columns (row-major, index = dy*3 + dx), as templates: cubes of an ownership
    /// group are recoloured for the slab's owner like slabset columns (`owner_column`).
    pub columns: [Column; 9],
    pub wibble: [u8; 9],
    pub objects: Vec<CustomObject>,
}

impl CustomSlab {
    /// A custom slab from nine columns (canonicalised: solid mask and heights recomputed).
    pub fn new(name: &str, recognized_as: u16, columns: [Column; 9]) -> CustomSlab {
        CustomSlab {
            id: 0,
            name: name.to_string(),
            recognized_as,
            liquid: None,
            columns: columns.map(|c| canonical_column(&c, true)),
            wibble: [0; 9],
            objects: vec![],
        }
    }
}

/// Decoration (cube variant randomisation, [`crate::decorate`]) settings of the map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecorationSettings {
    /// `None` = derive from the level number ([`crate::decorate::Decorator::seed_for_level`]).
    pub seed: Option<u64>,
    pub enabled: bool,
    /// Cube / floor families switched off (names as in `Decorator::set_family`).
    pub disabled_families: Vec<String>,
}

impl Default for DecorationSettings {
    fn default() -> Self {
        DecorationSettings {
            seed: None,
            enabled: true,
            disabled_families: vec![],
        }
    }
}

/// A rectangle of slabs the editor refuses to modify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockedRegion {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub note: String,
}

impl LockedRegion {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

/// A note pinned to a slab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub x: i32,
    pub y: i32,
    pub text: String,
    /// Optional CSS-style colour (`#rrggbb`) for the GUI.
    pub color: Option<String>,
}

/// Everything the sidecar stores except the per-slab custom ids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sidecar {
    pub custom_slabs: Vec<CustomSlab>,
    pub decoration: DecorationSettings,
    pub symmetry: Option<Symmetry>,
    pub locked: Vec<LockedRegion>,
    pub notes: Vec<Note>,
    /// The format the editor saves this map in: `Some("fx")` (KeeperFX `.tngfx` / `.aptfx` /
    /// `.lgtfx`) or `Some("legacy")` (original `.tng` / `.apt` / `.lgt`); `None` = editor default.
    pub save_format: Option<String>,
}

impl Sidecar {
    pub fn is_empty(&self) -> bool {
        *self == Sidecar::default()
    }
    pub fn custom_slab(&self, id: u16) -> Option<&CustomSlab> {
        self.custom_slabs.iter().find(|c| c.id == id)
    }
    /// The smallest id >= [`FIRST_CUSTOM_ID`] not used yet.
    pub fn free_custom_id(&self) -> u16 {
        (FIRST_CUSTOM_ID..u16::MAX)
            .find(|i| self.custom_slab(*i).is_none())
            .unwrap_or(u16::MAX)
    }
    /// Add a definition; an `id` of 0 (or one already taken) gets a free id. Returns the id.
    pub fn add_custom_slab(&mut self, mut def: CustomSlab) -> u16 {
        if def.id < FIRST_CUSTOM_ID || self.custom_slab(def.id).is_some() {
            def.id = self.free_custom_id();
        }
        let id = def.id;
        self.custom_slabs.push(def);
        id
    }
    /// Remove a definition (slabs still using it become undefined; see `validate`).
    pub fn remove_custom_slab(&mut self, id: u16) -> Option<CustomSlab> {
        let i = self.custom_slabs.iter().position(|c| c.id == id)?;
        Some(self.custom_slabs.remove(i))
    }
    /// The id of custom slab `id` after `t` (flip / rotation of the slab it stands on): its nine
    /// columns, wibble and objects are moved to where the transform puts them (cube faces are
    /// direction specific and are not rotated; hand-fix them if they matter). An existing
    /// definition with the same content is reused, else a new one named `"<name> [<t>]"` is added.
    /// Returns `id` itself when the transform changes nothing or `id` is undefined.
    pub fn transformed_custom_slab(&mut self, id: u16, t: Transform) -> u16 {
        let Some(src) = self.custom_slab(id).cloned() else {
            return id;
        };
        let mut out = src.clone();
        for y in 0..3i64 {
            for x in 0..3i64 {
                let (nx, ny) = t.slab(3, 3, x, y);
                let (a, b) = ((y * 3 + x) as usize, (ny * 3 + nx) as usize);
                out.columns[b] = src.columns[a];
                out.wibble[b] = src.wibble[a];
            }
        }
        for o in &mut out.objects {
            let (a, b) = t.raw(768, 768, o.dx as i64, o.dy as i64);
            o.dx = a.clamp(0, 767) as i32;
            o.dy = b.clamp(0, 767) as i32;
            o.orientation = t.angle(o.orientation as i64) as i16;
        }
        let same = |a: &CustomSlab, b: &CustomSlab| {
            a.recognized_as == b.recognized_as
                && a.liquid == b.liquid
                && a.columns == b.columns
                && a.wibble == b.wibble
                && a.objects == b.objects
        };
        if same(&src, &out) {
            return id;
        }
        if let Some(e) = self.custom_slabs.iter().find(|c| same(c, &out)) {
            return e.id;
        }
        out.name = format!("{} [{t:?}]", src.name);
        out.id = 0;
        self.add_custom_slab(out)
    }

    /// The locked region that contains slab (x, y), if any.
    pub fn locked_at(&self, x: i32, y: i32) -> Option<&LockedRegion> {
        self.locked.iter().find(|l| l.contains(x, y))
    }
    pub fn notes_at(&self, x: i32, y: i32) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(move |n| n.x == x && n.y == y)
    }
}

// ---------------------------------------------------------------------------------------------
// TOML

fn mode_name(m: SymmetryMode) -> &'static str {
    match m {
        SymmetryMode::NorthSouth => "north-south",
        SymmetryMode::EastWest => "east-west",
        SymmetryMode::FourWay => "four-way",
        SymmetryMode::Rot2 => "rot2",
        SymmetryMode::Rot4 => "rot4",
    }
}

fn mode_of(s: &str) -> Option<SymmetryMode> {
    Some(match s {
        "north-south" => SymmetryMode::NorthSouth,
        "east-west" => SymmetryMode::EastWest,
        "four-way" => SymmetryMode::FourWay,
        "rot2" => SymmetryMode::Rot2,
        "rot4" => SymmetryMode::Rot4,
        _ => return None,
    })
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
struct FileDecoration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    disabled_families: Vec<String>,
}

impl Default for FileDecoration {
    fn default() -> Self {
        FileDecoration {
            seed: None,
            enabled: true,
            disabled_families: vec![],
        }
    }
}

#[derive(Serialize, Deserialize)]
struct FileSymmetry {
    mode: String,
    #[serde(default)]
    owners: Vec<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
struct FileColumn {
    cubes: Vec<u16>,
    #[serde(default)]
    floor: u16,
    #[serde(default)]
    orient: u8,
    #[serde(default)]
    wibble: u8,
}

#[derive(Serialize, Deserialize)]
struct FileObject {
    /// `object`, `effect` or `light`.
    kind: String,
    #[serde(default)]
    model: i16,
    #[serde(default)]
    range: i32,
    #[serde(default)]
    intensity: i16,
    #[serde(default)]
    radius: i32,
    dx: i32,
    dy: i32,
    #[serde(default)]
    z: i32,
    #[serde(default)]
    orientation: i16,
}

#[derive(Serialize, Deserialize)]
struct FileCustomSlab {
    id: u16,
    #[serde(default)]
    name: String,
    recognized_as: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    liquid: Option<u8>,
    column: Vec<FileColumn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    object: Vec<FileObject>,
}

#[derive(Serialize, Deserialize)]
struct FileLocked {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    #[serde(default)]
    note: String,
}

#[derive(Serialize, Deserialize)]
struct FileNote {
    x: i32,
    y: i32,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileCells {
    /// `[x, y, id]` triples.
    #[serde(default)]
    cells: Vec<[u32; 3]>,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    save_format: Option<String>,
    #[serde(default)]
    decoration: FileDecoration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    symmetry: Option<FileSymmetry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    locked: Vec<FileLocked>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    note: Vec<FileNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    custom_slab: Vec<FileCustomSlab>,
    #[serde(default)]
    custom_cells: FileCells,
}

fn bad(path: &Path, msg: impl Into<String>) -> Error {
    Error::Toml {
        path: path.to_path_buf(),
        msg: msg.into(),
    }
}

/// Serialise the sidecar and the custom slab ids of `dims` (`cells`, row-major, 0 = none).
pub fn to_toml(s: &Sidecar, dims: MapDims, cells: &[u16]) -> String {
    let file = File {
        version: SIDECAR_VERSION,
        save_format: s.save_format.clone(),
        decoration: FileDecoration {
            seed: s.decoration.seed,
            enabled: s.decoration.enabled,
            disabled_families: s.decoration.disabled_families.clone(),
        },
        symmetry: s.symmetry.as_ref().map(|y| FileSymmetry {
            mode: mode_name(y.mode).to_string(),
            owners: y.owners.iter().map(|o| o.to_vec()).collect(),
        }),
        locked: s
            .locked
            .iter()
            .map(|l| FileLocked {
                x0: l.x0,
                y0: l.y0,
                x1: l.x1,
                y1: l.y1,
                note: l.note.clone(),
            })
            .collect(),
        note: s
            .notes
            .iter()
            .map(|n| FileNote {
                x: n.x,
                y: n.y,
                text: n.text.clone(),
                color: n.color.clone(),
            })
            .collect(),
        custom_slab: s
            .custom_slabs
            .iter()
            .map(|c| FileCustomSlab {
                id: c.id,
                name: c.name.clone(),
                recognized_as: c.recognized_as,
                liquid: c.liquid,
                column: c
                    .columns
                    .iter()
                    .zip(c.wibble)
                    .map(|(col, wibble)| FileColumn {
                        cubes: col.cubes.to_vec(),
                        floor: col.floor_texture,
                        orient: col.orient,
                        wibble,
                    })
                    .collect(),
                object: c
                    .objects
                    .iter()
                    .map(|o| {
                        let mut f = FileObject {
                            kind: String::new(),
                            model: 0,
                            range: 0,
                            intensity: 0,
                            radius: 0,
                            dx: o.dx,
                            dy: o.dy,
                            z: o.z,
                            orientation: o.orientation,
                        };
                        match o.kind {
                            CustomObjectKind::Object { model } => {
                                f.kind = "object".into();
                                f.model = model;
                            }
                            CustomObjectKind::EffectGen { model, range } => {
                                f.kind = "effect".into();
                                f.model = model;
                                f.range = range;
                            }
                            CustomObjectKind::Light { intensity, radius } => {
                                f.kind = "light".into();
                                f.intensity = intensity;
                                f.radius = radius;
                            }
                        }
                        f
                    })
                    .collect(),
            })
            .collect(),
        custom_cells: FileCells {
            cells: (0..dims.slabs())
                .filter(|&i| cells.get(i).is_some_and(|&c| c != 0))
                .map(|i| {
                    [
                        (i % dims.w as usize) as u32,
                        (i / dims.w as usize) as u32,
                        cells[i] as u32,
                    ]
                })
                .collect(),
        },
    };
    let mut out = String::from(
        "# KeeperFX editor sidecar: editor-only data of this map. The game ignores this file.\n",
    );
    out.push_str(&toml::to_string(&file).unwrap_or_default());
    out
}

/// Parse a sidecar. Cells outside `dims` are dropped; the returned list holds the problems.
pub fn from_toml(
    path: &Path,
    text: &str,
    dims: MapDims,
) -> Result<(Sidecar, Vec<u16>, Vec<String>)> {
    let f: File = toml::from_str(text).map_err(|e| bad(path, e.to_string()))?;
    let mut warnings = vec![];
    if f.version > SIDECAR_VERSION {
        warnings.push(format!(
            "{}: sidecar version {} is newer than {SIDECAR_VERSION}",
            path.display(),
            f.version
        ));
    }
    let mut s = Sidecar {
        decoration: DecorationSettings {
            seed: f.decoration.seed,
            enabled: f.decoration.enabled,
            disabled_families: f.decoration.disabled_families,
        },
        save_format: f.save_format,
        ..Default::default()
    };
    if let Some(y) = f.symmetry {
        let mode = mode_of(&y.mode)
            .ok_or_else(|| bad(path, format!("unknown symmetry mode {:?}", y.mode)))?;
        let mut sym = Symmetry::new(mode);
        for (i, o) in y.owners.iter().enumerate().take(sym.owners.len()) {
            if o.len() == 9 {
                sym.owners[i].copy_from_slice(o);
            } else {
                warnings.push(format!(
                    "symmetry owners row {i} has {} entries, expected 9",
                    o.len()
                ));
            }
        }
        s.symmetry = Some(sym);
    }
    s.locked = f
        .locked
        .into_iter()
        .map(|l| LockedRegion {
            x0: l.x0,
            y0: l.y0,
            x1: l.x1,
            y1: l.y1,
            note: l.note,
        })
        .collect();
    s.notes = f
        .note
        .into_iter()
        .map(|n| Note {
            x: n.x,
            y: n.y,
            text: n.text,
            color: n.color,
        })
        .collect();
    for c in f.custom_slab {
        if c.column.len() != 9 {
            return Err(bad(
                path,
                format!(
                    "custom slab {} has {} columns, expected 9",
                    c.id,
                    c.column.len()
                ),
            ));
        }
        let mut columns = [Column::default(); 9];
        let mut wibble = [0u8; 9];
        for (i, fc) in c.column.iter().enumerate() {
            if fc.cubes.len() > COLUMN_STACK_HEIGHT {
                return Err(bad(
                    path,
                    format!(
                        "custom slab {} column {i}: more than {COLUMN_STACK_HEIGHT} cubes",
                        c.id
                    ),
                ));
            }
            let mut col = Column {
                floor_texture: fc.floor,
                orient: fc.orient,
                ..Default::default()
            };
            col.cubes[..fc.cubes.len()].copy_from_slice(&fc.cubes);
            columns[i] = canonical_column(&col, true);
            wibble[i] = fc.wibble;
        }
        let mut objects = vec![];
        for o in c.object {
            let kind = match o.kind.as_str() {
                "object" => CustomObjectKind::Object { model: o.model },
                "effect" => CustomObjectKind::EffectGen {
                    model: o.model,
                    range: o.range,
                },
                "light" => CustomObjectKind::Light {
                    intensity: o.intensity,
                    radius: o.radius,
                },
                k => {
                    return Err(bad(
                        path,
                        format!("custom slab {}: unknown object kind {k:?}", c.id),
                    ))
                }
            };
            objects.push(CustomObject {
                kind,
                dx: o.dx,
                dy: o.dy,
                z: o.z,
                orientation: o.orientation,
            });
        }
        if s.custom_slab(c.id).is_some() {
            warnings.push(format!(
                "custom slab id {} defined twice; the first one is kept",
                c.id
            ));
            continue;
        }
        s.custom_slabs.push(CustomSlab {
            id: c.id,
            name: c.name,
            recognized_as: c.recognized_as,
            liquid: c.liquid,
            columns,
            wibble,
            objects,
        });
    }
    let mut cells = vec![0u16; dims.slabs()];
    for [x, y, id] in f.custom_cells.cells {
        if x >= dims.w as u32 || y >= dims.h as u32 || id > u16::MAX as u32 {
            warnings.push(format!(
                "custom cell ({x},{y}) = {id} outside the {}x{} map",
                dims.w, dims.h
            ));
            continue;
        }
        cells[dims.slab_index(x as usize, y as usize)] = id as u16;
    }
    Ok((s, cells, warnings))
}

/// Read the sidecar file at `path`.
pub fn load(path: &Path, dims: MapDims) -> Result<(Sidecar, Vec<u16>, Vec<String>)> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    from_toml(path, &text, dims)
}
