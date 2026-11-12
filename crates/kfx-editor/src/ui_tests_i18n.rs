//! Localisation of the editor: the translated calls of the code against the English catalogue,
//! and switching the language in Preferences (live, English for missing keys). Same harness as
//! `ui_tests`.
use crate::ui_tests::Ui;
use egui_kittest::kittest::Queryable;
use kfx_prefab::i18n::{self, scan, Catalog};
use std::path::Path;

/// Some widget has exactly this label (menus and toolbar groups share labels).
fn any(u: &Ui, label: &str) -> bool {
    u.h.query_all_by_label(label).next().is_some()
}

/// Every `trd` / `trf` / `trn` / `tx` key of the editor is in `data/i18n/en.toml` with the same
/// English text and placeholders that match the arguments (run
/// `cargo run -p kfx-prefab --example extract_strings` after changing a text).
#[test]
fn i18n_keys_are_in_the_english_catalogue() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .unwrap();
    let s = scan::scan_dir(
        &crate_dir.join("src"),
        crate_dir.parent().and_then(|p| p.parent()).unwrap(),
    );
    let en = Catalog::parse(i18n::ENGLISH).expect("en.toml parses");
    assert!(s.uses.len() > 1500, "{} translated calls", s.uses.len());
    let problems = scan::check_uses(&s.uses, &en);
    assert!(
        problems.is_empty(),
        "{} problems (regenerate data/i18n/en.toml with the extractor):\n{}",
        problems.len(),
        problems.join("\n")
    );
    // keys built at run time: only the shortcut table's, which the extractor reads from the table
    assert!(
        s.unkeyed
            .iter()
            .all(|u| u.starts_with("crates/kfx-editor/src/shortcuts.rs")),
        "{:?}",
        s.unkeyed
    );
    for d in crate::shortcuts::ACTIONS {
        let k = format!("kfx_editor.action.{}", d.id);
        assert_eq!(
            en.entries.get(&k).map(|e| e.text.as_str()),
            Some(d.label),
            "{k}"
        );
        let g = format!("kfx_editor.action_group.{}", scan::slug(d.group));
        assert_eq!(
            en.entries.get(&g).map(|e| e.text.as_str()),
            Some(d.group),
            "{g}"
        );
    }
}

/// The same check for a translation: placeholders a translation uses must be the English ones.
#[test]
fn i18n_translation_check_finds_bad_placeholders() {
    let en = Catalog::parse(i18n::ENGLISH).unwrap();
    let key = en
        .entries
        .iter()
        .find(|(_, e)| scan::placeholders(&e.text).len() == 1)
        .map(|(k, _)| k.clone())
        .expect("a text with one placeholder");
    let good = Catalog::parse(&format!(
        "[meta]\nlanguage = \"zz\"\n[strings]\n{key:?} = \"x {{{}}}\"\n",
        scan::placeholders(&en.entries[&key].text)
            .into_iter()
            .next()
            .unwrap()
    ))
    .unwrap();
    assert!(scan::check_translation(&en, &good).is_empty());
    let bad = Catalog::parse(&format!(
        "[meta]\nlanguage = \"zz\"\n[strings]\n{key:?} = \"x {{nope}}\"\n"
    ))
    .unwrap();
    assert_eq!(scan::check_translation(&en, &bad).len(), 1);
}

const TEST_CATALOG: &str = r#"# a partial test catalogue (not a real translation)
[meta]
language = "x-test"
name = "Test language"

[strings]
"kfx_editor.file" = "FILE (test)"
"kfx_editor.action.file.open" = "OPEN A MAP (test)"
"kfx_editor.language" = "LANGUAGE (test)"
"kfx_editor.preferences" = "PREFERENCES (test)"
"#;

/// Preferences > Interface > Language switches the whole editor at once; texts the catalogue
/// lacks stay English; back to English restores every text.
#[test]
fn ui_i18n_switch_language() {
    let Some(mut u) = Ui::new("i18n") else { return };
    let dir = kfx_formats::testutil::scratch_dir(&format!(
        "kfx-editor/uitest-i18n-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x-test.toml"), TEST_CATALOG).unwrap();
    assert_eq!(i18n::load_dir(&dir), vec!["x-test".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(i18n::scan::check_translation(
        &Catalog::parse(i18n::ENGLISH).unwrap(),
        &Catalog::parse(TEST_CATALOG).unwrap()
    )
    .is_empty());

    // English: the menu bar and an open menu
    u.click_top("File");
    u.steps(1);
    assert!(u.h.query_all_by_label_contains("Open").next().is_some());
    u.shot("menus-en");
    u.key(eframe::egui::Modifiers::NONE, eframe::egui::Key::Escape);

    // choose the test language in the preferences
    u.menu("Configuration", "Preferences…");
    u.click("Interface");
    u.steps(1);
    assert!(any(&u, "Language"));
    let lab = u.rect_of("Language");
    let combo =
        u.h.get_all_by_role(eframe::egui::Role::ComboBox)
            .map(|n| n.rect())
            .find(|r| (r.center().y - lab.center().y).abs() < 8.0)
            .expect("language box");
    u.click_at(combo.center());
    u.steps(1);
    u.click("Test language");
    u.steps(2);
    assert_eq!(
        u.app().settings.ui.language,
        "x-test",
        "the choice goes into the settings"
    );
    assert_eq!(i18n::language(), "x-test");
    // live: the window title, the label and the menu bar are translated, missing keys are English
    assert!(
        any(&u, "PREFERENCES (test)"),
        "the window title is translated"
    );
    assert!(any(&u, "LANGUAGE (test)"));
    assert!(
        any(&u, "Interface scale"),
        "a key the catalogue lacks shows its English text"
    );
    u.shot("prefs-test-language");
    u.app().open_named_dialog("none").unwrap();
    u.steps(1);
    assert!(any(&u, "FILE (test)"));
    assert!(!any(&u, "File"));
    assert!(any(&u, "Edit"), "untranslated menus stay English");
    // the shortcut table translates at display time (key from the action id)
    assert_eq!(
        crate::shortcuts::ACTIONS
            .iter()
            .find(|d| d.id == "file.open")
            .unwrap()
            .title(),
        "OPEN A MAP (test)"
    );
    assert_eq!(
        crate::shortcuts::ACTIONS
            .iter()
            .find(|d| d.id == "file.save")
            .unwrap()
            .title(),
        "Save map and scripts"
    );
    u.click_top("FILE (test)");
    u.steps(1);
    u.shot("menus-test-language");
    u.key(eframe::egui::Modifiers::NONE, eframe::egui::Key::Escape);

    // back to English
    u.app().settings.ui.language = "en".into();
    u.app().apply_ui_settings();
    u.steps(2);
    assert!(any(&u, "File"));
    assert!(!any(&u, "FILE (test)"));
    assert_eq!(i18n::language(), "en");
    // an unknown language falls back to English
    assert!(!crate::lang::apply("zz-none"));
    assert_eq!(i18n::language(), "en");
    i18n::set_thread_language(None);
}
