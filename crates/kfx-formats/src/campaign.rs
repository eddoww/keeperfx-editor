//! Campaign and map pack `.cfg` files (`campgns/*.cfg`, `levels/*.cfg`, `multiplayer/*.cfg`).
//!
//! Reference: `src/config_campaigns.c`: `cmpgn_common_commands` (61),
//! `parse_campaign_common_blocks` (432), `[strings]` (792), `[speech]` (846),
//! `parse_campaign_map_block` (~884), `load_campaign` (~1200), `prepare_campaign_file_name` (1260);
//! folder resolution in `src/config.c:1349-1395` (`FGrp_CmpgLvls` etc.).
//!
//! What the game does with the file:
//! * `[common]` holds the commands of [`CampaignCommon`]; parsing stops at the next `[`.
//! * `[strings]` / `[speech]` map a language code (ENG, FRE, ...) to a file / folder.
//! * `[mapNNNNN]` blocks (same keys as a LOF, see [`crate::levelinfo`]) are read **only for real
//!   campaigns** (those in `campgns/`, `fgroup == FGrp_Campgn`, 1213-1214); for map packs
//!   (`levels/`, `multiplayer/`) level data comes from `mapNNNNN.lof` / `.lif` files in the levels
//!   folder instead (`find_and_load_lof_files`, src/lvl_filesdk1.c:640).
//! * All `*_LOCATION` values are folders **relative to the install root** (not to the cfg file):
//!   `src/config.c:1349-1395` joins `install_info.inst_path` (or the runtime dir) with the value.
//! * `SINGLE_LEVELS` / `EXTRA_LEVELS` accept numbers > 0, `BONUS_LEVELS` numbers >= 0 (0 = no
//!   bonus for that single level, positions matter); each of these keys may repeat and the lists
//!   accumulate (src/config_campaigns.c:455-500).
use crate::cfg::{CfgDoc, Entry};
use crate::levelinfo::{atoi, put, LevelInfo, LevelKind, Lif, Lof, DEFAULT_MAP_SIZE};
use crate::{rnc, Error, Result};
use std::path::{Path, PathBuf};

/// Which family the cfg belongs to (decides how levels are found).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackKind {
    /// `campgns/*.cfg`: levels from SINGLE/BONUS/EXTRA_LEVELS, data in `[mapNNNNN]` blocks.
    Campaign,
    /// `levels/*.cfg`: levels from `.lof` / `.lif` files in the levels folder.
    Mappack,
    /// `multiplayer/*.cfg`: same as [`PackKind::Mappack`].
    MultiplayerMappack,
}

impl PackKind {
    /// Folder prefix (`cmpgn_prefix`, src/config_campaigns.c:49).
    pub fn folder(self) -> &'static str {
        match self {
            PackKind::Campaign => "campgns",
            PackKind::Mappack => "levels",
            PackKind::MultiplayerMappack => "multiplayer",
        }
    }
    pub fn from_folder(s: &str) -> Option<PackKind> {
        [
            PackKind::Campaign,
            PackKind::Mappack,
            PackKind::MultiplayerMappack,
        ]
        .into_iter()
        .find(|k| k.folder().eq_ignore_ascii_case(s))
    }
}

/// Typed `[common]` block. `None` = key absent/empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CampaignCommon {
    pub name: Option<String>,
    /// `NAME_TEXT_ID`: string index or translation.toml id (raw).
    pub name_text_id: Option<String>,
    pub single_levels: Vec<u32>,
    /// Same length as `single_levels` normally; 0 = no bonus.
    pub bonus_levels: Vec<u32>,
    pub extra_levels: Vec<u32>,
    /// (number of entries, file name).
    pub high_scores: Option<(u32, String)>,
    pub land_view_start: Option<(String, String)>,
    pub land_view_end: Option<(String, String)>,
    /// (good, bad) sample ids.
    pub land_ambient: Option<(i32, i32)>,
    pub levels_location: Option<String>,
    pub land_location: Option<String>,
    pub creatures_location: Option<String>,
    pub configs_location: Option<String>,
    pub media_location: Option<String>,
    pub credits: Option<String>,
    pub intro_movie: Option<String>,
    pub outro_movie: Option<String>,
    /// ENSIGNS or PINPOINTS.
    pub land_markers: Option<String>,
    /// RED BLUE GREEN YELLOW WHITE NEUTRAL PURPLE BLACK ORANGE.
    pub human_player: Option<String>,
    pub assign_cpu_keepers: Option<String>,
    pub soundtrack: Option<String>,
    pub show_description: Option<bool>,
    pub description_geo: Option<[i32; 4]>,
}

fn nums(ents: &[Entry], key: &str, min: i32) -> Vec<u32> {
    let mut out = vec![];
    for e in ents.iter().filter(|e| e.key.eq_ignore_ascii_case(key)) {
        for t in &e.tokens {
            let k = atoi(t);
            if k >= min {
                out.push(k as u32);
            }
        }
    }
    out
}

impl CampaignCommon {
    pub fn read(doc: &CfgDoc) -> CampaignCommon {
        let ents = doc.entries("common");
        let last = |k: &str| ents.iter().rev().find(|e| e.key.eq_ignore_ascii_case(k));
        let text = |k: &str| last(k).map(|e| e.value.clone()).filter(|v| !v.is_empty());
        let tok = |k: &str| last(k).and_then(|e| e.tokens.first().cloned());
        let pair = |k: &str| {
            last(k).and_then(|e| {
                (e.tokens.len() >= 2).then(|| (e.tokens[0].clone(), e.tokens[1].clone()))
            })
        };
        CampaignCommon {
            name: text("NAME"),
            name_text_id: text("NAME_TEXT_ID"),
            single_levels: nums(&ents, "SINGLE_LEVELS", 1),
            bonus_levels: nums(&ents, "BONUS_LEVELS", 0),
            extra_levels: nums(&ents, "EXTRA_LEVELS", 1),
            high_scores: last("HIGH_SCORES").and_then(|e| {
                let n = atoi(e.tokens.first()?);
                let rest = e
                    .value
                    .split_once(|c: char| c == ' ' || c == '\t')?
                    .1
                    .trim_start()
                    .to_string();
                (n >= 0 && !rest.is_empty()).then_some((n as u32, rest))
            }),
            land_view_start: pair("LAND_VIEW_START"),
            land_view_end: pair("LAND_VIEW_END"),
            land_ambient: last("LAND_AMBIENT").and_then(|e| {
                (e.tokens.len() >= 2).then(|| (atoi(&e.tokens[0]), atoi(&e.tokens[1])))
            }),
            levels_location: text("LEVELS_LOCATION"),
            land_location: text("LAND_LOCATION"),
            creatures_location: text("CREATURES_LOCATION"),
            configs_location: text("CONFIGS_LOCATION"),
            media_location: text("MEDIA_LOCATION"),
            credits: text("CREDITS"),
            intro_movie: text("INTRO_MOVIE"),
            outro_movie: text("OUTRO_MOVIE"),
            land_markers: tok("LAND_MARKERS"),
            human_player: tok("HUMAN_PLAYER"),
            assign_cpu_keepers: tok("ASSIGN_CPU_KEEPERS"),
            soundtrack: text("SOUNDTRACK"),
            show_description: tok("SHOW_DESCRIPTION").map(|t| atoi(&t) == 1),
            description_geo: last("DESCRIPTION_GEO").and_then(|e| {
                (e.tokens.len() >= 4).then(|| {
                    let v: Vec<i32> = e.tokens.iter().take(4).map(|t| atoi(t)).collect();
                    [v[0], v[1], v[2], v[3]]
                })
            }),
        }
    }

    /// Write changed fields into `[common]` (the section is created when missing). Fields equal
    /// to what is currently in the document are not touched. Level lists are rewritten as one
    /// line (`KEY = a b c`) replacing the last occurrence; extra occurrences are removed.
    pub fn write(&self, doc: &mut CfgDoc) {
        let old = CampaignCommon::read(doc);
        let s = "common";
        let list = |v: &[u32]| {
            (!v.is_empty()).then(|| {
                v.iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        };
        let pair = |p: &Option<(String, String)>| p.as_ref().map(|(a, b)| format!("{a} {b}"));
        macro_rules! t {
            ($key:literal, $f:ident) => {
                put(doc, s, &[$key], self.$f.clone(), self.$f != old.$f);
            };
        }
        t!("NAME", name);
        t!("NAME_TEXT_ID", name_text_id);
        for (key, new, o) in [
            ("SINGLE_LEVELS", &self.single_levels, &old.single_levels),
            ("BONUS_LEVELS", &self.bonus_levels, &old.bonus_levels),
            ("EXTRA_LEVELS", &self.extra_levels, &old.extra_levels),
        ] {
            if new != o {
                // repeated keys accumulate in the game: collapse them into one line
                if doc.get_all(s, key).len() > 1 {
                    doc.remove(s, key);
                }
                put(doc, s, &[key], list(new), true);
            }
        }
        put(
            doc,
            s,
            &["HIGH_SCORES"],
            self.high_scores.as_ref().map(|(n, f)| format!("{n} {f}")),
            self.high_scores != old.high_scores,
        );
        put(
            doc,
            s,
            &["LAND_VIEW_START"],
            pair(&self.land_view_start),
            self.land_view_start != old.land_view_start,
        );
        put(
            doc,
            s,
            &["LAND_VIEW_END"],
            pair(&self.land_view_end),
            self.land_view_end != old.land_view_end,
        );
        put(
            doc,
            s,
            &["LAND_AMBIENT"],
            self.land_ambient.map(|(a, b)| format!("{a} {b}")),
            self.land_ambient != old.land_ambient,
        );
        t!("LEVELS_LOCATION", levels_location);
        t!("LAND_LOCATION", land_location);
        t!("CREATURES_LOCATION", creatures_location);
        t!("CONFIGS_LOCATION", configs_location);
        t!("MEDIA_LOCATION", media_location);
        t!("CREDITS", credits);
        t!("INTRO_MOVIE", intro_movie);
        t!("OUTRO_MOVIE", outro_movie);
        t!("LAND_MARKERS", land_markers);
        t!("HUMAN_PLAYER", human_player);
        t!("ASSIGN_CPU_KEEPERS", assign_cpu_keepers);
        t!("SOUNDTRACK", soundtrack);
        put(
            doc,
            s,
            &["SHOW_DESCRIPTION"],
            self.show_description.map(|b| (b as i32).to_string()),
            self.show_description != old.show_description,
        );
        put(
            doc,
            s,
            &["DESCRIPTION_GEO"],
            self.description_geo
                .map(|g| format!("{} {} {} {}", g[0], g[1], g[2], g[3])),
            self.description_geo != old.description_geo,
        );
    }
}

/// A campaign or map pack cfg: the lossless [`CfgDoc`] plus typed accessors.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Campaign {
    pub doc: CfgDoc,
}

impl Campaign {
    pub fn parse(bytes: &[u8]) -> Campaign {
        Campaign {
            doc: CfgDoc::from_bytes(bytes),
        }
    }
    pub fn load(path: &Path) -> Result<Campaign> {
        Ok(Campaign::parse(&rnc::load_file(path)?))
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.doc.save(path)
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        self.doc.to_bytes()
    }
    pub fn common(&self) -> CampaignCommon {
        CampaignCommon::read(&self.doc)
    }
    pub fn set_common(&mut self, c: &CampaignCommon) {
        c.write(&mut self.doc);
    }
    /// `[strings]`: (language code, file) in order, including only uncommented entries.
    pub fn strings(&self) -> Vec<(String, String)> {
        self.doc
            .entries("strings")
            .into_iter()
            .map(|e| (e.key, e.value))
            .collect()
    }
    /// `[speech]`: (language code, folder).
    pub fn speech(&self) -> Vec<(String, String)> {
        self.doc
            .entries("speech")
            .into_iter()
            .map(|e| (e.key, e.value))
            .collect()
    }
    /// Level numbers that have a `[mapNNNNN]` block, in file order.
    pub fn map_block_numbers(&self) -> Vec<u32> {
        self.doc
            .section_names()
            .iter()
            .filter_map(|n| {
                let d = n.get(3..)?;
                (n.get(..3)?.eq_ignore_ascii_case("map")
                    && !d.is_empty()
                    && d.bytes().all(|c| c.is_ascii_digit()))
                .then(|| d.parse().ok())
                .flatten()
            })
            .collect()
    }
    fn block_name(n: u32) -> String {
        format!("map{n:05}")
    }
    /// Typed `[mapNNNNN]` block, `None` when there is no such block.
    pub fn map_info(&self, n: u32) -> Option<LevelInfo> {
        let name = Self::block_name(n);
        self.doc
            .has_section(&name)
            .then(|| LevelInfo::read(&self.doc, &name))
    }
    /// Write changed fields of a `[mapNNNNN]` block, creating the block when needed.
    pub fn set_map_info(&mut self, n: u32, info: &LevelInfo) {
        let name = Self::block_name(n);
        if !self.doc.has_section(&name) {
            self.doc.add_section(&name);
        }
        info.write(&mut self.doc, &name);
    }
    pub fn remove_map_block(&mut self, n: u32) -> bool {
        self.doc.remove_section(&Self::block_name(n))
    }
}

/// Where a level's map size came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapSizeSource {
    /// Nothing declared: `DEFAULT_MAP_SIZE` x `DEFAULT_MAP_SIZE`.
    Default,
    /// `MAPSIZE` in a `[mapNNNNN]` block of the campaign cfg (campaigns only).
    CampaignBlock,
    /// `MAPSIZE` in `mapNNNNN.lof`.
    Lof,
}

/// One level of a resolved campaign / map pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLevel {
    pub number: u32,
    /// Lists the level belongs to (campaign lists, or LOF `KIND`; LIF-only entries are `Free`).
    pub kinds: Vec<LevelKind>,
    pub name: Option<String>,
    /// Map dimensions in slabs (x, y).
    pub mapsize: (i32, i32),
    pub mapsize_source: MapSizeSource,
    /// `mapNNNNN.lof` in the levels folder, if present.
    pub lof: Option<PathBuf>,
    /// `mapNNNNN.slb` in the levels folder, if present (the level really exists).
    pub slb: Option<PathBuf>,
}

/// Result of [`resolve`].
#[derive(Clone, Debug)]
pub struct ResolvedCampaign {
    pub kind: PackKind,
    pub cfg_path: PathBuf,
    pub name: String,
    pub common: CampaignCommon,
    /// Existing folders (`None` when the location is unset or the folder does not exist).
    pub levels_dir: Option<PathBuf>,
    pub configs_dir: Option<PathBuf>,
    pub creatures_dir: Option<PathBuf>,
    pub media_dir: Option<PathBuf>,
    pub land_dir: Option<PathBuf>,
    /// Sorted by number, one entry per level number.
    pub levels: Vec<ResolvedLevel>,
}

fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
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

fn resolve_dir(root: &Path, loc: &Option<String>) -> Option<PathBuf> {
    let loc = loc.as_ref()?;
    crate::testutil::resolve_ci(root, &loc.replace('\\', "/")).filter(|p| p.is_dir())
}

/// Determine the pack kind from the cfg's parent folder name.
pub fn kind_of(cfg_path: &Path) -> Option<PackKind> {
    PackKind::from_folder(&cfg_path.parent()?.file_name()?.to_string_lossy())
}

/// Resolve a campaign / map pack cfg against an install root.
///
/// Level discovery: campaigns list SINGLE/BONUS/EXTRA_LEVELS; map packs list every
/// `mapNNNNN.lof` and every `.lif` entry found in the levels folder. Map size priority:
/// `[mapNNNNN] MAPSIZE` (campaign only, as in the game) > `mapNNNNN.lof` MAPSIZE (the game ignores
/// LOFs of campaigns; this is an editor convenience) > 85x85.
pub fn resolve(install_root: &Path, cfg_path: &Path) -> Result<ResolvedCampaign> {
    let camp = Campaign::load(cfg_path)?;
    let kind = kind_of(cfg_path).ok_or_else(|| {
        Error::format(
            "campaign",
            format!(
                "{}: not in campgns/, levels/ or multiplayer/",
                cfg_path.display()
            ),
        )
    })?;
    let common = camp.common();
    let levels_dir = resolve_dir(install_root, &common.levels_location);
    let mut levels: Vec<ResolvedLevel> = vec![];
    let mut add = |n: u32, kind: Option<LevelKind>, name: Option<String>| {
        if let Some(l) = levels.iter_mut().find(|l| l.number == n) {
            if let Some(k) = kind {
                if !l.kinds.contains(&k) {
                    l.kinds.push(k);
                }
            }
            if l.name.is_none() {
                l.name = name;
            }
        } else {
            levels.push(ResolvedLevel {
                number: n,
                kinds: kind.into_iter().collect(),
                name,
                mapsize: (DEFAULT_MAP_SIZE, DEFAULT_MAP_SIZE),
                mapsize_source: MapSizeSource::Default,
                lof: None,
                slb: None,
            });
        }
    };
    let block_name = |n: u32| camp.map_info(n).and_then(|i| i.name_text);
    if kind == PackKind::Campaign {
        for &n in &common.single_levels {
            add(n, Some(LevelKind::Single), block_name(n));
        }
        for &n in common.bonus_levels.iter().filter(|&&n| n > 0) {
            add(n, Some(LevelKind::Bonus), block_name(n));
        }
        for &n in &common.extra_levels {
            add(n, Some(LevelKind::Extra), block_name(n));
        }
    } else if let Some(dir) = &levels_dir {
        let mut lofs = vec![];
        let mut lifs = vec![];
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                let fname = e.file_name().to_string_lossy().to_string();
                let ext = p
                    .extension()
                    .map(|x| x.to_string_lossy().to_ascii_lowercase());
                match ext.as_deref() {
                    Some("lof") => lofs.push((fname, p)),
                    Some("lif") => lifs.push((fname, p)),
                    _ => {}
                }
            }
        }
        lofs.sort();
        lifs.sort();
        for (fname, p) in &lofs {
            if let Some(n) = crate::levelinfo::level_number_from_file_name(fname) {
                let info = Lof::load(p)?.info();
                if info.kinds.is_empty() {
                    add(n, None, info.name_text.clone());
                }
                for k in &info.kinds {
                    add(n, Some(*k), info.name_text.clone());
                }
            }
        }
        for (_, p) in &lifs {
            for e in Lif::load(p)?.entries() {
                if e.number > 0 {
                    add(e.number as u32, Some(LevelKind::Free), Some(e.name.clone()));
                }
            }
        }
    }
    // per-level files and map size
    for l in &mut levels {
        if let Some(dir) = &levels_dir {
            l.lof = find_ci(dir, &format!("map{:05}.lof", l.number));
            l.slb = find_ci(dir, &format!("map{:05}.slb", l.number));
        }
        let from_block = (kind == PackKind::Campaign)
            .then(|| camp.map_info(l.number).and_then(|i| i.mapsize))
            .flatten();
        if let Some(sz) = from_block {
            l.mapsize = sz;
            l.mapsize_source = MapSizeSource::CampaignBlock;
        } else if let Some(sz) = l
            .lof
            .as_ref()
            .and_then(|p| Lof::load(p).ok())
            .and_then(|f| f.info().mapsize)
        {
            l.mapsize = sz;
            l.mapsize_source = MapSizeSource::Lof;
        }
    }
    levels.sort_by_key(|l| l.number);
    Ok(ResolvedCampaign {
        kind,
        cfg_path: cfg_path.to_path_buf(),
        name: common.name.clone().unwrap_or_default(),
        configs_dir: resolve_dir(install_root, &common.configs_location),
        creatures_dir: resolve_dir(install_root, &common.creatures_location),
        media_dir: resolve_dir(install_root, &common.media_location),
        land_dir: resolve_dir(install_root, &common.land_location),
        common,
        levels_dir,
        levels,
    })
}
