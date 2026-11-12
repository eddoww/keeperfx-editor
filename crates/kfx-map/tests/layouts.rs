//! Hand-built layouts checked against values derived by reading the C (map_blocks.c), using the
//! shipped fxdata of the test install (skipped when absent).
//!
//! The expected slabset entries are worked out in the comments from `get_against`,
//! `slab_primitive`, `special_cases`, `place_single_slab_fill_style_array` and `get_slabset_index`;
//! the expected column *content* is read independently from `slabset.toml` / `columnset.toml` with
//! the plain `toml` crate.
use kfx_formats::level_bin::{Column, MapDims};
use kfx_formats::testutil;
use kfx_map::autotile::{kinds, slab_columns, Layout, MapGen, SlabObjectKind, PLAYER_NEUTRAL};
use kfx_map::column::{create_column_bitfields, solidmask_of};
use kfx_map::gamedata::{ConfigLayers, GameData, SLAB_STYLE_NAMES};

struct Fx {
    gd: GameData,
    slabset: toml::Table,
    columnset: toml::Table,
}

fn fx() -> Option<Fx> {
    let root = testutil::install_dir()?;
    let fxdata = testutil::resolve_ci(&root, "fxdata")?;
    let gd = GameData::load(&ConfigLayers::base(&fxdata)).unwrap();
    let read = |n: &str| {
        std::fs::read_to_string(fxdata.join(n))
            .unwrap()
            .parse::<toml::Table>()
            .unwrap()
    };
    Some(Fx {
        gd,
        slabset: read("slabset.toml"),
        columnset: read("columnset.toml"),
    })
}

impl Fx {
    /// Column `pos` of `[slab<kind>.<style>]` straight from the TOML files, owner-substituted the
    /// way copy_block_with_cube_groups does.
    fn expect(&self, kind: usize, style: usize, pos: usize, owner: u8) -> Column {
        let id = self.slabset[&format!("slab{kind}")][SLAB_STYLE_NAMES[style]]["Columns"][pos]
            .as_integer()
            .unwrap();
        let c = &self.columnset[&format!("column{id}")];
        let mut col = Column {
            floor_texture: c["FloorTexture"].as_integer().unwrap() as u16,
            orient: c["Orientation"].as_integer().unwrap() as u8,
            ..Default::default()
        };
        for (k, v) in c["Cubes"].as_array().unwrap().iter().enumerate() {
            col.cubes[k] = v.as_integer().unwrap() as u16;
        }
        col.solidmask = solidmask_of(&col.cubes);
        let color = if owner == PLAYER_NEUTRAL {
            5
        } else {
            owner as usize
        };
        for cube in col.cubes.iter_mut() {
            if *cube > 0 {
                let g = self.gd.cubes[*cube as usize].ownership_group as usize;
                if g > 0 && self.gd.cube_bits[g][color] > 0 {
                    *cube = self.gd.cube_bits[g][color];
                }
            }
        }
        col.bitfields = create_column_bitfields(&col.cubes, 0);
        col
    }
}

fn layout(kind: u8, owner: u8) -> Layout {
    Layout::new(MapDims::new(3, 3), kind, owner)
}

fn same(a: &Column, b: &Column) -> bool {
    a.cubes == b.cubes
        && a.floor_texture == b.floor_texture
        && a.solidmask == b.solidmask
        && a.orient == b.orient
        && (a.bitfields & 0xFE) == (b.bitfields & 0xFE)
}

#[test]
fn lone_room_tile() {
    let Some(f) = fx() else { return };
    // Treasury (16) of player 0 surrounded by neutral earth. Earth's SlbID (0) differs from the
    // treasury's (5) on all four sides -> against = 0b1111 -> slab_primitive[15] = 8 (ALL).
    // Neighbour fill styles are 0, nothing is a wall, so every subtile uses slab16.ALL.
    let mut l = layout(kinds::EARTH, PLAYER_NEUTRAL);
    l.set(1, 1, 16, 0);
    let g = slab_columns(&l, &f.gd, 1, 1);
    assert_eq!(g.slabset_ids, [16 * 28 + 8; 9]);
    for i in 0..9 {
        assert!(
            same(&g.columns[i], &f.expect(16, 8, i, 0)),
            "subtile {i}: {:?}",
            g.columns[i]
        );
    }
    // Same tile with a same-owner treasury to the east: against = N|S|W = 0b1011 = 11 ->
    // special case 3 = [5,5,5,8,8,5,4,4,4] (NW NW NW ALL ALL NW SW SW SW).
    l.set(2, 1, 16, 0);
    let g = slab_columns(&l, &f.gd, 1, 1);
    let picks = [5, 5, 5, 8, 8, 5, 4, 4, 4];
    for i in 0..9 {
        assert_eq!(g.slabset_ids[i] as usize, 16 * 28 + picks[i]);
        assert!(same(&g.columns[i], &f.expect(16, picks[i], i, 0)));
    }
}

#[test]
fn wall_between_two_owners() {
    let Some(f) = fx() else { return };
    // Drape wall (4) of player 0. West: wall of player 0 (same SlbID 0, same owner -> 0).
    // East: wall of player 1 (same SlbID, owner differs and is not neutral -> 1).
    // North/South: neutral rock (SlbID 0, owner neutral, not claimed -> 0).
    // against = E = 0b0100 = 4 -> slab_primitive[4] = 3 (E) for all 9 subtiles.
    let mut l = layout(kinds::ROCK, PLAYER_NEUTRAL);
    l.set(0, 1, kinds::WALLDRAPE, 0);
    l.set(1, 1, kinds::WALLDRAPE, 0);
    l.set(2, 1, kinds::WALLDRAPE, 1);
    let g = slab_columns(&l, &f.gd, 1, 1);
    assert_eq!(g.slabset_ids, [4 * 28 + 3; 9]);
    for i in 0..9 {
        assert!(same(&g.columns[i], &f.expect(4, 3, i, 0)));
    }
    // The east wall sees the west one as foreign: its own mask is W (1) -> pick 1, and its
    // columns use player 1's cubes.
    let g = slab_columns(&l, &f.gd, 2, 1);
    // its east neighbour is outside the map -> against also has E: 0b0101 = 5 -> special case 0.
    let picks = [1, 1, 3, 1, 1, 3, 1, 1, 3];
    for i in 0..9 {
        assert_eq!(g.slabset_ids[i] as usize, 4 * 28 + picks[i]);
        assert!(same(&g.columns[i], &f.expect(4, picks[i], i, 1)));
    }
    // The two walls really differ in their owner cubes.
    let g0 = slab_columns(&l, &f.gd, 1, 1);
    assert_ne!(g0.columns[0].cubes, g.columns[0].cubes);
}

#[test]
fn water_next_to_path() {
    let Some(f) = fx() else { return };
    // Path (10, SlbID 1) with water (13, SlbID 4, FillStyle 2) to the east.
    // against = E -> pick 3. fill_style_array marks subtiles 2,5,8 (the east column, from
    // slab_element_around_eight[1..=3] = -2, 1, 4) with style 2 -> E_WATER (9*2+3 = 21).
    let mut l = layout(kinds::PATH, PLAYER_NEUTRAL);
    l.set(2, 1, kinds::WATER, PLAYER_NEUTRAL);
    let g = slab_columns(&l, &f.gd, 1, 1);
    for i in 0..9 {
        let style = if i % 3 == 2 { 21 } else { 3 };
        assert_eq!(g.slabset_ids[i] as usize, 10 * 28 + style, "subtile {i}");
        assert!(same(&g.columns[i], &f.expect(10, style, i, PLAYER_NEUTRAL)));
    }
    assert_eq!(g.wibble, [1; 9]);
    // Water: liquid wibble (2) except on subtiles whose left/upper neighbour slab is not liquid
    // (set_subtile_wibble): column dx=0 looks at the path to the west, row dy=0 at the path
    // above -> 1 there, 2 inside.
    let w = slab_columns(&l, &f.gd, 2, 1);
    assert_eq!(w.wibble, [1, 1, 1, 1, 2, 2, 1, 2, 2]);
}

#[test]
fn door_slab() {
    let Some(f) = fx() else { return };
    // Wooden door (42, Animated) of player 2: dump_slab_on_map with frame 0, i.e. slabset entry
    // 42*28+0 (style "S") for all subtiles regardless of neighbours.
    let mut l = layout(kinds::CLAIMED, 2);
    l.set(1, 0, kinds::WALLDRAPE, 2);
    l.set(1, 2, kinds::WALLDRAPE, 2);
    l.set(1, 1, 42, 2);
    let g = slab_columns(&l, &f.gd, 1, 1);
    assert_eq!(g.slabset_ids, [42 * 28; 9]);
    for i in 0..9 {
        assert!(same(&g.columns[i], &f.expect(42, 0, i, 2)));
    }
    assert!(g.objects.is_empty());
}

#[test]
fn torch_dirt_objects() {
    let Some(f) = fx() else { return };
    // Torch dirt (3) at slab (5, 2) with claimed path south of it (5, 3):
    // torch_flags_for_slab: x % 5 == 0 and a claimed slab north/south -> flag 1.
    // set_torch_places: subtiles 3 and 5 take the undecorated kind (earth, 2); (x+y)&1 = 1 ->
    // subtile 1 too (the slab north is earth, not a room). Subtile 7 keeps torch dirt.
    // get_against: only the south (claimed, SlbID 2 vs 0) differs -> mask 0b0010 -> pick 0 (S).
    let mut l = Layout::new(MapDims::new(7, 5), kinds::EARTH, PLAYER_NEUTRAL);
    l.set(5, 2, kinds::TORCHDIRT, PLAYER_NEUTRAL);
    l.set(5, 3, kinds::CLAIMED, 0);
    let g = slab_columns(&l, &f.gd, 5, 2);
    for i in [1usize, 3, 5] {
        assert_eq!(g.subtile_kinds[i], kinds::EARTH, "subtile {i}");
        assert_eq!(g.slabset_ids[i], 2 * 28);
    }
    for i in [0usize, 2, 4, 6, 7, 8] {
        assert_eq!(g.subtile_kinds[i], kinds::TORCHDIRT, "subtile {i}");
        assert_eq!(g.slabset_ids[i], 3 * 28);
    }
    assert!(same(&g.columns[7], &f.expect(3, 0, 7, PLAYER_NEUTRAL)));
    assert!(same(&g.columns[3], &f.expect(2, 0, 3, PLAYER_NEUTRAL)));
    // slab3.S has one object: a torch (object 2) on subtile 7 at [128, 320, 736]. Its position
    // falls in subtile (16, 9), the claimed slab: not blocking, so it is created.
    let gen = MapGen::regenerate_all(&l, &f.gd);
    let objs = gen.placed_objects(&l, &f.gd);
    assert_eq!(objs.len(), 1, "{objs:?}");
    let o = objs[0];
    assert_eq!(o.kind, SlabObjectKind::Object { model: 2 });
    assert_eq!((o.x, o.y, o.z), ((16 << 8) + 128, (8 << 8) + 320, 736));
    assert_eq!(o.parent_slab, 2 * 7 + 5);
    // With parity even (slab (5, 1), claimed at (5, 2)) subtile 7 becomes earth: no torch.
    let mut l = Layout::new(MapDims::new(7, 5), kinds::EARTH, PLAYER_NEUTRAL);
    l.set(5, 1, kinds::TORCHDIRT, PLAYER_NEUTRAL);
    l.set(5, 2, kinds::CLAIMED, 0);
    let gen = MapGen::regenerate_all(&l, &f.gd);
    assert!(gen.placed_objects(&l, &f.gd).is_empty());
}
