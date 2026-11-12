//! Validation of a pack: every rule names the KeeperFX C code it comes from (`Issue::rule`).
//!
//! Severity: `Error` = the game fails or drops something (the campaign does not load, a level
//! cannot start, a land view cannot be shown); `Warning` = the game logs a warning or falls
//! back; `Info` = a suggestion.
use crate::landview::{self, MAP_H, MAP_W};
use crate::pack::*;
use crate::texts::Texts;
use kfx_formats::campaign::{CampaignCommon, PackKind};
use kfx_formats::levelinfo::{LevelKind, Lof, DEFAULT_MAP_SIZE};
use kfx_formats::strings::{Lang, STRINGS_MAX};
use kfx_formats::testutil::resolve_ci;
use kfx_formats::{Error, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// What an issue is about (the UI jumps there).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Pack,
    Level(u32),
    Strings(String),
    File(PathBuf),
}

/// A repair that is safe to apply automatically ([`Pack::apply_fix`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fix {
    /// Create a missing folder.
    CreateDir(PathBuf),
    /// Write a `mapNNNNN.lof` for a level of a map pack that only has a `.lif` entry.
    CreateLof(u32),
    /// Write the real map size into the level's MAPSIZE.
    SetMapSize { level: u32, size: (i32, i32) },
    /// Make `BONUS_LEVELS` as long as `SINGLE_LEVELS` (0 = no bonus level).
    PadBonusList,
    /// Add an empty `[mapNNNNN]` block.
    CreateBlock(u32),
    /// Create the strings file of a `[strings]` language (empty table).
    CreateStringsFile(String),
    /// Add the missing `[strings]` / `[speech]` section.
    AddSection(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// Stable short id of the rule.
    pub code: &'static str,
    pub message: String,
    /// KeeperFX C source the rule comes from, `file:line`.
    pub rule: &'static str,
    pub target: Target,
    pub fix: Option<Fix>,
}

struct Out(Vec<Issue>);

impl Out {
    fn add(
        &mut self,
        severity: Severity,
        code: &'static str,
        rule: &'static str,
        target: Target,
        message: String,
        fix: Option<Fix>,
    ) {
        self.0.push(Issue {
            severity,
            code,
            message,
            rule,
            target,
            fix,
        });
    }
}

fn find_in(dir: &Path, name: &str) -> Option<PathBuf> {
    resolve_ci(dir, &name.replace('\\', "/")).filter(|p| p.exists())
}

/// Check a pack. Pure reading; nothing is changed.
pub fn validate(pack: &Pack) -> Vec<Issue> {
    let mut o = Out(vec![]);
    let common = pack.common();
    let is_campaign = pack.is_campaign();
    check_cfg(pack, &common, &mut o);
    check_folders(pack, &common, &mut o);
    let levels_dir = pack.levels_dir().filter(|d| d.is_dir());
    let land_dir = pack.land_dir().filter(|d| d.is_dir());
    check_game_keys(pack, &common, land_dir.as_deref(), &mut o);
    let entries = pack.entries();
    check_level_lists(pack, &common, &entries, &mut o);
    check_land_view(&common, is_campaign, land_dir.as_deref(), &mut o);
    let (mut texts, speech) = check_texts(pack, &mut o);
    let cx = LevelCx {
        pack,
        is_campaign,
        levels_dir: levels_dir.as_deref(),
        land_dir: land_dir.as_deref(),
        translation_aliases: alias_set(pack),
        speech,
    };
    for e in &entries {
        check_level(&cx, e, &mut texts, &mut o);
    }
    o.0.sort_by_key(|i| i.severity);
    o.0
}

/// The cfg as a whole.
fn check_cfg(pack: &Pack, common: &CampaignCommon, o: &mut Out) {
    let is_campaign = pack.is_campaign();
    let doc = &pack.cfg.doc;
    let bytes = pack.cfg.to_bytes();
    if bytes.len() > MAX_CFG_BYTES {
        o.add(
            Severity::Error,
            "cfg-too-large",
            "config_campaigns.c:1222",
            Target::Pack,
            format!(
                "the cfg is {} bytes, the game refuses files over {MAX_CFG_BYTES}",
                bytes.len()
            ),
            None,
        );
    }
    if !doc.has_section("common") {
        o.add(
            Severity::Error,
            "common-missing",
            "config_campaigns.c:448",
            Target::Pack,
            "the cfg has no [common] block".into(),
            None,
        );
    }
    if common.name.is_none() {
        o.add(
            Severity::Warning,
            "name-missing",
            "config_campaigns.c:1210",
            Target::Pack,
            "NAME is not set: the game shows the file name".into(),
            None,
        );
    }
    for (sec, rule) in [
        ("strings", "config_campaigns.c:793"),
        ("speech", "config_campaigns.c:855"),
    ] {
        if sec == "speech" && !is_campaign {
            continue; // parse_campaign_speech_blocks runs for campaigns only (:1250)
        }
        if !doc.has_section(sec) {
            o.add(
                Severity::Error,
                "section-missing",
                rule,
                Target::Pack,
                format!(
                    "the cfg has no [{sec}] block: the block parser fails and \
                        the game falls back to the default campaign"
                ),
                Some(Fix::AddSection(if sec == "strings" {
                    "strings"
                } else {
                    "speech"
                })),
            );
        }
    }
}

/// The `*_LOCATION` folders.
fn check_folders(pack: &Pack, common: &CampaignCommon, o: &mut Out) {
    let is_campaign = pack.is_campaign();
    let loc_checks: [(&str, &Option<String>, bool, &'static str); 5] = [
        (
            "LEVELS_LOCATION",
            &common.levels_location,
            true,
            "config.c:1349-1395",
        ),
        (
            "LAND_LOCATION",
            &common.land_location,
            is_campaign,
            "config.c:1349-1395",
        ),
        (
            "CREATURES_LOCATION",
            &common.creatures_location,
            false,
            "config.c:1349-1395",
        ),
        (
            "CONFIGS_LOCATION",
            &common.configs_location,
            false,
            "config.c:1349-1395",
        ),
        (
            "MEDIA_LOCATION",
            &common.media_location,
            false,
            "config.c:1349-1395",
        ),
    ];
    for (key, val, required, rule) in loc_checks {
        match val {
            None if required => {
                let what = if key == "LEVELS_LOCATION" {
                    "the game finds no levels (no .LIF files could be found for \
                        a map pack, no level files for a campaign)"
                } else {
                    "the land view pictures cannot be found"
                };
                o.add(
                    Severity::Error,
                    "location-missing",
                    if key == "LEVELS_LOCATION" && !is_campaign {
                        "config_campaigns.c:1645"
                    } else {
                        rule
                    },
                    Target::Pack,
                    format!("{key} is not set: {what}"),
                    None,
                );
            }
            Some(v) => {
                let p = pack.loc(v);
                if !p.is_dir() {
                    let sev = if required {
                        Severity::Error
                    } else {
                        Severity::Warning
                    };
                    o.add(
                        sev,
                        "folder-missing",
                        rule,
                        Target::Pack,
                        format!("{key} = {v}: the folder does not exist"),
                        Some(Fix::CreateDir(p)),
                    );
                }
            }
            None => {}
        }
    }
}

/// Game-wide keys of `[common]`.
fn check_game_keys(pack: &Pack, common: &CampaignCommon, land_dir: Option<&Path>, o: &mut Out) {
    if let Some(h) = &common.human_player {
        if !HUMAN_PLAYERS.iter().any(|p| p.eq_ignore_ascii_case(h)) {
            o.add(
                Severity::Warning,
                "human-player",
                "config_campaigns.c:663",
                Target::Pack,
                format!(
                    "HUMAN_PLAYER = {h} is not one of {}",
                    HUMAN_PLAYERS.join(" ")
                ),
                None,
            );
        }
    }
    if let Some(m) = &common.land_markers {
        if !LAND_MARKERS.iter().any(|p| p.eq_ignore_ascii_case(m)) {
            o.add(
                Severity::Warning,
                "land-markers",
                "config_campaigns.c:653",
                Target::Pack,
                format!("LAND_MARKERS = {m} is not ENSIGNS or PINPOINTS"),
                None,
            );
        }
    }
    if let Some(c) = &common.credits {
        if let Some(d) = &land_dir {
            if find_in(d, c).is_none() {
                o.add(
                    Severity::Warning,
                    "credits-missing",
                    "config.c:1827",
                    Target::Pack,
                    format!("CREDITS = {c}: not found in the land location"),
                    None,
                );
            }
        }
    }
    let media = pack.media_dir().filter(|d| d.is_dir());
    for (key, v) in [
        ("INTRO_MOVIE", &common.intro_movie),
        ("OUTRO_MOVIE", &common.outro_movie),
    ] {
        if let Some(f) = v {
            let rule = if key == "INTRO_MOVIE" {
                "front_fmvids.c:141"
            } else {
                "front_fmvids.c:152"
            };
            match &media {
                Some(m) if find_in(m, f).is_some() => {}
                Some(_) => o.add(
                    Severity::Warning,
                    "movie-missing",
                    rule,
                    Target::Pack,
                    format!("{key} = {f}: not found in the media location"),
                    None,
                ),
                None => o.add(
                    Severity::Warning,
                    "movie-missing",
                    rule,
                    Target::Pack,
                    format!("{key} = {f}: the pack has no media location folder"),
                    None,
                ),
            }
        }
    }
    if let Some(s) = &common.soundtrack {
        if s.trim().parse::<i32>().map_or(true, |n| n < 1) {
            match &media {
                Some(m) if find_in(m, s).is_some() => {}
                _ => o.add(
                    Severity::Warning,
                    "soundtrack-missing",
                    "front_landview.c:1025",
                    Target::Pack,
                    format!(
                        "SOUNDTRACK = {s}: not a track number and not found in the media location"
                    ),
                    None,
                ),
            }
        }
    }
}

/// Level lists (campaigns) or level entries (map packs).
fn check_level_lists(pack: &Pack, common: &CampaignCommon, entries: &[LevelEntry], o: &mut Out) {
    let is_campaign = pack.is_campaign();
    let doc = &pack.cfg.doc;
    if is_campaign {
        let slots = pack.single_slots();
        let c = &common;
        if slots.is_empty() {
            o.add(
                Severity::Error,
                "no-single-level",
                "config_campaigns.c:1433",
                Target::Pack,
                "a campaign needs at least one single level in \
                    SINGLE_LEVELS: without it the game does not list the campaign"
                    .into(),
                None,
            );
        }
        if slots.len() > MAX_SINGLE_LEVELS {
            o.add(
                Severity::Error,
                "too-many-single",
                "config_campaigns.c:481",
                Target::Pack,
                format!(
                    "{} single levels, the game holds {MAX_SINGLE_LEVELS}",
                    slots.len()
                ),
                None,
            );
        }
        if c.bonus_levels.len() > MAX_SINGLE_LEVELS {
            o.add(
                Severity::Error,
                "too-many-bonus",
                "config_campaigns.c:500",
                Target::Pack,
                format!(
                    "{} BONUS_LEVELS entries, the game holds {MAX_SINGLE_LEVELS}",
                    c.bonus_levels.len()
                ),
                None,
            );
        }
        if c.extra_levels.len() > MAX_EXTRA_LEVELS {
            o.add(
                Severity::Error,
                "too-many-extra",
                "config_campaigns.c:518",
                Target::Pack,
                format!(
                    "{} extra levels, the game holds {MAX_EXTRA_LEVELS}",
                    c.extra_levels.len()
                ),
                None,
            );
        }
        if c.single_levels.len() != c.bonus_levels.len() {
            o.add(
                Severity::Warning,
                "bonus-length",
                "config_campaigns.c:779",
                Target::Pack,
                format!(
                    "SINGLE_LEVELS has {} entries but BONUS_LEVELS {}: the lists \
                        are parallel (0 = no bonus level)",
                    c.single_levels.len(),
                    c.bonus_levels.len()
                ),
                Some(Fix::PadBonusList),
            );
        }
        let linked = c.bonus_levels.iter().filter(|&&n| n > 0).count();
        if linked > MAX_BONUS_LEVELS_SAVED {
            o.add(
                Severity::Warning,
                "bonus-saved",
                "game_merge.h:44",
                Target::Pack,
                format!("{linked} bonus levels, a saved game remembers {MAX_BONUS_LEVELS_SAVED}"),
                None,
            );
        }
        let mut seen = BTreeSet::new();
        for (n, k) in pack.listed_levels() {
            if !seen.insert(n) {
                o.add(
                    Severity::Warning,
                    "duplicate-level",
                    "config_campaigns.c:1178",
                    Target::Level(n),
                    format!(
                        "level {n} is listed more than once ({}): the game keeps one \
                            description per number",
                        k.name()
                    ),
                    None,
                );
            }
            if n > MAX_LEVEL_NUMBER {
                o.add(
                    Severity::Error,
                    "level-number",
                    "lvl_filesdk1.c:167",
                    Target::Level(n),
                    format!("level number {n} does not fit the map{:05} file name", n),
                    None,
                );
            }
        }
        // the parser drops 0 from SINGLE_LEVELS: it can only be seen in the raw tokens
        let raw_single = doc.get_all("common", "SINGLE_LEVELS");
        if raw_single
            .iter()
            .flat_map(|e| e.tokens.iter())
            .any(|t| kfx_formats::levelinfo::atoi(t) <= 0)
        {
            o.add(
                Severity::Warning,
                "level-number",
                "config_campaigns.c:485",
                Target::Pack,
                "SINGLE_LEVELS has an entry that is not a level number (> 0): it is ignored".into(),
                None,
            );
        }
    } else {
        if entries.is_empty() {
            o.add(
                Severity::Error,
                "no-levels",
                "config_campaigns.c:1645",
                Target::Pack,
                "no .lof or .lif level entries found in the levels folder: \
                    the game says the map pack is empty"
                    .into(),
                None,
            );
        }
    }
}

/// Land view start / end.
fn check_land_view(
    common: &CampaignCommon,
    is_campaign: bool,
    land_dir: Option<&Path>,
    o: &mut Out,
) {
    if is_campaign || common.land_location.is_some() {
        for (key, v) in [
            ("LAND_VIEW_START", &common.land_view_start),
            ("LAND_VIEW_END", &common.land_view_end),
        ] {
            match v {
                None if is_campaign => o.add(
                    Severity::Warning,
                    "land-view-unset",
                    "config_campaigns.c:550",
                    Target::Pack,
                    format!("{key} is not set: that land view screen has no picture"),
                    None,
                ),
                Some((view, window)) => {
                    if let Some(d) = &land_dir {
                        for (msg, rule) in landview::check_files(d, view, window) {
                            o.add(
                                Severity::Error,
                                "land-view-file",
                                rule,
                                Target::Pack,
                                format!("{key}: {msg}"),
                                None,
                            );
                        }
                    }
                }
                None => {}
            }
        }
    }
}

/// The `[strings]` and `[speech]` tables. Returns the loaded texts and the speech folders, which
/// the level checks use.
fn check_texts(pack: &Pack, o: &mut Out) -> (Texts, Vec<(String, String)>) {
    let is_campaign = pack.is_campaign();
    let doc = &pack.cfg.doc;
    let texts = Texts::load(pack);
    let texts_mut = texts.clone();
    for l in &texts.langs {
        if l.lang.is_none() {
            o.add(
                Severity::Warning,
                "language-code",
                "config_campaigns.c:812",
                Target::Strings(l.code.clone()),
                format!("[strings] {}: not a language code", l.code),
                None,
            );
        }
        if !l.exists {
            o.add(
                Severity::Error,
                "strings-file",
                "config_strings.c:353",
                Target::Strings(l.code.clone()),
                format!("[strings] {} = {}: the file does not exist", l.code, l.file),
                (!l.shared).then(|| Fix::CreateStringsFile(l.code.clone())),
            );
        } else if let Some(e) = &l.error {
            o.add(
                Severity::Error,
                "strings-file",
                "config_strings.c:353",
                Target::Strings(l.code.clone()),
                format!("[strings] {} = {}: {e}", l.code, l.file),
                None,
            );
        }
    }
    if doc.has_section("strings") && !texts.langs.is_empty() && texts.index_of("ENG").is_none() {
        o.add(
            Severity::Info,
            "no-english",
            "config_campaigns.c:818",
            Target::Pack,
            "[strings] has no ENG entry: English is the fallback for languages without a table"
                .into(),
            None,
        );
    }
    let speech = pack.cfg.speech();
    if is_campaign {
        for (code, folder) in &speech {
            if Lang::from_code(code).is_none() {
                o.add(
                    Severity::Warning,
                    "language-code",
                    "config_campaigns.c:871",
                    Target::Pack,
                    format!("[speech] {code}: not a language code"),
                    None,
                );
            }
            let p = pack.loc(folder);
            if !p.is_dir() {
                o.add(
                    Severity::Warning,
                    "speech-folder",
                    "config.c:1334-1339",
                    Target::Pack,
                    format!("[speech] {code} = {folder}: the folder does not exist"),
                    Some(Fix::CreateDir(p)),
                );
            }
        }
    }
    (texts_mut, speech)
}

/// What the per-level checks share.
struct LevelCx<'a> {
    pack: &'a Pack,
    is_campaign: bool,
    levels_dir: Option<&'a Path>,
    land_dir: Option<&'a Path>,
    translation_aliases: BTreeSet<String>,
    speech: Vec<(String, String)>,
}

/// One level: its files, map size, name, and for campaigns its ensign, land view and speech.
fn check_level(cx: &LevelCx, e: &LevelEntry, texts_mut: &mut Texts, o: &mut Out) {
    let (pack, is_campaign) = (cx.pack, cx.is_campaign);
    let (levels_dir, land_dir) = (cx.levels_dir, cx.land_dir);
    let n = e.number;
    let t = Target::Level(n);
    let kinds: Vec<&str> = e.kinds.iter().map(|k| k.name()).collect();
    let kind_label = kinds.join("/");
    if is_campaign && !e.has_info {
        o.add(
            Severity::Warning,
            "block-missing",
            "config_campaigns.c:915",
            t.clone(),
            format!(
                "level {n} ({kind_label}) has no [map{n:05}] block: the \
                    ensign sits in the middle of the land view and the level has no name"
            ),
            Some(Fix::CreateBlock(n)),
        );
    }
    if !is_campaign && !e.has_info {
        o.add(
            Severity::Warning,
            "lof-missing",
            "lvl_filesdk1.c:344",
            t.clone(),
            format!(
                "level {n} has a .lif entry but no map{n:05}.lof (kind, \
                    ensign, size are defaults)"
            ),
            Some(Fix::CreateLof(n)),
        );
    }
    if let Some(d) = &levels_dir {
        if !level_exists(d, n) {
            o.add(
                Severity::Error,
                "level-file",
                "lvl_filesdk1.c:138",
                t.clone(),
                format!(
                    "level {n} ({kind_label}): map{n:05}.slb is missing in {}",
                    d.display()
                ),
                None,
            );
        } else {
            let has = |ext: &str| kfx_formats::level_bin::find_file(d, n, ext).is_some();
            let mut missing: Vec<&str> = vec![];
            for ext in ["own", "dat", "clm", "wib", "inf"] {
                if !has(ext) {
                    missing.push(ext);
                }
            }
            if !has("tng") && !has("tngfx") {
                missing.push("tng");
            }
            if !has("apt") && !has("aptfx") {
                missing.push("apt");
            }
            if !missing.is_empty() {
                o.add(
                    Severity::Warning,
                    "level-file",
                    "lvl_filesdk1.c:669-1312",
                    t.clone(),
                    format!(
                        "level {n}: missing map files {} (the game warns for each)",
                        missing
                            .iter()
                            .map(|e| format!("map{n:05}.{e}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    None,
                );
            }
            // MAPSIZE consistency
            let declared = e.info.mapsize;
            let (dw, dh) = declared.unwrap_or((DEFAULT_MAP_SIZE, DEFAULT_MAP_SIZE));
            let len = kfx_formats::level_bin::find_file(d, n, "slb")
                .and_then(|p| kfx_formats::rnc::load_file(&p).ok())
                .map(|b| b.len());
            if let Some(len) = len {
                if len != (dw as usize) * (dh as usize) * 2 {
                    let real = real_map_size(d, n, None);
                    let fix = real.map(|size| Fix::SetMapSize { level: n, size });
                    let msg = match real {
                        Some((w, h)) => format!(
                            "level {n}: MAPSIZE is {dw} {dh}{} but the map file has \
                                {w}x{h} slabs",
                            if declared.is_none() { " (default)" } else { "" }
                        ),
                        None => format!(
                            "level {n}: MAPSIZE is {dw} {dh} but map{n:05}.slb has {len} \
                                bytes ({} slabs)",
                            len / 2
                        ),
                    };
                    o.add(
                        Severity::Error,
                        "mapsize",
                        "lvl_filesdk1.c:1200",
                        t.clone(),
                        msg,
                        fix,
                    );
                }
            }
        }
    }
    // names
    let id = e.info.name_id.as_deref();
    match id {
        Some(tok) => match tok.parse::<u32>() {
            Ok(v) if v > 0 && (v as usize) <= STRINGS_MAX => {
                for i in 0..texts_mut.langs.len() {
                    if texts_mut.langs[i].table.is_some() && texts_mut.effective(i, v).is_none() {
                        let code = texts_mut.langs[i].code.clone();
                        o.add(
                            Severity::Warning,
                            "name-id-missing",
                            "config_strings.c:402-408",
                            t.clone(),
                            format!(
                                "level {n}: NAME_ID {v} has no text in {code} (campaign \
                                    table and GUI table are both empty)"
                            ),
                            None,
                        );
                    }
                }
            }
            Ok(v) => o.add(
                Severity::Warning,
                "name-id",
                "config_translation.c:203",
                t.clone(),
                format!("level {n}: NAME_ID {v} is not a valid string id (1..={STRINGS_MAX})"),
                None,
            ),
            Err(_) => {
                if !cx.translation_aliases.contains(tok) {
                    o.add(
                        Severity::Warning,
                        "name-id-alias",
                        "config_translation.c:215",
                        t.clone(),
                        format!(
                            "level {n}: NAME_ID {tok}: no translation.toml entry \
                                with this alias"
                        ),
                        None,
                    );
                }
            }
        },
        None => {
            if e.info.name_text.is_none() {
                o.add(
                    Severity::Info,
                    "name-missing",
                    "config_campaigns.c:929",
                    t.clone(),
                    format!("level {n} has neither NAME_TEXT nor NAME_ID"),
                    None,
                );
            }
        }
    }
    if !is_campaign {
        return;
    }
    // ensign
    let single = e.kinds.contains(&LevelKind::Single);
    for (key, p) in [
        ("ENSIGN_POS", e.info.ensign_pos),
        ("ENSIGN_ZOOM", e.info.ensign_zoom),
    ] {
        match p {
            Some((x, y)) => {
                if x as usize >= MAP_W || y as usize >= MAP_H {
                    o.add(
                        Severity::Error,
                        "ensign-outside",
                        "front_landview.c:583",
                        t.clone(),
                        format!(
                            "level {n}: {key} {x} {y} is outside the {MAP_W}x{MAP_H} \
                                land view picture"
                        ),
                        None,
                    );
                }
            }
            None => {
                if e.has_info {
                    o.add(
                        Severity::Info,
                        "ensign-unset",
                        "config_campaigns.c:192-195",
                        t.clone(),
                        format!(
                            "level {n}: {key} is not set: it defaults to the middle of \
                                the picture ({} {})",
                            MAP_W / 2,
                            MAP_H / 2
                        ),
                        None,
                    );
                }
            }
        }
    }
    if let Some(pl) = e.info.players {
        if pl < 1 {
            o.add(
                Severity::Warning,
                "players",
                "config_campaigns.c:1002",
                t.clone(),
                format!("level {n}: PLAYERS must be 1 or more"),
                None,
            );
        }
    }
    if let Some(en) = &e.info.ensign {
        if !en.is_empty() && !ENSIGN_KINDS.iter().any(|k| k.eq_ignore_ascii_case(en)) {
            o.add(
                Severity::Info,
                "ensign-kind",
                "config_campaigns.c:1018",
                t.clone(),
                format!(
                    "level {n}: ENSIGN {en} is not a built-in ensign: it needs a \
                        custom ensign of that name in the campaign's configs"
                ),
                None,
            );
        }
    }
    // land view of single levels
    if single {
        match &e.info.land_view {
            None => o.add(
                Severity::Error,
                "land-view-unset",
                "front_landview.c:965-970",
                t.clone(),
                format!(
                    "single level {n} has no LAND_VIEW: the land view screen \
                        cannot load a picture"
                ),
                None,
            ),
            Some((view, window)) => {
                if let Some(d) = &land_dir {
                    for (msg, rule) in landview::check_files(d, view, window) {
                        o.add(
                            Severity::Error,
                            "land-view-file",
                            rule,
                            t.clone(),
                            format!("level {n}: {msg}"),
                            None,
                        );
                    }
                }
            }
        }
    }
    // speech
    if let Some((before, after)) = &e.info.speech {
        for (what, f) in [("before", before), ("after", after)] {
            if f.is_empty() {
                continue;
            }
            let name = if f.contains('.') {
                f.clone()
            } else {
                format!("{f}.wav")
            };
            for (code, folder) in &cx.speech {
                let dir = pack.loc(folder);
                if dir.is_dir() && find_in(&dir, &name).is_none() {
                    o.add(
                        Severity::Warning,
                        "speech-file",
                        "front_landview.c:846-876",
                        t.clone(),
                        format!(
                            "level {n}: speech {what} {name} is not in [speech] {code} ({folder})"
                        ),
                        None,
                    );
                }
            }
        }
    }
}

/// Aliases of `fxdata/translation.toml` and the campaign's configs folder.
fn alias_set(pack: &Pack) -> BTreeSet<String> {
    let mut files = vec![];
    if let Some(fx) = resolve_ci(&pack.install, "fxdata") {
        files.push(fx.join("translation.toml"));
    }
    if let Some(c) = pack.common().configs_location {
        files.push(pack.loc(&c).join("translation.toml"));
    }
    let mut set = BTreeSet::new();
    for f in files {
        if let Ok(t) = kfx_formats::translation::TranslationFile::load(&f) {
            for e in t.entries {
                set.insert(e.alias);
            }
        }
    }
    set
}

fn err(msg: impl Into<String>) -> Error {
    Error::format("campaign", msg)
}

impl Pack {
    /// Apply a fix suggested by [`validate`].
    pub fn apply_fix(&mut self, fix: &Fix) -> Result<()> {
        match fix {
            Fix::CreateDir(p) => std::fs::create_dir_all(p).map_err(|e| Error::io(p, e)),
            Fix::PadBonusList => {
                let slots = self.single_slots();
                self.set_single_slots(&slots);
                Ok(())
            }
            Fix::CreateBlock(n) => {
                let info = self.level_info(*n);
                self.set_level_info(*n, &info);
                if self.cfg.map_info(*n).is_none() {
                    self.cfg.doc.add_section(&format!("map{n:05}"));
                }
                Ok(())
            }
            Fix::SetMapSize { level, size } => {
                let mut info = self.level_info(*level);
                info.mapsize = Some(*size);
                self.set_level_info(*level, &info);
                Ok(())
            }
            Fix::CreateLof(n) => {
                if self.kind == PackKind::Campaign {
                    return Err(err("a campaign does not use .lof files"));
                }
                let dir = self.levels_dir().ok_or_else(|| err("no LEVELS_LOCATION"))?;
                if kfx_formats::level_bin::find_file(&dir, *n, "lof").is_some() {
                    return Ok(());
                }
                let mut info = self.level_info(*n);
                if info.kinds.is_empty() {
                    info.kinds = vec![LevelKind::Free];
                }
                if info.mapsize.is_none() {
                    if let Some(sz) = real_map_size(&dir, *n, None) {
                        if sz != (DEFAULT_MAP_SIZE, DEFAULT_MAP_SIZE) {
                            info.mapsize = Some(sz);
                        }
                    }
                }
                let mut tx = kfx_formats::atomic::SaveTransaction::new(&dir);
                tx.write(
                    kfx_formats::level_bin::file_name(*n, "lof"),
                    Lof::new(&info).to_bytes(),
                );
                tx.commit()?;
                Ok(())
            }
            Fix::CreateStringsFile(code) => {
                let texts = Texts::load(self);
                let l = texts
                    .langs
                    .iter()
                    .find(|l| l.code.eq_ignore_ascii_case(code))
                    .ok_or_else(|| err(format!("{code} is not in [strings]")))?;
                let lang = l
                    .lang
                    .ok_or_else(|| err(format!("{code} is not a language code")))?;
                if l.shared {
                    return Err(err("the file is not owned by the pack"));
                }
                let mut table = kfx_formats::strings::StringTable::new(lang);
                table.set_raw(crate::texts::NEW_TABLE_STRINGS - 1, vec![]);
                let dir = l.path.parent().map(Path::to_path_buf).unwrap_or_default();
                let mut tx = kfx_formats::atomic::SaveTransaction::new(&dir);
                tx.write(
                    l.path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    table.to_bytes(),
                );
                tx.commit()?;
                Ok(())
            }
            Fix::AddSection(name) => {
                self.cfg.doc.add_section(name);
                Ok(())
            }
        }
    }
}
