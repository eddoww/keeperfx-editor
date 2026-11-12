//! Headless widget tests (egui_kittest, no rendering).
use egui::vec2;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use kfx_logic_ui::*;
use kfx_script::{ConstantProvider, StaticProvider};

struct Host(StaticProvider);
impl LogicHost for Host {
    fn provider(&self) -> &dyn ConstantProvider {
        &self.0
    }
}

struct State {
    ed: LogicEditor,
    host: Host,
    events: Vec<LogicEvent>,
}

const SCRIPT: &str = "LEVEL_VERSION(1)\nCREATE_PARTY(RAID)\n\
    \tADD_TO_PARTY(RAID,KNIGHT,3,100,ATTACK_ENEMIES,0)\nIF_ACTION_POINT(2,PLAYER0)\n\
    \tADD_PARTY_TO_LEVEL(PLAYER_GOOD,RAID,2,1)\nENDIF\nIF(PLAYER0,GAME_TURN >= \
    1200)\n\tSET_TIMER(PLAYER0,TIMER1)\nENDIF\nIF(PLAYER0,TIMER1 >= 600)\n\
    \tWIN_GAME\nENDIF\n";

fn harness(script: &str) -> Harness<'static, State> {
    let mut ed = LogicEditor::new();
    ed.load(script.as_bytes());
    let st = State {
        ed,
        host: Host(StaticProvider::default()),
        events: vec![],
    };
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1300.0, 900.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, s: &mut State| {
                let ev = s.ed.show(ui, &s.host);
                s.events.extend(ev);
            },
            st,
        );
    h.run_steps(3);
    h
}

#[test]
fn delete_and_undo() {
    let mut h = harness(SCRIPT);
    assert_eq!(h.state().ed.script_bytes(), SCRIPT.as_bytes());
    let n = h.get_all_by_label("Del").count();
    assert!(n >= 3, "{n} Del buttons");
    h.get_all_by_label("Del").last().unwrap().click();
    h.run_steps(3);
    let after = String::from_utf8(h.state().ed.script_bytes()).unwrap();
    assert!(!after.contains("WIN_GAME"), "{after}");
    assert!(h.state().events.contains(&LogicEvent::Changed));
    h.get_by_label("Undo").click();
    h.run_steps(3);
    assert_eq!(h.state().ed.script_bytes(), SCRIPT.as_bytes());
    h.get_by_label("Redo").click();
    h.run_steps(3);
    assert_eq!(
        String::from_utf8(h.state().ed.script_bytes()).unwrap(),
        after
    );
}

#[test]
fn map_pick_round_trip() {
    let mut h = harness(SCRIPT);
    h.state_mut().events.clear();
    // the first "Map" button belongs to the IF_ACTION_POINT condition's action point
    h.get_all_by_label("Map").next().unwrap().click();
    h.run_steps(3);
    let req = h
        .state()
        .events
        .iter()
        .find_map(|e| match e {
            LogicEvent::PickLocation { request, kind } => Some((*request, *kind)),
            _ => None,
        })
        .expect("pick event");
    assert_eq!(req.1, PickKind::ActionPoint);
    h.state_mut()
        .ed
        .answer_pick(req.0, PickValue::ActionPoint(7));
    h.run_steps(2);
    let s = String::from_utf8(h.state().ed.script_bytes()).unwrap();
    assert!(s.contains("IF_ACTION_POINT(7,PLAYER0)"), "{s}");
    // the spawn location of the party: pick a hero gate
    let id = h
        .state()
        .ed
        .doc()
        .events()
        .into_iter()
        .find(|(e, _)| {
            e.conditions
                .first()
                .map(|c| c.cmd.name == "IF_ACTION_POINT")
                .unwrap_or(false)
        })
        .unwrap()
        .0
        .body[0]
        .id();
    h.state_mut()
        .ed
        .answer_pick(PickRequest { node: id, slot: 2 }, PickValue::HeroGate(3));
    let s = String::from_utf8(h.state().ed.script_bytes()).unwrap();
    assert!(
        s.contains("ADD_PARTY_TO_LEVEL(PLAYER_GOOD,RAID,-3,1)"),
        "{s}"
    );
}

#[test]
fn views_switch_and_highlight() {
    let mut h = harness(SCRIPT);
    for (label, v) in [
        ("Graph", View::Graph),
        ("Timeline", View::Timeline),
        ("Parties & waves", View::Parties),
        ("Variables", View::Variables),
        ("Lua export", View::Lua),
        ("Events", View::Cards),
    ] {
        h.get_by_label(label).click();
        h.run_steps(3);
        assert_eq!(h.state().ed.view(), v);
    }
    // selecting an event highlights its locations
    let ev = h
        .state()
        .ed
        .doc()
        .events()
        .into_iter()
        .find(|(e, _)| !e.conditions.is_empty())
        .unwrap()
        .0
        .id;
    h.state_mut().events.clear();
    h.state_mut().ed.select(ev);
    h.run_steps(2);
    let hl = h.state().events.iter().find_map(|e| match e {
        LogicEvent::Highlight { event, locations } => Some((*event, locations.clone())),
        _ => None,
    });
    let (e, locs) = hl.expect("highlight");
    assert_eq!(e, Some(ev));
    assert!(locs
        .iter()
        .any(|l| l.loc == analysis::MapLoc::ActionPoint(2)));
    // "Text" asks the host to open the script at the event's line
    h.state_mut().events.clear();
    h.get_all_by_label("Text").nth(1).unwrap().click();
    h.run_steps(2);
    assert!(
        h.state()
            .events
            .iter()
            .any(|e| matches!(e, LogicEvent::OpenInText { line: 4 })),
        "{:?}",
        h.state().events
    );
}

#[test]
fn timeline_retime_and_connect() {
    let mut h = harness(SCRIPT);
    let doc = h.state().ed.doc().clone();
    let turn_ev = doc
        .events()
        .into_iter()
        .find(|(e, _)| e.conditions.iter().any(|c| c.cmd.arg(1) == "GAME_TURN"))
        .unwrap()
        .0
        .clone();
    h.state_mut().ed.retime(turn_ev.conditions[0].id, 3000);
    let s = String::from_utf8(h.state().ed.script_bytes()).unwrap();
    assert!(s.contains("IF(PLAYER0,GAME_TURN >= 3000)"), "{s}");
    let ap_ev = doc
        .events()
        .into_iter()
        .find(|(e, _)| e.conditions.iter().any(|c| c.cmd.name == "IF_ACTION_POINT"))
        .unwrap()
        .0
        .id;
    h.state_mut().ed.connect(ap_ev, turn_ev.id);
    let s = String::from_utf8(h.state().ed.script_bytes()).unwrap();
    assert!(s.contains("REM kfx-var Chain0 = PLAYER0.FLAG0"), "{s}");
    assert!(s.contains("\tSET_FLAG(PLAYER0,FLAG0,1)"), "{s}");
    assert!(s.contains("\tIF(PLAYER0,FLAG0 == 1)"), "{s}");
    let back = import(s.as_bytes());
    assert!(back.read_only.is_none());
    h.state_mut().ed.undo();
    h.state_mut().ed.undo();
    assert_eq!(h.state().ed.script_bytes(), SCRIPT.as_bytes());
}

#[test]
fn view_only_has_no_edit_buttons() {
    let mut h = harness("IF(PLAYER0,FLAG1 == 1)\nWIN_GAME\n");
    assert!(h.state().ed.read_only().is_some());
    assert_eq!(h.query_all_by_label("Del").count(), 0);
    // programmatic edits are refused too
    h.state_mut().ed.edit(|d| d.nodes.clear());
    h.run_steps(2);
    assert_eq!(
        h.state().ed.script_bytes(),
        b"IF(PLAYER0,FLAG1 == 1)\nWIN_GAME\n"
    );
}

#[test]
fn wave_template_from_designer() {
    let mut h = harness(SCRIPT);
    h.get_by_label("Parties & waves").click();
    h.run_steps(3);
    h.get_by_label("Create events").click();
    h.run_steps(3);
    let s = String::from_utf8(h.state().ed.script_bytes()).unwrap();
    assert!(
        s.contains(
            "IF(PLAYER0,GAME_TURN >= 6000)\n\tADD_PARTY_TO_LEVEL(PLAYER_GOOD,RAID,-1,1)\nENDIF"
        ),
        "{s}"
    );
    assert!(
        s.contains(
            "IF(PLAYER0,GAME_TURN >= 12000)\n\tADD_PARTY_TO_LEVEL(PLAYER_GOOD,RAID,-1,3)\nENDIF"
        ),
        "{s}"
    );
    let r = kfx_script::validate(
        &kfx_script::Document::parse(s.as_bytes()),
        &kfx_script::NullProvider,
    );
    assert!(
        !r.diagnostics
            .iter()
            .any(|d| d.severity == kfx_script::Severity::Error),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn drag_card_to_reorder() {
    let mut h = harness(SCRIPT);
    let head = |h: &Harness<'static, State>, text: &str| h.get_by_label(text).rect();
    let src = head(&h, "WHEN PLAYER0: TIMER1 >= 600");
    let dst = head(&h, "WHEN PLAYER0 reaches action point 2");
    let handle = h
        .get_all_by_label("::")
        .map(|n| n.rect())
        .find(|r| (r.center().y - src.center().y).abs() < 4.0)
        .expect("handle of the TIMER1 card");
    h.hover_at(handle.center());
    h.run_steps(2);
    h.drag_at(handle.center());
    h.run_steps(2);
    for k in 1..=6 {
        let t = k as f32 / 6.0;
        h.hover_at(handle.center() + (dst.center() - handle.center()) * t);
        h.run_steps(2);
    }
    h.drop_at(dst.center());
    h.run_steps(4);
    let s = String::from_utf8(h.state().ed.script_bytes()).unwrap();
    let (a, b) = (
        s.find("TIMER1 >= 600").unwrap(),
        s.find("IF_ACTION_POINT").unwrap(),
    );
    assert!(
        a < b,
        "the TIMER1 event moved before the action point event:\n{s}"
    );
    compare_per_context(SCRIPT.as_bytes(), s.as_bytes()).expect("same commands per context");
}

/// At the width of the docked script panel the toolbar and the script meters stay a few rows
/// tall, so that the cards start near the top instead of below an empty band.
#[test]
fn narrow_panel_keeps_toolbar_compact() {
    let mut ed = LogicEditor::new();
    ed.load(SCRIPT.as_bytes());
    let st = State {
        ed,
        host: Host(StaticProvider::default()),
        events: vec![],
    };
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(600.0, 900.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, s: &mut State| {
                let ev = s.ed.show(ui, &s.host);
                s.events.extend(ev);
            },
            st,
        );
    h.run_steps(3);
    let search = h.get_by_label("Search").rect();
    assert!(
        search.top() < 160.0,
        "the cards start at y {} in a 600 point wide panel",
        search.top()
    );
    let add = h.get_by_label("+ Event").rect();
    assert!(add.height() < 30.0, "+ Event wraps: {add:?}");
}

/// Renaming the party in the designer also renames it in the wave form, so that the waves
/// created next use the party that exists.
#[test]
fn wave_form_follows_a_party_rename() {
    let mut h = harness(SCRIPT);
    h.get_by_label("Parties & waves").click();
    h.run_steps(3);
    // the rename field holds the party name; replace it
    h.get_all_by_value("RAID")
        .next()
        .expect("rename field")
        .click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in "RAIDERS".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.run_steps(3);
    h.get_by_label("Create events").click();
    h.run_steps(3);
    let s = String::from_utf8(h.state().ed.script_bytes()).unwrap();
    assert!(s.contains("CREATE_PARTY(RAIDERS)"), "{s}");
    assert!(
        s.contains("ADD_PARTY_TO_LEVEL(PLAYER_GOOD,RAIDERS,-1,1)"),
        "{s}"
    );
    assert!(!s.contains("RAID,"), "{s}");
}
