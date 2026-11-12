//! Interaction tests of the Events editor (the visual logic editor as a tab of the script
//! workspace, over the DK script text buffer). Same harness as `ui_tests`.
use crate::ui_tests::{level_copy, Ui};
use eframe::egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use kfx_formats::testutil;

fn count(text: &str, needle: &str) -> usize {
    text.matches(needle).count()
}

/// Click the script workspace's tab with this label (the tab bar is the highest one).
fn tab(u: &mut Ui, label: &str) {
    let r =
        u.h.query_all_by_label_contains(label)
            .map(|n| n.rect())
            .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
            .unwrap_or_else(|| panic!("no tab {label}"));
    u.click_at(r.center());
    u.steps(2);
}

/// Open level 4 (hero gate 1, five parties) with the script panel docked and wide.
fn level4(u: &mut Ui, tag: &str) -> crate::ui_tests::Tmp {
    let inst = testutil::install_dir().unwrap();
    let dir = level_copy(&inst, 4, tag);
    u.open(&dir.join("map00004.slb"));
    u.app().settings.script_panel_w = 900.0;
    u.key(Modifiers::NONE, Key::F3);
    u.steps(2);
    assert!(u.app().script_panel_open(), "F3 opens the script panel");
    dir
}

/// Open the Events tab, design a party, add escalating waves through the wave designer: the DK
/// script text gets the party and the waves and validates without new errors; pick a spawn
/// action point on the map; open a wave in the text.
#[test]
fn ui_events_add_wave() {
    if testutil::install_dir().is_none() {
        return;
    }
    let Some(mut u) = Ui::new("events-wave") else {
        return;
    };
    let _dir = level4(&mut u, "events-wave");
    let before = u.app().dk_script_text();
    let (errs0, _) = u.app().dk_script_problems();
    // the Events tab of the workspace (View > Events editor does the same)
    tab(&mut u, "Events");
    assert!(u.app().events_shown(), "the Events tab is shown");
    assert!(
        u.app().events_rect().is_some(),
        "the Events editor is laid out"
    );
    assert!(
        u.app().settings.logic.events_tab,
        "the tab choice goes into the settings"
    );
    u.shot("cards");

    u.click("Parties & waves");
    // room for the wave designer: the workspace over the map (F4)
    u.key(Modifiers::NONE, Key::F4);
    u.steps(2);
    assert_eq!(u.app().settings.logic.view, "parties");
    u.click("+ New");
    let text = u.app().dk_script_text();
    assert_eq!(
        count(&text, "CREATE_PARTY(PARTY1)"),
        1,
        "the new party is in the text at once"
    );
    assert_eq!(
        u.app().events_editor().unwrap().shown_party(),
        Some("PARTY1")
    );
    u.shot("party");
    // escalating waves (default: 3 waves of the first party from turn 6000 at hero gate 1)
    let adds = count(&text, "ADD_PARTY_TO_LEVEL");
    u.click("Create events");
    u.steps(2);
    let text = u.app().dk_script_text();
    assert_eq!(
        count(&text, "ADD_PARTY_TO_LEVEL"),
        adds + 3,
        "three waves written into the DK script"
    );
    assert!(
        text.contains("REM Wave 1 of 3: party WEAKLINGS"),
        "the wave events are in the text"
    );
    assert!(text.len() > before.len());
    let (errs1, _) = u.app().dk_script_problems();
    assert_eq!(errs1, errs0, "no new script errors");
    let report = u.app().events_report();
    assert!(report.contains("generated script: 0 error(s)"), "{report}");
    // selecting the new wave highlights its hero gate and draws the route to a heart
    let (routes, marks) = u.app().events_marks();
    assert!(marks.iter().any(|m| m.label() == "Gate 1"), "{marks:?}");
    assert!(
        routes
            .iter()
            .any(|r| r.party == "WEAKLINGS" && r.head_for.is_none()),
        "{routes:?}"
    );
    u.shot("waves");

    // the wave card's location: pick an action point on the map
    // the Events view (the lower of the two "Events": the workspace tab is above it)
    let r =
        u.h.query_all_by_label("Events")
            .map(|n| n.rect())
            .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
            .unwrap();
    u.click_at(r.center());
    u.steps(2);
    let maps: Vec<_> = u.h.query_all_by_label("Map").map(|n| n.rect()).collect();
    let r = *maps.last().expect("a Map button on the wave card");
    u.click_at(r.center());
    assert!(u.app().pick_kind().is_some(), "the map pick mode is on");
    u.app()
        .pick_with(kfx_script_ui::PickKind::Location, None, Some(3), None);
    assert!(u.app().pick_kind().is_none(), "the pick finished");
    let text = u.app().dk_script_text();
    assert!(
        text.contains("WEAKLINGS,3,"),
        "the picked action point is written: {}",
        &text[text.len().saturating_sub(300)..]
    );
    u.shot("picked");

    // the map marks follow the edit: action point 3 now
    let (routes, marks) = u.app().events_marks();
    assert!(marks.iter().any(|m| m.label() == "AP 3"), "{marks:?}");
    assert!(
        routes
            .iter()
            .any(|r| r.spawn.as_ref().is_some_and(|s| s.label() == "AP 3")),
        "{routes:?}"
    );

    // open in text: search the third wave, its card's Text button opens the text at its line
    let r = u.rect_of("Search");
    u.click_at(r.right_center() + eframe::egui::vec2(60.0, 0.0));
    u.text("Wave 3 of 3");
    u.key(Modifiers::NONE, Key::Escape);
    u.steps(2);
    let er = u.app().events_rect().unwrap();
    let texts: Vec<_> =
        u.h.query_all_by_label("Text")
            .map(|n| n.rect())
            .filter(|r| er.contains(r.center()))
            .collect();
    assert_eq!(texts.len(), 1, "one card matches the search");
    u.click_at(texts[0].center());
    u.steps(2);
    assert!(!u.app().events_shown(), "back in the text tab");
    let (line, _) = u
        .app()
        .script_ws()
        .unwrap()
        .active_editor_mut()
        .caret_line_col();
    let lines: Vec<String> = u
        .app()
        .dk_script_text()
        .lines()
        .map(str::to_string)
        .collect();
    let near = lines[line.saturating_sub(2)..(line + 1).min(lines.len())].join(" | ");
    assert!(
        near.contains("12000") || near.contains("Wave 3 of 3"),
        "the caret is at the third wave: line {line}: {near}"
    );
    u.shot("text");
}

/// One buffer: an edit of the text shows in Events, an Events edit shows in the text, the
/// Events undo reverts it in the text, and the text undo reverts an Events change.
#[test]
fn ui_events_handoff_undo() {
    if testutil::install_dir().is_none() {
        return;
    }
    let Some(mut u) = Ui::new("events-handoff") else {
        return;
    };
    let _dir = level4(&mut u, "events-handoff");
    let original = u.app().dk_script_text();

    // an edit in the text tab (a new timed event at the end)
    {
        let ws = u.app().script_ws().unwrap();
        let dk = ws.dk_name();
        let e = ws.editor_mut(&dk).unwrap();
        let end = e.text_bytes().len();
        assert!(e.replace_spans(&[(
            kfx_script::Span::new(end, end),
            "\nIF(PLAYER0,GAME_TURN >= 777)\n\tADD_GOLD_TO_PLAYER(PLAYER0,500)\nENDIF\n".into()
        )]));
    }
    u.steps(2);
    tab(&mut u, "Events");
    assert!(u.app().events_shown());
    let report = u.app().events_report();
    assert!(
        report.contains("GAME_TURN >= 777"),
        "the text edit shows in Events: {report}"
    );
    let n0 = report.lines().filter(|l| l.contains("When")).count();
    u.shot("from-text");

    // an edit in Events: + Event (IF(PLAYER0, GAME_TURN >= 2000))
    let with_text_edit = u.app().dk_script_text();
    u.click("+ Event");
    u.steps(2);
    let text = u.app().dk_script_text();
    assert_eq!(
        count(&text, "GAME_TURN >= 2000") + count(&text, "GAME_TURN,>=,2000"),
        count(&with_text_edit, "GAME_TURN >= 2000")
            + count(&with_text_edit, "GAME_TURN,>=,2000")
            + 1,
        "the new event is in the text"
    );
    assert!(text.contains("777"), "the text edit is kept");
    // Events undo: the text follows
    u.click("Undo");
    u.steps(2);
    assert_eq!(
        u.app().dk_script_text(),
        with_text_edit,
        "the Events undo reverts the text"
    );
    // redo it, then see it in the text tab
    u.click("Redo");
    u.steps(2);
    let with_event = u.app().dk_script_text();
    assert_ne!(with_event, with_text_edit);
    tab(&mut u, "map00004.txt");
    assert!(!u.app().events_shown(), "the text tab is shown");
    assert_eq!(
        u.app().script_active_text(),
        with_event,
        "the text tab shows the Events edit"
    );
    u.shot("to-text");

    // undo in the text editor reverts the Events change (and the model follows)
    let r = u.app().script_text_rect().expect("script editor laid out");
    u.click_at(r.center());
    // egui's text undo: when the caret moved since the last undo point (the click), the first
    // Ctrl+Z only puts the caret back; the text changes on the next one
    let mut after_undo = with_event.clone();
    for _ in 0..2 {
        if after_undo != with_event {
            break;
        }
        u.key(Modifiers::COMMAND, Key::Z);
        u.steps(2);
        after_undo = u.app().script_active_text();
    }
    assert!(
        count(&after_undo, "2000") < count(&with_event, "2000"),
        "Ctrl+Z in the text undoes the Events edit"
    );
    tab(&mut u, "Events");
    let report = u.app().events_report();
    let n1 = report.lines().filter(|l| l.contains("When")).count();
    assert!(
        n1 <= n0,
        "the Events model follows the text undo ({n1} vs {n0} events)"
    );
    assert!(
        after_undo.len() <= with_text_edit.len()
            || after_undo == original
            || after_undo.contains("777")
    );
    u.shot("after-undo");

    // the keyboard shortcut toggles back to the text
    u.key(Modifiers::COMMAND, Key::F3);
    assert!(!u.app().events_shown(), "Ctrl+F3 goes back to the text");
    u.key(Modifiers::COMMAND, Key::F3);
    assert!(u.app().events_shown(), "Ctrl+F3 shows the Events tab");
}

/// A message argument's "Str" button opens the string picker of the level's string tables;
/// the chosen id is written into the script.
#[test]
fn ui_events_string_pick() {
    if testutil::install_dir().is_none() {
        return;
    }
    let Some(mut u) = Ui::new("events-strings") else {
        return;
    };
    let _dir = level4(&mut u, "events-strings");
    u.app().open_events();
    u.steps(2);
    assert!(u.app().events_shown(), "Level > Events opens the tab");
    // the winning card: DISPLAY_OBJECTIVE(53, PLAYER0) then WIN_GAME
    let r = u.rect_of("Search");
    u.click_at(r.right_center() + eframe::egui::vec2(60.0, 0.0));
    u.text("WIN_GAME");
    u.key(Modifiers::NONE, Key::Escape);
    u.steps(2);
    let er = u.app().events_rect().unwrap();
    let strs: Vec<_> =
        u.h.query_all_by_label("Str")
            .map(|n| n.rect())
            .filter(|r| er.contains(r.center()))
            .collect();
    assert_eq!(strs.len(), 1, "one message argument on the card");
    u.click_at(strs[0].center());
    u.steps(2);
    assert!(u.has("Pick a string"), "the string picker opens");
    // filter by id and choose it
    let field =
        u.h.query_all_by_label_contains("New level string")
            .next()
            .unwrap()
            .rect();
    u.click_at(field.left_center() - eframe::egui::vec2(60.0, 0.0));
    u.text("54");
    u.steps(2);
    u.shot("picker");
    let row =
        u.h.query_all_by_label_contains("  54  ")
            .next()
            .expect("string 54 listed")
            .rect();
    u.click_at(row.center());
    u.steps(2);
    assert!(!u.has("Pick a string"), "the picker closes");
    let text = u.app().dk_script_text();
    assert!(
        text.contains("DISPLAY_OBJECTIVE(54,PLAYER0)")
            || text.contains("DISPLAY_OBJECTIVE(54, PLAYER0)"),
        "the id is written: {}",
        text.lines()
            .find(|l| l.contains("DISPLAY_OBJECTIVE(5"))
            .unwrap_or("")
    );
    u.shot("picked");
}
