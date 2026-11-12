//! Columns: the game's column rules and a pool that builds `.clm` + `.dat`.
//!
//! Reference: `src/map_columns.c` (`make_solidmask` 174, `column_is_equivalent` 270,
//! `find_column` 280, `create_column` 293, `clear_columns` 360, `init_whole_blocks` 420) and
//! `columns_add_static_entries` (`src/lvl_filesdk1.c:938`).
use crate::gamedata::{GameData, SlabsetColumn, COLUMNS_COUNT};
use crate::{Error, Result};
use kfx_formats::level_bin::{Clm, Column, Dat, MapDims};
use std::collections::HashMap;

/// `COLUMN_STACK_HEIGHT`.
pub const COLUMN_STACK_HEIGHT: usize = 8;
/// `CLF_ACTIVE` (src/map_columns.h:53).
pub const CLF_ACTIVE: u8 = 0x01;
/// `CLF_CEILING_MASK`.
pub const CLF_CEILING_MASK: u8 = 0x0E;
/// `CLF_FLOOR_MASK`.
pub const CLF_FLOOR_MASK: u8 = 0xF0;

/// `make_solidmask` (src/map_columns.c:174): bit i set when cube i is non-zero.
pub fn solidmask_of(cubes: &[u16; 8]) -> u16 {
    cubes
        .iter()
        .enumerate()
        .filter(|(_, &c)| c != 0)
        .fold(0, |m, (i, _)| m | (1 << i))
}

/// `column_is_equivalent` (src/map_columns.c:270): floor texture, solid mask, orientation and cubes.
/// Bitfields and use count are not compared.
pub fn column_equivalent(a: &Column, b: &Column) -> bool {
    a.floor_texture == b.floor_texture
        && a.solidmask == b.solidmask
        && a.orient == b.orient
        && a.cubes == b.cubes
}

/// Floor and ceiling bits as `create_column` (src/map_columns.c:293-358) computes them, applied on
/// top of `bitfields` (whose active bit is kept).
///
/// Quirk kept: the ceiling height is taken from the lowest non-empty cube at index 1..7 (the loop
/// walks from the top down and the last hit wins), which for a column with both floor cubes and a
/// ceiling is the floor's second cube, not the ceiling.
pub fn create_column_bitfields(cubes: &[u16; 8], bitfields: u8) -> u8 {
    let mut bf = bitfields;
    let top = cubes
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(COLUMN_STACK_HEIGHT);
    bf &= !CLF_FLOOR_MASK;
    bf |= ((top as u8) << 4) & CLF_FLOOR_MASK;
    if top >= COLUMN_STACK_HEIGHT {
        bf &= CLF_FLOOR_MASK | CLF_ACTIVE;
    } else if !cubes[top..].iter().any(|&c| c != 0) {
        bf &= CLF_FLOOR_MASK | CLF_ACTIVE;
    } else {
        let mut shift: u8 = 0;
        for i in 0..COLUMN_STACK_HEIGHT - 1 {
            if cubes[7 - i] != 0 {
                bf ^= (shift ^ bf) & CLF_CEILING_MASK;
            }
            shift = shift.wrapping_add(2);
        }
    }
    bf
}

/// A column as `create_column` stores it: solid mask from the cubes, recomputed heights, use 0.
/// `active` sets `CLF_ACTIVE` (slabset columns are created active).
pub fn canonical_column(c: &Column, active: bool) -> Column {
    let mut out = *c;
    out.use_count = 0;
    out.solidmask = solidmask_of(&c.cubes);
    let bf = if active {
        c.bitfields | CLF_ACTIVE
    } else {
        c.bitfields & !CLF_ACTIVE
    };
    out.bitfields = create_column_bitfields(&c.cubes, bf);
    out
}

/// The "unrevealed" column every level gets (`init_whole_blocks`, src/map_columns.c:420).
pub fn unrevealed_column() -> Column {
    let mut c = Column {
        floor_texture: 22,
        ..Default::default()
    };
    c.cubes[..5].copy_from_slice(&[10, 1, 1, 1, 141]);
    canonical_column(&c, true)
}

/// `player_cubes` (src/player_data.c:48).
pub const PLAYER_CUBES: [u16; 6] = [0xC0, 0xC1, 0xC2, 0xC3, 0xC7, 0xC6];

/// The 18 columns of `columns_add_static_entries` (src/lvl_filesdk1.c:938): floor texture 0 (the C
/// array `c[3]` is all zeros) with one player cube at the bottom.
pub fn static_columns() -> Vec<Column> {
    let mut v = vec![];
    for _ in 0..3 {
        for &pc in &PLAYER_CUBES {
            let mut c = Column::default();
            c.cubes[0] = pc;
            v.push(canonical_column(&c, true));
        }
    }
    v
}

/// Content key: what `find_column` compares.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ColumnKey {
    pub cubes: [u16; 8],
    pub floor_texture: u16,
    pub solidmask: u16,
    pub orient: u8,
}

impl From<&Column> for ColumnKey {
    fn from(c: &Column) -> Self {
        ColumnKey {
            cubes: c.cubes,
            floor_texture: c.floor_texture,
            solidmask: c.solidmask,
            orient: c.orient,
        }
    }
}

/// What the pool merges on: the [`ColumnKey`] plus the ceiling bits of `bitfields`. The game
/// keeps those as loaded from the `.clm` (only the floor bits are recomputed,
/// `set_column_floor_filled_subtiles` src/lvl_filesdk1.c:694), so two loaded columns that differ only
/// there must stay apart (a map from another tool can hold such pairs). Columns the editor
/// generates have the ceiling `create_column` computes, so this splits nothing for them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct PoolKey {
    key: ColumnKey,
    ceiling: u8,
}

impl From<&Column> for PoolKey {
    fn from(c: &Column) -> Self {
        PoolKey {
            key: ColumnKey::from(c),
            ceiling: c.bitfields & CLF_CEILING_MASK,
        }
    }
}

/// Interns columns (find-or-create by content, like `find_column` / `create_column`) and
/// writes `.clm` + `.dat`. Index 0 is the all-zero column the original files start with.
#[derive(Clone, Debug)]
pub struct ColumnPool {
    pub columns: Vec<Column>,
    index: HashMap<PoolKey, u16>,
}

impl Default for ColumnPool {
    fn default() -> Self {
        Self::new()
    }
}

impl ColumnPool {
    pub fn new() -> Self {
        let zero = Column::default();
        let mut index = HashMap::new();
        index.insert(PoolKey::from(&zero), 0);
        ColumnPool {
            columns: vec![zero],
            index,
        }
    }

    /// Index of `c` (by content), creating it when new. Fails when the pool is full
    /// (`COLUMNS_COUNT` entries including index 0).
    pub fn intern(&mut self, c: &Column) -> Result<u16> {
        let k = PoolKey::from(c);
        if let Some(&i) = self.index.get(&k) {
            return Ok(i);
        }
        if self.columns.len() >= COLUMNS_COUNT {
            return Err(Error::PoolOverflow {
                needed: self.columns.len() + 1,
                map: self.columns.len() + 1,
                game: 0,
                limit: COLUMNS_COUNT,
            });
        }
        let i = self.columns.len() as u16;
        let mut stored = *c;
        stored.use_count = 0;
        self.columns.push(stored);
        self.index.insert(k, i);
        Ok(i)
    }

    pub fn len(&self) -> usize {
        self.columns.len()
    }
    pub fn is_empty(&self) -> bool {
        self.columns.len() <= 1
    }

    /// Columns the game itself adds to the pool when it loads the level and that are not in
    /// this pool already: the unrevealed column, every resolved slabset column, the 18 static
    /// entries (`load_level_file`, src/lvl_filesdk1.c:1434-1438).
    pub fn game_added_columns(&self, gd: &GameData) -> usize {
        // the game's `find_column` ignores `bitfields`
        let have: std::collections::HashSet<ColumnKey> = self.index.keys().map(|k| k.key).collect();
        let mut extra: HashMap<ColumnKey, ()> = HashMap::new();
        let mut add = |c: &Column| {
            let k = ColumnKey::from(c);
            if !have.contains(&k) {
                extra.insert(k, ());
            }
        };
        add(&unrevealed_column());
        for ss in &gd.slabset {
            for c in ss {
                if let SlabsetColumn::Column(c) = c {
                    add(c);
                }
            }
        }
        for c in static_columns() {
            add(&c);
        }
        extra.len()
    }

    /// Check the game's pool limit: map columns (index 1..) plus the game-added ones must fit
    /// in `COLUMNS_COUNT - 1` slots (index 0 is never handed out by `create_column`).
    pub fn check_game_limit(&self, gd: &GameData) -> Result<()> {
        let map = self.columns.len() - 1;
        let game = self.game_added_columns(gd);
        let limit = COLUMNS_COUNT - 1;
        if map + game > limit {
            return Err(Error::PoolOverflow {
                needed: map + game,
                map,
                game,
                limit,
            });
        }
        Ok(())
    }

    /// Build `.clm` + `.dat` from a full `(3W+1) x (3H+1)` grid of columns. Use counts are the
    /// number of `.dat` cells referencing each column (the game keeps a column with use > 0 from
    /// being recycled by `create_column`, src/map_columns.c:298).
    pub fn build(gd: &GameData, dims: MapDims, grid: &[Column]) -> Result<(Clm, Dat)> {
        let mut pool = ColumnPool::new();
        let mut dat = Dat {
            raw: vec![0; dims.stl_grid()],
            trailing: vec![],
        };
        for (i, c) in grid.iter().enumerate().take(dims.stl_grid()) {
            let idx = pool.intern(c)?;
            dat.set_column_index(i, idx);
        }
        pool.check_game_limit(gd)?;
        let mut cols = pool.columns.clone();
        for i in 0..dims.stl_grid() {
            if let Some(idx) = dat.column_index(i) {
                if let Some(c) = cols.get_mut(idx as usize) {
                    c.use_count = c.use_count.saturating_add(1);
                }
            }
        }
        Ok((
            Clm {
                ignored: 0,
                columns: cols,
                declared_count: None,
                trailing: vec![],
            },
            dat,
        ))
    }
}

/// Column of `.dat` cell `i` from a loaded `.clm` (index 0 / out of range gives the default
/// column, like the game's `columns_data[0]` after `clear_columns`).
pub fn loaded_column(clm: &Clm, dat: &Dat, i: usize) -> Column {
    let idx = dat.column_index(i).unwrap_or(0);
    if idx < 0 {
        return Column::default();
    }
    clm.columns.get(idx as usize).copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitfields_like_create_column() {
        // Plain earth: 5 cubes, no ceiling.
        let mut c = [0u16; 8];
        c[..5].copy_from_slice(&[25, 10, 2, 2, 5]);
        assert_eq!(create_column_bitfields(&c, 1), 0x51);
        // Floor of 1 and a lintel of three cubes at the top: the last write is for cube 5
        // (shift 2*(7-5) = 4 -> ceiling bits 4 = 2 cubes).
        let mut d = [0u16; 8];
        d[0] = 25;
        d[5] = 1;
        d[6] = 1;
        d[7] = 1;
        assert_eq!(create_column_bitfields(&d, 0), 0x10 | 0x04);
        // Full stack.
        assert_eq!(create_column_bitfields(&[1; 8], 0xFF), 0x81);
        assert_eq!(solidmask_of(&c), 0x1F);
    }

    #[test]
    fn pool_interns_by_content() {
        let mut p = ColumnPool::new();
        let mut a = Column {
            floor_texture: 3,
            ..Default::default()
        };
        a.cubes[0] = 7;
        a.solidmask = 1;
        let i = p.intern(&a).unwrap();
        let mut b = a;
        b.bitfields = 0xF1; // floor height and active bit: recomputed / not part of the content
        assert_eq!(p.intern(&b).unwrap(), i);
        // the ceiling bits are kept by the game as loaded: such a column stays apart
        let mut c = a;
        c.bitfields = 0x02;
        assert_ne!(p.intern(&c).unwrap(), i);
        assert_eq!(p.intern(&Column::default()).unwrap(), 0);
    }
}
