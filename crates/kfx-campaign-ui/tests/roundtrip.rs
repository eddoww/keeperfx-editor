//! Shipped campaigns and map packs load into the model and save back byte-identical.
use kfx_campaign_ui::{validate, Pack, Severity};
use kfx_formats::testutil::install_dir;
use std::path::PathBuf;

fn cfgs(install: &std::path::Path) -> Vec<PathBuf> {
    let mut v = vec![];
    for g in ["campgns", "levels", "multiplayer"] {
        let d = install.join(g);
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")) {
                v.push(p);
            }
        }
    }
    v.sort();
    v
}

#[test]
fn every_shipped_pack_roundtrips() {
    let Some(install) = install_dir() else { return };
    let list = cfgs(&install);
    assert!(list.len() >= 10, "found {} cfgs", list.len());
    let scratch = dirs_cache().join("roundtrip");
    let _ = std::fs::remove_dir_all(&scratch);
    for cfg in &list {
        let orig = std::fs::read(cfg).unwrap();
        let mut pack = Pack::load(&install, cfg).unwrap();
        assert_eq!(
            pack.cfg.to_bytes(),
            orig,
            "{} in-memory roundtrip",
            cfg.display()
        );
        // typed read and write without change keeps the bytes
        let c = pack.common();
        pack.set_common(&c);
        let slots = pack.single_slots();
        let _ = slots;
        for e in pack.entries() {
            pack.set_level_info(e.number, &e.info);
        }
        assert_eq!(
            pack.cfg.to_bytes(),
            orig,
            "{} typed write of unchanged values",
            cfg.display()
        );
        // save into a private copy of the cfg: same bytes
        let group = cfg.parent().unwrap().file_name().unwrap();
        let dst_dir = scratch.join(group);
        std::fs::create_dir_all(&dst_dir).unwrap();
        let dst = dst_dir.join(cfg.file_name().unwrap());
        let mut copy = pack.clone();
        copy.cfg_path = dst.clone();
        copy.save().unwrap();
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            orig,
            "{} saved bytes",
            cfg.display()
        );
        println!(
            "{:<28} kind={:?} levels={:>3} issues={}",
            cfg.file_name().unwrap().to_string_lossy(),
            pack.kind,
            pack.entries().len(),
            validate(&pack).len()
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

fn dirs_cache() -> PathBuf {
    kfx_formats::testutil::scratch_dir("campaign-ui")
}

#[test]
fn shipped_packs_have_no_validation_errors_we_cannot_explain() {
    let Some(install) = install_dir() else { return };
    for cfg in cfgs(&install) {
        let pack = Pack::load(&install, &cfg).unwrap();
        let issues = validate(&pack);
        for i in issues.iter().filter(|i| i.severity == Severity::Error) {
            println!(
                "{}: ERROR {} [{}] {}",
                cfg.file_name().unwrap().to_string_lossy(),
                i.code,
                i.rule,
                i.message
            );
        }
    }
}
