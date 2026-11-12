//! Interoperability with the custom ("fake") slabs of the Unearth map editor: reading and
//! writing its `.une` file and reading its `custom_slabs.cfg`.
//!
//! # Formats
//! * `mapNNNNN.une`: a raw grid of one little-endian `u16` per slab, row by row; `0` means no
//!   custom slab, other values are custom slab ids (from 1000).
//! * `custom_slabs.cfg`: the editor's global definitions, Godot `ConfigFile` text with one
//!   `[slab1000]` section per id holding `name`, `recognized_as`, `liquid_type`, `wibble_type`,
//!   `wibble_edges`, `bitmask`, `is_solid`, `ownable`, `edge_blend_group`, `cubes0..8` (cube index
//!   arrays, one per column) and `floor0..8`. Keys are read case-insensitively; only `name`,
//!   `recognized_as`, `liquid_type`, `cubes*` and `floor*` are used here.
//!
//! A custom slab writes its nine columns, their wibble and its `recognized_as` kind into the
//! map files, so the map is complete without the `.une`; the `.une` only says *which* slabs
//! are custom. On import ([`MapDoc::import_une`]) a definition comes from `custom_slabs.cfg`
//! when it is given and has the id, else it is rebuilt from the columns the map holds at the
//! first slab with that id (`recognized_as` = the slab kind stored there). Columns taken from
//! the map carry the owner colour of that slab; `custom_slabs.cfg` columns are the neutral
//! templates.
use crate::column::canonical_column;
use crate::gamedata::GameData;
use crate::mapdoc::MapDoc;
use crate::sidecar::{CustomSlab, FIRST_CUSTOM_ID};
use crate::{Error, Result};
use kfx_formats::level_bin::{Column, MapDims};

/// Parse a `.une` file: `dims.slabs()` little-endian `u16`s (extra bytes are ignored, a short
/// file is an error).
pub fn parse_une(bytes: &[u8], dims: MapDims) -> Result<Vec<u16>> {
    let n = dims.slabs();
    if bytes.len() < 2 * n {
        return Err(Error::Invalid(format!(
            ".une has {} bytes, a {}x{} map needs {}",
            bytes.len(),
            dims.w,
            dims.h,
            2 * n
        )));
    }
    Ok((0..n)
        .map(|i| u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]))
        .collect())
}

/// The `.une` bytes for custom cells.
pub fn write_une(cells: &[u16]) -> Vec<u8> {
    cells.iter().flat_map(|c| c.to_le_bytes()).collect()
}

/// One `[slabNNNN]` of `custom_slabs.cfg`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UneDefinition {
    pub id: u16,
    pub name: String,
    pub recognized_as: Option<u16>,
    pub liquid_type: Option<u8>,
    /// `cubes0..8`.
    pub cubes: Vec<Vec<u16>>,
    /// `floor0..8`.
    pub floor: Vec<u16>,
}

fn parse_list(v: &str) -> Vec<i64> {
    v.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .filter_map(|t| t.trim().parse().ok())
        .collect()
}

/// Parse `custom_slabs.cfg` (Godot ConfigFile text, keys and section names read
/// case-insensitively). Sections below id 1000 are not custom slabs and are ignored.
pub fn parse_custom_slabs_cfg(text: &str) -> Vec<UneDefinition> {
    let mut out: Vec<UneDefinition> = vec![];
    let mut cubes: Vec<(usize, Vec<u16>)> = vec![];
    let mut floors: Vec<(usize, u16)> = vec![];
    let flush = |out: &mut Vec<UneDefinition>,
                 cubes: &mut Vec<(usize, Vec<u16>)>,
                 floors: &mut Vec<(usize, u16)>| {
        if let Some(d) = out.last_mut() {
            cubes.sort_by_key(|c| c.0);
            floors.sort_by_key(|c| c.0);
            d.cubes = cubes.drain(..).map(|c| c.1).collect();
            d.floor = floors.drain(..).map(|c| c.1).collect();
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(sec) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            flush(&mut out, &mut cubes, &mut floors);
            if let Some(id) = sec
                .trim()
                .to_ascii_lowercase()
                .strip_prefix("slab")
                .and_then(|n| n.parse::<u32>().ok())
            {
                if id >= FIRST_CUSTOM_ID as u32 && id <= u16::MAX as u32 {
                    out.push(UneDefinition {
                        id: id as u16,
                        ..Default::default()
                    });
                    continue;
                }
            }
            // a section that is not a custom slab: push a sentinel that is dropped below
            out.push(UneDefinition {
                id: 0,
                ..Default::default()
            });
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        let Some(d) = out.last_mut() else { continue };
        match k.as_str() {
            "name" => d.name = v.trim_matches('"').to_string(),
            "recognized_as" => d.recognized_as = v.parse().ok(),
            "liquid_type" => d.liquid_type = v.parse().ok(),
            _ => {
                if let Some(i) = k
                    .strip_prefix("cubes")
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    cubes.push((
                        i,
                        parse_list(v)
                            .into_iter()
                            .map(|c| c.clamp(0, u16::MAX as i64) as u16)
                            .collect(),
                    ));
                } else if let Some(i) = k
                    .strip_prefix("floor")
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    if let Ok(f) = v.parse::<u16>() {
                        floors.push((i, f));
                    }
                }
            }
        }
    }
    flush(&mut out, &mut cubes, &mut floors);
    out.retain(|d| d.id != 0);
    out
}

/// What [`MapDoc::import_une`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UneReport {
    /// Slabs marked custom.
    pub cells: usize,
    /// Ids imported, with where the definition came from.
    pub definitions: Vec<(u16, UneSource)>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UneSource {
    /// From `custom_slabs.cfg`.
    CustomSlabsCfg,
    /// Rebuilt from the columns the map holds.
    MapColumns,
}

impl MapDoc {
    /// Import the custom slabs of a map made with the Unearth editor: `une` is the `.une` file,
    /// `custom_slabs_cfg` the text of that editor's `custom_slabs.cfg` when available. Slabs with
    /// ids that already exist in the sidecar keep the existing definition. The document's slabs /
    /// columns are not changed (the map files already contain the fake slabs' result); only the
    /// custom ids and the definitions are added. Journaled like any other edit.
    pub fn import_une(
        &mut self,
        gd: &GameData,
        une: &[u8],
        custom_slabs_cfg: Option<&str>,
    ) -> Result<UneReport> {
        let cells = parse_une(une, self.dims)?;
        let defs = custom_slabs_cfg
            .map(parse_custom_slabs_cfg)
            .unwrap_or_default();
        let d = self.dims;
        let mut report = UneReport::default();
        self.touch_props();
        let mut first: std::collections::BTreeMap<u16, usize> = Default::default();
        for (i, &id) in cells.iter().enumerate() {
            if id != 0 {
                first.entry(id).or_insert(i);
            }
        }
        for (&id, &i) in &first {
            if self.sidecar.custom_slab(id).is_some() {
                continue;
            }
            let (x, y) = (i % d.w as usize, i / d.w as usize);
            let from_map = |me: &MapDoc| -> ([Column; 9], [u8; 9]) {
                let mut cols = [Column::default(); 9];
                let mut wib = [0u8; 9];
                for k in 0..9 {
                    let s = d.stl_index(3 * x + k % 3, 3 * y + k / 3);
                    cols[k] = canonical_column(&me.columns[s], true);
                    wib[k] = me.wibble[s];
                }
                (cols, wib)
            };
            let (map_cols, wib) = from_map(self);
            let kind_here = self.kinds[i];
            let cfg = defs.iter().find(|u| u.id == id);
            let (columns, source) = match cfg {
                Some(u) if u.cubes.len() == 9 => {
                    let mut cols = [Column::default(); 9];
                    for k in 0..9 {
                        let mut c = Column {
                            floor_texture: u.floor.get(k).copied().unwrap_or(0),
                            ..Default::default()
                        };
                        for (j, cube) in u.cubes[k].iter().take(8).enumerate() {
                            c.cubes[j] = *cube;
                        }
                        cols[k] = canonical_column(&c, true);
                    }
                    (cols, UneSource::CustomSlabsCfg)
                }
                _ => {
                    if cfg.is_some() {
                        report.warnings.push(format!(
                            "fake slab {id}: custom_slabs.cfg entry lacks \
                            nine cube lists; using the map's columns"
                        ));
                    } else if custom_slabs_cfg.is_some() {
                        report.warnings.push(format!(
                            "fake slab {id}: not in custom_slabs.cfg; \
                            rebuilt from the map's columns at ({x},{y})"
                        ));
                    }
                    (map_cols, UneSource::MapColumns)
                }
            };
            let recognized = cfg.and_then(|u| u.recognized_as).unwrap_or(kind_here);
            if gd.slab_count() > 0 && recognized as usize >= gd.slab_count() {
                report.warnings.push(format!(
                    "fake slab {id}: recognized_as {recognized} is not a slab of terrain.cfg"
                ));
            }
            self.sidecar.custom_slabs.push(CustomSlab {
                id,
                name: cfg
                    .map(|u| u.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("Fake slab {id}")),
                recognized_as: recognized,
                liquid: cfg.and_then(|u| u.liquid_type),
                columns,
                wibble: wib,
                objects: vec![],
            });
            report.definitions.push((id, source));
        }
        for (i, &id) in cells.iter().enumerate() {
            if id != 0 {
                self.j_slab(i);
                self.custom_cells[i] = id;
                report.cells += 1;
            }
        }
        Ok(report)
    }

    /// The `.une` bytes of the current custom cells.
    pub fn export_une(&self) -> Vec<u8> {
        write_une(&self.custom_cells)
    }
}
