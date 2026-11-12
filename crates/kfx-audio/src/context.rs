//! Where the game looks for sound files: the install, the campaign folders, the mods, the map zip.
//!
//! Folder groups are `FGrp_*` in `src/config.c:1285-1415` (`_resolve_file_path_internal`):
//! `FxData` = `<install>/fxdata`, `CmpgConfig` / `CmpgLvls` / `CmpgCrtrs` / `CmpgMedia` /
//! `AtlSound` = the campaign's `CONFIGS_LOCATION` / `LEVELS_LOCATION` / `CREATURES_LOCATION` /
//! `MEDIA_LOCATION` / `[speech]` folder, `CrtrData` = `<install>/creatrs`, `LrgSound` =
//! `<install>/sound`, `Music` = `<install>/music`, `Main` = the install root. A mod adds its folder
//! in front of the group folder: `<install>/mods/<mod>/<group folder>/<file>`
//! (`src/config.c:1423-1440`).
//!
//! Mods come from `mods/load_order.cfg` (`src/config_mods.c:144`, [`MODS_LOAD_ORDER_FILE_NAME`]) in
//! three tiers `[after_base]`, `[after_campaign]`, `[after_map]`; later mods win.
use crate::error::{Error, Result};
use kfx_formats::campaign::Campaign;
use kfx_formats::cfg::CfgDoc;
use kfx_formats::testutil::resolve_ci;
use std::path::{Path, PathBuf};

pub const MODS_LOAD_ORDER_FILE_NAME: &str = "load_order.cfg";

/// Extension order when a sound path has none (`resolve_sounds_cfg_sound_path`,
/// `sound_manager.cpp:667`).
pub const SOUND_EXTS: [&str; 4] = [".wav", ".mp3", ".ogg", ".flac"];

/// The mod tiers, each in load order (later = higher priority).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModTiers {
    pub after_base: Vec<String>,
    pub after_campaign: Vec<String>,
    pub after_map: Vec<String>,
}

impl ModTiers {
    /// Read `<install>/mods/load_order.cfg`; a mod whose folder does not exist is dropped
    /// (`recheck_all_mod_exist`). A missing file means no mods.
    pub fn load(install: &Path) -> ModTiers {
        let Some(path) = resolve_ci(install, &format!("mods/{MODS_LOAD_ORDER_FILE_NAME}"))
            .filter(|p| p.is_file())
        else {
            return ModTiers::default();
        };
        let Ok(doc) = CfgDoc::load(&path) else {
            return ModTiers::default();
        };
        let tier = |name: &str| -> Vec<String> {
            doc.entries(name)
                .into_iter()
                .map(|e| e.key)
                .filter(|m| resolve_ci(install, &format!("mods/{m}")).is_some_and(|p| p.is_dir()))
                .collect()
        };
        ModTiers {
            after_base: tier("after_base"),
            after_campaign: tier("after_campaign"),
            after_map: tier("after_map"),
        }
    }
}

/// The campaign (or map pack) folders, already resolved against the install (`None` = not set or
/// the folder does not exist).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CampaignDirs {
    pub cfg: Option<PathBuf>,
    pub levels: Option<PathBuf>,
    pub creatures: Option<PathBuf>,
    pub configs: Option<PathBuf>,
    pub media: Option<PathBuf>,
    /// The `[speech]` folder for the context language (`parse_campaign_speech_blocks`,
    /// `src/config_campaigns.c:847`: the first entry, replaced by an entry of the install language).
    pub speech: Option<PathBuf>,
    /// `SOUNDTRACK` key (a track number or a file in the media folder, `src/front_landview.c:1020`).
    pub soundtrack: Option<String>,
    /// Location strings as written (for the mods' copies of the folders).
    pub levels_rel: Option<String>,
    pub creatures_rel: Option<String>,
    pub configs_rel: Option<String>,
}

/// Everything resolution depends on.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// The install (the game's runtime directory).
    pub install: PathBuf,
    /// Three letter language code (`eng`), `get_language_lwrstr`.
    pub language: String,
    pub mods: ModTiers,
    pub campaign: CampaignDirs,
    /// The level whose layers apply (`mapNNNNN.sounds.cfg`, `mapNNNNN.zip`, ...).
    pub level: Option<u32>,
}

fn rel_dir(install: &Path, loc: &Option<String>) -> Option<PathBuf> {
    let loc = loc.as_ref()?.replace('\\', "/");
    resolve_ci(install, &loc).filter(|p| p.is_dir())
}

impl Context {
    pub fn new(install: impl Into<PathBuf>, language: &str) -> Context {
        let install = install.into();
        let mods = ModTiers::load(&install);
        Context {
            install,
            language: language.to_ascii_lowercase(),
            mods,
            campaign: CampaignDirs::default(),
            level: None,
        }
    }

    /// Select a campaign / map pack by its `.cfg` file.
    pub fn with_campaign(mut self, cfg: &Path) -> Result<Context> {
        let camp = Campaign::load(cfg)?;
        let c = camp.common();
        let mut speech: Option<String> = None;
        let mut n = 0;
        for (lang, folder) in camp.speech() {
            if lang.eq_ignore_ascii_case(&self.language) || n == 0 {
                speech = Some(folder);
                n += 1;
            }
        }
        self.campaign = CampaignDirs {
            cfg: Some(cfg.to_path_buf()),
            levels: rel_dir(&self.install, &c.levels_location),
            creatures: rel_dir(&self.install, &c.creatures_location),
            configs: rel_dir(&self.install, &c.configs_location),
            media: rel_dir(&self.install, &c.media_location),
            speech: rel_dir(&self.install, &speech),
            soundtrack: c.soundtrack.clone(),
            levels_rel: c.levels_location.clone(),
            creatures_rel: c.creatures_location.clone(),
            configs_rel: c.configs_location.clone(),
        };
        Ok(self)
    }

    pub fn with_level(mut self, level: u32) -> Context {
        self.level = Some(level);
        self
    }

    pub fn with_mods(mut self, mods: ModTiers) -> Context {
        self.mods = mods;
        self
    }

    pub fn mod_dir(&self, name: &str) -> Option<PathBuf> {
        resolve_ci(&self.install, &format!("mods/{name}")).filter(|p| p.is_dir())
    }

    /// `<mod>/<rel>/<file>` case-insensitively, if it exists as a file.
    pub fn mod_file(&self, name: &str, rel: &str, file: &str) -> Option<PathBuf> {
        let base = self.mod_dir(name)?;
        let rel = rel.replace('\\', "/");
        let joined = if rel.is_empty() {
            file.to_string()
        } else {
            format!("{rel}/{file}")
        };
        resolve_ci(&base, &joined).filter(|p| p.is_file())
    }

    pub fn install_file(&self, rel: &str) -> Option<PathBuf> {
        resolve_ci(&self.install, &rel.replace('\\', "/")).filter(|p| p.is_file())
    }

    /// `<levels folder>/mapNNNNN.zip` of the context level.
    pub fn map_zip(&self) -> Option<PathBuf> {
        let lv = self.level?;
        let dir = self.campaign.levels.as_ref()?;
        resolve_ci(dir, &format!("map{lv:05}.zip")).filter(|p| p.is_file())
    }
}

/// `dir/file` case-insensitively, if it is a file.
pub fn file_in(dir: &Option<PathBuf>, file: &str) -> Option<PathBuf> {
    resolve_ci(dir.as_ref()?, &file.replace('\\', "/")).filter(|p| p.is_file())
}

/// Does the path (after the last `/`) have an extension? (`has_ext` in `sound_manager.cpp`)
pub fn has_ext(path: &str) -> bool {
    let dot = path.rfind('.');
    let slash = path.rfind('/');
    match (dot, slash) {
        (Some(d), Some(s)) => d > s,
        (Some(_), None) => true,
        _ => false,
    }
}

/// The extension candidates the game tries for `path`: just the path with an extension, else
/// `path.wav`, `.mp3`, `.ogg`, `.flac` in that order.
pub fn ext_candidates(path: &str) -> Vec<String> {
    if has_ext(path) {
        vec![path.to_string()]
    } else {
        SOUND_EXTS.iter().map(|e| format!("{path}{e}")).collect()
    }
}

/// `sounds.cfg` variants (`sound_manager_load_named_sound`, `sound_manager.cpp:790-815`): with a
/// count of 1 the path is used as written; with more, the trailing digit run of the stem is
/// incremented (`hit1.wav` x3 -> `hit1`, `hit2`, `hit3`; `Hit_01.mp3` x2 -> `_01`, `_02`) and a
/// stem without digits gets `1`, `2`... appended (`boom.wav` x2 -> `boom1.wav`, `boom2.wav`;
/// `boom.wav` itself is not tried). Capped at 32.
pub fn expand_numbered(base: &str, count: usize) -> Vec<String> {
    if count <= 1 {
        return vec![base.to_string()];
    }
    let count = count.clamp(1, 32);
    let slash = base.rfind('/').or_else(|| base.rfind('\\'));
    let name_start = slash.map(|s| s + 1).unwrap_or(0);
    let dot = base.rfind('.').filter(|&d| d >= name_start);
    let (stem, ext) = match dot {
        Some(d) => (&base[..d], &base[d..]),
        None => (base, ""),
    };
    let digits_start = stem
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(i, _)| i)
        .unwrap_or(stem.len());
    let digits = &stem[digits_start..];
    let (prefix, mut width, mut base_num) = (
        &stem[..digits_start],
        digits.len(),
        digits.parse::<u64>().unwrap_or(1),
    );
    if width == 0 {
        width = 1;
        base_num = 1;
    }
    (0..count as u64)
        .map(|i| format!("{prefix}{:0width$}{ext}", base_num + i, width = width))
        .collect()
}

/// Creature `[sounds]` variants (`expand_numbered_sound_paths`, `src/config_crtrmodel.c:2321`): the
/// trailing digits of the *file name* (before the extension) are incremented; a name without digits
/// yields `count` copies of the same path. Capped at 32. Unlike [`expand_numbered`] a count of 1
/// still goes through this (same result: the path as written).
pub fn expand_creature_numbered(base: &str, count: usize) -> Vec<String> {
    let count = count.clamp(1, 32);
    let name_start = base
        .rfind('/')
        .or_else(|| base.rfind('\\'))
        .map(|s| s + 1)
        .unwrap_or(0);
    let dot = base.rfind('.').filter(|&d| d >= name_start);
    let (stem, ext) = match dot {
        Some(d) => (&base[..d], &base[d..]),
        None => (base, ""),
    };
    let mut num_start = stem.len();
    while num_start > name_start && stem.as_bytes()[num_start - 1].is_ascii_digit() {
        num_start -= 1;
    }
    if num_start >= stem.len() {
        return vec![base.to_string(); count];
    }
    let width = stem.len() - num_start;
    let n: u64 = stem[num_start..].parse().unwrap_or(0);
    (0..count as u64)
        .map(|i| {
            format!(
                "{}{:0width$}{ext}",
                &stem[..num_start],
                n + i,
                width = width
            )
        })
        .collect()
}

/// Read one entry of a zip, matching the name case-insensitively and `\` as `/`.
pub fn read_zip_entry(zip: &Path, entry: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(zip).map_err(|e| Error::io(zip, e))?;
    let mut ar = zip::ZipArchive::new(f)
        .map_err(|e| Error::format("zip", format!("{}: {e}", zip.display())))?;
    let want = entry.replace('\\', "/").to_ascii_lowercase();
    for i in 0..ar.len() {
        let mut ent = ar
            .by_index(i)
            .map_err(|e| Error::format("zip", e.to_string()))?;
        if ent.name().replace('\\', "/").to_ascii_lowercase() == want {
            if ent.size() > 32 * 1024 * 1024 {
                return Err(Error::format(
                    "zip",
                    format!("entry {entry} larger than 32 MiB (the game refuses it too)"),
                ));
            }
            let mut v = Vec::with_capacity(ent.size() as usize);
            ent.read_to_end(&mut v).map_err(|e| Error::io(zip, e))?;
            return Ok(v);
        }
    }
    Err(Error::NotFound(format!("{} in {}", entry, zip.display())))
}

/// Does the zip have this entry (same matching as [`read_zip_entry`])?
pub fn zip_has_entry(zip: &Path, entry: &str) -> bool {
    let Ok(f) = std::fs::File::open(zip) else {
        return false;
    };
    let Ok(ar) = zip::ZipArchive::new(f) else {
        return false;
    };
    let want = entry.replace('\\', "/").to_ascii_lowercase();
    let found = ar
        .file_names()
        .any(|n| n.replace('\\', "/").to_ascii_lowercase() == want);
    found
}
