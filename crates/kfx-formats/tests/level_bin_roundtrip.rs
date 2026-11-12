//! Round-trips every legacy level file of the test install (skipped when absent).
use kfx_formats::level_bin::*;
use kfx_formats::{rnc, testutil};
use std::collections::BTreeMap;
use std::path::Path;

fn map_numbers(dir: &Path) -> Vec<u32> {
    let mut v: Vec<u32> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            let rest = n.strip_prefix("map")?;
            let (num, ext) = rest.split_once('.')?;
            (num.len() == 5 && EXTENSIONS.contains(&ext)).then(|| num.parse().ok())?
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

fn folders(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for top in ["campgns", "levels", "multiplayer"] {
        let Some(t) = testutil::resolve_ci(root, top) else {
            continue;
        };
        out.push(t.clone());
        let mut stack = vec![t];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    out.push(e.path());
                    stack.push(e.path());
                }
            }
        }
    }
    out.sort();
    out
}

fn roundtrip(ext: &str, data: &[u8], d: MapDims) -> Result<Vec<u8>, kfx_formats::Error> {
    Ok(match ext {
        "slb" => Slb::parse(data, d)?.to_bytes(),
        "own" => Own::parse(data, d)?.to_bytes(),
        "dat" => Dat::parse(data, d)?.to_bytes(),
        "clm" => Clm::parse(data, d)?.to_bytes(),
        "wib" => Wib::parse(data, d)?.to_bytes(),
        "wlb" => Wlb::parse(data, d)?.to_bytes(),
        "flg" => Flg::parse(data, d)?.to_bytes(),
        "inf" => Inf::parse(data, d)?.to_bytes(),
        "slx" => Slx::parse(data, d)?.to_bytes(),
        "tng" => Tng::parse(data, d)?.to_bytes(),
        "apt" => Apt::parse(data, d)?.to_bytes(),
        "lgt" => Lgt::parse(data, d)?.to_bytes(),
        _ => unreachable!(),
    })
}

fn expected(ext: &str, d: MapDims) -> usize {
    match ext {
        "slb" => 2 * d.slabs(),
        "own" | "wib" => d.stl_grid(),
        "dat" | "flg" => 2 * d.stl_grid(),
        "wlb" | "slx" => d.slabs(),
        _ => 0, // variable size
    }
}

#[test]
fn install_roundtrip() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let mut ok: BTreeMap<String, usize> = BTreeMap::new();
    let mut anomalies: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut maps = 0;
    let mut packed = 0;
    for dir in folders(&root) {
        for n in map_numbers(&dir) {
            maps += 1;
            let tag = format!("{}/map{n:05}", dir.strip_prefix(&root).unwrap().display());
            let slb = find_file(&dir, n, "slb").and_then(|p| rnc::load_file(&p).ok());
            let dims = match slb
                .as_deref()
                .map(|s| (s.len(), MapDims::from_slb_len(s.len())))
            {
                Some((_, Some(d))) => d,
                Some((len, None)) => {
                    anomalies.push(format!("{tag}: slb size {len} is not 2*N*N"));
                    continue;
                }
                None => {
                    anomalies.push(format!("{tag}: no slb (other files present)"));
                    continue;
                }
            };
            if dims.w != 85 {
                anomalies.push(format!("{tag}: map is {}x{}", dims.w, dims.h));
            }
            for ext in EXTENSIONS {
                let Some(p) = find_file(&dir, n, ext) else {
                    continue;
                };
                if std::fs::read(&p).map(|d| rnc::is_rnc(&d)).unwrap_or(false) {
                    packed += 1;
                }
                let data = match rnc::load_file(&p) {
                    Ok(d) => d,
                    Err(e) => {
                        failures.push(format!("{tag}.{ext}: load: {e}"));
                        continue;
                    }
                };
                let exp = expected(ext, dims);
                if exp != 0 && data.len() != exp {
                    anomalies.push(format!(
                        "{tag}.{ext}: size {} but expected {exp}",
                        data.len()
                    ));
                }
                match roundtrip(ext, &data, dims) {
                    Ok(out) if out == data => *ok.entry(ext.to_string()).or_default() += 1,
                    Ok(_) => failures.push(format!("{tag}.{ext}: re-serialised bytes differ")),
                    Err(e) => failures.push(format!("{tag}.{ext}: parse: {e}")),
                }
                // Variable-size files: report structural oddities.
                if ext == "tng" || ext == "apt" || ext == "lgt" || ext == "clm" {
                    let (extra, declared) = match ext {
                        "tng" => {
                            let t = Tng::parse(&data, dims).unwrap();
                            (t.trailing.len(), t.declared_count)
                        }
                        "apt" => {
                            let t = Apt::parse(&data, dims).unwrap();
                            (t.trailing.len(), t.declared_count)
                        }
                        "lgt" => {
                            let t = Lgt::parse(&data, dims).unwrap();
                            (t.trailing.len(), t.declared_count)
                        }
                        _ => {
                            let t = Clm::parse(&data, dims).unwrap();
                            (t.trailing.len(), t.declared_count)
                        }
                    };
                    if extra != 0 || declared.is_some() {
                        anomalies.push(format!(
                            "{tag}.{ext}: trailing {extra} bytes, bad declared count {declared:?}"
                        ));
                    }
                }
                if matches!(ext, "slb" | "own" | "dat" | "wib" | "wlb" | "flg" | "slx")
                    && data.len() > exp
                    && exp != 0
                {
                    // already reported as size anomaly
                }
            }
            // The whole-set loader must agree.
            match LevelBin::load(&dir, n) {
                Ok(b) => {
                    for (ext, bytes) in b.files() {
                        let p = find_file(&dir, n, ext).unwrap();
                        if rnc::load_file(&p).unwrap() != bytes {
                            failures.push(format!("{tag}.{ext}: LevelBin differs"));
                        }
                    }
                }
                Err(e) => failures.push(format!("{tag}: LevelBin::load: {e}")),
            }
        }
    }
    println!("maps checked: {maps}, files that were RNC packed: {packed}");
    for (e, c) in &ok {
        println!("  .{e}: {c} files round-trip exactly");
    }
    println!("anomalies ({}):", anomalies.len());
    for a in &anomalies {
        println!("  {a}");
    }
    println!("failures ({}):", failures.len());
    for f in &failures {
        println!("  {f}");
    }
    assert!(failures.is_empty());
}
