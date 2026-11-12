//! Headless commands (no window): `validate`, `convert`, `regenerate`, `export-image`,
//! `script-check`, `info`. Each takes a map file (`mapNNNNN.slb`, or any file of the level).
use crate::document::{self, Document};
use crate::scripts::{self, LevelFiles, MapConsts};
use crate::settings::{pick_install, Settings};
use kfx_map::gamedata::GameData;
use kfx_map::validate::Severity;
use kfx_script_ui::workspace::{ScriptFiles, ScriptWorkspace};
use kfx_script_ui::Language;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const COMMANDS: &[&str] = &[
    "validate",
    "convert",
    "regenerate",
    "export-image",
    "script-check",
    "info",
];

pub const USAGE: &str = "\
commands (no window; MAP is a mapNNNNN.slb or another file of the level):
  kfx-editor validate MAP [--json]       map checks and map <-> script links; exit 1 on errors
  kfx-editor convert MAP --to fx|legacy  rewrite the map in the FX format (.tngfx/.aptfx/.lgtfx) or
                                         the original .tng/.apt/.lgt (exit 1 when it cannot hold it)
  kfx-editor regenerate MAP [--decorate] regenerate columns, slab objects and liquids, then save
  kfx-editor export-image MAP OUT.png [--iso] [--scale N] [--no-things]
                                         render the whole map (N pixels per slab, default 16;
                                         top-down unless --iso)
  kfx-editor script-check MAP [--json]   DK script and Lua diagnostics; exit 1 on errors
  kfx-editor info MAP [--json]           size, keepers, things, scripts, format, campaign
Maps are backed up to the editor's backups folder before convert / regenerate overwrite them.
A map with a file the editor cannot read (a .tngfx/.aptfx/.lgtfx or sidecar) is read-only: the
problem is printed, convert / regenerate refuse to write, and every command exits non-zero.";

/// Set when a loaded map has files the editor could not read (the exit code is then not 0).
static UNREADABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct Loaded {
    install: PathBuf,
    doc: Document,
}

fn load(map: &Path) -> Result<Loaded, String> {
    let settings = Settings::load();
    let install = pick_install(&settings, Some(map))
        .ok_or("no KeeperFX install: set KFX_INSTALL, or keep the map inside an install")?;
    let level = document::map_number(map)
        .ok_or_else(|| format!("{}: not a mapNNNNN file", map.display()))?;
    let folder = map
        .parent()
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    let mods = if settings.ignore_mods {
        Default::default()
    } else {
        kfx_map::mods::ModsConfig::load(&install)
            .map(|m| m.for_game(false))
            .unwrap_or_default()
    };
    let loc = document::locate_with(&install, &folder, level, &mods)?;
    let gd =
        Arc::new(GameData::load_stack(&loc.stack).map_err(|e| format!("game configuration: {e}"))?);
    let mut doc = Document::load(&loc, gd)?;
    doc.backups = crate::files::SessionBackups::new(Settings::backup_dir());
    for u in &doc.doc.unreadable {
        eprintln!(
            "kfx-editor: {} ({}) could not be read: {}",
            u.path.display(),
            u.what,
            u.error
        );
    }
    if let Some(why) = doc.doc.read_only_reason() {
        UNREADABLE.store(true, std::sync::atomic::Ordering::Relaxed);
        eprintln!("kfx-editor: the map is {why}");
    }
    Ok(Loaded { install, doc })
}

fn workspace(l: &Loaded) -> (ScriptWorkspace, LevelFiles, MapConsts) {
    let d = &l.doc;
    let mut files = LevelFiles::new(&l.install, &d.doc.folder, d.doc.level, &d.layers);
    files.backups = d.backups.clone();
    let mut consts = MapConsts::new(&d.gd, &d.layers);
    consts.set_map(&d.doc, &d.gd);
    let mut ws = ScriptWorkspace::new(d.doc.level);
    ws.set_lua_index(Arc::new(scripts::lua_index(&d.doc.folder, &d.layers)));
    ws.load(&files);
    (ws, files, consts)
}

fn sev(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}

fn backup(doc: &Document) -> Result<Option<PathBuf>, String> {
    let dir = Settings::backup_dir().ok_or("no config directory for backups")?;
    document::backup_level(&doc.doc.folder, doc.doc.level, &dir)
}

/// Run a command; returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    match run_inner(args) {
        Ok(code) if UNREADABLE.load(std::sync::atomic::Ordering::Relaxed) => code.max(1),
        Ok(code) => code,
        Err(e) => {
            eprintln!("kfx-editor: {e}");
            2
        }
    }
}

fn run_inner(args: &[String]) -> Result<i32, String> {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let pos: Vec<&String> = {
        let mut v = vec![];
        let mut skip = false;
        for a in &args[1..] {
            if skip {
                skip = false;
                continue;
            }
            if a == "--to" || a == "--scale" {
                skip = true;
                continue;
            }
            if !a.starts_with("--") {
                v.push(a);
            }
        }
        v
    };
    let map = PathBuf::from(
        pos.first()
            .ok_or_else(|| format!("{cmd} needs a map (see --help)"))?
            .as_str(),
    );
    let json = flag("--json");
    match cmd {
        "validate" => {
            let l = load(&map)?;
            let d = &l.doc;
            let mut v = kfx_map::validate::validate(&d.doc, &d.gd);
            v.extend(document::extra_issues(&d.doc, &d.gd));
            let (mut ws, _, _) = workspace(&l);
            let refs = ws.map_references();
            let exist = ws.tab_infos().iter().any(|t| t.exists);
            v.extend(
                scripts::link_issues(&d.doc, &d.gd, &refs, exist)
                    .into_iter()
                    .map(|(i, _)| i),
            );
            v.sort_by(|a, b| b.severity.cmp(&a.severity));
            let errors = v.iter().filter(|i| i.severity == Severity::Error).count();
            if json {
                let arr: Vec<serde_json::Value> = v
                    .iter()
                    .map(|i| {
                        serde_json::json!({
                            "severity": sev(i.severity),
                            "code": i.code,
                            "message": i.message,
                            "at": format!("{:?}", i.at)
                        })
                    })
                    .collect();
                let report = serde_json::json!({
                    "map": map.display().to_string(),
                    "errors": errors,
                    "issues": arr
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).unwrap_or_default()
                );
            } else {
                for i in &v {
                    println!("{}: [{}] {}", sev(i.severity), i.code, i.message);
                }
                let w = v.iter().filter(|i| i.severity == Severity::Warning).count();
                println!(
                    "{}: {errors} errors, {w} warnings, {} notes",
                    map.display(),
                    v.len() - errors - w
                );
            }
            Ok(if errors > 0 { 1 } else { 0 })
        }
        "convert" => {
            let to = value("--to").ok_or("convert needs --to fx|legacy")?;
            match to.to_ascii_lowercase().as_str() {
                "fx" => {
                    let mut l = load(&map)?;
                    let was = l.doc.doc.loaded_format;
                    let b = backup(&l.doc)?;
                    let (folder, level) = (l.doc.doc.folder.clone(), l.doc.doc.level);
                    l.doc.save_to(&folder, level)?;
                    println!(
                        "{}: written in the FX format (was {}){}",
                        map.display(),
                        if was == kfx_map::mapdoc::ThingFormat::Fx {
                            "FX"
                        } else {
                            "legacy"
                        },
                        b.map(|b| format!("; previous files backed up to {}", b.display()))
                            .unwrap_or_default()
                    );
                    Ok(0)
                }
                "legacy" => {
                    let mut l = load(&map)?;
                    let rep = l.doc.doc.legacy_report(&l.doc.gd);
                    for w in &rep.warnings {
                        println!("warning: {w}");
                    }
                    if !rep.errors.is_empty() {
                        for e in &rep.errors {
                            eprintln!("error: {e}");
                        }
                        eprintln!(
                            "kfx-editor: {}: not converted, the original format cannot hold \
                            this map",
                            map.display()
                        );
                        return Ok(1);
                    }
                    let was = l.doc.doc.loaded_format;
                    let b = backup(&l.doc)?;
                    let (folder, level) = (l.doc.doc.folder.clone(), l.doc.doc.level);
                    l.doc.save_to_format(&folder, level, true)?;
                    println!(
                        "{}: written in the original DK format .tng/.apt/.lgt (was {}){}",
                        map.display(),
                        if was == kfx_map::mapdoc::ThingFormat::Fx {
                            "FX"
                        } else {
                            "legacy"
                        },
                        b.map(|b| format!("; previous files backed up to {}", b.display()))
                            .unwrap_or_default()
                    );
                    Ok(0)
                }
                x => Err(format!("unknown format {x}: fx or legacy")),
            }
        }
        "regenerate" => {
            let mut l = load(&map)?;
            let decorate = flag("--decorate");
            let dims = l.doc.dims();
            let what = document::Regen {
                columns: true,
                decoration: decorate,
                objects: true,
                liquid: true,
                doodads: true,
            };
            l.doc.regenerate(
                "Regenerate all slabs",
                kfx_map::autotile::SlabRect {
                    x0: 0,
                    y0: 0,
                    x1: dims.w as i32 - 1,
                    y1: dims.h as i32 - 1,
                },
                what,
            );
            let b = backup(&l.doc)?;
            let (folder, level) = (l.doc.doc.folder.clone(), l.doc.doc.level);
            l.doc.save_to(&folder, level)?;
            println!(
                "{}: regenerated columns, slab objects and liquids{}{}",
                map.display(),
                if decorate { " with decoration" } else { "" },
                b.map(|b| format!("; previous files backed up to {}", b.display()))
                    .unwrap_or_default()
            );
            Ok(0)
        }
        "export-image" => {
            let out = PathBuf::from(pos.get(1).ok_or("export-image needs MAP OUT.png")?.as_str());
            let scale: u32 = value("--scale")
                .map(|s| s.parse().map_err(|_| "--scale: not a number"))
                .transpose()?
                .unwrap_or(16);
            let iso = flag("--iso");
            let (w, h, tiles) = crate::headless::export(
                &map,
                &out,
                iso,
                scale.clamp(1, 256),
                !flag("--no-things"),
            )?;
            println!(
                "wrote {} ({w} x {h} pixels, {tiles} tiles, {})",
                out.display(),
                if iso { "isometric" } else { "top-down" }
            );
            Ok(0)
        }
        "script-check" => {
            let l = load(&map)?;
            let (_, files, consts) = workspace(&l);
            let d = &l.doc;
            let mut errors = 0;
            let mut out = vec![];
            let dk_name = kfx_script_ui::workspace::dk_file_name(d.doc.level);
            let lua_name = kfx_script_ui::workspace::lua_file_name(d.doc.level);
            let lua = kfx_script_ui::LuaLanguage::with_index(Arc::new(scripts::lua_index(
                &d.doc.folder,
                &d.layers,
            )));
            let langs: [(&str, &dyn Language); 2] =
                [(&dk_name, &kfx_script_ui::DkLanguage), (&lua_name, &lua)];
            for (name, lang) in langs {
                let Some(bytes) = files.read(name) else {
                    if !json {
                        println!("{name}: none");
                    }
                    continue;
                };
                let a = lang.analyze(&bytes);
                let v = a.validate(&consts);
                for x in &v.diagnostics {
                    if x.severity == kfx_script::Severity::Error {
                        errors += 1;
                    }
                    out.push((name.to_string(), x.clone()));
                }
                if !json {
                    let e = v
                        .diagnostics
                        .iter()
                        .filter(|x| x.severity == kfx_script::Severity::Error)
                        .count();
                    println!(
                        "{name}: {} lines, {e} errors, {} other diagnostics",
                        bytes.split(|&b| b == b'\n').count(),
                        v.diagnostics.len() - e
                    );
                }
            }
            if json {
                let arr: Vec<serde_json::Value> = out
                    .iter()
                    .map(|(f, x)| {
                        serde_json::json!({
                            "file": f,
                            "line": x.line,
                            "severity": format!("{:?}", x.severity).to_ascii_lowercase(),
                            "code": x.code,
                            "message": x.message
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"errors": errors, "diagnostics": arr})
                    )
                    .unwrap_or_default()
                );
            } else {
                for (f, x) in &out {
                    println!(
                        "{f}:{}: {}: {} [{}]",
                        x.line,
                        format!("{:?}", x.severity).to_ascii_lowercase(),
                        x.message,
                        x.code
                    );
                }
            }
            Ok(if errors > 0 { 1 } else { 0 })
        }
        "info" => {
            let l = load(&map)?;
            let d = &l.doc;
            let gd = &d.gd;
            let mut keepers: Vec<(u8, usize)> = vec![];
            let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
            for t in &d.doc.things.items {
                let c = kfx_map::things::thing_class(t);
                *counts.entry(crate::app::class_label(c).en).or_default() += 1;
                if c == Some(kfx_map::things::class::OBJECT)
                    && t.subtype
                        .is_some_and(|s| kfx_map::validate::is_heart_model(gd, s))
                {
                    let o = t.owner.unwrap_or(5).clamp(0, 8) as u8;
                    match keepers.iter_mut().find(|k| k.0 == o) {
                        Some(k) => k.1 += 1,
                        None => keepers.push((o, 1)),
                    }
                }
            }
            keepers.sort();
            let name = crate::files::level_name(&l.install, &d.doc.folder, d.doc.level);
            let target = crate::playtest::play_target(&l.install, &d.doc.folder, d.doc.level);
            let has = |e: &str| {
                kfx_formats::level_bin::find_file(&d.doc.folder, d.doc.level, e).is_some()
            };
            let format = if d.doc.loaded_format == kfx_map::mapdoc::ThingFormat::Fx {
                "FX"
            } else {
                "legacy"
            };
            let (aps, gates) = scripts::map_numbers(&d.doc, gd);
            let campaign = match &target {
                crate::playtest::PlayTarget::Pack { campaign, name, .. } => {
                    format!("{name} (-campaign {campaign})")
                }
                crate::playtest::PlayTarget::Loose => "none (loose map)".into(),
            };
            if json {
                let k: Vec<serde_json::Value> = keepers
                    .iter()
                    .map(|(o, n)| {
                        serde_json::json!({
                            "owner": o,
                            "player": crate::assets::PLAYERS[*o as usize].0.en,
                            "hearts": n
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "map": map.display().to_string(),
                        "level": d.doc.level,
                        "name": name,
                        "size": [d.dims().w, d.dims().h],
                        "keepers": k,
                        "things": counts,
                        "action_points": aps,
                        "hero_gates": gates,
                        "lights": d.doc.lights.items.len(),
                        "dk_script": has("txt"),
                        "lua_script": has("lua"),
                        "format": format,
                        "texture_set": d.doc.texture_id,
                        "campaign": campaign,
                        "config_layers": d
                            .layers
                            .layers
                            .iter()
                            .map(|l| format!("{}/{}", l.dir.display(), l.prefix))
                            .collect::<Vec<_>>()
                    }))
                    .unwrap_or_default()
                );
            } else {
                println!("{}", map.display());
                println!(
                    "  level      map{:05}{}",
                    d.doc.level,
                    name.map(|n| format!(" \"{n}\"")).unwrap_or_default()
                );
                println!("  campaign   {campaign}");
                println!("  size       {} x {} slabs", d.dims().w, d.dims().h);
                println!(
                    "  keepers    {}",
                    if keepers.is_empty() {
                        "none".to_string()
                    } else {
                        keepers
                            .iter()
                            .map(|(o, n)| {
                                format!(
                                    "{}{}",
                                    crate::assets::PLAYERS[*o as usize].0,
                                    if *n > 1 {
                                        format!(" x{n}")
                                    } else {
                                        String::new()
                                    }
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                );
                println!(
                    "  things     {} ({})",
                    d.doc.things.items.len(),
                    counts
                        .iter()
                        .map(|(k, v)| format!("{v} {k}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!("  act.points {} {:?}", aps.len(), aps);
                println!("  hero gates {:?}", gates);
                println!("  lights     {}", d.doc.lights.items.len());
                println!(
                    "  scripts    DK {}, Lua {}",
                    if has("txt") { "yes" } else { "no" },
                    if has("lua") { "yes" } else { "no" }
                );
                println!(
                    "  format     {format} things files, texture set {}",
                    d.doc.texture_id
                );
            }
            Ok(0)
        }
        x => Err(format!("unknown command {x:?}")),
    }
}
