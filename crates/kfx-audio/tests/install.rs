//! Resolution against the real test install.
use kfx_audio::{Audio, Category, Context, Source};
use kfx_formats::testutil::{install_dir, resolve_ci};

fn audio(level: Option<u32>) -> Option<Audio> {
    let install = install_dir()?;
    let cfg = resolve_ci(&install, "campgns/keeporig.cfg")?;
    let mut ctx = Context::new(&install, "eng").with_campaign(&cfg).unwrap();
    if let Some(l) = level {
        ctx = ctx.with_level(l);
    }
    Some(Audio::open(ctx))
}

fn bank_index(r: &kfx_audio::Resolved, i: usize) -> u16 {
    match &r.variants[i].source {
        Source::Bank { index, .. } => *index,
        s => panic!("{s:?}"),
    }
}

#[test]
fn names_of_fxdata_sounds_cfg() {
    let Some(a) = audio(Some(1)) else { return };
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);
    // GOLD_PICKUP = 32 3 in fxdata/sounds.cfg
    let g = a.resolve("GOLD_PICKUP").unwrap();
    assert_eq!(
        (g.variants.len(), bank_index(&g, 0), bank_index(&g, 2)),
        (3, 32, 34)
    );
    assert_eq!(g.layer.kind, kfx_audio::LayerKind::Install);
    assert_eq!(bank_index(&a.resolve("tab_click").unwrap(), 0), 60);
    // every name of the file resolves and the first variant decodes
    let install = install_dir().unwrap();
    let doc = kfx_formats::cfg::CfgDoc::load(&resolve_ci(&install, "fxdata/sounds.cfg").unwrap())
        .unwrap();
    let mut n = 0;
    let mut bad = vec![];
    for e in doc.entries("sounds") {
        n += 1;
        match a.resolve(&e.key) {
            Ok(r) => {
                if let Err(err) = a.decode_resolved(&r) {
                    bad.push(format!("{}: {err}", e.key));
                }
            }
            Err(err) => bad.push(format!("{}: {err}", e.key)),
        }
    }
    eprintln!("{n} names in fxdata/sounds.cfg, {} problems", bad.len());
    assert!(n > 50 && bad.is_empty(), "{bad:?}");
}

#[test]
fn creature_sets_of_the_install() {
    let Some(a) = audio(Some(1)) else { return };
    let imp = a.creature_set("imp");
    let foot = &imp
        .slots
        .iter()
        .find(|(k, _)| *k == kfx_audio::CreatureSound::Foot)
        .unwrap()
        .1;
    // creatrs/imp.cfg: Foot = 9 4
    assert_eq!(
        (
            foot.variants.len(),
            bank_index(foot, 0),
            bank_index(foot, 3)
        ),
        (4, 9, 12)
    );
    let names = a.creature_names();
    let (mut slots, mut bad) = (0, vec![]);
    for n in &names {
        for (slot, r) in a.creature_set(n).slots {
            slots += 1;
            for v in &r.variants {
                if let Err(e) = a.decode(&v.source) {
                    bad.push(format!("{n}.{}: {e}", slot.key()));
                }
            }
        }
    }
    eprintln!("{} creatures, {slots} sound slots", names.len());
    assert!(
        names.len() >= 30 && slots > 200 && bad.is_empty(),
        "{bad:?}"
    );
}

#[test]
fn speech_and_level_speech_of_the_install() {
    let Some(a) = audio(Some(1)) else { return };
    let r = a.resolve_speech("GoldLow").unwrap();
    assert_eq!(bank_index(&r, 0), 86);
    eprintln!("GoldLow -> speech bank sample {:?}", r.variants[0].label);
    let d = a.decode_resolved(&r).unwrap();
    assert!(d.stats().rms > 0.01);
    // campaign keeporig, level 1: SPEECH = good01.mp3 bad01.mp3 in campgns/keeporig_eng
    let b = a.resolve_level_speech(1, true).unwrap();
    let pcm = a.decode_resolved(&b).unwrap();
    eprintln!(
        "level 1 good speech: {} {:.1}s",
        b.variants[0].label,
        pcm.duration_secs()
    );
    assert_eq!(b.variants[0].label, "good01.mp3");
    assert!(pcm.duration_secs() > 3.0);
    assert!(a
        .decode_resolved(&a.resolve_level_speech(1, false).unwrap())
        .is_ok());
    // names of the speech enum that exist in the bank all have a decodable sample
    for (name, idx) in kfx_audio::speech_names::SPEECH_MESSAGES {
        let r = a.resolve_speech(name).unwrap();
        assert_eq!(bank_index(&r, 0), *idx, "{name}");
        assert!(a.decode_resolved(&r).is_ok(), "{name}");
    }
}

#[test]
fn catalog_of_the_install() {
    let Some(a) = audio(Some(1)) else { return };
    let cat = a.catalog();
    let count = |c| cat.iter().filter(|e| e.category == c).count();
    eprintln!(
        "catalog: effects {} speech {} creatures {} music {}",
        count(Category::Effects),
        count(Category::Speech),
        count(Category::Creatures),
        count(Category::Music)
    );
    assert_eq!(count(Category::Effects), 1067);
    assert!(count(Category::Speech) >= 125 + 2 * 20);
    assert!(count(Category::Creatures) > 200);
    // the install ships no music files (music/MusicReadme.txt only)
    assert_eq!(count(Category::Music), 0);
    let keys: std::collections::HashSet<_> = cat.iter().map(|e| &e.key).collect();
    assert_eq!(keys.len(), cat.len());
    // effects carry the aliases of sounds.cfg
    let e = cat.iter().find(|e| e.key == "fx:60").unwrap();
    assert!(e.detail.contains("TAB_CLICK"), "{e:?}");
}
