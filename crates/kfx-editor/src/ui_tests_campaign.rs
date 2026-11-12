//! Campaign > Campaign editor through the real editor UI: the window opens from the menu with
//! the last campaign (a private copy of a shipped campaign in a
//! scratch folder of the test build), an ensign is dragged on the land view, the campaign is
//! saved (into the copy; the shipped install is checked to be untouched); the "New campaign /
//! map pack" wizard makes a pack in a private install.
use crate::ui_tests::Ui;
use crate::ui_tests_workshop::Scratch;
use eframe::egui::{self, Modifiers};
use egui_kittest::kittest::Queryable;
use std::path::Path;

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_tree(&p, &dst.join(e.file_name()));
        } else {
            std::fs::copy(&p, dst.join(e.file_name())).unwrap();
        }
    }
}

/// Copy `campgns/<name>.cfg` with its levels and land view folders into `root/campgns/`.
fn campaign_copy(real: &Path, root: &Path, name: &str) -> std::path::PathBuf {
    let src = kfx_formats::testutil::resolve_ci(real, "campgns").unwrap();
    let dst = root.join("campgns");
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::copy(
        src.join(format!("{name}.cfg")),
        dst.join(format!("{name}.cfg")),
    )
    .unwrap();
    for d in [name.to_string(), format!("{name}_lnd")] {
        copy_tree(&src.join(&d), &dst.join(&d));
    }
    dst.join(format!("{name}.cfg"))
}

fn positions(u: &mut Ui) -> Vec<(u32, Option<(i32, i32)>)> {
    let ed = u.app().campaign_editor().unwrap();
    ed.pack
        .entries()
        .iter()
        .map(|e| (e.number, e.info.ensign_pos))
        .collect()
}

#[test]
fn ui_campaign_drag_ensign_and_save() {
    let Some(real) = kfx_formats::testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("campaign") else {
        return;
    };
    let s = Scratch::new("camp");
    let shipped = std::fs::read(real.join("campgns/burdnimp.cfg")).unwrap();
    let cfg = campaign_copy(&real, &s.0, "burdnimp");
    // the window reopens the last campaign
    u.app().settings.layout.campaign.campaign_cfg = Some(cfg.clone());
    u.menu("Campaign", "Campaign editor…");
    u.steps(2);
    assert!(u.app().settings.layout.campaign.campaign_open);
    let ed = u
        .app()
        .campaign_editor()
        .expect("the campaign opens with the window");
    assert_eq!(ed.pack.cfg_path, cfg);
    assert!(!ed.pack.is_dirty());
    u.shot("opened");

    u.click_first("Land view");
    u.steps(2);
    let before = positions(&mut u);
    let ed = u.app().campaign_editor().unwrap();
    let canvas = ed.last_canvas.expect("the land view is drawn");
    // the first single level's ensign: press on its flag pole, drag right and down
    let first = ed.pack.single_slots()[0].level;
    let pos = ed
        .pack
        .level_info(first)
        .ensign_pos
        .expect("burdnimp places its ensigns");
    let p = kfx_campaign_ui::ui::to_screen(canvas, pos);
    let grab = p - egui::vec2(0.0, 12.0);
    let delta = egui::vec2(48.0, 30.0);
    u.drag(grab, grab + delta, 8, Modifiers::NONE);
    u.steps(2);
    let after = positions(&mut u);
    let moved: Vec<_> = before
        .iter()
        .zip(&after)
        .filter(|(a, b)| a.1 != b.1)
        .map(|(a, b)| (a.0, a.1, b.1))
        .collect();
    assert_eq!(moved.len(), 1, "one ensign moved: {moved:?}");
    let (n, Some(old), Some(new)) = moved[0] else {
        panic!("{moved:?}")
    };
    let scale = 1280.0 / canvas.width();
    let want = (
        old.0 + (delta.x * scale).round() as i32,
        old.1 + (delta.y * scale).round() as i32,
    );
    assert!(
        (new.0 - want.0).abs() <= 3 && (new.1 - want.1).abs() <= 3,
        "level {n}: {old:?} -> {new:?}, expected about {want:?}"
    );
    let ed = u.app().campaign_editor().unwrap();
    assert!(ed.pack.is_dirty(), "the pack is modified");
    assert_eq!(ed.selected, Some(n), "the dragged level is selected");
    u.shot("dragged");

    // save through the widget's button
    u.click("Save *");
    u.steps(2);
    assert!(!u.app().campaign_dirty());
    let text = kfx_formats::cfg::latin1_decode(&std::fs::read(&cfg).unwrap());
    assert!(
        text.contains(&format!("{} {}", new.0, new.1)),
        "ENSIGN_POS of map{n:05} written to the copy"
    );
    assert_eq!(
        std::fs::read(real.join("campgns/burdnimp.cfg")).unwrap(),
        shipped,
        "the shipped campaign is untouched"
    );
    u.shot("saved");

    // closing and reopening the window keeps the pack
    u.app().show_campaign_window(false);
    u.steps(1);
    u.app().show_campaign_window(true);
    u.steps(2);
    assert!(u.app().campaign_editor().is_some());
}

#[test]
fn ui_campaign_wizard() {
    let Some(_) = kfx_formats::testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("campaign-wizard") else {
        return;
    };
    let s = Scratch::new("wiz");
    let root = s.0.join("install");
    std::fs::create_dir_all(root.join("campgns")).unwrap();
    u.menu("Campaign", "New campaign / map pack…");
    assert!(u.has("Create"), "the wizard opens");
    {
        let w = u.app().campaign_wizard().unwrap();
        assert!(
            w.land_from.is_some(),
            "a shipped campaign with a land view is offered"
        );
        // made in the private install (which has no campaign to copy a land view from)
        w.install = root.clone();
        w.land_from = None;
        w.display_name = "Test campaign".into();
        w.name = "bad name".into();
    }
    u.steps(2);
    assert!(
        u.h.query_all_by_label_contains("use letters, digits")
            .count()
            > 0,
        "the wizard says why the name is refused"
    );
    u.app().campaign_wizard().unwrap().name = "testcamp".into();
    u.click("FRE");
    u.steps(1);
    u.shot("wizard");
    u.click("Create");
    u.steps(2);
    assert!(u.app().campaign_wizard().is_none(), "the wizard closes");
    let cfg = root.join("campgns/testcamp.cfg");
    assert!(cfg.is_file());
    assert!(
        root.join("campgns/testcamp/text_fre.dat").is_file(),
        "French table"
    );
    assert!(
        root.join("campgns/testcamp_lnd").is_dir(),
        "land view folder"
    );
    let ed = u
        .app()
        .campaign_editor()
        .expect("the new pack opens in the campaign editor");
    assert_eq!(ed.pack.cfg_path, cfg);
    assert_eq!(ed.pack.common().name.as_deref(), Some("Test campaign"));
    u.shot("created");
}
