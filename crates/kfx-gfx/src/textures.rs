//! Block texture atlases (`tmapaNNN.dat` / `tmapbNNN.dat`) and animated texture slots.
//!
//! Each file is raw 8-bit palette indices, 256 px wide: 8 tiles of 32x32 per row, 544 tiles
//! (557056 bytes), `src/engine_textures.h:27-33`, `TileAtlasPacker.cpp:60-100`.
//! Texture id space (`engine_textures.h`, `fxdata/textureanim.toml`):
//! 0..=543 tmapa, 544..=999 animated slots (8 frames each), 1000..=1543 tmapb.
use crate::image::Rgba;
use crate::install::{find_in_dir, Install};
use crate::palette::Palette;
use crate::{Error, Result};
use std::path::{Path, PathBuf};

pub const TILE_DIM: usize = 32;
pub const TILES_PER_ROW: usize = 8;
pub const TILES_PER_FILE: usize = 544;
pub const ATLAS_W: usize = TILE_DIM * TILES_PER_ROW;
pub const ATLAS_H: usize = TILE_DIM * (TILES_PER_FILE / TILES_PER_ROW);
pub const FILE_BYTES: usize = ATLAS_W * ATLAS_H;
pub const ANIM_FIRST: u16 = 544;
pub const B_FIRST: u16 = 1000;
pub const ANIM_FRAMES: usize = 8;
/// Number of animated slots (ids 544..=999).
pub const ANIM_SLOTS: usize = (B_FIRST - ANIM_FIRST) as usize;
/// Number of texture sets the game keeps (`TEXTURE_VARIATIONS_COUNT`).
pub const TEXTURE_VARIATIONS: usize = 32;
/// Fill value of tiles whose file is missing (`engine_textures.c:load_texture_map_file` memset
/// 130).
pub const MISSING_FILL: u8 = 130;
/// Palette ids of the "marked land/gold" overlay tiles used by the game
/// (`src/engine_textures.h:40-41`).
pub const TEXTURE_LAND_MARKED_LAND: u16 = 578;
pub const TEXTURE_LAND_MARKED_GOLD: u16 = 579;

/// Which half of the texture space a tile lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    A,
    B,
}

/// A concrete (non-animated) tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileRef {
    pub part: Part,
    /// 0..544 inside its file.
    pub tile: u16,
}

/// Map a texture id to its static tile. `None` for animated slots (544..=999) and ids >= 1544.
pub fn static_tile(id: u16) -> Option<TileRef> {
    if (id as usize) < TILES_PER_FILE {
        Some(TileRef {
            part: Part::A,
            tile: id,
        })
    } else if id >= B_FIRST && ((id - B_FIRST) as usize) < TILES_PER_FILE {
        Some(TileRef {
            part: Part::B,
            tile: id - B_FIRST,
        })
    } else {
        None
    }
}

/// One texture set: the tmapa and tmapb files of the same index, palette indices.
#[derive(Clone, Debug)]
pub struct TextureSet {
    pub index: u32,
    a: Vec<u8>,
    b: Vec<u8>,
    /// False when no tmapb file was found (tiles are then `MISSING_FILL`).
    pub has_b: bool,
}

impl TextureSet {
    /// Build from raw file bytes. `a` is required; `b` optional (as in the game: a missing tmapb
    /// is not an error). Files shorter than 1024 bytes are rejected
    /// (`engine_textures.c:load_letter_one_file`); shorter-than-full files leave the rest at 130.
    pub fn from_bytes(index: u32, a: &[u8], b: Option<&[u8]>) -> Result<Self> {
        let fit = |src: &[u8]| -> Result<Vec<u8>> {
            if src.len() < 1024 {
                return Err(Error::format(
                    "tmap",
                    format!("file too small ({} bytes)", src.len()),
                ));
            }
            let mut v = vec![MISSING_FILL; FILE_BYTES];
            let n = src.len().min(FILE_BYTES);
            v[..n].copy_from_slice(&src[..n]);
            Ok(v)
        };
        let a = fit(a)?;
        let (b, has_b) = match b {
            Some(b) => (fit(b)?, true),
            None => (vec![MISSING_FILL; FILE_BYTES], false),
        };
        Ok(TextureSet { index, a, b, has_b })
    }

    /// Load `data/tmapaNNN.dat` + `data/tmapbNNN.dat` from the install's `data` dir only
    /// (no campaign/map overrides; use [`TextureSearch`] for those).
    pub fn load_std(install: &Install, index: u32) -> Result<Self> {
        let dirs = TextureSearch::new(install.data_dir());
        Self::load(&dirs, index, None)
    }

    /// Load a set following the game's search order ([`TextureSearch::candidates`]).
    pub fn load(search: &TextureSearch, index: u32, lvnum: Option<u32>) -> Result<Self> {
        let pa = search
            .find('a', index, lvnum)
            .ok_or_else(|| Error::format("tmap", format!("tmapa{index:03}.dat not found")))?;
        let a = crate::install::load_file(&pa)?;
        let b = match search.find('b', index, lvnum) {
            Some(pb) => Some(crate::install::load_file(&pb)?),
            None => None,
        };
        Self::from_bytes(index, &a, b.as_deref())
    }

    fn part(&self, p: Part) -> &[u8] {
        match p {
            Part::A => &self.a,
            Part::B => &self.b,
        }
    }

    /// Raw palette indices of a whole file as a 256 x 2176 image.
    pub fn atlas_indexed(&self, p: Part) -> &[u8] {
        self.part(p)
    }

    /// Palette indices of one 32x32 tile (row-major, 1024 bytes).
    pub fn tile_indexed(&self, t: TileRef) -> Option<Vec<u8>> {
        let t_ = t.tile as usize;
        if t_ >= TILES_PER_FILE {
            return None;
        }
        let (col, row) = (t_ % TILES_PER_ROW, t_ / TILES_PER_ROW);
        let src = self.part(t.part);
        let mut out = Vec::with_capacity(TILE_DIM * TILE_DIM);
        for y in 0..TILE_DIM {
            let o = (row * TILE_DIM + y) * ATLAS_W + col * TILE_DIM;
            out.extend_from_slice(&src[o..o + TILE_DIM]);
        }
        Some(out)
    }

    /// One tile as opaque RGBA (32x32).
    pub fn tile_rgba(&self, t: TileRef, pal: &Palette) -> Option<Rgba> {
        let idx = self.tile_indexed(t)?;
        let mut out = Rgba::new(TILE_DIM as u32, TILE_DIM as u32);
        for (i, &p) in idx.iter().enumerate() {
            out.data[i * 4..i * 4 + 4].copy_from_slice(&pal.rgba(p));
        }
        Some(out)
    }

    /// The whole file as one opaque RGBA atlas (256 x 2176), tile layout unchanged.
    pub fn atlas_rgba(&self, p: Part, pal: &Palette) -> Rgba {
        let mut out = Rgba::new(ATLAS_W as u32, ATLAS_H as u32);
        for (i, &v) in self.part(p).iter().enumerate() {
            out.data[i * 4..i * 4 + 4].copy_from_slice(&pal.rgba(v));
        }
        out
    }

    /// Texture id + animation frame -> RGBA tile. Static ids ignore `frame`.
    pub fn id_rgba(
        &self,
        id: u16,
        frame: usize,
        anim: &TextureAnim,
        pal: &Palette,
    ) -> Option<Rgba> {
        self.tile_rgba(anim.resolve(id, frame)?, pal)
    }
}

/// `fxdata/textureanim.toml`: for each animated slot the 8 frame texture ids.
#[derive(Clone, Debug)]
pub struct TextureAnim {
    /// `frames[id - 544]`. Unlisted slots stay all-zero like the game's zeroed array (tile 0).
    frames: Vec<[u16; ANIM_FRAMES]>,
    pub names: Vec<Option<String>>,
}

impl Default for TextureAnim {
    fn default() -> Self {
        TextureAnim {
            frames: vec![[0; ANIM_FRAMES]; ANIM_SLOTS],
            names: vec![None; ANIM_SLOTS],
        }
    }
}

impl TextureAnim {
    /// Parse `[textureN] name=..., frames=[8 ids]` (`src/config_textures.c:46-75`); `N` is the full
    /// texture id (544..=999). Unknown keys are ignored.
    pub fn parse(text: &str) -> Result<Self> {
        let doc: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| Error::format("textureanim", e.to_string()))?;
        let mut out = TextureAnim::default();
        for (key, val) in &doc {
            let Some(n) = key
                .strip_prefix("texture")
                .and_then(|s| s.parse::<usize>().ok())
            else {
                continue;
            };
            if !(ANIM_FIRST as usize..B_FIRST as usize).contains(&n) {
                return Err(Error::format(
                    "textureanim",
                    format!("[{key}] outside 544..=999"),
                ));
            }
            let slot = n - ANIM_FIRST as usize;
            if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
                out.names[slot] = Some(name.to_string());
            }
            if let Some(arr) = val.get("frames").and_then(|v| v.as_array()) {
                for (f, v) in arr.iter().take(ANIM_FRAMES).enumerate() {
                    let id = v.as_integer().ok_or_else(|| {
                        Error::format("textureanim", format!("[{key}] frame {f} not an integer"))
                    })?;
                    out.frames[slot][f] = u16::try_from(id).map_err(|_| {
                        Error::format("textureanim", format!("[{key}] frame id {id} out of range"))
                    })?;
                }
            }
        }
        Ok(out)
    }

    pub fn load(install: &Install) -> Result<Self> {
        let b = install.load("fxdata/textureanim.toml")?;
        Self::parse(&String::from_utf8_lossy(&b))
    }

    /// The 8 raw frame ids of animated slot `id`.
    pub fn frames(&self, id: u16) -> Option<&[u16; ANIM_FRAMES]> {
        (ANIM_FIRST..B_FIRST)
            .contains(&id)
            .then(|| &self.frames[(id - ANIM_FIRST) as usize])
    }

    /// Resolve a texture id and animation frame (`gameturn / 2 % 8`, `src/engine_textures.c:82-104`)
    /// to a concrete tile. Static ids resolve to themselves. A frame id that is neither a tmapa
    /// nor a tmapb tile gives `None` (the game leaves the slot unchanged).
    pub fn resolve(&self, id: u16, frame: usize) -> Option<TileRef> {
        match self.frames(id) {
            Some(f) => static_tile(f[frame % ANIM_FRAMES]),
            None => static_tile(id),
        }
    }
}

/// A mod's directories, for the texture search (each may be absent).
#[derive(Clone, Debug, Default)]
pub struct ModDirs {
    /// `<mod>/campgns/<campaign>/levels` style dir holding `mapNNNNN.tmapX000.dat`.
    pub levels: Option<PathBuf>,
    /// Campaign config dir holding `tmapX000.dat`.
    pub config: Option<PathBuf>,
    /// The mod's `data` dir holding `tmapX000.dat`.
    pub data: Option<PathBuf>,
}

/// Candidate directories for texture lookup, mirroring `src/engine_textures.c:155-193`.
/// Mod lists are in load order; the search walks them in reverse (last loaded wins).
#[derive(Clone, Debug, Default)]
pub struct TextureSearch {
    pub mods_after_map: Vec<ModDirs>,
    /// Directory of the current level's files (`mapNNNNN.*`), campaign `levels` dir.
    pub level_dir: Option<PathBuf>,
    pub mods_after_campaign: Vec<ModDirs>,
    /// Campaign config dir (`campgns/xxx` or `FGrp_CmpgConfig`).
    pub campaign_config: Option<PathBuf>,
    pub mods_after_base: Vec<ModDirs>,
    /// Standard `data` dir (always last; returned even if the file does not exist).
    pub data: PathBuf,
}

impl TextureSearch {
    pub fn new(data: impl Into<PathBuf>) -> Self {
        TextureSearch {
            data: data.into(),
            ..Default::default()
        }
    }

    fn mod_candidates(
        m: &ModDirs,
        letter: char,
        set: u32,
        lvnum: Option<u32>,
        out: &mut Vec<(PathBuf, String)>,
    ) {
        if let (Some(d), Some(l)) = (&m.levels, lvnum) {
            out.push((d.clone(), format!("map{l:05}.tmap{letter}{set:03}.dat")));
        }
        if let Some(d) = &m.config {
            out.push((d.clone(), format!("tmap{letter}{set:03}.dat")));
        }
        if let Some(d) = &m.data {
            out.push((d.clone(), format!("tmap{letter}{set:03}.dat")));
        }
    }

    /// Ordered (dir, file name) candidates for `tmap{letter}{set:03}.dat` of level `lvnum`:
    /// mods-after-map (reverse, per mod: levels, config, data); level dir `mapNNNNN.tmapX000.dat`;
    /// mods-after-campaign (reverse); campaign config `tmapX000.dat`; mods-after-base (reverse);
    /// standard data dir. (`prepare_letter_one_file_path`, src/engine_textures.c:155-193; per-mod order
    /// from `prepare_letter_one_file_path_for_mod`, :112-135.)
    pub fn candidates(&self, letter: char, set: u32, lvnum: Option<u32>) -> Vec<(PathBuf, String)> {
        let mut out = Vec::new();
        for m in self.mods_after_map.iter().rev() {
            Self::mod_candidates(m, letter, set, lvnum, &mut out);
        }
        if let (Some(d), Some(l)) = (&self.level_dir, lvnum) {
            out.push((d.clone(), format!("map{l:05}.tmap{letter}{set:03}.dat")));
        }
        for m in self.mods_after_campaign.iter().rev() {
            Self::mod_candidates(m, letter, set, lvnum, &mut out);
        }
        if let Some(d) = &self.campaign_config {
            out.push((d.clone(), format!("tmap{letter}{set:03}.dat")));
        }
        for m in self.mods_after_base.iter().rev() {
            Self::mod_candidates(m, letter, set, lvnum, &mut out);
        }
        out.push((self.data.clone(), format!("tmap{letter}{set:03}.dat")));
        out
    }

    /// First candidate that exists (case-insensitive file name match).
    pub fn find(&self, letter: char, set: u32, lvnum: Option<u32>) -> Option<PathBuf> {
        self.candidates(letter, set, lvnum)
            .into_iter()
            .find_map(|(d, n)| find_in_dir(&d, &n))
    }
}

/// Helper for callers that only want to know whether a path is inside a dir-based set.
pub fn set_files_exist(data_dir: &Path, index: u32) -> (bool, bool) {
    (
        find_in_dir(data_dir, &format!("tmapa{index:03}.dat")).is_some(),
        find_in_dir(data_dir, &format!("tmapb{index:03}.dat")).is_some(),
    )
}
