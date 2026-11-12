//! Override writers against the real install (skipped when it is absent): minimal partial
//! files, revert, comment preservation, slabset / columnset merge checked through
//! `kfx_map::gamedata`, schema, creature copy, effective GameData with unsaved edits.
use kfx_config_ui::cfgedit::Pending;
use kfx_config_ui::schema::Schema;
use kfx_config_ui::slabset::ObjView;
use kfx_config_ui::{ConfigSession, LayerStack, MemFs, StdFs};
use kfx_formats::testutil;
use kfx_map::gamedata::{ConfigLayers, GameData};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const LEVEL: u32 = 12;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kfx-config-ui-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Session over the install's base + a map layer in `levels` (through `fs`).
fn session(
    fs: Arc<dyn kfx_config_ui::ConfigFs + Send + Sync>,
    install: &Path,
    levels: &Path,
) -> ConfigSession {
    let layers = LayerStack::for_level(&*fs, install, None, None, Some(levels), Some(LEVEL));
    ConfigSession::new(fs, layers)
}

fn text(p: &Pending) -> String {
    match p {
        Pending::Write(b) => String::from_utf8_lossy(b).into_owned(),
        Pending::Delete => "<delete>".into(),
        Pending::Unchanged => "<unchanged>".into(),
    }
}

#[test]
fn rules_map_override_is_minimal_and_reverts() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let levels = PathBuf::from("/virtual/levels");
    let fs = MemFs::over(StdFs);
    let mut s = session(Arc::new(fs.clone()), &inst, &levels);
    assert_eq!(s.target(), 1);
    let base = s.cfg_value("rules.cfg", "game", "GoldPerHoard").unwrap();
    let new = if base.trim() == "2500" {
        "2600"
    } else {
        "2500"
    };
    assert!(s.set_cfg("rules.cfg", "game", "GoldPerHoard", new));
    let p = s.cfg("rules.cfg").unwrap().pending();
    assert_eq!(text(&p), format!("[game]\nGoldPerHoard = {new}\n"));
    s.save().unwrap();
    let path = levels.join("map00012.rules.cfg");
    assert_eq!(
        String::from_utf8(fs.changes()[&path].clone().unwrap()).unwrap(),
        format!("[game]\nGoldPerHoard = {new}\n")
    );
    // revert removes the file again
    assert!(s.revert_cfg("rules.cfg", "game", "GoldPerHoard"));
    assert_eq!(s.cfg("rules.cfg").unwrap().pending(), Pending::Delete);
    s.save().unwrap();
    assert_eq!(fs.changes()[&path], None);
}

#[test]
fn existing_override_keeps_comments() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let levels = PathBuf::from("/virtual/levels2");
    let fs = MemFs::over(StdFs);
    let orig = "; level 12 tweaks\r\n[game]\r\n; cheaper hoards\r\n\
        GoldPerHoard = 1000\r\n\r\n[research]\r\nResearch = MAGIC POWER_HAND 0\r\n";
    kfx_config_ui::ConfigFs::write(&fs, &levels.join("map00012.rules.cfg"), orig.as_bytes())
        .unwrap();
    let mut s = session(Arc::new(fs.clone()), &inst, &levels);
    assert_eq!(
        s.cfg_value("rules.cfg", "game", "GoldPerHoard").as_deref(),
        Some("1000")
    );
    assert!(s.set_cfg("rules.cfg", "game", "PayDayGap", "6000"));
    assert!(s.set_cfg("rules.cfg", "game", "GoldPerHoard", "1200"));
    let t = text(&s.cfg("rules.cfg").unwrap().pending());
    assert_eq!(
        t,
        "; level 12 tweaks\r\n[game]\r\n; cheaper hoards\r\n\
            GoldPerHoard = 1200\r\nPayDayGap = 6000\r\n\r\n[research]\r\nResearch = MAGIC \
            POWER_HAND 0\r\n"
    );
    // the research list is a whole block: editing it keeps it complete
    let c = s.cfg("rules.cfg").unwrap();
    let (_, mut lines) = c.block_at_target("research").unwrap();
    lines.push(("Research".into(), "ROOM TREASURE 100".into()));
    assert!(c.set_block("research", &lines));
    assert!(c.target.to_string().ends_with(
        "[research]\r\nResearch = MAGIC POWER_HAND 0\r\nResearch = ROOM TREASURE 100\r\n"
    ));
}

/// Load slabsets two ways and compare: base + our partial map file, versus a single full file
/// of the edited state as the base.
#[test]
fn slabset_partial_merges_like_the_game() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let dir = tmp("slabset");
    let levels = dir.join("levels");
    std::fs::create_dir_all(&levels).unwrap();
    let mut s = session(Arc::new(StdFs), &inst, &levels);
    const LIB: usize = 18; // BOOK_SHELVES
    let m = s.slabset();
    // 1. columns of one style changed (objects must be written too when the style has some)
    let mut st = m.style(LIB, 8);
    st.columns[4] = 1;
    m.set_style(LIB, 8, st.clone());
    // 2. objects only, on another style
    let mut st2 = m.style(LIB, 0);
    let mut o = toml::Table::new();
    ObjView {
        is_light: true,
        subtile: 4,
        rel: [128, 128, 512],
        thing_type: "1".into(),
        subtype: "2".into(),
        effect_range: 5,
        orientation: 0,
    }
    .write(&mut o);
    st2.objects.push(o);
    m.set_style(LIB, 0, st2);
    // 3. objects cleared on a style that has some (if any)
    let with_objs = (0..28).find(|&k| k != 0 && k != 8 && !m.style(LIB, k).objects.is_empty());
    if let Some(k) = with_objs {
        let mut st3 = m.style(LIB, k);
        st3.objects.clear();
        m.set_style(LIB, k, st3);
    }
    // 4. an edit put back to the inherited value disappears
    let mut st4 = m.style(LIB, 1);
    let keep = st4.clone();
    st4.columns[0] += 1;
    m.set_style(LIB, 1, st4);
    m.set_style(LIB, 1, keep);
    let pending = text(&m.pending());
    println!("{pending}");
    assert!(
        !pending.contains("[slab18.W]"),
        "reverted style must not be written"
    );
    assert!(pending.contains("[slab18.ALL]"));
    // the game's TOML parser needs explicit parent headers (deps/centitoml/toml_api.c:702)
    assert!(pending.starts_with("[slab18]\n"), "{pending}");
    assert!(pending.contains("[[slab18.S_objects]]"));
    assert!(
        !pending.contains("[slab18.S]"),
        "objects-only change writes no style table"
    );
    s.save().unwrap();
    assert!(levels.join("map00012.slabset.toml").exists());

    // expected: the whole edited slabset as one full base file
    let expected_dir = dir.join("expected");
    std::fs::create_dir_all(&expected_dir).unwrap();
    let fx = testutil::resolve_ci(&inst, "fxdata").unwrap();
    for f in [
        "terrain.cfg",
        "cubes.cfg",
        "columnset.toml",
        "objects.cfg",
        "effects.toml",
        "spritecolors.toml",
    ] {
        std::fs::copy(fx.join(f), expected_dir.join(f)).unwrap();
    }
    let mut full = String::new();
    let m = s.slabset();
    for kind in 0..m.kind_count() {
        for style in 0..28 {
            let st = m.style(kind, style);
            let name = kfx_map::gamedata::SLAB_STYLE_NAMES[style];
            full.push_str(&format!(
                "[slab{kind}.{name}]\nColumns = {:?}\n",
                st.columns
            ));
            for o in &st.objects {
                full.push_str(&format!("[[slab{kind}.{name}_objects]]\n{}", o));
            }
        }
    }
    std::fs::write(expected_dir.join("slabset.toml"), full).unwrap();
    let got = GameData::load(&ConfigLayers::for_level(
        &inst,
        None,
        Some(&levels),
        None,
        Some(LEVEL),
    ))
    .unwrap();
    let want = GameData::load(&ConfigLayers::base(&expected_dir)).unwrap();
    assert_eq!(got.slabset_raw, want.slabset_raw);
    assert_eq!(got.slab_objects, want.slab_objects);
    // and the edit is really there
    let id = GameData::slabset_index(LIB, 8);
    assert_eq!(got.slabset_raw[id][4], -1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn columnset_override_writes_whole_columns() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let dir = tmp("columnset");
    let mut s = session(Arc::new(StdFs), &inst, &dir);
    let cs = s.columnset();
    let mut c = cs.col(5);
    c.cubes[3] = 7;
    let c = c.with_auto();
    cs.set_col(5, c);
    let p = text(&cs.pending());
    assert!(p.starts_with("[column5]\n"));
    for k in [
        "Lintel",
        "Height",
        "SolidMask",
        "FloorTexture",
        "Orientation",
        "Cubes",
    ] {
        assert!(p.contains(k), "{k} missing in\n{p}");
    }
    s.save().unwrap();
    let gd = GameData::load(&ConfigLayers::for_level(
        &inst,
        None,
        Some(&dir),
        None,
        Some(LEVEL),
    ))
    .unwrap();
    assert_eq!(gd.columnset[5].cubes[3], 7);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn effective_gamedata_sees_unsaved_edits() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let dir = tmp("effective");
    let fs = MemFs::over(StdFs);
    let mut s = session(Arc::new(fs.clone()), &inst, &dir.join("levels"));
    let w = s.cfg_value("terrain.cfg", "slab12", "Wibble").unwrap();
    let new = if w.trim() == "2" { "1" } else { "2" };
    s.set_cfg("terrain.cfg", "slab12", "Wibble", new);
    let mut st = s.slabset().style(10, 8);
    st.columns[0] = 3;
    s.slabset().set_style(10, 8, st);
    let gd = s.effective_gamedata(&dir.join("scratch")).unwrap();
    assert_eq!(gd.slabs[12].wibble.to_string(), new);
    assert_eq!(gd.slabset_raw[GameData::slabset_index(10, 8)][0], -3);
    // nothing written through the host fs
    assert!(fs.changes().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_creature_from_copy() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let levels = PathBuf::from("/virtual/levels3");
    let fs = MemFs::over(StdFs);
    let mut s = session(Arc::new(fs.clone()), &inst, &levels);
    let key = kfx_config_ui::creature::new_creature_from_copy(&mut s, "DRAGON", "WYRM").unwrap();
    assert_eq!(key, "creatrs/wyrm.cfg");
    assert!(s.creature_names().last().is_some_and(|n| n == "WYRM"));
    assert_eq!(
        s.cfg_value(&key, "attributes", "Health"),
        s.cfg_value("creatrs/dragon.cfg", "attributes", "Health")
    );
    s.save().unwrap();
    let ch = fs.changes();
    assert!(ch.contains_key(&levels.join("map00012.wyrm.cfg")));
    let cc = String::from_utf8(ch[&levels.join("map00012.creature.cfg")].clone().unwrap()).unwrap();
    assert!(
        cc.starts_with("[common]\nCreatures = ") && cc.trim_end().ends_with("WYRM"),
        "{cc}"
    );
    assert_eq!(cc.lines().count(), 2);
}

#[test]
fn schema_loads_and_covers_the_files() {
    let s = Schema::builtin();
    assert!(s.version >= 1);
    for f in [
        "rules.cfg",
        "terrain.cfg",
        "objects.cfg",
        "trapdoor.cfg",
        "magic.cfg",
        "creature.cfg",
        "cubes.cfg",
        "slabset.toml",
        "columnset.toml",
        "effects.toml",
        "sounds.cfg",
    ] {
        assert!(s.file(f).is_some(), "{f} missing from the schema");
    }
    assert!(s.file("creatrs/dragon.cfg").is_some());
    let r = s.file("rules.cfg").unwrap();
    let k = r.section("game").unwrap().key("PotOfGoldHolds").unwrap();
    assert_eq!(k.args[0].ty, "int");
    assert!(r.section("research").unwrap().replaces_whole_block());
    assert!(s.file("terrain.cfg").unwrap().section("slab12").is_some());
    let c = s.coverage();
    println!(
        "schema coverage: full {} partial {} unknown {} documented {}",
        c.full, c.partial, c.unknown, c.documented
    );
    for f in &s.files {
        let c = Schema::coverage_of(f);
        println!(
            "  {:20} full {:4} partial {:4} unknown {:3} documented {:4}",
            f.file, c.full, c.partial, c.unknown, c.documented
        );
    }
    assert!(c.full > 300);
}

#[test]
fn shipped_values_validate() {
    // every shipped rules.cfg / terrain.cfg / creatrs value of a key with full type info passes
    // without hard errors (tests the validator against real data)
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let mut s = session(Arc::new(StdFs), &inst, Path::new("/virtual/none"));
    let schema = Schema::builtin();
    let mut errors = vec![];
    let mut checked = 0;
    for file in [
        "rules.cfg",
        "terrain.cfg",
        "objects.cfg",
        "trapdoor.cfg",
        "magic.cfg",
        "creatrs/dragon.cfg",
        "cubes.cfg",
    ] {
        let spec = schema.file(file).unwrap().clone();
        let secs = s.cfg(file).unwrap().section_names();
        for sec in secs {
            let Some(sp) = spec.section(&sec) else {
                continue;
            };
            let keys = s.cfg(file).unwrap().keys(&sec);
            for k in keys {
                let Some(ks) = sp.key(&k) else { continue };
                let v = s.cfg_value(file, &sec, &k).unwrap();
                let issues = kfx_config_ui::validate::validate(ks, &v, schema, &mut |r| s.names(r));
                checked += 1;
                for i in issues.into_iter().filter(|i| i.error) {
                    errors.push(format!("{file} [{sec}] {k} = {v}: {}", i.msg));
                }
            }
        }
    }
    println!("checked {checked} values, {} hard errors", errors.len());
    for e in errors.iter().take(40) {
        println!("  {e}");
    }
    assert!(errors.len() * 50 < checked, "too many false errors");
}

#[test]
fn zip_is_readable_by_customset() {
    use kfx_config_ui::spritezip::{write_zip, NewIcon, NewSprite};
    use kfx_gfx::{CustomSet, Rgba};
    let dir = tmp("zip");
    let mut f1 = Rgba::new(20, 30);
    let mut f2 = Rgba::new(20, 30);
    for y in 0..30 {
        for x in 0..20 {
            f1.put(x, y, [200, 10, 10, 255]);
            f2.put(x, y, [10, 200, 10, if x < 10 { 255 } else { 0 }]);
        }
    }
    let rot: Vec<Vec<Rgba>> = (0..5).map(|_| vec![f1.clone(), f2.clone()]).collect();
    let z = write_zip(
        None,
        &[
            NewSprite {
                name: "TEST_ANIM".into(),
                td: vec![vec![f1.clone(), f2.clone()]],
                ..Default::default()
            },
            NewSprite {
                name: "TEST_ROT".into(),
                rotatable: true,
                td: rot,
                td_offset: Some((10, 29)),
                ..Default::default()
            },
        ],
        &[NewIcon {
            name: "TEST_ICON".into(),
            frames: vec![f1.clone(), f2.clone()],
        }],
    )
    .unwrap();
    // merge into the existing zip: replace TEST_ANIM, keep the others
    let z = write_zip(
        Some(&z),
        &[NewSprite {
            name: "TEST_ANIM".into(),
            td: vec![vec![f2.clone(), f1.clone(), f2.clone()]],
            ..Default::default()
        }],
        &[],
    )
    .unwrap();
    let p = dir.join("map00012.zip");
    std::fs::write(&p, z).unwrap();
    let mut cs = CustomSet::new();
    let sum = cs.load_zip(&p).unwrap();
    assert!(sum.warnings.is_empty(), "{:?}", sum.warnings);
    assert_eq!(sum.animations, 2);
    assert_eq!(sum.icons, 1);
    let a = cs.animation("TEST_ANIM").unwrap();
    assert_eq!(a.frames(), 3);
    let r = cs.animation("TEST_ROT").unwrap();
    assert!(r.rotable);
    assert_eq!(r.td.len(), 5);
    let fr = cs.frame(r.id).unwrap();
    assert_eq!((fr.offset_x, fr.offset_y), (-10, -29));
    let i = cs.icon("TEST_ICON").unwrap();
    assert_eq!(i.frames, 2);
    assert_eq!(cs.icon_image(i.id).unwrap().pixel(0, 0), [200, 10, 10, 255]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tile_import_writes_a_map_tmap() {
    use kfx_config_ui::tiles::{atlas_png, import_atlas_png, tmap_override_path};
    use kfx_gfx::textures::{Part, TextureSet};
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let i = kfx_gfx::Install::new(&inst);
    let pal = kfx_gfx::Palette::load(&i).unwrap();
    let set = TextureSet::load_std(&i, 0).unwrap();
    let png = atlas_png(&set, Part::A, &pal).unwrap();
    let back = import_atlas_png(&png, &pal, false).unwrap();
    // exported colours map back to colours of the same value
    let a = set.atlas_indexed(Part::A);
    let same = a
        .iter()
        .zip(&back)
        .filter(|(x, y)| pal.rgba(**x) == pal.rgba(**y))
        .count();
    assert_eq!(same, a.len());
    let layer = kfx_config_ui::Layer::map("/x/levels", 12);
    assert_eq!(
        tmap_override_path(&layer, Part::B, 3).unwrap(),
        PathBuf::from("/x/levels/map00012.tmapb003.dat")
    );
    let dithered = import_atlas_png(&png, &pal, true).unwrap();
    assert_eq!(dithered.len(), back.len());
}
