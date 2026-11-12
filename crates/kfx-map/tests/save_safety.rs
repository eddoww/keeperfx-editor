//! Map saves never lose data: a failed save leaves the map as it was (with a fault-injection
//! check of every step of the save transaction), unreadable FX and `.une` files are never
//! overwritten, and things the game rejects block the save. Needs the test install (skipped
//! otherwise). Scratch folders go under `kfx_formats::testutil::scratch_dir`.
use kfx_formats::atomic::FaultFs;
use kfx_formats::level_bin::MapDims;
use kfx_formats::level_fx::{FxClass, FxThing, StlCoord};
use kfx_formats::testutil;
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns};
use kfx_map::validate::{validate, Severity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn setup() -> Option<(PathBuf, GameData)> {
    let install = testutil::install_dir()?;
    let gd = GameData::load(&ConfigLayers::for_level(&install, None, None, None, None)).ok()?;
    Some((install, gd))
}

fn scratch(name: &str) -> PathBuf {
    let d = kfx_formats::testutil::scratch_dir(&format!(
        "kfx-map/kfx-save-safety-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Copy the files of level `n` (lower-case names) into a fresh `dst`.
fn copy_map(src: &Path, n: u32, dst: &Path) {
    let _ = std::fs::remove_dir_all(dst);
    std::fs::create_dir_all(dst).unwrap();
    let pre = format!("map{n:05}.");
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let f = e.file_name().to_string_lossy().to_ascii_lowercase();
        if f.starts_with(&pre) && e.path().is_file() {
            std::fs::copy(e.path(), dst.join(f)).unwrap();
        }
    }
}

/// Every file of the folder by name.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

fn d85() -> MapDims {
    MapDims::new(85, 85)
}

fn set_mode(p: &Path, readonly: bool) {
    let mut perm = std::fs::metadata(p).unwrap().permissions();
    perm.set_readonly(readonly);
    std::fs::set_permissions(p, perm).unwrap();
}

/// A save that fails part-way must not leave some files new and some old. A read-only `.tngfx`
/// stands for any write failure (disk full, permission): the save must fail and leave every file of
/// the map as it was.
#[test]
fn failed_save_leaves_the_map_unchanged() {
    let Some((install, gd)) = setup() else { return };
    let w = scratch("partial");
    copy_map(&install.join("multiplayer/classic"), 55, &w);
    let mut d = MapDoc::load(&w, 55, d85(), &gd).unwrap();
    d.set_slab(&gd, 10, 10, 10).unwrap();
    let before = snapshot(&w);
    let victim = w.join("map00055.tngfx");
    set_mode(&victim, true);
    let r = d.save(&gd, &w, SaveColumns::Current);
    set_mode(&victim, false);
    assert!(r.is_err(), "the save must report the failure");
    assert_eq!(snapshot(&w), before, "a failed save changed files");
    let _ = std::fs::remove_dir_all(&w);
}

/// Inject a failure at every step of the save transaction (each temporary file
/// write, backup, rename, removal and the folder sync) in turn. After each, the folder holds
/// exactly the old files or exactly the new ones, and no temporary file is left.
#[test]
fn save_transaction_is_all_or_nothing_at_every_step() {
    let Some((install, gd)) = setup() else { return };
    for (rel, n, legacy) in [
        ("multiplayer/classic", 55u32, false),
        ("multiplayer/classic", 55, true),
        ("campgns/keeporig", 6, false),
    ] {
        let src = install.join(rel);
        let w = scratch(&format!("faults-{n}-{legacy}"));
        let mut d = MapDoc::load(&src, n, d85(), &gd).unwrap();
        d.set_slab(&gd, 20, 20, 10).unwrap();
        // an upper-case leftover the save removes, to cover removals too
        let tx = |dir: &Path| {
            if legacy {
                d.save_legacy_transaction(&gd, dir, SaveColumns::Current)
                    .unwrap()
                    .0
            } else {
                d.save_transaction(&gd, dir, SaveColumns::Current).unwrap()
            }
        };
        // (only where it is a separate file: on a case-insensitive file system it would be the
        // map's own .slb)
        let prepare = || {
            copy_map(&src, n, &w);
            if !w.join(format!("MAP{n:05}.SLB")).exists() {
                std::fs::write(w.join(format!("MAP{n:05}.SLB")), b"stale upper-case copy").unwrap();
            }
        };
        prepare();
        let old = snapshot(&w);
        let counter = FaultFs::new(None);
        tx(&w).commit_with(&counter).unwrap();
        let new = snapshot(&w);
        assert_ne!(old, new);
        assert!(!new.contains_key(&format!("MAP{n:05}.SLB")));
        let steps = counter.steps();
        for k in 0..steps {
            prepare();
            let f = FaultFs::new(Some(k));
            let r = tx(&w).commit_with(&f);
            let now = snapshot(&w);
            assert!(
                now.keys().all(|k| !kfx_formats::atomic::is_scratch_name(k)),
                "{rel}/{n} legacy={legacy} step {k}: scratch files left: {:?}",
                now.keys().collect::<Vec<_>>()
            );
            match r {
                Ok(_) => assert!(
                    now == new,
                    "{rel}/{n} legacy={legacy} step {k}: reported success but not the new state"
                ),
                Err(e) => assert!(
                    now == old,
                    "{rel}/{n} legacy={legacy} step {k}: failed ({e}) but not the old state: {:?}",
                    now.keys().collect::<Vec<_>>()
                ),
            }
            // and the map loads as one or the other
            let back = MapDoc::load(&w, n, d85(), &gd).unwrap();
            assert_eq!(
                back.kinds,
                if now == new {
                    d.kinds.clone()
                } else {
                    MapDoc::load(&src, n, d85(), &gd).unwrap().kinds
                }
            );
        }
        println!(
            "{rel}/{n} legacy={legacy}: {steps} steps, each failure left the old or the new map"
        );
        let _ = std::fs::remove_dir_all(&w);
    }
}

/// A `.tngfx` the editor cannot parse must not load as "no things" and be emptied by the next
/// save: the save is refused and the file kept.
#[test]
fn unreadable_fx_file_is_not_overwritten() {
    let Some((install, gd)) = setup() else { return };
    let w = scratch("unreadable");
    copy_map(&install.join("multiplayer/classic"), 55, &w);
    let p = w.join("map00055.tngfx");
    let text = std::fs::read_to_string(&p).unwrap().replacen(
        "[thing0]\n",
        "[thing0]\nThingType = = 1\n",
        1,
    );
    std::fs::write(&p, &text).unwrap();
    let d = MapDoc::load(&w, 55, d85(), &gd).unwrap();
    let r = d.save(&gd, &w, SaveColumns::Current);
    assert!(
        r.is_err(),
        "saving over an unreadable .tngfx must be refused"
    );
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        text,
        "the unreadable file was changed"
    );
    let _ = std::fs::remove_dir_all(&w);
}

/// What the unreadable state says, and that discarding is an explicit step.
#[test]
fn unreadable_parts_are_reported_and_discarding_is_explicit() {
    let Some((install, gd)) = setup() else { return };
    let w = scratch("discard");
    copy_map(&install.join("multiplayer/classic"), 55, &w);
    let p = w.join("map00055.aptfx");
    std::fs::write(&p, "[common]\nActionPointsCount = 1\n[actionpoint0\n").unwrap();
    let mut d = MapDoc::load(&w, 55, d85(), &gd).unwrap();
    assert_eq!(d.unreadable.len(), 1);
    assert_eq!(d.unreadable[0].ext, "aptfx");
    assert!(
        d.unreadable[0].error.contains("line 3"),
        "{}",
        d.unreadable[0].error
    );
    assert_eq!(d.unreadable[0].raw, std::fs::read(&p).unwrap());
    let why = d.read_only_reason().unwrap();
    assert!(why.contains("map00055.aptfx"), "{why}");
    d.discard_unreadable();
    d.save(&gd, &w, SaveColumns::Current).unwrap();
    assert!(MapDoc::load(&w, 55, d85(), &gd)
        .unwrap()
        .unreadable
        .is_empty());
    let _ = std::fs::remove_dir_all(&w);
}

/// A quoted unknown key, which the game accepts, must not make the editor's parser fail (and
/// the map open without things): the map loads with all its things and saves in a form the
/// game reads.
#[test]
fn quoted_key_map_keeps_its_things() {
    let Some((install, gd)) = setup() else { return };
    let w = scratch("quoted");
    copy_map(&install.join("multiplayer/classic"), 55, &w);
    let p = w.join("map00055.tngfx");
    let orig = MapDoc::load(&w, 55, d85(), &gd).unwrap();
    let text = std::fs::read_to_string(&p).unwrap().replacen(
        "[thing0]\n",
        "[thing0]\n\"Foo Bar\" = 1\n",
        1,
    );
    std::fs::write(&p, &text).unwrap();
    let d = MapDoc::load(&w, 55, d85(), &gd).unwrap();
    assert_eq!(d.things.items.len(), orig.things.items.len());
    d.save(&gd, &w, SaveColumns::Current).unwrap();
    let saved = std::fs::read_to_string(&p).unwrap();
    assert_eq!(kfx_formats::game_toml::check(&saved), Ok(()));
    assert!(saved.contains("\"Foo Bar\" = 1"));
    let _ = std::fs::remove_dir_all(&w);
}

/// A `.une` the editor cannot read is left untouched by a save (not overwritten with zeros).
#[test]
fn unreadable_une_is_left_untouched() {
    let Some((install, gd)) = setup() else { return };
    let w = scratch("une");
    copy_map(&install.join("multiplayer/classic"), 55, &w);
    let une = w.join("map00055.une");
    let orig = std::fs::read(&une).unwrap();
    let half = orig[..orig.len() / 2].to_vec();
    std::fs::write(&une, &half).unwrap();
    let d = MapDoc::load(&w, 55, d85(), &gd).unwrap();
    d.save(&gd, &w, SaveColumns::Current).unwrap();
    assert_eq!(
        std::fs::read(&une).unwrap(),
        half,
        "the unreadable .une was rewritten"
    );
    // a readable one is kept in step
    std::fs::write(&une, &orig).unwrap();
    let d = MapDoc::load(&w, 55, d85(), &gd).unwrap();
    d.save(&gd, &w, SaveColumns::Current).unwrap();
    assert_eq!(std::fs::read(&une).unwrap(), d.export_une());
    let _ = std::fs::remove_dir_all(&w);
}

fn shot(owner: i64) -> FxThing {
    FxThing {
        class: Some(FxClass::from_id(2)),
        subtype: Some(1),
        owner: Some(owner),
        x: Some(StlCoord::new(40, 128)),
        y: Some(StlCoord::new(40, 128)),
        z: Some(StlCoord::new(1, 0)),
        ..Default::default()
    }
}

/// The game's FX loader fails the whole level on a thing of a class it
/// does not create (a Shot here) or with an unknown `SubtypeStringID`. The validator calls it an
/// error and the save refuses until the thing is fixed or removed.
#[test]
fn things_the_game_rejects_block_the_save() {
    let Some((install, gd)) = setup() else { return };
    let w = scratch("shot");
    copy_map(&install.join("multiplayer/classic"), 55, &w);
    let before = snapshot(&w);
    let mut d = MapDoc::load(&w, 55, d85(), &gd).unwrap();
    let n = d.things.items.len();
    d.things.items.push(shot(0));
    let mut named = shot(0);
    named.class = Some(FxClass::from_id(1));
    named.subtype = None;
    named.subtype_name = Some("NO_SUCH_OBJECT".into());
    d.things.items.push(named);
    let errs: Vec<_> = validate(&d, &gd)
        .into_iter()
        .filter(|i| {
            i.severity == Severity::Error && matches!(i.at, kfx_map::validate::Location::Item(_))
        })
        .map(|i| i.code)
        .collect();
    assert!(errs.contains(&"thing-class-not-loadable"), "{errs:?}");
    assert!(errs.contains(&"thing-subtype-name"), "{errs:?}");
    assert!(
        d.save(&gd, &w, SaveColumns::Current).is_err(),
        "the save must refuse a level the game cannot load"
    );
    assert_eq!(snapshot(&w), before);
    assert_eq!(d.game_rejected_things(&gd).len(), 2);
    assert_eq!(d.drop_game_rejected_things(&gd), 2);
    assert_eq!(d.things.items.len(), n);
    d.save(&gd, &w, SaveColumns::Current).unwrap();
    let _ = std::fs::remove_dir_all(&w);
}

/// Owner 9 is accepted by the FX loader (`owner > PLAYERS_COUNT`,
/// src/thing_factory.c:254), so it is not an error; the legacy loader drops owners above 5
/// (src/thing_factory.c:135), so a legacy save must report them as errors.
#[test]
fn owner_rules_follow_the_loaders() {
    let Some((install, gd)) = setup() else { return };
    let mut d = MapDoc::load(&install.join("multiplayer/classic"), 55, d85(), &gd).unwrap();
    let i = d
        .things
        .items
        .iter()
        .position(|t| t.class.as_ref().and_then(|c| c.id()) == Some(1))
        .unwrap();
    d.things.items[i].owner = Some(9);
    let issues: Vec<_> = validate(&d, &gd)
        .into_iter()
        .filter(|x| x.at == kfx_map::validate::Location::Item(kfx_map::things::ItemRef::Thing(i)))
        .collect();
    assert!(
        issues.iter().all(|x| x.severity != Severity::Error),
        "{issues:?}"
    );
    assert!(d.game_rejected_things(&gd).is_empty());
    for o in [6, 7, 8] {
        d.things.items[i].owner = Some(o);
        let rep = d.legacy_report(&gd);
        assert!(
            rep.errors.iter().any(|e| e.contains("owner above 5")),
            "owner {o}: {:?}",
            rep.errors
        );
    }
}
