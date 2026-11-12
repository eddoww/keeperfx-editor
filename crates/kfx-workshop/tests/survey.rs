//! Classification and install / uninstall over every archive of a Workshop survey: a folder of
//! downloaded Workshop data in the seed layout of [`kfx_workshop::client`]
//! (`pages/item-<id>.html`, `api/item-<id>.json`, `files/<item>/<file>/<name>`) named by the
//! environment variable `KFX_WORKSHOP_SURVEY`. Skipped when it is unset.

use kfx_workshop::archive::Limits;
use kfx_workshop::classify::{classify_archive, Kind};
use kfx_workshop::install::PlanOptions;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn survey() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("KFX_WORKSHOP_SURVEY")?);
    p.join("files").is_dir().then_some(p)
}

fn all_archives(root: &std::path::Path) -> Vec<PathBuf> {
    let mut v = vec![];
    for item in std::fs::read_dir(root.join("files"))
        .into_iter()
        .flatten()
        .flatten()
    {
        for file in std::fs::read_dir(item.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            for f in std::fs::read_dir(file.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let p = f.path();
                let e = p
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if matches!(e.as_str(), "zip" | "7z" | "rar") {
                    v.push(p);
                }
            }
        }
    }
    v.sort();
    v
}

#[test]
fn classify_every_survey_archive() {
    let Some(root) = survey() else {
        eprintln!("survey cache absent; skipped");
        return;
    };
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_format: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut failed: Vec<(PathBuf, String)> = vec![];
    let mut other: Vec<(PathBuf, Vec<String>)> = vec![];
    let mut multiplayer = 0;
    let mut total = 0;
    for p in all_archives(&root) {
        total += 1;
        let ext = p
            .extension()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        match classify_archive(&p, &Limits::default()) {
            Ok(i) => {
                let label = i.class.kind.label().to_string();
                *kinds.entry(label.clone()).or_default() += 1;
                *by_format.entry(ext).or_default().entry(label).or_default() += 1;
                if i.class.levels.iter().any(|g| g.multiplayer) {
                    multiplayer += 1;
                }
                if i.class.kind == Kind::Other {
                    other.push((p.clone(), i.class.notes.clone()));
                }
                if !i.class.unsafe_entries.is_empty() {
                    failed.push((
                        p.clone(),
                        format!("UNSAFE ENTRIES: {:?}", i.class.unsafe_entries),
                    ));
                }
            }
            Err(e) => failed.push((p.clone(), e.to_string())),
        }
    }
    let rel = |p: &PathBuf| {
        p.strip_prefix(root.join("files"))
            .unwrap()
            .display()
            .to_string()
    };
    println!("archives: {total}");
    println!("kinds: {kinds:#?}");
    println!("by format: {by_format:#?}");
    println!("with multiplayer levels: {multiplayer}");
    println!("--- Other ({}):", other.len());
    for (p, n) in &other {
        println!("  {}  {}", rel(p), n.join("; "));
    }
    println!("--- failed ({}):", failed.len());
    for (p, e) in &failed {
        println!("  {}  {e}", rel(p));
    }
    assert!(total > 0);
}

/// Install every survey archive up to a size (`KFX_SURVEY_MAX_BYTES`, default 6 MiB) into a
/// private install, check the plan's invariants, and uninstall again: the install folder must
/// be byte-identical afterwards.
#[test]
fn install_and_uninstall_survey_archives() {
    mod_common::run();
}

#[path = "common/mod.rs"]
mod common;

mod mod_common {
    use super::*;
    use crate::common::*;
    use kfx_workshop::install::*;

    pub fn run() {
        let Some(root) = survey() else { return };
        let max_bytes: u64 = std::env::var("KFX_SURVEY_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6 << 20);
        let s = Scratch::new("survey-install");
        let install = s.join("install");
        private_install(&install);
        let store = Store::new(s.join("store"));
        let base = snapshot(&install);
        let (mut ok, mut blocked, mut failed) = (0, vec![], vec![]);
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        let mut renumbered = 0;
        let mut tried = 0;
        for p in all_archives(&root) {
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            if size > max_bytes {
                continue;
            }
            tried += 1;
            let rel = p
                .strip_prefix(root.join("files"))
                .unwrap()
                .display()
                .to_string();
            let Ok(insp) = classify_archive(&p, &Limits::default()) else {
                continue;
            };
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            let plan = match plan(&install, &p, &insp, &PlanOptions::new(&name)) {
                Ok(pl) => pl,
                Err(e) => {
                    failed.push((rel, format!("plan: {e}")));
                    continue;
                }
            };
            // invariants of any plan
            for i in &plan.items {
                assert!(
                    kfx_workshop::archive::safe_rel_path(&i.dest).is_ok(),
                    "{rel}: unsafe dest {}",
                    i.dest
                );
                assert!(
                    i.action != Action::Overwrite,
                    "{rel}: default plan must not overwrite {}",
                    i.dest
                );
            }
            if plan.is_blocked() {
                blocked.push((rel, plan.blocked.join("; ")));
                continue;
            }
            renumbered += plan.renumbered.len();
            match apply(&install, &plan, &store, &Limits::default()) {
                Ok(m) => {
                    ok += 1;
                    *kinds.entry(plan.kind.label().to_string()).or_default() += 1;
                    uninstall(&install, &m.id, &store, false).unwrap();
                    let now = snapshot(&install);
                    if now != base {
                        let diff: Vec<&String> = now
                            .keys()
                            .filter(|k| !base.contains_key(*k))
                            .chain(base.keys().filter(|k| !now.contains_key(*k)))
                            .take(5)
                            .collect();
                        panic!("{rel}: install folder differs after uninstall: {diff:?}");
                    }
                }
                Err(e) => failed.push((rel, format!("apply: {e}"))),
            }
        }
        println!(
            "survey install round trip: tried {tried} archives up to \
                {max_bytes} bytes: {ok} installed+uninstalled byte-identically, {} blocked, {} \
                failed, {renumbered} renumbered levels",
            blocked.len(),
            failed.len()
        );
        println!("installed kinds: {kinds:?}");
        let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
        for (_, r) in &blocked {
            *reasons.entry(r.chars().take(70).collect()).or_default() += 1;
        }
        println!("blocked reasons: {reasons:#?}");
        for (p, e) in failed.iter().take(20) {
            println!("  FAILED {p}: {e}");
        }
        assert!(failed.is_empty(), "{failed:?}");
        assert!(ok > 50);
    }
}
