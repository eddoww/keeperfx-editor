//! Data safety through the real UI: a map with a file the editor
//! cannot read opens behind a blocking dialog and stays read-only until the user chooses to
//! discard the file (after a backup); saving the configuration overrides is not mistaken for a
//! change by another program; string tables are backed up before their first overwrite.
use crate::ui_tests::{level_copy, Ui};
use kfx_formats::testutil;
use std::path::Path;

fn backups_holding(name_part: &str) -> Vec<std::path::PathBuf> {
    let Some(root) = crate::settings::Settings::backup_dir() else {
        return vec![];
    };
    let mut out = vec![];
    for d in std::fs::read_dir(root).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
            if f.file_name().to_string_lossy().contains(name_part) {
                out.push(f.path());
            }
        }
    }
    out
}

/// A `.tngfx` that cannot be parsed opens with a dialog naming the file
/// and the error (with line and column); "Open read-only" keeps the file safe from Save; "Open
/// and discard" makes a backup first and then allows the save.
#[test]
fn ui_unreadable_file_opens_read_only() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("unreadable") else {
        return;
    };
    let dir = level_copy(&inst, 1, "unreadable");
    let slb = dir.join("map00001.slb");
    // an FX copy of the level, then a broken .tngfx
    u.open(&slb);
    u.app().set_save_format(false);
    u.app().save().unwrap();
    let tngfx = dir.join("map00001.tngfx");
    let text = std::fs::read_to_string(&tngfx).unwrap().replacen(
        "[thing0]\n",
        "[thing0]\nThingType = = 1\n",
        1,
    );
    std::fs::write(&tngfx, &text).unwrap();
    u.open(&slb);
    assert!(
        u.has("Some files of this map could not be read"),
        "the dialog is shown"
    );
    assert!(u.has("Open read-only"));
    u.shot("unreadable-dialog");
    u.click("Open read-only");
    u.steps(2);
    assert!(!u.has("Some files of this map could not be read"));
    assert!(u.app().save().is_err(), "a read-only map is not saved");
    assert_eq!(
        std::fs::read_to_string(&tngfx).unwrap(),
        text,
        "the unreadable file is untouched"
    );
    // reopen and discard: a backup of the file exists before anything is written
    u.open(&slb);
    u.click("Open and discard the unreadable things (a backup is kept)");
    u.steps(2);
    let copies: Vec<_> = backups_holding("map00001.tngfx")
        .into_iter()
        .filter(|p| std::fs::read_to_string(p).ok().as_deref() == Some(text.as_str()))
        .collect();
    assert!(!copies.is_empty(), "the unreadable file is in a backup");
    u.app().save().unwrap();
    assert_ne!(std::fs::read_to_string(&tngfx).unwrap(), text);
}

/// Saving a configuration override (`mapNNNNN.rules.cfg`) refreshes the external-change
/// fingerprint, so the next focus change neither asks about nor silently reloads over the
/// editor's own write.
#[test]
fn ui_config_save_is_not_an_external_change() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("cfgfp") else {
        return;
    };
    let dir = level_copy(&inst, 1, "cfgfp");
    u.open(&dir.join("map00001.slb"));
    u.app().set_save_format(false);
    u.app().save().unwrap();
    // unsaved map edits, then a config override saved
    {
        let d = u.app().doc.as_mut().unwrap();
        d.edit("test", |m, gd, _| m.set_slab(gd, 5, 5, 10).unwrap());
    }
    u.app().open_config_editor(None);
    u.steps(2);
    {
        let s = &mut u.app().config.as_mut().unwrap().editor.session;
        assert!(s.set_cfg("rules.cfg", "game", "GoldPerHoard", "1234"));
        s.save().unwrap();
    }
    u.steps(3);
    assert!(dir.join("map00001.rules.cfg").exists());
    u.app().check_external_changes();
    u.steps(2);
    assert!(
        !u.has("Map changed on disk"),
        "the editor's own config write is not an external change"
    );
    assert!(
        u.app().doc.as_ref().unwrap().is_dirty(),
        "the unsaved edit is still there"
    );
    // a real external change is still noticed
    std::fs::write(
        dir.join("map00001.rules.cfg"),
        "[game]\nGoldPerHoard = 99\n",
    )
    .unwrap();
    u.app().check_external_changes();
    u.steps(2);
    assert!(u.has("Map changed on disk"));
}

/// A string table of the campaign or the level is backed up before its
/// first overwrite in a session.
#[test]
fn ui_string_tables_are_backed_up() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("strbk") else {
        return;
    };
    let dir = level_copy(&inst, 1, "strbk");
    let table = dir.join("map00001.eng.dat");
    let original = b"\x00old level text\x00".to_vec();
    std::fs::write(&table, &original).unwrap();
    u.open(&dir.join("map00001.slb"));
    let id = u.app().new_level_string("a new message").unwrap();
    assert!(id > 0);
    u.app().save_strings().unwrap();
    assert_ne!(std::fs::read(&table).unwrap(), original);
    let copies: Vec<_> = backups_holding("map00001.eng.dat")
        .into_iter()
        .filter(|p| std::fs::read(p).ok().as_deref() == Some(original.as_slice()))
        .collect();
    assert!(!copies.is_empty(), "the old table is in a backup");
    let _ = Path::new("");
}

/// The crash-recovery snapshot is kept until a save succeeds: a save that fails (here: a
/// read-only map file) leaves the only copy of the unsaved changes in place.
#[test]
fn ui_snapshot_survives_a_failed_save() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::persistent("snapkeep") else {
        return;
    };
    let dir = level_copy(&inst, 1, "snapkeep");
    let slb = dir.join("map00001.slb");
    u.open(&slb);
    u.app().set_save_format(false);
    u.app().save().unwrap();
    {
        let d = u.app().doc.as_mut().unwrap();
        d.edit("test", |m, gd, _| m.set_slab(gd, 6, 6, 10).unwrap());
    }
    let snap = u.app().write_snapshot().unwrap();
    assert!(snap.join("snapshot.json").exists());
    let victim = dir.join("map00001.tngfx");
    let mut perm = std::fs::metadata(&victim).unwrap().permissions();
    perm.set_readonly(true);
    std::fs::set_permissions(&victim, perm.clone()).unwrap();
    let before = std::fs::read(&slb).unwrap();
    let r = u.app().save();
    perm.set_readonly(false);
    std::fs::set_permissions(&victim, perm).unwrap();
    assert!(
        r.is_err() || u.app().status.contains("Not saved"),
        "the save failed: {:?} / {}",
        r,
        u.app().status
    );
    assert_eq!(
        std::fs::read(&slb).unwrap(),
        before,
        "nothing of the map was written"
    );
    assert!(
        snap.join("snapshot.json").exists(),
        "the snapshot survives a failed save"
    );
    u.app().save().unwrap();
    assert!(!snap.exists(), "and goes after a successful one");
}
