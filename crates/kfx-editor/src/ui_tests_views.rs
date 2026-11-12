//! UI interaction tests of views and windows through the real `EditorApp` (like
//! `crate::ui_tests`): the grid data overlay, the fly view, the minimap in the 2D view, the docked
//! configuration editor's width, the first run without an install, and window positions kept
//! across restarts.
use crate::ui_tests::{level_copy, Ui};
use crate::ui_tests_editing::setup;
use eframe::egui::{self, Key, Modifiers};
use kfx_formats::testutil;

/// View > Grid data writes the chosen grid over the map.
#[test]
fn ui_grid_data() {
    let Some((mut u, _dir)) = setup("grid") else {
        return;
    };
    u.look_at(40, 40, 2.5);
    u.click_top("View");
    u.steps(1);
    u.click_first("Grid data");
    u.steps(1);
    u.click_first("Slab kind ID (.slb)");
    u.steps(2);
    assert_eq!(
        u.app().settings.layout.grid_data,
        crate::griddata::GridData::Kind
    );
    u.shot("kind");
}

/// V enters the fly view, W moves forward, M captures the mouse, Esc frees it, V leaves.
#[test]
fn ui_fly_view() {
    let Some((mut u, _dir)) = setup("fly") else {
        return;
    };
    u.look_at(42, 42, 1.0);
    let p = u.slab_px(42, 42);
    u.move_to(p);
    u.key(Modifiers::NONE, Key::V);
    assert!(u.app().camera.fly, "V enters the fly view");
    u.shot("entered");
    let start = u.app().camera.target;
    u.app().settings.camera.fly_collision = false;
    u.h.event(egui::Event::Key {
        key: Key::W,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    u.steps(10);
    u.h.event(egui::Event::Key {
        key: Key::W,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    u.steps(1);
    let moved = u.app().camera.target - start;
    assert!(
        moved.length() > 0.05 && moved.y.abs() < 1e-3,
        "W moves horizontally: {moved}"
    );
    u.key(Modifiers::NONE, Key::M);
    assert!(u.app().fly_capture, "M captures the mouse");
    u.key(Modifiers::NONE, Key::Escape);
    assert!(!u.app().fly_capture, "Esc frees it");
    u.key(Modifiers::NONE, Key::V);
    assert!(!u.app().camera.fly, "V leaves");
}

/// In the 2D view the minimap moves the 2D camera (and draws its outline, not the 3D one's).
#[test]
fn ui_minimap_follows_2d() {
    let Some((mut u, _dir)) = setup("mini") else {
        return;
    };
    u.key(Modifiers::NONE, Key::F2);
    assert_eq!(
        u.app().settings.layout.view_mode,
        crate::settings::ViewMode::TwoD
    );
    u.steps(2);
    let label = u.rect_of("Minimap");
    let cam3 = u.app().camera.target;
    let before = u.app().camera2.target;
    u.click_at(label.left_bottom() + egui::vec2(30.0, 30.0));
    u.steps(1);
    let after = u.app().camera2.target;
    assert!(
        (after - before).length() > 1.0,
        "the 2D camera moved: {before} -> {after}"
    );
    assert_eq!(u.app().camera.target, cam3, "the hidden 3D camera stays");
    u.shot("2d");
}

/// The docked configuration editor leaves the map at least MIN_MAP_W wide.
#[test]
fn ui_docked_config_keeps_the_map() {
    let Some((mut u, _dir)) = setup("dock") else {
        return;
    };
    u.app().settings.layout.config_dock_w = 5000.0;
    u.menu("Configuration", "Rules…");
    u.app().config.as_mut().unwrap().docked = true;
    u.steps(3);
    let w = u
        .app()
        .pane_rects()
        .first()
        .map(|r| r.width())
        .unwrap_or(0.0);
    assert!(w >= crate::app::MIN_MAP_W - 1.0, "map width {w}");
    u.shot("docked");
}

/// First run at 1280x720: no install configured (the dialog explains what to choose and checks
/// the folder), then the install chosen (the empty view says how to open a map), then a map open.
#[test]
fn ui_first_run() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    if std::env::var_os("KFX_INSTALL").is_some() {
        // the variable overrides the settings: the first-run dialog never shows
        return;
    }
    let mut u = Ui::with_settings(
        "firstrun",
        crate::settings::Settings::default(),
        egui::vec2(1280.0, 720.0),
    );
    assert!(u.app().install().is_none());
    assert!(u.has("Use this folder"), "the first-run dialog is shown");
    u.shot("no-install");
    u.app().first_run_text(&inst.display().to_string());
    u.steps(2);
    u.shot("folder-typed");
    u.click("Use this folder");
    u.steps(3);
    assert!(u.app().install().is_some());
    u.shot("install-chosen");
    let dir = level_copy(&inst, 1, "firstrun");
    u.open(&dir.join("map00001.slb"));
    u.shot("map-open");
}

/// Window positions survive a restart: the egui memory (areas of the open windows) round trips
/// through the JSON kept in the settings (`remember_ui_memory` / `startup_extras`).
#[test]
fn ui_window_positions_round_trip() {
    let Some((mut u, _dir)) = setup("memory") else {
        return;
    };
    u.menu("Configuration", "Preferences…");
    u.steps(3);
    let text =
        u.h.ctx
            .memory(|m| ron::to_string(m))
            .expect("memory serialises");
    assert!(text.len() > 100, "{text}");
    let back: egui::Memory = ron::from_str(&text).expect("memory deserialises");
    let again = ron::to_string(&back).unwrap();
    assert_eq!(text.len(), again.len(), "stable round trip");
    // the settings file holds it as a string
    let s = crate::settings::Settings {
        ui_memory: text.clone(),
        ..Default::default()
    };
    let j = serde_json::to_string(&s).unwrap();
    let s2: crate::settings::Settings = serde_json::from_str(&j).unwrap();
    assert_eq!(s2.ui_memory, text);
}
