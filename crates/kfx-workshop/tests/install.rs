//! Install plans and transactional apply/uninstall on private install layouts.
mod common;
use common::*;
use kfx_workshop::archive::Limits;
use kfx_workshop::classify::*;
use kfx_workshop::install::*;
use kfx_workshop::model::*;
use std::path::Path;

struct Env {
    s: Scratch,
    root: std::path::PathBuf,
    store: Store,
}

fn env(tag: &str) -> Env {
    let s = Scratch::new(tag);
    let root = s.join("install");
    private_install(&root);
    let store = Store::new(s.join("store"));
    Env { s, root, store }
}

fn do_plan(e: &Env, zip: &Path, opts: &PlanOptions) -> InstallPlan {
    let insp = classify_archive(zip, &Limits::default()).unwrap();
    plan(&e.root, zip, &insp, opts).unwrap()
}

fn lv(n: u32, lof: bool) -> Vec<(String, Vec<u8>)> {
    level_files(n, lof)
}

fn zip_levels(path: &Path, prefix: &str, levels: &[(u32, bool)], extra: &[(&str, &[u8])]) {
    let mut files: Vec<(String, Vec<u8>)> = vec![];
    for (n, lof) in levels {
        for (name, b) in lv(*n, *lof) {
            files.push((format!("{prefix}{name}"), b));
        }
    }
    for (n, b) in extra {
        files.push((n.to_string(), b.to_vec()));
    }
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    make_zip(path, &refs);
}

#[test]
fn flat_level_goes_to_personal_and_uninstalls_to_the_byte() {
    let e = env("in-flat");
    let z = e.s.join("lvl.zip");
    zip_levels(&z, "", &[(500, true)], &[("readme.txt", b"hello")]);
    let before = snapshot(&e.root);
    let p = do_plan(&e, &z, &PlanOptions::new("My Level"));
    assert_eq!(p.kind, Kind::Level);
    assert!(!p.is_blocked(), "{:?}", p.blocked);
    assert!(
        p.items
            .iter()
            .all(|i| i.dest.starts_with("levels/personal/map00500.")),
        "{:?}",
        p.items.iter().map(|i| &i.dest).collect::<Vec<_>>()
    );
    assert_eq!(p.count(Action::Overwrite), 0);
    assert_eq!(snapshot(&e.root), before, "planning writes nothing");
    let m = apply(&e.root, &p, &e.store, &Limits::default()).unwrap();
    assert!(e.root.join("levels/personal/map00500.slb").is_file());
    assert_eq!(
        std::fs::read(e.root.join("levels/personal/map00500.slb")).unwrap(),
        b"slb of level 500"
    );
    assert_eq!(m.levels.len(), 1);
    assert_eq!(e.store.list().len(), 1);
    let targets = open_targets(&e.root, &m);
    assert!(
        matches!(&targets[0], OpenTarget::Level { number: 500, slb, .. } if slb.ends_with("map00500.slb"))
    );
    let r = uninstall(&e.root, &m.id, &e.store, false).unwrap();
    assert_eq!(r.removed.len(), m.files.len());
    assert_eq!(snapshot(&e.root), before, "byte-identical after uninstall");
    assert!(e.store.list().is_empty());
}

#[test]
fn level_number_conflict_is_renumbered_and_lif_rewritten() {
    let e = env("in-renum");
    for (n, b) in lv(500, false) {
        std::fs::write(e.root.join("levels/personal").join(n), b).unwrap();
    }
    let before = snapshot(&e.root);
    let z = e.s.join("lvl.zip");
    zip_levels(&z, "", &[(500, false)], &[]);
    let p = do_plan(&e, &z, &PlanOptions::new("Other Level"));
    assert_eq!(p.renumbered.len(), 1);
    assert_eq!((p.renumbered[0].from, p.renumbered[0].to), (500, 501));
    assert_eq!(p.count(Action::Overwrite), 0, "never overwrites by default");
    let m = apply(&e.root, &p, &e.store, &Limits::default()).unwrap();
    assert!(e.root.join("levels/personal/map00501.slb").is_file());
    assert_eq!(
        std::fs::read(e.root.join("levels/personal/map00500.slb")).unwrap(),
        b"slb of level 500",
        "existing level untouched"
    );
    let lif =
        String::from_utf8(std::fs::read(e.root.join("levels/personal/map00501.lif")).unwrap())
            .unwrap();
    assert!(lif.contains("501, Level 500"), "{lif:?}");
    assert!(!lif.contains("500,"));
    assert_eq!(m.levels[0].renumbered_from, Some(500));
    uninstall(&e.root, &m.id, &e.store, false).unwrap();
    assert_eq!(snapshot(&e.root), before);
}

#[test]
fn skip_and_overwrite_policies() {
    let e = env("in-policy");
    for (n, b) in lv(7, true) {
        std::fs::write(e.root.join("levels/personal").join(n), b).unwrap();
    }
    let before = snapshot(&e.root);
    let z = e.s.join("lvl.zip");
    let mut files: Vec<(String, Vec<u8>)> = lv(7, true);
    for f in &mut files {
        f.1 = format!("NEW {}", f.0).into_bytes();
    }
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    make_zip(&z, &refs);
    // skip: nothing to do, plan says so
    let mut o = PlanOptions::new("L");
    o.conflict = Conflict::Skip;
    let p = do_plan(&e, &z, &o);
    assert!(
        p.is_blocked() && p.blocked[0].contains("nothing to install"),
        "{:?}",
        p.blocked
    );
    // overwrite: listed, backed up, restored
    o.conflict = Conflict::Overwrite;
    let p = do_plan(&e, &z, &o);
    assert!(p.count(Action::Overwrite) >= 10);
    assert!(p.describe().iter().any(|l| l.contains("OVERWRITTEN")));
    let m = apply(&e.root, &p, &e.store, &Limits::default()).unwrap();
    assert_eq!(
        std::fs::read(e.root.join("levels/personal/map00007.slb")).unwrap(),
        b"NEW map00007.slb"
    );
    assert!(m.files.iter().any(|f| f.backup.is_some()));
    uninstall(&e.root, &m.id, &e.store, false).unwrap();
    assert_eq!(
        snapshot(&e.root),
        before,
        "the old files are back, byte for byte"
    );
}

#[test]
fn modified_files_are_kept_unless_forced() {
    let e = env("in-modified");
    let z = e.s.join("lvl.zip");
    zip_levels(&z, "", &[(600, true)], &[]);
    let before = snapshot(&e.root);
    let m = apply(
        &e.root,
        &do_plan(&e, &z, &PlanOptions::new("L")),
        &e.store,
        &Limits::default(),
    )
    .unwrap();
    std::fs::write(e.root.join("levels/personal/map00600.txt"), b"my own edits").unwrap();
    let r = uninstall(&e.root, &m.id, &e.store, false).unwrap();
    assert_eq!(r.kept_modified, vec!["levels/personal/map00600.txt"]);
    assert_eq!(
        std::fs::read(e.root.join("levels/personal/map00600.txt")).unwrap(),
        b"my own edits"
    );
    assert_eq!(
        e.store.list().len(),
        1,
        "the record stays so the user can force later"
    );
    let r = uninstall(&e.root, &m.id, &e.store, true).unwrap();
    assert!(r.kept_modified.is_empty());
    assert_eq!(snapshot(&e.root), before);
}

#[test]
fn map_pack_creates_its_own_pack_cfg_and_folder() {
    let e = env("in-pack");
    let z = e.s.join("pack.zip");
    zip_levels(
        &z,
        "Cool Pack/",
        &[(900, true), (901, true), (902, false)],
        &[("Cool Pack/readme.txt", b"x")],
    );
    let before = snapshot(&e.root);
    let mut o = PlanOptions::new("Cool Pack");
    o.item = Some(ItemRef {
        id: 42,
        title: "Cool Pack".into(),
        slug: "cool-pack".into(),
        file_id: 1,
        file_name: "pack.zip".into(),
        ..Default::default()
    });
    let p = do_plan(&e, &z, &o);
    assert_eq!(p.kind, Kind::MapPack);
    assert!(
        p.items.iter().any(|i| i.dest == "levels/cool_pack.cfg"),
        "{:?}",
        p.items.iter().map(|i| &i.dest).collect::<Vec<_>>()
    );
    let m = apply(&e.root, &p, &e.store, &Limits::default()).unwrap();
    let cfg = std::fs::read_to_string(e.root.join("levels/cool_pack.cfg")).unwrap();
    assert!(
        cfg.contains("NAME = Cool Pack") && cfg.contains("LEVELS_LOCATION = levels/cool_pack"),
        "{cfg}"
    );
    assert!(
        !cfg.contains("NAME_TEXT_ID"),
        "a copied text id would show another pack's name"
    );
    assert!(
        cfg.contains("[strings]"),
        "kept the template's strings section"
    );
    // the game's own reader agrees where the levels are
    let resolved =
        kfx_formats::campaign::resolve(&e.root, &e.root.join("levels/cool_pack.cfg")).unwrap();
    assert_eq!(
        resolved.levels.iter().map(|l| l.number).collect::<Vec<_>>(),
        vec![900, 901, 902]
    );
    assert!(resolved.levels.iter().all(|l| l.slb.is_some()));
    assert_eq!(m.item.as_ref().unwrap().id, 42);
    uninstall(&e.root, &m.id, &e.store, false).unwrap();
    assert_eq!(
        snapshot(&e.root),
        before,
        "folders and the generated cfg are gone"
    );
}

#[test]
fn level_without_lof_or_lif_gets_a_generated_lif() {
    let e = env("in-genlif");
    let z = e.s.join("l.zip");
    let mut files = lv(321, true);
    files.retain(|f| !f.0.ends_with(".lof"));
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    make_zip(&z, &refs);
    let p = do_plan(&e, &z, &PlanOptions::new("Nameless"));
    apply(&e.root, &p, &e.store, &Limits::default()).unwrap();
    assert_eq!(
        std::fs::read_to_string(e.root.join("levels/personal/map00321.lif")).unwrap(),
        "321, Nameless\n"
    );
}

#[test]
fn multiplayer_levels_go_to_multiplayer_modern() {
    let e = env("in-mp");
    let z = e.s.join("mp.zip");
    let mut files = lv(777, false);
    files.retain(|f| !f.0.ends_with(".lif"));
    files.push((
        "map00777.lof".into(),
        b"KIND = MULTI\nNAME_TEXT = Duel\nPLAYERS = 2\n".to_vec(),
    ));
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    make_zip(&z, &refs);
    let insp = classify_archive(&z, &Limits::default()).unwrap();
    assert!(insp.class.levels[0].multiplayer);
    let p = plan(&e.root, &z, &insp, &PlanOptions::new("Duel")).unwrap();
    assert!(
        p.items
            .iter()
            .all(|i| i.dest.starts_with("multiplayer/modern/map00777.")),
        "{:?}",
        p.items.iter().map(|i| &i.dest).collect::<Vec<_>>()
    );
    assert!(p.warnings.iter().all(|w| !w.contains("without a .lof")));
}

#[test]
fn campaign_installs_rooted_and_a_second_install_is_blocked() {
    let e = env("in-camp");
    let z = e.s.join("camp.zip");
    let cfg = b"[common]\nNAME = Mine\nLEVELS_LOCATION = campgns/mine\n\
        LAND_LOCATION = campgns/mine_lnd\nSINGLE_LEVELS = 1 2\n[map00001]\nNAME_TEXT = \
        One\n[map00002]\nNAME_TEXT = Two\n";
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("campgns/mine.cfg".into(), cfg.to_vec()),
        ("campgns/mine_lnd/rgmap00.raw".into(), vec![1; 100]),
    ];
    for n in [1, 2] {
        for (name, b) in lv(n, true) {
            files.push((format!("campgns/mine/{name}"), b));
        }
    }
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    make_zip(&z, &refs);
    let before = snapshot(&e.root);
    let p = do_plan(&e, &z, &PlanOptions::new("Mine"));
    assert_eq!(p.kind, Kind::Campaign);
    let m = apply(&e.root, &p, &e.store, &Limits::default()).unwrap();
    assert!(
        e.root.join("campgns/mine.cfg").is_file()
            && e.root.join("campgns/mine/map00002.slb").is_file()
    );
    let resolved =
        kfx_formats::campaign::resolve(&e.root, &e.root.join("campgns/mine.cfg")).unwrap();
    assert_eq!(resolved.levels.len(), 2);
    assert!(open_targets(&e.root, &m)
        .iter()
        .any(|t| matches!(t, OpenTarget::Campaign { .. })));
    // again: the campaign exists
    let p2 = do_plan(&e, &z, &PlanOptions::new("Mine"));
    assert!(
        p2.is_blocked() && p2.blocked.iter().any(|b| b.contains("already installed")),
        "{:?}",
        p2.blocked
    );
    assert!(apply(&e.root, &p2, &e.store, &Limits::default()).is_err());
    uninstall(&e.root, &m.id, &e.store, false).unwrap();
    assert_eq!(snapshot(&e.root), before);
}

#[test]
fn rooted_mod_creates_load_order_and_uninstall_removes_it() {
    let e = env("in-mod1");
    let z = e.s.join("mods.zip");
    make_zip(
        &z,
        &[
            ("mods/", b""),
            ("mods/cool_mod/fxdata/magic.cfg", b"[power1]\n"),
            (
                "mods/cool_mod/mod.cfg",
                b"[mod]\nName = Cool\nDefaultLoadOrder = after_map\n",
            ),
        ],
    );
    let before = snapshot(&e.root);
    let p = do_plan(&e, &z, &PlanOptions::new("Cool"));
    assert_eq!(p.kind, Kind::Mod);
    assert!(p.creates_load_order);
    assert_eq!(
        p.load_order,
        vec![LoadOrderEntry {
            stage: "after_map".into(),
            name: "cool_mod".into(),
            already_listed: false
        }]
    );
    let m = apply(&e.root, &p, &e.store, &Limits::default()).unwrap();
    let lo = std::fs::read_to_string(e.root.join("mods/load_order.cfg")).unwrap();
    assert_eq!(load_order_mods(&lo, "after_map"), vec!["cool_mod"]);
    // the game-side parser agrees
    let cfg = kfx_map::mods::ModsConfig::parse(lo.as_bytes());
    assert_eq!(cfg.after_map, vec!["cool_mod"]);
    uninstall(&e.root, &m.id, &e.store, false).unwrap();
    assert_eq!(snapshot(&e.root), before);
}

#[test]
fn existing_load_order_is_edited_in_place_and_restored() {
    let e = env("in-mod2");
    let user = "; my order\r\n[after_base]\r\nmy_old_mod\r\n\
        ;disabled_mod\r\n\r\n[after_campaign]\r\n\r\n[after_map]\r\nother\r\n";
    std::fs::write(e.root.join("mods/load_order.cfg"), user).unwrap();
    let z = e.s.join("m.zip");
    make_zip(&z, &[("mods/new_mod/fxdata/rules.cfg", b"[game]\n")]);
    let before = snapshot(&e.root);
    let p = do_plan(&e, &z, &PlanOptions::new("New"));
    let m = apply(&e.root, &p, &e.store, &Limits::default()).unwrap();
    let after = std::fs::read_to_string(e.root.join("mods/load_order.cfg")).unwrap();
    assert!(
        after.contains("\r\nmy_old_mod\r\n;disabled_mod\r\nnew_mod\r\n\r\n[after_campaign]"),
        "CRLF kept, inserted at the end of the block: {after:?}"
    );
    assert!(!after.replace("\r\n", "").contains('\n'));
    // untouched since: restored byte for byte
    uninstall(&e.root, &m.id, &e.store, false).unwrap();
    assert_eq!(snapshot(&e.root), before);
    // edited by the user since: only our line goes
    let m = apply(
        &e.root,
        &do_plan(&e, &z, &PlanOptions::new("New")),
        &e.store,
        &Limits::default(),
    )
    .unwrap();
    let mut t = std::fs::read_to_string(e.root.join("mods/load_order.cfg")).unwrap();
    t.push_str("user_added_later\r\n");
    std::fs::write(e.root.join("mods/load_order.cfg"), &t).unwrap();
    let r = uninstall(&e.root, &m.id, &e.store, false).unwrap();
    assert_eq!(
        r.load_order_lines_removed,
        vec![("after_base".to_string(), "new_mod".to_string())]
    );
    let t = std::fs::read_to_string(e.root.join("mods/load_order.cfg")).unwrap();
    assert!(
        t.contains("user_added_later") && !t.contains("new_mod"),
        "{t:?}"
    );
}

#[test]
fn already_listed_mod_is_not_duplicated_and_full_blocks_are_reported() {
    let e = env("in-mod3");
    std::fs::write(
        e.root.join("mods/load_order.cfg"),
        "[after_base]\n;new_mod\n",
    )
    .unwrap();
    let z = e.s.join("m.zip");
    make_zip(&z, &[("mods/new_mod/fxdata/rules.cfg", b"x")]);
    let p = do_plan(&e, &z, &PlanOptions::new("New"));
    assert!(
        p.load_order[0].already_listed,
        "a disabled entry counts: the user decided"
    );
    let m = apply(&e.root, &p, &e.store, &Limits::default()).unwrap();
    assert!(m.load_order.is_none());
    let full: String = std::iter::once("[after_base]\n".to_string())
        .chain((0..50).map(|i| format!("m{i}\n")))
        .collect();
    std::fs::write(e.root.join("mods/load_order.cfg"), full).unwrap();
    let z2 = e.s.join("m2.zip");
    make_zip(&z2, &[("mods/another/fxdata/rules.cfg", b"x")]);
    let p = do_plan(&e, &z2, &PlanOptions::new("Another"));
    assert!(
        p.is_blocked() && p.blocked.iter().any(|b| b.contains("50")),
        "{:?}",
        p.blocked
    );
}

#[test]
fn load_order_text_helpers() {
    let t = "[after_base]\na\n\n[after_map]\n";
    assert_eq!(
        load_order_add(t, "after_base", "b"),
        "[after_base]\na\nb\n\n[after_map]\n"
    );
    assert_eq!(
        load_order_add(t, "after_map", "z"),
        "[after_base]\na\n\n[after_map]\nz\n"
    );
    assert!(load_order_add(t, "after_campaign", "c").ends_with("[after_campaign]\nc\n"));
    assert_eq!(
        load_order_remove("[after_base]\na\nb\n", "after_base", "a"),
        "[after_base]\nb\n"
    );
    assert_eq!(
        load_order_mods("[AFTER_BASE]\na\n; c\n\nb\n[x]\nq\n", "after_base"),
        vec!["a", "b"]
    );
}

#[test]
fn loose_config_bundle_becomes_a_mod_with_sprites_repacked() {
    let e = env("in-loose");
    let z = e.s.join("creature.zip");
    make_zip(
        &z,
        &[
            ("sprites.json", b"{}"),
            ("icons.json", b"{}"),
            ("stand_fp/r1frame01.png", b"png"),
            ("stand_tp/r1frame01.png", b"png2"),
            ("@cfg/dragon.cfg", b"[general]\n"),
            ("Thumbs.db", b"junk"),
        ],
    );
    let insp = classify_archive(&z, &Limits::default()).unwrap();
    assert_eq!(insp.class.kind, Kind::CreaturePack);
    assert_eq!(insp.class.junk, vec!["Thumbs.db"]);
    let before = snapshot(&e.root);
    let mut o = PlanOptions::new("Fire Dragon");
    o.item = Some(ItemRef {
        id: 9,
        ..Default::default()
    });
    let p = plan(&e.root, &z, &insp, &o).unwrap();
    let dests: Vec<&str> = p.items.iter().map(|i| i.dest.as_str()).collect();
    assert!(
        dests.contains(&"mods/fire_dragon/fxdata/fire_dragon.zip")
            && dests.contains(&"mods/fire_dragon/creatrs/dragon.cfg")
            && dests.contains(&"mods/fire_dragon/mod.cfg"),
        "{dests:?}"
    );
    let m = apply(&e.root, &p, &e.store, &Limits::default()).unwrap();
    // the repacked sprite zip has the frames under their own names
    let zp = e.root.join("mods/fire_dragon/fxdata/fire_dragon.zip");
    let l = kfx_workshop::archive::list(&zp, &Limits::default()).unwrap();
    let names: Vec<&str> = l.files().map(|f| f.path.as_str()).collect();
    assert!(
        names.contains(&"sprites.json") && names.contains(&"stand_fp/r1frame01.png"),
        "{names:?}"
    );
    let modcfg = std::fs::read_to_string(e.root.join("mods/fire_dragon/mod.cfg")).unwrap();
    assert!(modcfg.contains("KfxNetWorkshopItemId = 9"));
    assert_eq!(
        load_order_mods(
            &std::fs::read_to_string(e.root.join("mods/load_order.cfg")).unwrap(),
            "after_base"
        ),
        vec!["fire_dragon"]
    );
    uninstall(&e.root, &m.id, &e.store, false).unwrap();
    assert_eq!(snapshot(&e.root), before);
}

/// Unix only: the failure is injected with folder permissions, which Windows does not have (its
/// read-only attribute does not stop new files in a folder).
#[cfg(unix)]
#[test]
fn failure_in_a_later_folder_rolls_everything_back() {
    use std::os::unix::fs::PermissionsExt;
    let e = env("in-fail");
    // a rooted archive that touches two folders; the second is not writable
    let z = e.s.join("two.zip");
    let mut files: Vec<(String, Vec<u8>)> = vec![];
    for (n, b) in lv(450, true) {
        files.push((format!("levels/personal/{n}"), b));
    }
    for (n, b) in lv(451, true) {
        files.push((format!("levels/standard/{n}"), b));
    }
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    make_zip(&z, &refs);
    let before = snapshot(&e.root);
    let p = do_plan(&e, &z, &PlanOptions::new("Two"));
    assert!(!p.is_blocked(), "{:?}", p.blocked);
    let ro = e.root.join("levels/standard");
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::write(ro.join("probe"), b"x").is_ok();
    let r = apply(&e.root, &p, &e.store, &Limits::default());
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _ = std::fs::remove_file(ro.join("probe"));
    if writable {
        eprintln!(
            "running as a user that ignores directory permissions; the \
                failure injection did not bite"
        );
        return;
    }
    assert!(r.is_err());
    assert_eq!(
        snapshot(&e.root),
        before,
        "the folder committed first was rolled back"
    );
    assert!(
        e.store.list().is_empty(),
        "no manifest for a failed install"
    );
}

#[test]
fn plan_is_revalidated_at_apply_time() {
    let e = env("in-stale");
    let z = e.s.join("l.zip");
    zip_levels(&z, "", &[(610, true)], &[]);
    let p = do_plan(&e, &z, &PlanOptions::new("L"));
    std::fs::write(
        e.root.join("levels/personal/map00610.slb"),
        b"appeared meanwhile",
    )
    .unwrap();
    let err = apply(&e.root, &p, &e.store, &Limits::default()).unwrap_err();
    assert!(err.to_string().contains("appeared since"), "{err}");
    assert_eq!(
        std::fs::read(e.root.join("levels/personal/map00610.slb")).unwrap(),
        b"appeared meanwhile"
    );
}

#[test]
fn update_check_compares_ids_dates_and_sizes() {
    let file = |id: u32, date: &str, label: &str, broken: bool| FileEntry {
        id,
        name: "a.zip".into(),
        date: date.into(),
        size_label: label.into(),
        broken,
        ..Default::default()
    };
    let mut m = Manifest {
        version: 1,
        id: "x".into(),
        name: "x".into(),
        kind: "map".into(),
        install_root: String::new(),
        item: Some(ItemRef {
            id: 1,
            file_id: 10,
            file_name: "a.zip".into(),
            file_date: "2025-01-01".into(),
            file_size_label: "10.00KiB".into(),
            ..Default::default()
        }),
        archive_sha256: String::new(),
        installed_at: 0,
        files: vec![],
        dirs: vec![],
        load_order: None,
        levels: vec![],
        campaigns: vec![],
    };
    let d = |files: Vec<FileEntry>| ItemDetail {
        id: 1,
        files,
        ..Default::default()
    };
    assert_eq!(
        check_update(&m, &d(vec![file(10, "2025-01-01", "10.00KiB", false)])),
        UpdateStatus::UpToDate
    );
    assert!(matches!(
        check_update(
            &m,
            &d(vec![
                file(12, "2025-03-01", "11.00KiB", false),
                file(10, "2025-01-01", "10.00KiB", false)
            ])
        ),
        UpdateStatus::Available { .. }
    ));
    assert!(
        matches!(check_update(&m, &d(vec![file(10, "2025-01-01", "12.50KiB", false)])), UpdateStatus::Available { reason, .. } if reason.contains("replaced in place"))
    );
    assert_eq!(
        check_update(
            &m,
            &d(vec![
                file(12, "2025-03-01", "11.00KiB", true),
                file(10, "2025-01-01", "10.00KiB", false)
            ])
        ),
        UpdateStatus::UpToDate,
        "a broken newer file is not an update"
    );
    assert!(matches!(
        check_update(&m, &d(vec![])),
        UpdateStatus::Unknown(_)
    ));
    m.item = None;
    assert!(matches!(
        check_update(&m, &d(vec![file(12, "2025-03-01", "1B", false)])),
        UpdateStatus::Unknown(_)
    ));
}

#[test]
fn other_archives_are_unpacked_outside_the_game_folders() {
    let e = env("in-other");
    let z = e.s.join("tool.zip");
    make_zip(&z, &[("tool.exe", b"MZ"), ("lib.dll", b"MZ")]);
    let p = do_plan(&e, &z, &PlanOptions::new("Tool"));
    assert_eq!(p.kind, Kind::Other);
    assert!(p
        .items
        .iter()
        .all(|i| i.dest.starts_with("workshop_extras/tool/")));
    assert!(p.warnings.iter().any(|w| w.contains("not game content")));
}
