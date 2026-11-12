//! Runs the parser and validator over every shipped level script of the install.
use kfx_formats::campaign::CampaignCommon;
use kfx_formats::cfg::{latin1_decode, CfgDoc};
use kfx_formats::level_bin::{Apt, MapDims};
use kfx_formats::{rnc, testutil};
use kfx_script::{validate, ConstantProvider, Document, InstallProvider, Severity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            let n = p
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            if n.starts_with("map")
                && n.ends_with(".txt")
                && n[3..n.len() - 4].bytes().all(|b| b.is_ascii_digit())
            {
                out.push(p);
            }
        }
    }
}

fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

/// Provider for one map pack: base fxdata + the pack's CONFIGS_LOCATION (+ APs of the map).
fn provider_for(
    install: &Path,
    script: &Path,
    cache: &mut BTreeMap<PathBuf, InstallProvider>,
    layered: bool,
) -> InstallProvider {
    let pack_dir = script.parent().unwrap().to_path_buf();
    let base = cache.entry(pack_dir.clone()).or_insert_with(|| {
        let mut p = InstallProvider::new(install);
        if layered {
            let parent = pack_dir.parent().unwrap();
            let stem = pack_dir.file_name().unwrap().to_string_lossy().to_string();
            if let Some(cfg) = find_ci(parent, &format!("{stem}.cfg")) {
                if let Ok(b) = rnc::load_file(&cfg) {
                    let c = CampaignCommon::read(&CfgDoc::parse(&latin1_decode(&b)));
                    if let Some(loc) = c.configs_location {
                        if let Some(d) = testutil::resolve_ci(install, &loc) {
                            p.add_layer(&d);
                        }
                    }
                    if let Some(loc) = c.creatures_location {
                        if let Some(d) = testutil::resolve_ci(install, &loc) {
                            p.add_layer(&d);
                        }
                    }
                }
            }
        } else {
            // union of every layer directory of the install
            for d in ["levels", "campgns", "multiplayer"] {
                if let Ok(rd) = std::fs::read_dir(install.join(d)) {
                    for e in rd.flatten() {
                        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                        if e.path().is_dir() && (n.ends_with("_cfgs") || n.ends_with("_crtr")) {
                            p.add_layer(&e.path());
                        }
                    }
                }
            }
        }
        p
    });
    let mut p = base.clone();
    if let Some(apt) = find_ci(
        script.parent().unwrap(),
        &script
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase()
            .replace(".txt", ".apt"),
    ) {
        if let Ok(b) = rnc::load_file(&apt) {
            if let Ok(a) = Apt::parse(&b, MapDims::new(85, 85)) {
                p.set_map(a.points.iter().map(|x| x.number as i32).collect(), None);
            }
        }
    }
    p
}

#[test]
fn corpus() {
    let Some(install) = testutil::install_dir() else {
        eprintln!("no install, skipping");
        return;
    };
    let layered = std::env::var("KFX_SCRIPT_UNION").is_err();
    let mut files = vec![];
    for d in ["campgns", "levels", "multiplayer"] {
        walk(&install.join(d), &mut files);
    }
    files.sort();
    let mut cache = BTreeMap::new();
    let mut by_code: BTreeMap<(Severity, &'static str), Vec<String>> = BTreeMap::new();
    let (mut lines, mut cmds, mut bytes) = (0usize, 0usize, 0usize);
    let mut stats_max = kfx_script::Stats::default();
    let mut checked_sanity = 0;
    for f in &files {
        let data = rnc::load_file(f).expect("load");
        bytes += data.len();
        let doc = Document::parse(&data);
        assert_eq!(doc.reassemble(), data, "lossless {}", f.display());
        lines += doc.line_count();
        let prov = provider_for(&install, f, &mut cache, layered);
        let rep = validate(&doc, &prov);
        cmds += rep.stats.commands;
        stats_max.values = stats_max.values.max(rep.stats.values);
        stats_max.conditions = stats_max.conditions.max(rep.stats.conditions);
        stats_max.party_triggers = stats_max.party_triggers.max(rep.stats.party_triggers);
        stats_max.tunneller_triggers = stats_max
            .tunneller_triggers
            .max(rep.stats.tunneller_triggers);
        stats_max.parties = stats_max.parties.max(rep.stats.parties);
        stats_max.win_conditions = stats_max.win_conditions.max(rep.stats.win_conditions);
        stats_max.lose_conditions = stats_max.lose_conditions.max(rep.stats.lose_conditions);
        stats_max.commands = stats_max.commands.max(rep.stats.commands);
        let rel = f.strip_prefix(&install).unwrap().display().to_string();
        if checked_sanity == 0 {
            use kfx_script::ConstKind::*;
            for k in [
                Creature, Room, Slab, Trap, Door, Power, Spell, Object, Effect, EffectGen,
            ] {
                println!(
                    "sanity: {} {:?} names loaded for {rel}",
                    prov.names(k).len(),
                    k
                );
                assert!(!prov.names(k).is_empty(), "{k:?}");
            }
            let mut bogus = data.clone();
            bogus.extend_from_slice(
                b"\nLEVEL_VERSION(1)\n\
                ADD_CREATURE_TO_LEVEL(PLAYER0,NOSUCHCRTR,1,1,1,0)\nZOOM_TO_LOCATION(PLAYER0,977)\n",
            );
            let r = validate(&Document::parse(&bogus), &prov);
            assert!(r.diagnostics.iter().any(|d| d.code == "unknown-creature"));
            assert!(r
                .diagnostics
                .iter()
                .any(|d| d.code == "missing-action-point"));
            checked_sanity = 1;
        }
        for d in &rep.diagnostics {
            by_code
                .entry((d.severity, d.code))
                .or_default()
                .push(format!("{rel}:{}: {}", d.line, d.message));
        }
        // services must not panic on real data (sampled offsets)
        for off in (0..=doc.src.len()).step_by(37) {
            let _ = kfx_script::completions(&doc, off, &prov);
            let _ = kfx_script::signature_help(&doc, off);
            let _ = kfx_script::hover(&doc, off);
        }
        let _ = kfx_script::highlight(&doc);
        let _ = kfx_script::outline(&doc);
        let _ = kfx_script::references(&doc);
        let fm = kfx_script::format(&doc, &kfx_script::FormatOptions::default());
        let doc2 = Document::parse(&fm);
        let strip = |d: &Document| -> Vec<u8> {
            d.tokens
                .iter()
                .filter(|t| t.kind != kfx_script::TokKind::Whitespace)
                .flat_map(|t| d.src[t.span.start..t.span.end].to_vec())
                .collect()
        };
        assert_eq!(
            strip(&doc),
            strip(&doc2),
            "format changed tokens in {}",
            f.display()
        );
        assert_eq!(
            kfx_script::format(&doc2, &kfx_script::FormatOptions::default()),
            fm,
            "format not idempotent {}",
            f.display()
        );
    }
    println!("layered configs: {layered}");
    println!(
        "files {} lines {} commands {} bytes {}",
        files.len(),
        lines,
        cmds,
        bytes
    );
    println!("max resource use: {stats_max:?}");
    let verbose = std::env::var("KFX_SCRIPT_VERBOSE").is_ok();
    for ((sev, code), v) in &by_code {
        println!("{sev:?} {code}: {}", v.len());
        let n = if verbose {
            100
        } else if *sev == Severity::Error {
            12
        } else {
            3
        };
        for s in v.iter().take(n) {
            println!("    {s}");
        }
    }
    let _ = &InstallProvider::new(&install).action_points();
}
