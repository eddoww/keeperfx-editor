//! Packaging a level or a campaign for the Workshop (offline).
//!
//! # Layouts (as used by the most downloaded Workshop items)
//!
//! * single player map: flat `mapNNNNN.*` at the zip root (8 of the 12 most downloaded maps), a
//!   readme beside it; the alternative is rooted `levels/<pack>/mapNNNNN.*`
//! * multiplayer map: `multiplayer/modern/mapNNNNN.*` (9 of the 12 most downloaded), or
//!   `campgns/keeporig/`
//! * map pack: `levels/<name>.cfg` + `levels/<name>/` (+ `_cfgs`, `_crtr`, `_lnd`, `_media`)
//! * campaign: `campgns/<name>.cfg` + `campgns/<name>/` + `_lnd`, `_cfgs`, `_crtr`, speech folders
//! * mod: `mods/<name>/...`
//!
//! The installer in [`crate::install`] reads every one of these layouts back.
//!
//! Uploading needs the user's own login in a browser; this module only produces the zip and
//! the text for the upload form ([`upload_page_url`], [`upload_form_text`]).

use crate::archive::{self, Limits};
use crate::error::{Error, Result};
use crate::util::{format_size, sha256_hex};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Where the user starts an upload (they log in with their own account there).
pub fn upload_page_url() -> &'static str {
    "https://keeperfx.net/workshop"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A finding passed in by the host (the editor's validator) or produced by the pre-flight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub message: String,
    /// Archive path the finding is about.
    pub path: Option<String>,
}

impl Issue {
    pub fn new(severity: Severity, message: impl Into<String>) -> Issue {
        Issue {
            severity,
            message: message.into(),
            path: None,
        }
    }
    fn at(severity: Severity, path: &str, message: impl Into<String>) -> Issue {
        Issue {
            severity,
            message: message.into(),
            path: Some(path.to_string()),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Subject {
    /// One level in its folder.
    Level { folder: PathBuf, number: u32 },
    /// A campaign or map pack, by its `.cfg` inside an install.
    Campaign { install_root: PathBuf, cfg: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum PackLayout {
    /// Flat for single player levels, `multiplayer/modern` for multiplayer ones.
    #[default]
    Auto,
    Flat,
    /// `<dir>/mapNNNNN.*`, e.g. `levels/standard`.
    Rooted(String),
}

#[derive(Clone, Debug, Default)]
pub struct PackageOptions {
    pub title: String,
    pub author: String,
    pub description: String,
    pub layout: PackLayout,
    /// Include the editor's `mapNNNNN.kfxed.toml` sidecar (custom slabs, notes, locks).
    pub include_sidecar: bool,
    /// Add a generated `.lif` when a free play level has neither `.lof` nor `.lif`.
    pub add_overview_if_missing: bool,
    pub write_readme: bool,
    /// Findings of the editor's validator, shown in the pre-flight and counted for blocking.
    pub host_issues: Vec<Issue>,
    /// Archive paths the user unchecked.
    pub excluded: BTreeSet<String>,
    /// A preview image copied next to the zip (the Workshop takes screenshots as separate uploads).
    pub preview_image: Option<PathBuf>,
}

impl PackageOptions {
    pub fn new(title: &str) -> Self {
        PackageOptions {
            title: title.into(),
            add_overview_if_missing: true,
            write_readme: true,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug)]
pub enum EntrySource {
    File(PathBuf),
    Text(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct PackageEntry {
    pub path: String,
    pub source: EntrySource,
    pub size: u64,
    pub role: &'static str,
    pub included: bool,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Preflight {
    pub issues: Vec<Issue>,
    pub total_size: u64,
    /// Bytes per role, biggest first.
    pub by_role: Vec<(String, u64)>,
}

impl Preflight {
    pub fn count(&self, s: Severity) -> usize {
        self.issues.iter().filter(|i| i.severity == s).count()
    }
    pub fn has_errors(&self) -> bool {
        self.count(Severity::Error) > 0
    }
}

#[derive(Clone, Debug)]
pub struct PackagePlan {
    pub subject: Subject,
    pub opts: PackageOptions,
    /// Suggested zip file name.
    pub zip_name: String,
    pub entries: Vec<PackageEntry>,
    pub preflight: Preflight,
}

impl PackagePlan {
    pub fn included(&self) -> impl Iterator<Item = &PackageEntry> {
        self.entries.iter().filter(|e| e.included)
    }
    /// Re-run the pre-flight after the user (un)checked files.
    pub fn refresh(&mut self) {
        for e in &mut self.entries {
            e.included = !self.opts.excluded.contains(&e.path);
        }
        self.preflight = preflight(&self.entries, &self.opts, &self.subject);
    }
}

#[derive(Clone, Debug)]
pub struct Packaged {
    pub zip: PathBuf,
    pub size: u64,
    pub sha256: String,
    pub files: usize,
    pub preview: Option<PathBuf>,
    /// What the installer reads back from the zip (self-check).
    pub kind: crate::classify::Kind,
}

// ---------------------------------------------------------------- gathering

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Role of a file inside a level (`mapNNNNN.<ext>`), after `kfx-editor`'s level file list.
pub fn level_role(ext: &str) -> &'static str {
    let e = ext.to_ascii_lowercase();
    match e.as_str() {
        "slb" | "own" | "wlb" | "inf" | "wib" | "clm" | "dat" | "slx" | "flg" | "tng" | "apt"
        | "lgt" | "tngfx" | "aptfx" | "lgtfx" | "adi" | "vsn" | "une" => "map",
        "lof" | "lif" => "overview",
        "txt" | "lua" => "script",
        "zip" => "sprites",
        "kfxed.toml" => "editor data",
        _ if e.ends_with(".dat") => "strings",
        _ if e.ends_with(".cfg") || e.ends_with(".toml") => "config",
        _ if e.starts_with("tmap") => "texture",
        _ => "other",
    }
}

fn is_scratch(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    kfx_formats::atomic::is_scratch_name(name)
        || l.ends_with(".bak")
        || l.ends_with('~')
        || l.ends_with(".tmp")
        || l == "thumbs.db"
        || l == ".ds_store"
        || l == "desktop.ini"
}

fn push_file(
    entries: &mut Vec<PackageEntry>,
    path: String,
    file: &Path,
    role: &'static str,
    note: Option<String>,
) {
    let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    entries.push(PackageEntry {
        path,
        source: EntrySource::File(file.to_path_buf()),
        size,
        role,
        included: true,
        note,
    });
}

fn walk_files(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    for p in read_dir_sorted(dir) {
        let n = name_of(&p);
        let Ok(md) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if md.file_type().is_symlink() {
            continue; // never package links
        }
        let r = if rel.is_empty() {
            n.clone()
        } else {
            format!("{rel}/{n}")
        };
        if md.is_dir() {
            walk_files(&p, &r, out);
        } else if md.is_file() {
            out.push((r, p));
        }
    }
}

/// Collect everything the subject consists of and run the pre-flight.
pub fn gather(subject: &Subject, opts: &PackageOptions) -> Result<PackagePlan> {
    let mut entries = vec![];
    let zip_name;
    match subject {
        Subject::Level { folder, number } => {
            // the shared listing of a level's files (any case, sorted)
            let files: Vec<PathBuf> = kfx_formats::level_bin::level_files(folder, *number);
            if files.is_empty() {
                return Err(Error::Other(format!(
                    "no map{number:05}.* files in {}",
                    folder.display()
                )));
            }
            // multiplayer?
            let multi = files
                .iter()
                .find(|p| name_of(p).to_ascii_lowercase().ends_with(".lof"))
                .and_then(|p| kfx_formats::levelinfo::Lof::load(p).ok())
                .is_some_and(|l| {
                    l.info()
                        .kinds
                        .contains(&kfx_formats::levelinfo::LevelKind::Multi)
                });
            let dir = match &opts.layout {
                PackLayout::Flat => String::new(),
                PackLayout::Rooted(d) => d.trim_matches('/').to_string(),
                PackLayout::Auto if multi => "multiplayer/modern".to_string(),
                PackLayout::Auto => String::new(),
            };
            let mut has_overview = false;
            for f in &files {
                let name = name_of(f);
                let ext =
                    kfx_formats::level_bin::level_file_ext(&name, *number).unwrap_or_default();
                let role = level_role(&ext);
                if role == "overview" {
                    has_overview = true;
                }
                let path = if dir.is_empty() {
                    name.clone()
                } else {
                    format!("{dir}/{name}")
                };
                let (included, note) = if role == "editor data" && !opts.include_sidecar {
                    (
                        false,
                        Some("editor data (.kfxed.toml): excluded by choice".to_string()),
                    )
                } else if is_scratch(&name) {
                    (false, Some("scratch/backup file".to_string()))
                } else {
                    (true, None)
                };
                push_file(&mut entries, path, f, role, note);
                if let Some(e) = entries.last_mut() {
                    e.included = included;
                }
            }
            if !has_overview && opts.add_overview_if_missing && !multi {
                let t = kfx_formats::cfg::latin1_encode(&format!(
                    "{number}, {}\n",
                    if opts.title.is_empty() {
                        format!("Level {number}")
                    } else {
                        opts.title.clone()
                    }
                ));
                let path = if dir.is_empty() {
                    format!("map{number:05}.lif")
                } else {
                    format!("{dir}/map{number:05}.lif")
                };
                entries.push(PackageEntry {
                    path,
                    size: t.len() as u64,
                    source: EntrySource::Text(t),
                    role: "overview",
                    included: true,
                    note: Some("generated: free play needs a .lif or .lof".into()),
                });
            }
            zip_name = format!("map{number:05}.zip");
            if opts.write_readme {
                let base = format!("map{number:05}");
                let install = if dir.is_empty() {
                    "Extract the files into a folder under keeperfx/levels (for \
                        example levels/personal), then pick the level in 'Free play levels'."
                        .to_string()
                } else {
                    "Extract the archive into your KeeperFX folder (the paths \
                        inside are relative to it)."
                        .to_string()
                };
                let text = readme(opts, &install, &[format!("Level number: {number}")]);
                let path = if dir.is_empty() {
                    format!("{base}_readme.txt")
                } else {
                    format!("{dir}/{base}_readme.txt")
                };
                entries.push(PackageEntry {
                    path,
                    size: text.len() as u64,
                    source: EntrySource::Text(text.into_bytes()),
                    role: "readme",
                    included: true,
                    note: Some("generated stub: edit before publishing".into()),
                });
            }
        }
        Subject::Campaign { install_root, cfg } => {
            let resolved = kfx_formats::campaign::resolve(install_root, cfg)?;
            let cfg_rel = rel_to(install_root, cfg).ok_or_else(|| {
                Error::Other(format!(
                    "{} is not inside {}",
                    cfg.display(),
                    install_root.display()
                ))
            })?;
            let stem = name_of(cfg)
                .trim_end_matches(".cfg")
                .trim_end_matches(".CFG")
                .to_string();
            push_file(
                &mut entries,
                cfg_rel.clone(),
                cfg,
                "config",
                Some("campaign configuration".into()),
            );
            let owns = |dir: &Path| -> Option<String> {
                let rel = rel_to(install_root, dir)?;
                let leaf = rel.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
                let shipped = leaf.starts_with("keeporig")
                    && !stem.to_ascii_lowercase().starts_with("keeporig");
                (!shipped && rel.contains('/')).then_some(rel)
            };
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut note_skipped: Vec<String> = vec![];
            let mut add_dir = |dir: &Option<PathBuf>,
                               role: &'static str,
                               entries: &mut Vec<PackageEntry>,
                               note_skipped: &mut Vec<String>| {
                let Some(d) = dir else { return };
                let Some(rel) = owns(d) else {
                    note_skipped.push(format!(
                        "{} is shipped with the game or shared and is not packaged",
                        d.display()
                    ));
                    return;
                };
                if !seen.insert(rel.to_ascii_lowercase()) {
                    return;
                }
                let mut v = vec![];
                walk_files(d, &rel, &mut v);
                for (path, f) in v {
                    let name = name_of(&f);
                    let ext_role = name
                        .to_ascii_lowercase()
                        .strip_prefix("map")
                        .and_then(|r| r.get(5..))
                        .map(|e| level_role(e.trim_start_matches('.')));
                    let (inc, note) = if is_scratch(&name) {
                        (false, Some("scratch/backup file".to_string()))
                    } else if ext_role == Some("editor data") && !opts.include_sidecar {
                        (
                            false,
                            Some("editor data (.kfxed.toml): excluded by choice".to_string()),
                        )
                    } else {
                        (true, None)
                    };
                    push_file(
                        entries,
                        path,
                        &f,
                        if ext_role == Some("editor data") {
                            "editor data"
                        } else {
                            role
                        },
                        note,
                    );
                    if let Some(e) = entries.last_mut() {
                        e.included = inc;
                    }
                }
            };
            add_dir(&resolved.levels_dir, "map", &mut entries, &mut note_skipped);
            add_dir(&resolved.land_dir, "land", &mut entries, &mut note_skipped);
            add_dir(
                &resolved.creatures_dir,
                "creatures",
                &mut entries,
                &mut note_skipped,
            );
            add_dir(
                &resolved.configs_dir,
                "config",
                &mut entries,
                &mut note_skipped,
            );
            // media only when it is the campaign's own folder
            if let Some(m) = &resolved.media_dir {
                if owns(m).is_some() {
                    add_dir(
                        &resolved.media_dir,
                        "media",
                        &mut entries,
                        &mut note_skipped,
                    );
                }
            }
            // speech folders and string files named by the cfg
            let camp = kfx_formats::campaign::Campaign::load(cfg)?;
            for (_, loc) in camp.speech() {
                let loc = loc.trim().replace('\\', "/");
                let leaf = loc
                    .trim_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if leaf.starts_with("keeporig")
                    && !stem.to_ascii_lowercase().starts_with("keeporig")
                {
                    let n = format!("{loc} is shipped with the game and is not packaged");
                    if !note_skipped.contains(&n) {
                        note_skipped.push(n);
                    }
                    continue;
                }
                let p = kfx_formats::testutil::resolve_ci(install_root, loc.trim_matches('/'))
                    .filter(|p| p.is_dir());
                add_dir(&p, "speech", &mut entries, &mut note_skipped);
            }
            for (_, loc) in camp.strings() {
                if let Some(f) = kfx_formats::testutil::resolve_ci(
                    install_root,
                    loc.trim().replace('\\', "/").trim_matches('/'),
                )
                .filter(|p| p.is_file())
                {
                    if let Some(rel) = rel_to(install_root, &f) {
                        let first = rel.split('/').next().unwrap_or("").to_ascii_lowercase();
                        if first == "campgns" || first == "levels" || first == "multiplayer" {
                            if !entries.iter().any(|e| e.path.eq_ignore_ascii_case(&rel)) {
                                push_file(&mut entries, rel, &f, "strings", None);
                            }
                        }
                    }
                }
            }
            zip_name = format!("{stem}.zip");
            if opts.write_readme {
                let install = "Extract the archive into your KeeperFX folder (the paths \
                    inside are relative to it)."
                    .to_string();
                let extra = vec![
                    format!("Campaign configuration: {cfg_rel}"),
                    format!("Levels: {}", resolved.levels.len()),
                ];
                let text = readme(opts, &install, &extra);
                let root = cfg_rel.rsplit_once('/').map(|x| x.0).unwrap_or("");
                let path = if root.is_empty() {
                    format!("{stem}_readme.txt")
                } else {
                    format!("{root}/{stem}_readme.txt")
                };
                entries.push(PackageEntry {
                    path,
                    size: text.len() as u64,
                    source: EntrySource::Text(text.into_bytes()),
                    role: "readme",
                    included: true,
                    note: Some("generated stub: edit before publishing".into()),
                });
            }
            for n in note_skipped {
                entries.push(PackageEntry {
                    path: String::new(),
                    source: EntrySource::Text(vec![]),
                    size: 0,
                    role: "note",
                    included: false,
                    note: Some(n),
                });
            }
        }
    }
    for e in &mut entries {
        if opts.excluded.contains(&e.path) {
            e.included = false;
        }
    }
    let pf = preflight(&entries, opts, subject);
    Ok(PackagePlan {
        subject: subject.clone(),
        opts: opts.clone(),
        zip_name,
        entries,
        preflight: pf,
    })
}

fn rel_to(root: &Path, p: &Path) -> Option<String> {
    let r = p.strip_prefix(root).ok()?;
    Some(
        r.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn readme(opts: &PackageOptions, install: &str, extra: &[String]) -> String {
    let mut t = String::new();
    t.push_str(&format!(
        "{}\n{}\n\n",
        if opts.title.is_empty() {
            "Untitled"
        } else {
            &opts.title
        },
        "=".repeat(opts.title.len().max(8))
    ));
    if !opts.author.is_empty() {
        t.push_str(&format!("Author: {}\n", opts.author));
    }
    for e in extra {
        t.push_str(&format!("{e}\n"));
    }
    t.push_str("Requires: KeeperFX (state the minimum version here)\n\n");
    if !opts.description.is_empty() {
        t.push_str(&format!(
            "Description\n-----------\n{}\n\n",
            opts.description
        ));
    } else {
        t.push_str(
            "Description\n-----------\n(describe the map, its objective and difficulty)\n\n",
        );
    }
    t.push_str(&format!(
        "Installation\n------------\n{install}\n\nCredits\n-------\n\
            (list testers, tools and resources used)\n"
    ));
    t
}

// ---------------------------------------------------------------- pre-flight

fn looks_absolute(line: &str) -> bool {
    let b = line.as_bytes();
    for i in 0..b.len().saturating_sub(2) {
        if b[i].is_ascii_alphabetic()
            && b[i + 1] == b':'
            && (b[i + 2] == b'\\' || b[i + 2] == b'/')
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric())
        {
            return true;
        }
    }
    line.contains("/home/") || line.contains("/Users/") || line.contains("\\Users\\")
}

fn preflight(entries: &[PackageEntry], opts: &PackageOptions, subject: &Subject) -> Preflight {
    let mut issues: Vec<Issue> = opts.host_issues.clone();
    let inc: Vec<&PackageEntry> = entries.iter().filter(|e| e.included).collect();
    let total: u64 = inc.iter().map(|e| e.size).sum();
    // missing files (levels)
    let level_numbers: BTreeSet<u32> = inc
        .iter()
        .filter_map(|e| crate::classify::level_number(e.path.rsplit('/').next().unwrap_or("")))
        .collect();
    for n in &level_numbers {
        let has = |exts: &[&str]| {
            inc.iter().any(|e| {
                let name = e.path.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
                exts.iter().any(|x| name == format!("map{n:05}.{x}"))
            })
        };
        for (exts, sev, what) in [
            (&["slb"][..], Severity::Error, "slabs (.slb)"),
            (&["own"][..], Severity::Error, "ownership (.own)"),
            (&["clm"][..], Severity::Error, "column definitions (.clm)"),
            (&["dat"][..], Severity::Error, "column grid (.dat)"),
            (&["tng", "tngfx"][..], Severity::Warning, "things (.tng)"),
            (
                &["apt", "aptfx"][..],
                Severity::Warning,
                "action points (.apt)",
            ),
            (&["lgt", "lgtfx"][..], Severity::Warning, "lights (.lgt)"),
            (&["wib"][..], Severity::Warning, "wibble (.wib)"),
            (&["inf"][..], Severity::Warning, "texture set (.inf)"),
            (
                &["txt", "lua"][..],
                Severity::Warning,
                "script (.txt or .lua)",
            ),
        ] {
            if !has(exts) {
                issues.push(Issue {
                    severity: sev,
                    message: format!("level {n} has no {what}"),
                    path: None,
                });
            }
        }
        if matches!(subject, Subject::Level { .. }) && !has(&["lof", "lif"]) {
            issues.push(Issue::new(
                Severity::Warning,
                format!("level {n} has neither .lof nor .lif: it will not be listed in the game"),
            ));
        }
        if *n > 32766 {
            issues.push(Issue::new(
                Severity::Error,
                format!("level number {n} is above 32766, the game ignores it"),
            ));
        }
    }
    if level_numbers.is_empty() && matches!(subject, Subject::Level { .. }) {
        issues.push(Issue::new(Severity::Error, "no level files selected"));
    }
    // files the game does not look for, absolute paths in configs
    for e in &inc {
        let name = e.path.rsplit('/').next().unwrap_or("");
        let lower = name.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("map").and_then(|r| r.get(5..)) {
            let ext = rest.trim_start_matches('.');
            if level_role(ext) == "other" && !lower.contains("readme") {
                issues.push(Issue::at(
                    Severity::Warning,
                    &e.path,
                    format!("{name}: the game does not read this kind of file"),
                ));
            }
        } else if matches!(subject, Subject::Level { .. })
            && e.role == "map"
            && !lower.ends_with("_readme.txt")
            && !matches!(
                lower.rsplit('.').next(),
                Some("txt" | "cfg" | "toml" | "lua" | "png" | "jpg")
            )
        {
            issues.push(Issue::at(
                Severity::Info,
                &e.path,
                format!("{name}: not a level file; kept because it sits in the level folder"),
            ));
        }
        if lower.ends_with(".kfxed.toml") {
            issues.push(Issue::at(
                Severity::Info,
                &e.path,
                "editor data (.kfxed.toml) included: it holds custom slabs, notes and \
                    locks (not used by the game)",
            ));
        }
        let texty = matches!(
            lower.rsplit('.').next(),
            Some("cfg" | "toml" | "txt" | "lua" | "lof" | "lif")
        ) && e.size <= 512 * 1024;
        if texty && e.role != "readme" {
            let bytes = match &e.source {
                EntrySource::File(p) => std::fs::read(p).unwrap_or_default(),
                EntrySource::Text(t) => t.clone(),
            };
            let text = kfx_formats::cfg::latin1_decode(&bytes);
            for (ln, l) in text.lines().enumerate() {
                let t = l.trim();
                if t.starts_with(';') || t.starts_with("--") || t.starts_with('#') {
                    continue;
                }
                let is_loc = t.to_ascii_uppercase().contains("_LOCATION")
                    && t.split_once('=').is_some_and(|(_, v)| {
                        v.trim().starts_with('/') || v.trim().chars().nth(1) == Some(':')
                    });
                if is_loc
                    || (lower.ends_with(".cfg") || lower.ends_with(".toml")) && looks_absolute(l)
                {
                    issues.push(Issue::at(
                        Severity::Error,
                        &e.path,
                        format!(
                            "{name}:{}: absolute path ({})",
                            ln + 1,
                            t.chars().take(60).collect::<String>()
                        ),
                    ));
                    break;
                }
            }
        }
    }
    // size report
    let mut by_role: std::collections::BTreeMap<String, u64> = Default::default();
    for e in &inc {
        *by_role.entry(e.role.to_string()).or_default() += e.size;
    }
    let mut by_role: Vec<(String, u64)> = by_role.into_iter().collect();
    by_role.sort_by(|a, b| b.1.cmp(&a.1));
    issues.push(Issue::new(
        Severity::Info,
        format!("{} files, {} unpacked", inc.len(), format_size(total)),
    ));
    if total > 64 << 20 {
        issues.push(Issue::new(
            Severity::Warning,
            format!(
                "{} is large; the Workshop's upload limit is unknown to the \
                    editor, check it before uploading",
                format_size(total)
            ),
        ));
    }
    if let Some((role, sz)) = by_role.first() {
        if *sz > 16 << 20 && by_role.len() > 1 {
            issues.push(Issue::new(
                Severity::Info,
                format!("most of the size is '{role}' ({})", format_size(*sz)),
            ));
        }
    }
    issues.sort_by(|a, b| b.severity.cmp(&a.severity));
    Preflight {
        issues,
        total_size: total,
        by_role,
    }
}

// ---------------------------------------------------------------- writing

/// Write the zip. Refuses when the pre-flight has errors unless `force`. The zip is
/// deterministic (fixed timestamps, sorted entries) and is read back through the installer's
/// own archive code as a self-check.
pub fn write_zip(plan: &PackagePlan, dest: &Path, force: bool) -> Result<Packaged> {
    use std::io::Write;
    if plan.preflight.has_errors() && !force {
        return Err(Error::Other(format!(
            "pre-flight found {} error(s); fix them or force the packaging",
            plan.preflight.count(Severity::Error)
        )));
    }
    let mut files: Vec<&PackageEntry> = plan.included().collect();
    files.sort_by(|a, b| {
        // readme first, then by path
        (a.role != "readme", a.path.to_ascii_lowercase())
            .cmp(&(b.role != "readme", b.path.to_ascii_lowercase()))
    });
    let mut seen = BTreeSet::new();
    let tmp = dest.with_extension("zip.part");
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| Error::io(p, e))?;
    }
    {
        let f = std::fs::File::create(&tmp).map_err(|e| Error::io(&tmp, e))?;
        let mut zw = zip::ZipWriter::new(std::io::BufWriter::new(f));
        for e in &files {
            if e.path.is_empty() || archive::safe_rel_path(&e.path).is_err() {
                return Err(Error::Unsafe(format!(
                    "refusing to package path {:?}",
                    e.path
                )));
            }
            if !seen.insert(e.path.to_ascii_lowercase()) {
                return Err(Error::Other(format!("{} would be packaged twice", e.path)));
            }
            let stored = e.path.to_ascii_lowercase().ends_with(".zip")
                || e.path.to_ascii_lowercase().ends_with(".png");
            let o = zip::write::SimpleFileOptions::default()
                .compression_method(if stored {
                    zip::CompressionMethod::Stored
                } else {
                    zip::CompressionMethod::Deflated
                })
                .last_modified_time(zip::DateTime::default())
                .unix_permissions(0o644);
            zw.start_file(e.path.as_str(), o)
                .map_err(|x| Error::Other(x.to_string()))?;
            match &e.source {
                EntrySource::File(p) => {
                    let mut r = std::fs::File::open(p).map_err(|x| Error::io(p, x))?;
                    std::io::copy(&mut r, &mut zw).map_err(|x| Error::io(p, x))?;
                }
                EntrySource::Text(t) => zw.write_all(t).map_err(|x| Error::io(&tmp, x))?,
            }
        }
        zw.finish().map_err(|x| Error::Other(x.to_string()))?;
    }
    std::fs::rename(&tmp, dest).map_err(|e| Error::io(dest, e))?;
    let bytes = std::fs::read(dest).map_err(|e| Error::io(dest, e))?;
    let insp = crate::classify::classify_archive(dest, &Limits::default())?;
    let preview = match &plan.opts.preview_image {
        Some(img) => {
            let ext = img
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_else(|| "png".into());
            let p = dest.with_extension(format!("preview.{ext}"));
            std::fs::copy(img, &p).map_err(|e| Error::io(img, e))?;
            Some(p)
        }
        None => None,
    };
    Ok(Packaged {
        zip: dest.to_path_buf(),
        size: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
        files: files.len(),
        preview,
        kind: insp.class.kind,
    })
}

/// Text for the Workshop upload form (title, category, description, installation).
pub fn upload_form_text(plan: &PackagePlan) -> String {
    let cat = match (
        &plan.subject,
        plan.entries
            .iter()
            .any(|e| e.path.to_ascii_lowercase().starts_with("multiplayer/")),
    ) {
        (Subject::Level { .. }, true) => "Multiplayer Map",
        (Subject::Level { .. }, false) => "Map",
        (Subject::Campaign { cfg, .. }, _) => {
            if kfx_formats::campaign::kind_of(cfg)
                == Some(kfx_formats::campaign::PackKind::Campaign)
            {
                "Campaign"
            } else {
                "Map Pack"
            }
        }
    };
    format!(
        "Title: {}\nCategory: {cat}\nFile: {}\n\nDescription:\n{}\n\n\
            Installation instructions:\nExtract the archive into your KeeperFX folder \
            (levels with no folder go into a subfolder of levels/).\n",
        plan.opts.title, plan.zip_name, plan.opts.description
    )
}
