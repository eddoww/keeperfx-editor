//! Resolution of sound names, ids, speech, creature sound sets and music like the game does, and
//! decoding of what they resolve to.
//!
//! # What the game does (and this module mirrors)
//!
//! * **`sounds.cfg` layers**: `fxdata/sounds.cfg`, then the mods' copies, the campaign's
//!   (`CONFIGS_LOCATION`), the per-map `levels/mapNNNNN.sounds.cfg` (`load_sounds_config_file`,
//!   `src/config_sounds.c:869`; layering `src/config.c:2321-2360`, campaign load
//!   `src/config_campaigns.c:1338-1350`, level load `src/lvl_filesdk1.c:1488`, `src/dungeon_stats.c:70`).
//!   Blocks read: `[system]`, `[sounds]`, `[speech]`.
//! * **`[sounds]` lines** (`parse_sound_line`, `src/config_sounds.c:520`): `NAME = id [count]`
//!   registers a name for sound.dat ids `id .. id+count-1`; `NAME = file [count]` loads a custom
//!   file (a count expands the trailing digits of the file name, see
//!   [`crate::context::expand_numbered`]); `id = file [count] [ALIAS]` redirects a raw sound.dat id
//!   to a custom file. Names are upper-cased. Tokens are positional: the second token is always
//!   *the count* (`atoi`, a non-number counts as 1), the alias is the third. `STACK=` is accepted
//!   and ignored here (it only changes mixing).
//! * **Custom files win** over numeric registrations of the same name (`getSoundId`,
//!   `sound_manager.cpp:243`), and the *first* custom definition of a name keeps its file
//!   (`loadCustomSound` returns the already loaded entry, `sound_manager.cpp:120`), while numeric
//!   registrations and raw-id redirects are overwritten by later layers. The campaign pass is
//!   replayed before the level pass exactly as the game does (see [`sounds_passes`]).
//! * **File search** for a custom file: `resolve_sounds_cfg_sound_path`
//!   (`sound_manager.cpp:655-700`): for each extension candidate (`.wav .mp3 .ogg .flac` when the
//!   path has none; the extension is the *outer* loop) the folders `mods(after_map)/sound`, levels,
//!   `mods(after_campaign)/sound`, configs, `mods(after_base)/sound`, `fxdata`, media, install
//!   root; mods in reverse load order. Then the map zip `sound/` folder
//!   (`resolve_sound_path_in_map_zip`, `sound_manager.cpp:704`).
//! * **Speech overrides** (`[speech]`): `output_message`, `gui_soundmsgs.cpp:340-370`; file search
//!   `resolve_speech_path`, `gui_soundmsgs.cpp:200-300`.
//! * **Creature sounds**: `[sounds]` block of `creatrs/<creature>.cfg`
//!   (`parse_creaturemodel_sounds_blocks`, `src/config_crtrmodel.c:2395`; layering
//!   `load_creaturemodel_config`, `src/config_crtrmodel.c:2894`).
//! * **Level speech**: `SPEECH = before after` of the level block, played from the campaign's
//!   `[speech]` folder (`play_description_speech`, `src/front_landview.c:836`).
//! * **Music**: numbered tracks map to files in `music/` (`resolve_track_music_path`,
//!   `bflib_sndlib.cpp:798`).
use crate::bank::{self, BankKind, SoundBank};
use crate::codec::{self, Codec};
use crate::context::{
    expand_creature_numbered, expand_numbered, ext_candidates, file_in, read_zip_entry,
    zip_has_entry, Context,
};
use crate::error::{Error, Result};
use crate::pcm::Pcm;
use crate::speech_names::{speech_index, speech_name};
use kfx_formats::cfg::CfgDoc;
use kfx_formats::testutil::resolve_ci;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// Where the sound data of a resolved sound is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A sample of `sound.dat` / `speech_<lang>.dat`.
    Bank { kind: BankKind, index: u16 },
    /// A file on disk (WAV, MP3, OGG, FLAC, `BMU V1.0`).
    File(PathBuf),
    /// An entry of `mapNNNNN.zip`.
    ZipEntry { zip: PathBuf, entry: String },
    /// Explicitly silent (`NONE`, `none`/`null`/`0` speech override).
    Silence,
}

/// Which configuration level supplied a sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerKind {
    /// The sound banks themselves (no configuration involved).
    Bank,
    /// The install: `fxdata/sounds.cfg`, `creatrs/*.cfg`, `music/`, install root files.
    Install,
    ModAfterBase,
    /// The campaign: `CONFIGS_LOCATION` / `CREATURES_LOCATION` / `[speech]` / media folder.
    Campaign,
    ModAfterCampaign,
    /// A per-map file (`mapNNNNN.sounds.cfg`, `mapNNNNN.<creature>.cfg`, `mapNNNNN.zip`).
    Level,
    ModAfterMap,
    /// `mods/<name>/sounds.cfg` loaded by the campaign loader (`load_mod_sounds_config`).
    ModRoot,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Layer {
    pub kind: LayerKind,
    /// Human text: `campaign sounds.cfg`, `mod foo (after_base)`.
    pub label: String,
    /// The file that defined it.
    pub file: Option<PathBuf>,
}

impl Layer {
    fn new(kind: LayerKind, label: impl Into<String>, file: Option<PathBuf>) -> Layer {
        Layer {
            kind,
            label: label.into(),
            file,
        }
    }
    fn bank(label: &str) -> Layer {
        Layer::new(LayerKind::Bank, label, None)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub source: Source,
    /// File name / sample name for display.
    pub label: String,
}

/// A resolved sound: one or more variants (the game picks one at random when `count > 1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub query: String,
    pub variants: Vec<Variant>,
    /// The `count` the configuration declared (the game may have found fewer files).
    pub declared_count: usize,
    pub layer: Layer,
    pub note: Option<String>,
}

impl Resolved {
    pub fn first(&self) -> Option<&Variant> {
        self.variants.first()
    }
}

#[derive(Clone, Debug)]
struct NameEntry {
    id: u16,
    count: usize,
    layer: Layer,
}

#[derive(Clone, Debug)]
struct CustomEntry {
    variants: Vec<Variant>,
    declared: usize,
    layer: Layer,
}

/// The sound a creature makes (`CrSnd_*`, `creatmodel_sounds_commands`, `src/config_crtrmodel.c:241`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CreatureSound {
    Foot,
    Hit,
    Happy,
    Sad,
    Die,
    Hang,
    Drop,
    Torture,
    Slap,
    Fight,
    Piss,
}

impl CreatureSound {
    pub const ALL: [CreatureSound; 11] = [
        CreatureSound::Foot,
        CreatureSound::Hit,
        CreatureSound::Happy,
        CreatureSound::Sad,
        CreatureSound::Die,
        CreatureSound::Hang,
        CreatureSound::Drop,
        CreatureSound::Torture,
        CreatureSound::Slap,
        CreatureSound::Fight,
        CreatureSound::Piss,
    ];
    pub fn key(self) -> &'static str {
        match self {
            CreatureSound::Foot => "Foot",
            CreatureSound::Hit => "Hit",
            CreatureSound::Happy => "Happy",
            CreatureSound::Sad => "Sad",
            CreatureSound::Die => "Die",
            CreatureSound::Hang => "Hang",
            CreatureSound::Drop => "Drop",
            CreatureSound::Torture => "Torture",
            CreatureSound::Slap => "Slap",
            CreatureSound::Fight => "Fight",
            CreatureSound::Piss => "Piss",
        }
    }
    pub fn from_key(k: &str) -> Option<CreatureSound> {
        Self::ALL
            .into_iter()
            .find(|s| s.key().eq_ignore_ascii_case(k))
    }
}

/// All the sounds of one creature.
#[derive(Clone, Debug)]
pub struct CreatureSet {
    pub name: String,
    pub slots: Vec<(CreatureSound, Resolved)>,
}

/// A numbered music track and its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicTrack {
    pub number: u32,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Effects,
    Speech,
    Creatures,
    Music,
}

impl Category {
    pub const ALL: [Category; 4] = [
        Category::Effects,
        Category::Speech,
        Category::Creatures,
        Category::Music,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Category::Effects => "Effects",
            Category::Speech => "Speech",
            Category::Creatures => "Creature sets",
            Category::Music => "Music",
        }
    }
}

/// One row of the sound browser.
#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub category: Category,
    /// Unique and stable.
    pub key: String,
    pub title: String,
    /// Aliases / slot / extra text.
    pub detail: String,
    pub resolved: Resolved,
    /// Known without decoding (bank samples only).
    pub duration: Option<f64>,
}

/// A numbered or named sound line, parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SoundLine {
    key: String,
    raw_id: Option<u32>,
    value: String,
    count: usize,
    alias: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    Sound(SoundLine),
    Speech { msg: u16, path: String },
}

fn is_stack_token(t: &str) -> bool {
    t.len() >= 6 && t[..6].eq_ignore_ascii_case("STACK=")
}

fn atoi(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let n: i64 = rest
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .fold(0i64, |a, c| {
            a.saturating_mul(10).saturating_add(c as i64 - '0' as i64)
        });
    if neg {
        -n
    } else {
        n
    }
}

/// `strtol` with the `*end == 0 && value > 0` test of the game: the whole token is a positive
/// number.
fn pos_int(s: &str) -> Option<u32> {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse::<u64>()
            .ok()
            .filter(|&n| n > 0)
            .map(|n| n.min(u32::MAX as u64) as u32)
    } else {
        None
    }
}

/// Parse the `[sounds]` and `[speech]` blocks of one `sounds.cfg`.
fn parse_sounds_cfg(doc: &CfgDoc) -> Vec<Op> {
    let mut ops = vec![];
    for e in doc.entries("sounds") {
        let key = e.key.to_ascii_uppercase();
        if e.tokens.is_empty() {
            continue; // "Sound 'X' has no value"
        }
        let value = e.tokens[0].clone();
        let count_tok = e.tokens.get(1);
        let count = count_tok.map(|t| atoi(t)).filter(|&n| n > 0).unwrap_or(1) as usize;
        let raw_id = pos_int(&key);
        let mut alias = None;
        if raw_id.is_some() {
            if pos_int(&value).is_some() || value.bytes().all(|b| b.is_ascii_digit()) {
                continue; // a raw id redirect needs a file path
            }
            if let Some(t) = e.tokens.get(2) {
                let c = t.chars().next().unwrap_or(' ');
                if !is_stack_token(t) && (c.is_ascii_alphabetic() || c == '_') {
                    alias = Some(t.to_ascii_uppercase());
                }
            }
        }
        ops.push(Op::Sound(SoundLine {
            key,
            raw_id,
            value,
            count,
            alias,
        }));
    }
    for e in doc.entries("speech") {
        let Some(msg) = speech_index(&e.key) else {
            continue;
        };
        if msg == 0 {
            continue;
        }
        let Some(path) = e.tokens.first() else {
            continue;
        };
        let path = if path == "=" {
            e.tokens.get(1).cloned().unwrap_or_default()
        } else {
            path.clone()
        };
        if path.is_empty() {
            continue;
        }
        ops.push(Op::Speech { msg, path });
    }
    ops
}

/// The sequence of `sounds.cfg` files the game applies, in order: `(layer, file)`.
///
/// The campaign loader (`src/config_campaigns.c:1338`) clears everything, then loads fxdata, the
/// after-base mods' copies, the campaign `sounds.cfg`, the after-campaign mods, then (second loop)
/// the mods' root-level `sounds.cfg` and the campaign file again. At level start (`src/main_game.c:203`
/// restores that state, `src/dungeon_stats.c:70` reloads the generic chain, `src/lvl_filesdk1.c:1488` loads
/// the map file once more) the generic chain runs again with the map file and the after-map mods.
/// The returned list is that whole sequence.
pub fn sounds_passes(ctx: &Context) -> Vec<(Layer, PathBuf)> {
    let mut out: Vec<(Layer, PathBuf)> = vec![];
    let level = ctx.level;
    let lvfile = |n: u32| format!("map{n:05}.sounds.cfg");
    let generic = |out: &mut Vec<(Layer, PathBuf)>, with_level: bool| {
        if let Some(p) = ctx.install_file("fxdata/sounds.cfg") {
            out.push((
                Layer::new(LayerKind::Install, "fxdata/sounds.cfg", Some(p.clone())),
                p,
            ));
        }
        let mod_tier = |out: &mut Vec<(Layer, PathBuf)>,
                        tier: &[String],
                        kind: LayerKind,
                        tname: &str| {
            for m in tier {
                let mut cands: Vec<PathBuf> = vec![];
                cands.extend(ctx.mod_file(m, "fxdata", "sounds.cfg"));
                if let Some(rel) = &ctx.campaign.configs_rel {
                    cands.extend(ctx.mod_file(m, rel, "sounds.cfg"));
                }
                if let (true, Some(n), Some(rel)) = (with_level, level, &ctx.campaign.levels_rel) {
                    cands.extend(ctx.mod_file(m, rel, &lvfile(n)));
                }
                for p in cands {
                    out.push((
                        Layer::new(kind, format!("mod {m} ({tname})"), Some(p.clone())),
                        p,
                    ));
                }
            }
        };
        mod_tier(
            out,
            &ctx.mods.after_base,
            LayerKind::ModAfterBase,
            "after_base",
        );
        if let Some(p) = file_in(&ctx.campaign.configs, "sounds.cfg") {
            out.push((
                Layer::new(LayerKind::Campaign, "campaign sounds.cfg", Some(p.clone())),
                p,
            ));
        }
        mod_tier(
            out,
            &ctx.mods.after_campaign,
            LayerKind::ModAfterCampaign,
            "after_campaign",
        );
        if with_level {
            if let Some(p) = level.and_then(|n| file_in(&ctx.campaign.levels, &lvfile(n))) {
                out.push((
                    Layer::new(
                        LayerKind::Level,
                        format!("level {}", level.unwrap_or(0)),
                        Some(p.clone()),
                    ),
                    p,
                ));
            }
            mod_tier(
                out,
                &ctx.mods.after_map,
                LayerKind::ModAfterMap,
                "after_map",
            );
        }
    };
    // campaign load
    generic(&mut out, false);
    let root = |out: &mut Vec<(Layer, PathBuf)>, tier: &[String], tname: &str| {
        for m in tier {
            if let Some(p) = ctx.mod_file(m, "", "sounds.cfg") {
                out.push((
                    Layer::new(
                        LayerKind::ModRoot,
                        format!("mod {m} root ({tname})"),
                        Some(p.clone()),
                    ),
                    p,
                ));
            }
        }
    };
    root(&mut out, &ctx.mods.after_base, "after_base");
    if let Some(p) = file_in(&ctx.campaign.configs, "sounds.cfg") {
        out.push((
            Layer::new(LayerKind::Campaign, "campaign sounds.cfg", Some(p.clone())),
            p,
        ));
    }
    root(&mut out, &ctx.mods.after_campaign, "after_campaign");
    // level load
    if level.is_some() {
        generic(&mut out, true);
        if let Some(p) = level.and_then(|n| file_in(&ctx.campaign.levels, &lvfile(n))) {
            out.push((
                Layer::new(
                    LayerKind::Level,
                    format!("level {}", level.unwrap_or(0)),
                    Some(p.clone()),
                ),
                p,
            ));
        }
    }
    out
}

/// A sound library for one [`Context`]: the banks, the layered registry and the file search.
pub struct Audio {
    pub ctx: Context,
    effects: Option<SoundBank>,
    speech: Option<SoundBank>,
    names: HashMap<String, NameEntry>,
    custom: HashMap<String, CustomEntry>,
    /// raw sound.dat id -> replacement.
    redirects: BTreeMap<u16, (Variant, Layer)>,
    speech_overrides: HashMap<u16, (String, Layer)>,
    /// Problems met while loading the layers (missing files, bad lines).
    pub warnings: Vec<String>,
}

impl Audio {
    /// Open the banks of the install and apply every `sounds.cfg` layer.
    pub fn open(ctx: Context) -> Audio {
        let (effects, speech) = bank::open_banks(&ctx.install, &ctx.language);
        Self::with_banks(ctx, effects, speech)
    }

    /// Like [`Audio::open`] with banks the caller already has (tests).
    pub fn with_banks(
        ctx: Context,
        effects: Option<SoundBank>,
        speech: Option<SoundBank>,
    ) -> Audio {
        let mut a = Audio {
            ctx,
            effects,
            speech,
            names: HashMap::new(),
            custom: HashMap::new(),
            redirects: BTreeMap::new(),
            speech_overrides: HashMap::new(),
            warnings: vec![],
        };
        a.load_layers();
        a
    }

    pub fn effects_bank(&self) -> Option<&SoundBank> {
        self.effects.as_ref()
    }

    pub fn speech_bank(&self) -> Option<&SoundBank> {
        self.speech.as_ref()
    }

    fn load_layers(&mut self) {
        for (layer, file) in sounds_passes(&self.ctx) {
            let doc = match CfgDoc::load(&file) {
                Ok(d) => d,
                Err(e) => {
                    self.warnings.push(format!("{}: {e}", file.display()));
                    continue;
                }
            };
            for op in parse_sounds_cfg(&doc) {
                self.apply(op, &layer);
            }
        }
    }

    fn apply(&mut self, op: Op, layer: &Layer) {
        match op {
            Op::Speech { msg, path } => {
                self.speech_overrides.insert(msg, (path, layer.clone()));
            }
            Op::Sound(l) => match l.raw_id {
                Some(raw) => {
                    let internal = l.alias.clone().unwrap_or_else(|| format!("__RAW_{raw}"));
                    let Some(entry) = self.load_named(&internal, &l.value, l.count, layer) else {
                        self.warnings.push(format!(
                            "raw id {raw}: file '{}' not found ({})",
                            l.value, layer.label
                        ));
                        return;
                    };
                    for i in 0..l.count {
                        if let Some(v) = entry.variants.get(i) {
                            self.redirects.insert(
                                (raw as usize + i).min(u16::MAX as usize) as u16,
                                (v.clone(), layer.clone()),
                            );
                        }
                    }
                }
                None => match pos_int(&l.value) {
                    Some(id) => {
                        self.names.insert(
                            l.key,
                            NameEntry {
                                id: id.min(u16::MAX as u32) as u16,
                                count: l.count,
                                layer: layer.clone(),
                            },
                        );
                    }
                    None => {
                        if self.load_named(&l.key, &l.value, l.count, layer).is_none() {
                            self.warnings.push(format!(
                                "sound {}: file '{}' not found ({})",
                                l.key, l.value, layer.label
                            ));
                        }
                    }
                },
            },
        }
    }

    /// `sound_manager_load_named_sound` (`sound_manager.cpp:760`): the first definition of a name
    /// wins.
    fn load_named(
        &mut self,
        name: &str,
        path: &str,
        count: usize,
        layer: &Layer,
    ) -> Option<CustomEntry> {
        if let Some(e) = self.custom.get(name) {
            return Some(e.clone());
        }
        let mut variants = vec![];
        if count <= 1 {
            variants.extend(self.find_sound_file(path, false));
        } else {
            for p in expand_numbered(path, count) {
                variants.extend(self.find_sound_file(&p, false));
            }
        }
        if variants.is_empty() {
            return None;
        }
        let e = CustomEntry {
            variants,
            declared: count.max(1),
            layer: layer.clone(),
        };
        self.custom.insert(name.to_string(), e.clone());
        Some(e)
    }

    /// Folders searched for custom sound files, in game order.
    fn search_dirs(&self, creature: bool) -> Vec<(PathBuf, Layer)> {
        let ctx = &self.ctx;
        let mut dirs: Vec<(PathBuf, Layer)> = vec![];
        let mod_dirs =
            |dirs: &mut Vec<(PathBuf, Layer)>, tier: &[String], kind: LayerKind, tname: &str| {
                for m in tier.iter().rev() {
                    if let Some(d) = ctx
                        .mod_dir(m)
                        .and_then(|b| resolve_ci(&b, "sound"))
                        .filter(|p| p.is_dir())
                    {
                        dirs.push((
                            d,
                            Layer::new(kind, format!("mod {m} sound/ ({tname})"), None),
                        ));
                    }
                }
            };
        let plain = |dirs: &mut Vec<(PathBuf, Layer)>,
                     d: &Option<PathBuf>,
                     kind: LayerKind,
                     label: &str| {
            if let Some(d) = d {
                dirs.push((d.clone(), Layer::new(kind, label, None)));
            }
        };
        let install_sub = |s: &str| resolve_ci(&ctx.install, s).filter(|p| p.is_dir());
        mod_dirs(
            &mut dirs,
            &ctx.mods.after_map,
            LayerKind::ModAfterMap,
            "after_map",
        );
        plain(
            &mut dirs,
            &ctx.campaign.levels,
            LayerKind::Level,
            "campaign levels folder",
        );
        if creature {
            plain(
                &mut dirs,
                &ctx.campaign.creatures,
                LayerKind::Campaign,
                "campaign creatures folder",
            );
        }
        mod_dirs(
            &mut dirs,
            &ctx.mods.after_campaign,
            LayerKind::ModAfterCampaign,
            "after_campaign",
        );
        plain(
            &mut dirs,
            &ctx.campaign.configs,
            LayerKind::Campaign,
            "campaign configs folder",
        );
        if creature {
            plain(
                &mut dirs,
                &install_sub("creatrs"),
                LayerKind::Install,
                "creatrs/",
            );
        }
        mod_dirs(
            &mut dirs,
            &ctx.mods.after_base,
            LayerKind::ModAfterBase,
            "after_base",
        );
        plain(
            &mut dirs,
            &install_sub("fxdata"),
            LayerKind::Install,
            "fxdata/",
        );
        plain(
            &mut dirs,
            &ctx.campaign.media,
            LayerKind::Campaign,
            "campaign media folder",
        );
        dirs.push((
            ctx.install.clone(),
            Layer::new(LayerKind::Install, "install root", None),
        ));
        dirs
    }

    /// Find a custom sound file (file system first, then the map zip).
    pub fn find_sound_file(&self, path: &str, creature: bool) -> Option<Variant> {
        let path = path.replace('\\', "/");
        let dirs = self.search_dirs(creature);
        for cand in ext_candidates(&path) {
            for (d, _) in &dirs {
                if let Some(f) = resolve_ci(d, &cand).filter(|p| p.is_file()) {
                    let label = f
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    return Some(Variant {
                        source: Source::File(f),
                        label,
                    });
                }
            }
        }
        // `resolve_sound_path_in_map_zip`: the "sound/" root is mandatory, the game strips it from
        // the cfg path
        let zip = self.ctx.map_zip()?;
        let rel = if path.len() >= 6 && path[..6].eq_ignore_ascii_case("sound/") {
            &path[6..]
        } else {
            &path[..]
        };
        for cand in ext_candidates(rel) {
            let entry = format!("sound/{cand}");
            if zip_has_entry(&zip, &entry) {
                return Some(Variant {
                    label: cand.clone(),
                    source: Source::ZipEntry { zip, entry },
                });
            }
        }
        None
    }

    fn bank_variant(&self, kind: BankKind, index: u16) -> Variant {
        let bank = match kind {
            BankKind::Effects => self.effects.as_ref(),
            BankKind::Speech => self.speech.as_ref(),
        };
        let label = bank
            .and_then(|b| b.get(index as usize))
            .map(|s| s.name.clone())
            .unwrap_or_default();
        Variant {
            source: Source::Bank { kind, index },
            label,
        }
    }

    fn bank_len(&self, kind: BankKind) -> usize {
        match kind {
            BankKind::Effects => self.effects.as_ref().map(|b| b.len()).unwrap_or(0),
            BankKind::Speech => self.speech.as_ref().map(|b| b.len()).unwrap_or(0),
        }
    }

    /// Resolve a sound by name (`GOLD_PICKUP`, case-insensitive), numeric sound.dat id (`36`) or
    /// bank file name (`foot1a.wav`).
    pub fn resolve(&self, name_or_id: &str) -> Result<Resolved> {
        let q = name_or_id.trim();
        if q.is_empty() {
            return Err(Error::NotFound("empty sound name".into()));
        }
        if let Some(id) = pos_int(q) {
            return self.resolve_id(id);
        }
        let up = q.to_ascii_uppercase();
        // custom files take priority over numeric names (`getSoundId`)
        if let Some(c) = self.custom.get(&up) {
            return Ok(Resolved {
                query: q.into(),
                variants: c.variants.clone(),
                declared_count: c.declared,
                layer: c.layer.clone(),
                note: None,
            });
        }
        if let Some(n) = self.names.get(&up) {
            let mut variants = vec![];
            for i in 0..n.count {
                let id = n.id as usize + i;
                if id < self.bank_len(BankKind::Effects) {
                    // a raw-id redirect replaces the sample
                    variants.push(match self.redirects.get(&(id as u16)) {
                        Some((v, _)) => v.clone(),
                        None => self.bank_variant(BankKind::Effects, id as u16),
                    });
                }
            }
            if variants.is_empty() {
                return Err(Error::NotFound(format!(
                    "{q}: id {} is outside sound.dat ({} samples)",
                    n.id,
                    self.bank_len(BankKind::Effects)
                )));
            }
            return Ok(Resolved {
                query: q.into(),
                variants,
                declared_count: n.count,
                layer: n.layer.clone(),
                note: None,
            });
        }
        if let Some(i) = self.effects.as_ref().and_then(|b| b.find(q)) {
            return Ok(Resolved {
                query: q.into(),
                variants: vec![self.bank_variant(BankKind::Effects, i as u16)],
                declared_count: 1,
                layer: Layer::bank("sound.dat"),
                note: Some("bank file name".into()),
            });
        }
        Err(Error::NotFound(format!("sound '{q}'")))
    }

    /// Resolve a raw sound.dat id (applies raw-id redirects).
    pub fn resolve_id(&self, id: u32) -> Result<Resolved> {
        let n = self.bank_len(BankKind::Effects);
        if id == 0 || id as usize >= n.max(1) && self.effects.is_some() {
            return Err(Error::NotFound(format!(
                "sound id {id} (sound.dat has {n} samples)"
            )));
        }
        let id = id as u16;
        if let Some((v, layer)) = self.redirects.get(&id) {
            return Ok(Resolved {
                query: id.to_string(),
                variants: vec![v.clone()],
                declared_count: 1,
                layer: layer.clone(),
                note: Some(format!("redirect of sound.dat id {id}")),
            });
        }
        Ok(Resolved {
            query: id.to_string(),
            variants: vec![self.bank_variant(BankKind::Effects, id)],
            declared_count: 1,
            layer: Layer::bank("sound.dat"),
            note: None,
        })
    }

    /// Speech message by config name (`GoldLow`) or bank index; honours `[speech]` overrides.
    pub fn resolve_speech(&self, name_or_index: &str) -> Result<Resolved> {
        let q = name_or_index.trim();
        let msg = match pos_int(q).or(if q == "0" { Some(0) } else { None }) {
            Some(i) => i as u16,
            None => {
                speech_index(q).ok_or_else(|| Error::NotFound(format!("speech message '{q}'")))?
            }
        };
        if let Some((path, layer)) = self.speech_overrides.get(&msg) {
            let silent = path.eq_ignore_ascii_case("none")
                || path.eq_ignore_ascii_case("null")
                || path == "0";
            if silent {
                return Ok(Resolved {
                    query: q.into(),
                    variants: vec![Variant {
                        source: Source::Silence,
                        label: path.clone(),
                    }],
                    declared_count: 1,
                    layer: layer.clone(),
                    note: Some("silenced by [speech] override".into()),
                });
            }
            return Ok(match self.find_speech_file(path) {
                Some((v, how)) => Resolved {
                    query: q.into(),
                    variants: vec![v],
                    declared_count: 1,
                    layer: layer.clone(),
                    note: Some(format!("[speech] override, {how}")),
                },
                None => Resolved {
                    query: q.into(),
                    variants: vec![],
                    declared_count: 1,
                    layer: layer.clone(),
                    note: Some(format!(
                        "[speech] override '{path}' not found (the game then plays nothing)"
                    )),
                },
            });
        }
        if self.speech.is_some() && msg as usize >= self.bank_len(BankKind::Speech) {
            return Err(Error::NotFound(format!("speech index {msg}")));
        }
        Ok(Resolved {
            query: q.into(),
            variants: vec![self.bank_variant(BankKind::Speech, msg)],
            declared_count: 1,
            layer: Layer::bank("speech bank"),
            note: speech_name(msg).map(|n| n.to_string()),
        })
    }

    /// `resolve_speech_path` (`gui_soundmsgs.cpp:205`): language folder, English, base, any
    /// language folder, then the map zip. Returns the file and how it was found.
    pub fn find_speech_file(&self, path: &str) -> Option<(Variant, &'static str)> {
        let path = path.replace('\\', "/");
        let ctx = &self.ctx;
        let groups: Vec<PathBuf> = [
            &ctx.campaign.configs,
            &ctx.campaign.levels,
            &ctx.campaign.media,
        ]
        .into_iter()
        .flatten()
        .cloned()
        .chain(std::iter::once(ctx.install.clone()))
        .collect();
        let (dir_part, file) = match path.rfind('/') {
            Some(s) => (Some(&path[..s]), &path[s + 1..]),
            None => (None, &path[..]),
        };
        let with_lang = |lang: &str| match dir_part {
            Some(d) => format!("{d}/{lang}/{file}"),
            None => format!("{lang}/{path}"),
        };
        let mk = |p: PathBuf| Variant {
            label: p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            source: Source::File(p),
        };
        let lang = ctx.language.as_str();
        if !lang.is_empty() {
            let cand = with_lang(lang);
            for g in &groups {
                if let Some(f) = resolve_ci(g, &cand).filter(|p| p.is_file()) {
                    return Some((mk(f), "language folder"));
                }
            }
        }
        if lang != "eng" {
            let cand = with_lang("eng");
            for g in &groups {
                if let Some(f) = resolve_ci(g, &cand).filter(|p| p.is_file()) {
                    return Some((mk(f), "English fallback"));
                }
            }
        }
        for g in &groups {
            if let Some(f) = resolve_ci(g, &path).filter(|p| p.is_file()) {
                return Some((mk(f), "base path"));
            }
        }
        for g in &groups {
            let base = match dir_part {
                Some(d) => resolve_ci(g, d),
                None => Some(g.clone()),
            };
            let Some(base) = base.filter(|p| p.is_dir()) else {
                continue;
            };
            let mut subs: Vec<PathBuf> = std::fs::read_dir(&base)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            subs.sort();
            for s in subs {
                if let Some(f) = resolve_ci(&s, file).filter(|p| p.is_file()) {
                    return Some((mk(f), "first available language folder"));
                }
            }
        }
        if let Some(zip) = ctx.map_zip() {
            let mut cands = vec![];
            if !lang.is_empty() {
                cands.push(format!("speech/{lang}/{file}"));
            }
            if lang != "eng" {
                cands.push(format!("speech/eng/{file}"));
            }
            cands.push(format!("speech/{file}"));
            for entry in cands {
                if zip_has_entry(&zip, &entry) {
                    return Some((
                        Variant {
                            label: file.to_string(),
                            source: Source::ZipEntry {
                                zip: zip.clone(),
                                entry,
                            },
                        },
                        "map zip",
                    ));
                }
            }
        }
        None
    }

    /// The `SPEECH = before after` pair of a level (campaign `[mapNNNNN]` block, else the level's
    /// `.lof`) and the resolved file in the campaign `[speech]` folder (`play_description_speech`,
    /// `src/front_landview.c:836`: `.wav` is appended when there is no extension; the folder is already
    /// language specific).
    pub fn resolve_level_speech(&self, level: u32, before: bool) -> Result<Resolved> {
        let pair = self
            .level_speech_names(level)
            .ok_or_else(|| Error::NotFound(format!("SPEECH of level {level}")))?;
        let name = if before { pair.0 } else { pair.1 };
        if name.is_empty() {
            return Err(Error::NotFound(format!(
                "level {level} has no {} speech",
                if before { "before" } else { "after" }
            )));
        }
        let file = if name.contains('.') {
            name.clone()
        } else {
            format!("{name}.wav")
        };
        let query = format!("level {level} {}", if before { "before" } else { "after" });
        let layer = Layer::new(
            LayerKind::Campaign,
            "campaign SPEECH",
            self.ctx.campaign.cfg.clone(),
        );
        match file_in(&self.ctx.campaign.speech, &file) {
            Some(p) => Ok(Resolved {
                query,
                variants: vec![Variant {
                    label: file,
                    source: Source::File(p),
                }],
                declared_count: 1,
                layer,
                note: None,
            }),
            None => Ok(Resolved {
                query,
                variants: vec![],
                declared_count: 1,
                layer,
                note: Some(format!("'{file}' not found in the campaign speech folder")),
            }),
        }
    }

    fn level_speech_names(&self, level: u32) -> Option<(String, String)> {
        use kfx_formats::campaign::Campaign;
        use kfx_formats::levelinfo::Lof;
        if let Some(cfg) = &self.ctx.campaign.cfg {
            if let Some(sp) = Campaign::load(cfg)
                .ok()
                .and_then(|c| c.map_info(level))
                .and_then(|i| i.speech)
            {
                return Some(sp);
            }
        }
        let lof = file_in(&self.ctx.campaign.levels, &format!("map{level:05}.lof"))?;
        Lof::load(&lof).ok()?.info().speech
    }

    /// Levels of the campaign that declare speech (for the browser).
    fn levels_with_speech(&self) -> Vec<u32> {
        use kfx_formats::campaign::resolve;
        let Some(cfg) = &self.ctx.campaign.cfg else {
            return vec![];
        };
        let Ok(r) = resolve(&self.ctx.install, cfg) else {
            return vec![];
        };
        r.levels
            .iter()
            .map(|l| l.number)
            .filter(|&n| self.level_speech_names(n).is_some())
            .collect()
    }

    // ---- creatures --------------------------------------------------------------------------

    /// Config files of one creature in game order (`load_creaturemodel_config`).
    fn creature_files(&self, name: &str) -> Vec<(Layer, PathBuf)> {
        let ctx = &self.ctx;
        let lower = name.to_ascii_lowercase();
        let fname = format!("{lower}.cfg");
        let mut out = vec![];
        let mut push = |kind: LayerKind, label: String, p: Option<PathBuf>| {
            if let Some(p) = p {
                out.push((Layer::new(kind, label, Some(p.clone())), p));
            }
        };
        push(
            LayerKind::Install,
            "creatrs/".into(),
            resolve_ci(&ctx.install, &format!("creatrs/{fname}")).filter(|p| p.is_file()),
        );
        for m in &ctx.mods.after_base {
            push(
                LayerKind::ModAfterBase,
                format!("mod {m} (after_base)"),
                ctx.mod_file(m, "creatrs", &fname),
            );
            if let Some(rel) = &ctx.campaign.creatures_rel {
                push(
                    LayerKind::ModAfterBase,
                    format!("mod {m} (after_base)"),
                    ctx.mod_file(m, rel, &fname),
                );
            }
        }
        push(
            LayerKind::Campaign,
            "campaign creatures".into(),
            file_in(&ctx.campaign.creatures, &fname),
        );
        for m in &ctx.mods.after_campaign {
            push(
                LayerKind::ModAfterCampaign,
                format!("mod {m} (after_campaign)"),
                ctx.mod_file(m, "creatrs", &fname),
            );
            if let Some(rel) = &ctx.campaign.creatures_rel {
                push(
                    LayerKind::ModAfterCampaign,
                    format!("mod {m} (after_campaign)"),
                    ctx.mod_file(m, rel, &fname),
                );
            }
        }
        if let Some(n) = ctx.level {
            let lv = format!("map{n:05}.{fname}");
            push(
                LayerKind::Level,
                format!("level {n}"),
                file_in(&ctx.campaign.levels, &lv),
            );
            for m in &ctx.mods.after_map {
                push(
                    LayerKind::ModAfterMap,
                    format!("mod {m} (after_map)"),
                    ctx.mod_file(m, "creatrs", &fname),
                );
            }
        }
        out
    }

    /// Names of the creatures that have a config file in any layer.
    pub fn creature_names(&self) -> Vec<String> {
        let mut set = std::collections::BTreeSet::new();
        let ctx = &self.ctx;
        let mut scan = |dir: Option<PathBuf>| {
            if let Some(d) = dir {
                for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                    let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                    if let Some(stem) = n.strip_suffix(".cfg") {
                        if !stem.starts_with("map") || stem.len() < 8 {
                            set.insert(stem.to_string());
                        }
                    }
                }
            }
        };
        scan(resolve_ci(&ctx.install, "creatrs").filter(|p| p.is_dir()));
        scan(ctx.campaign.creatures.clone());
        for m in ctx
            .mods
            .after_base
            .iter()
            .chain(&ctx.mods.after_campaign)
            .chain(&ctx.mods.after_map)
        {
            scan(
                ctx.mod_dir(m)
                    .and_then(|b| resolve_ci(&b, "creatrs"))
                    .filter(|p| p.is_dir()),
            );
        }
        set.into_iter()
            .filter(|n| !self.creature_files(n).is_empty())
            .collect()
    }

    /// The sound set of a creature (`imp`, case-insensitive) after all layers. Slots the
    /// configuration never mentions are absent (the game leaves them at 0 = silent).
    pub fn creature_set(&self, name: &str) -> CreatureSet {
        let mut slots: BTreeMap<CreatureSound, Resolved> = BTreeMap::new();
        for (layer, file) in self.creature_files(name) {
            let Ok(doc) = CfgDoc::load(&file) else {
                continue;
            };
            for e in doc.entries("sounds") {
                let Some(slot) = CreatureSound::from_key(&e.key) else {
                    continue;
                };
                let Some(first) = e.tokens.first() else {
                    continue;
                };
                let query = format!("{name}.{}", slot.key());
                let resolved = if is_sound_file_path(first) {
                    let count = e.tokens.get(1).map(|t| atoi(t)).unwrap_or(1).max(1) as usize;
                    let mut variants = vec![];
                    for p in expand_creature_numbered(first, count) {
                        variants.extend(self.find_sound_file(&p, true));
                    }
                    let note = if variants.is_empty() {
                        Some(format!("file '{first}' not found"))
                    } else {
                        None
                    };
                    Resolved {
                        query,
                        variants,
                        declared_count: count,
                        layer: layer.clone(),
                        note,
                    }
                } else if first.eq_ignore_ascii_case("none") {
                    Resolved {
                        query,
                        variants: vec![Variant {
                            source: Source::Silence,
                            label: "NONE".into(),
                        }],
                        declared_count: 1,
                        layer: layer.clone(),
                        note: Some("NONE".into()),
                    }
                } else {
                    let idx = atoi(first);
                    let count = e.tokens.get(1).map(|t| atoi(t)).unwrap_or(1).max(1) as usize;
                    if idx <= 0 {
                        Resolved {
                            query,
                            variants: vec![Variant {
                                source: Source::Silence,
                                label: "0".into(),
                            }],
                            declared_count: 1,
                            layer: layer.clone(),
                            note: Some("index 0 is silent".into()),
                        }
                    } else {
                        let mut variants = vec![];
                        for i in 0..count {
                            let id = (idx as usize + i).min(u16::MAX as usize) as u16;
                            if (id as usize) < self.bank_len(BankKind::Effects)
                                || self.effects.is_none()
                            {
                                variants.push(match self.redirects.get(&id) {
                                    Some((v, _)) => v.clone(),
                                    None => self.bank_variant(BankKind::Effects, id),
                                });
                            }
                        }
                        Resolved {
                            query,
                            variants,
                            declared_count: count,
                            layer: layer.clone(),
                            note: None,
                        }
                    }
                };
                slots.insert(slot, resolved);
            }
        }
        CreatureSet {
            name: name.to_ascii_lowercase(),
            slots: slots.into_iter().collect(),
        }
    }

    // ---- music ------------------------------------------------------------------------------

    /// The files that numbered tracks map to: the `music/` files of the best extension present
    /// (`.flac`, `.wav`, `.ogg`, `.mp3` in that order of preference), sorted by name; track numbers
    /// start at 2 (`resolve_track_music_path`, `bflib_sndlib.cpp:798`; the game takes the directory
    /// order of the file system, sorting by name is the same on the usual layouts).
    pub fn music_tracks(&self) -> Vec<MusicTrack> {
        let Some(dir) = resolve_ci(&self.ctx.install, "music").filter(|p| p.is_dir()) else {
            return vec![];
        };
        const EXTS: [&str; 4] = ["flac", "wav", "ogg", "mp3"];
        let mut files: Vec<(usize, String, PathBuf)> = vec![];
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            let Some(ext) = p
                .extension()
                .map(|x| x.to_string_lossy().to_ascii_lowercase())
            else {
                continue;
            };
            if let Some(rank) = EXTS.iter().position(|x| *x == ext) {
                files.push((rank, e.file_name().to_string_lossy().to_string(), p));
            }
        }
        let Some(best) = files.iter().map(|f| f.0).min() else {
            return vec![];
        };
        let mut v: Vec<(String, PathBuf)> = files
            .into_iter()
            .filter(|f| f.0 == best)
            .map(|f| (f.1, f.2))
            .collect();
        v.sort_by_key(|a| a.0.to_ascii_lowercase());
        v.into_iter()
            .enumerate()
            .map(|(i, (_, p))| MusicTrack {
                number: i as u32 + 2,
                path: p,
            })
            .collect()
    }

    /// A numbered music track.
    pub fn resolve_music_track(&self, track: u32) -> Result<Resolved> {
        let t = self
            .music_tracks()
            .into_iter()
            .find(|t| t.number == track)
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "music track {track} (tracks start at 2, found {} files)",
                    self.music_tracks().len()
                ))
            })?;
        let label = t
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(Resolved {
            query: format!("track {track}"),
            variants: vec![Variant {
                source: Source::File(t.path),
                label,
            }],
            declared_count: 1,
            layer: Layer::new(LayerKind::Install, "music/", None),
            note: None,
        })
    }

    /// A custom music file as the script / `music.set` console command name it: relative to the
    /// campaign media folder, the install root, then `music/`.
    pub fn resolve_music_file(&self, path: &str) -> Result<Resolved> {
        let path = path.replace('\\', "/");
        let music = resolve_ci(&self.ctx.install, "music").filter(|p| p.is_dir());
        let mut where_: Vec<(Option<PathBuf>, Layer)> = vec![
            (
                self.ctx.campaign.media.clone(),
                Layer::new(LayerKind::Campaign, "campaign media folder", None),
            ),
            (
                Some(self.ctx.install.clone()),
                Layer::new(LayerKind::Install, "install root", None),
            ),
            (music, Layer::new(LayerKind::Install, "music/", None)),
        ];
        for (dir, layer) in where_.drain(..) {
            if let Some(f) = file_in(&dir, &path) {
                let label = f
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                return Ok(Resolved {
                    query: path,
                    variants: vec![Variant {
                        source: Source::File(f),
                        label,
                    }],
                    declared_count: 1,
                    layer,
                    note: None,
                });
            }
        }
        Err(Error::NotFound(format!("music file '{path}'")))
    }

    /// The campaign's `SOUNDTRACK` (`src/front_landview.c:1020`: a number is a track, else a media
    /// file).
    pub fn resolve_soundtrack(&self) -> Result<Resolved> {
        let s = self
            .ctx
            .campaign
            .soundtrack
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::NotFound("campaign has no SOUNDTRACK".into()))?;
        let n = atoi(&s);
        if n > 0 {
            self.resolve_music_track(n as u32)
        } else {
            self.resolve_music_file(&s)
        }
    }

    // ---- decoding ---------------------------------------------------------------------------

    /// Decode what a [`Source`] points at.
    pub fn decode(&self, source: &Source) -> Result<Pcm> {
        decode_source(source, self.effects.as_ref(), self.speech.as_ref())
    }

    /// Decode the first variant of a resolved sound.
    pub fn decode_resolved(&self, r: &Resolved) -> Result<Pcm> {
        let v = r
            .first()
            .ok_or_else(|| Error::NotFound(r.note.clone().unwrap_or_else(|| r.query.clone())))?;
        self.decode(&v.source)
    }

    // ---- catalogue --------------------------------------------------------------------------

    /// Every sound the browser lists.
    pub fn catalog(&self) -> Vec<CatalogEntry> {
        let mut out = vec![];
        // aliases of sound.dat ids
        let mut aliases: HashMap<u16, Vec<String>> = HashMap::new();
        for (name, n) in &self.names {
            for i in 0..n.count {
                aliases
                    .entry((n.id as usize + i).min(u16::MAX as usize) as u16)
                    .or_default()
                    .push(name.clone());
            }
        }
        for v in aliases.values_mut() {
            v.sort();
        }
        if let Some(b) = &self.effects {
            for s in b.samples.iter().filter(|s| !s.reserved) {
                let resolved = self
                    .resolve_id(s.index as u32)
                    .unwrap_or_else(|_| Resolved {
                        query: s.index.to_string(),
                        variants: vec![self.bank_variant(BankKind::Effects, s.index)],
                        declared_count: 1,
                        layer: Layer::bank("sound.dat"),
                        note: None,
                    });
                let mut detail = aliases
                    .get(&s.index)
                    .map(|a| a.join(", "))
                    .unwrap_or_default();
                if let Some(n) = &resolved.note {
                    if !detail.is_empty() {
                        detail.push_str("; ");
                    }
                    detail.push_str(n);
                }
                out.push(CatalogEntry {
                    category: Category::Effects,
                    key: format!("fx:{}", s.index),
                    title: format!("{} {}", s.index, s.name),
                    detail,
                    resolved,
                    duration: (!s.broken).then(|| s.duration_secs()),
                });
            }
        }
        let mut custom: Vec<_> = self.custom.iter().collect();
        custom.sort_by(|a, b| a.0.cmp(b.0));
        for (name, c) in custom {
            if name.starts_with("__RAW_") {
                continue;
            }
            out.push(CatalogEntry {
                category: Category::Effects,
                key: format!("fxname:{name}"),
                title: name.clone(),
                detail: format!("custom file, {} variant(s)", c.variants.len()),
                resolved: Resolved {
                    query: name.clone(),
                    variants: c.variants.clone(),
                    declared_count: c.declared,
                    layer: c.layer.clone(),
                    note: None,
                },
                duration: None,
            });
        }
        if let Some(b) = &self.speech {
            for s in b.samples.iter().filter(|s| !s.reserved) {
                let resolved = self
                    .resolve_speech(&s.index.to_string())
                    .unwrap_or_else(|_| Resolved {
                        query: s.index.to_string(),
                        variants: vec![self.bank_variant(BankKind::Speech, s.index)],
                        declared_count: 1,
                        layer: Layer::bank("speech bank"),
                        note: None,
                    });
                let overridden = resolved.layer.kind != LayerKind::Bank;
                let title = match speech_name(s.index) {
                    Some(n) => format!("{n} ({})", s.name),
                    None => format!("{} {}", s.index, s.name),
                };
                out.push(CatalogEntry {
                    category: Category::Speech,
                    key: format!("speech:{}", s.index),
                    title,
                    detail: if overridden {
                        resolved.note.clone().unwrap_or_default()
                    } else {
                        String::new()
                    },
                    duration: (!overridden && !s.broken).then(|| s.duration_secs()),
                    resolved,
                });
            }
        }
        for lv in self.levels_with_speech() {
            for before in [true, false] {
                if let Ok(r) = self.resolve_level_speech(lv, before) {
                    out.push(CatalogEntry {
                        category: Category::Speech,
                        key: format!(
                            "levelspeech:{lv}:{}",
                            if before { "before" } else { "after" }
                        ),
                        title: format!(
                            "Level {lv} mentor ({})",
                            if before { "before" } else { "after" }
                        ),
                        detail: r
                            .variants
                            .first()
                            .map(|v| v.label.clone())
                            .unwrap_or_default(),
                        resolved: r,
                        duration: None,
                    });
                }
            }
        }
        for name in self.creature_names() {
            let set = self.creature_set(&name);
            for (slot, r) in set.slots {
                let detail = if r.declared_count > 1 {
                    format!("{} variants", r.variants.len())
                } else {
                    String::new()
                };
                out.push(CatalogEntry {
                    category: Category::Creatures,
                    key: format!("creature:{name}:{}", slot.key()),
                    title: format!("{name} {}", slot.key()),
                    detail,
                    resolved: r,
                    duration: None,
                });
            }
        }
        for t in self.music_tracks() {
            if let Ok(r) = self.resolve_music_track(t.number) {
                out.push(CatalogEntry {
                    category: Category::Music,
                    key: format!("music:{}", t.number),
                    title: format!("Track {}", t.number),
                    detail: r
                        .variants
                        .first()
                        .map(|v| v.label.clone())
                        .unwrap_or_default(),
                    resolved: r,
                    duration: None,
                });
            }
        }
        if let Ok(r) = self.resolve_soundtrack() {
            out.push(CatalogEntry {
                category: Category::Music,
                key: "music:soundtrack".into(),
                title: "Campaign soundtrack".into(),
                detail: r
                    .variants
                    .first()
                    .map(|v| v.label.clone())
                    .unwrap_or_default(),
                resolved: r,
                duration: None,
            });
        }
        out
    }
}

/// `is_sound_file_path`, `src/config_crtrmodel.c:2297`.
fn is_sound_file_path(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    if s.contains('/') || s.contains('\\') {
        return true;
    }
    s.contains('.') && atoi(s) == 0 && !s.starts_with('0') && !s.starts_with('-')
}

/// Decode a source with explicit banks.
pub fn decode_source(
    source: &Source,
    effects: Option<&SoundBank>,
    speech: Option<&SoundBank>,
) -> Result<Pcm> {
    match source {
        Source::Bank { kind, index } => {
            let b = match kind {
                BankKind::Effects => effects,
                BankKind::Speech => speech,
            }
            .ok_or_else(|| Error::NotFound(format!("{kind:?} bank not loaded")))?;
            b.decode(*index as usize)
        }
        Source::File(p) => {
            let bytes = std::fs::read(p).map_err(|e| Error::io(p, e))?;
            Ok(codec::decode_audio(&bytes, &p.to_string_lossy())?.0)
        }
        Source::ZipEntry { zip, entry } => {
            let bytes = read_zip_entry(zip, entry)?;
            Ok(codec::decode_audio(&bytes, entry)?.0)
        }
        Source::Silence => Ok(Pcm::new(22050, 1, vec![])),
    }
}

/// Which codec a source holds (reads the file head only for files / zip entries).
pub fn source_codec(
    source: &Source,
    effects: Option<&SoundBank>,
    speech: Option<&SoundBank>,
) -> Codec {
    match source {
        Source::Bank { kind, index } => match kind {
            BankKind::Effects => effects,
            BankKind::Speech => speech,
        }
        .and_then(|b| b.get(*index as usize))
        .map(|s| s.codec)
        .unwrap_or(Codec::Unknown),
        Source::File(p) => std::fs::read(p)
            .map(|b| codec::sniff(&b))
            .unwrap_or(Codec::Unknown),
        Source::ZipEntry { zip, entry } => read_zip_entry(zip, entry)
            .map(|b| codec::sniff(&b))
            .unwrap_or(Codec::Unknown),
        Source::Silence => Codec::Unknown,
    }
}
