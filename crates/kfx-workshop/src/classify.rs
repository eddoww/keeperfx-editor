//! Decide what a downloaded archive contains by looking at its file tree.
//!
//! The Workshop has no manifest, so the kind is inferred from where the files sit
//! (`levels/`, `campgns/`, `mods/`, `multiplayer/` roots as the game's folders are named,
//! `mapNNNNN.slb` groups, campaign/pack `.cfg` files with a `[common]` section, `mod.cfg`,
//! creature `.cfg` + sprite zips, `sprites.json`). Where each kind must live for the game to find
//! it: `src/config_campaigns.c` (`campgns/*.cfg` campaigns, `levels/*.cfg` map packs,
//! `multiplayer/*.cfg` multiplayer packs) and `src/config_mods.c` (`mods/<name>` listed in
//! `mods/load_order.cfg`).

use crate::archive::{self, Limits, Listing};
use crate::error::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// One level (`mapNNNNN.*`).
    Level,
    /// Several levels, or a map-pack `.cfg`.
    MapPack,
    /// A `campgns/<name>.cfg` with its folders.
    Campaign,
    /// A `mods/<name>/` folder, `mod.cfg`, or a loose bundle of config overrides.
    Mod,
    /// Creature/object sprites and their configs.
    CreaturePack,
    /// Tools, translations, texture files, anything the game does not load by itself.
    Other,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Level => "single level",
            Kind::MapPack => "map pack",
            Kind::Campaign => "campaign",
            Kind::Mod => "mod",
            Kind::CreaturePack => "creature pack",
            Kind::Other => "other",
        }
    }
}

/// Where the archive's paths are anchored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Paths start with `levels/`, `campgns/`, `mods/` or `multiplayer/`: extract at the install
    /// root.
    Rooted,
    /// Files without the game's folder names: the installer picks the destination.
    Loose,
}

/// The files of one level folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelGroup {
    /// Folder in the archive (after wrapper stripping), `""` for the archive root.
    pub dir: String,
    /// Level numbers with a `.slb` in that folder.
    pub numbers: Vec<u32>,
    /// A `.lof` with `KIND = MULTI` (or the folder is under `multiplayer/`).
    pub multiplayer: bool,
}

#[derive(Clone, Debug)]
pub struct Classification {
    pub kind: Kind,
    pub layout: Layout,
    /// A single wrapper folder that was stripped from every path before classifying.
    pub wrapper: Option<String>,
    pub levels: Vec<LevelGroup>,
    /// Campaign `.cfg` files (stripped path), rooted: `campgns/x.cfg`; loose: `x.cfg`.
    pub campaigns: Vec<String>,
    /// Map-pack `.cfg` files (`levels/x.cfg`, `multiplayer/x.cfg`, or loose `x.cfg` without
    /// SINGLE_LEVELS).
    pub packs: Vec<String>,
    /// Mod folder names (rooted `mods/<name>/`, or the single loose mod).
    pub mods: Vec<String>,
    /// Sprite content (`sprites.json`, frame folders) that is repacked into one zip.
    pub sprite_content: bool,
    /// Files ignored (OS junk).
    pub junk: Vec<String>,
    /// Files that are not installed (documentation, tools, unknown roots).
    pub unused: Vec<String>,
    /// Entries that make the archive unsafe to extract.
    pub unsafe_entries: Vec<String>,
    /// Why `Kind::Other`, and other remarks.
    pub notes: Vec<String>,
    /// Number of files considered.
    pub file_count: usize,
}

const ROOTS: [&str; 4] = ["levels", "campgns", "mods", "multiplayer"];

fn is_junk(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.starts_with("__macosx/")
        || l.contains("/__macosx/")
        || l.ends_with(".ds_store")
        || l.ends_with("/thumbs.db")
        || l == "thumbs.db"
        || l.ends_with("desktop.ini")
        || l.starts_with(".git/")
        || l.contains("/.git/")
}

/// `map00012.slb` -> 12 (any case).
pub fn level_number(name: &str) -> Option<u32> {
    let l = name.to_ascii_lowercase();
    let r = l.strip_prefix("map")?;
    let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() != 5 || !r[5..].starts_with('.') {
        return None;
    }
    digits.parse().ok()
}

fn split_dir(p: &str) -> (&str, &str) {
    match p.rfind('/') {
        Some(i) => (&p[..i], &p[i + 1..]),
        None => ("", p),
    }
}

fn ext(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Config file names the game reads from `fxdata/` / a campaign's `CONFIGS_LOCATION` (the shipped
/// `fxdata` folder, `src/config.c` file table).
pub const FXDATA_CFG: &[&str] = &[
    "creature.cfg",
    "crstates.cfg",
    "cubes.cfg",
    "keepcompp.cfg",
    "lenses.cfg",
    "magic.cfg",
    "objects.cfg",
    "rules.cfg",
    "sounds.cfg",
    "terrain.cfg",
    "trapdoor.cfg",
    "columnset.toml",
    "effects.toml",
    "playerstates.toml",
    "powerhands.toml",
    "slabset.toml",
    "spritecolors.toml",
    "textureanim.toml",
    "translation.toml",
];

fn is_fxdata_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    FXDATA_CFG.contains(&l.as_str())
        || (l.starts_with("gtext_") && l.ends_with(".dat"))
        || l.starts_with("tmapa")
        || l.starts_with("tmapb")
}

fn looks_like_campaign_cfg(text: &str) -> Option<bool> {
    // Some(true) campaign (SINGLE_LEVELS), Some(false) pack, None not a pack/campaign cfg
    let t = text.to_ascii_uppercase();
    if !t.contains("[COMMON]") || !(t.contains("LEVELS_LOCATION") || t.contains("SINGLE_LEVELS")) {
        return None;
    }
    Some(
        t.lines()
            .any(|l| l.trim_start().starts_with("SINGLE_LEVELS")),
    )
}

/// Classify a list of archive file paths. `peek` gives the content of small text files
/// (`.cfg`, `.lof`) when available; classification degrades gracefully without it.
pub fn classify_paths(
    paths: &[String],
    peek: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
) -> Classification {
    let mut c = Classification {
        kind: Kind::Other,
        layout: Layout::Loose,
        wrapper: None,
        levels: vec![],
        campaigns: vec![],
        packs: vec![],
        mods: vec![],
        sprite_content: false,
        junk: vec![],
        unused: vec![],
        unsafe_entries: vec![],
        notes: vec![],
        file_count: 0,
    };
    let mut files: Vec<String> = vec![];
    for p in paths {
        if is_junk(p) {
            c.junk.push(p.clone());
        } else {
            files.push(p.clone());
        }
    }
    c.file_count = files.len();
    if files.is_empty() {
        c.notes.push("the archive contains no files".into());
        return c;
    }
    // strip wrapper folders: one top-level dir holding everything, that is not a game root
    let mut strip = String::new();
    loop {
        let tops: BTreeSet<&str> = files
            .iter()
            .map(|f| f.split('/').next().unwrap_or(""))
            .collect();
        let all_nested = files.iter().all(|f| f.contains('/'));
        if tops.len() == 1 && all_nested {
            let top = tops.iter().next().copied().unwrap_or("");
            if ROOTS.contains(&top.to_ascii_lowercase().as_str())
                || matches!(
                    top.to_ascii_lowercase().as_str(),
                    "fxdata" | "creatrs" | "cfgs" | "cfg" | "configs" | "crtr"
                )
            {
                break;
            }
            // keep a folder that directly holds level files and has siblings as the pack name only
            strip = if strip.is_empty() {
                top.to_string()
            } else {
                format!("{strip}/{top}")
            };
            let prefix = format!("{top}/");
            files = files
                .iter()
                .map(|f| f.strip_prefix(&prefix).unwrap_or(f).to_string())
                .collect();
            if strip.matches('/').count() >= 3 {
                break;
            }
        } else {
            break;
        }
    }
    if !strip.is_empty() {
        c.wrapper = Some(strip.clone());
    }
    let full = |p: &str| {
        if strip.is_empty() {
            p.to_string()
        } else {
            format!("{strip}/{p}")
        }
    };

    let rooted = files.iter().any(|f| {
        f.split('/')
            .next()
            .is_some_and(|t| f.contains('/') && ROOTS.contains(&t.to_ascii_lowercase().as_str()))
    });
    c.layout = if rooted {
        Layout::Rooted
    } else {
        Layout::Loose
    };

    // level groups
    let mut groups: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut lofs: HashMap<String, Vec<String>> = HashMap::new();
    for f in &files {
        let (dir, name) = split_dir(f);
        if let Some(n) = level_number(name) {
            if ext(name) == "slb" {
                groups.entry(dir.to_string()).or_default().insert(n);
            }
            if ext(name) == "lof" {
                lofs.entry(dir.to_string()).or_default().push(f.clone());
            }
        }
    }
    for (dir, nums) in &groups {
        let under_mp = dir
            .split('/')
            .next()
            .is_some_and(|t| t.eq_ignore_ascii_case("multiplayer"));
        let mut multi = under_mp;
        for lof in lofs.get(dir).into_iter().flatten() {
            if let Some(b) = peek(&full(lof)) {
                let t = kfx_formats::cfg::latin1_decode(&b).to_ascii_uppercase();
                if t.lines()
                    .any(|l| l.replace(' ', "").starts_with("KIND=MULTI"))
                {
                    multi = true;
                }
            }
        }
        c.levels.push(LevelGroup {
            dir: dir.clone(),
            numbers: nums.iter().copied().collect(),
            multiplayer: multi,
        });
    }

    // cfg files at the roots
    for f in &files {
        let (dir, name) = split_dir(f);
        if ext(name) != "cfg" {
            continue;
        }
        let lower_dir = dir.to_ascii_lowercase();
        if rooted {
            if lower_dir == "campgns" {
                c.campaigns.push(f.clone());
            } else if lower_dir == "levels" || lower_dir == "multiplayer" {
                c.packs.push(f.clone());
            }
        } else if dir.is_empty() {
            if let Some(b) = peek(&full(f)) {
                match looks_like_campaign_cfg(&kfx_formats::cfg::latin1_decode(&b)) {
                    Some(true) => c.campaigns.push(f.clone()),
                    Some(false) => c.packs.push(f.clone()),
                    None => {}
                }
            }
        }
    }

    // mods
    let mut mod_names: BTreeSet<String> = BTreeSet::new();
    if rooted {
        for f in &files {
            let parts: Vec<&str> = f.split('/').collect();
            if parts.len() >= 3 && parts[0].eq_ignore_ascii_case("mods") {
                mod_names.insert(parts[1].to_string());
            }
        }
    }
    let has_mod_cfg = files.iter().any(|f| f.eq_ignore_ascii_case("mod.cfg"));
    c.mods = mod_names.into_iter().collect();

    // loose asset detection
    let root_files: Vec<&String> = files.iter().filter(|f| !f.contains('/')).collect();
    let has_sprites_json = files.iter().any(|f| {
        matches!(
            f.to_ascii_lowercase().as_str(),
            "sprites.json" | "icons.json"
        )
    });
    let frame_dirs = files
        .iter()
        .filter_map(|f| f.split('/').next().filter(|_| f.contains('/')))
        .any(|d| {
            let l = d.to_ascii_lowercase();
            l.ends_with("_fp") || l.ends_with("_tp") || l.ends_with("_td")
        });
    c.sprite_content = !rooted && (has_sprites_json || frame_dirs);
    let has_cfg = root_files.iter().any(|f| ext(f) == "cfg");
    let has_sprite_zip = root_files.iter().any(|f| ext(f) == "zip");
    let creature_cfg = root_files
        .iter()
        .any(|f| ext(f) == "cfg" && !is_fxdata_name(f) && !f.eq_ignore_ascii_case("mod.cfg"))
        || files
            .iter()
            .any(|f| f.to_ascii_lowercase().starts_with("@cfg/"));
    let fx_only = !root_files.is_empty()
        && root_files.iter().all(|f| {
            is_fxdata_name(f)
                || ext(f) == "zip"
                || matches!(ext(f).as_str(), "txt" | "md" | "png" | "jpg" | "lua")
        });

    // top-level folders that hold configuration (a mod's `fxdata/`, `creatrs/`, or a campaign's
    // `cfgs/`, `crtr/` that the author did not name as a mod)
    let cfg_dirs: Vec<String> = files
        .iter()
        .filter(|f| f.contains('/'))
        .filter_map(|f| f.split('/').next())
        .map(|d| d.to_ascii_lowercase())
        .filter(|d| {
            matches!(
                d.as_str(),
                "fxdata" | "creatrs" | "cfgs" | "cfg" | "configs" | "crtr"
            )
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    // kind
    let n_levels: usize = c.levels.iter().map(|g| g.numbers.len()).sum();
    c.kind = if !c.campaigns.is_empty() {
        Kind::Campaign
    } else if !c.levels.is_empty() || !c.packs.is_empty() {
        if n_levels > 1 || !c.packs.is_empty() {
            Kind::MapPack
        } else {
            Kind::Level
        }
    } else if !c.mods.is_empty() || has_mod_cfg {
        Kind::Mod
    } else if !rooted && !cfg_dirs.is_empty() {
        if cfg_dirs
            .iter()
            .any(|d| matches!(d.as_str(), "creatrs" | "crtr"))
            || c.sprite_content
        {
            Kind::CreaturePack
        } else {
            Kind::Mod
        }
    } else if c.sprite_content || (creature_cfg && (has_sprite_zip || has_cfg)) {
        Kind::CreaturePack
    } else if (has_cfg || has_sprite_zip) && fx_only {
        Kind::Mod
    } else {
        Kind::Other
    };
    if !c.levels.is_empty() && !c.mods.is_empty() && c.kind != Kind::Mod {
        c.notes
            .push(format!("also contains mod(s): {}", c.mods.join(", ")));
    }
    if c.kind == Kind::MapPack && n_levels == 0 {
        c.notes.push(
            "a pack configuration without level files (it reuses levels from elsewhere)".into(),
        );
    }
    if c.kind == Kind::Other && rooted {
        c.notes.push(
            "add-on files under the game's own folders (e.g. translated \
                speech) without a level, campaign or mod"
                .into(),
        );
    } else if c.kind == Kind::Other {
        let exts: BTreeSet<String> = files.iter().map(|f| ext(split_dir(f).1)).collect();
        let mut why = vec![];
        if exts.contains("exe")
            || exts.contains("dll")
            || exts.contains("sh")
            || exts.contains("bat")
            || exts.contains("pck")
            || exts.contains("vsix")
        {
            why.push("contains programs (a tool or application, not game content)");
        }
        if exts.contains("po") || exts.contains("pot") {
            why.push("translation sources (.po/.pot)");
        }
        if files.iter().all(|f| level_number(split_dir(f).1).is_none())
            && files.iter().any(|f| level_number(split_dir(f).1).is_some())
        {
            why.push("level files without a .slb (incomplete level)");
        }
        if exts.contains("dat")
            && files
                .iter()
                .any(|f| split_dir(f).1.to_ascii_lowercase().starts_with("tmap"))
        {
            why.push(
                "texture sets (tmapa/tmapb .dat): per-level files, copy next \
                    to a level as mapNNNNN.tmapaNNN.dat",
            );
        }
        if why.is_empty() {
            why.push("no level, campaign, mod, creature or configuration files recognised");
        }
        c.notes.extend(why.into_iter().map(String::from));
    }
    // files that are not installed
    let used = |f: &str| -> bool {
        match c.kind {
            Kind::Other if !rooted => false,
            _ if rooted => f
                .split('/')
                .next()
                .is_some_and(|t| ROOTS.contains(&t.to_ascii_lowercase().as_str())),
            _ => true,
        }
    };
    c.unused = files.iter().filter(|f| !used(f)).map(|f| full(f)).collect();
    if c.layout == Layout::Rooted {
        c.unused.retain(|_| true);
    }
    c
}

/// Everything known about an archive file.
#[derive(Debug)]
pub struct Inspected {
    pub listing: Listing,
    pub class: Classification,
    /// Small text files read while classifying (stripped-path keys are the full archive paths).
    pub peeked: HashMap<String, Vec<u8>>,
}

/// List and classify an archive file.
pub fn classify_archive(path: &Path, limits: &Limits) -> Result<Inspected> {
    let listing = archive::list(path, limits)?;
    // read the small text files classification may look at, in one pass
    let mut peeked: HashMap<String, Vec<u8>> = HashMap::new();
    let sizes: HashMap<&str, u64> = listing.files().map(|e| (e.path.as_str(), e.size)).collect();
    let want = |p: &str| {
        let e = ext(split_dir(p).1);
        (e == "cfg" || e == "lof") && sizes.get(p).is_some_and(|s| *s <= 256 * 1024)
    };
    archive::read_entries(path, &want, limits, &mut |p, b| {
        peeked.insert(p.to_string(), b);
        Ok(())
    })?;
    let paths: Vec<String> = listing.files().map(|e| e.path.clone()).collect();
    let mut class = classify_paths(&paths, &mut |p| peeked.get(p).cloned());
    class.unsafe_entries = listing.problems();
    Ok(Inspected {
        listing,
        class,
        peeked,
    })
}
