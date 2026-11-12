//! Legacy output (`save_legacy`): errors / warnings, and a round trip over the shipped legacy maps.
use kfx_formats::level_bin::{self, LevelBin, MapDims};
use kfx_formats::level_fx::StlCoord;
use kfx_formats::{rnc, testutil};
use kfx_map::gamedata::{pack_folders, ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns, ThingFormat};
use kfx_map::things::{self, class};
use std::path::Path;

fn gd() -> Option<GameData> {
    let root = testutil::install_dir()?;
    Some(GameData::load(&ConfigLayers::for_level(&root, None, None, None, None)).unwrap())
}

fn sorted<T: Ord + Clone>(v: &[T]) -> Vec<T> {
    let mut v = v.to_vec();
    v.sort();
    v
}

#[test]
fn legacy_errors_and_warnings() {
    let Some(gd) = gd() else { return };
    // not 85x85: refused, nothing written
    let doc = MapDoc::new(Path::new("/tmp"), 1, MapDims::new(40, 40), 0, &gd);
    // a folder that does not exist yet: a refused save must not create it
    let dir = kfx_formats::testutil::scratch_dir("kfx-map")
        .join(format!("kfx_legacy_err_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let err = doc
        .save_legacy(&gd, &dir, SaveColumns::Current)
        .unwrap_err()
        .to_string();
    assert!(err.contains("40x40") && err.contains("85x85"), "{err}");
    assert!(!dir.exists());
    // 85x85 with things the legacy format cannot express
    let mut doc = MapDoc::new(Path::new("/tmp"), 1, MapDims::new(85, 85), 0, &gd);
    let mut t = things::new_thing(
        class::CREATURE,
        300,
        0,
        things::centred(30),
        things::centred(30),
        StlCoord::new(1, 0),
    );
    t.creature_name = Some("Bob".into());
    doc.things.items.push(t); // model 300
    let mut t = things::new_thing(
        class::CREATURE,
        2,
        0,
        things::centred(31),
        things::centred(30),
        StlCoord::new(1, 0),
    );
    t.creature_name = Some("Bob".into());
    t.creature_gold = Some(10);
    doc.things.items.push(t); // extended fields only
    let mut t = things::new_thing(
        class::OBJECT,
        3,
        5,
        things::centred(32),
        things::centred(30),
        StlCoord::new(1, 0),
    );
    t.orientation = Some(512);
    doc.things.items.push(t);
    let rep = doc.legacy_report(&gd);
    println!("errors: {:?}\nwarnings: {:?}", rep.errors, rep.warnings);
    assert!(rep.errors.iter().any(|e| e.contains("model above 255")));
    assert!(rep.warnings.iter().any(|w| w.contains("CreatureName")));
    assert!(rep.warnings.iter().any(|w| w.contains("Orientation")));
    assert!(doc.save_legacy(&gd, &dir, SaveColumns::Current).is_err());
    doc.things.items.remove(0);
    let rep = doc.save_legacy(&gd, &dir, SaveColumns::Current).unwrap();
    assert!(rep.errors.is_empty() && rep.warnings.len() == 2);
    // legacy files written, FX files absent, and the game's rule picks legacy
    for ext in ["tng", "apt", "lgt", "slb", "clm", "dat"] {
        assert!(level_bin::find_file(&dir, 1, ext).is_some(), "{ext}");
    }
    for ext in ["tngfx", "aptfx", "lgtfx"] {
        assert!(level_bin::find_file(&dir, 1, ext).is_none(), "{ext}");
    }
    let back = MapDoc::load(&dir, 1, doc.dims, &gd).unwrap();
    assert_eq!(back.loaded_format, ThingFormat::Legacy);
    assert_eq!(back.things.items.len(), 2);
    // saving FX again removes the legacy files
    back.save(&gd, &dir, SaveColumns::Current).unwrap();
    assert!(
        level_bin::find_file(&dir, 1, "tng").is_none()
            && level_bin::find_file(&dir, 1, "lgtfx").is_some()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Shipped legacy maps: load, `save_legacy`, compare the three record lists with the originals.
#[test]
fn shipped_legacy_maps_round_trip() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let (mut maps, mut refused, mut skipped_fx) = (0, 0, 0);
    let (mut things, mut things_exact, mut things_game, mut order_kept) = (0, 0, 0, 0);
    let (mut apts, mut apts_exact, mut lgts, mut lgts_exact, mut lgts_game) = (0, 0, 0, 0, 0);
    let mut warn_kinds: std::collections::BTreeMap<String, usize> = Default::default();
    let mut diff_kinds: std::collections::BTreeMap<String, usize> = Default::default();
    let mut seen = std::collections::HashSet::new();
    for (levels, cfgs, crtr) in pack_folders(&root) {
        let nums: Vec<u32> = {
            let mut v: Vec<u32> = std::fs::read_dir(&levels)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .strip_prefix("map")?
                        .strip_suffix(".slb")?
                        .parse()
                        .ok()
                })
                .collect();
            v.sort();
            v.dedup();
            v
        };
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
                continue;
            };
            if doc.loaded_format != ThingFormat::Legacy {
                skipped_fx += 1;
                continue;
            }
            let orig = LevelBin::load_with_dims(&levels, n, Some(dims)).unwrap();
            let dir = kfx_formats::testutil::scratch_dir(&format!(
                "kfx-map/kfx_legacy_rt_{}_{n}",
                std::process::id()
            ));
            let rep = match doc.save_legacy(&gd, &dir, SaveColumns::Current) {
                Ok(r) => r,
                Err(_) => {
                    refused += 1;
                    continue;
                }
            };
            for w in &rep.warnings {
                *warn_kinds
                    .entry(
                        w.split(':')
                            .next()
                            .unwrap_or("")
                            .split(" (")
                            .next()
                            .unwrap_or("")
                            .to_string(),
                    )
                    .or_default() += 1;
            }
            maps += 1;
            let back = LevelBin::load_with_dims(&dir, n, Some(dims)).unwrap();
            let (ot, bt) = (
                orig.tng.clone().unwrap_or(level_bin::Tng {
                    things: vec![],
                    declared_count: None,
                    trailing: vec![],
                }),
                back.tng.clone().unwrap(),
            );
            let key = |t: &level_bin::Thing| {
                (
                    t.x, t.y, t.z, t.class, t.model, t.owner, t.range, t.index, t.params,
                )
            };
            let (ok, bk): (Vec<_>, Vec<_>) = (
                ot.things.iter().map(key).collect(),
                bt.things.iter().map(key).collect(),
            );
            things += ok.len();
            assert_eq!(ok.len(), bk.len(), "map {n}: thing count");
            let (so, sb) = (sorted(&ok), sorted(&bk));
            if so == sb {
                things_exact += ok.len();
            } else {
                // records that differ: which fields (class, model, index, params)
                let mut pool = sb.clone();
                for r in &so {
                    match pool.iter().position(|p| p == r) {
                        Some(i) => {
                            pool.swap_remove(i);
                        }
                        None => {
                            things_game += 1;
                            let near = sb
                                .iter()
                                .find(|p| p.0 == r.0 && p.1 == r.1 && p.3 == r.3 && p.4 == r.4);
                            *diff_kinds
                                .entry(format!(
                                    "class {} differs in {}",
                                    r.3,
                                    match near {
                                        Some(p) => format!(
                                            "{}{}{}{}",
                                            if p.6 != r.6 { "range " } else { "" },
                                            if p.7 != r.7 { "index " } else { "" },
                                            if p.8 != r.8 { "params " } else { "" },
                                            if p.2 != r.2 || p.5 != r.5 {
                                                "z/owner"
                                            } else {
                                                ""
                                            }
                                        ),
                                        None => "position/model (no counterpart)".to_string(),
                                    }
                                ))
                                .or_default() += 1;
                        }
                    }
                }
            }
            if ok == bk {
                order_kept += 1;
            }
            let oa = orig.apt.clone().map(|a| a.points).unwrap_or_default();
            let ba = back.apt.clone().unwrap().points;
            apts += oa.len();
            if sorted(
                &oa.iter()
                    .map(|a| (a.x, a.y, a.range, a.number))
                    .collect::<Vec<_>>(),
            ) == sorted(
                &ba.iter()
                    .map(|a| (a.x, a.y, a.range, a.number))
                    .collect::<Vec<_>>(),
            ) {
                apts_exact += oa.len();
            }
            assert_eq!(oa.len(), ba.len());
            let ol = orig.lgt.clone().map(|l| l.lights).unwrap_or_default();
            let bl = back.lgt.clone().unwrap().lights;
            lgts += ol.len();
            let lk = |l: &level_bin::Light| {
                (
                    l.radius,
                    l.intensity,
                    l.flags,
                    l.unused,
                    l.x,
                    l.y,
                    l.z,
                    l.unused_10,
                    l.is_dynamic,
                    l.attached_slb,
                )
            };
            let lg = |l: &level_bin::Light| {
                (
                    l.radius,
                    l.intensity,
                    l.x,
                    l.y,
                    l.z,
                    l.is_dynamic,
                    l.attached_slb,
                )
            };
            assert_eq!(ol.len(), bl.len());
            if sorted(&ol.iter().map(lk).collect::<Vec<_>>())
                == sorted(&bl.iter().map(lk).collect::<Vec<_>>())
            {
                lgts_exact += ol.len();
            }
            if sorted(&ol.iter().map(lg).collect::<Vec<_>>())
                == sorted(&bl.iter().map(lg).collect::<Vec<_>>())
            {
                lgts_game += ol.len();
            }
            assert_eq!(
                sorted(&ol.iter().map(lg).collect::<Vec<_>>()),
                sorted(&bl.iter().map(lg).collect::<Vec<_>>()),
                "map {n}: lights"
            );
            // columns / slabs are those of the document
            assert_eq!(back.slb.unwrap().kinds, doc.kinds);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    println!(
        "[legacy] {maps} shipped legacy maps saved ({refused} refused, {skipped_fx} FX maps \
            skipped): things {things} (identical as sets: {things_exact}, records that differ: \
            {things_game} {diff_kinds:?}), maps with the original record order kept: {order_kept}; \
            action points {apts} ({apts_exact} identical); lights {lgts} ({lgts_exact} \
            byte-identical, {lgts_game} identical on game-read fields); warnings by kind: \
            {warn_kinds:?}"
    );
}
