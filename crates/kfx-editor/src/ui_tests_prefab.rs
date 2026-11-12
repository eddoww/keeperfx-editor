//! Prefabs and translations through the real UI (same harness as `ui_tests`): pick a
//! prefab in the Prefabs panel, place it with the paste ghost (R turns it), undo restores the
//! map and takes its script fragment out of the level script again, redo puts both back; save a
//! selection as a prefab; open the translation grid from the Strings window and edit a cell.
use crate::ui_tests::{level_copy, Ui};
use eframe::egui::{self, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use kfx_formats::testutil;

const FRAGMENT: &str = "REM --- prefab: Ambush corridor ---";

#[test]
fn ui_prefab_place_and_undo() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("prefab") else {
        return;
    };
    let dir = level_copy(&inst, 1, "prefab");
    u.open(&dir.join("map00001.slb"));
    // a clipboard of the user's own, given back when the placing ends
    u.app().select_rect((40, 40), (41, 41));
    assert!(u.app().copy());
    let users_clip = u.app().clipboard_for_test().cloned();
    u.app().cancel_tool();

    u.menu("Window", "Prefabs");
    assert!(
        u.app().settings.prefabs.open,
        "Window > Prefabs opens the panel"
    );
    u.steps(6);
    assert!(
        u.app().prefab_thumb_count() >= 20,
        "thumbnails from the terrain renderer: {}",
        u.app().prefab_thumb_count()
    );
    u.shot("browser");

    // select the card: click its thumbnail (above the name)
    let card =
        u.h.get_all_by_label("Ambush corridor")
            .map(|n| n.rect())
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap();
    u.click_at(card.center());
    assert_eq!(
        u.app().prefab_selected_name().as_deref(),
        Some("Ambush corridor")
    );
    u.steps(1);
    u.click("Place on map");
    assert!(
        u.app().prefab_placing().is_some(),
        "the ghost follows the pointer"
    );
    assert!(u.app().pasting_for_test());

    // look at an earth area and turn the ghost with R (the paste shortcut)
    u.look_at(20, 45, 0.18);
    let at = u.slab_px(20, 45);
    u.move_to(at);
    u.key(Modifiers::NONE, Key::R);
    assert_eq!(
        u.app().prefab_placing_orientation(),
        Some((90, false)),
        "R turns the prefab"
    );
    let clip = u.app().clipboard_for_test().cloned().unwrap();
    assert_eq!((clip.w, clip.h), (7, 19), "the ghost is the turned prefab");
    u.move_to(at);
    u.shot("ghost");

    let kinds_before = u.app().doc.as_ref().unwrap().doc.kinds.clone();
    let script_before = u.app().script_text("map00001.txt").unwrap();
    let hist = u.history_len();
    u.click_at(at);
    assert_eq!(u.history_len(), hist + 1, "one undo step");
    let kinds_after = u.app().doc.as_ref().unwrap().doc.kinds.clone();
    let changed = kinds_before
        .iter()
        .zip(&kinds_after)
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 40, "{changed} slabs changed");
    // the turned corridor runs north-south through the click
    let x0 = 20 - 3;
    assert!(
        (36..=54).any(|y| u.kind_at(x0 + 3, y) != u.kind_at(x0 + 3, 30)),
        "slabs placed along the turned corridor"
    );
    let script = u.app().script_text("map00001.txt").unwrap();
    assert!(
        script.contains(FRAGMENT),
        "the fragment is appended to the DK script"
    );
    assert!(script.contains("IF_ACTION_POINT("), "with its trigger");
    assert!(!script.contains("{{"), "every placeholder is filled in");
    u.shot("placed");

    // Escape ends the placing and gives the user's clipboard back
    u.key(Modifiers::NONE, Key::Escape);
    assert!(!u.app().pasting_for_test());
    u.steps(1);
    assert_eq!(
        u.app().clipboard_for_test().cloned(),
        users_clip,
        "the user's clipboard is back"
    );

    // undo: map and script as before, no renumbering question
    u.key(Modifiers::COMMAND, Key::Z);
    u.steps(1);
    assert_eq!(
        u.app().doc.as_ref().unwrap().doc.kinds,
        kinds_before,
        "undo restores the map"
    );
    assert_eq!(
        u.app().script_text("map00001.txt").unwrap(),
        script_before,
        "undo takes the fragment out again"
    );
    assert!(!u.has("Script references"), "no renumbering prompt");
    // redo puts both back
    u.key(Modifiers::COMMAND, Key::Y);
    u.steps(1);
    assert_eq!(u.app().doc.as_ref().unwrap().doc.kinds, kinds_after);
    assert!(u
        .app()
        .script_text("map00001.txt")
        .unwrap()
        .contains(FRAGMENT));
    u.key(Modifiers::COMMAND, Key::Z);
    u.steps(1);
    assert_eq!(u.app().script_text("map00001.txt").unwrap(), script_before);
    u.shot("undone");
}

#[test]
fn ui_prefab_save_selection() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("prefab-save") else {
        return;
    };
    let dir = level_copy(&inst, 1, "prefab-save");
    u.open(&dir.join("map00001.slb"));
    u.app().open_prefabs(true);
    u.steps(2);
    // the heart area of map 1
    u.app().select_rect((38, 38), (46, 46));
    u.menu("Edit", "Save selection as prefab…");
    assert!(u.app().prefab_save_dialog_open());
    let name =
        u.h.get_all_by_role(egui::Role::TextInput)
            .map(|n| n.rect())
            .find(|r| (r.center().y - u.rect_of("Name").center().y).abs() < 8.0)
            .expect("name field");
    u.click_at(name.center());
    u.text("Test heart room");
    u.steps(1);
    u.shot("save-dialog");
    u.click("Save prefab");
    u.steps(2);
    assert!(
        !u.app().prefab_save_dialog_open(),
        "the dialog closes: {}",
        u.app().status
    );
    assert_eq!(
        u.app().prefab_selected_name().as_deref(),
        Some("Test heart room")
    );
    let user = crate::prefabs::user_folder().unwrap();
    let file = user.join("test-heart-room.kfxprefab");
    assert!(file.is_file(), "{} written", file.display());
    let p = kfx_prefab::Prefab::load(&file).unwrap();
    assert_eq!((p.w, p.h), (9, 9));
    assert!(p.thumbnail.is_some(), "with a thumbnail");
    u.shot("saved");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn ui_translation_grid() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("texts") else {
        return;
    };
    let dir = level_copy(&inst, 1, "texts");
    u.open(&dir.join("map00001.slb"));
    u.menu("Configuration", "Strings…");
    u.steps(1);
    u.click("Translations…");
    u.steps(2);
    assert!(
        u.app().texts_open_for_test(),
        "the grid opens: {}",
        u.app().status
    );
    assert!(!u.app().strings_dirty());
    // a French column
    u.click("Add");
    u.steps(2);
    let cell =
        u.h.get_all_by_role(egui::Role::MultilineTextInput)
            .find(|n| {
                n.value()
                    .is_some_and(|v| v.starts_with("Ceci est un message"))
            })
            .map(|n| n.rect())
            .expect("the French cell of string 1");
    u.click_at(cell.center());
    u.key(Modifiers::COMMAND, Key::A);
    u.text("Ceci est un nouveau message.");
    u.steps(1);
    // leaving the cell sets the text
    let away =
        u.h.query_all_by_label_contains("string IDs")
            .next()
            .unwrap()
            .rect();
    u.click_at(away.center());
    u.steps(2);
    assert!(
        u.app().strings_dirty(),
        "the grid's change counts as unsaved strings"
    );
    assert!(u.has("Translations *"), "the window title marks it");
    let g = u.app().texts_grid().unwrap();
    let row = g.project.rows.iter().position(|r| r.id == 1).unwrap();
    assert_eq!(
        g.project.cell(row, kfx_formats::strings::Lang::French).text,
        "Ceci est un nouveau message."
    );
    u.shot("edited");
    // saved through the editor's string save (with the map's Save)
    let written = u.app().save_strings().unwrap();
    assert!(
        written.iter().any(|p| p.ends_with("map00001.fre.dat")),
        "{written:?}"
    );
    assert!(!u.app().strings_dirty());
    u.steps(2);
    assert!(!u.has("Translations *"));
    assert!(dir.join("map00001.fre.dat").is_file());
}
