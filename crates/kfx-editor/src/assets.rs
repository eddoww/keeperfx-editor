//! Install-wide display assets (palette, texture animation, panel sprites) and the helpers the
//! UI needs on top of `kfx_map::gamedata::GameData` (palette previews, slab colours, player
//! colours). All game configuration (terrain, cubes, slab sets, column sets) comes from
//! `kfx_map`.
use kfx_gfx::sheet::SpriteSheet;
use kfx_gfx::textures::TextureAnim;
use kfx_gfx::{Install, Palette, Rgba};
use kfx_map::gamedata::{category, slab_flags, GameData, SlabsetColumn, SLABSETS_PER_SLAB};
use kfx_prefab::i18n::{trd, trf, tx, Tx};

/// Face order of a cube's `Textures` (`config_cubes.c`): North, East, South, West, Top, Bottom.
pub const FACE_N: usize = 0;
pub const FACE_E: usize = 1;
pub const FACE_S: usize = 2;
pub const FACE_W: usize = 3;
pub const FACE_TOP: usize = 4;
pub const FACE_BOTTOM: usize = 5;

/// Slab set style `ALL` (`slab_styles_commands`, src/config_slabsets.c:57-85).
pub const STYLE_ALL: usize = 8;

/// Player owner names and swatch colours, by player number (`src/player_instances.h:31-41`).
pub const PLAYERS: [(Tx, [u8; 3]); 9] = [
    (
        tx("kfx_editor.keeper_1_red", "Keeper 1 (Red)"),
        [200, 40, 32],
    ),
    (
        tx("kfx_editor.keeper_2_blue", "Keeper 2 (Blue)"),
        [48, 88, 220],
    ),
    (
        tx("kfx_editor.keeper_3_green", "Keeper 3 (Green)"),
        [40, 170, 60],
    ),
    (
        tx("kfx_editor.keeper_4_yellow", "Keeper 4 (Yellow)"),
        [230, 200, 40],
    ),
    (tx("kfx_editor.good_white", "Good (White)"), [235, 235, 235]),
    (tx("kfx_editor.neutral", "Neutral"), [120, 110, 100]),
    (
        tx("kfx_editor.keeper_5_purple", "Keeper 5 (Purple)"),
        [150, 60, 190],
    ),
    (
        tx("kfx_editor.keeper_6_black", "Keeper 6 (Black)"),
        [40, 40, 44],
    ),
    (
        tx("kfx_editor.keeper_7_orange", "Keeper 7 (Orange)"),
        [235, 130, 30],
    ),
];

/// Order shown in the player tree: Good, Neutral, then keepers.
pub const PLAYER_TREE_ORDER: [u8; 9] = [4, 5, 0, 1, 2, 3, 6, 7, 8];

/// Texture ids of the 6 faces of every cube, the only cube data the mesher needs.
#[derive(Clone, Debug, Default)]
pub struct CubeFaces(pub Vec<[u16; 6]>);

impl CubeFaces {
    pub fn from_gamedata(gd: &GameData) -> CubeFaces {
        CubeFaces(gd.cubes.iter().map(|c| c.textures).collect())
    }
    /// Texture id of face `face` of cube `cube` (0 for unknown cubes, like the game's
    /// zero-initialised `CubeConfigStats`).
    pub fn face(&self, cube: u16, face: usize) -> u16 {
        self.0.get(cube as usize).map(|t| t[face]).unwrap_or(0)
    }
}

/// Number of consecutive solid cubes from the bottom (`floor_height_table[solidmask & 0xFF]`,
/// src/engine_arrays.c:1010-1044).
pub fn floor_height(solidmask: u16) -> usize {
    (solidmask as u32 & 0xFF).trailing_ones() as usize
}

/// Top texture id of a column: the top face of the cube below the first gap
/// (src/engine_render.c:4152-4163), or the floor texture when the bottom cube is empty.
pub fn column_top_texture(
    cubes: &CubeFaces,
    solidmask: u16,
    floor_texture: u16,
    cube_ids: &[u16; 8],
) -> u16 {
    let h = floor_height(solidmask);
    if h > 0 {
        cubes.face(cube_ids[h - 1], FACE_TOP)
    } else {
        floor_texture
    }
}

/// Top texture ids of the 9 subtiles of slab kind `kind` (row-major), used for palette
/// previews. Animated slabs are drawn by the game from slab set `kind * 28 + frame`
/// (`slab_columns` in kfx-map, src/map_blocks.c:1471), so frame 0 is used; other slabs use style
/// `ALL`, or style `S` / `CENTER` when every `ALL` top is texture 0 (the entrance's `ALL` set is
/// only a ceiling over an empty floor). `None` for unresolved columns.
pub fn slab_preview_textures(gd: &GameData, cubes: &CubeFaces, kind: usize) -> [Option<u16>; 9] {
    let styles: &[usize] = if gd.slab(kind).animated != 0 {
        &[0]
    } else {
        &[STYLE_ALL, 0, 27]
    };
    let mut first = None;
    for &style in styles {
        let mut out = [None; 9];
        let Some(set) = gd.slabset.get(kind * SLABSETS_PER_SLAB + style) else {
            continue;
        };
        for (i, c) in set.iter().enumerate() {
            if let SlabsetColumn::Column(c) = c {
                out[i] = Some(column_top_texture(
                    cubes,
                    c.solidmask,
                    c.floor_texture,
                    &c.cubes,
                ));
            }
        }
        if out.iter().any(|t| t.is_some_and(|t| t != 0)) {
            return out;
        }
        first.get_or_insert(out);
    }
    first.unwrap_or([None; 9])
}

/// Is slab `kind` a door (`IS_DOOR` block flag)? Doors are placed with things, not slabs.
pub fn is_door(gd: &GameData, kind: usize) -> bool {
    gd.slab(kind).block_flags & slab_flags::IS_DOOR != 0
}

/// `slab_kind_is_room_wall` (src/config_terrain.c:709).
pub fn is_room_wall(gd: &GameData, kind: usize) -> bool {
    let s = gd.slab(kind);
    s.category == category::FORTIFIED_WALL && s.slb_id != 0
}

/// "Solid" for the hover readout: a full-height block (rock, earth, walls, gold, gems).
pub fn is_solid(gd: &GameData, kind: usize) -> bool {
    gd.slab(kind).block_flags & (slab_flags::FILLED | slab_flags::DIGGABLE | slab_flags::VALUABLE)
        != 0
}

/// Representative colour of a slab for the minimap and previews: liquids, gold, rock, earth,
/// walls and floors get fixed colours, owned floors and walls take their owner's colour.
pub fn slab_color(gd: &GameData, kind: u16, owner: u8) -> [u8; 3] {
    use kfx_map::autotile::kinds as k;
    let s = gd.slab(kind as usize);
    let own = |base: [u8; 3], f: f32| -> [u8; 3] {
        if (owner as usize) < 9 && owner != kfx_map::autotile::PLAYER_NEUTRAL {
            let p = PLAYERS[owner as usize].1;
            [0, 1, 2].map(|i| (base[i] as f32 * (1.0 - f) + p[i] as f32 * f) as u8)
        } else {
            base
        }
    };
    match kind as u8 {
        x if x == k::ROCK => [52, 50, 52],
        x if x == k::GOLD => [214, 178, 40],
        x if x == k::GEMS => [90, 230, 210],
        x if x == k::EARTH || x == k::TORCHDIRT => [112, 78, 46],
        x if x == k::LAVA => [236, 92, 18],
        x if x == k::WATER => [40, 92, 210],
        x if x == k::PATH => [150, 124, 86],
        _ if s.name.eq_ignore_ascii_case("DENSE_GOLD") => [240, 200, 60],
        _ if s.name.eq_ignore_ascii_case("HARD_FLOOR") => [96, 96, 104],
        _ if s.name.eq_ignore_ascii_case("ABYSS") => [16, 10, 24],
        _ => match s.category {
            category::FORTIFIED_WALL => own([150, 146, 140], 0.35),
            category::FORTIFIED_GROUND => own([132, 112, 92], 0.45),
            category::ROOM_INTERIOR => own([96, 140, 96], 0.55),
            category::OBSTACLE => own([170, 120, 70], 0.5),
            _ => own([128, 120, 110], 0.3),
        },
    }
}

/// Everything loaded once per install that is not game configuration.
pub struct Assets {
    pub install: Install,
    pub palette: Palette,
    pub anim: TextureAnim,
    pub gui2: Option<SpriteSheet>,
    /// `gui1-64`: button sprites, creature portraits (`QuerySymbol`, GBS_creature_portrait_*).
    pub gui1: Option<SpriteSheet>,
}

impl Assets {
    pub fn load(install: Install) -> Result<Assets, String> {
        let palette = Palette::load(&install)
            .map_err(|e| trf("kfx_editor.palette_e", "palette: {e}", &[("e", &e)]))?;
        // the game data the editor needs must exist; check terrain.cfg early for a clear error
        if install.path("fxdata/terrain.cfg").is_none() {
            return Err(trd(
                "kfx_editor.fxdata_terrain_cfg_not_found",
                "fxdata/terrain.cfg not found",
            ));
        }
        let anim = TextureAnim::load(&install).unwrap_or_default();
        let gui2 = SpriteSheet::load(&install, "gui2-64").ok();
        let gui1 = SpriteSheet::load(&install, "gui1-64").ok();
        Ok(Assets {
            install,
            palette,
            anim,
            gui2,
            gui1,
        })
    }

    /// A gui2-64 sprite as RGBA, `None` when missing or blank (fully transparent / black).
    pub fn panel_icon(&self, index: u16) -> Option<Rgba> {
        let s = self.gui2.as_ref()?;
        let img = s.decode(index as usize, &self.palette).ok()?;
        let visible = img
            .data
            .chunks_exact(4)
            .any(|p| p[3] > 0 && (p[0] as u16 + p[1] as u16 + p[2] as u16) > 24);
        (img.w > 0 && img.h > 0 && visible).then_some(img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_height_matches_table() {
        assert_eq!(floor_height(0), 0);
        assert_eq!(floor_height(0b1), 1);
        assert_eq!(floor_height(0b11111), 5);
        assert_eq!(floor_height(0b11101), 1);
        assert_eq!(floor_height(0xFF), 8);
    }

    #[test]
    fn cube_faces_lookup() {
        let c = CubeFaces(vec![[0; 6], [1, 2, 3, 4, 5, 6]]);
        assert_eq!(c.face(1, FACE_TOP), 5);
        assert_eq!(c.face(1, FACE_BOTTOM), 6);
        assert_eq!(c.face(9, FACE_N), 0);
    }
}
