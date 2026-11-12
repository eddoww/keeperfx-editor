//! Runs the validator over every shipped map and prints the issue counts per code (skipped
//! without the test install).
use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::{rnc, testutil};
use kfx_map::gamedata::{pack_folders, ConfigLayers, GameData};
use kfx_map::mapdoc::MapDoc;
use kfx_map::validate::{validate, Severity};
use std::collections::BTreeMap;

#[test]
fn validator_counts_on_shipped_maps() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let mut per_code: BTreeMap<(&'static str, Severity), (usize, usize)> = BTreeMap::new();
    let mut examples: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let (mut maps, mut failed) = (0, 0);
    let mut seen = std::collections::HashSet::new();
    for (levels, cfgs, crtr) in pack_folders(&root) {
        let mut nums: Vec<u32> = std::fs::read_dir(&levels)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                n.strip_prefix("map")?.strip_suffix(".slb")?.parse().ok()
            })
            .collect();
        nums.sort();
        nums.dedup();
        for n in nums {
            if !seen.insert((levels.clone(), n)) {
                continue;
            }
            let gd = GameData::load(&ConfigLayers::for_level(
                &root,
                cfgs.as_deref(),
                Some(&levels),
                crtr.as_deref(),
                Some(n),
            ))
            .unwrap();
            let Some(slb) =
                level_bin::find_file(&levels, n, "slb").and_then(|p| rnc::load_file(&p).ok())
            else {
                continue;
            };
            let Some(dims) = MapDims::from_slb_len(slb.len()) else {
                continue;
            };
            let Ok(doc) = MapDoc::load(&levels, n, dims, &gd) else {
                failed += 1;
                continue;
            };
            maps += 1;
            let mut here: BTreeMap<(&'static str, Severity), usize> = BTreeMap::new();
            for i in validate(&doc, &gd) {
                *here.entry((i.code, i.severity)).or_default() += 1;
                let ex = examples.entry(i.code).or_default();
                if ex.len() < 3 {
                    ex.push(format!(
                        "{}/map{n:05}: {:?} {}",
                        levels.display(),
                        i.at,
                        i.message
                    ));
                }
            }
            for (k, c) in here {
                let e = per_code.entry(k).or_default();
                e.0 += c;
                e.1 += 1;
            }
        }
    }
    println!("validated {maps} shipped maps ({failed} did not load)");
    println!(
        "{:<28} {:<8} {:>8} {:>6}",
        "code", "severity", "issues", "maps"
    );
    for ((code, sev), (c, m)) in &per_code {
        println!("{code:<28} {sev:<8?} {c:>8} {m:>6}");
    }
    for (code, ex) in &examples {
        if ["door-cannot-stand", "thing-unreachable"].contains(code) {
            for e in ex {
                println!("  e.g. {code}: {e}");
            }
        }
    }
    assert!(maps >= 200, "expected the ~274 shipped maps, got {maps}");
}
