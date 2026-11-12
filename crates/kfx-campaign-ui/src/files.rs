//! Pack-level file operations: create from a template, rename, duplicate, package as a zip.
//!
//! Every file this module writes goes through [`kfx_formats::atomic`]. Folders are renamed with
//! `std::fs::rename` (with rollback).
use crate::pack::*;
use crate::texts::NEW_TABLE_STRINGS;
use kfx_formats::atomic::{self, SaveTransaction};
use kfx_formats::campaign::PackKind;
use kfx_formats::strings::{Lang, StringTable};
use kfx_formats::{Error, Result};
use std::path::{Path, PathBuf};

fn err(msg: impl Into<String>) -> Error {
    Error::format("campaign", msg)
}

/// A pack name is the cfg file name without `.cfg` and the start of its folder names: ASCII
/// letters, digits, `_` and `-`, at most 40 characters.
pub fn valid_pack_name(name: &str) -> std::result::Result<(), String> {
    if name.is_empty() {
        return Err("the name is empty".into());
    }
    if name.len() > 40 {
        return Err("the name is longer than 40 characters".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("use letters, digits, _ and - only".into());
    }
    Ok(())
}

/// What a new pack is made of.
#[derive(Clone, Debug)]
pub struct NewPack {
    /// File stem (`mycampaign` gives `campgns/mycampaign.cfg`).
    pub name: String,
    /// `NAME` (shown in the menu).
    pub display_name: String,
    pub kind: PackKind,
    /// The cfg of an existing campaign of the install whose land view is used. For a campaign
    /// its land folder is copied into `<name>_lnd` (never bundled with the editor); for a map
    /// pack the land location is referenced.
    pub land_from: Option<PathBuf>,
    /// Languages with a strings table to create; English is always added first.
    pub languages: Vec<Lang>,
}

impl NewPack {
    pub fn campaign(name: &str, display_name: &str) -> NewPack {
        NewPack {
            name: name.into(),
            display_name: display_name.into(),
            kind: PackKind::Campaign,
            land_from: None,
            languages: vec![Lang::English],
        }
    }
}

/// Create a pack on disk: the cfg, the folders and empty strings tables, with the land view
/// copied from `spec.land_from`. Nothing existing is overwritten.
pub fn create_pack(install: &Path, spec: &NewPack) -> Result<Pack> {
    valid_pack_name(&spec.name).map_err(|m| err(format!("pack name: {m}")))?;
    let group = spec.kind.folder();
    let group_dir = resolve_loose(install, group);
    let cfg_path = group_dir.join(format!("{}.cfg", spec.name));
    if existing(install, &format!("{group}/{}.cfg", spec.name)).is_some() {
        return Err(err(format!("{} already exists", cfg_path.display())));
    }
    let name = &spec.name;
    let levels_loc = format!("{group}/{name}");
    let land_loc = format!("{group}/{name}_lnd");
    let speech_loc = format!("{group}/{name}_eng");
    for loc in [&levels_loc, &land_loc] {
        if let Some(p) = existing(install, loc) {
            if std::fs::read_dir(&p)
                .map(|mut r| r.next().is_some())
                .unwrap_or(false)
            {
                return Err(err(format!(
                    "{} already exists and is not empty",
                    p.display()
                )));
            }
        }
    }
    let mut langs = vec![Lang::English];
    for l in &spec.languages {
        if !langs.contains(l) {
            langs.push(*l);
        }
    }
    // the land view source
    let source = match &spec.land_from {
        Some(c) => Some(Pack::load(install, c)?),
        None => None,
    };
    let src_common = source.as_ref().map(|s| s.common());
    let mut text = String::new();
    let head = if spec.kind == PackKind::Campaign {
        "KeeperFX campaign file"
    } else {
        "KeeperFX Mappack file"
    };
    text.push_str(&format!(
        "; {head}\n; Created with the KeeperFX map editor.\n\n[common]\n"
    ));
    text.push_str(&format!("NAME = {}\n", spec.display_name));
    text.push_str("; Folders storing data files\n");
    text.push_str(&format!("LEVELS_LOCATION = {levels_loc}\n"));
    let is_campaign = spec.kind == PackKind::Campaign;
    let mut copy_land_from: Option<PathBuf> = None;
    if let (Some(s), Some(c)) = (&source, &src_common) {
        let src_land = s.land_dir();
        if is_campaign {
            text.push_str(&format!("LAND_LOCATION = {land_loc}\n"));
            copy_land_from = src_land.filter(|d| d.is_dir());
        } else if let Some(l) = &c.land_location {
            text.push_str(&format!("LAND_LOCATION = {l}\n"));
        }
        if let Some(m) = &c.media_location {
            text.push_str(&format!("MEDIA_LOCATION = {m}\n"));
        }
    } else if is_campaign {
        text.push_str(&format!("LAND_LOCATION = {land_loc}\n"));
    }
    if is_campaign {
        text.push_str(
            "; Single player level numbers, and bonus levels for 'Reveal \
                hidden land' boxes (0 = none;\n; the two lists are parallel). Extra levels: \
                full moon, new moon. The editor writes the lists.\n;SINGLE_LEVELS = 1 2 3\n\
                ;BONUS_LEVELS = 0 0 0\n;EXTRA_LEVELS = 4 5\n",
        );
    }
    text.push_str(&format!(
        "HIGH_SCORES = 1024 scr_{}.dat\n",
        name.chars().take(8).collect::<String>()
    ));
    if let Some(c) = &src_common {
        if is_campaign {
            if let Some((a, b)) = &c.land_view_start {
                text.push_str(&format!("LAND_VIEW_START = {a} {b}\n"));
            }
            if let Some((a, b)) = &c.land_view_end {
                text.push_str(&format!("LAND_VIEW_END = {a} {b}\n"));
            }
            if let Some(cr) = &c.credits {
                text.push_str(&format!("CREDITS = {cr}\n"));
            }
            if let Some((g, b)) = c.land_ambient {
                text.push_str(&format!("LAND_AMBIENT = {g} {b}\n"));
            }
            text.push_str(
                "; The sprites used for marking locations which can be \
                    clicked. ENSIGNS or PINPOINTS.\n",
            );
            text.push_str(&format!(
                "LAND_MARKERS = {}\n",
                c.land_markers.clone().unwrap_or_else(|| "ENSIGNS".into())
            ));
        } else if let Some((a, b)) = &c.land_view_start {
            text.push_str(&format!(
                "LAND_VIEW_START = {a} {b}\nLAND_MARKERS = ENSIGNS\n"
            ));
        }
    }
    text.push_str(
        "; Player number to use for the human player. RED BLUE GREEN \
            YELLOW WHITE NEUTRAL PURPLE BLACK ORANGE\nHUMAN_PLAYER = RED\n",
    );
    text.push_str(
        "\n; Text strings file used for ingame info messages and objectives.\n[strings]\n",
    );
    for l in &langs {
        text.push_str(&format!(
            "{} = {levels_loc}/text_{}.dat\n",
            l.code(),
            l.lower()
        ));
    }
    if is_campaign {
        text.push_str("\n; Location of the speech sound files.\n[speech]\n");
        text.push_str(&format!("ENG = {speech_loc}\n"));
        text.push_str(
            "\n; Details about single player levels follow, one [mapNNNNN] block per level.\n",
        );
    }
    // write: folders and tables first, the cfg last (the pack appears only when complete)
    let levels_dir = resolve_loose(install, &levels_loc);
    let mut tx = SaveTransaction::new(&levels_dir);
    for l in &langs {
        let mut t = StringTable::new(*l);
        t.set_raw(NEW_TABLE_STRINGS - 1, vec![]);
        tx.write(format!("text_{}.dat", l.lower()), t.to_bytes());
    }
    tx.commit()?;
    if is_campaign {
        let speech_dir = resolve_loose(install, &speech_loc);
        std::fs::create_dir_all(&speech_dir).map_err(|e| Error::io(&speech_dir, e))?;
        let land_dir = resolve_loose(install, &land_loc);
        match copy_land_from {
            Some(src) => {
                copy_dir_files(&src, &land_dir)?;
            }
            None => std::fs::create_dir_all(&land_dir).map_err(|e| Error::io(&land_dir, e))?,
        }
    }
    let mut tx = SaveTransaction::new(&group_dir);
    tx.write(format!("{name}.cfg"), text.into_bytes());
    tx.commit()?;
    Pack::load(install, &cfg_path)
}

/// Copy the regular files of `src` (not sub folders) into `dst`, in one transaction.
pub fn copy_dir_files(src: &Path, dst: &Path) -> Result<Vec<PathBuf>> {
    let mut tx = SaveTransaction::new(dst);
    let mut names: Vec<(String, PathBuf)> = std::fs::read_dir(src)
        .map_err(|e| Error::io(src, e))?
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| !atomic::is_scratch_name(n))
        .collect();
    names.sort();
    for (n, p) in &names {
        tx.write(n.clone(), std::fs::read(p).map_err(|e| Error::io(p, e))?);
    }
    Ok(tx.commit()?.written)
}

fn copy_tree(src: &Path, dst: &Path, skip: &dyn Fn(&Path) -> bool) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(|e| Error::io(dst, e))?;
    let mut subs = vec![];
    let mut tx = SaveTransaction::new(dst);
    let mut any = false;
    for e in std::fs::read_dir(src)
        .map_err(|e| Error::io(src, e))?
        .flatten()
    {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        if skip(&p) || atomic::is_scratch_name(&n) {
            continue;
        }
        if p.is_dir() {
            subs.push((p, dst.join(&n)));
        } else if p.is_file() {
            tx.write(n, std::fs::read(&p).map_err(|e| Error::io(&p, e))?);
            any = true;
        }
    }
    if any {
        tx.commit()?;
    }
    for (s, d) in subs {
        copy_tree(&s, &d, skip)?;
    }
    Ok(())
}

/// The folders that belong to a pack: every folder next to its levels folder (`campgns/`,
/// `levels/` ...) named `<stem>` or `<stem>_...`, plus the locations the cfg names that follow
/// the same rule. Existing folders only.
pub fn own_folders(pack: &Pack) -> Vec<PathBuf> {
    let stem = pack.stem().to_ascii_lowercase();
    let group = resolve_loose(&pack.install, pack.kind.folder());
    let mut out: Vec<PathBuf> = vec![];
    if let Ok(rd) = std::fs::read_dir(&group) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            if e.path().is_dir() && (n == stem || n.starts_with(&format!("{stem}_"))) {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out
}

/// A folder name mapped from one pack name to another: `old` -> `new`, `old_lnd` -> `new_lnd`.
fn retarget_name(name: &str, old: &str, new: &str) -> Option<String> {
    let l = name.to_ascii_lowercase();
    let o = old.to_ascii_lowercase();
    if l == o || l.starts_with(&format!("{o}_")) {
        Some(format!("{new}{}", &name[old.len()..]))
    } else {
        None
    }
}

fn retarget_value(v: &str, old: &str, new: &str) -> Option<String> {
    let norm = v.replace('\\', "/");
    let mut parts: Vec<String> = norm.split('/').map(str::to_string).collect();
    if parts.len() >= 2 && PackKind::from_folder(&parts[0]).is_some() {
        if let Some(n) = retarget_name(&parts[1], old, new) {
            parts[1] = n;
            let r = parts.join("/");
            return (r != norm).then_some(r);
        }
    }
    None
}

/// Rewrite the locations that point into the folders of pack `old` so they point into `new`.
fn retarget_cfg(pack: &mut Pack, old: &str, new: &str) {
    let mut changes: Vec<(&'static str, String, String)> = vec![];
    for key in [
        "LEVELS_LOCATION",
        "LAND_LOCATION",
        "CREATURES_LOCATION",
        "CONFIGS_LOCATION",
        "MEDIA_LOCATION",
    ] {
        if let Some(v) = pack.cfg.doc.get("common", key) {
            if let Some(n) = retarget_value(&v, old, new) {
                changes.push(("common", key.to_string(), n));
            }
        }
    }
    for sec in ["strings", "speech"] {
        for e in pack.cfg.doc.entries(sec) {
            if let Some(n) = retarget_value(&e.value, old, new) {
                let sec: &'static str = if sec == "strings" {
                    "strings"
                } else {
                    "speech"
                };
                changes.push((sec, e.key.clone(), n));
            }
        }
    }
    for (sec, key, val) in changes {
        pack.cfg.doc.set(sec, &key, &val);
    }
}

/// Copy a pack under a new name: the cfg and every own folder (`<old>` -> `<new>`,
/// `<old>_lnd` -> `<new>_lnd`), with the locations in the cfg rewritten. With `without_levels`
/// the level lists, level blocks and `mapNNNNN.*` files are left out.
pub fn duplicate_pack(
    install: &Path,
    src_cfg: &Path,
    new_name: &str,
    without_levels: bool,
) -> Result<Pack> {
    valid_pack_name(new_name).map_err(|m| err(format!("pack name: {m}")))?;
    let mut pack = Pack::load(install, src_cfg)?;
    let old = pack.stem();
    let group_dir = src_cfg.parent().map(Path::to_path_buf).unwrap_or_default();
    let new_cfg = group_dir.join(format!("{new_name}.cfg"));
    if existing(install, &format!("{}/{new_name}.cfg", pack.kind.folder())).is_some() {
        return Err(err(format!("{} already exists", new_cfg.display())));
    }
    let folders = own_folders(&pack);
    let mut targets = vec![];
    for f in &folders {
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let Some(nn) = retarget_name(&name, &old, new_name) else {
            continue;
        };
        let dest = group_dir.join(&nn);
        if dest.exists() {
            return Err(err(format!("{} already exists", dest.display())));
        }
        targets.push((f.clone(), dest));
    }
    let levels_src = pack.levels_dir();
    retarget_cfg(&mut pack, &old, new_name);
    if without_levels {
        pack.set_single_slots(&[]);
        pack.set_extra_levels(&[]);
        for n in pack.cfg.map_block_numbers() {
            pack.cfg.remove_map_block(n);
        }
    }
    let skip = move |p: &Path| {
        without_levels
            && p.is_file()
            && p.parent() == levels_src.as_deref()
            && p.file_name()
                .is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().starts_with("map"))
    };
    let mut made: Vec<PathBuf> = vec![];
    for (s, d) in &targets {
        if let Err(e) = copy_tree(s, d, &skip) {
            for m in &made {
                let _ = std::fs::remove_dir_all(m);
            }
            let _ = std::fs::remove_dir_all(d);
            return Err(e);
        }
        made.push(d.clone());
    }
    pack.cfg_path = new_cfg.clone();
    if let Err(e) = pack.save() {
        for m in &made {
            let _ = std::fs::remove_dir_all(m);
        }
        return Err(e);
    }
    Pack::load(install, &new_cfg)
}

/// Rename a pack: the cfg and its own folders get the new name, the cfg locations follow. The
/// moves are undone when one fails.
pub fn rename_pack(install: &Path, cfg: &Path, new_name: &str) -> Result<Pack> {
    valid_pack_name(new_name).map_err(|m| err(format!("pack name: {m}")))?;
    let mut pack = Pack::load(install, cfg)?;
    let old = pack.stem();
    if old.eq_ignore_ascii_case(new_name) {
        return Err(err("the new name is the same"));
    }
    let group_dir = cfg.parent().map(Path::to_path_buf).unwrap_or_default();
    let new_cfg = group_dir.join(format!("{new_name}.cfg"));
    if existing(install, &format!("{}/{new_name}.cfg", pack.kind.folder())).is_some() {
        return Err(err(format!("{} already exists", new_cfg.display())));
    }
    let mut moves = vec![];
    for f in own_folders(&pack) {
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if let Some(nn) = retarget_name(&name, &old, new_name) {
            let dest = group_dir.join(nn);
            if dest.exists() {
                return Err(err(format!("{} already exists", dest.display())));
            }
            moves.push((f, dest));
        }
    }
    retarget_cfg(&mut pack, &old, new_name);
    let bytes = pack.cfg.to_bytes();
    let mut tx = SaveTransaction::new(&group_dir);
    tx.write(format!("{new_name}.cfg"), bytes);
    tx.commit()?;
    let mut done: Vec<(PathBuf, PathBuf)> = vec![];
    for (from, to) in &moves {
        if let Err(e) = std::fs::rename(from, to) {
            for (f, t) in done.iter().rev() {
                let _ = std::fs::rename(t, f);
            }
            let _ = std::fs::remove_file(&new_cfg);
            return Err(Error::io(from, e));
        }
        done.push((from.clone(), to.clone()));
    }
    std::fs::remove_file(cfg).map_err(|e| Error::io(cfg, e))?;
    Pack::load(install, &new_cfg)
}

/// Options of [`package`].
#[derive(Clone, Copy, Debug, Default)]
pub struct PackageOptions {
    /// Also pack folders of other campaigns / map packs that the cfg points at (shared speech,
    /// creatures, configs). Folders outside `campgns/`, `levels/` and `multiplayer/` (for
    /// example the game's `ldata`) are never packed.
    pub include_external: bool,
    /// Pack the editor's `mapNNNNN.kfxed.toml` sidecars (the game never reads them).
    pub include_sidecar: bool,
}

/// What [`package`] wrote.
#[derive(Clone, Debug, Default)]
pub struct PackageReport {
    /// Paths inside the zip (relative to the install root).
    pub files: Vec<String>,
    pub bytes: u64,
    /// Locations the cfg points at that were not packed, with the reason.
    pub not_packed: Vec<(String, String)>,
}

/// Pack a campaign or map pack into a zip that unpacks into an install root: the cfg as
/// `campgns/<name>.cfg` (or `levels/`, `multiplayer/`) and every own folder below the same
/// group folder (`campgns/<name>/`, `campgns/<name>_lnd/`, speech, creatures, configs). Paths use
/// `/`, entries are sorted and carry no timestamps, so the same pack gives the same zip.
pub fn package(pack: &Pack, out: &Path, opts: PackageOptions) -> Result<PackageReport> {
    use std::io::Write;
    let mut report = PackageReport::default();
    let mut dirs: Vec<PathBuf> = own_folders(pack);
    let common = pack.common();
    let stem = pack.stem();
    let mut check = |key: &str, loc: &str, report: &mut PackageReport| {
        let p = pack.loc(loc);
        if !p.is_dir() || dirs.iter().any(|d| d == &p) {
            return;
        }
        let in_group = rel_to_install(&pack.install, &p)
            .and_then(|r| r.split('/').next().and_then(PackKind::from_folder))
            .is_some();
        let own = is_own_location(&stem, loc);
        if own || (opts.include_external && in_group) {
            dirs.push(p);
        } else if in_group {
            report
                .not_packed
                .push((format!("{key} = {loc}"), "belongs to another pack".into()));
        } else {
            report
                .not_packed
                .push((format!("{key} = {loc}"), "a game folder".into()));
        }
    };
    for (key, v) in [
        ("LEVELS_LOCATION", &common.levels_location),
        ("LAND_LOCATION", &common.land_location),
        ("CREATURES_LOCATION", &common.creatures_location),
        ("CONFIGS_LOCATION", &common.configs_location),
        ("MEDIA_LOCATION", &common.media_location),
    ] {
        if let Some(v) = v {
            check(key, v, &mut report);
        }
    }
    for (code, f) in pack.cfg.speech() {
        check(&format!("[speech] {code}"), &f, &mut report);
    }
    // collect (zip path, disk path)
    let mut items: Vec<(String, PathBuf)> = vec![];
    let cfg_name = pack
        .cfg_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    items.push((
        format!("{}/{cfg_name}", pack.kind.folder()),
        pack.cfg_path.clone(),
    ));
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.is_file() {
                    out.push(p);
                }
            }
        }
    }
    for d in &dirs {
        let mut files = vec![];
        walk(d, &mut files);
        for f in files {
            let name = f
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if atomic::is_scratch_name(&name)
                || (!opts.include_sidecar && name.to_ascii_lowercase().ends_with(".kfxed.toml"))
            {
                continue;
            }
            if let Some(rel) = rel_to_install(&pack.install, &f) {
                items.push((rel, f));
            }
        }
    }
    items.sort();
    items.dedup_by(|a, b| a.0 == b.0);
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        for (zname, disk) in &items {
            let bytes = std::fs::read(disk).map_err(|e| Error::io(disk, e))?;
            zw.start_file(zname.as_str(), options)
                .map_err(|e| err(format!("zip: {e}")))?;
            zw.write_all(&bytes).map_err(|e| Error::io(out, e))?;
            report.bytes += bytes.len() as u64;
            report.files.push(zname.clone());
        }
        zw.finish().map_err(|e| err(format!("zip: {e}")))?;
    }
    atomic::atomic_write(out, buf.get_ref()).map_err(|e| Error::io(out, e))?;
    Ok(report)
}

/// A level of the install that can be copied into a pack.
#[derive(Clone, Debug)]
pub struct SourceEntry {
    /// Name of the pack the level belongs to (`keeporig`).
    pub pack: String,
    /// `keeporig 7 Eversmile`
    pub label: String,
    pub source: LevelSource,
}

/// Every level that has a map file in a campaign, map pack or multiplayer pack of the install.
pub fn install_levels(install: &Path) -> Vec<SourceEntry> {
    let mut out = vec![];
    for g in ["campgns", "levels", "multiplayer"] {
        let dir = resolve_loose(install, g);
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut cfgs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")))
            .collect();
        cfgs.sort();
        for cfg in cfgs {
            let Ok(r) = kfx_formats::campaign::resolve(install, &cfg) else {
                continue;
            };
            let Some(levels_dir) = r.levels_dir.clone() else {
                continue;
            };
            let camp = (r.kind == PackKind::Campaign)
                .then(|| kfx_formats::campaign::Campaign::load(&cfg).ok())
                .flatten();
            let stem = cfg
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            for l in &r.levels {
                if l.slb.is_none() {
                    continue;
                }
                let mut src = LevelSource::new(&levels_dir, l.number);
                src.info = camp.as_ref().and_then(|c| c.map_info(l.number));
                out.push(SourceEntry {
                    label: format!("{stem} {} {}", l.number, l.name.clone().unwrap_or_default()),
                    pack: stem.clone(),
                    source: src,
                });
            }
        }
    }
    out
}
