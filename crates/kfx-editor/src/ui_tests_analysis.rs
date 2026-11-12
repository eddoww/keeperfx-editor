//! Interaction tests of the analysis and generator wiring: the Analysis panel (heat map
//! layers on and off, symmetry breakers clicked to locate), the New map dialog's generator
//! (regenerate with a new seed, create, save, the level validates) and Map > Generate into
//! selection with its undo. Same harness as `ui_tests`.
use crate::ui_tests::{Tmp, Ui};
use eframe::egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use kfx_formats::testutil;
use kfx_map::validate::Severity;

/// A scratch folder of the test build (`testutil::scratch_dir`), removed when dropped.
fn scratch(tag: &str) -> Tmp {
    let d = kfx_formats::testutil::scratch_dir(&format!(
        "kfx-editor/uitest-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d)
}

/// A generated two-keeper map with an exact half-turn symmetry, opened in the editor.
fn generated_map(u: &mut Ui, dir: &Tmp) {
    let spec = crate::newmap::NewMapSpec {
        generate: true,
        seed: 21,
        players: 2,
        ..Default::default()
    };
    u.app().new_map_spec(&dir.0, 1, &spec).expect("generated");
    u.steps(3);
}

/// Analysis panel: View > Analysis opens it; choosing a layer draws the heat map, "No layer"
/// removes it; after an asymmetric edit the Symmetry tab lists the breaker, and clicking it
/// moves the camera to the slab and selects it.
#[test]
fn ui_analysis_layers_and_breakers() {
    if testutil::install_dir().is_none() {
        return;
    }
    let Some(mut u) = Ui::new("analysis") else {
        return;
    };
    let dir = scratch("analysis");
    generated_map(&mut u, &dir);
    u.menu("View", "Analysis");
    assert!(
        u.app().settings.layout.analysis.open,
        "the menu opens the panel"
    );
    u.app().analysis_wait().unwrap();
    u.steps(2);
    assert_eq!(u.app().analysis_overlay_quads(), 0, "no layer at first");
    u.click("Territory");
    u.steps(2);
    let q = u.app().analysis_overlay_quads();
    assert!(q > 50, "the territory layer is drawn ({q} quads)");
    u.shot("territory");
    u.click("Dig distance");
    u.steps(2);
    assert!(u.app().analysis_overlay_quads() > 50, "dig distance drawn");
    assert!(
        u.has("RED") && u.has("BLUE"),
        "keeper choice of the per-keeper layer"
    );
    u.click("No layer");
    u.steps(2);
    assert_eq!(u.app().analysis_overlay_quads(), 0, "the heat map is gone");

    // break the half-turn symmetry with one rock slab (an edit through the tools)
    u.app().select_palette("HARD").unwrap();
    u.app().set_tool(crate::tools::Tool::Paint);
    u.app().set_brush(1, false);
    let (x, y) = (12, 40);
    u.app().hover_slab((x, y));
    u.app().tool_press((x, y), false);
    u.app().tool_release((x, y));
    assert_eq!(u.history_len(), 1);
    u.app().analysis_wait().unwrap();
    u.click("Symmetry");
    u.steps(2);
    let list = u.app().analysis_breakers();
    assert!(!list.is_empty(), "the edit breaks the symmetry");
    assert!(list.iter().any(|b| b.slab == (x, y)), "{list:?}");
    u.shot("symmetry-tab");
    // move the camera away, then click the first breaker
    u.look_at(70, 70, 0.6);
    let first = list[0].clone();
    u.click_first("1. ");
    u.steps(2);
    let t = u.app().camera.target;
    assert!(
        (t.x - (first.slab.0 as f32 * 3.0 + 1.5)).abs() < 0.01
            && (t.z - (first.slab.1 as f32 * 3.0 + 1.5)).abs() < 0.01,
        "camera at the breaker: {t:?} vs {:?}",
        first.slab
    );
    assert_eq!(
        u.app().selection_rect(),
        Some((first.slab, first.slab)),
        "the breaker is selected"
    );
    u.shot("breaker-located");
    // the Validate panel got no warning from a fair generated map
    assert!(
        u.app()
            .analysis_issue_codes()
            .iter()
            .all(|(_, s)| *s == Severity::Info),
        "{:?}",
        u.app().analysis_issue_codes()
    );
    // Ctrl+Shift+A closes the panel again
    u.key(Modifiers::COMMAND | Modifiers::SHIFT, Key::A);
    assert!(
        !u.app().settings.layout.analysis.open,
        "the shortcut toggles the panel"
    );
}

/// New map dialog with the generator: Generate, Regenerate (new seed, new preview), Create;
/// the saved level has no validation error. Then Map > Generate into selection is one undo step
/// and Ctrl+Z restores the slabs.
#[test]
fn ui_generator_new_map_and_patch() {
    if testutil::install_dir().is_none() {
        return;
    }
    let Some(mut u) = Ui::new("generator") else {
        return;
    };
    let dir = scratch("generator");
    u.menu("File", "New map…");
    assert!(u.app().new_dialog_open());
    u.app().new_dialog_target(&dir.0, 3);
    u.click("Generate");
    u.app().new_dialog_spec().unwrap().players = 2;
    u.steps(2);
    u.app().new_dialog_wait().unwrap();
    u.steps(2);
    let (seed0, pending, r) = u.app().new_dialog_generated().unwrap();
    assert!(!pending);
    assert_eq!(r, Some(Ok(2)), "a two-keeper map is previewed");
    assert!(
        u.h.query_all_by_label_contains("keeper count").count() > 0,
        "the constraints report is shown"
    );
    u.shot("dialog");
    u.click("🎲 Regenerate");
    u.app().new_dialog_wait().unwrap();
    u.steps(2);
    let (seed1, _, r1) = u.app().new_dialog_generated().unwrap();
    assert_ne!(seed0, seed1, "Regenerate picks a new seed");
    assert_eq!(r1, Some(Ok(2)));
    u.shot("regenerated");
    u.click("Create");
    u.steps(2);
    assert!(!u.app().new_dialog_open(), "Create closes the dialog");
    let d = u.app().doc.as_ref().expect("a new map");
    assert_eq!((d.doc.level, d.doc.dims.w), (3, 85));
    let gd = d.gd.clone();
    u.app().save().expect("saved");
    let errors: Vec<_> = kfx_map::validate::validate(&u.app().doc.as_ref().unwrap().doc, &gd)
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(dir.join("map00003.slb").exists() && dir.join("map00003.tngfx").exists());

    // generate into a selection, one undo step
    let before = u.app().doc.as_ref().unwrap().doc.kinds.clone();
    let h0 = u.history_len();
    u.app().select_rect((20, 20), (45, 44));
    u.menu("Map", "Generate into selection…");
    assert!(u.app().gen_patch_open());
    u.shot("patch-dialog");
    u.click("Generate");
    u.steps(2);
    assert_eq!(u.history_len(), h0 + 1, "one undo step");
    let after = u.app().doc.as_ref().unwrap().doc.kinds.clone();
    assert_ne!(after, before, "the rectangle changed");
    u.shot("patched");
    // close the window, then Ctrl+Z on the map
    u.app().close_gen_patch();
    u.steps(1);
    u.key(Modifiers::COMMAND, Key::Z);
    assert_eq!(
        u.app().doc.as_ref().unwrap().doc.kinds,
        before,
        "Ctrl+Z restores the slabs"
    );
}
