//! The widget, driven headless through egui_kittest (no rendering backend needed).
mod common;
use common::*;
use egui::{vec2, Event as Ev};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use kfx_campaign_ui::pack::{AddLevel, LevelSource};
use kfx_campaign_ui::ui::{to_picture, to_screen, CampaignEditor, Tab};
use kfx_campaign_ui::{create_pack, Event, NewPack, Pack};
use kfx_formats::levelinfo::LevelKind;
use kfx_formats::testutil::install_dir;
use std::path::Path;

struct S {
    ed: CampaignEditor,
    events: Vec<Event>,
}

fn harness(pack: Pack) -> Harness<'static, S> {
    let h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1280.0, 860.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, s: &mut S| {
                let ev = s.ed.show(ui);
                s.events.extend(ev);
            },
            S {
                ed: CampaignEditor::new(pack),
                events: vec![],
            },
        );
    // the default fonts have no CJK glyphs; the shipped string tables do
    h.ctx
        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    h
}

fn press(h: &mut Harness<'static, S>, p: egui::Pos2, down: bool) {
    h.event(Ev::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed: down,
        modifiers: Default::default(),
    });
    h.run_steps(2);
}

fn drag(h: &mut Harness<'static, S>, from: egui::Pos2, to: egui::Pos2) {
    h.hover_at(from);
    h.run_steps(2);
    press(h, from, true);
    let steps = 6;
    for i in 1..=steps {
        let p = from + (to - from) * (i as f32 / steps as f32);
        h.hover_at(p);
        h.run_steps(2);
    }
    press(h, to, false);
}

#[test]
fn every_tab_of_every_shipped_campaign_renders() {
    let Some(inst) = install_dir() else { return };
    for e in std::fs::read_dir(inst.join("campgns")).unwrap().flatten() {
        let p = e.path();
        if !p.extension().is_some_and(|x| x == "cfg") {
            continue;
        }
        let pack = Pack::load(&inst, &p).unwrap();
        let mut h = harness(pack);
        for tab in [
            Tab::Overview,
            Tab::Levels,
            Tab::LandView,
            Tab::Texts,
            Tab::Health,
        ] {
            h.state_mut().ed.tab = tab;
            h.run_steps(3);
        }
        assert!(
            !h.state().ed.pack.is_dirty(),
            "{}: viewing changes nothing",
            p.display()
        );
    }
}

#[test]
fn map_packs_render_too() {
    let Some(inst) = install_dir() else { return };
    for rel in ["levels/classic.cfg", "multiplayer/original.cfg"] {
        let mut h = harness(Pack::load(&inst, &inst.join(rel)).unwrap());
        for tab in [
            Tab::Overview,
            Tab::Levels,
            Tab::LandView,
            Tab::Texts,
            Tab::Health,
        ] {
            h.state_mut().ed.tab = tab;
            h.run_steps(3);
        }
        assert!(!h.state().ed.pack.is_dirty());
    }
}

#[test]
fn open_and_play_events() {
    let Some(inst) = install_dir() else { return };
    let mut h = harness(Pack::load(&inst, &inst.join("campgns/keeporig.cfg")).unwrap());
    h.state_mut().ed.tab = Tab::Levels;
    h.state_mut().ed.selected = Some(3);
    h.run_steps(3);
    h.get_by_label("Open in editor").click();
    h.run_steps(3);
    h.get_by_label("Play").click();
    h.run_steps(3);
    let ev = &h.state().events;
    assert!(
        ev.iter().any(
            |e| matches!(e, Event::OpenLevel { number: 3, folder } if folder.ends_with("keeporig"))
        ),
        "{ev:?}"
    );
    assert!(
        ev.iter().any(|e| *e
            == Event::PlayLevel {
                campaign: "keeporig".into(),
                number: 3
            }),
        "{ev:?}"
    );
}

#[test]
fn dragging_an_ensign_edits_the_cfg() {
    let Some(inst) = install_dir() else { return };
    let mut h = harness(Pack::load(&inst, &inst.join("campgns/keeporig.cfg")).unwrap());
    h.state_mut().ed.tab = Tab::LandView;
    h.state_mut().ed.selected = Some(1);
    h.run_steps(4);
    let canvas = h.state().ed.last_canvas.expect("canvas drawn");
    let before = h.state().ed.pack.level_info(1);
    assert_eq!(before.ensign_pos, Some((808, 546)));
    // the ensign pole is drawn above its position; grab it in the middle
    let at = to_screen(canvas, (808, 546)) - vec2(0.0, 14.0);
    let delta = vec2(60.0, -40.0);
    drag(&mut h, at, at + delta);
    h.run_steps(3);
    let after = h.state().ed.pack.level_info(1);
    let k = 1280.0 / canvas.width();
    let want = ((808.0 + delta.x * k) as i32, (546.0 + delta.y * k) as i32);
    let got = after.ensign_pos.unwrap();
    assert!(
        (got.0 - want.0).abs() <= 3 && (got.1 - want.1).abs() <= 3,
        "ensign {got:?}, wanted about {want:?}"
    );
    assert_eq!(
        after.ensign_zoom, after.ensign_pos,
        "the zoom point followed (they were equal)"
    );
    assert!(h.state().ed.pack.is_dirty());
    assert!(h.state().events.contains(&Event::Changed));
    // only the two lines of that level changed in the cfg text
    let old = std::fs::read_to_string(inst.join("campgns/keeporig.cfg")).unwrap();
    let new = String::from_utf8_lossy(&h.state().ed.pack.cfg.to_bytes()).to_string();
    let diff: Vec<_> = old
        .lines()
        .zip(new.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(old.lines().count(), new.lines().count());
    assert_eq!(diff.len(), 2, "{diff:?}");
    assert!(diff
        .iter()
        .all(|(a, _)| a.starts_with("ENSIGN_POS") || a.starts_with("ENSIGN_ZOOM")));
    assert_eq!(
        to_picture(canvas, to_screen(canvas, (100, 200))),
        (100, 200)
    );
}

#[test]
fn dragging_a_level_row_reorders_the_campaign() {
    let Some(inst) = private_install("ui-reorder") else {
        return;
    };
    let mut spec = NewPack::campaign("rows", "Rows");
    spec.land_from = Some(inst.join("campgns/keeporig.cfg"));
    let mut pack = create_pack(&inst, &spec).unwrap();
    for n in [1u32, 2, 3] {
        let mut s = LevelSource::new(&inst.join("campgns/keeporig"), n);
        s.info = None;
        pack.add_level(&AddLevel::new(s, LevelKind::Single))
            .unwrap();
    }
    pack.save().unwrap();
    let mut h = harness(pack);
    h.state_mut().ed.tab = Tab::Levels;
    h.run_steps(4);
    let rows = h.state().ed.last_rows.clone();
    assert_eq!(rows.len(), 3);
    // drag the first row's handle below the last row
    let from = egui::pos2(rows[0].1.min.x + 8.0, rows[0].1.center().y);
    let to = egui::pos2(from.x, rows[2].1.center().y + 4.0);
    drag(&mut h, from, to);
    h.run_steps(3);
    let order: Vec<u32> = h
        .state()
        .ed
        .pack
        .single_slots()
        .iter()
        .map(|s| s.level)
        .collect();
    assert_eq!(order, vec![2, 3, 1]);
    assert!(h.state().ed.pack.is_dirty());
    // Save writes the cfg
    h.get_by_label_contains("Save").click();
    h.run_steps(3);
    assert!(h.state().events.contains(&Event::Saved));
    let r = kfx_formats::campaign::resolve(&inst, &h.state().ed.pack.cfg_path).unwrap();
    assert_eq!(r.common.single_levels, vec![2, 3, 1]);
    cleanup(&inst);
}

#[test]
fn health_fix_button() {
    let Some(inst) = private_install("ui-health") else {
        return;
    };
    let mut pack = create_pack(&inst, &NewPack::campaign("hl", "Health")).unwrap();
    pack.update_common(|c| c.creatures_location = Some("campgns/hl_crtr".into()));
    let mut h = harness(pack);
    h.state_mut().ed.tab = Tab::Health;
    h.run_steps(3);
    assert!(h
        .state()
        .ed
        .issues
        .iter()
        .any(|i| i.code == "folder-missing"));
    h.get_by_label_contains("Apply all fixes").click();
    h.run_steps(3);
    assert!(Path::new(&inst).join("campgns/hl_crtr").is_dir());
    assert!(!h
        .state()
        .ed
        .issues
        .iter()
        .any(|i| i.code == "folder-missing"));
    cleanup(&inst);
}

#[test]
fn viewing_out_of_range_values_changes_nothing() {
    let Some(inst) = private_install("ui-view") else {
        return;
    };
    let mut spec = NewPack::campaign("view", "View");
    spec.land_from = Some(inst.join("campgns/keeporig.cfg"));
    let mut pack = create_pack(&inst, &spec).unwrap();
    pack.add_level(&AddLevel::new(
        LevelSource::new(&inst.join("campgns/keeporig"), 1),
        LevelKind::Single,
    ))
    .unwrap();
    let n = pack.single_slots()[0].level;
    // values the form's drag boxes cannot show: they must stay as written until the user edits them
    let mut info = pack.level_info(n);
    info.ensign_pos = Some((1300, 2000));
    info.players = Some(12);
    info.mapsize = Some((300, 300));
    pack.set_level_info(n, &info);
    pack.save().unwrap();
    let before = pack.cfg.to_bytes();
    let mut h = harness(pack);
    for tab in [
        Tab::Overview,
        Tab::Levels,
        Tab::LandView,
        Tab::Texts,
        Tab::Health,
    ] {
        h.state_mut().ed.tab = tab;
        h.run_steps(4);
    }
    assert!(
        !h.state().ed.pack.is_dirty(),
        "looking at a level edited it"
    );
    assert_eq!(h.state().ed.pack.cfg.to_bytes(), before);
    assert!(
        h.state()
            .ed
            .issues
            .iter()
            .any(|i| i.code == "ensign-outside"),
        "the bad ensign is still reported"
    );
    // the add panel lists the install's levels
    h.state_mut().ed.tab = Tab::Levels;
    h.state_mut().ed.add_open = true;
    h.run_steps(4);
    h.get_by_label_contains("keeporig 7 Wishvale");
    cleanup(&inst);
}
