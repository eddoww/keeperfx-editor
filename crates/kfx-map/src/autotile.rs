//! Port of the game's slab placement logic (`src/map_blocks.c`).
//!
//! The game loads `.clm` / `.dat` precomputed and runs this logic only when a slab changes in
//! play. An editor has to produce the columns itself, so this module computes, for every slab,
//! the 9 columns, the wibble values and the slab objects the game would produce for the slab
//! layout.
//!
//! Every function cites the C function it ports. Quirks of the C are kept (and marked).
//!
//! # Differences from the game that are deliberate
//! * The game re-places a changed slab and then its 8 neighbours, skipping neighbours whose kind
//!   did not change and whose category is Obstacle or Unclaimed (`place_slab_type_on_map_f`,
//!   src/map_blocks.c:1748-1758). Rock/gold next to a changed slab therefore keep columns computed for
//!   the *old* neighbourhood: the game's result depends on edit history. This module is a pure
//!   function of the layout (as if every slab had been placed with its final neighbours);
//!   [`MapGen::regenerate_around`] recomputes the whole 3x3 block.
//! * Slab objects whose target subtile lies in a neighbour slab are filtered with the
//!   neighbour's *final* column (the game uses whatever column is there at that moment).
use crate::gamedata::{category, slab_flags, GameData, SlabConfig, SLABSETS_PER_SLAB};
use kfx_formats::level_bin::{Column, MapDims};

/// `PLAYER_NEUTRAL` (src/player_instances.h:38).
pub const PLAYER_NEUTRAL: u8 = 5;

/// Hard-coded slab kinds the C refers to (`enum SlabTypes`, src/slab_data.h:34).
pub mod kinds {
    pub const ROCK: u8 = 0;
    pub const GOLD: u8 = 1;
    pub const EARTH: u8 = 2;
    pub const TORCHDIRT: u8 = 3;
    pub const WALLDRAPE: u8 = 4;
    pub const WALLTORCH: u8 = 5;
    pub const WALLWTWINS: u8 = 6;
    pub const WALLWWOMAN: u8 = 7;
    pub const WALLPAIRSHR: u8 = 8;
    pub const DAMAGEDWALL: u8 = 9;
    pub const PATH: u8 = 10;
    pub const CLAIMED: u8 = 11;
    pub const LAVA: u8 = 12;
    pub const WATER: u8 = 13;
    pub const PRISON: u8 = 20;
    pub const BRIDGE: u8 = 51;
    pub const GEMS: u8 = 52;
    pub const GUARDPOST: u8 = 53;
}

/// `ObjMdl_PrisonBar` (src/thing_objects.h:72).
const OBJ_PRISON_BAR: i16 = 27;
/// `TCls_Object`, `TCls_EffectGen`.
const TCLS_OBJECT: u8 = 1;
const TCLS_EFFECTGEN: u8 = 7;
/// `WlbT_*` (src/slab_data.h:98).
pub const WLB_LAVA: u8 = 1;
pub const WLB_WATER: u8 = 2;
pub const WLB_BRIDGE: u8 = 3;

/// `my_around_eight` (src/map_utils.c:78): N, NE, E, SE, S, SW, W, NW.
pub const AROUND_EIGHT: [(i32, i32); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];
/// `my_around_nine` (src/map_utils.c:85).
pub const AROUND_NINE: [(i32, i32); 9] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (0, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];
/// `slab_element_around_eight` (src/map_blocks.c:53): subtile offset from the centre (4) for each
/// `my_around_eight` direction.
const SLAB_ELEMENT_AROUND_EIGHT: [i32; 8] = [-3, -2, 1, 4, 3, 2, -1, -4];
/// `slab_primitive` (src/map_blocks.c:57): style pick for each "against" mask (-1 = special case).
const SLAB_PRIMITIVE: [i32; 16] = [-1, 1, 0, 4, 3, -1, 7, -1, 2, 5, -1, -1, 6, -1, -1, 8];
/// `slab_element_to_corner` (src/map_blocks.c:61).
const SLAB_ELEMENT_TO_CORNER: [i32; 12] = [6, -1, 0, -1, -1, -1, 4, -1, 2, 0, 0, 0];
/// `special_cases` (src/map_blocks.c:65).
const SPECIAL_CASES: [[u8; 9]; 7] = [
    [1, 1, 3, 1, 1, 3, 1, 1, 3],
    [4, 4, 7, 4, 8, 7, 4, 8, 7],
    [2, 2, 2, 2, 2, 2, 0, 0, 0],
    [5, 5, 5, 8, 8, 5, 4, 4, 4],
    [5, 8, 6, 5, 8, 6, 5, 5, 6],
    [6, 6, 6, 6, 8, 8, 7, 7, 7],
    [0, 0, 0, 3, 3, 1, 2, 2, 2],
];
/// `against_to_case` (src/map_blocks.c:75): mask -> index into `SPECIAL_CASES`.
const AGAINST_TO_CASE: [Option<usize>; 16] = [
    None,
    None,
    None,
    None,
    None,
    Some(0),
    None,
    Some(1),
    None,
    None,
    Some(2),
    Some(3),
    None,
    Some(4),
    Some(5),
    None,
];

/// Read access to a slab layout.
pub trait SlabView {
    fn dims(&self) -> MapDims;
    /// Slab kind at (x, y), `None` outside the map. Kinds must already be normalised the way
    /// the game loads them (`load_map_slab_file`: a value above the slab count becomes rock).
    fn kind(&self, x: i32, y: i32) -> Option<u8>;
    /// Slab owner at (x, y), `None` outside the map.
    fn owner(&self, x: i32, y: i32) -> Option<u8>;
    /// Water/lava under bridge (`.wlb`) at (x, y); used to pick the bridge animation frame.
    fn wlb(&self, _x: i32, _y: i32) -> u8 {
        0
    }
}

/// A simple owned layout, handy for tests and tools.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub dims: MapDims,
    pub kinds: Vec<u8>,
    pub owners: Vec<u8>,
    pub wlb: Vec<u8>,
}

impl Layout {
    pub fn new(dims: MapDims, kind: u8, owner: u8) -> Self {
        let n = dims.slabs();
        Layout {
            dims,
            kinds: vec![kind; n],
            owners: vec![owner; n],
            wlb: vec![0; n],
        }
    }
    pub fn set(&mut self, x: usize, y: usize, kind: u8, owner: u8) {
        let i = self.dims.slab_index(x, y);
        self.kinds[i] = kind;
        self.owners[i] = owner;
    }
}

impl SlabView for Layout {
    fn dims(&self) -> MapDims {
        self.dims
    }
    fn kind(&self, x: i32, y: i32) -> Option<u8> {
        in_map(self.dims, x, y).then(|| self.kinds[self.dims.slab_index(x as usize, y as usize)])
    }
    fn owner(&self, x: i32, y: i32) -> Option<u8> {
        in_map(self.dims, x, y).then(|| self.owners[self.dims.slab_index(x as usize, y as usize)])
    }
    fn wlb(&self, x: i32, y: i32) -> u8 {
        if in_map(self.dims, x, y) {
            self.wlb[self.dims.slab_index(x as usize, y as usize)]
        } else {
            0
        }
    }
}

pub fn in_map(d: MapDims, x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && x < d.w as i32 && y < d.h as i32
}

/// What a slab object becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlabObjectKind {
    /// An object thing of this model (already player-coloured).
    Object { model: i16 },
    /// An effect generator; `range` in map coordinates (`EffectRange * 256`).
    EffectGen { model: i16, range: i32 },
    /// A static light; `radius` in map coordinates (`EffectRange * 256`).
    Light { intensity: i16, radius: i32 },
}

/// A thing or light the game creates when it places a slab (`place_slab_object`,
/// src/map_blocks.c:822). These are generated from the slabset, they are not part of the slab
/// layout; whether they must be written into the map files is discussed in [`crate::mapdoc`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlabObject {
    pub kind: SlabObjectKind,
    /// Absolute position in map coordinates (subtile * 256 + fraction).
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub owner: u8,
    pub orientation: i16,
    /// `get_slab_number(slb_x, slb_y)`: stored as the thing's parent (`ParentTile`) / the
    /// light's `attached_slb`, which is how the game deletes them when the slab changes.
    pub parent_slab: u32,
    /// Slabset entry and subtile (0..8) it came from.
    pub slabset_id: u16,
    pub stl_id: u8,
    /// Subtile whose map flags decide whether the object is created (it is not when the
    /// subtile is `BLOCKING`).
    pub check_stl: (i32, i32),
}

/// The result of placing one slab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlabGen {
    /// Columns of the 9 subtiles, row-major (index = dy*3 + dx).
    pub columns: [Column; 9],
    /// Wibble values of the 9 subtiles.
    pub wibble: [u8; 9],
    /// Slab objects before the `BLOCKING` check (see [`SlabObject::check_stl`]).
    pub objects: Vec<SlabObject>,
    /// Slabset entry used for each subtile (`slab_number_list`).
    pub slabset_ids: [u16; 9],
    /// Slab kind whose slabset was used for each subtile (`slab_type_list` after truncation).
    pub subtile_kinds: [u8; 9],
    /// Subtiles whose slabset column could not be resolved (see
    /// [`crate::gamedata::SlabsetColumn::Unresolved`]); their column is left default.
    pub unresolved: [bool; 9],
}

struct Ctx<'a, V: SlabView> {
    v: &'a V,
    gd: &'a GameData,
}

impl<'a, V: SlabView> Ctx<'a, V> {
    fn valid(&self, x: i32, y: i32) -> bool {
        in_map(self.v.dims(), x, y)
    }
    /// `get_slabmap_block(x, y)->kind`: the invalid block reads as kind 0.
    fn kind(&self, x: i32, y: i32) -> u8 {
        self.v.kind(x, y).unwrap_or(0)
    }
    /// `slabmap_owner` (src/slab_data.c:152): neutral outside the map.
    fn owner(&self, x: i32, y: i32) -> u8 {
        self.v.owner(x, y).unwrap_or(PLAYER_NEUTRAL)
    }
    /// `get_slab_stats` (src/config_terrain.c:363).
    fn stats_at(&self, x: i32, y: i32) -> &'a SlabConfig {
        self.gd.slab(self.kind(x, y) as usize)
    }
    fn stats(&self, kind: usize) -> &'a SlabConfig {
        self.gd.slab(kind)
    }
}

/// `slab_kind_is_room_wall` (src/config_terrain.c:709).
fn is_room_wall(gd: &GameData, kind: usize) -> bool {
    let s = gd.slab(kind);
    s.category == category::FORTIFIED_WALL && s.slb_id != 0
}

/// `slab_kind_is_liquid` (src/config_terrain.c:749): hard-coded water and lava.
pub fn is_liquid(kind: u8) -> bool {
    kind == kinds::WATER || kind == kinds::LAVA
}

/// `get_player_color_idx` (src/player_data.c:785) with every player on its default colour
/// (`dungeon->color_idx = i`, src/dungeon_data.c:562). Owners that are not players read the zeroed
/// `bad_dungeon`, colour 0.
pub fn player_color_idx(owner: u8) -> usize {
    if owner == PLAYER_NEUTRAL {
        return owner as usize;
    }
    if (owner as usize) < 9 {
        owner as usize
    } else {
        0
    }
}

/// `get_slabset_index_f` (src/map_blocks.c:796).
pub fn slabset_index(gd: &GameData, slbkind: i32, style: u8, pick: u8) -> usize {
    let mut kind = (slbkind as u8) as usize; // SlabKind is uint8_t
    let mut style = style;
    let mut pick = pick;
    if kind >= gd.slab_count() {
        kind = 0;
    }
    if style > 3 {
        style = 0;
    }
    if pick >= 9 || (style == 3 && pick >= 1) {
        if is_room_wall(gd, kind) && pick < 9 {
            // "There's no columns for room walls next to water, so we're using a regular wall."
            style = 2;
            kind = kinds::DAMAGEDWALL as usize;
        } else {
            pick = 0;
        }
    }
    SLABSETS_PER_SLAB * kind + 9 * style as usize + pick as usize
}

/// `torch_flags_for_slab` (src/map_blocks.c:353).
pub fn torch_flags_for_slab<V: SlabView>(v: &V, slb_x: i32, slb_y: i32) -> u16 {
    let k = |x, y| v.kind(x, y).unwrap_or(0);
    let mut f = 0;
    if slb_x % 5 == 0
        && (k(slb_x, slb_y + 1) == kinds::CLAIMED || k(slb_x, slb_y - 1) == kinds::CLAIMED)
    {
        f |= 0x01;
    }
    if slb_y % 5 == 0
        && (k(slb_x + 1, slb_y) == kinds::CLAIMED || k(slb_x - 1, slb_y) == kinds::CLAIMED)
    {
        f |= 0x02;
    }
    f
}

/// `get_against` (src/map_blocks.c:637).
fn get_against<V: SlabView>(c: &Ctx<V>, plyr: u8, kind: usize, x: i32, y: i32) -> bool {
    if !c.valid(x, y) {
        return true;
    }
    let s = c.stats_at(x, y);
    let a = c.stats(kind);
    let o = c.owner(x, y);
    s.slb_id != a.slb_id || (o != plyr && (o != PLAYER_NEUTRAL || c.kind(x, y) == kinds::CLAIMED))
}

/// `place_single_slab_fill_arrays_std` (src/map_blocks.c:998).
fn fill_arrays_std<V: SlabView>(
    c: &Ctx<V>,
    x: i32,
    y: i32,
    types: &mut [i32; 9],
    pretty: &mut [u8; 9],
) {
    for i in (0..8).step_by(2) {
        let (sx, sy) = (x + AROUND_EIGHT[i].0, y + AROUND_EIGHT[i].1);
        if !c.valid(sx, sy) {
            continue;
        }
        let k = c.kind(sx, sy);
        if k == kinds::GUARDPOST || k == kinds::BRIDGE {
            continue;
        }
        let s = c.stats(k as usize);
        let a = AROUND_EIGHT[(i + 7) & 7];
        let b = AROUND_EIGHT[(i + 1) & 7];
        // Siblings read the raw block (kind 0 outside the map).
        let pv = (c.kind(x + a.0, y + a.1) == k && c.kind(x + b.0, y + b.1) == k) as u8;
        if s.category == category::ROOM_INTERIOR {
            for n in [7usize, 0, 1] {
                let neigh = (4 + SLAB_ELEMENT_AROUND_EIGHT[(i + n) & 7]) as usize;
                types[neigh] = k as i32 + 1;
                pretty[neigh] = pv;
            }
        }
    }
}

/// `place_single_slab_fill_style_array` (src/map_blocks.c:1098).
fn fill_style_array<V: SlabView>(c: &Ctx<V>, x: i32, y: i32, style: &mut [u8; 9]) {
    for i in (0..8).step_by(2) {
        let (sx, sy) = (x + AROUND_EIGHT[i].0, y + AROUND_EIGHT[i].1);
        let sv = if c.valid(sx, sy) {
            c.stats_at(sx, sy).fill_style
        } else {
            0
        };
        for n in [7usize, 0, 1] {
            let neigh = (4 + SLAB_ELEMENT_AROUND_EIGHT[(i + n) & 7]) as usize;
            if style[neigh] < sv {
                style[neigh] = sv;
            }
        }
    }
}

/// `place_single_slab_set_torch_places` (src/map_blocks.c:1128).
///
/// Quirk kept: the "undecorated" kind is `slbkind + 4` for anything but torch dirt, so a torch
/// wall (5) falls back to the damaged wall (9).
fn set_torch_places<V: SlabView>(c: &Ctx<V>, kind: u8, x: i32, y: i32, types: &mut [i32; 9]) {
    let undecorated = if kind == kinds::TORCHDIRT {
        kinds::EARTH as i32
    } else {
        kind as i32 + 4
    };
    let tf = torch_flags_for_slab(c.v, x, y);
    let not_room = |sx, sy| c.stats_at(sx, sy).category != category::ROOM_INTERIOR;
    if tf & 0x01 != 0 {
        types[3] = undecorated;
        types[5] = undecorated;
        if (y + x) & 1 != 0 {
            if not_room(x, y - 1) {
                types[1] = undecorated;
            }
        } else if not_room(x, y + 1) {
            types[7] = undecorated;
        }
    } else if tf & 0x02 != 0 {
        types[1] = undecorated;
        types[7] = undecorated;
        if (y + x) & 1 != 0 {
            if not_room(x - 1, y) {
                types[3] = undecorated;
            }
        } else if not_room(x + 1, y) {
            types[5] = undecorated;
        }
    }
}

/// `place_single_slab_prepare_column_index` (src/map_blocks.c:1189). Fills `ids` (the slabset entry
/// of each subtile; the subtile's column is entry `ids[i]` position `i`).
#[allow(clippy::too_many_arguments)]
fn prepare_column_index<V: SlabView>(
    c: &Ctx<V>,
    kind: u8,
    x: i32,
    y: i32,
    plyr: u8,
    types: &[i32; 9],
    pretty: &[u8; 9],
    style: &[u8; 9],
    ids: &mut [usize; 9],
) {
    let gd = c.gd;
    let place = c.stats(kind as usize);
    let mut against: usize = 0;
    for i in (0..8).step_by(2) {
        against <<= 1;
        against |= get_against(
            c,
            plyr,
            kind as usize,
            x + AROUND_EIGHT[i].0,
            y + AROUND_EIGHT[i].1,
        ) as usize;
    }
    if against != 0 {
        let prim = SLAB_PRIMITIVE[against];
        if prim == -1 {
            // against_to_case is never NULL for a -1 primitive with a non-zero mask.
            if let Some(case) = AGAINST_TO_CASE[against] {
                for i in 0..9 {
                    ids[i] =
                        slabset_index(gd, types[i], style[i] + pretty[i], SPECIAL_CASES[case][i]);
                }
            }
        } else {
            for i in 0..9 {
                ids[i] = slabset_index(gd, types[i], style[i] + pretty[i], prim as u8);
            }
        }
    } else if place.category == category::ROOM_INTERIOR {
        for i in (1..8).step_by(2) {
            against <<= 1;
            against |= get_against(
                c,
                plyr,
                kind as usize,
                x + AROUND_EIGHT[i].0,
                y + AROUND_EIGHT[i].1,
            ) as usize;
        }
        if against != 0 {
            for i in 0..9 {
                ids[i] = slabset_index(gd, types[i], style[i], SPECIAL_CASES[6][i]);
            }
        } else {
            for i in 0..9 {
                ids[i] = slabset_index(gd, types[i], 3, 0);
            }
        }
    } else {
        for i in 0..9 {
            ids[i] = slabset_index(gd, types[i], 3, 0);
        }
    }
}

/// `place_single_slab_modify_column_near_liquid` (src/map_blocks.c:1282): a wall corner between two
/// walls, diagonal to ground / room / obstacle / liquid, becomes the twins-wall "ALL" column.
#[allow(clippy::too_many_arguments)]
fn modify_column_near_liquid<V: SlabView>(
    c: &Ctx<V>,
    x: i32,
    y: i32,
    types: &mut [i32; 9],
    style: &[u8; 9],
    ids: &mut [usize; 9],
) {
    for i in (0..8).step_by(2) {
        let d = AROUND_EIGHT[(i + 7) & 7];
        let (sx, sy) = (x + d.0, y + d.1);
        let sa = c.stats_at(sx, sy);
        let ka = c.kind(sx, sy);
        if sa.category == category::FORTIFIED_GROUND
            || sa.category == category::ROOM_INTERIOR
            || sa.category == category::OBSTACLE
            || ka == kinds::WATER
            || ka == kinds::LAVA
        {
            let d2 = AROUND_EIGHT[(i + 6) & 7];
            if c.stats_at(x + d2.0, y + d2.1).category == category::FORTIFIED_WALL {
                let d3 = AROUND_EIGHT[i & 7];
                if c.stats_at(x + d3.0, y + d3.1).category == category::FORTIFIED_WALL {
                    let neigh = (4 + SLAB_ELEMENT_AROUND_EIGHT[(i + 7) & 7]) as usize;
                    types[neigh] = kinds::WALLWTWINS as i32;
                    ids[neigh] = slabset_index(c.gd, types[neigh], style[neigh], 8);
                }
            }
        }
    }
}

/// `copy_block_with_cube_groups` (src/map_blocks.c:695): the slabset column with every cube of an
/// ownership group replaced by that group's cube for the owner's colour (when one exists).
pub fn owner_column(gd: &GameData, template: &Column, owner: u8) -> Column {
    let mut col = *template;
    col.use_count = 0;
    col.bitfields &= !0x01;
    let color = player_color_idx(owner);
    for i in 0..8 {
        if col.cubes[i] > 0 {
            if let Some(cube) = gd.cube(col.cubes[i] as usize) {
                let g = cube.ownership_group as usize;
                if g > 0 && g < gd.cube_bits.len() {
                    let new = gd.cube_bits[g][color];
                    if new > 0 {
                        col.cubes[i] = new;
                    }
                }
            }
        }
    }
    col
}

/// `set_subtile_wibble` (src/map_blocks.c:737) for subtile (stl_x, stl_y) of a slab of `kind`.
pub fn subtile_wibble<V: SlabView>(v: &V, gd: &GameData, kind: u8, stl_x: i32, stl_y: i32) -> u8 {
    let mut wibble = gd.slab(kind as usize).wibble;
    if is_liquid(kind) {
        let (sx, sy) = (stl_x / 3, stl_y / 3);
        let x0 = if stl_x % 3 == 0 { sx - 1 } else { sx };
        let y0 = if stl_y % 3 == 0 { sy - 1 } else { sy };
        for y in y0..=sy {
            for x in x0..=sx {
                if (x != sx || y != sy) && !is_liquid(v.kind(x, y).unwrap_or(0)) {
                    wibble = 1;
                }
            }
        }
    }
    wibble
}

/// `get_slab_number` (src/slab_data.c:25).
pub fn slab_number(d: MapDims, x: i32, y: i32) -> u32 {
    let x = x.clamp(0, d.w as i32);
    let y = y.clamp(0, d.h as i32);
    (y as u32) * d.w as u32 + x as u32
}

/// `place_slab_object` (src/map_blocks.c:822) for one subtile, before the `BLOCKING` check.
#[allow(clippy::too_many_arguments)]
fn slab_objects_for<V: SlabView>(
    c: &Ctx<V>,
    out: &mut Vec<SlabObject>,
    slb_num: u32,
    stl_x: i32,
    stl_y: i32,
    slabset_id: usize,
    stl_id: u8,
    owner: u8,
) {
    let gd = c.gd;
    let Some(objs) = gd.slab_objects.get(slabset_id) else {
        return;
    };
    for so in objs.iter().filter(|o| o.stl_id == stl_id) {
        let px = (stl_x << 8) + so.offset[0] as i32;
        let py = (stl_y << 8) + so.offset[1] as i32;
        let pz = so.offset[2] as i32;
        // coord_subtile: C division truncates toward zero.
        let check_stl = (px / 256, py / 256);
        let base = SlabObject {
            kind: SlabObjectKind::Object { model: 0 },
            x: px,
            y: py,
            z: pz,
            owner,
            orientation: so.orientation,
            parent_slab: slb_num,
            slabset_id: slabset_id as u16,
            stl_id,
            check_stl,
        };
        if so.is_light == 1 {
            out.push(SlabObject {
                kind: SlabObjectKind::Light {
                    intensity: so.model,
                    radius: so.range as i32 * 256,
                },
                owner: PLAYER_NEUTRAL,
                orientation: 0,
                ..base
            });
        } else if so.is_light == 0 {
            if so.class_id == TCLS_OBJECT {
                let mut model = so.model;
                let b = gd.coloured_object_base(model);
                if b != 0 {
                    model = gd.coloured_object_model(b, player_color_idx(owner));
                }
                if model <= 0 {
                    continue;
                }
                if model == OBJ_PRISON_BAR
                    && stl_id != 4
                    && !prison_bar_needed(c, stl_x / 3, stl_y / 3, stl_id)
                {
                    continue;
                }
                out.push(SlabObject {
                    kind: SlabObjectKind::Object { model },
                    ..base
                });
            } else if so.class_id == TCLS_EFFECTGEN {
                out.push(SlabObject {
                    kind: SlabObjectKind::EffectGen {
                        model: so.model,
                        range: so.range as i32 * 256,
                    },
                    ..base
                });
            }
            // Any other class: "Stupid thing class", nothing is created.
        }
    }
}

/// The prison bar test of `place_slab_object` (src/map_blocks.c:880-915).
fn prison_bar_needed<V: SlabView>(c: &Ctx<V>, slb_x: i32, slb_y: i32, stl_id: u8) -> bool {
    let filled = |s: &SlabConfig| {
        s.block_flags & (slab_flags::FILLED | slab_flags::DIGGABLE | slab_flags::VALUABLE) != 0
    };
    if stl_id & 1 != 0 {
        let d = AROUND_NINE[stl_id as usize];
        !filled(c.stats_at(slb_x + d.0, slb_y + d.1))
    } else {
        let icorn = SLAB_ELEMENT_TO_CORNER[stl_id as usize];
        let (mut nfilled, mut nprison) = (0, 0);
        if icorn != -1 {
            for d in [
                AROUND_EIGHT[icorn as usize],
                AROUND_EIGHT[(icorn as usize + 2) & 7],
            ] {
                let (sx, sy) = (slb_x + d.0, slb_y + d.1);
                if filled(c.stats_at(sx, sy)) {
                    nfilled += 1;
                }
                if c.kind(sx, sy) == kinds::PRISON {
                    nprison += 1;
                }
            }
        }
        nprison + nfilled < 2
    }
}

/// Columns, wibble and objects of slab (x, y) as the game places it.
///
/// Animated slabs (`Animated = 1`: doors, bridge, gems, purple path, slab 50) go through
/// `dump_slab_on_map` (src/map_blocks.c:1471) with frame 0, except the bridge whose frame is 1 over
/// lava (`place_room` uses `subtile_has_lava_on_top`, src/room_data.c:3316; here the `.wlb` value
/// tells). Doors are re-dumped like this by `create_door` when the level loads (src/thing_doors.c:140).
/// Everything else goes through `place_single_slab_type_on_map` (src/map_blocks.c:1328).
pub fn slab_columns<V: SlabView>(v: &V, gd: &GameData, x: i32, y: i32) -> SlabGen {
    let c = Ctx { v, gd };
    let kind = c.kind(x, y);
    let owner = c.owner(x, y);
    let stats = gd.slab(kind as usize);
    let dims = v.dims();
    let slb_num = slab_number(dims, x, y);
    let mut types = [kind as i32; 9];
    let ids = if stats.animated != 0 {
        let frame = if stats.wlb_type == WLB_BRIDGE && v.wlb(x, y) == WLB_LAVA {
            1
        } else {
            0
        };
        let mut id = SLABSETS_PER_SLAB * kind as usize + frame;
        if id >= gd.slabset.len() {
            id = 0;
        }
        [id; 9]
    } else {
        let mut style = [0u8; 9];
        let mut pretty = [0u8; 9];
        if stats.category == category::FORTIFIED_WALL {
            fill_arrays_std(&c, x, y, &mut types, &mut pretty);
        }
        let mut ids = [slabset_index(gd, kind as i32, 3, 0); 9];
        fill_style_array(&c, x, y, &mut style);
        if kind == kinds::WALLTORCH || kind == kinds::TORCHDIRT {
            set_torch_places(&c, kind, x, y, &mut types);
        }
        prepare_column_index(&c, kind, x, y, owner, &types, &pretty, &style, &mut ids);
        if stats.category == category::FORTIFIED_WALL {
            modify_column_near_liquid(&c, x, y, &mut types, &style, &mut ids);
        }
        ids
    };
    let mut gen = SlabGen {
        columns: [Column::default(); 9],
        wibble: [0; 9],
        objects: vec![],
        slabset_ids: [0; 9],
        subtile_kinds: [0; 9],
        unresolved: [false; 9],
    };
    for i in 0..9 {
        let (dx, dy) = ((i % 3) as i32, (i / 3) as i32);
        let (sx, sy) = (3 * x + dx, 3 * y + dy);
        gen.slabset_ids[i] = ids[i] as u16;
        gen.subtile_kinds[i] = (ids[i] / SLABSETS_PER_SLAB) as u8;
        match gd.slabset_column(ids[i], i) {
            Some(t) => gen.columns[i] = owner_column(gd, t, owner),
            None => gen.unresolved[i] = true,
        }
        gen.wibble[i] = subtile_wibble(v, gd, kind, sx, sy);
        slab_objects_for(
            &c,
            &mut gen.objects,
            slb_num,
            sx,
            sy,
            ids[i],
            i as u8,
            owner,
        );
    }
    gen
}

/// `update_map_collide` (src/slab_data.c:596): map flags of a subtile holding `col` in a slab of
/// `kind`.
pub fn subtile_flags(gd: &GameData, kind: u8, col: &Column) -> u32 {
    let mut smask = col.solidmask;
    let mut z = 0;
    while z < 8 && smask & 1 != 0 {
        smask >>= 1;
        z += 1;
    }
    let s = gd.slab(kind as usize);
    if (s.block_flags_height as i32) < z {
        s.block_flags
    } else {
        s.noblck_flags
    }
}

/// A rectangle of slabs, inclusive bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlabRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl SlabRect {
    pub fn single(x: i32, y: i32) -> Self {
        SlabRect {
            x0: x,
            y0: y,
            x1: x,
            y1: y,
        }
    }
    pub fn grow(self, n: i32) -> Self {
        SlabRect {
            x0: self.x0 - n,
            y0: self.y0 - n,
            x1: self.x1 + n,
            y1: self.y1 + n,
        }
    }
    pub fn clip(self, d: MapDims) -> Self {
        SlabRect {
            x0: self.x0.max(0),
            y0: self.y0.max(0),
            x1: self.x1.min(d.w as i32 - 1),
            y1: self.y1.min(d.h as i32 - 1),
        }
    }
    pub fn union(self, o: SlabRect) -> Self {
        SlabRect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.x0 > self.x1 || self.y0 > self.y1
    }
    pub fn iter(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        let s = *self;
        (s.y0..=s.y1).flat_map(move |y| (s.x0..=s.x1).map(move |x| (x, y)))
    }
}

/// Generated columns / wibble / objects of a whole map.
#[derive(Clone, Debug)]
pub struct MapGen {
    pub dims: MapDims,
    /// `(3W+1) x (3H+1)` subtile grid. The extra row and column are not touched by slab
    /// placement; they start as the default column (or whatever the caller puts there).
    pub columns: Vec<Column>,
    pub wibble: Vec<u8>,
    /// Per-slab unfiltered objects (index = slab index).
    pub slab_objects: Vec<Vec<SlabObject>>,
    /// Per-slab slabset ids / kinds used, for diagnostics.
    pub slabset_ids: Vec<[u16; 9]>,
    pub subtile_kinds: Vec<[u8; 9]>,
    /// Count of subtiles whose slabset column was unresolved.
    pub unresolved: usize,
}

impl MapGen {
    /// An empty generation (all default) for `dims`.
    pub fn empty(dims: MapDims) -> Self {
        MapGen {
            dims,
            columns: vec![Column::default(); dims.stl_grid()],
            wibble: vec![0; dims.stl_grid()],
            slab_objects: vec![vec![]; dims.slabs()],
            slabset_ids: vec![[0; 9]; dims.slabs()],
            subtile_kinds: vec![[0; 9]; dims.slabs()],
            unresolved: 0,
        }
    }

    /// Place every slab.
    pub fn regenerate_all<V: SlabView>(v: &V, gd: &GameData) -> Self {
        let d = v.dims();
        let mut g = MapGen::empty(d);
        g.regenerate_rect(
            v,
            gd,
            SlabRect {
                x0: 0,
                y0: 0,
                x1: d.w as i32 - 1,
                y1: d.h as i32 - 1,
            },
        );
        g
    }

    /// Recompute the slabs of `rect` (clipped).
    pub fn regenerate_rect<V: SlabView>(&mut self, v: &V, gd: &GameData, rect: SlabRect) {
        let d = self.dims;
        for (x, y) in rect.clip(d).iter() {
            let gen = slab_columns(v, gd, x, y);
            let si = d.slab_index(x as usize, y as usize);
            for i in 0..9 {
                let (sx, sy) = (3 * x as usize + i % 3, 3 * y as usize + i / 3);
                let k = d.stl_index(sx, sy);
                if !gen.unresolved[i] {
                    self.columns[k] = gen.columns[i];
                }
                self.wibble[k] = gen.wibble[i];
            }
            self.unresolved += gen.unresolved.iter().filter(|&&u| u).count();
            self.slab_objects[si] = gen.objects;
            self.slabset_ids[si] = gen.slabset_ids;
            self.subtile_kinds[si] = gen.subtile_kinds;
        }
    }

    /// Slab (x, y) changed (kind or owner): recompute it and its 8 neighbours, the set
    /// `place_slab_type_on_map_f` re-places (src/map_blocks.c:1706, 1748-1758). That set also covers
    /// every liquid slab whose wibble depends on (x, y). Returns the rectangle whose columns
    /// changed. Objects of a further ring may change their `BLOCKING` filter result;
    /// [`MapGen::placed_objects`] always filters with the current columns.
    pub fn regenerate_around<V: SlabView>(
        &mut self,
        v: &V,
        gd: &GameData,
        x: i32,
        y: i32,
    ) -> SlabRect {
        let r = SlabRect::single(x, y).grow(1).clip(self.dims);
        self.regenerate_rect(v, gd, r);
        r
    }

    /// Column at subtile (stl_x, stl_y), `None` outside the grid.
    pub fn column_at(&self, stl_x: i32, stl_y: i32) -> Option<&Column> {
        let d = self.dims;
        if stl_x < 0 || stl_y < 0 || stl_x as usize > d.stl_w() || stl_y as usize > d.stl_h() {
            return None;
        }
        self.columns
            .get(d.stl_index(stl_x as usize, stl_y as usize))
    }

    /// Whether the game would see `BLOCKING` at a subtile (`update_map_collide` with the slab
    /// containing it). Subtiles outside the slab area (including the extra row/column) are not
    /// blocking.
    pub fn subtile_blocking<V: SlabView>(
        &self,
        v: &V,
        gd: &GameData,
        stl_x: i32,
        stl_y: i32,
    ) -> bool {
        subtile_blocking(&self.columns, v, gd, stl_x, stl_y)
    }

    /// Slab objects of the slabs in `rect` that the game would actually create.
    pub fn placed_objects_in<V: SlabView>(
        &self,
        v: &V,
        gd: &GameData,
        rect: SlabRect,
    ) -> Vec<SlabObject> {
        let d = self.dims;
        let mut out = vec![];
        for (x, y) in rect.clip(d).iter() {
            for o in &self.slab_objects[d.slab_index(x as usize, y as usize)] {
                if !self.subtile_blocking(v, gd, o.check_stl.0, o.check_stl.1) {
                    out.push(*o);
                }
            }
        }
        out
    }

    /// All slab objects the game would create, in slab order.
    pub fn placed_objects<V: SlabView>(&self, v: &V, gd: &GameData) -> Vec<SlabObject> {
        let d = self.dims;
        self.placed_objects_in(
            v,
            gd,
            SlabRect {
                x0: 0,
                y0: 0,
                x1: d.w as i32 - 1,
                y1: d.h as i32 - 1,
            },
        )
    }
}

/// `alter_rock_style` (src/map_blocks.c:1613): plain earth next to ground / room / obstacle becomes
/// torch dirt when the torch grid says so.
pub fn alter_rock_style<V: SlabView>(v: &V, gd: &GameData, kind: u8, x: i32, y: i32) -> u8 {
    if kind != kinds::EARTH {
        return kind;
    }
    for (dx, dy) in AROUND_EIGHT {
        let Some(k) = v.kind(x + dx, y + dy) else {
            continue;
        };
        let c = gd.slab(k as usize).category;
        if c == category::FORTIFIED_GROUND
            || c == category::ROOM_INTERIOR
            || c == category::OBSTACLE
        {
            return if torch_flags_for_slab(v, x, y) < 1 {
                kinds::EARTH
            } else {
                kinds::TORCHDIRT
            };
        }
    }
    kinds::EARTH
}

/// `slab_kind_from_wlb_type` (src/config_terrain.c:762).
pub fn slab_kind_from_wlb(gd: &GameData, wlb: u8) -> Option<usize> {
    if wlb == 0 || wlb == WLB_BRIDGE {
        return None;
    }
    gd.slabs.iter().position(|s| s.wlb_type == wlb)
}

/// `initialise_map_wlb_auto` (src/lvl_filesdk1.c:1098) for one slab: the `.wlb` value the game
/// derives when there is no `.wlb` file. Bridges take the liquid most present around them
/// (water first, then any bridgeable kind with a strictly larger count), default path.
pub fn auto_wlb<V: SlabView>(v: &V, gd: &GameData, x: i32, y: i32) -> u8 {
    let kind = v.kind(x, y).unwrap_or(0) as usize;
    if gd.slab(kind).wlb_type != WLB_BRIDGE {
        return gd.slab(kind).wlb_type;
    }
    let count = |k: usize| {
        let mut n = 0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if v.kind(x + dx, y + dy) == Some(k as u8) {
                    n += 1;
                }
            }
        }
        n
    };
    let mut n = slab_kind_from_wlb(gd, WLB_WATER)
        .map(|k| k as i64)
        .unwrap_or(-1);
    let mut best = if n >= 0 { count(n as usize) } else { 0 };
    for (k, s) in gd.slabs.iter().enumerate() {
        if s.wlb_type != 0 && s.wlb_type != WLB_BRIDGE && count(k) > best {
            n = k as i64;
            best = count(k);
        }
    }
    if n < 0 {
        n = kinds::PATH as i64;
    }
    gd.slab(n as usize).wlb_type
}

/// [`MapGen::subtile_blocking`] over any column grid of `v`'s map (no copy of the grid).
pub fn subtile_blocking<V: SlabView>(
    columns: &[Column],
    v: &V,
    gd: &GameData,
    stl_x: i32,
    stl_y: i32,
) -> bool {
    let d = v.dims();
    if stl_x < 0 || stl_y < 0 || stl_x as usize >= d.stl_w() || stl_y as usize >= d.stl_h() {
        return false;
    }
    let kind = v.kind(stl_x / 3, stl_y / 3).unwrap_or(0);
    let Some(col) = columns.get(d.stl_index(stl_x as usize, stl_y as usize)) else {
        return false;
    };
    subtile_flags(gd, kind, col) & slab_flags::BLOCKING != 0
}
