//! Every shipped map: load -> save -> load -> save, in both column modes. The second save must
//! be byte-identical to the first, the reloaded document must equal the original in everything
//! the game reads, and no stale legacy file may be left. Needs the test install (skipped
//! otherwise); split by top folder so the parts run in parallel.
use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::levelinfo::Lof;
use kfx_formats::{rnc, testutil};
use kfx_map::column::column_equivalent;
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn map_files(dir: &Path, n: u32) -> Vec<PathBuf> {
    let pre = format!("map{n:05}.");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with(&pre)
                && e.path().is_file()
        })
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

fn snapshot(dir: &Path, n: u32) -> BTreeMap<String, Vec<u8>> {
    map_files(dir, n)
        .into_iter()
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().to_string(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect()
}

fn copy_map(src: &Path, n: u32, dst: &Path) {
    let _ = std::fs::remove_dir_all(dst);
    std::fs::create_dir_all(dst).unwrap();
    for p in map_files(src, n) {
        std::fs::copy(&p, dst.join(p.file_name().unwrap())).unwrap();
    }
}

/// Legacy residue keys are bytes the game never reads (`kfx_map::legacy`); the FX files drop them.
fn strip(doc: &mut MapDoc) {
    for t in doc.things.items.iter_mut() {
        t.extra.retain(|k, _| !kfx_map::legacy::is_legacy_key(k));
    }
    for l in doc.lights.items.iter_mut() {
        l.extra.retain(|k, _| !kfx_map::legacy::is_legacy_key(k));
    }
}

/// Bits of a column's `bitfields` the game recomputes on load (`set_column_floor_filled_subtiles`,
/// src/lvl_filesdk1.c:694, the floor height in bits 4-7).
const RECOMPUTED_BITS: u8 = 0xF0;

fn compare(tag: &str, a: &MapDoc, b: &MapDoc, columns: bool, out: &mut Vec<String>) {
    let mut a = a.clone();
    let mut b = b.clone();
    strip(&mut a);
    strip(&mut b);
    macro_rules! eq {
        ($f:ident) => {
            if a.$f != b.$f {
                out.push(format!("{tag}: {} differs", stringify!($f)));
            }
        };
    }
    eq!(kinds);
    eq!(owners);
    eq!(wlb);
    eq!(slx);
    eq!(inf_trailing);
    eq!(texture_id);
    eq!(flg);
    if a.things.items != b.things.items {
        let i = (0..a.things.items.len().max(b.things.items.len()))
            .find(|&i| a.things.items.get(i) != b.things.items.get(i))
            .unwrap();
        out.push(format!(
            "{tag}: thing {i}: {:?} vs {:?}",
            a.things.items.get(i),
            b.things.items.get(i)
        ));
    }
    if a.action_points.items != b.action_points.items {
        out.push(format!("{tag}: action points differ"));
    }
    if a.lights.items != b.lights.items {
        out.push(format!("{tag}: lights differ"));
    }
    if columns {
        let ne = a
            .columns
            .iter()
            .zip(b.columns.iter())
            .filter(|(x, y)| !column_equivalent(x, y))
            .count();
        let bits = a
            .columns
            .iter()
            .zip(b.columns.iter())
            .filter(|(x, y)| (x.bitfields & !RECOMPUTED_BITS) != (y.bitfields & !RECOMPUTED_BITS))
            .count();
        if ne > 0 {
            out.push(format!("{tag}: {ne} subtile columns not equivalent"));
        }
        if bits > 0 {
            out.push(format!(
                "{tag}: {bits} subtile columns differ in the bitfield bits the game keeps"
            ));
        }
        eq!(wibble);
    }
}

fn dims_of(dir: &Path, n: u32) -> Option<MapDims> {
    if let Some(l) = level_bin::find_file(dir, n, "lof").and_then(|p| Lof::load(&p).ok()) {
        let (w, h) = l.info().mapsize_or_default();
        let d = MapDims::new(w as u16, h as u16);
        let slb = rnc::load_file(&level_bin::find_file(dir, n, "slb")?).ok()?;
        if slb.len() == d.slabs() * 2 {
            return Some(d);
        }
    }
    MapDims::from_slb_len(
        rnc::load_file(&level_bin::find_file(dir, n, "slb")?)
            .ok()?
            .len(),
    )
}

fn run(part: &str, filter: impl Fn(&str) -> bool) {
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let work = kfx_formats::testutil::scratch_dir(&format!(
        "kfx-map/kfx-idempotence-{part}-{}",
        std::process::id()
    ));
    let mut maps = vec![];
    for top in ["campgns", "levels", "multiplayer"] {
        let Some(root) = testutil::resolve_ci(&install, top) else {
            continue;
        };
        for d in std::fs::read_dir(root).unwrap().flatten() {
            if !d.path().is_dir() {
                continue;
            }
            let rel = format!("{top}/{}", d.file_name().to_string_lossy());
            if !filter(&rel) {
                continue;
            }
            for e in std::fs::read_dir(d.path()).unwrap().flatten() {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if let Some(n) = name
                    .strip_prefix("map")
                    .and_then(|r| r.strip_suffix(".slb"))
                    .and_then(|r| r.parse::<u32>().ok())
                {
                    maps.push((d.path(), rel.clone(), n));
                }
            }
        }
    }
    maps.sort();
    let mut issues = vec![];
    let mut checked = 0;
    for (dir, rel, n) in maps {
        let tag = format!("{rel}/map{n:05}");
        let Some(dims) = dims_of(&dir, n) else {
            continue;
        };
        let gd = GameData::load(&ConfigLayers::for_level(
            &install,
            None,
            Some(&dir),
            None,
            Some(n),
        ))
        .unwrap();
        let Ok(d0) = MapDoc::load(&dir, n, dims, &gd) else {
            continue;
        };
        if !d0.unreadable.is_empty() {
            issues.push(format!(
                "{tag}: shipped map has unreadable parts {:?}",
                d0.unreadable.iter().map(|u| &u.error).collect::<Vec<_>>()
            ));
            continue;
        }
        checked += 1;
        for (mode, mname) in [
            (SaveColumns::Current, "cur"),
            (SaveColumns::Regenerate, "regen"),
        ] {
            let w = work.join(mname);
            copy_map(&dir, n, &w);
            if let Err(e) = d0.save(&gd, &w, mode) {
                issues.push(format!("{tag} {mname}: first save failed: {e}"));
                continue;
            }
            let s1 = snapshot(&w, n);
            let d1 = MapDoc::load(&w, n, dims, &gd).unwrap();
            if !d1.unreadable.is_empty() {
                issues.push(format!("{tag} {mname}: the saved map has unreadable parts"));
            }
            d1.save(&gd, &w, mode).unwrap();
            let s2 = snapshot(&w, n);
            if s1 != s2 {
                let diff: Vec<_> = s1
                    .keys()
                    .chain(s2.keys())
                    .filter(|k| s1.get(*k) != s2.get(*k))
                    .cloned()
                    .collect();
                issues.push(format!(
                    "{tag} {mname}: second save not byte-identical: {diff:?}"
                ));
            }
            compare(
                &format!("{tag} {mname} original vs reload"),
                &d0,
                &d1,
                mode == SaveColumns::Current,
                &mut issues,
            );
            for ext in ["tng", "apt", "lgt"] {
                if s2
                    .keys()
                    .any(|x| x.to_ascii_lowercase().ends_with(&format!(".{ext}")))
                {
                    issues.push(format!("{tag} {mname}: stale .{ext} left"));
                }
            }
            for (name, bytes) in &s2 {
                let lower = name.to_ascii_lowercase();
                if lower.ends_with("fx") {
                    if let Err(e) = kfx_formats::game_toml::check(&String::from_utf8_lossy(bytes)) {
                        issues.push(format!("{tag} {mname}: the game rejects {name}: {e}"));
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    println!("[{part}] {checked} maps checked, {} issues", issues.len());
    assert!(
        issues.is_empty(),
        "{} issues:\n{}",
        issues.len(),
        issues.join("\n")
    );
}

#[test]
fn campaigns_a_to_k() {
    run("campgns-a-k", |r| {
        r.starts_with("campgns/")
            && r.as_bytes()
                .get(8)
                .is_some_and(|c| c.to_ascii_lowercase() <= b'k')
    });
}

#[test]
fn campaigns_l_to_z() {
    run("campgns-l-z", |r| {
        r.starts_with("campgns/")
            && r.as_bytes()
                .get(8)
                .is_some_and(|c| c.to_ascii_lowercase() > b'k')
    });
}

#[test]
fn levels_and_multiplayer() {
    run("levels-mp", |r| {
        r.starts_with("levels/") || r.starts_with("multiplayer/")
    });
}
