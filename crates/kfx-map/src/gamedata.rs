//! The layered game configuration the slab logic needs.
//!
//! # Layering (`load_config`, `src/config.c:2318-2360`)
//! For every config file name the game loads, in order:
//! 1. `fxdata/<file>` (the base; "standard" load, it resets the tables first),
//! 2. mods "after base" (not modelled yet, see [`ConfigLayers::insert`]),
//! 3. the campaign `CONFIGS_LOCATION/<file>` (partial load: only the fields present change),
//! 4. mods "after campaign",
//! 5. `LEVELS_LOCATION/map%05d.<file>` (partial),
//! 6. mods "after map".
//!
//! A missing file, or one smaller than `MIN_CONFIG_FILE_SIZE` (4 bytes, src/config.h:37), is skipped.
//! [`ConfigLayers`] is that ordered list; every loader here walks it in order and applies each file
//! on top of the previous result, field by field, the way the C named-field parser does.
//!
//! # What is loaded
//! * `terrain.cfg` `[slabN]` / `[roomN]` (`src/config_terrain.c:112-200`): typed where the slab
//!   logic needs it, every field also kept raw.
//! * `cubes.cfg` (`src/config_cubes.c:55-70`), including the ownership-group table `cube_bits`
//!   filled by `assign_owner` (src/config_cubes.c:94-104).
//! * `slabset.toml` / `columnset.toml` (`src/config_slabsets.c:85-233`), then the level-load
//!   resolution of slabset column ids into actual columns (`load_slab_datclm_files`,
//!   `src/lvl_filesdk1.c:917-1031`).
//! * `spritecolors.toml` `objects` (`src/config_spritecolors.c:57-103`): player coloured object
//!   models used when slab objects are created.
//! * Generic (section -> key/values) views of `objects.cfg`, `trapdoor.cfg`, `creature.cfg`,
//!   `effects.toml`, and which `creatrs/<name>.cfg` exist.
use crate::column::{canonical_column, column_equivalent};
use crate::creature::CreatureConfig;
use crate::layered::LayeredCfg;
use crate::mods::{LayerOrigin, LayerStack};
use crate::{Error, Result};
use kfx_formats::cfg::{CfgDoc, Entry};
use kfx_formats::level_bin::Column;
use kfx_formats::rnc;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// `TERRAIN_ITEMS_MAX` (src/config_terrain.h:34): max slab kinds and room kinds.
pub const TERRAIN_ITEMS_MAX: usize = 256;
/// `SLABSETS_PER_SLAB` (src/config_terrain.h:35): 9 styles x 3 fill styles + centre.
pub const SLABSETS_PER_SLAB: usize = 28;
/// `COLUMNS_COUNT` (src/map_columns.h:29): size of the game's column pool, index 0 included.
pub const COLUMNS_COUNT: usize = 16384;
/// `CUBE_ITEMS_MAX` (src/config_cubes.h:31).
pub const CUBE_ITEMS_MAX: usize = 1024;
/// `CUBE_OWNERSHIP_GROUPS` (src/config_cubes.h:33).
pub const CUBE_OWNERSHIP_GROUPS: usize = 20;
/// `COLOURS_COUNT` (src/player_data.h:35).
pub const COLOURS_COUNT: usize = 9;
/// `SLABOBJS_COUNT` (src/slab_data.h:32): max slab objects over all slabset loads.
pub const SLABOBJS_COUNT: usize = 1024;
/// `PLAYER_COLORS_COUNT` (src/config_spritecolors.c:48) = `COLOURS_COUNT + 2`.
pub const PLAYER_COLORS_COUNT: usize = COLOURS_COUNT + 2;
/// `MAX_COLORED_SPRITES` (src/config_spritecolors.c:47).
pub const MAX_COLORED_SPRITES: usize = 255;
/// `MIN_CONFIG_FILE_SIZE` (src/config.h:37).
const MIN_CONFIG_FILE_SIZE: usize = 4;

/// Style names of `slab_styles_commands` (src/config_slabsets.c:57-85), index = style number.
pub const SLAB_STYLE_NAMES: [&str; SLABSETS_PER_SLAB] = [
    "S",
    "W",
    "N",
    "E",
    "SW",
    "NW",
    "NE",
    "SE",
    "ALL",
    "S_LAVA",
    "W_LAVA",
    "N_LAVA",
    "E_LAVA",
    "SW_LAVA",
    "NW_LAVA",
    "NE_LAVA",
    "SE_LAVA",
    "ALL_LAVA",
    "S_WATER",
    "W_WATER",
    "N_WATER",
    "E_WATER",
    "SW_WATER",
    "NW_WATER",
    "NE_WATER",
    "SE_WATER",
    "ALL_WATER",
    "CENTER",
];

/// Slab attribute flags (`SlabAttrFlags`, src/config_terrain.h:50).
pub mod slab_flags {
    pub const VALUABLE: u32 = 0x01;
    pub const IS_ROOM: u32 = 0x02;
    pub const UNEXPLORED: u32 = 0x04;
    pub const DIGGABLE: u32 = 0x08;
    pub const BLOCKING: u32 = 0x10;
    pub const FILLED: u32 = 0x20;
    pub const IS_DOOR: u32 = 0x40;
    pub const TAGGED_VALUABLE: u32 = 0x80;
    /// `terrain_flags` (src/config_terrain.c:60).
    pub const NAMES: [(&str, u32); 8] = [
        ("VALUABLE", VALUABLE),
        ("IS_ROOM", IS_ROOM),
        ("UNEXPLORED", UNEXPLORED),
        ("DIGGABLE", DIGGABLE),
        ("BLOCKING", BLOCKING),
        ("FILLED", FILLED),
        ("IS_DOOR", IS_DOOR),
        ("TAGGED_VALUABLE", TAGGED_VALUABLE),
    ];
}

/// Slab categories (`SlabAttrCategory`, src/config_terrain.h:41).
pub mod category {
    pub const UNCLAIMED: u8 = 0;
    pub const FRIABLE_DIRT: u8 = 1;
    pub const FORTIFIED_GROUND: u8 = 2;
    pub const FORTIFIED_WALL: u8 = 3;
    pub const ROOM_INTERIOR: u8 = 4;
    pub const OBSTACLE: u8 = 5;
}

/// Fill styles (`SlabFillStyle`, src/config_terrain.h:62).
pub mod fill_style {
    pub const NORMAL: u8 = 0;
    pub const LAVA: u8 = 1;
    pub const WATER: u8 = 2;
}

/// Cube property flags (`CPF_*`, src/config_cubes.h:36).
pub mod cube_flags {
    pub const LAVA: u8 = 0x01;
    pub const WATER: u8 = 0x02;
    pub const SACRIFICIAL: u8 = 0x04;
    pub const UNCLAIMED_PATH: u8 = 0x08;
    pub const ABYSS: u8 = 0x10;
    /// `cubes_properties_flags` (src/config_cubes.c:47).
    pub const NAMES: [(&str, u32); 5] = [
        ("LAVA", 0x01),
        ("WATER", 0x02),
        ("SACRIFICIAL", 0x04),
        ("UNCLAIMED_PATH", 0x08),
        ("ABYSS", 0x10),
    ];
}

/// Player colour names (`cmpgn_human_player_options`, src/config_campaigns.c:139).
pub const PLAYER_NAMES: [&str; 9] = [
    "RED", "BLUE", "GREEN", "YELLOW", "WHITE", "NEUTRAL", "PURPLE", "BLACK", "ORANGE",
];

// ---------------------------------------------------------------------------------------------
// Layers

/// One configuration layer: the file `<dir>/<prefix><name>` for config file `name`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConfigLayer {
    pub dir: PathBuf,
    /// `""` for fxdata and campaign config folders, `"map00001."` for per-map overrides
    /// (`snprintf(map_fname, ..., "map%05d.%s", ...)`, src/config.c:2344).
    pub prefix: String,
}

impl ConfigLayer {
    pub fn dir(dir: impl Into<PathBuf>) -> Self {
        ConfigLayer {
            dir: dir.into(),
            prefix: String::new(),
        }
    }
    pub fn map(dir: impl Into<PathBuf>, level: u32) -> Self {
        ConfigLayer {
            dir: dir.into(),
            prefix: format!("map{level:05}."),
        }
    }
    /// The existing file for config `name` in this layer (case-insensitive file name).
    pub fn file(&self, name: &str) -> Option<PathBuf> {
        find_ci(&self.dir, &format!("{}{}", self.prefix, name))
    }
}

/// The ordered list of configuration layers. The first one is the base (`fxdata`), loaded
/// "standard"; all later ones are partial overrides.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ConfigLayers {
    pub layers: Vec<ConfigLayer>,
    /// Folders searched for `<creature>.cfg`, base first (`creatrs/`, then the campaign
    /// `CREATURES_LOCATION`).
    pub creature_dirs: Vec<PathBuf>,
}

impl ConfigLayers {
    /// Only the base folder (normally `<install>/fxdata`).
    pub fn base(fxdata: impl Into<PathBuf>) -> Self {
        ConfigLayers {
            layers: vec![ConfigLayer::dir(fxdata)],
            creature_dirs: vec![],
        }
    }

    /// The layers the game uses for a level of a campaign / map pack: `fxdata`, then the
    /// campaign `CONFIGS_LOCATION` (when set), then `LEVELS_LOCATION/map%05d.*` (when a
    /// level is given). Folder names are resolved case-insensitively.
    pub fn for_level(
        install: &Path,
        configs_dir: Option<&Path>,
        levels_dir: Option<&Path>,
        creatures_dir: Option<&Path>,
        level: Option<u32>,
    ) -> Self {
        let mut l = ConfigLayers::base(
            find_ci(install, "fxdata").unwrap_or_else(|| install.join("fxdata")),
        );
        if let Some(c) = configs_dir {
            l.layers.push(ConfigLayer::dir(c));
        }
        if let (Some(d), Some(n)) = (levels_dir, level) {
            l.layers.push(ConfigLayer::map(d, n));
        }
        if let Some(c) = find_ci(install, "creatrs") {
            l.creature_dirs.push(c);
        }
        if let Some(c) = creatures_dir {
            l.creature_dirs.push(c.to_path_buf());
        }
        l
    }

    /// Insert a layer (for example a mod folder) at `pos` (0 = before the base).
    pub fn insert(&mut self, pos: usize, layer: ConfigLayer) {
        self.layers.insert(pos.min(self.layers.len()), layer);
    }

    /// Files for config `name`, in load order, each flagged `true` when it is the base layer.
    pub fn files(&self, name: &str) -> Vec<(PathBuf, bool)> {
        self.layers
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.file(name).map(|p| (p, i == 0)))
            .collect()
    }
}

/// Case-insensitive `dir/name` lookup.
pub fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

/// Read a config file like the game: through RNC, `None` when shorter than 4 bytes.
fn read_cfg_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    let b = rnc::load_file(path)?;
    Ok((b.len() >= MIN_CONFIG_FILE_SIZE).then_some(b))
}

fn read_toml(path: &Path) -> Result<Option<toml::Table>> {
    let Some(b) = read_cfg_bytes(path)? else {
        return Ok(None);
    };
    let text = String::from_utf8_lossy(&b);
    text.parse::<toml::Table>()
        .map(Some)
        .map_err(|e| Error::Toml {
            path: path.to_path_buf(),
            msg: e.to_string(),
        })
}

// ---------------------------------------------------------------------------------------------
// Value parsing like the C named-field parser (src/config.c:81-113, 437-560)

/// `parameter_is_number` + `atoll` (src/config.c:81).
pub fn parse_number(s: &str) -> Option<i64> {
    let t = s.trim_matches(' ');
    let b = t.as_bytes();
    if b.is_empty()
        || !(b[0] == b'-' || b[0].is_ascii_digit())
        || !b[1..].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    // atoll: overflow is undefined in C; saturate.
    Some(
        t.parse::<i64>()
            .unwrap_or(if b[0] == b'-' { i64::MIN } else { i64::MAX }),
    )
}

/// `value_default` (src/config.c:437): number clamped to `[min, max]`, else a name from `names`
/// (case-insensitive, `get_id`), else 0.
fn value_default(text: &str, min: i64, max: i64, names: &dyn Fn(&str) -> Option<i64>) -> i64 {
    match parse_number(text) {
        Some(v) => v.clamp(min, max),
        None => names(text).unwrap_or(0),
    }
}

fn no_names(_: &str) -> Option<i64> {
    None
}

/// `value_flagsfield` (src/config.c:522): number, `none`, or blank separated flag names; a second
/// token `0`/`1` toggles the first flag in the previous value.
fn value_flags(text: &str, table: &[(&str, u32)], previous: i64) -> i64 {
    if let Some(n) = parse_number(text) {
        return n;
    }
    if text.eq_ignore_ascii_case("none") {
        return 0;
    }
    let mut value: i64 = 0;
    for (i, w) in kfx_formats::cfg::split_tokens(text).iter().enumerate() {
        if i == 1 && (w == "0" || w == "1") {
            return if w == "1" {
                previous | value
            } else {
                previous & !value
            };
        }
        if let Some(&(_, f)) = table.iter().find(|(n, _)| n.eq_ignore_ascii_case(w)) {
            value |= f as i64;
        }
    }
    value
}

/// Numbered blocks `[<base>N]` of a document (`parse_named_field_blocks`, src/config.c:1054): the
/// base name is matched case-sensitively, N must be < `max`. Each distinct block name is listed
/// once with the entries of the first block of that name (`find_conf_block` semantics).
fn numbered_blocks(doc: &CfgDoc, base: &str, max: usize) -> Vec<(usize, Vec<Entry>)> {
    let mut seen: Vec<String> = vec![];
    let mut out = vec![];
    for name in doc.section_names() {
        let Some(rest) = name.strip_prefix(base) else {
            continue;
        };
        if rest.is_empty() || !rest.bytes().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(i) = rest.parse::<usize>() else {
            continue;
        };
        if i >= max || seen.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
            continue;
        }
        seen.push(name.clone());
        out.push((i, doc.entries(&name)));
    }
    out
}

/// Keeps `key = value` pairs of a block, later assignments replacing earlier ones
/// (case-insensitive keys), insertion order preserved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawFields(pub Vec<(String, String)>);

impl RawFields {
    pub fn set(&mut self, key: &str, value: &str) {
        match self.0.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some(e) => e.1 = value.to_string(),
            None => self.0.push((key.to_string(), value.to_string())),
        }
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

// ---------------------------------------------------------------------------------------------
// terrain.cfg

/// One `[slabN]` (`struct SlabConfigStats`, src/config_terrain.h:126; fields src/config_terrain.c:112).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlabConfig {
    pub name: String,
    pub block_flags_height: i16,
    pub block_health_index: i16,
    pub block_flags: u32,
    pub noblck_flags: u32,
    pub fill_style: u8,
    pub category: u8,
    pub slb_id: u8,
    pub wibble: u8,
    pub is_safe_land: u8,
    pub animated: u8,
    pub is_diggable: u8,
    pub wlb_type: u8,
    pub is_ownable: u8,
    pub indestructible: u8,
    pub gold_held: i32,
    /// Every field as written (last layer wins per key).
    pub raw: RawFields,
}

/// One `[roomN]` (`struct RoomConfigStats`, src/config_terrain.h:146; fields src/config_terrain.c:152).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomConfig {
    pub name: String,
    /// `SlabAssign`: the slab kind of the room's floor.
    pub assigned_slab: u8,
    pub raw: RawFields,
}

// ---------------------------------------------------------------------------------------------
// cubes.cfg

/// One `[cubeN]` (`struct CubeConfigStats`, src/config_cubes.h:44).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CubeConfig {
    pub name: String,
    /// Texture ids: north, east, south, west, top, bottom.
    pub textures: [u16; 6],
    pub ownership_group: u8,
    pub owner: u8,
    pub properties: u8,
    pub raw: RawFields,
}

// ---------------------------------------------------------------------------------------------
// slabset objects

/// One slab object definition (`struct SlabObj`, src/slab_data.h:125; loaded
/// src/config_slabsets.c:126-148).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlabObjDef {
    /// `IsLight` (0/1; other values make the game create nothing, src/map_blocks.c:845/869).
    pub is_light: u8,
    /// `Subtile`: which of the 9 subtiles of the slab this belongs to (0..8).
    pub stl_id: u8,
    /// `RelativePosition` in 1/256 subtile, relative to the subtile's corner (x, y) and the
    /// map floor (z).
    pub offset: [i16; 3],
    /// `ThingType` (`TCls_*`); 1 = Object, 7 = EffectGen.
    pub class_id: u8,
    /// `Subtype`; for lights it is the intensity.
    pub model: i16,
    /// `EffectRange` in subtiles: light radius or effect generator range.
    pub range: u8,
    pub orientation: i16,
}

/// A slabset column after level-load resolution (src/lvl_filesdk1.c:968-1003).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlabsetColumn {
    /// The column content the slabset entry points at.
    Column(Column),
    /// The game could not find the columnset column in its pool ("referenced in slabset.toml
    /// but not present in columnset.toml", src/lvl_filesdk1.c:996) and left the raw id, so the
    /// slabset points at whatever pool column has that index (usually a column of the
    /// level's own `.clm`).
    Unresolved(u16),
}

// ---------------------------------------------------------------------------------------------
// generic configs

/// A generic, layered view of a `.cfg`: sections with their key/values. Later layers replace
/// keys (case-insensitive) inside the same section and append new keys / sections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GenericCfg {
    pub sections: Vec<(String, RawFields)>,
}

impl GenericCfg {
    fn apply(&mut self, doc: &CfgDoc) {
        let mut seen: Vec<String> = vec![];
        for name in doc.section_names() {
            if seen.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
                continue;
            }
            seen.push(name.clone());
            let ents = doc.entries(&name);
            let idx = match self
                .sections
                .iter()
                .position(|(n, _)| n.eq_ignore_ascii_case(&name))
            {
                Some(i) => i,
                None => {
                    self.sections.push((name.clone(), RawFields::default()));
                    self.sections.len() - 1
                }
            };
            for e in ents {
                self.sections[idx].1.set(&e.key, &e.value);
            }
        }
    }
    pub fn section(&self, name: &str) -> Option<&RawFields> {
        self.sections
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, f)| f)
    }
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section)?.get(key)
    }
    /// `(N, Name)` of every `[<base>N]` section, sorted by N.
    pub fn numbered_names(&self, base: &str) -> Vec<(usize, String)> {
        let mut v: Vec<(usize, String)> = self
            .sections
            .iter()
            .filter_map(|(n, f)| {
                let rest = n.strip_prefix(base)?;
                if rest.is_empty() || !rest.bytes().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                let name = f
                    .get("Name")
                    .and_then(|v| v.split_whitespace().next())
                    .unwrap_or("")
                    .to_string();
                Some((rest.parse().ok()?, name))
            })
            .collect();
        v.sort();
        v
    }
}

/// Recursively merge `src` into `dst` (tables merge, other values replace).
fn merge_toml(dst: &mut toml::Table, src: toml::Table) {
    for (k, v) in src {
        match (dst.get_mut(&k), v) {
            (Some(toml::Value::Table(d)), toml::Value::Table(s)) => merge_toml(d, s),
            (_, v) => {
                dst.insert(k, v);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// GameData

/// Everything loaded from the configuration layers.
#[derive(Clone, Debug, Default)]
pub struct GameData {
    /// `[slabN]`, index = slab kind; length = `slab_types_count`.
    pub slabs: Vec<SlabConfig>,
    /// `[roomN]`, index = room kind.
    pub rooms: Vec<RoomConfig>,
    /// `[cubeN]`, index = cube id; length = `cube_types_count`.
    pub cubes: Vec<CubeConfig>,
    /// `cube_bits[group][colour]`: the cube that replaces an ownership-group cube for a player
    /// colour (src/config_cubes.h:55, filled by `assign_owner`).
    pub cube_bits: [[u16; COLOURS_COUNT]; CUBE_OWNERSHIP_GROUPS],
    /// `columnset.toml` columns as loaded (`game.conf.column_conf.cols`), index = column id.
    pub columnset: Vec<Column>,
    /// `slabset.toml` column ids as the loader stores them (negated, `ColumnIndex`),
    /// index = `kind * 28 + style`. Length `slab_types_count * 28`.
    pub slabset_raw: Vec<[i16; 9]>,
    /// Slabset columns after the level-load resolution.
    pub slabset: Vec<[SlabsetColumn; 9]>,
    /// Slab objects per slabset index (`game.slabobjs`, grouped by `slabset_id`).
    pub slab_objects: Vec<Vec<SlabObjDef>>,
    /// `spritecolors.toml` `objects` rows (`objects_eq`): `[base, colour0+1 ...]`.
    pub coloured_objects: Vec<[u16; PLAYER_COLORS_COUNT]>,
    /// `creature.cfg` `[common] Creatures`, index + 1 = creature model.
    pub creatures: Vec<String>,
    /// Lower-case names of creatures with a `<name>.cfg` in a creature folder.
    pub creature_cfgs: Vec<String>,
    pub creature_cfg: GenericCfg,
    pub objects_cfg: GenericCfg,
    pub trapdoor_cfg: GenericCfg,
    pub effects: toml::Table,
    /// Every file that contributed, in load order.
    pub sources: Vec<PathBuf>,
    /// Non-fatal problems (unresolved slabset columns, invalid column values, ...).
    pub warnings: Vec<String>,
    /// The layers the data was loaded from, in load order, with what each one is (base,
    /// campaign, map, mod): see [`crate::mods`].
    pub config_stack: Vec<(LayerOrigin, ConfigLayer)>,
    /// Layers of the per-creature configs (`<name>.cfg`), in load order.
    pub creature_stack: Vec<(LayerOrigin, ConfigLayer)>,
    creature_cache: CreatureCache,
}

/// Lazily loaded [`CreatureConfig`]s of a [`GameData`] (interior mutability so `&GameData` can
/// fill it; clones copy the cached entries).
#[derive(Default)]
struct CreatureCache(Mutex<HashMap<String, Option<Arc<CreatureConfig>>>>);

impl Clone for CreatureCache {
    fn clone(&self) -> Self {
        CreatureCache(Mutex::new(
            self.0.lock().map(|m| m.clone()).unwrap_or_default(),
        ))
    }
}

impl std::fmt::Debug for CreatureCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CreatureCache({} entries)",
            self.0.lock().map_or(0, |m| m.len())
        )
    }
}

impl GameData {
    /// Load everything from `layers`.
    pub fn load(layers: &ConfigLayers) -> Result<GameData> {
        let mut gd = GameData::default();
        let stack = crate::mods::stack_of_layers(layers);
        gd.config_stack = stack.config;
        gd.creature_stack = stack.creatures;
        gd.load_terrain(layers)?;
        gd.load_cubes(layers)?;
        gd.load_spritecolors(layers)?;
        gd.load_slabset(layers)?;
        gd.load_columnset(layers)?;
        gd.resolve_slabset();
        gd.load_generic(layers)?;
        Ok(gd)
    }

    /// [`GameData::load`] through a [`LayerStack`], mods included.
    pub fn load_stack(stack: &LayerStack) -> Result<GameData> {
        let mut gd = GameData::load(&stack.to_config_layers())?;
        gd.config_stack = stack.config.clone();
        gd.creature_stack = stack.creatures.clone();
        Ok(gd)
    }

    // -------------------------------------------------------------- creature configs

    /// The typed config of creature `name` (`imp`, case-insensitive), loaded through the
    /// creature layers on first use and cached. `None` when no layer has `<name>.cfg` or it does
    /// not load ([`GameData::try_creature_config`] has the error).
    pub fn creature_config(&self, name: &str) -> Option<Arc<CreatureConfig>> {
        self.try_creature_config(name).ok()
    }

    pub fn try_creature_config(&self, name: &str) -> Result<Arc<CreatureConfig>> {
        let key = name.to_ascii_lowercase();
        if let Some(Some(c)) = self
            .creature_cache
            .0
            .lock()
            .ok()
            .and_then(|m| m.get(&key).cloned())
        {
            return Ok(c);
        }
        let c = Arc::new(CreatureConfig::load(&key, &self.creature_stack)?);
        if let Ok(mut m) = self.creature_cache.0.lock() {
            m.insert(key, Some(c.clone()));
        }
        Ok(c)
    }

    /// [`GameData::creature_config`] of creature model `model` (1-based, see `creatures`).
    pub fn creature_config_of_model(&self, model: usize) -> Option<Arc<CreatureConfig>> {
        self.creature_config(self.creatures.get(model.checked_sub(1)?)?)
    }

    /// Forget the cached creature configs (after a config file changed on disk).
    pub fn clear_creature_cache(&self) {
        if let Ok(mut m) = self.creature_cache.0.lock() {
            m.clear();
        }
    }

    /// Config file `name` (`rules.cfg`, ...) loaded through the layers with provenance.
    pub fn layered_cfg(&self, name: &str) -> Result<LayeredCfg> {
        let files = self
            .config_stack
            .iter()
            .filter_map(|(o, l)| l.file(name).map(|p| (o.clone(), p)))
            .collect();
        LayeredCfg::from_cfg_files(files)
    }

    /// TOML config file `name` (`effects.toml`, ...) through the layers with provenance.
    pub fn layered_toml(&self, name: &str) -> Result<LayeredCfg> {
        let files = self
            .config_stack
            .iter()
            .filter_map(|(o, l)| l.file(name).map(|p| (o.clone(), p)))
            .collect();
        LayeredCfg::from_toml_files(files)
    }

    // -------------------------------------------------------------- lookups

    /// `get_slab_kind_stats` (src/config_terrain.c:357): kinds beyond the count read slab 0.
    pub fn slab(&self, kind: usize) -> &SlabConfig {
        static EMPTY: std::sync::OnceLock<SlabConfig> = std::sync::OnceLock::new();
        self.slabs
            .get(kind)
            .or_else(|| self.slabs.first())
            .unwrap_or_else(|| EMPTY.get_or_init(SlabConfig::default))
    }
    pub fn slab_count(&self) -> usize {
        self.slabs.len()
    }
    pub fn slab_id(&self, name: &str) -> Option<usize> {
        self.slabs
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
    }
    pub fn slab_name(&self, kind: usize) -> Option<&str> {
        self.slabs.get(kind).map(|s| s.name.as_str())
    }
    pub fn room_id(&self, name: &str) -> Option<usize> {
        self.rooms
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
    }
    pub fn room_name(&self, kind: usize) -> Option<&str> {
        self.rooms.get(kind).map(|s| s.name.as_str())
    }
    /// `slab_corresponding_room` (src/config_terrain.c:875): first room whose `SlabAssign` is the
    /// kind, 0 when none.
    pub fn slab_room(&self, kind: usize) -> usize {
        self.rooms
            .iter()
            .position(|r| r.assigned_slab as usize == kind)
            .unwrap_or(0)
    }
    pub fn cube(&self, id: usize) -> Option<&CubeConfig> {
        self.cubes.get(id)
    }
    pub fn cube_id(&self, name: &str) -> Option<usize> {
        self.cubes
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
    }
    /// Object models: `(id, name)` from `objects.cfg` `[objectN]`.
    pub fn object_names(&self) -> Vec<(usize, String)> {
        self.objects_cfg.numbered_names("object")
    }
    pub fn object_id(&self, name: &str) -> Option<usize> {
        self.object_names()
            .into_iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(name))
            .map(|(i, _)| i)
    }
    /// `Genre` of an object model (raw, e.g. `DECORATION`, `HEROGATE`).
    pub fn object_genre(&self, model: usize) -> Option<&str> {
        self.objects_cfg.get(&format!("object{model}"), "Genre")
    }
    pub fn trap_names(&self) -> Vec<(usize, String)> {
        self.trapdoor_cfg.numbered_names("trap")
    }
    pub fn door_names(&self) -> Vec<(usize, String)> {
        self.trapdoor_cfg.numbered_names("door")
    }
    /// `(id, Name)` of `effects.toml` `[effectGeneratorN]`.
    pub fn effect_generator_names(&self) -> Vec<(usize, String)> {
        let mut v: Vec<(usize, String)> = self
            .effects
            .iter()
            .filter_map(|(k, v)| {
                let n: usize = k.strip_prefix("effectGenerator")?.parse().ok()?;
                let name = v
                    .get("Name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                Some((n, name))
            })
            .collect();
        v.sort();
        v
    }
    /// Creature model (1-based, 0 = none) of a creature name.
    pub fn creature_id(&self, name: &str) -> Option<usize> {
        self.creatures
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .map(|i| i + 1)
    }
    /// Index into [`GameData::slabset`]: `kind * 28 + style`.
    pub fn slabset_index(kind: usize, style: usize) -> usize {
        kind * SLABSETS_PER_SLAB + style
    }

    // -------------------------------------------------------------- terrain

    /// `load_terrain_config_file` (src/config_terrain.c:483) run as the game does it: first a
    /// `CnfLd_ListOnly` pass over all layers for the names and counts (src/dungeon_stats.c:59), then
    /// the full pass with `CnfLd_PreListed` (src/dungeon_stats.c:73) so `SlabAssign` sees every name.
    fn load_terrain(&mut self, layers: &ConfigLayers) -> Result<()> {
        let mut docs = vec![];
        for (p, _) in layers.files("terrain.cfg") {
            if let Some(b) = read_cfg_bytes(&p)? {
                docs.push(CfgDoc::from_bytes(&b));
                self.sources.push(p);
            }
        }
        let mut slabs: Vec<SlabConfig> = vec![];
        let mut rooms: Vec<RoomConfig> = vec![];
        // List pass: names and counts.
        for doc in &docs {
            for (i, ents) in numbered_blocks(doc, "slab", TERRAIN_ITEMS_MAX) {
                if i >= slabs.len() {
                    slabs.resize(i + 1, SlabConfig::default());
                }
                for e in ents.iter().filter(|e| e.key.eq_ignore_ascii_case("NAME")) {
                    if let Some(t) = e.tokens.first() {
                        slabs[i].name = t.clone();
                    }
                }
            }
            for (i, ents) in numbered_blocks(doc, "room", TERRAIN_ITEMS_MAX) {
                if i >= rooms.len() {
                    rooms.resize(i + 1, RoomConfig::default());
                }
                for e in ents.iter().filter(|e| e.key.eq_ignore_ascii_case("NAME")) {
                    if let Some(t) = e.tokens.first() {
                        rooms[i].name = t.clone();
                    }
                }
            }
        }
        let slab_names: Vec<String> = slabs.iter().map(|s| s.name.clone()).collect();
        let slab_lookup = |t: &str| {
            slab_names
                .iter()
                .position(|n| !n.is_empty() && n.eq_ignore_ascii_case(t))
                .map(|i| i as i64)
        };
        // Full pass.
        for doc in &docs {
            for (i, ents) in numbered_blocks(doc, "slab", TERRAIN_ITEMS_MAX) {
                let s = &mut slabs[i];
                for e in &ents {
                    apply_slab_field(s, e, &slab_lookup);
                }
            }
            for (i, ents) in numbered_blocks(doc, "room", TERRAIN_ITEMS_MAX) {
                let r = &mut rooms[i];
                for e in &ents {
                    r.raw.set(&e.key, &e.value);
                    let Some(t) = e.tokens.first() else { continue };
                    if e.key.eq_ignore_ascii_case("NAME") {
                        r.name = t.clone();
                    } else if e.key.eq_ignore_ascii_case("SLABASSIGN") {
                        r.assigned_slab = value_default(t, 0, 255, &slab_lookup) as u8;
                    }
                }
            }
        }
        if slabs.is_empty() {
            return Err(Error::Invalid(
                "terrain.cfg: no [slabN] blocks found in any layer".into(),
            ));
        }
        self.slabs = slabs;
        self.rooms = rooms;
        Ok(())
    }

    // -------------------------------------------------------------- cubes

    /// `load_cubes_config_file` (src/config_cubes.c:116). The base load resets the table
    /// (`set_defaults`); `Owner` registers `cube_bits[group][owner] = cube` when assigned, using
    /// the group known at that moment (`assign_owner`, src/config_cubes.c:94).
    fn load_cubes(&mut self, layers: &ConfigLayers) -> Result<()> {
        for (p, base) in layers.files("cubes.cfg") {
            let Some(b) = read_cfg_bytes(&p)? else {
                continue;
            };
            self.sources.push(p);
            let doc = CfgDoc::from_bytes(&b);
            if base {
                self.cubes.clear();
                self.cube_bits = [[0; COLOURS_COUNT]; CUBE_OWNERSHIP_GROUPS];
            }
            for (i, ents) in numbered_blocks(&doc, "cube", CUBE_ITEMS_MAX) {
                if i >= self.cubes.len() {
                    self.cubes.resize(i + 1, CubeConfig::default());
                }
                for e in &ents {
                    let c = &mut self.cubes[i];
                    c.raw.set(&e.key, &e.value);
                    let k = e.key.to_ascii_uppercase();
                    match k.as_str() {
                        "NAME" => {
                            if let Some(t) = e.tokens.first() {
                                c.name = t.clone();
                            }
                        }
                        "TEXTURES" => {
                            for (n, t) in e.tokens.iter().take(6).enumerate() {
                                c.textures[n] =
                                    value_default(t, 0, u16::MAX as i64, &no_names) as u16;
                            }
                        }
                        "OWNERSHIPGROUP" => {
                            if let Some(t) = e.tokens.first() {
                                c.ownership_group =
                                    value_default(t, 0, CUBE_OWNERSHIP_GROUPS as i64, &no_names)
                                        as u8;
                            }
                        }
                        "OWNER" => {
                            if let Some(t) = e.tokens.first() {
                                let v = value_default(t, 0, 9, &|n: &str| {
                                    PLAYER_NAMES
                                        .iter()
                                        .position(|p| p.eq_ignore_ascii_case(n))
                                        .map(|x| x as i64)
                                }) as usize;
                                let g = c.ownership_group as usize;
                                if g > 0 {
                                    if g < CUBE_OWNERSHIP_GROUPS && v < COLOURS_COUNT {
                                        self.cube_bits[g][v] = i as u16;
                                    }
                                    c.owner = v as u8;
                                } else {
                                    self.warnings.push(format!(
                                        "cubes.cfg: Owner without OwnershipGroup for [cube{i}]"
                                    ));
                                }
                            }
                        }
                        "PROPERTIES" => {
                            c.properties =
                                value_flags(&e.value, &cube_flags::NAMES, c.properties as i64)
                                    as u8;
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    // -------------------------------------------------------------- spritecolors

    /// `load_spritecolors_config_file` / `load_array` (src/config_spritecolors.c:57): rows are
    /// written by position; the base load clears the table first.
    fn load_spritecolors(&mut self, layers: &ConfigLayers) -> Result<()> {
        for (p, base) in layers.files("spritecolors.toml") {
            let Some(t) = read_toml(&p)? else { continue };
            self.sources.push(p);
            if base {
                self.coloured_objects.clear();
            }
            let Some(rows) = t.get("objects").and_then(|v| v.as_array()) else {
                continue;
            };
            for (i, row) in rows.iter().enumerate().take(MAX_COLORED_SPRITES) {
                let Some(row) = row.as_array() else { continue };
                if row.len() > PLAYER_COLORS_COUNT {
                    continue;
                }
                if i >= self.coloured_objects.len() {
                    self.coloured_objects
                        .resize(i + 1, [0; PLAYER_COLORS_COUNT]);
                }
                for (j, v) in row.iter().enumerate() {
                    // Strings go through get_anim_id_ (animation names, not object names);
                    // an unknown name stores -2.
                    self.coloured_objects[i][j] = match v.as_integer() {
                        Some(n) => n as u16,
                        None => (-2i16) as u16,
                    };
                }
            }
        }
        Ok(())
    }

    /// `get_coloured_object_base_model` (src/config_spritecolors.c:173).
    pub fn coloured_object_base(&self, model: i16) -> i16 {
        for row in &self.coloured_objects {
            if row[0] == 0 {
                return 0;
            }
            if row.iter().any(|&m| m as i16 == model) {
                return row[0] as i16;
            }
        }
        0
    }

    /// `get_player_colored_object_model` (src/config_spritecolors.c:168) for a player colour index
    /// (`get_player_color_idx`).
    pub fn coloured_object_model(&self, base: i16, color_idx: usize) -> i16 {
        let c = color_idx + 1;
        if c >= PLAYER_COLORS_COUNT {
            return base;
        }
        for row in &self.coloured_objects {
            if row[0] as i16 == base {
                return row[c] as i16;
            } else if row[0] == 0 {
                return base;
            }
        }
        base
    }

    // -------------------------------------------------------------- slabset / columnset

    /// `load_slabset_config_file` (src/config_slabsets.c:87). Only `slab0..slab_types_count-1` are
    /// read. The base load clears every slabset (`clear_slabsets`).
    fn load_slabset(&mut self, layers: &ConfigLayers) -> Result<()> {
        let n = self.slabs.len() * SLABSETS_PER_SLAB;
        self.slabset_raw = vec![[0; 9]; n];
        self.slab_objects = vec![vec![]; n];
        let mut objs_total = 0usize;
        for (p, base) in layers.files("slabset.toml") {
            let Some(root) = read_toml(&p)? else { continue };
            self.sources.push(p.clone());
            if base {
                self.slabset_raw = vec![[0; 9]; n];
                self.slab_objects = vec![vec![]; n];
                objs_total = 0;
            }
            for kind in 0..self.slabs.len() {
                let Some(toml::Value::Table(slb)) = root.get(&format!("slab{kind}")) else {
                    continue;
                };
                for (style, sname) in SLAB_STYLE_NAMES.iter().enumerate() {
                    let id = kind * SLABSETS_PER_SLAB + style;
                    let section = slb.get(*sname);
                    if let Some(toml::Value::Array(cols)) = section.and_then(|s| s.get("Columns")) {
                        for k in 0..9 {
                            let v = cols.get(k).map(value_int32).unwrap_or(0) as i16;
                            self.slabset_raw[id][k] = v.wrapping_neg();
                        }
                    }
                    let objs = slb
                        .get(&format!("{sname}_objects"))
                        .and_then(|v| v.as_array());
                    let nobj = objs.map_or(0, |a| a.len());
                    if section.is_some_and(|s| s.is_table()) && nobj == 0 {
                        self.slab_objects[id].clear();
                    } else if let Some(arr) = objs {
                        let mut list = vec![];
                        for o in arr {
                            if objs_total >= SLABOBJS_COUNT {
                                self.warnings.push(format!(
                                    "{}: exceeding max of {SLABOBJS_COUNT} slabobjects",
                                    p.display()
                                ));
                                break;
                            }
                            list.push(self.parse_slab_obj(o));
                            objs_total += 1;
                        }
                        if !list.is_empty() {
                            self.slab_objects[id] = list;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn parse_slab_obj(&self, o: &toml::Value) -> SlabObjDef {
        let mut d = SlabObjDef::default();
        if let Some(v) = o.get("ThingType") {
            d.class_id = match v {
                toml::Value::Integer(i) => *i as u8,
                toml::Value::String(s) => kfx_formats::level_fx::CLASS_NAMES
                    .iter()
                    .find(|(n, _)| n == s)
                    .map(|&(_, c)| c)
                    .unwrap_or(0),
                _ => d.class_id,
            };
        }
        match o.get("IsLight") {
            Some(toml::Value::Boolean(b)) => d.is_light = *b as u8,
            Some(toml::Value::Integer(i)) => d.is_light = *i as u8,
            _ => {}
        }
        if let Some(v) = o.get("Subtype") {
            d.model = match v {
                toml::Value::Integer(i) => *i as i16,
                // value_parse_model (src/value_util.c:93) by class
                toml::Value::String(s) => match d.class_id {
                    1 | 12 => self.object_id(s).map_or(-1, |x| x as i16),
                    7 => self
                        .effect_generator_names()
                        .into_iter()
                        .find(|(_, n)| n.eq_ignore_ascii_case(s))
                        .map_or(-1, |(i, _)| i as i16),
                    _ => -1,
                },
                _ => d.model,
            };
        }
        if let Some(toml::Value::Array(a)) = o.get("RelativePosition") {
            for (k, slot) in d.offset.iter_mut().enumerate() {
                *slot = a.get(k).map(value_int32).unwrap_or(0) as i16;
            }
        }
        if let Some(toml::Value::Integer(i)) = o.get("EffectRange") {
            d.range = *i as u8;
        }
        if let Some(toml::Value::Integer(i)) = o.get("Subtile") {
            d.stl_id = *i as u8;
        }
        if let Some(toml::Value::Integer(i)) = o.get("Orientation") {
            d.orientation = *i as i16;
        }
        d
    }

    /// `load_columns_config_file` (src/config_slabsets.c:166). Every `[columnN]` gets the
    /// "permanent" bit; `Lintel`/`Height` out of range skip the rest of that column (but the
    /// column count was already raised). Not cleared by the base load.
    fn load_columnset(&mut self, layers: &ConfigLayers) -> Result<()> {
        for (p, _) in layers.files("columnset.toml") {
            let Some(root) = read_toml(&p)? else { continue };
            self.sources.push(p.clone());
            let mut keys: Vec<(usize, &toml::Table)> = root
                .iter()
                .filter_map(|(k, v)| {
                    let n: usize = k.strip_prefix("column")?.parse().ok()?;
                    (n < COLUMNS_COUNT && format!("column{n}") == *k).then_some((n, v.as_table()?))
                })
                .collect();
            keys.sort_by_key(|(n, _)| *n);
            for (n, sec) in keys {
                if n + 1 > self.columnset.len() {
                    self.columnset.resize(n + 1, Column::default());
                }
                let mut bitfields: u8 = 1;
                if let Some(toml::Value::Integer(v)) = sec.get("Lintel") {
                    let l = *v as i32 as i8;
                    if !(0..=7).contains(&l) {
                        self.warnings.push(format!(
                            "{}: invalid Lintel ({l}) for column {n}",
                            p.display()
                        ));
                        continue;
                    }
                    bitfields |= (l as u8) << 1;
                }
                if let Some(toml::Value::Integer(v)) = sec.get("Height") {
                    let h = *v as i32 as i8;
                    if !(0..=8).contains(&h) {
                        self.warnings.push(format!(
                            "{}: invalid floorHeight ({h}) for column {n}",
                            p.display()
                        ));
                        continue;
                    }
                    bitfields |= ((h as u8) << 4) & 0xF0;
                }
                let c = &mut self.columnset[n];
                c.bitfields = bitfields;
                if let Some(toml::Value::Integer(v)) = sec.get("SolidMask") {
                    c.solidmask = *v as u16;
                }
                if let Some(toml::Value::Integer(v)) = sec.get("FloorTexture") {
                    c.floor_texture = *v as u16;
                }
                if let Some(toml::Value::Integer(v)) = sec.get("Orientation") {
                    c.orient = *v as u8;
                }
                if let Some(toml::Value::Array(a)) = sec.get("Cubes") {
                    for k in 0..8 {
                        c.cubes[k] = a.get(k).map(value_int32).unwrap_or(0) as u16;
                    }
                }
            }
        }
        Ok(())
    }

    /// `load_slab_datclm_files` (src/lvl_filesdk1.c:1019): count slabset references per columnset
    /// column (`update_columns_use`), create the referenced ones in the pool
    /// (`create_columns_from_list`, which recomputes the solid mask and heights), then map each
    /// slabset id to a pool column (`update_slabset_column_indices`, which searches with the
    /// columnset entry *as loaded*, including its `SolidMask`).
    ///
    /// Not modelled: columns already present in the level's `.clm` are found first by the game.
    /// They are content-equivalent, so the resulting column content is the same.
    fn resolve_slabset(&mut self) {
        let ccount = self.columnset.len();
        let mut used = vec![0u32; ccount];
        for ss in &self.slabset_raw {
            for &v in ss {
                let ncol = -(v as i32);
                if ncol >= 0 && (ncol as usize) < ccount {
                    used[ncol as usize] += 1;
                }
            }
        }
        // Pool order: the "unrevealed" column of init_whole_blocks (src/map_columns.c:420), then the
        // used columnset columns in id order.
        let mut pool: Vec<Column> = vec![crate::column::unrevealed_column()];
        for (i, c) in self.columnset.iter().enumerate().skip(1) {
            if used[i] > 0 && !pool.iter().any(|p| column_equivalent(p, c)) {
                pool.push(canonical_column(c, true));
            }
        }
        let mut unresolved = 0;
        self.slabset = self
            .slabset_raw
            .iter()
            .map(|ss| {
                let mut out = [SlabsetColumn::Unresolved(0); 9];
                for (k, &n) in ss.iter().enumerate() {
                    out[k] = if n >= 0 {
                        let l = Column {
                            floor_texture: n as u16,
                            ..Default::default()
                        };
                        SlabsetColumn::Column(canonical_column(&l, true))
                    } else {
                        let id = (-(n as i32)) as usize;
                        let found = (id < ccount)
                            .then(|| {
                                pool.iter()
                                    .find(|p| column_equivalent(p, &self.columnset[id]))
                            })
                            .flatten();
                        match found {
                            Some(c) => SlabsetColumn::Column(*c),
                            None => {
                                unresolved += 1;
                                SlabsetColumn::Unresolved(id as u16)
                            }
                        }
                    };
                }
                out
            })
            .collect();
        if unresolved > 0 {
            self.warnings.push(format!(
                "{unresolved} slabset column references not resolvable in the columnset"
            ));
        }
    }

    /// The column at position `i` (0..8) of slabset entry `id`, `None` when unresolved.
    pub fn slabset_column(&self, id: usize, i: usize) -> Option<&Column> {
        match self.slabset.get(id)?.get(i)? {
            SlabsetColumn::Column(c) => Some(c),
            SlabsetColumn::Unresolved(_) => None,
        }
    }

    // -------------------------------------------------------------- generic

    fn load_generic(&mut self, layers: &ConfigLayers) -> Result<()> {
        for (p, _) in layers.files("objects.cfg") {
            if let Some(b) = read_cfg_bytes(&p)? {
                self.objects_cfg.apply(&CfgDoc::from_bytes(&b));
                self.sources.push(p);
            }
        }
        for (p, _) in layers.files("trapdoor.cfg") {
            if let Some(b) = read_cfg_bytes(&p)? {
                self.trapdoor_cfg.apply(&CfgDoc::from_bytes(&b));
                self.sources.push(p);
            }
        }
        for (p, _) in layers.files("creature.cfg") {
            if let Some(b) = read_cfg_bytes(&p)? {
                self.creature_cfg.apply(&CfgDoc::from_bytes(&b));
                self.sources.push(p);
            }
        }
        if let Some(c) = self.creature_cfg.get("common", "Creatures") {
            self.creatures = c.split_whitespace().map(str::to_string).collect();
        }
        self.creature_cfgs.clear();
        for name in &self.creatures {
            let f = format!("{}.cfg", name.to_ascii_lowercase());
            if layers
                .creature_dirs
                .iter()
                .any(|d| find_ci(d, &f).is_some())
            {
                self.creature_cfgs.push(name.to_ascii_lowercase());
            }
        }
        for (p, _) in layers.files("effects.toml") {
            if let Some(t) = read_toml(&p)? {
                merge_toml(&mut self.effects, t);
                self.sources.push(p);
            }
        }
        Ok(())
    }
}

/// `value_int32` of the TOML library: integers truncated to 32 bits, anything else 0.
fn value_int32(v: &toml::Value) -> i32 {
    v.as_integer().map_or(0, |i| i as i32)
}

fn apply_slab_field(s: &mut SlabConfig, e: &Entry, slab_lookup: &dyn Fn(&str) -> Option<i64>) {
    s.raw.set(&e.key, &e.value);
    let k = e.key.to_ascii_uppercase();
    // Flag fields take the whole line (argnum -1).
    match k.as_str() {
        "BLOCKFLAGS" => {
            s.block_flags = value_flags(&e.value, &slab_flags::NAMES, s.block_flags as i64) as u32;
            return;
        }
        "NOBLOCKFLAGS" => {
            s.noblck_flags =
                value_flags(&e.value, &slab_flags::NAMES, s.noblck_flags as i64) as u32;
            return;
        }
        _ => {}
    }
    let Some(t) = e.tokens.first() else { return };
    let u8f = |t: &str| value_default(t, 0, 255, &no_names) as u8;
    let i16f = |t: &str| value_default(t, i16::MIN as i64, i16::MAX as i64, &no_names) as i16;
    match k.as_str() {
        "NAME" => s.name = t.clone(),
        "BLOCKFLAGSHEIGHT" => s.block_flags_height = i16f(t),
        "BLOCKHEALTHINDEX" => s.block_health_index = i16f(t),
        "FILLSTYLE" => s.fill_style = u8f(t),
        "CATEGORY" => s.category = u8f(t),
        "SLBID" => s.slb_id = u8f(t),
        "WIBBLE" => s.wibble = u8f(t),
        "ISSAFELAND" => s.is_safe_land = u8f(t),
        "ISDIGGABLE" => s.is_diggable = u8f(t),
        "WLBTYPE" => s.wlb_type = u8f(t),
        "ANIMATED" => s.animated = u8f(t),
        "ISOWNABLE" => s.is_ownable = u8f(t),
        "INDESTRUCTIBLE" => s.indestructible = u8f(t),
        "GOLDHELD" => {
            s.gold_held = value_default(t, i32::MIN as i64, i32::MAX as i64, &no_names) as i32
        }
        _ => {
            let _ = slab_lookup;
        }
    }
}

/// Group config files of a whole install by the cfg that owns a levels folder. Convenience for
/// tools and tests: `(levels_dir, configs_dir, creatures_dir)` of every campaign / map pack cfg.
pub fn pack_folders(install: &Path) -> Vec<(PathBuf, Option<PathBuf>, Option<PathBuf>)> {
    let mut out = vec![];
    for top in ["campgns", "levels", "multiplayer"] {
        let Some(dir) = find_ci(install, top) else {
            continue;
        };
        let mut cfgs: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cfg")))
            .collect();
        cfgs.sort();
        for cfg in cfgs {
            if let Ok(r) = kfx_formats::campaign::resolve(install, &cfg) {
                if let Some(l) = r.levels_dir {
                    out.push((l, r.configs_dir, r.creatures_dir));
                }
            }
        }
    }
    out
}

/// Per-key map of all raw fields of a slab, convenient for display.
pub fn raw_map(r: &RawFields) -> BTreeMap<String, String> {
    r.0.iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_flags() {
        assert_eq!(parse_number(" 12 "), Some(12));
        assert_eq!(parse_number("-3"), Some(-3));
        assert_eq!(parse_number("1a"), None);
        assert_eq!(parse_number(""), None);
        assert_eq!(value_flags("BLOCKING FILLED", &slab_flags::NAMES, 0), 0x30);
        assert_eq!(value_flags("", &slab_flags::NAMES, 7), 0);
        assert_eq!(value_flags("none", &slab_flags::NAMES, 7), 0);
        assert_eq!(value_flags("FILLED 1", &slab_flags::NAMES, 0x10), 0x30);
        assert_eq!(value_flags("BLOCKING 0", &slab_flags::NAMES, 0x30), 0x20);
        assert_eq!(value_default("300", 0, 255, &no_names), 255);
    }

    #[test]
    fn layered_terrain_and_cubes() {
        let dir = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-map/kfx_map_gd_{}",
            std::process::id()
        ));
        let base = dir.join("fx");
        let camp = dir.join("camp");
        let lvl = dir.join("lvl");
        for d in [&base, &camp, &lvl] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            base.join("terrain.cfg"),
            "[slab0]\nName = HARD\nCategory = 0\nBlockFlags = BLOCKING FILLED\n[slab1]\nName = \
                GOLD\nWibble = 1\n[room0]\nName = NULL\nSlabAssign = HARD\n[room1]\nName = \
                X\nSlabAssign = NEWSLAB\n",
        )
        .unwrap();
        std::fs::write(
            camp.join("terrain.cfg"),
            "[slab1]\nWibble = 2\n[slab2]\nName = NEWSLAB\n",
        )
        .unwrap();
        std::fs::write(
            lvl.join("map00003.terrain.cfg"),
            "[slab0]\nBlockFlags = DIGGABLE 1\n",
        )
        .unwrap();
        std::fs::write(
            base.join("cubes.cfg"),
            "[cube1]\nName = A\nOwnershipGroup = 2\nOwner = BLUE\n[cube2]\nOwner = RED\n",
        )
        .unwrap();
        let layers = ConfigLayers {
            layers: vec![
                ConfigLayer::dir(&base),
                ConfigLayer::dir(&camp),
                ConfigLayer::map(&lvl, 3),
            ],
            creature_dirs: vec![],
        };
        let gd = GameData {
            ..Default::default()
        };
        let mut gd = gd;
        gd.load_terrain(&layers).unwrap();
        gd.load_cubes(&layers).unwrap();
        assert_eq!(gd.slabs.len(), 3);
        assert_eq!(gd.slabs[1].wibble, 2);
        assert_eq!(gd.slabs[1].name, "GOLD");
        assert_eq!(gd.slabs[0].block_flags, 0x38);
        // SlabAssign resolves a name defined in a later layer (list pass first).
        assert_eq!(gd.rooms[1].assigned_slab, 2);
        assert_eq!(gd.slab_room(2), 1);
        assert_eq!(gd.cube_bits[2][1], 1);
        assert!(gd.warnings.iter().any(|w| w.contains("cube2")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
