//! Legacy binary per-level files
//! (`mapNNNNN.slb/.own/.dat/.clm/.wib/.wlb/.flg/.inf/.slx/.tng/.apt/.lgt`).
//!
//! Reference: KeeperFX `src/lvl_filesdk1.c` (`load_level_file`, line ~1422, and the individual
//! loaders) and `src/map_columns.h` (`struct Column`, packed, 24 bytes).
//!
//! Every file type is one struct with `parse(&[u8], MapDims)` and `to_bytes()`. The pair is
//! lossless: `to_bytes(parse(x)) == x` for any `x` that `parse` accepts. Anything the game ignores
//! (ignored header words, unused struct fields, bytes beyond the expected size) is kept.
//!
//! Size policy, from `load_single_map_file_to_buffer` (lvl_filesdk1.c:~125-160): the game rejects a
//! file smaller than the expected size and accepts a larger one, ignoring the excess. `parse`
//! mirrors that: too small is an error, excess lands in `trailing`.
//!
//! Subtile grids are `(3W+1) x (3H+1)`: one extra row and column beyond the `3W x 3H` playable
//! area (src/lvl_filesdk1.c:703, 1071, 1226).
use crate::{rnc, Error, Result};
use std::path::{Path, PathBuf};

/// Map size in slabs (tiles). The game default is 85x85, the maximum 170x170
/// (`game.map_tiles_x/y`, `game.map_subtiles_x/y = 3 * tiles`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapDims {
    pub w: u16,
    pub h: u16,
}

impl MapDims {
    pub fn new(w: u16, h: u16) -> Self {
        Self { w, h }
    }
    /// Infer square dims from the byte length of an `.slb` (2 bytes per slab). `None` when the
    /// length is odd or `len/2` is not a perfect square.
    pub fn from_slb_len(len: usize) -> Option<Self> {
        if len % 2 != 0 {
            return None;
        }
        let n = len / 2;
        let s = (n as f64).sqrt().round() as usize;
        (s > 0 && s * s == n && s <= u16::MAX as usize).then(|| Self::new(s as u16, s as u16))
    }
    /// Slab count `W*H`.
    pub fn slabs(&self) -> usize {
        self.w as usize * self.h as usize
    }
    /// Playable subtile width `SW = 3W`.
    pub fn stl_w(&self) -> usize {
        3 * self.w as usize
    }
    /// Playable subtile height `SH = 3H`.
    pub fn stl_h(&self) -> usize {
        3 * self.h as usize
    }
    /// Subtile grid cell count including the extra row/column, `(SW+1)*(SH+1)`.
    pub fn stl_grid(&self) -> usize {
        (self.stl_w() + 1) * (self.stl_h() + 1)
    }
    /// Index into a slab grid.
    pub fn slab_index(&self, x: usize, y: usize) -> usize {
        y * self.w as usize + x
    }
    /// Index into a `(SW+1) x (SH+1)` subtile grid.
    pub fn stl_index(&self, x: usize, y: usize) -> usize {
        y * (self.stl_w() + 1) + x
    }
}

fn fmt_err(what: &'static str, msg: impl Into<String>) -> Error {
    Error::format(what, msg)
}

/// Split `data` at `expected` bytes; error when shorter.
fn split_expected<'a>(
    what: &'static str,
    data: &'a [u8],
    expected: usize,
) -> Result<(&'a [u8], &'a [u8])> {
    if data.len() < expected {
        return Err(fmt_err(
            what,
            format!("file is {} bytes, expected at least {expected}", data.len()),
        ));
    }
    Ok(data.split_at(expected))
}

fn u16s(b: &[u8]) -> Vec<u16> {
    b.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn push_u16s(out: &mut Vec<u8>, v: &[u16]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

/// Decode `[u32|u16 count][hdr_extra bytes][records...]`. Mirrors the game's count validation
/// (src/lvl_filesdk1.c:748-755): a count larger than what fits is corrected to what fits. Returns
/// (declared count, record bytes, trailing). `declared` is `Some` only when it differs from the
/// number of records kept.
fn counted<'a>(
    what: &'static str,
    data: &'a [u8],
    count_len: usize,
    hdr_len: usize,
    rec_len: usize,
) -> Result<(Option<u32>, &'a [u8], &'a [u8])> {
    if data.len() < hdr_len {
        return Err(fmt_err(
            what,
            format!("file is {} bytes, expected at least {hdr_len}", data.len()),
        ));
    }
    let declared = if count_len == 2 {
        u16::from_le_bytes([data[0], data[1]]) as u32
    } else {
        u32::from_le_bytes([data[0], data[1], data[2], data[3]])
    };
    let body = &data[hdr_len..];
    let n = (declared as usize).min(body.len() / rec_len);
    let (recs, trailing) = body.split_at(n * rec_len);
    Ok((
        ((declared as usize) != n).then_some(declared),
        recs,
        trailing,
    ))
}

fn put_count(out: &mut Vec<u8>, count_len: usize, declared: Option<u32>, n: usize) {
    let c = declared.unwrap_or(n as u32);
    if count_len == 2 {
        out.extend_from_slice(&(c as u16).to_le_bytes());
    } else {
        out.extend_from_slice(&c.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------------------------
// Slab grids

/// `.slb`: slab kind per slab, `u16` each, row-major, `2*W*H` bytes (`load_map_slab_file`,
/// src/lvl_filesdk1.c:1189). Kinds above the configured slab count become rock in the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slb {
    pub kinds: Vec<u16>,
    pub trailing: Vec<u8>,
}

impl Slb {
    pub fn parse(data: &[u8], dims: MapDims) -> Result<Self> {
        let (b, t) = split_expected("slb", data, 2 * dims.slabs())?;
        Ok(Self {
            kinds: u16s(b),
            trailing: t.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(self.kinds.len() * 2 + self.trailing.len());
        push_u16s(&mut o, &self.kinds);
        o.extend_from_slice(&self.trailing);
        o
    }
}

/// `.slx`: extra per-slab byte, `W*H` bytes (`load_ext_slabs`, src/lvl_filesdk1.c:1323). The game
/// discards the data entirely when the unpacked size is not exactly `W*H`; `parse` keeps excess in
/// `trailing`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slx {
    pub values: Vec<u8>,
    pub trailing: Vec<u8>,
}

impl Slx {
    pub fn parse(data: &[u8], dims: MapDims) -> Result<Self> {
        let (b, t) = split_expected("slx", data, dims.slabs())?;
        Ok(Self {
            values: b.to_vec(),
            trailing: t.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.values.as_slice(), &self.trailing].concat()
    }
}

/// `.wlb` water/lava/bridge type per slab, `W*H` bytes, optional file (`load_map_wlb_file`,
/// src/lvl_filesdk1.c:1141). Values are the `WlbT_*` constants below; the game resets values that
/// disagree with the slab kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wlb {
    pub values: Vec<u8>,
    pub trailing: Vec<u8>,
}

impl Wlb {
    pub const NONE: u8 = 0;
    pub const LAVA: u8 = 1;
    pub const WATER: u8 = 2;
    pub const BRIDGE: u8 = 3;
    pub const ABYSS: u8 = 4;
    pub fn parse(data: &[u8], dims: MapDims) -> Result<Self> {
        let (b, t) = split_expected("wlb", data, dims.slabs())?;
        Ok(Self {
            values: b.to_vec(),
            trailing: t.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.values.as_slice(), &self.trailing].concat()
    }
}

// ---------------------------------------------------------------------------------------------
// Subtile grids

/// `.own`: owner byte per subtile, `(SW+1)*(SH+1)` bytes (`load_map_ownership_file`,
/// src/lvl_filesdk1.c:1071). The game calls `set_slab_owner(subtile_slab(x), subtile_slab(y), byte)`
/// for every playable subtile in row-major order, so the slab owner is the byte of the *last*
/// subtile visited in that slab, i.e. subtile (3x+2, 3y+2). The extra row/column become neutral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Own {
    pub owners: Vec<u8>,
    pub trailing: Vec<u8>,
}

impl Own {
    pub fn parse(data: &[u8], dims: MapDims) -> Result<Self> {
        let (b, t) = split_expected("own", data, dims.stl_grid())?;
        Ok(Self {
            owners: b.to_vec(),
            trailing: t.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.owners.as_slice(), &self.trailing].concat()
    }
    /// Owner of slab (x, y) as the game derives it: the byte at subtile (3x+2, 3y+2).
    pub fn slab_owner(&self, dims: MapDims, x: usize, y: usize) -> Option<u8> {
        self.owners
            .get(dims.stl_index(3 * x + 2, 3 * y + 2))
            .copied()
    }
}

/// `.wib`: wibble (water/lava animation) value per subtile, `(SW+1)*(SH+1)` bytes
/// (`load_map_wibble_file`, lvl_filesdk1.c:~1050).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wib {
    pub values: Vec<u8>,
    pub trailing: Vec<u8>,
}

impl Wib {
    pub fn parse(data: &[u8], dims: MapDims) -> Result<Self> {
        let (b, t) = split_expected("wib", data, dims.stl_grid())?;
        Ok(Self {
            values: b.to_vec(),
            trailing: t.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.values.as_slice(), &self.trailing].concat()
    }
}

/// `.dat`: column reference per subtile, `u16` each, `2*(SW+1)*(SH+1)` bytes
/// (`load_map_data_file`, src/lvl_filesdk1.c:703). The game stores `col_idx = -lword(value)`, so the
/// file holds the negated column index; use [`Dat::column_index`] / [`Dat::set_column_index`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dat {
    /// Raw on-disk values.
    pub raw: Vec<u16>,
    pub trailing: Vec<u8>,
}

impl Dat {
    pub fn parse(data: &[u8], dims: MapDims) -> Result<Self> {
        let (b, t) = split_expected("dat", data, 2 * dims.stl_grid())?;
        Ok(Self {
            raw: u16s(b),
            trailing: t.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(self.raw.len() * 2 + self.trailing.len());
        push_u16s(&mut o, &self.raw);
        o.extend_from_slice(&self.trailing);
        o
    }
    /// Column index for grid cell `i` (`-(int16)raw`), an index into `.clm` columns.
    pub fn column_index(&self, i: usize) -> Option<i32> {
        self.raw.get(i).map(|&v| -(v as i16 as i32))
    }
    /// Store column index `idx` (0..=32768) into cell `i`.
    pub fn set_column_index(&mut self, i: usize, idx: u16) {
        if let Some(v) = self.raw.get_mut(i) {
            *v = (idx as i16).wrapping_neg() as u16;
        }
    }
}

/// `.flg`: map flags per subtile, `u16` each, `2*(SW+1)*(SH+1)` bytes, optional file
/// (`load_map_flag_file`, src/lvl_filesdk1.c:1226). The game reads only the low byte of each word
/// (`mapblk->flags = buf[i]; i += 2`); the high byte is preserved here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flg {
    pub raw: Vec<u16>,
    pub trailing: Vec<u8>,
}

impl Flg {
    pub fn parse(data: &[u8], dims: MapDims) -> Result<Self> {
        let (b, t) = split_expected("flg", data, 2 * dims.stl_grid())?;
        Ok(Self {
            raw: u16s(b),
            trailing: t.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(self.raw.len() * 2 + self.trailing.len());
        push_u16s(&mut o, &self.raw);
        o.extend_from_slice(&self.trailing);
        o
    }
}

// ---------------------------------------------------------------------------------------------
// Columns

/// One column, `struct Column` (src/map_columns.h:38, packed, 24 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Column {
    /// Use count (`short use`); the game recomputes it.
    pub use_count: i16,
    /// Packed: bit 0 active, bits 1-3 ceiling height, bits 4-7 floor height (`ColumnFlags`,
    /// src/map_columns.h:52). The game recomputes the floor part on load
    /// (`set_column_floor_filled_subtiles`).
    pub bitfields: u8,
    pub solidmask: u16,
    pub floor_texture: u16,
    pub orient: u8,
    /// `COLUMN_STACK_HEIGHT` (8) cube indices, bottom to top.
    pub cubes: [u16; 8],
}

impl Column {
    pub const SIZE: usize = 24;
    fn from_bytes(b: &[u8]) -> Self {
        let w = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let mut cubes = [0u16; 8];
        for (i, c) in cubes.iter_mut().enumerate() {
            *c = w(8 + 2 * i);
        }
        Column {
            use_count: w(0) as i16,
            bitfields: b[2],
            solidmask: w(3),
            floor_texture: w(5),
            orient: b[7],
            cubes,
        }
    }
    fn write(&self, o: &mut Vec<u8>) {
        o.extend_from_slice(&self.use_count.to_le_bytes());
        o.push(self.bitfields);
        o.extend_from_slice(&self.solidmask.to_le_bytes());
        o.extend_from_slice(&self.floor_texture.to_le_bytes());
        o.push(self.orient);
        push_u16s(o, &self.cubes);
    }
}

/// `.clm`: `u32 count`, `u32 ignored`, then `count` x 24-byte [`Column`] (`load_column_file`,
/// src/lvl_filesdk1.c:666). A count larger than fits is corrected by the game to `(size-8)/24`. The
/// game also caps at `COLUMNS_COUNT`; not applied here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clm {
    /// Second header word, ignored by the game ("The second lot of 4 bytes here are ignored").
    pub ignored: u32,
    pub columns: Vec<Column>,
    /// Header count when it differs from `columns.len()` (a file whose count is wrong).
    /// `None` means "write `columns.len()`".
    pub declared_count: Option<u32>,
    pub trailing: Vec<u8>,
}

impl Clm {
    pub fn parse(data: &[u8], _dims: MapDims) -> Result<Self> {
        if data.len() < 8 {
            return Err(fmt_err(
                "clm",
                format!("file is {} bytes, expected at least 8", data.len()),
            ));
        }
        let ignored = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let (declared, recs, trailing) = counted("clm", data, 4, 8, Column::SIZE)?;
        Ok(Self {
            ignored,
            columns: recs
                .chunks_exact(Column::SIZE)
                .map(Column::from_bytes)
                .collect(),
            declared_count: declared,
            trailing: trailing.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(8 + self.columns.len() * Column::SIZE + self.trailing.len());
        put_count(&mut o, 4, self.declared_count, self.columns.len());
        o.extend_from_slice(&self.ignored.to_le_bytes());
        for c in &self.columns {
            c.write(&mut o);
        }
        o.extend_from_slice(&self.trailing);
        o
    }
}

// ---------------------------------------------------------------------------------------------
// Info

/// `.inf`: byte 0 is the texture set id (`load_and_setup_map_info`, src/lvl_filesdk1.c:1309). Any
/// further bytes are ignored by the game and kept in `trailing`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inf {
    pub texture_id: u8,
    pub trailing: Vec<u8>,
}

impl Inf {
    pub fn parse(data: &[u8], _dims: MapDims) -> Result<Self> {
        let (b, t) = split_expected("inf", data, 1)?;
        Ok(Self {
            texture_id: b[0],
            trailing: t.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = vec![self.texture_id];
        o.extend_from_slice(&self.trailing);
        o
    }
}

// ---------------------------------------------------------------------------------------------
// Things, action points, lights

/// One legacy thing record, `struct LegacyInitThing` (src/lvl_filesdk1.c:89, 21 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Thing {
    /// Position words: high byte = subtile, low byte = fraction within the subtile (so
    /// `x >> 8` is the subtile coordinate).
    pub x: u16,
    pub y: u16,
    pub z: u16,
    /// Thing class (`oclass`).
    pub class: u8,
    pub model: u8,
    pub owner: u8,
    pub range: u16,
    pub index: u16,
    pub params: [u8; 8],
}

impl Thing {
    pub const SIZE: usize = 21;
    pub fn subtile_x(&self) -> u8 {
        (self.x >> 8) as u8
    }
    pub fn subtile_y(&self) -> u8 {
        (self.y >> 8) as u8
    }
    fn from_bytes(b: &[u8]) -> Self {
        let w = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        Thing {
            x: w(0),
            y: w(2),
            z: w(4),
            class: b[6],
            model: b[7],
            owner: b[8],
            range: w(9),
            index: w(11),
            params: b[13..21].try_into().unwrap(),
        }
    }
    fn write(&self, o: &mut Vec<u8>) {
        push_u16s(o, &[self.x, self.y, self.z]);
        o.extend_from_slice(&[self.class, self.model, self.owner]);
        push_u16s(o, &[self.range, self.index]);
        o.extend_from_slice(&self.params);
    }
}

/// `.tng`: `u16 count`, then 21-byte [`Thing`] records (`load_thing_file`, src/lvl_filesdk1.c:740).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tng {
    pub things: Vec<Thing>,
    /// Header count when it differs from `things.len()`; `None` writes `things.len()`.
    pub declared_count: Option<u32>,
    pub trailing: Vec<u8>,
}

impl Tng {
    pub fn parse(data: &[u8], _dims: MapDims) -> Result<Self> {
        let (declared, recs, trailing) = counted("tng", data, 2, 2, Thing::SIZE)?;
        Ok(Self {
            things: recs
                .chunks_exact(Thing::SIZE)
                .map(Thing::from_bytes)
                .collect(),
            declared_count: declared,
            trailing: trailing.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(2 + self.things.len() * Thing::SIZE + self.trailing.len());
        put_count(&mut o, 2, self.declared_count, self.things.len());
        for t in &self.things {
            t.write(&mut o);
        }
        o.extend_from_slice(&self.trailing);
        o
    }
}

/// One legacy action point, `struct LegacyInitActionPoint` (src/lvl_filesdk1.c:99, 8 bytes).
/// Positions are subtile-with-fraction words like [`Thing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActionPoint {
    pub x: u16,
    pub y: u16,
    pub range: u16,
    pub number: u16,
}

impl ActionPoint {
    pub const SIZE: usize = 8;
}

/// `.apt`: `u32 count`, then 8-byte [`ActionPoint`] records (`load_action_point_file`,
/// src/lvl_filesdk1.c:870).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Apt {
    pub points: Vec<ActionPoint>,
    pub declared_count: Option<u32>,
    pub trailing: Vec<u8>,
}

impl Apt {
    pub fn parse(data: &[u8], _dims: MapDims) -> Result<Self> {
        let (declared, recs, trailing) = counted("apt", data, 4, 4, ActionPoint::SIZE)?;
        let points = recs
            .chunks_exact(ActionPoint::SIZE)
            .map(|c| {
                let v = u16s(c);
                ActionPoint {
                    x: v[0],
                    y: v[1],
                    range: v[2],
                    number: v[3],
                }
            })
            .collect();
        Ok(Self {
            points,
            declared_count: declared,
            trailing: trailing.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o =
            Vec::with_capacity(4 + self.points.len() * ActionPoint::SIZE + self.trailing.len());
        put_count(&mut o, 4, self.declared_count, self.points.len());
        for p in &self.points {
            push_u16s(&mut o, &[p.x, p.y, p.range, p.number]);
        }
        o.extend_from_slice(&self.trailing);
        o
    }
}

/// One legacy static light, `struct LegacyInitLight` (src/lvl_filesdk1.c:105, 20 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Light {
    pub radius: i16,
    pub intensity: u8,
    pub flags: u8,
    /// `field_4_unused`, `field_6_unused`, `field_8_unused`: ignored by the game.
    pub unused: [u16; 3],
    pub x: u16,
    pub y: u16,
    pub z: u16,
    /// `field_10_unused`: ignored by the game.
    pub unused_10: u8,
    pub is_dynamic: u8,
    pub attached_slb: i16,
}

impl Light {
    pub const SIZE: usize = 20;
    fn from_bytes(b: &[u8]) -> Self {
        let w = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        Light {
            radius: w(0) as i16,
            intensity: b[2],
            flags: b[3],
            unused: [w(4), w(6), w(8)],
            x: w(10),
            y: w(12),
            z: w(14),
            unused_10: b[16],
            is_dynamic: b[17],
            attached_slb: w(18) as i16,
        }
    }
    fn write(&self, o: &mut Vec<u8>) {
        o.extend_from_slice(&self.radius.to_le_bytes());
        o.extend_from_slice(&[self.intensity, self.flags]);
        push_u16s(o, &self.unused);
        push_u16s(o, &[self.x, self.y, self.z]);
        o.extend_from_slice(&[self.unused_10, self.is_dynamic]);
        o.extend_from_slice(&self.attached_slb.to_le_bytes());
    }
}

/// `.lgt`: `u32 count`, then 20-byte [`Light`] records, optional file (`load_static_light_file`,
/// src/lvl_filesdk1.c:1247).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lgt {
    pub lights: Vec<Light>,
    pub declared_count: Option<u32>,
    pub trailing: Vec<u8>,
}

impl Lgt {
    pub fn parse(data: &[u8], _dims: MapDims) -> Result<Self> {
        let (declared, recs, trailing) = counted("lgt", data, 4, 4, Light::SIZE)?;
        Ok(Self {
            lights: recs
                .chunks_exact(Light::SIZE)
                .map(Light::from_bytes)
                .collect(),
            declared_count: declared,
            trailing: trailing.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(4 + self.lights.len() * Light::SIZE + self.trailing.len());
        put_count(&mut o, 4, self.declared_count, self.lights.len());
        for l in &self.lights {
            l.write(&mut o);
        }
        o.extend_from_slice(&self.trailing);
        o
    }
}

// ---------------------------------------------------------------------------------------------
// Whole set

/// Extensions of the legacy files, in load order of `load_level_file`.
pub const EXTENSIONS: [&str; 12] = [
    "slb", "own", "dat", "clm", "wib", "wlb", "flg", "inf", "slx", "tng", "apt", "lgt",
];

/// The whole legacy file set of one level. A file that was absent is `None` and is not written
/// back. Loading goes through [`rnc::load_file`]; saving always writes unpacked lowercase files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelBin {
    pub number: u32,
    /// Map size used to parse the files.
    pub dims: Option<MapDims>,
    pub slb: Option<Slb>,
    pub own: Option<Own>,
    pub dat: Option<Dat>,
    pub clm: Option<Clm>,
    pub wib: Option<Wib>,
    pub wlb: Option<Wlb>,
    pub flg: Option<Flg>,
    pub inf: Option<Inf>,
    pub slx: Option<Slx>,
    pub tng: Option<Tng>,
    pub apt: Option<Apt>,
    pub lgt: Option<Lgt>,
}

/// File name `map%05u.<ext>` (lowercase).
pub fn file_name(number: u32, ext: &str) -> String {
    format!("map{number:05}.{ext}")
}

/// Find `map%05u.<ext>` in `dir`, ignoring case.
pub fn find_file(dir: &Path, number: u32, ext: &str) -> Option<PathBuf> {
    let want = file_name(number, ext);
    let exact = dir.join(&want);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(&want) && e.path().is_file())
        .map(|e| e.path())
}

/// Every file of level `number` in `dir`: the regular files (symlinks followed) whose name is
/// `map%05u.<anything>` in any case (`MAP00007.SLB`, `map00007.rules.cfg`, `Map00007.kfxed.toml`),
/// sorted by path (byte order, so upper case names come first). A folder that cannot be read gives
/// an empty list.
///
/// On a case-sensitive file system two case variants of one name (`MAP00001.SLB` and
/// `map00001.slb`) are both listed; the game opens the lower case one first (`prepare_file_path`,
/// the loaders lowercase the name).
///
/// This is the one listing of a level's files: Save as / backups and the "Level files" panel of
/// the editor, campaign level copies and Workshop packaging all use it.
pub fn level_files(dir: &Path, number: u32) -> Vec<PathBuf> {
    let prefix = format!("map{number:05}.");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name().is_some_and(|f| {
                            f.to_string_lossy()
                                .to_ascii_lowercase()
                                .starts_with(&prefix)
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// The part of a level file name after `mapNNNNN.`, lower case (`slb`, `rules.cfg`); `None` when
/// `name` is not a file of level `number`.
pub fn level_file_ext(name: &str, number: u32) -> Option<String> {
    name.to_ascii_lowercase()
        .strip_prefix(&format!("map{number:05}."))
        .map(str::to_string)
}

impl LevelBin {
    /// Load level `number` from `dir`, inferring square dims from the `.slb` size.
    pub fn load(dir: &Path, number: u32) -> Result<Self> {
        Self::load_with_dims(dir, number, None)
    }

    /// Load with explicit dims, or infer them from the `.slb` size when `None`.
    /// Fails when dims are needed but cannot be determined, or when a present file is too small.
    pub fn load_with_dims(dir: &Path, number: u32, dims: Option<MapDims>) -> Result<Self> {
        let mut bin = LevelBin {
            number,
            ..Default::default()
        };
        let mut raw: Vec<(&str, Vec<u8>)> = Vec::new();
        for ext in EXTENSIONS {
            if let Some(p) = find_file(dir, number, ext) {
                raw.push((ext, rnc::load_file(&p)?));
            }
        }
        let get = |e: &str| raw.iter().find(|(x, _)| *x == e).map(|(_, d)| d.as_slice());
        let dims = match dims {
            Some(d) => Some(d),
            None => match get("slb") {
                Some(s) => Some(MapDims::from_slb_len(s.len()).ok_or_else(|| {
                    fmt_err(
                        "slb",
                        format!("cannot infer square map size from {} bytes", s.len()),
                    )
                })?),
                None => None,
            },
        };
        bin.dims = dims;
        let d = dims.unwrap_or(MapDims::new(0, 0));
        let need = || {
            dims.ok_or_else(|| {
                fmt_err(
                    "level_bin",
                    format!("map{number:05}: no .slb and no dims given"),
                )
            })
        };
        macro_rules! ld {
            ($f:ident, $ext:literal, $t:ty, $needs_dims:expr) => {
                if let Some(b) = get($ext) {
                    if $needs_dims {
                        need()?;
                    }
                    bin.$f = Some(<$t>::parse(b, d)?);
                }
            };
        }
        ld!(slb, "slb", Slb, true);
        ld!(own, "own", Own, true);
        ld!(dat, "dat", Dat, true);
        ld!(clm, "clm", Clm, false);
        ld!(wib, "wib", Wib, true);
        ld!(wlb, "wlb", Wlb, true);
        ld!(flg, "flg", Flg, true);
        ld!(inf, "inf", Inf, false);
        ld!(slx, "slx", Slx, true);
        ld!(tng, "tng", Tng, false);
        ld!(apt, "apt", Apt, false);
        ld!(lgt, "lgt", Lgt, false);
        Ok(bin)
    }

    /// Serialised bytes of every present file, as `(extension, bytes)`.
    pub fn files(&self) -> Vec<(&'static str, Vec<u8>)> {
        let mut v = Vec::new();
        macro_rules! w {
            ($f:ident, $ext:literal) => {
                if let Some(x) = &self.$f {
                    v.push(($ext, x.to_bytes()));
                }
            };
        }
        w!(slb, "slb");
        w!(own, "own");
        w!(dat, "dat");
        w!(clm, "clm");
        w!(wib, "wib");
        w!(wlb, "wlb");
        w!(flg, "flg");
        w!(inf, "inf");
        w!(slx, "slx");
        w!(tng, "tng");
        w!(apt, "apt");
        w!(lgt, "lgt");
        v
    }

    /// Write every present file unpacked as `map%05u.<ext>` (lowercase) into `dir`, all or
    /// nothing ([`crate::atomic::SaveTransaction`]). When an existing file for the same
    /// map/extension has a different name (for example `MAP00001.SLB` or an RNC-packed original),
    /// it is removed so it cannot shadow the new file.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let mut tx = crate::atomic::SaveTransaction::new(dir);
        for (ext, bytes) in self.files() {
            tx.write(file_name(self.number, ext), bytes);
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: MapDims = MapDims { w: 2, h: 2 };

    #[test]
    fn dims() {
        assert_eq!(
            MapDims::from_slb_len(2 * 85 * 85),
            Some(MapDims::new(85, 85))
        );
        assert_eq!(MapDims::from_slb_len(7), None);
        assert_eq!(MapDims::from_slb_len(2 * 6), None);
        assert_eq!(D.stl_grid(), 49);
        assert_eq!(D.stl_index(1, 2), 15);
    }

    #[test]
    fn grids_roundtrip_with_trailing() {
        let slb: Vec<u8> = (0..8).chain([9, 9]).collect();
        let s = Slb::parse(&slb, D).unwrap();
        assert_eq!(s.kinds.len(), 4);
        assert_eq!(s.trailing, vec![9, 9]);
        assert_eq!(s.to_bytes(), slb);
        assert!(Slb::parse(&slb[..7], D).is_err());
        let dat: Vec<u8> = (0..98u8).collect();
        let d = Dat::parse(&dat, D).unwrap();
        assert_eq!(d.to_bytes(), dat);
        let flg: Vec<u8> = (0..100u8).collect();
        assert_eq!(Flg::parse(&flg, D).unwrap().to_bytes(), flg);
    }

    #[test]
    fn dat_negated_index() {
        let mut d = Dat {
            raw: vec![0; 49],
            trailing: vec![],
        };
        d.set_column_index(3, 5);
        assert_eq!(d.raw[3], 0xFFFB);
        assert_eq!(d.column_index(3), Some(5));
        assert_eq!(d.column_index(0), Some(0));
    }

    #[test]
    fn clm_roundtrip_and_bad_count() {
        let mut b = Vec::new();
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&0xDEADBEEFu32.to_le_bytes());
        for i in 0..48u8 {
            b.push(i);
        }
        b.push(77); // trailing
        let c = Clm::parse(&b, D).unwrap();
        assert_eq!(c.columns.len(), 2);
        assert_eq!(c.ignored, 0xDEADBEEF);
        assert_eq!(c.declared_count, None);
        assert_eq!(c.to_bytes(), b);
        // count says 9 but only 2 fit
        b[0] = 9;
        let c = Clm::parse(&b, D).unwrap();
        assert_eq!(c.columns.len(), 2);
        assert_eq!(c.declared_count, Some(9));
        assert_eq!(c.to_bytes(), b);
    }

    #[test]
    fn records_roundtrip() {
        let mut t = vec![2, 0];
        t.extend((0..42u8).map(|i| i.wrapping_mul(7)));
        t.push(1);
        let tng = Tng::parse(&t, D).unwrap();
        assert_eq!(tng.things.len(), 2);
        assert_eq!(tng.to_bytes(), t);
        let mut a = vec![1, 0, 0, 0];
        a.extend(1..=8u8);
        assert_eq!(Apt::parse(&a, D).unwrap().to_bytes(), a);
        let mut l = vec![1, 0, 0, 0];
        l.extend((0..20u8).map(|i| i * 3));
        let lgt = Lgt::parse(&l, D).unwrap();
        assert_eq!(lgt.lights[0].attached_slb, i16::from_le_bytes([54, 57]));
        assert_eq!(lgt.to_bytes(), l);
        // header-only file with count 0
        assert_eq!(Tng::parse(&[0, 0], D).unwrap().to_bytes(), vec![0, 0]);
    }

    #[test]
    fn inf_and_levelbin_io() {
        assert_eq!(Inf::parse(&[3, 4, 5], D).unwrap().texture_id, 3);
        assert!(Inf::parse(&[], D).is_err());
        let dir = crate::testutil::scratch_dir(&format!(
            "kfx-formats/kfx_level_bin_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let slb: Vec<u8> = vec![0; 2 * 4];
        std::fs::write(dir.join("MAP00042.SLB"), &slb).unwrap();
        std::fs::write(dir.join("map00042.INF"), [1u8]).unwrap();
        let bin = LevelBin::load(&dir, 42).unwrap();
        assert_eq!(bin.dims, Some(D));
        assert!(bin.slb.is_some() && bin.inf.is_some() && bin.own.is_none());
        bin.save(&dir).unwrap();
        assert!(dir.join("map00042.slb").is_file());
        assert!(dir.join("map00042.inf").is_file());
        assert!(!dir.join("map00042.own").exists());
        let again = LevelBin::load(&dir, 42).unwrap();
        assert_eq!(again, bin);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The shared level file listing: any case, only the level's own files, sorted, sub
    /// folders and other levels left out, a missing folder empty.
    #[test]
    fn level_files_any_case() {
        let dir = crate::testutil::scratch_dir(&format!(
            "kfx-formats/kfx_level_files_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("map00007.sub")).unwrap();
        // No two names here differ only in case, so the test also holds on case-insensitive
        // file systems (macOS, Windows).
        for n in [
            "MAP00007.SLB",
            "map00007.own",
            "Map00007.Rules.cfg",
            "map00007.kfxed.toml",
            "map00070.slb",
            "map0007.slb",
            "xmap00007.slb",
            "map00007",
        ] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        let names: Vec<String> = level_files(&dir, 7)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "MAP00007.SLB",
                "Map00007.Rules.cfg",
                "map00007.kfxed.toml",
                "map00007.own"
            ]
        );
        let exts: Vec<String> = names.iter().filter_map(|n| level_file_ext(n, 7)).collect();
        assert_eq!(exts, ["slb", "rules.cfg", "kfxed.toml", "own"]);
        assert_eq!(level_file_ext("map00070.slb", 7), None);
        assert!(level_files(&dir, 8).is_empty());
        assert!(level_files(&dir.join("missing"), 7).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
