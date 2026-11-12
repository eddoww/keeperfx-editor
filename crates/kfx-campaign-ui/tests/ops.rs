//! Operations produce cfgs that `kfx_formats::campaign::resolve` reads back correctly.
mod common;
use common::*;
use kfx_campaign_ui::pack::{level_files, AddLevel, LevelSource, SingleSlot};
use kfx_campaign_ui::{
    create_pack, duplicate_pack, package, rename_pack, validate, Fix, NewPack, Pack,
    PackageOptions, Severity,
};
use kfx_formats::campaign::{resolve, PackKind};
use kfx_formats::levelinfo::{LevelKind, Lof};
use std::path::Path;

fn keeporig_level(inst: &Path, n: u32) -> LevelSource {
    let dir = inst.join("campgns/keeporig");
    let camp = kfx_formats::campaign::Campaign::load(&inst.join("campgns/keeporig.cfg")).unwrap();
    let mut s = LevelSource::new(&dir, n);
    s.info = camp.map_info(n);
    s
}

fn new_campaign(inst: &Path, name: &str) -> Pack {
    let mut spec = NewPack::campaign(name, "My Test Campaign");
    spec.land_from = Some(inst.join("campgns/keeporig.cfg"));
    create_pack(inst, &spec).unwrap()
}

fn assert_resolves(inst: &Path, pack: &Pack, singles: &[u32], bonus: &[u32], extra: &[u32]) {
    let r = resolve(inst, &pack.cfg_path).unwrap();
    assert_eq!(r.common.single_levels, singles, "SINGLE_LEVELS");
    assert_eq!(r.common.bonus_levels, bonus, "BONUS_LEVELS");
    assert_eq!(r.common.extra_levels, extra, "EXTRA_LEVELS");
    for n in singles
        .iter()
        .chain(bonus.iter().filter(|&&n| n > 0))
        .chain(extra)
    {
        let l = r
            .levels
            .iter()
            .find(|l| l.number == *n)
            .unwrap_or_else(|| panic!("level {n} resolved"));
        assert!(l.slb.is_some(), "level {n} has its map file");
    }
}

#[test]
fn new_campaign_from_template_and_two_copied_levels() {
    let Some(inst) = private_install("ops-new") else {
        return;
    };
    let mut pack = new_campaign(&inst, "mytest");
    // folders, cfg, land view copy (user's choice of source; nothing bundled)
    assert!(inst.join("campgns/mytest/text_eng.dat").is_file());
    assert!(inst.join("campgns/mytest_eng").is_dir());
    let src_raw = std::fs::read(inst.join("campgns/keeporig_lnd/rgmap01.raw")).unwrap();
    assert_eq!(
        std::fs::read(inst.join("campgns/mytest_lnd/rgmap01.raw")).unwrap(),
        src_raw
    );
    assert!(inst.join("campgns/mytest_lnd/viframe21.dat").is_file());
    let c = pack.common();
    assert_eq!(c.name.as_deref(), Some("My Test Campaign"));
    assert_eq!(c.land_view_end.as_ref().unwrap().0, "rgmap21");
    assert!(
        validate(&pack).iter().any(|i| i.code == "no-single-level"),
        "an empty campaign is flagged"
    );

    let a = pack
        .add_level(&{
            let mut r = AddLevel::new(keeporig_level(&inst, 1), LevelKind::Single);
            r.number = Some(11);
            r
        })
        .unwrap();
    let b = pack
        .add_level(&AddLevel::new(keeporig_level(&inst, 2), LevelKind::Single))
        .unwrap();
    assert_eq!(a.number, 11);
    assert_eq!(b.number, 1, "smallest free number");
    pack.save().unwrap();
    assert_resolves(&inst, &pack, &[11, 1], &[0, 0], &[]);
    let r = resolve(&inst, &pack.cfg_path).unwrap();
    assert_eq!(
        r.levels
            .iter()
            .find(|l| l.number == 11)
            .unwrap()
            .name
            .as_deref(),
        Some("Eversmile")
    );
    assert_eq!(
        r.levels.iter().find(|l| l.number == 11).unwrap().mapsize,
        (85, 85)
    );
    // files renamed, content identical
    let dir = inst.join("campgns/mytest");
    let src = inst.join("campgns/keeporig");
    assert!(!level_files(&dir, 11).is_empty());
    for f in level_files(&src, 1) {
        let ext = f
            .extension()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        if ext == "lof" || ext == "lif" {
            continue;
        }
        let copy = dir.join(format!("map00011.{ext}"));
        assert_eq!(
            std::fs::read(&f).unwrap(),
            std::fs::read(&copy).unwrap(),
            "{ext}"
        );
    }
    // each single level got its own land view frame
    let info = pack.level_info(11);
    assert_eq!(info.land_view, Some(("rgmap01".into(), "viframe01".into())));
    assert_eq!(
        pack.level_info(1).land_view,
        Some(("rgmap02".into(), "viframe02".into()))
    );
    assert_eq!(info.ensign_pos, Some((808, 546)));
    let issues = validate(&pack);
    for i in &issues {
        println!("{:?} {} {}", i.severity, i.code, i.message);
    }
    assert!(
        !issues.iter().any(|i| i.severity == Severity::Error),
        "no errors: {issues:?}"
    );
    assert!(scratch_files(&inst.join("campgns/mytest")).is_empty());
    cleanup(&inst);
}

#[test]
fn reorder_bonus_remove_and_renumber() {
    let Some(inst) = private_install("ops-edit") else {
        return;
    };
    let mut pack = new_campaign(&inst, "edit");
    for n in [1u32, 2, 3] {
        pack.add_level(&AddLevel::new(keeporig_level(&inst, n), LevelKind::Single))
            .unwrap();
    }
    let mut bonus = AddLevel::new(keeporig_level(&inst, 100), LevelKind::Bonus);
    bonus.bonus_for = Some(1);
    let b = pack.add_level(&bonus).unwrap();
    assert!(b.number >= 100);
    let moon = pack
        .add_level(&AddLevel::new(keeporig_level(&inst, 105), LevelKind::Extra))
        .unwrap()
        .number;
    pack.save().unwrap();
    assert_resolves(&inst, &pack, &[1, 2, 3], &[0, b.number, 0], &[moon]);
    // the bonus travels with its single level
    pack.move_single(1, 0).unwrap();
    pack.save().unwrap();
    assert_resolves(&inst, &pack, &[2, 1, 3], &[b.number, 0, 0], &[moon]);
    pack.move_single(0, 2).unwrap();
    assert_resolves_unsaved(&pack, &[1, 3, 2], &[0, 0, b.number]);
    // relink the bonus to the first level; a level that is already listed is refused
    pack.set_bonus(2, 0).unwrap();
    pack.set_bonus(0, b.number).unwrap();
    assert!(pack.set_bonus(1, 3).is_err());
    assert_resolves_unsaved(&pack, &[1, 3, 2], &[b.number, 0, 0]);
    // renumber moves files, block and list entry
    pack.renumber_level(3, 30).unwrap();
    assert!(level_files(&inst.join("campgns/edit"), 3).is_empty());
    assert!(!level_files(&inst.join("campgns/edit"), 30).is_empty());
    assert_eq!(
        pack.single_slots()[1],
        SingleSlot {
            level: 30,
            bonus: 0
        }
    );
    assert!(pack.cfg.map_info(30).is_some() && pack.cfg.map_info(3).is_none());
    // removing a single level takes its bonus level along; files stay unless asked
    let removed = pack.remove_level(1, false).unwrap();
    assert_eq!(removed, vec![1, b.number]);
    assert!(!level_files(&inst.join("campgns/edit"), 1).is_empty());
    assert!(pack.cfg.map_info(1).is_none());
    pack.remove_level(2, true).unwrap();
    assert!(level_files(&inst.join("campgns/edit"), 2).is_empty());
    pack.save().unwrap();
    let r = resolve(&inst, &pack.cfg_path).unwrap();
    assert_eq!(r.common.single_levels, vec![30]);
    assert_eq!(r.common.bonus_levels, vec![0]);
    // the limits of the game's arrays
    let mut full = new_campaign(&inst, "full");
    let src = keeporig_level(&inst, 1);
    for _ in 0..50 {
        full.add_level(&AddLevel::new(src.clone(), LevelKind::Single))
            .unwrap();
    }
    assert!(full
        .add_level(&AddLevel::new(src, LevelKind::Single))
        .unwrap_err()
        .to_string()
        .contains("at most 50"));
    cleanup(&inst);
}

fn assert_resolves_unsaved(pack: &Pack, singles: &[u32], bonus: &[u32]) {
    let slots = pack.single_slots();
    assert_eq!(slots.iter().map(|s| s.level).collect::<Vec<_>>(), singles);
    assert_eq!(slots.iter().map(|s| s.bonus).collect::<Vec<_>>(), bonus);
    // the saved text parses to the same lists
    let c = kfx_formats::campaign::Campaign::parse(&pack.cfg.to_bytes()).common();
    assert_eq!(c.single_levels, singles);
    assert_eq!(c.bonus_levels, bonus);
}

#[test]
fn rename_duplicate_package() {
    let Some(inst) = private_install("ops-pack") else {
        return;
    };
    let mut pack = new_campaign(&inst, "orig");
    pack.add_level(&AddLevel::new(keeporig_level(&inst, 1), LevelKind::Single))
        .unwrap();
    pack.save().unwrap();
    // duplicate keeps everything under the new name
    let dup = duplicate_pack(&inst, &pack.cfg_path, "copy", false).unwrap();
    assert_eq!(
        dup.common().levels_location.as_deref(),
        Some("campgns/copy")
    );
    assert_eq!(
        dup.common().land_location.as_deref(),
        Some("campgns/copy_lnd")
    );
    assert!(
        dup.cfg
            .strings()
            .iter()
            .all(|(_, f)| f.starts_with("campgns/copy/")),
        "{:?}",
        dup.cfg.strings()
    );
    assert_eq!(dup.cfg.speech()[0].1, "campgns/copy_eng");
    assert!(inst.join("campgns/copy/map00001.slb").is_file());
    assert!(
        inst.join("campgns/orig/map00001.slb").is_file(),
        "the original stays"
    );
    assert!(
        duplicate_pack(&inst, &pack.cfg_path, "copy", false).is_err(),
        "no overwrite"
    );
    let bare = duplicate_pack(&inst, &pack.cfg_path, "bare", true).unwrap();
    assert!(bare.single_slots().is_empty());
    assert!(level_files(&inst.join("campgns/bare"), 1).is_empty());
    assert!(inst.join("campgns/bare/text_eng.dat").is_file());
    // rename moves the cfg and the folders
    let ren = rename_pack(&inst, &dup.cfg_path, "moved").unwrap();
    assert!(!inst.join("campgns/copy.cfg").exists() && !inst.join("campgns/copy").exists());
    assert!(inst.join("campgns/moved_lnd/rgmap01.raw").is_file());
    assert_eq!(
        ren.common().levels_location.as_deref(),
        Some("campgns/moved")
    );
    assert_resolves(&inst, &ren, &[1], &[0], &[]);
    // package: right folder structure, deterministic
    let zip1 = cache().join("pack1.zip");
    let zip2 = cache().join("pack2.zip");
    let rep = package(&ren, &zip1, PackageOptions::default()).unwrap();
    package(&ren, &zip2, PackageOptions::default()).unwrap();
    assert_eq!(
        std::fs::read(&zip1).unwrap(),
        std::fs::read(&zip2).unwrap(),
        "same pack, same zip"
    );
    let names: Vec<String> = {
        let f = std::fs::File::open(&zip1).unwrap();
        let z = zip::ZipArchive::new(f).unwrap();
        z.file_names().map(str::to_string).collect()
    };
    assert!(names.contains(&"campgns/moved.cfg".to_string()));
    assert!(names.contains(&"campgns/moved/map00001.slb".to_string()));
    assert!(names.contains(&"campgns/moved/text_eng.dat".to_string()));
    assert!(names.contains(&"campgns/moved_lnd/rgmap01.raw".to_string()));
    assert!(
        names.iter().all(|n| n.starts_with("campgns/moved")),
        "{names:?}"
    );
    assert_eq!(names.len(), rep.files.len());
    // the not packed list names the game folder
    assert!(
        rep.not_packed.iter().all(|(_, why)| why == "a game folder"),
        "{:?}",
        rep.not_packed
    );
    let _ = std::fs::remove_file(&zip1);
    let _ = std::fs::remove_file(&zip2);
    assert!(scratch_files(&inst.join("campgns")).is_empty());
    cleanup(&inst);
}

#[test]
fn map_pack_levels_use_lof_and_lif() {
    let Some(inst) = private_install("ops-mappack") else {
        return;
    };
    let mut spec = NewPack::campaign("mymaps", "My maps");
    spec.kind = PackKind::Mappack;
    let mut pack = create_pack(&inst, &spec).unwrap();
    assert!(
        validate(&pack).iter().any(|i| i.code == "no-levels"),
        "an empty map pack is flagged"
    );
    let mut req = AddLevel::new(keeporig_level(&inst, 3), LevelKind::Free);
    req.number = Some(5);
    pack.add_level(&req).unwrap();
    pack.save().unwrap();
    let r = resolve(&inst, &pack.cfg_path).unwrap();
    assert_eq!(r.levels.len(), 1);
    assert_eq!(r.levels[0].number, 5);
    assert_eq!(r.levels[0].kinds, vec![LevelKind::Free]);
    assert_eq!(r.levels[0].name.as_deref(), Some("Waterdream Warm"));
    assert!(r.levels[0].lof.is_some());
    // level info of a map pack goes to the LOF on save
    let mut info = pack.level_info(5);
    info.name_text = Some("Renamed".into());
    pack.set_level_info(5, &info);
    assert!(pack.is_dirty());
    pack.save().unwrap();
    let lof = Lof::load(&inst.join("levels/mymaps/map00005.lof")).unwrap();
    assert_eq!(lof.info().name_text.as_deref(), Some("Renamed"));
    assert!(
        validate(&pack)
            .iter()
            .all(|i| i.severity != Severity::Error),
        "{:?}",
        validate(&pack)
    );
    // a .lif entry without a .lof: flagged, fixed by generating the .lof
    std::fs::remove_file(inst.join("levels/mymaps/map00005.lof")).unwrap();
    let issues = validate(&pack);
    let fix = issues
        .iter()
        .find(|i| i.code == "lof-missing")
        .and_then(|i| i.fix.clone())
        .expect("lof-missing with a fix");
    assert_eq!(fix, Fix::CreateLof(5));
    pack.apply_fix(&fix).unwrap();
    assert!(inst.join("levels/mymaps/map00005.lof").is_file());
    assert!(!validate(&pack).iter().any(|i| i.code == "lof-missing"));
    cleanup(&inst);
}

#[test]
fn validation_rules() {
    let Some(inst) = private_install("ops-validate") else {
        return;
    };
    let mut pack = new_campaign(&inst, "bad");
    pack.add_level(&AddLevel::new(keeporig_level(&inst, 1), LevelKind::Single))
        .unwrap();
    let codes = |p: &Pack| validate(p).into_iter().map(|i| i.code).collect::<Vec<_>>();
    assert!(!codes(&pack).iter().any(|c| matches!(
        *c,
        "level-file" | "mapsize" | "land-view-file" | "ensign-outside"
    )));

    // MAPSIZE disagrees with the map file
    let mut info = pack.level_info(1);
    info.mapsize = Some((60, 60));
    pack.set_level_info(1, &info);
    let issue = validate(&pack)
        .into_iter()
        .find(|i| i.code == "mapsize")
        .expect("mapsize flagged");
    assert_eq!(
        issue.fix,
        Some(Fix::SetMapSize {
            level: 1,
            size: (85, 85)
        })
    );
    pack.apply_fix(&issue.fix.unwrap()).unwrap();
    assert!(!codes(&pack).contains(&"mapsize"));
    // ensign outside the picture, missing land view file, missing speech
    let mut info = pack.level_info(1);
    info.ensign_pos = Some((1300, 100));
    info.land_view = Some(("rgmap77".into(), "viframe01".into()));
    info.speech = Some(("nothere.mp3".into(), "good01".into()));
    pack.set_level_info(1, &info);
    let c = codes(&pack);
    assert!(
        c.contains(&"ensign-outside")
            && c.contains(&"land-view-file")
            && c.contains(&"speech-file"),
        "{c:?}"
    );
    // duplicate level number and bonus list length
    pack.update_common(|c| {
        c.single_levels = vec![1, 1];
        c.bonus_levels = vec![0];
    });
    let c = codes(&pack);
    assert!(
        c.contains(&"duplicate-level") && c.contains(&"bonus-length"),
        "{c:?}"
    );
    let fix = validate(&pack)
        .into_iter()
        .find(|i| i.code == "bonus-length")
        .unwrap()
        .fix
        .unwrap();
    pack.apply_fix(&fix).unwrap();
    assert_eq!(pack.common().bonus_levels, vec![0, 0]);
    // a missing level file, a missing folder (fix creates it), a missing [speech] section
    std::fs::remove_file(inst.join("campgns/bad/map00001.slb")).unwrap();
    assert!(codes(&pack).contains(&"level-file"));
    pack.update_common(|c| c.creatures_location = Some("campgns/bad_crtr".into()));
    let i = validate(&pack)
        .into_iter()
        .find(|i| i.code == "folder-missing")
        .unwrap();
    assert_eq!(i.severity, Severity::Warning);
    pack.apply_fix(&i.fix.unwrap()).unwrap();
    assert!(inst.join("campgns/bad_crtr").is_dir());
    pack.cfg.doc.remove_section("speech");
    assert!(codes(&pack).contains(&"section-missing"));
    // a language whose strings file is missing: the fix creates an empty table the game accepts
    pack.cfg
        .doc
        .add("strings", "FRE", "campgns/bad/text_fre.dat");
    let i = validate(&pack)
        .into_iter()
        .find(|i| i.code == "strings-file")
        .unwrap();
    pack.apply_fix(&i.fix.unwrap()).unwrap();
    let t = kfx_formats::strings::StringTable::load(
        &inst.join("campgns/bad/text_fre.dat"),
        kfx_formats::strings::Lang::French,
    )
    .unwrap();
    assert!(t.len() >= 16);
    // every issue cites the C source
    assert!(validate(&pack)
        .iter()
        .all(|i| i.rule.contains(".c:") || i.rule.contains(".h:")));
    cleanup(&inst);
}

#[test]
fn texts_languages_and_shared_tables() {
    let Some(inst) = private_install("ops-texts") else {
        return;
    };
    let mut pack = new_campaign(&inst, "words");
    pack.add_level(&AddLevel::new(keeporig_level(&inst, 1), LevelKind::Single))
        .unwrap();
    let mut texts = kfx_campaign_ui::Texts::load(&pack);
    assert_eq!(texts.langs.len(), 1);
    // the English table starts empty; the name id resolves through the GUI table fallback
    let mut info = pack.level_info(1);
    info.name_id = Some("500".into());
    pack.set_level_info(1, &info);
    texts.set(0, 500, "Eversmile Reborn").unwrap();
    let fre = texts
        .add_language(
            &mut pack,
            kfx_formats::strings::Lang::French,
            Some("campgns/words_fre"),
        )
        .unwrap();
    assert_eq!(
        texts.cell(fre, 500),
        kfx_campaign_ui::texts::CellState::Missing
    );
    assert_eq!(texts.cell(0, 500), kfx_campaign_ui::texts::CellState::Ok);
    assert_eq!(texts.cell(0, 501), kfx_campaign_ui::texts::CellState::Empty);
    texts.set(fre, 500, "Souriant").unwrap();
    assert_eq!(texts.cell(fre, 500), kfx_campaign_ui::texts::CellState::Ok);
    assert!(
        texts.set(0, 2000, "x").is_err(),
        "ids from 2000 are not campaign strings"
    );
    let written = texts.save().unwrap();
    assert_eq!(written.len(), 2);
    pack.save().unwrap();
    let again = kfx_campaign_ui::Texts::load(&pack);
    assert_eq!(again.get(0, 500).as_deref(), Some("Eversmile Reborn"));
    assert_eq!(again.get(1, 500).as_deref(), Some("Souriant"));
    assert_eq!(pack.cfg.strings().len(), 2);
    // the strings parse the way the game picks them
    let sel = kfx_formats::strings::campaign_strings_files(
        &pack.cfg.strings(),
        kfx_formats::strings::Lang::French,
    );
    assert_eq!(sel.fname.as_deref(), Some("campgns/words/text_fre.dat"));
    // a table of the game (the original campaign uses fxdata/gtext_eng.dat) is read only
    let orig = Pack::load(&inst, &inst.join("campgns/keeporig.cfg")).unwrap();
    let mut t = kfx_campaign_ui::Texts::load(&orig);
    assert!(t.langs[0].shared);
    assert!(t.set(0, 202, "changed").is_err());
    assert_eq!(t.get(0, 202).as_deref(), Some("Eversmile"));
    cleanup(&inst);
}
