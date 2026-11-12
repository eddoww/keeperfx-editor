//! Configuration layers and where each config file of a layer lives.
//!
//! Load order of every config file (`load_config`, `src/config.c:2318-2360`): `fxdata/<file>`
//! (the base, a full load), mods "after base", the campaign `CONFIGS_LOCATION/<file>`, mods
//! "after campaign", `LEVELS_LOCATION/map%05d.<file>` (src/config.c:2344), mods "after map". All but
//! the base are partial loads (`CnfLd_AcceptPartial`): only what the file contains changes.
//!
//! Per-creature model files (`load_creaturemodel_config`, `src/config_crtrmodel.c:2905-2950`):
//! `creatrs/<name>.cfg` (`FGrp_CrtrData`), then the campaign `CREATURES_LOCATION/<name>.cfg`
//! (`FGrp_CmpgCrtrs`), then `LEVELS_LOCATION/map%05lu.<name>.cfg`, with mods in between
//! (src/config_crtrmodel.c:2838-2866).
//!
//! A mod folder `<mods>/<name>` holds `fxdata/`, `creatrs/`, ... like the install
//! (`prepare_file_path_buf_mod`).
use crate::fs::{find_ci, ConfigFs};
use kfx_map::gamedata::{ConfigLayer, ConfigLayers};
use std::path::{Path, PathBuf};

/// What a layer is, in load order terms.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LayerKind {
    /// `fxdata` + `creatrs` of the install.
    Base,
    /// A mod folder (name shown in the UI).
    Mod(String),
    /// The campaign / map pack `CONFIGS_LOCATION` (+ `CREATURES_LOCATION`).
    Campaign,
    /// `LEVELS_LOCATION/mapNNNNN.*` of one level.
    Map(u32),
}

impl LayerKind {
    /// Short badge text.
    pub fn badge(&self) -> String {
        match self {
            LayerKind::Base => "base".into(),
            LayerKind::Mod(n) => format!("mod {n}"),
            LayerKind::Campaign => "campaign".into(),
            LayerKind::Map(n) => format!("map {n}"),
        }
    }
}

/// One layer: where its config and creature files are, and the file name prefix.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Layer {
    pub kind: LayerKind,
    /// Folder of `<prefix><file>` config files (`fxdata`, `CONFIGS_LOCATION`, levels folder).
    pub dir: Option<PathBuf>,
    /// Folder of `<prefix><creature>.cfg` files (`creatrs`, `CREATURES_LOCATION`, levels folder).
    pub creature_dir: Option<PathBuf>,
    /// `""`, or `"map00012."` for a map layer.
    pub prefix: String,
}

/// A config file: a plain config (`rules.cfg`, `slabset.toml`) or a creature model file
/// (`creatrs/<lower-case name>.cfg`).
pub fn is_creature_file(key: &str) -> Option<&str> {
    key.strip_prefix("creatrs/")
}

impl Layer {
    pub fn base(fxdata: impl Into<PathBuf>, creatrs: Option<PathBuf>) -> Layer {
        Layer {
            kind: LayerKind::Base,
            dir: Some(fxdata.into()),
            creature_dir: creatrs,
            prefix: String::new(),
        }
    }
    pub fn campaign(configs: Option<PathBuf>, creatures: Option<PathBuf>) -> Layer {
        Layer {
            kind: LayerKind::Campaign,
            dir: configs,
            creature_dir: creatures,
            prefix: String::new(),
        }
    }
    pub fn map(levels_dir: impl Into<PathBuf>, level: u32) -> Layer {
        let d = levels_dir.into();
        Layer {
            kind: LayerKind::Map(level),
            dir: Some(d.clone()),
            creature_dir: Some(d),
            prefix: format!("map{level:05}."),
        }
    }
    /// A mod folder: `<mod>/fxdata` and `<mod>/creatrs`.
    pub fn mod_dir(name: impl Into<String>, root: &Path) -> Layer {
        Layer {
            kind: LayerKind::Mod(name.into()),
            dir: Some(root.join("fxdata")),
            creature_dir: Some(root.join("creatrs")),
            prefix: String::new(),
        }
    }

    /// `(folder, file name)` where config `key` of this layer lives, `None` when the layer has
    /// no folder for it.
    pub fn location(&self, key: &str) -> Option<(PathBuf, String)> {
        match is_creature_file(key) {
            Some(rest) => Some((
                self.creature_dir.clone()?,
                format!("{}{}", self.prefix, rest),
            )),
            None => Some((self.dir.clone()?, format!("{}{}", self.prefix, key))),
        }
    }

    /// The existing file for `key` (case-insensitive), else `None`.
    pub fn existing(&self, fs: &dyn ConfigFs, key: &str) -> Option<PathBuf> {
        let (d, n) = self.location(key)?;
        find_ci(fs, &d, &n)
    }

    /// Where a file for `key` is (existing spelling) or would be written.
    pub fn write_path(&self, fs: &dyn ConfigFs, key: &str) -> Option<PathBuf> {
        self.existing(fs, key)
            .or_else(|| self.location(key).map(|(d, n)| d.join(n)))
    }
}

/// The ordered layers, base first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LayerStack {
    pub layers: Vec<Layer>,
}

impl LayerStack {
    /// The layers the game uses for one level: base (`<install>/fxdata`, `<install>/creatrs`),
    /// the campaign when it has a `CONFIGS_LOCATION` or `CREATURES_LOCATION`, the map when a level
    /// is given. Folder names are resolved case-insensitively through `fs`.
    pub fn for_level(
        fs: &dyn ConfigFs,
        install: &Path,
        configs_dir: Option<&Path>,
        creatures_dir: Option<&Path>,
        levels_dir: Option<&Path>,
        level: Option<u32>,
    ) -> Self {
        let fx = find_ci(fs, install, "fxdata").unwrap_or_else(|| install.join("fxdata"));
        let cr = find_ci(fs, install, "creatrs");
        let mut layers = vec![Layer::base(fx, cr)];
        if configs_dir.is_some() || creatures_dir.is_some() {
            layers.push(Layer::campaign(
                configs_dir.map(Path::to_path_buf),
                creatures_dir.map(Path::to_path_buf),
            ));
        }
        if let (Some(d), Some(n)) = (levels_dir, level) {
            layers.push(Layer::map(d, n));
        }
        LayerStack { layers }
    }

    /// Insert a mod layer at load position `pos` (1 = right after the base).
    pub fn insert_mod(&mut self, pos: usize, name: &str, root: &Path) {
        self.layers
            .insert(pos.clamp(1, self.layers.len()), Layer::mod_dir(name, root));
    }

    pub fn index_of(&self, kind: &LayerKind) -> Option<usize> {
        self.layers.iter().position(|l| &l.kind == kind)
    }

    /// The map layer's level number, if any.
    pub fn level(&self) -> Option<u32> {
        self.layers.iter().find_map(|l| {
            if let LayerKind::Map(n) = l.kind {
                Some(n)
            } else {
                None
            }
        })
    }

    /// The equivalent `kfx_map` layer list (creature folders: base then campaign).
    pub fn to_config_layers(&self) -> ConfigLayers {
        let mut out = ConfigLayers::default();
        for l in &self.layers {
            if let Some(d) = &l.dir {
                out.layers.push(ConfigLayer {
                    dir: d.clone(),
                    prefix: l.prefix.clone(),
                });
            }
            if matches!(
                l.kind,
                LayerKind::Base | LayerKind::Campaign | LayerKind::Mod(_)
            ) {
                if let Some(c) = &l.creature_dir {
                    out.creature_dirs.push(c.clone());
                }
            }
        }
        out
    }
}
