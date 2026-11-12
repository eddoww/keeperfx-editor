//! The KeeperFX mod layers: `mods/load_order.cfg`, and the full ordered list of configuration
//! layers (base, `after_base` mods, campaign, `after_campaign` mods, map, `after_map` mods) the
//! game loads every config and creature config through.
//!
//! # `load_order.cfg` (`src/config_mods.c`, `src/config_mods.h`)
//! File `mods/load_order.cfg` (`MODS_DIR_NAME` / `MODS_LOAD_ORDER_FILE_NAME`, src/config_mods.h:42-43;
//! the shipped `config/mods/_load_order.cfg` is a disabled example: it only counts once renamed).
//! Three blocks, `[after_base]`, `[after_campaign]`, `[after_map]` (src/config_mods.h:45-47), found
//! with `find_conf_block` (case-insensitive name). Inside a block every non-blank line that is not
//! a `;` comment is one mod folder name under `mods/`, at most [`MOD_ITEM_MAX`] (50) per block,
//! names must be shorter than 64 bytes, and the block ends at the next `[` (`parse_block_mods`
//! src/config_mods.c:35-65). There are no inline comments: `mymod ; x` is the name `mymod ; x`. A mod
//! whose folder does not exist is skipped with a warning (`recheck_block_mod_list_exist`
//! src/config_mods.c:67-132). The file must be 2..=65536 bytes (`load_mods_order_config_file`
//! src/config_mods.c:149-185). Nothing is read with `-nomods` (src/config_keeperfx.c:1262).
//!
//! # Mods are ignored in multiplayer
//! `get_loaded_mods_conf` (src/config_mods.c:24-29) returns an empty list when the network is active
//! or the game kind is multiplayer, so [`ModsConfig::for_game`] with `multiplayer = true` is empty.
//!
//! # Layer order
//! For a config file `F` (`load_config`, src/config.c:2326-2370): `fxdata/F`; the `after_base` mods;
//! the campaign `CONFIGS_LOCATION/F`; the `after_campaign` mods; `LEVELS_LOCATION/map%05d.F`; the
//! `after_map` mods. A mod contributes up to three files (`load_config_for_mod`, src/config.c:2256):
//! `mods/<m>/fxdata/F`, `mods/<m>/<CONFIGS_LOCATION>/F` and `mods/<m>/<LEVELS_LOCATION>/map%05d.F`.
//! Later layers win. For a creature `C` (`load_creaturemodel_config`, src/config_crtrmodel.c:2894-2946,
//! mods `load_creaturemodel_config_for_mod` src/config_crtrmodel.c:2831): `creatrs/C.cfg`, the
//! `after_base` mods, `CREATURES_LOCATION/C.cfg`, `after_campaign` mods,
//! `LEVELS_LOCATION/map%05d.C.cfg`, `after_map` mods; a mod contributes `mods/<m>/creatrs/C.cfg`,
//! `mods/<m>/<CREATURES_LOCATION>/C.cfg`, `mods/<m>/<LEVELS_LOCATION>/map%05d.C.cfg`. Path groups:
//! `prepare_file_path_buf_mod` / `_resolve_file_path_internal` (src/config.c:1270-1511).
use crate::gamedata::{find_ci, ConfigLayer, ConfigLayers};
use kfx_formats::campaign::CampaignCommon;
use kfx_formats::{rnc, testutil};
use std::path::{Path, PathBuf};

/// `MOD_ITEM_MAX` (src/config_mods.h:52).
pub const MOD_ITEM_MAX: usize = 50;
/// Mod names must be shorter than `COMMAND_WORD_LEN` = 64 (src/bflib_basics.h:49).
pub const MOD_NAME_MAX: usize = 63;
/// `MODS_AFTER_*_BLOCK_NAME` (src/config_mods.h:45-47).
pub const BLOCK_NAMES: [&str; 3] = ["after_base", "after_campaign", "after_map"];

/// The parsed `load_order.cfg`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModsConfig {
    pub after_base: Vec<String>,
    pub after_campaign: Vec<String>,
    pub after_map: Vec<String>,
    /// Problems met while reading (too long names, too many mods, missing blocks are fine).
    pub warnings: Vec<String>,
}

fn is_blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 26) || b < 7
}

/// `find_conf_block` (src/config.c:186): the offset after the line of the first `[name]`.
fn find_block(buf: &[u8], name: &str) -> Option<usize> {
    let mut pos = 0;
    let n = name.len();
    let skip_line = |mut p: usize| {
        while p < buf.len() && buf[p] != b'\r' && buf[p] != b'\n' {
            p += 1;
        }
        while p < buf.len() && buf[p] <= 32 {
            p += 1;
        }
        p
    };
    while pos + n + 2 < buf.len() {
        while pos < buf.len() && matches!(buf[pos], b' ' | b'\t' | 26)
            || (pos < buf.len() && buf[pos] < 7)
        {
            pos += 1;
        }
        if pos >= buf.len() {
            return None;
        }
        if buf[pos] != b'[' {
            pos = skip_line(pos);
            continue;
        }
        pos += 1;
        while pos < buf.len() && (matches!(buf[pos], b' ' | b'\t' | 26) || buf[pos] < 7) {
            pos += 1;
        }
        if pos + n + 2 >= buf.len() {
            return None;
        }
        if !buf[pos..pos + n].eq_ignore_ascii_case(name.as_bytes()) {
            pos = skip_line(pos);
            continue;
        }
        pos += n;
        while pos < buf.len() && (matches!(buf[pos], b' ' | b'\t' | 26) || buf[pos] < 7) {
            pos += 1;
        }
        if pos >= buf.len() || buf[pos] != b']' {
            pos = skip_line(pos);
            continue;
        }
        return Some(skip_line(pos));
    }
    None
}

/// `parse_block_mods` (src/config_mods.c:35).
fn parse_block(buf: &[u8], block: &str, warnings: &mut Vec<String>) -> Vec<String> {
    let mut out = vec![];
    let Some(mut pos) = find_block(buf, block) else {
        return out;
    };
    while pos < buf.len() {
        if out.len() >= MOD_ITEM_MAX {
            warnings.push(format!(
                "[{block}]: more than {MOD_ITEM_MAX} mods, the rest is ignored"
            ));
            break;
        }
        while pos < buf.len() && is_blank(buf[pos]) {
            pos += 1;
        }
        if pos >= buf.len() {
            break;
        }
        match buf[pos] {
            b';' => {}
            b'[' => break,
            _ => {
                let start = pos;
                while pos < buf.len() && buf[pos] != b'\r' && buf[pos] != b'\n' && buf[pos] >= 7 {
                    pos += 1;
                }
                let mut end = pos;
                while end > start && matches!(buf[end - 1], b' ' | b'\t' | 26) {
                    end -= 1;
                }
                let name = kfx_formats::cfg::latin1_decode(&buf[start..end]);
                if name.len() > MOD_NAME_MAX {
                    warnings.push(format!(
                        "[{block}]: mod name longer than {MOD_NAME_MAX} bytes is ignored: {name}"
                    ));
                } else if !name.is_empty() {
                    out.push(name);
                }
            }
        }
        while pos < buf.len() && buf[pos] != b'\r' && buf[pos] != b'\n' {
            pos += 1;
        }
    }
    out
}

impl ModsConfig {
    /// Parse the text of a `load_order.cfg`.
    pub fn parse(bytes: &[u8]) -> ModsConfig {
        let mut w = vec![];
        let after_base = parse_block(bytes, BLOCK_NAMES[0], &mut w);
        let after_campaign = parse_block(bytes, BLOCK_NAMES[1], &mut w);
        let after_map = parse_block(bytes, BLOCK_NAMES[2], &mut w);
        ModsConfig {
            after_base,
            after_campaign,
            after_map,
            warnings: w,
        }
    }

    /// Read `<install>/mods/load_order.cfg`. `None` when absent, shorter than 2 bytes or longer
    /// than 65536 bytes (the game then runs without mods).
    pub fn load(install: &Path) -> Option<ModsConfig> {
        let p = testutil::resolve_ci(install, "mods/load_order.cfg").filter(|p| p.is_file())?;
        let b = rnc::load_file(&p).ok()?;
        (2..=65536)
            .contains(&b.len())
            .then(|| ModsConfig::parse(&b))
    }

    /// The mods that apply to a game: none in multiplayer (`get_loaded_mods_conf`,
    /// src/config_mods.c:24-29).
    pub fn for_game(&self, multiplayer: bool) -> ModsConfig {
        if multiplayer {
            ModsConfig::default()
        } else {
            self.clone()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.after_base.is_empty() && self.after_campaign.is_empty() && self.after_map.is_empty()
    }

    fn block(&self, s: Stage) -> &[String] {
        match s {
            Stage::AfterBase => &self.after_base,
            Stage::AfterCampaign => &self.after_campaign,
            Stage::AfterMap => &self.after_map,
            _ => &[],
        }
    }
}

/// Where in the load order a layer sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    Base,
    AfterBase,
    Campaign,
    AfterCampaign,
    Map,
    AfterMap,
}

/// What a layer is: the base data, the campaign, the map, or one of the mods of a block.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayerOrigin {
    pub stage: Stage,
    /// The mod folder for `After*` stages.
    pub mod_name: Option<String>,
}

impl LayerOrigin {
    pub fn base() -> LayerOrigin {
        LayerOrigin {
            stage: Stage::Base,
            mod_name: None,
        }
    }
    pub fn campaign() -> LayerOrigin {
        LayerOrigin {
            stage: Stage::Campaign,
            mod_name: None,
        }
    }
    pub fn map() -> LayerOrigin {
        LayerOrigin {
            stage: Stage::Map,
            mod_name: None,
        }
    }
    pub fn is_mod(&self) -> bool {
        self.mod_name.is_some()
    }
}

impl std::fmt::Display for LayerOrigin {
    /// "base", "campaign", "this map", or "mod X (after_base)".
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.mod_name, self.stage) {
            (Some(m), Stage::AfterBase) => write!(f, "mod {m} (after_base)"),
            (Some(m), Stage::AfterCampaign) => write!(f, "mod {m} (after_campaign)"),
            (Some(m), _) => write!(f, "mod {m} (after_map)"),
            (None, Stage::Base) => write!(f, "base"),
            (None, Stage::Campaign) => write!(f, "campaign"),
            (None, _) => write!(f, "this map"),
        }
    }
}

/// The relative folder names of a campaign (`CONFIGS_LOCATION`, `LEVELS_LOCATION`,
/// `CREATURES_LOCATION`), which a mod mirrors inside its own folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CampaignLocations {
    pub configs: Option<String>,
    pub levels: Option<String>,
    pub creatures: Option<String>,
}

impl CampaignLocations {
    pub fn from_common(c: &CampaignCommon) -> CampaignLocations {
        CampaignLocations {
            configs: c.configs_location.clone(),
            levels: c.levels_location.clone(),
            creatures: c.creatures_location.clone(),
        }
    }
}

/// Every layer a level's configs and creature configs are loaded through, in load order.
#[derive(Clone, Debug, Default)]
pub struct LayerStack {
    /// Layers of `fxdata`-style config files (`F` and `map%05d.F`).
    pub config: Vec<(LayerOrigin, ConfigLayer)>,
    /// Layers of creature configs (`C.cfg` and `map%05d.C.cfg`).
    pub creatures: Vec<(LayerOrigin, ConfigLayer)>,
    /// The mods that were applied (already filtered by [`ModsConfig::for_game`]).
    pub mods: ModsConfig,
}

fn sub_dir(root: &Path, loc: &Option<String>) -> Option<PathBuf> {
    let loc = loc.as_ref()?;
    testutil::resolve_ci(root, &loc.replace('\\', "/")).filter(|p| p.is_dir())
}

impl LayerStack {
    /// Build the stack for `level` (`None` = no map layer). `mods` is what
    /// [`ModsConfig::for_game`] returned (empty = no mods). Layers whose folder does not exist are
    /// left out, except the base. Mods whose folder does not exist are skipped.
    pub fn build(
        install: &Path,
        loc: &CampaignLocations,
        level: Option<u32>,
        mods: &ModsConfig,
    ) -> LayerStack {
        let mut st = LayerStack {
            mods: mods.clone(),
            ..Default::default()
        };
        let fx = find_ci(install, "fxdata").unwrap_or_else(|| install.join("fxdata"));
        st.config.push((LayerOrigin::base(), ConfigLayer::dir(fx)));
        let crt = find_ci(install, "creatrs").unwrap_or_else(|| install.join("creatrs"));
        st.creatures
            .push((LayerOrigin::base(), ConfigLayer::dir(crt)));
        let campaign_cfg = sub_dir(install, &loc.configs);
        let campaign_crt = sub_dir(install, &loc.creatures);
        let levels = sub_dir(install, &loc.levels);
        st.add_mods(install, loc, level, Stage::AfterBase);
        if let Some(d) = campaign_cfg {
            st.config
                .push((LayerOrigin::campaign(), ConfigLayer::dir(d)));
        }
        if let Some(d) = campaign_crt {
            st.creatures
                .push((LayerOrigin::campaign(), ConfigLayer::dir(d)));
        }
        st.add_mods(install, loc, level, Stage::AfterCampaign);
        if let (Some(d), Some(n)) = (&levels, level) {
            st.config.push((LayerOrigin::map(), ConfigLayer::map(d, n)));
            st.creatures
                .push((LayerOrigin::map(), ConfigLayer::map(d, n)));
        }
        st.add_mods(install, loc, level, Stage::AfterMap);
        st
    }

    fn add_mods(
        &mut self,
        install: &Path,
        loc: &CampaignLocations,
        level: Option<u32>,
        stage: Stage,
    ) {
        let names: Vec<String> = self.mods.block(stage).to_vec();
        for m in names {
            let Some(root) =
                testutil::resolve_ci(install, &format!("mods/{m}")).filter(|p| p.is_dir())
            else {
                continue;
            };
            let origin = LayerOrigin {
                stage,
                mod_name: Some(m.clone()),
            };
            // configs: fxdata, campaign configs folder, per-map file in the levels folder
            if let Some(d) = sub_dir(&root, &Some("fxdata".into())) {
                self.config.push((origin.clone(), ConfigLayer::dir(d)));
            }
            if let Some(d) = sub_dir(&root, &loc.configs) {
                self.config.push((origin.clone(), ConfigLayer::dir(d)));
            }
            let lv = sub_dir(&root, &loc.levels);
            if let (Some(d), Some(n)) = (&lv, level) {
                self.config.push((origin.clone(), ConfigLayer::map(d, n)));
            }
            // creatures: creatrs, campaign creatures folder, per-map file
            if let Some(d) = sub_dir(&root, &Some("creatrs".into())) {
                self.creatures.push((origin.clone(), ConfigLayer::dir(d)));
            }
            if let Some(d) = sub_dir(&root, &loc.creatures) {
                self.creatures.push((origin.clone(), ConfigLayer::dir(d)));
            }
            if let (Some(d), Some(n)) = (&lv, level) {
                self.creatures.push((origin, ConfigLayer::map(d, n)));
            }
        }
    }

    /// Existing files of config `name` (e.g. `rules.cfg`), in load order, with their layer.
    pub fn files(&self, name: &str) -> Vec<(LayerOrigin, PathBuf)> {
        self.config
            .iter()
            .filter_map(|(o, l)| l.file(name).map(|p| (o.clone(), p)))
            .collect()
    }

    /// Existing files of creature config `<creature>.cfg`, in load order, with their layer.
    pub fn creature_files(&self, creature: &str) -> Vec<(LayerOrigin, PathBuf)> {
        let f = format!("{}.cfg", creature.to_ascii_lowercase());
        self.creatures
            .iter()
            .filter_map(|(o, l)| l.file(&f).map(|p| (o.clone(), p)))
            .collect()
    }

    /// The [`ConfigLayers`] for [`crate::gamedata::GameData::load`] (mods included).
    pub fn to_config_layers(&self) -> ConfigLayers {
        ConfigLayers {
            layers: self.config.iter().map(|(_, l)| l.clone()).collect(),
            creature_dirs: self
                .creatures
                .iter()
                .filter(|(_, l)| l.prefix.is_empty())
                .map(|(_, l)| l.dir.clone())
                .collect(),
        }
    }
}

/// The stack a plain [`ConfigLayers`] corresponds to (no mods): the first layer is the base,
/// layers with a `mapNNNNN.` prefix are the map, the rest campaign; creature folders likewise.
pub fn stack_of_layers(layers: &ConfigLayers) -> LayerStack {
    let mut st = LayerStack::default();
    for (i, l) in layers.layers.iter().enumerate() {
        let o = if i == 0 {
            LayerOrigin::base()
        } else if l.prefix.is_empty() {
            LayerOrigin::campaign()
        } else {
            LayerOrigin::map()
        };
        st.config.push((o, l.clone()));
    }
    for (i, d) in layers.creature_dirs.iter().enumerate() {
        st.creatures.push((
            if i == 0 {
                LayerOrigin::base()
            } else {
                LayerOrigin::campaign()
            },
            ConfigLayer::dir(d),
        ));
    }
    // per-map creature files live next to the map config layers
    for (o, l) in st.config.clone() {
        if !l.prefix.is_empty() {
            st.creatures.push((o, l));
        }
    }
    st
}
