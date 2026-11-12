//! The campaign / map pack model: a lossless cfg ([`Campaign`]) plus typed operations.
//!
//! Reference: `src/config_campaigns.c` (see [`kfx_formats::campaign`] for the key table). The cfg
//! document is the only source of truth, so an untouched pack saves back byte-identical.
//!
//! * A **campaign** (`campgns/*.cfg`) lists its levels in `SINGLE_LEVELS` / `BONUS_LEVELS` /
//!   `EXTRA_LEVELS` and describes each in a `[mapNNNNN]` block of the same cfg
//!   (`parse_campaign_map_blocks`, src/config_campaigns.c:1162). `BONUS_LEVELS` is parallel to
//!   `SINGLE_LEVELS`: entry *i* is the bonus level revealed by single level *i* (0 = none;
//!   `add_bonus_level_to_campaign`, :~284); the game warns when the lists differ in length
//!   (:779).
//! * A **map pack** (`levels/*.cfg`, `multiplayer/*.cfg`) has no level lists: its levels are the
//!   `mapNNNNN.lof` / `.lif` files of `LEVELS_LOCATION` (`find_and_load_lof_files`,
//!   src/lvl_filesdk1.c:632). Level info edits of a map pack go to the LOF files.
//!
//! Edits to the cfg stay in memory until [`Pack::save`]; edits that copy or remove level files
//! happen immediately and crash-safely (each folder through a
//! [`kfx_formats::atomic::SaveTransaction`]).
use kfx_formats::atomic::{self, SaveTransaction};
use kfx_formats::campaign::{kind_of, Campaign, CampaignCommon, PackKind};
use kfx_formats::level_bin::MapDims;
use kfx_formats::levelinfo::{LevelInfo, LevelKind, Lif, Lof, DEFAULT_MAP_SIZE};
use kfx_formats::testutil::resolve_ci;
use kfx_formats::{rnc, Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `CAMPAIGN_LEVELS_COUNT` (src/config_campaigns.h:33): single and bonus level slots.
pub const MAX_SINGLE_LEVELS: usize = 50;
/// `EXTRA_LEVELS_COUNT` (src/config_campaigns.h:34).
pub const MAX_EXTRA_LEVELS: usize = 10;
/// `BONUS_LEVEL_STORAGE_COUNT` (src/game_merge.h:44): bonus levels the save game can remember.
pub const MAX_BONUS_LEVELS_SAVED: usize = 6;
/// A campaign cfg larger than this is refused (`load_campaign`, src/config_campaigns.c:1220).
pub const MAX_CFG_BYTES: usize = 65536;
/// Highest level number that fits the `mapNNNNN` file name.
pub const MAX_LEVEL_NUMBER: u32 = 99999;

/// Values of `HUMAN_PLAYER` (`cmpgn_human_player_options`, src/config_campaigns.c:139).
pub const HUMAN_PLAYERS: [&str; 9] = [
    "RED", "BLUE", "GREEN", "YELLOW", "WHITE", "NEUTRAL", "PURPLE", "BLACK", "ORANGE",
];
/// Values of `LAND_MARKERS` (`cmpgn_level_markers_options`, config_campaigns.c:~121).
pub const LAND_MARKERS: [&str; 2] = ["ENSIGNS", "PINPOINTS"];
/// Values of a level's `ENSIGN` (`cmpgn_map_ensign_flag_options`, config_campaigns.c:~109).
pub const ENSIGN_KINDS: [&str; 7] = [
    "DEFAULT",
    "TUTORIAL",
    "SINGLE",
    "BONUS",
    "FULL_MOON",
    "NEW_MOON",
    "COOP",
];

/// Look a path up below `base` ignoring case; the part that does not exist is joined as written.
pub fn resolve_loose(base: &Path, rel: &str) -> PathBuf {
    let rel = rel.replace('\\', "/");
    let mut cur = base.to_path_buf();
    let mut missing = false;
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        if !missing {
            let exact = cur.join(part);
            if exact.exists() {
                cur = exact;
                continue;
            }
            let found = std::fs::read_dir(&cur).ok().and_then(|rd| {
                rd.flatten()
                    .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))
            });
            if let Some(f) = found {
                cur = f.path();
                continue;
            }
            missing = true;
        }
        cur.push(part);
    }
    cur
}

/// Files `mapNNNNN.*` of a level (any case, sorted) and the part of a name after `mapNNNNN.`:
/// the shared listing of `kfx_formats`.
pub use kfx_formats::level_bin::{level_file_ext, level_files};

/// Whether a level has its map file (`mapNNNNN.slb`), in `dir`.
pub fn level_exists(dir: &Path, number: u32) -> bool {
    kfx_formats::level_bin::find_file(dir, number, "slb").is_some()
}

/// Size of the map in slabs from the length of `mapNNNNN.slb` (2 bytes per slab): `declared`
/// when it has that many slabs, else the square size, else `None`.
pub fn real_map_size(dir: &Path, number: u32, declared: Option<(i32, i32)>) -> Option<(i32, i32)> {
    let p = kfx_formats::level_bin::find_file(dir, number, "slb")?;
    let len = rnc::load_file(&p).ok()?.len();
    if let Some((w, h)) = declared {
        if w > 0 && h > 0 && (w as usize) * (h as usize) * 2 == len {
            return Some((w, h));
        }
    }
    MapDims::from_slb_len(len).map(|d| (d.w as i32, d.h as i32))
}

/// A level of another pack or folder that can be copied into a pack.
#[derive(Clone, Debug)]
pub struct LevelSource {
    pub dir: PathBuf,
    pub number: u32,
    /// Level info the source pack declares (a `[mapNNNNN]` block), used when the folder has no LOF.
    pub info: Option<LevelInfo>,
}

impl LevelSource {
    pub fn new(dir: &Path, number: u32) -> LevelSource {
        LevelSource {
            dir: dir.to_path_buf(),
            number,
            info: None,
        }
    }
    pub fn files(&self) -> Vec<PathBuf> {
        level_files(&self.dir, self.number)
    }
    /// Info from the source: the given info, else the LOF, else just the LIF name.
    pub fn level_info(&self) -> LevelInfo {
        let mut info = self.info.clone().unwrap_or_default();
        if self.info.is_none() {
            if let Some(p) = kfx_formats::level_bin::find_file(&self.dir, self.number, "lof") {
                if let Ok(l) = Lof::load(&p) {
                    info = l.info();
                }
            }
        }
        if info.name_text.is_none() {
            if let Some(p) = kfx_formats::level_bin::find_file(&self.dir, self.number, "lif") {
                if let Ok(l) = Lif::load(&p) {
                    info.name_text = l
                        .entries()
                        .iter()
                        .find(|e| e.number == self.number as i32 && e.string_id.is_none())
                        .map(|e| e.name.clone());
                }
            }
        }
        info
    }
}

/// How a level takes part in a pack, one row per level number.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelEntry {
    pub number: u32,
    pub kinds: Vec<LevelKind>,
    /// Index in `SINGLE_LEVELS` (for a bonus level: of the single level that reveals it);
    /// `None` for extra levels and for map pack levels.
    pub slot: Option<usize>,
    pub info: LevelInfo,
    /// Campaign: there is a `[mapNNNNN]` block. Map pack: there is a `.lof`.
    pub has_info: bool,
    /// The level's `mapNNNNN.slb` exists in the levels folder.
    pub exists: bool,
}

impl LevelEntry {
    pub fn kind(&self) -> LevelKind {
        self.kinds.first().copied().unwrap_or(LevelKind::Free)
    }
    /// The name the game shows when no string id is set (`NAME_TEXT`).
    pub fn display_name(&self) -> String {
        self.info.name_text.clone().unwrap_or_default()
    }
}

/// One entry of `SINGLE_LEVELS` with its `BONUS_LEVELS` partner (0 = no bonus level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SingleSlot {
    pub level: u32,
    pub bonus: u32,
}

/// What [`Pack::add_level`] copies and where it goes.
#[derive(Clone, Debug)]
pub struct AddLevel {
    pub source: LevelSource,
    /// New level number; `None` takes the smallest free one (from 1 for single levels, 100 else).
    pub number: Option<u32>,
    pub kind: LevelKind,
    /// Position in the list of its kind (single / extra); `None` = the end.
    pub at: Option<usize>,
    /// Bonus levels: the single level slot that reveals it; `None` = the first slot without a
    /// bonus.
    pub bonus_for: Option<usize>,
    /// Replace the level files when the number already has some in the levels folder.
    pub overwrite: bool,
}

impl AddLevel {
    pub fn new(source: LevelSource, kind: LevelKind) -> AddLevel {
        AddLevel {
            source,
            number: None,
            kind,
            at: None,
            bonus_for: None,
            overwrite: false,
        }
    }
}

/// What [`Pack::add_level`] did.
#[derive(Clone, Debug, Default)]
pub struct Added {
    pub number: u32,
    pub copied: Vec<PathBuf>,
}

/// A campaign or map pack open for editing.
#[derive(Clone, Debug)]
pub struct Pack {
    pub install: PathBuf,
    pub cfg_path: PathBuf,
    pub kind: PackKind,
    pub cfg: Campaign,
    saved: Vec<u8>,
    /// Map packs: level info waiting to be written to `mapNNNNN.lof` on save.
    lof_edits: BTreeMap<u32, LevelInfo>,
}

fn err(msg: impl Into<String>) -> Error {
    Error::format("campaign", msg)
}

impl Pack {
    /// Open the cfg at `cfg_path` (a file in the install's `campgns/`, `levels/` or
    /// `multiplayer/` folder).
    pub fn load(install: &Path, cfg_path: &Path) -> Result<Pack> {
        let kind = kind_of(cfg_path).ok_or_else(|| {
            err(format!(
                "{}: not in campgns/, levels/ or multiplayer/",
                cfg_path.display()
            ))
        })?;
        let cfg = Campaign::load(cfg_path)?;
        let saved = cfg.to_bytes();
        Ok(Pack {
            install: install.to_path_buf(),
            cfg_path: cfg_path.to_path_buf(),
            kind,
            cfg,
            saved,
            lof_edits: BTreeMap::new(),
        })
    }

    /// A pack from cfg text that is not on disk yet.
    pub fn from_text(install: &Path, cfg_path: &Path, kind: PackKind, text: &str) -> Pack {
        let cfg = Campaign::parse(text.as_bytes());
        Pack {
            install: install.to_path_buf(),
            cfg_path: cfg_path.to_path_buf(),
            kind,
            saved: Vec::new(),
            cfg,
            lof_edits: BTreeMap::new(),
        }
    }

    /// The cfg file name without `.cfg` (what `-campaign` takes).
    pub fn stem(&self) -> String {
        self.cfg_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn is_campaign(&self) -> bool {
        self.kind == PackKind::Campaign
    }

    pub fn is_dirty(&self) -> bool {
        !self.lof_edits.is_empty() || self.cfg.to_bytes() != self.saved
    }

    /// Write the cfg (and pending LOF edits of a map pack). Refuses a cfg the game would reject
    /// as too large (src/config_campaigns.c:1220).
    pub fn save(&mut self) -> Result<()> {
        let bytes = self.cfg.to_bytes();
        if bytes.len() > MAX_CFG_BYTES {
            return Err(err(format!(
                "the cfg would be {} bytes, the game refuses files over {MAX_CFG_BYTES}",
                bytes.len()
            )));
        }
        if let Some(dir) = self.levels_dir() {
            if !self.lof_edits.is_empty() {
                let mut tx = SaveTransaction::new(&dir);
                for (n, info) in &self.lof_edits {
                    let mut lof = match kfx_formats::level_bin::find_file(&dir, *n, "lof") {
                        Some(p) => Lof::load(&p)?,
                        None => Lof::new(&LevelInfo::default()),
                    };
                    lof.set_info(info);
                    tx.write(kfx_formats::level_bin::file_name(*n, "lof"), lof.to_bytes());
                }
                tx.commit()?;
            }
        }
        self.lof_edits.clear();
        let name = self
            .cfg_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = self
            .cfg_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut tx = SaveTransaction::new(&dir);
        tx.write(name, bytes.clone());
        tx.commit()?;
        self.saved = bytes;
        Ok(())
    }

    // ------------------------------------------------------------- settings and folders

    pub fn common(&self) -> CampaignCommon {
        self.cfg.common()
    }

    pub fn set_common(&mut self, c: &CampaignCommon) {
        self.cfg.set_common(c);
    }

    /// Change `[common]` with a closure.
    pub fn update_common(&mut self, f: impl FnOnce(&mut CampaignCommon)) {
        let mut c = self.common();
        f(&mut c);
        self.set_common(&c);
    }

    /// An install-relative location (`LEVELS_LOCATION` ...) as a path, matched case-insensitively
    /// as far as it exists (`src/config.c:1349-1395` joins the value to the install root).
    pub fn loc(&self, rel: &str) -> PathBuf {
        resolve_loose(&self.install, rel)
    }

    pub fn levels_dir(&self) -> Option<PathBuf> {
        self.common().levels_location.map(|l| self.loc(&l))
    }

    pub fn land_dir(&self) -> Option<PathBuf> {
        self.common().land_location.map(|l| self.loc(&l))
    }

    pub fn media_dir(&self) -> Option<PathBuf> {
        self.common().media_location.map(|l| self.loc(&l))
    }

    /// The speech folder of a language code (`ENG`) as written in `[speech]`.
    pub fn speech_dir(&self, code: &str) -> Option<PathBuf> {
        self.cfg
            .speech()
            .into_iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(code))
            .map(|(_, f)| self.loc(&f))
    }

    /// The speech folder the game uses for the first language entry (`parse_campaign_speech_blocks`
    /// takes the language of the install, else the first entry: src/config_campaigns.c:871).
    pub fn default_speech_dir(&self) -> Option<PathBuf> {
        self.cfg
            .speech()
            .into_iter()
            .next()
            .map(|(_, f)| self.loc(&f))
    }

    // ------------------------------------------------------------- level lists

    /// `SINGLE_LEVELS` with their bonus partners. A bonus list shorter than the single list is
    /// padded with 0 (what the game's arrays hold).
    pub fn single_slots(&self) -> Vec<SingleSlot> {
        let c = self.common();
        c.single_levels
            .iter()
            .enumerate()
            .map(|(i, &level)| SingleSlot {
                level,
                bonus: c.bonus_levels.get(i).copied().unwrap_or(0),
            })
            .collect()
    }

    /// Write `SINGLE_LEVELS` and `BONUS_LEVELS` (always the same length).
    pub fn set_single_slots(&mut self, slots: &[SingleSlot]) {
        self.update_common(|c| {
            c.single_levels = slots.iter().map(|s| s.level).collect();
            c.bonus_levels = slots.iter().map(|s| s.bonus).collect();
        });
    }

    /// `EXTRA_LEVELS`: the full moon level, the new moon level, ...
    pub fn extra_levels(&self) -> Vec<u32> {
        self.common().extra_levels
    }

    pub fn set_extra_levels(&mut self, extra: &[u32]) {
        self.update_common(|c| c.extra_levels = extra.to_vec());
    }

    /// Every level number the cfg lists (single, bonus, extra), in that order.
    pub fn listed_levels(&self) -> Vec<(u32, LevelKind)> {
        let mut out = vec![];
        let slots = self.single_slots();
        out.extend(slots.iter().map(|s| (s.level, LevelKind::Single)));
        out.extend(
            slots
                .iter()
                .filter(|s| s.bonus > 0)
                .map(|s| (s.bonus, LevelKind::Bonus)),
        );
        out.extend(
            self.extra_levels()
                .into_iter()
                .map(|n| (n, LevelKind::Extra)),
        );
        out
    }

    /// The levels of the pack, one entry per number: singles in order, bonus levels in order,
    /// extra levels (campaigns); every LOF / LIF level sorted by number (map packs).
    pub fn entries(&self) -> Vec<LevelEntry> {
        let levels_dir = self.levels_dir();
        let exists = |n: u32| levels_dir.as_ref().is_some_and(|d| level_exists(d, n));
        if self.is_campaign() {
            let mut out: Vec<LevelEntry> = vec![];
            let slots = self.single_slots();
            let push = |n: u32, kind: LevelKind, slot: Option<usize>, out: &mut Vec<LevelEntry>| {
                if let Some(e) = out.iter_mut().find(|e| e.number == n) {
                    if !e.kinds.contains(&kind) {
                        e.kinds.push(kind);
                    }
                    return;
                }
                let block = self.cfg.map_info(n);
                out.push(LevelEntry {
                    number: n,
                    kinds: vec![kind],
                    slot,
                    has_info: block.is_some(),
                    info: block.unwrap_or_default(),
                    exists: exists(n),
                });
            };
            for (i, s) in slots.iter().enumerate() {
                push(s.level, LevelKind::Single, Some(i), &mut out);
            }
            for (i, s) in slots.iter().enumerate() {
                if s.bonus > 0 {
                    push(s.bonus, LevelKind::Bonus, Some(i), &mut out);
                }
            }
            for n in self.extra_levels() {
                push(n, LevelKind::Extra, None, &mut out);
            }
            out
        } else {
            let mut by_number: BTreeMap<u32, LevelEntry> = BTreeMap::new();
            if let Some(dir) = &levels_dir {
                if let Ok(rd) = std::fs::read_dir(dir) {
                    for e in rd.flatten() {
                        let fname = e.file_name().to_string_lossy().to_string();
                        let lower = fname.to_ascii_lowercase();
                        if lower.ends_with(".lof") {
                            if let Some(n) =
                                kfx_formats::levelinfo::level_number_from_file_name(&fname)
                            {
                                if let Ok(l) = Lof::load(&e.path()) {
                                    let info = l.info();
                                    let kinds = if info.kinds.is_empty() {
                                        vec![LevelKind::Free]
                                    } else {
                                        info.kinds.clone()
                                    };
                                    let ent = by_number.entry(n).or_insert_with(|| LevelEntry {
                                        number: n,
                                        kinds: vec![],
                                        slot: None,
                                        info: LevelInfo::default(),
                                        has_info: false,
                                        exists: exists(n),
                                    });
                                    ent.kinds = kinds;
                                    ent.info = info;
                                    ent.has_info = true;
                                }
                            }
                        } else if lower.ends_with(".lif") {
                            if let Ok(l) = Lif::load(&e.path()) {
                                for le in l.entries() {
                                    if le.number > 0 {
                                        let n = le.number as u32;
                                        let ent =
                                            by_number.entry(n).or_insert_with(|| LevelEntry {
                                                number: n,
                                                kinds: vec![LevelKind::Free],
                                                slot: None,
                                                info: LevelInfo::default(),
                                                has_info: false,
                                                exists: exists(n),
                                            });
                                        if ent.info.name_text.is_none() {
                                            ent.info.name_text = Some(le.name.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for (n, info) in &self.lof_edits {
                let ent = by_number.entry(*n).or_insert_with(|| LevelEntry {
                    number: *n,
                    kinds: vec![LevelKind::Free],
                    slot: None,
                    info: LevelInfo::default(),
                    has_info: false,
                    exists: exists(*n),
                });
                ent.info = info.clone();
                ent.has_info = true;
                if !info.kinds.is_empty() {
                    ent.kinds = info.kinds.clone();
                }
            }
            by_number.into_values().collect()
        }
    }

    /// Level info of a level: the `[mapNNNNN]` block (campaigns) or the LOF (map packs).
    pub fn level_info(&self, number: u32) -> LevelInfo {
        if self.is_campaign() {
            return self.cfg.map_info(number).unwrap_or_default();
        }
        if let Some(i) = self.lof_edits.get(&number) {
            return i.clone();
        }
        self.levels_dir()
            .and_then(|d| kfx_formats::level_bin::find_file(&d, number, "lof"))
            .and_then(|p| Lof::load(&p).ok())
            .map(|l| l.info())
            .unwrap_or_default()
    }

    /// Change the info of a level. Campaigns get the `[mapNNNNN]` block created when needed;
    /// map packs keep the info until [`Pack::save`] writes the LOF.
    pub fn set_level_info(&mut self, number: u32, info: &LevelInfo) {
        if self.is_campaign() {
            self.cfg.set_map_info(number, info);
        } else {
            self.lof_edits.insert(number, info.clone());
        }
    }

    /// Smallest level number `>= from` that is neither listed nor has files in the levels folder.
    pub fn free_number(&self, from: u32) -> u32 {
        let listed: Vec<u32> = self.entries().iter().map(|e| e.number).collect();
        let dir = self.levels_dir();
        let mut n = from.max(1);
        while listed.contains(&n) || dir.as_ref().is_some_and(|d| !level_files(d, n).is_empty()) {
            n += 1;
        }
        n
    }

    // ------------------------------------------------------------- list editing

    /// Move single level slot `from` to `to` (the bonus partner moves with it).
    pub fn move_single(&mut self, from: usize, to: usize) -> Result<()> {
        let mut slots = self.single_slots();
        if from >= slots.len() || to >= slots.len() {
            return Err(err("level position out of range"));
        }
        let s = slots.remove(from);
        slots.insert(to, s);
        self.set_single_slots(&slots);
        Ok(())
    }

    /// Move an extra level (`EXTRA_LEVELS` index `from` to `to`).
    pub fn move_extra(&mut self, from: usize, to: usize) -> Result<()> {
        let mut e = self.extra_levels();
        if from >= e.len() || to >= e.len() {
            return Err(err("level position out of range"));
        }
        let v = e.remove(from);
        e.insert(to, v);
        self.set_extra_levels(&e);
        Ok(())
    }

    /// Link bonus level `bonus` to single level slot `slot` (0 unlinks). The level must not be
    /// listed elsewhere.
    pub fn set_bonus(&mut self, slot: usize, bonus: u32) -> Result<()> {
        let mut slots = self.single_slots();
        if slot >= slots.len() {
            return Err(err("single level position out of range"));
        }
        if bonus > 0 {
            if bonus > MAX_LEVEL_NUMBER {
                return Err(err(format!(
                    "level number {bonus} is over {MAX_LEVEL_NUMBER}"
                )));
            }
            let used = self
                .listed_levels()
                .into_iter()
                .any(|(n, k)| n == bonus && !(k == LevelKind::Bonus && slots[slot].bonus == bonus));
            if used {
                return Err(err(format!("level {bonus} is already in the campaign")));
            }
        }
        slots[slot].bonus = bonus;
        self.set_single_slots(&slots);
        Ok(())
    }

    /// Take a level out of the lists and drop its `[mapNNNNN]` block. A single level takes its
    /// bonus level with it (the bonus list is parallel to the single list). With `delete_files`
    /// the level's `mapNNNNN.*` files are deleted from the levels folder. Returns the level
    /// numbers removed.
    pub fn remove_level(&mut self, number: u32, delete_files: bool) -> Result<Vec<u32>> {
        let mut removed = vec![];
        if self.is_campaign() {
            let mut slots = self.single_slots();
            if let Some(i) = slots.iter().position(|s| s.level == number) {
                let s = slots.remove(i);
                removed.push(s.level);
                if s.bonus > 0 {
                    removed.push(s.bonus);
                }
            } else if let Some(s) = slots.iter_mut().find(|s| s.bonus == number) {
                s.bonus = 0;
                removed.push(number);
            }
            self.set_single_slots(&slots);
            let mut extra = self.extra_levels();
            if let Some(i) = extra.iter().position(|&n| n == number) {
                extra.remove(i);
                self.set_extra_levels(&extra);
                removed.push(number);
            }
            if removed.is_empty() {
                return Err(err(format!("level {number} is not in the campaign")));
            }
            for &n in &removed {
                self.cfg.remove_map_block(n);
            }
        } else {
            removed.push(number);
            self.lof_edits.remove(&number);
        }
        if delete_files {
            if let Some(dir) = self.levels_dir() {
                for &n in &removed {
                    for f in level_files(&dir, n) {
                        std::fs::remove_file(&f).map_err(|e| Error::io(&f, e))?;
                    }
                }
            }
        }
        Ok(removed)
    }

    /// Give a level another number: rename its files, its `[mapNNNNN]` block, its list entries.
    pub fn renumber_level(&mut self, old: u32, new: u32) -> Result<()> {
        if new == 0 || new > MAX_LEVEL_NUMBER {
            return Err(err(format!(
                "level number {new} is not in 1..={MAX_LEVEL_NUMBER}"
            )));
        }
        if self.entries().iter().any(|e| e.number == new) {
            return Err(err(format!("level {new} already exists in the pack")));
        }
        if let Some(dir) = self.levels_dir() {
            if !level_files(&dir, new).is_empty() {
                return Err(err(format!(
                    "map{new:05}.* files already exist in {}",
                    dir.display()
                )));
            }
            let files = level_files(&dir, old);
            for f in &files {
                let ext = level_file_ext(&f.file_name().unwrap_or_default().to_string_lossy(), old)
                    .unwrap_or_default();
                let to = dir.join(format!("map{new:05}.{ext}"));
                std::fs::rename(f, &to).map_err(|e| Error::io(f, e))?;
            }
        }
        if self.is_campaign() {
            let mut slots = self.single_slots();
            for s in &mut slots {
                if s.level == old {
                    s.level = new;
                }
                if s.bonus == old {
                    s.bonus = new;
                }
            }
            self.set_single_slots(&slots);
            let extra: Vec<u32> = self
                .extra_levels()
                .iter()
                .map(|&n| if n == old { new } else { n })
                .collect();
            self.set_extra_levels(&extra);
            if let Some(info) = self.cfg.map_info(old) {
                self.cfg.remove_map_block(old);
                self.cfg.set_map_info(new, &info);
            }
        } else if let Some(i) = self.lof_edits.remove(&old) {
            self.lof_edits.insert(new, i);
        }
        Ok(())
    }

    /// Copy a level of another pack or folder into this pack (renumbered), list it as `kind`,
    /// and describe it: a `[mapNNNNN]` block (campaigns) or a LOF + LIF entry (map packs).
    ///
    /// All `mapNNNNN.*` files of the source are copied with the new number (map files, scripts
    /// `.txt` / `.lua`, config overrides `mapNNNNN.rules.cfg`, string tables `mapNNNNN.<lang>.dat`,
    /// custom sprites `.zip`, the editor sidecar); a `.lif` is rewritten to the new number and a
    /// `.lof` / `.lif` is not copied into a campaign (the game reads level info from the cfg
    /// there, src/config_campaigns.c:1213). The copy is one transaction: nothing changes when a
    /// file cannot be written. The speech names of the source are not kept (they point into
    /// the source's speech folder).
    pub fn add_level(&mut self, req: &AddLevel) -> Result<Added> {
        let levels_rel = self
            .common()
            .levels_location
            .ok_or_else(|| err("the pack has no LEVELS_LOCATION"))?;
        let dir = self.loc(&levels_rel);
        let src_files = req.source.files();
        if !src_files
            .iter()
            .any(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("slb")))
        {
            return Err(err(format!(
                "level {} has no map file (map{:05}.slb) in {}",
                req.source.number,
                req.source.number,
                req.source.dir.display()
            )));
        }
        let kind = req.kind;
        if self.is_campaign()
            && !matches!(
                kind,
                LevelKind::Single | LevelKind::Bonus | LevelKind::Extra
            )
        {
            return Err(err("a campaign has single, bonus and extra levels"));
        }
        let number = match req.number {
            Some(n) => n,
            None => self.free_number(if kind == LevelKind::Single { 1 } else { 100 }),
        };
        if number == 0 || number > MAX_LEVEL_NUMBER {
            return Err(err(format!(
                "level number {number} is not in 1..={MAX_LEVEL_NUMBER}"
            )));
        }
        if self.entries().iter().any(|e| e.number == number) {
            return Err(err(format!("level {number} is already in the pack")));
        }
        if !req.overwrite && !level_files(&dir, number).is_empty() {
            return Err(err(format!(
                "map{number:05}.* files already exist in {}",
                dir.display()
            )));
        }
        // list capacity (src/config_campaigns.c:293-326)
        let mut slots = self.single_slots();
        let mut extra = self.extra_levels();
        let mut bonus_slot = None;
        if self.is_campaign() {
            match kind {
                LevelKind::Single if slots.len() >= MAX_SINGLE_LEVELS => {
                    return Err(err(format!(
                        "a campaign holds at most {MAX_SINGLE_LEVELS} single levels"
                    )))
                }
                LevelKind::Extra if extra.len() >= MAX_EXTRA_LEVELS => {
                    return Err(err(format!(
                        "a campaign holds at most {MAX_EXTRA_LEVELS} extra levels"
                    )))
                }
                LevelKind::Bonus => {
                    let s = match req.bonus_for {
                        Some(s) => s,
                        None => slots
                            .iter()
                            .position(|s| s.bonus == 0)
                            .ok_or_else(|| err("every single level already has a bonus level"))?,
                    };
                    if s >= slots.len() {
                        return Err(err("single level position out of range"));
                    }
                    if slots[s].bonus != 0 {
                        return Err(err(format!(
                            "single level {} already has bonus level {}",
                            slots[s].level, slots[s].bonus
                        )));
                    }
                    bonus_slot = Some(s);
                }
                _ => {}
            }
        }
        // the level info
        let src_info = req.source.level_info();
        let mut info = LevelInfo {
            name_text: src_info.name_text.clone(),
            name_id: None,
            players: src_info.players,
            ensign: src_info.ensign.clone().filter(|e| !e.is_empty()),
            ensign_pos: src_info.ensign_pos,
            ensign_zoom: src_info.ensign_zoom,
            mapsize: src_info.mapsize,
            ..Default::default()
        };
        if info.mapsize.is_none() {
            if let Some(sz) = real_map_size(&req.source.dir, req.source.number, None) {
                if sz != (DEFAULT_MAP_SIZE, DEFAULT_MAP_SIZE) {
                    info.mapsize = Some(sz);
                }
            }
        }
        if info.name_text.is_none() {
            info.name_text = Some(format!("Level {number}"));
        }
        if self.is_campaign() {
            if info.players.is_none() {
                info.players = Some(1);
            }
            // a land view for single levels: the frame of its position
            if kind == LevelKind::Single {
                let pos = at_index(req.at, slots.len());
                info.land_view = self.default_land_view(pos);
            }
        } else {
            info.kinds = vec![kind];
        }
        // copy the files in one transaction
        let mut tx = SaveTransaction::new(&dir);
        let mut copied = vec![];
        let is_campaign = self.is_campaign();
        let mut wrote_lof = false;
        let mut wrote_lif = false;
        for f in &src_files {
            let fname = f
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if atomic::is_scratch_name(&fname) {
                continue;
            }
            let Some(ext) = level_file_ext(&fname, req.source.number) else {
                continue;
            };
            let dest_name = format!("map{number:05}.{ext}");
            match ext.as_str() {
                "lof" | "lif" if is_campaign => continue,
                "lof" => {
                    let mut lof = Lof::load(f)?;
                    let mut i = lof.info();
                    i.kinds = vec![kind];
                    if i.name_text.is_none() {
                        i.name_text = info.name_text.clone();
                    }
                    lof.set_info(&i);
                    info = i;
                    tx.write(dest_name, lof.to_bytes());
                    wrote_lof = true;
                }
                "lif" => {
                    let lif = Lif::load(f)?;
                    let name = lif
                        .entries()
                        .iter()
                        .find(|e| e.number == req.source.number as i32)
                        .map(|e| e.name.clone());
                    let mut fresh = Lif::default();
                    fresh.set_entry(
                        number as i32,
                        &name.unwrap_or_else(|| info.name_text.clone().unwrap_or_default()),
                    );
                    tx.write(dest_name, fresh.to_bytes());
                    wrote_lif = true;
                }
                _ => {
                    let bytes = std::fs::read(f).map_err(|e| Error::io(f, e))?;
                    tx.write(dest_name, bytes);
                }
            }
        }
        if !is_campaign {
            if !wrote_lof {
                tx.write(format!("map{number:05}.lof"), Lof::new(&info).to_bytes());
            }
            if !wrote_lif {
                let mut lif = Lif::default();
                lif.set_entry(number as i32, info.name_text.as_deref().unwrap_or(""));
                tx.write(format!("map{number:05}.lif"), lif.to_bytes());
            }
        }
        let done = tx.commit()?;
        copied.extend(done.written);
        // lists and description
        if is_campaign {
            match kind {
                LevelKind::Single => {
                    let at = at_index(req.at, slots.len());
                    slots.insert(
                        at,
                        SingleSlot {
                            level: number,
                            bonus: 0,
                        },
                    );
                    self.set_single_slots(&slots);
                }
                LevelKind::Bonus => {
                    slots[bonus_slot.unwrap_or(0)].bonus = number;
                    self.set_single_slots(&slots);
                }
                _ => {
                    let at = at_index(req.at, extra.len());
                    extra.insert(at, number);
                    self.set_extra_levels(&extra);
                }
            }
            self.cfg.set_map_info(number, &info);
        }
        Ok(Added { number, copied })
    }

    /// The `LAND_VIEW` the Nth single level gets by default: `rgmapNN viframeNN` with NN = N + 1
    /// when the land folder has them (the original campaign numbers its views like this), else
    /// the last view that exists.
    pub fn default_land_view(&self, index: usize) -> Option<(String, String)> {
        let dir = self.land_dir()?;
        let views = crate::landview::available_views(&dir);
        let windows = crate::landview::available_windows(&dir);
        if views.is_empty() {
            return None;
        }
        let want = format!("rgmap{:02}", index + 1);
        let view = if views.contains(&want) {
            want
        } else {
            views
                .iter()
                .rev()
                .find(|v| v.as_str() <= want.as_str())
                .cloned()
                .unwrap_or_else(|| views[0].clone())
        };
        let num = view.trim_start_matches(|c: char| !c.is_ascii_digit());
        let win = format!("viframe{num}");
        let win = if windows.contains(&win) {
            win
        } else {
            windows.first().cloned()?
        };
        Some((view, win))
    }
}

fn at_index(at: Option<usize>, len: usize) -> usize {
    at.map_or(len, |a| a.min(len))
}

/// Install-relative path (forward slashes) of `p` when it is inside the install.
pub fn rel_to_install(install: &Path, p: &Path) -> Option<String> {
    let rel = p.strip_prefix(install).ok()?;
    Some(
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

/// Folder of a pack's own files: whether the install-relative `loc` names a folder that belongs
/// to the pack called `stem` (`campgns/<stem>` or `campgns/<stem>_lnd` ...).
pub fn is_own_location(stem: &str, loc: &str) -> bool {
    let loc = loc.replace('\\', "/");
    let mut parts = loc.split('/').filter(|s| !s.is_empty());
    let (Some(_group), Some(name)) = (parts.next(), parts.next()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    let stem = stem.to_ascii_lowercase();
    name == stem || name.starts_with(&format!("{stem}_"))
}

/// Resolve a path below the install, ignoring case; `None` when it does not exist.
pub fn existing(install: &Path, rel: &str) -> Option<PathBuf> {
    resolve_ci(install, &rel.replace('\\', "/"))
}
