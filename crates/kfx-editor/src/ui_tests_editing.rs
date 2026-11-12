//! UI interaction tests of map editing helpers through the real `EditorApp` (like
//! `crate::ui_tests`): random doodads, the creature stats window, the script wizard, door locks,
//! the save format kept in the sidecar, palette options, fortify and toasts, and backups that can
//! only be saved elsewhere.
use crate::ui_tests::{level_copy, Ui};
use eframe::egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use kfx_formats::testutil;
use kfx_map::autotile::kinds;

/// The editor with a private copy of level 1 open.
pub(crate) fn setup(name: &str) -> Option<(Ui, crate::ui_tests::Tmp)> {
    let inst = testutil::install_dir()?;
    let mut u = Ui::new(name)?;
    let dir = level_copy(&inst, 1, name);
    u.open(&dir.join("map00001.slb"));
    Some((u, dir))
}

/// Map > Random doodads: rules applied as one undo step, saved in the sidecar next to
/// kfx-map's tables, scattered again when their slab is painted.
#[test]
fn ui_doodads() {
    let Some((mut u, dir)) = setup("doodads") else {
        return;
    };
    u.menu("Map", "Random doodads…");
    let set = u.app().doodad_dialog_set().expect("the dialog is open");
    set.seed = Some(3);
    set.rules = vec![crate::doodads::DoodadRule {
        slab: "DIRT".into(),
        model: "BARREL".into(),
        chance: 5.0,
        ..Default::default()
    }];
    u.steps(2);
    u.shot("dialog");
    let before = u.app().doc.as_ref().unwrap().doc.things.items.len();
    u.click("Apply to the whole map");
    let after = u.app().doc.as_ref().unwrap().doc.things.items.len();
    assert!(after > before + 50, "{before} -> {after}");
    assert_eq!(u.history_len(), 1);
    u.key(Modifiers::COMMAND, Key::Z);
    assert_eq!(
        u.app().doc.as_ref().unwrap().doc.things.items.len(),
        before,
        "one undo step"
    );
    u.key(Modifiers::COMMAND, Key::Y);
    u.app().save().unwrap();
    let text = std::fs::read_to_string(dir.join("map00001.kfxed.toml")).expect("sidecar written");
    assert!(
        text.contains("doodads.rule") && text.contains("BARREL"),
        "{text}"
    );
    assert_eq!(
        crate::doodads::Doodads::load(&dir, 1).unwrap().rules.len(),
        1
    );
    u.shot("applied");
}

/// Configuration > Creature stats… opens the comparison window.
#[test]
fn ui_creature_stats() {
    let Some((mut u, _dir)) = setup("stats") else {
        return;
    };
    u.menu("Configuration", "Creature stats…");
    u.steps(2);
    assert!(u.app().config.as_ref().is_some_and(|c| c.editor.stats_open));
    assert!(u.has("Creature stats"), "the window is shown");
    u.shot("stats");
}

/// Level > Script wizard…: set the start gold and Apply; the block lands in the DK script.
#[test]
fn ui_script_wizard() {
    let Some((mut u, _dir)) = setup("wizard") else {
        return;
    };
    u.menu("Level", "Script wizard…");
    u.steps(2);
    assert!(u.app().wizard_open());
    {
        let ws = u.app().script_ws().unwrap();
        let w = ws.wizard_mut().unwrap();
        w.model_mut()
            .player_mut(kfx_script_ui::wizard::Player::P0)
            .start_money = Some(12345);
    }
    u.steps(2);
    u.shot("form");
    assert_eq!(
        u.h.query_all_by_label("Apply").count(),
        1,
        "one Apply button (the wizard's)"
    );
    u.click_first("Apply");
    u.steps(2);
    let text = u.app().script_text("map00001.txt").expect("dk script");
    // the script already adds 2500 (line 34): the block adds the difference to reach 12345
    assert!(
        text.contains("kfx-editor wizard begin") && text.contains("START_MONEY(PLAYER0, 9845)"),
        "{text}"
    );
    u.shot("applied");
}

/// Space toggles the lock of the door under the pointer.
#[test]
fn ui_door_lock() {
    let Some((mut u, _dir)) = setup("door") else {
        return;
    };
    // a door slab with walls on two sides: find one on the shipped map, else skip
    let Some((x, y)) = ({
        let d = &u.app().doc.as_ref().unwrap().doc;
        d.things
            .items
            .iter()
            .find(|t| kfx_map::things::thing_class(t) == Some(kfx_map::things::class::DOOR))
            .and_then(|t| kfx_map::things::position(t.x, t.y))
            .map(|(a, b)| (a / 3, b / 3))
    }) else {
        return;
    };
    u.look_at(x, y, 2.0);
    let p = u.slab_px(x, y);
    u.move_to(p);
    let locked = |u: &mut Ui| {
        u.app()
            .doc
            .as_ref()
            .unwrap()
            .doc
            .door_on_slab(x, y)
            .map(|i| {
                u.app().doc.as_ref().unwrap().doc.things.items[i]
                    .door_locked
                    .unwrap_or(0)
            })
    };
    let before = locked(&mut u);
    u.key(Modifiers::NONE, Key::Space);
    assert_ne!(locked(&mut u), before, "Space toggled the lock");
    u.key(Modifiers::NONE, Key::Space);
    assert_eq!(locked(&mut u), before);
}

/// File > Save format is stored in the map's sidecar.
#[test]
fn ui_save_format_in_sidecar() {
    let Some((mut u, dir)) = setup("format") else {
        return;
    };
    u.app().set_save_format(true);
    u.steps(1);
    // the legacy report prompt may show; close it
    if u.has("OK") {
        u.click_first("OK");
    }
    u.app().save().unwrap();
    let text = std::fs::read_to_string(dir.join("map00001.kfxed.toml")).expect("sidecar");
    assert!(text.contains("save_format = \"legacy\""), "{text}");
    u.app().open_map(&dir.join("map00001.slb")).unwrap();
    assert!(u.app().save_format_legacy());
    assert!(dir.join("map00001.tng").exists());
}

/// Palette options: ids on the tiles, one list by id.
#[test]
fn ui_palette_options() {
    let Some((mut u, _dir)) = setup("palette") else {
        return;
    };
    u.click("IDs");
    u.click("By ID");
    u.steps(1);
    assert!(u.app().settings.layout.palette_ids && u.app().settings.layout.palette_by_id);
    assert!(u.has("All, by slab ID"));
    u.shot("ids");
}

/// Fortify: claimed ground of a keeper walls the earth around it; a toast reports changes.
#[test]
fn ui_fortify() {
    let Some((mut u, _dir)) = setup("fortify") else {
        return;
    };
    u.app().fortify = true;
    // an all-earth 3x3 spot
    let (x, y) = {
        let d = &u.app().doc.as_ref().unwrap().doc;
        let dm = d.dims;
        let earth =
            |x: i32, y: i32| d.kinds[dm.slab_index(x as usize, y as usize)] == kinds::EARTH as u16;
        (3..dm.h as i32 - 3)
            .flat_map(|y| (3..dm.w as i32 - 3).map(move |x| (x, y)))
            .find(|&(x, y)| (-1..=1).all(|dy| (-1..=1).all(|dx| earth(x + dx, y + dy))))
            .unwrap()
    };
    u.app().select_palette("PRETTY_PATH").unwrap();
    u.app().set_active_owner(0);
    u.app().set_tool(crate::tools::Tool::Paint);
    u.app().set_brush(1, false);
    u.app().hover_slab((x, y));
    u.app().tool_press((x, y), false);
    u.app().tool_release((x, y));
    let gd = u.app().doc.as_ref().unwrap().gd.clone();
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1)] {
        let k = u.kind_at(x + dx, y + dy);
        assert_eq!(
            gd.slab(k as usize).category,
            kfx_map::gamedata::category::FORTIFIED_WALL,
            "({dx},{dy}) is a wall"
        );
    }
    u.app().status = "a test message".into();
    u.steps(1);
    assert!(
        u.app().toast_texts().iter().any(|t| t == "a test message"),
        "status messages show as toasts"
    );
    u.look_at(x, y, 2.0);
    u.shot("fortified");
}

/// A backup opened from the Backups window can only be saved elsewhere.
#[test]
fn ui_backup_is_save_as_only() {
    let Some((mut u, dir)) = setup("backup") else {
        return;
    };
    let bk = dir.join("bk");
    std::fs::create_dir_all(&bk).unwrap();
    for f in crate::document::level_files(&dir, 1) {
        std::fs::copy(&f, bk.join(f.file_name().unwrap())).unwrap();
    }
    u.app().open_backup(&bk, 1).unwrap();
    assert!(u.app().from_backup);
    assert!(u.app().save().is_err(), "Save is refused");
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    u.app().save_as(&out, 7).unwrap();
    assert!(!u.app().from_backup);
    assert!(out.join("map00007.slb").exists());
}
