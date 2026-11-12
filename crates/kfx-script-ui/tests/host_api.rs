//! Additive host APIs: `replace_spans` (one undo step), string lookups for message-id
//! arguments, the new-string event, diagnostics hidden at the caret, command names on map
//! references.
use egui::{vec2, Event, Key, Modifiers};
use egui_kittest::Harness;
use kfx_script::{NullProvider, Span, StaticProvider};
use kfx_script_ui::strings::NEW_STRING_LABEL;
use kfx_script_ui::*;
use std::sync::Arc;

struct State {
    ed: ScriptEditor,
    host: StaticProvider,
    events: Vec<ScriptEvent>,
}

fn harness(script: &[u8]) -> Harness<'static, State> {
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(script);
    ed.set_side_panel(SidePanel::None);
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(900.0, 500.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, s: &mut State| {
                let ev = s.ed.show(ui, &s.host);
                s.events.extend(ev);
            },
            State {
                ed,
                host: StaticProvider::default(),
                events: vec![],
            },
        );
    h.run_steps(3);
    h
}

fn text(h: &Harness<'static, State>) -> String {
    h.state()
        .ed
        .text_bytes()
        .iter()
        .map(|&b| b as char)
        .collect()
}

fn lookup() -> StringLookup {
    Arc::new(|id| match id {
        7 => Some("Dig to the east and find the gold.".to_string()),
        42 => Some("Line one\nline two".to_string()),
        100 => Some("Third".to_string()),
        _ => None,
    })
}

#[test]
fn replace_spans_is_one_undo_step() {
    let src = "LEVEL_VERSION(1)\nIF_ACTION_POINT(3, PLAYER0)\n\
        ZOOM_TO_LOCATION(PLAYER0, 3)\nZOOM_TO_LOCATION(PLAYER0, 3)\n";
    let mut h = harness(src.as_bytes());
    // focus the text like a user would have
    h.state_mut().ed.set_caret(src.len());
    h.state_mut()
        .ed
        .reveal_span(Span::new(src.len(), src.len()));
    h.run_steps(3);
    let spans: Vec<Span> = src
        .match_indices('3')
        .map(|(i, _)| Span::new(i, i + 1))
        .collect();
    assert_eq!(spans.len(), 3);
    let edits: Vec<(Span, String)> = spans.iter().map(|s| (*s, "12".to_string())).collect();
    assert!(h.state_mut().ed.replace_spans(&edits));
    h.run_steps(3);
    assert_eq!(text(&h), src.replace('3', "12"));
    assert!(h.state().ed.is_dirty());
    // one Ctrl+Z reverts all three
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(4);
    assert_eq!(text(&h), src, "all replacements undone together");
    // overlapping or out-of-range spans change nothing
    assert!(!h
        .state_mut()
        .ed
        .replace_spans(&[(Span::new(0, 5), "x".into()), (Span::new(3, 8), "y".into())]));
    assert!(!h
        .state_mut()
        .ed
        .replace_spans(&[(Span::new(0, 100000), "x".into())]));
    h.run_steps(2);
    assert_eq!(text(&h), src);
}

#[test]
fn replace_spans_keeps_the_caret_and_maps_crlf() {
    let src = "A(1)\r\nB(2)\r\nC(3)\r\n";
    let mut e = ScriptEditor::new();
    e.set_text_bytes(src.as_bytes());
    let at = |needle: &str| src.find(needle).unwrap();
    let caret_buf = at("C(3)") - 2 + 2; // buffer coordinates drop the CRs: line 3 starts at 10
    e.set_caret(caret_buf - 2);
    assert!(e.replace_spans(&[
        (Span::new(at("1"), at("1") + 1), "100".into()),
        (Span::new(at("3"), at("3") + 1), "300".into())
    ]));
    e.apply_pending();
    assert_eq!(e.text_bytes(), b"A(100)\r\nB(2)\r\nC(300)\r\n");
}

#[test]
fn workspace_replace_spans_per_file() {
    let mut ws = workspace::ScriptWorkspace::new(7);
    struct F(std::cell::RefCell<std::collections::HashMap<String, Vec<u8>>>);
    impl workspace::ScriptFiles for F {
        fn read(&self, n: &str) -> Option<Vec<u8>> {
            self.0.borrow().get(n).cloned()
        }
        fn write(&self, n: &str, b: &[u8]) -> Result<(), String> {
            self.0.borrow_mut().insert(n.to_string(), b.to_vec());
            Ok(())
        }
    }
    let files = F(Default::default());
    files.0.borrow_mut().insert(
        "map00007.txt".into(),
        b"LEVEL_VERSION(1)\nZOOM_TO_LOCATION(PLAYER0, 3)\n".to_vec(),
    );
    ws.load(&files);
    let src = "LEVEL_VERSION(1)\nZOOM_TO_LOCATION(PLAYER0, 3)\n";
    let i = src.find('3').unwrap();
    assert!(ws.replace_spans("map00007.txt", &[(Span::new(i, i + 1), "9".into())]));
    ws.editor_mut("map00007.txt").unwrap().apply_pending();
    assert!(ws
        .editor("map00007.txt")
        .unwrap()
        .text_bytes()
        .ends_with(b"PLAYER0, 9)\n"));
    assert!(!ws.replace_spans("nope.txt", &[(Span::new(0, 1), "x".into())]));
}

#[test]
fn string_hover_and_completion_for_message_arguments() {
    let src = "LEVEL_VERSION(1)\nDISPLAY_OBJECTIVE(7, PLAYER0)\n\
        DISPLAY_OBJECTIVE(55, PLAYER0)\nDISPLAY_OBJECTIVE(\n";
    let an = DkLanguage.analyze(src.as_bytes());
    let l = lookup();
    let at7 = src.find("(7").unwrap() + 1;
    let h = an.hover_with(at7, Some(&l)).expect("hover");
    assert!(h.summary.starts_with("Dig to the east"), "{h:?}");
    assert!(h.title.unwrap().contains("string 7"));
    // an id without text says so
    let at55 = src.find("(55").unwrap() + 2;
    assert!(an
        .hover_with(at55, Some(&l))
        .unwrap()
        .summary
        .contains("No text for string 55"));
    // without a lookup the ordinary hover is used, a non message argument is not affected
    assert!(an
        .hover_with(at7, None)
        .map_or(true, |h| !h.summary.starts_with("Dig")));
    let player = src.find("PLAYER0").unwrap() + 2;
    assert!(an
        .hover_with(player, Some(&l))
        .map_or(true, |h| !h.summary.contains("string")));
    // completion: ids with a preview, first the new-string item
    let end = src.len() - 1;
    let c = an.completions_with(end, &NullProvider, Some(&l));
    let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![NEW_STRING_LABEL, "7", "42", "100"],
        "{labels:?}"
    );
    assert_eq!(c[0].action, Some(CompletionAction::NewString));
    assert!(c[1].detail.starts_with("Dig to the east"));
    assert_eq!(
        c[2].detail, "Line one …",
        "multi-line texts are cut at the first line"
    );
    assert_eq!(c[2].doc, "Line one\nline two");
    // typed digits filter
    let src2 = "LEVEL_VERSION(1)\nDISPLAY_OBJECTIVE(4";
    let c =
        DkLanguage
            .analyze(src2.as_bytes())
            .completions_with(src2.len(), &NullProvider, Some(&l));
    assert_eq!(
        c.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        vec![NEW_STRING_LABEL, "42"]
    );
}

#[test]
fn lua_equivalents_use_the_lookup() {
    let lang = LuaLanguage::new();
    let src = "DisplayInformation(7, PLAYER0)\nDisplayInformation(42)\n\
        GetString(100)\nSetBoxTooltipId(1, 7)\nStartMoney(PLAYER0, 7)\n";
    let an = lang.analyze(src.as_bytes());
    let l = lookup();
    assert!(an
        .hover_with(src.find("(7").unwrap() + 1, Some(&l))
        .unwrap()
        .summary
        .starts_with("Dig to the east"));
    assert!(
        an.hover_with(src.find("(100").unwrap() + 2, Some(&l))
            .unwrap()
            .summary
            == "Third"
    );
    let tip = src.find("1, 7").unwrap() + 3;
    assert!(an
        .hover_with(tip, Some(&l))
        .unwrap()
        .summary
        .starts_with("Dig"));
    // an integer parameter of another function is not a string id
    let money = src.find("PLAYER0, 7)\n").unwrap() + 10;
    assert!(an
        .hover_with(money, Some(&l))
        .map_or(true, |h| !h.summary.starts_with("Dig")));
    let s2 = "DisplayInformation(";
    let c = lang
        .analyze(s2.as_bytes())
        .completions_with(s2.len(), &NullProvider, Some(&l));
    assert_eq!(
        c.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        vec![NEW_STRING_LABEL, "7", "42", "100"]
    );
}

#[test]
fn new_string_event_and_answer() {
    let src = "LEVEL_VERSION(1)\nDISPLAY_OBJECTIVE(7, PLAYER0)\nDISPLAY_INFORMATION(";
    let mut h = harness(src.as_bytes());
    h.state_mut().ed.set_string_lookup(Some(lookup()));
    // caret at the open argument of the last command: ask for a new string
    let n = src.len();
    h.state_mut().ed.set_caret(n);
    h.run_steps(3);
    h.state_mut().ed.request_new_string();
    h.run_steps(2);
    let ev: Vec<ScriptEvent> = h.state_mut().events.drain(..).collect();
    assert!(
        ev.contains(&ScriptEvent::RequestNewString {
            at: Span::new(n, n)
        }),
        "{ev:?}"
    );
    h.state_mut().ed.insert_string_id(321);
    h.run_steps(3);
    assert_eq!(text(&h), format!("{src}321"));
    // the id argument under the caret is replaced
    let at = src.find("(7").unwrap() + 2;
    h.state_mut().ed.set_caret(at);
    h.run_steps(3);
    h.state_mut().ed.request_new_string();
    h.run_steps(2);
    let ev: Vec<ScriptEvent> = h.state_mut().events.drain(..).collect();
    assert!(
        ev.iter().any(
            |e| matches!(e, ScriptEvent::RequestNewString { at: s } if *s == Span::new(at - 1, at))
        ),
        "{ev:?}"
    );
    h.state_mut().ed.insert_string_id(88);
    h.run_steps(3);
    assert!(
        text(&h).contains("DISPLAY_OBJECTIVE(88, PLAYER0)"),
        "{}",
        text(&h)
    );
    // accepting the completion item raises the same event
    let n = text(&h).len();
    h.state_mut().ed.set_caret(n);
    h.run_steps(3);
    h.state_mut()
        .ed
        .refresh_completions_for_test(&StaticProvider::default());
    assert_eq!(
        h.state().ed.completion_labels().first().map(|s| s.as_str()),
        Some(NEW_STRING_LABEL)
    );
    h.state_mut().ed.accept_completion(0);
    h.run_steps(2);
    assert!(h
        .state()
        .events
        .iter()
        .any(|e| matches!(e, ScriptEvent::RequestNewString { .. })));
    let _ = Event::Text(String::new());
}

#[test]
fn value_diagnostics_are_hidden_at_the_caret() {
    let src = "LEVEL_VERSION(1)\nCREATURE_AVAILABLE(PLAYER0,DR";
    let mut e = ScriptEditor::new();
    e.set_text_bytes(src.as_bytes());
    let mut p = StaticProvider::default();
    p.names.insert(
        kfx_script::ConstKind::Creature,
        vec!["DRAGON".into(), "IMP".into()],
    );
    e.analyze_now(&p);
    let all = e.diagnostics();
    assert!(
        all.iter().any(|d| d.code == "unfinished-arg")
            && all.iter().any(|d| d.code == "missing-close-paren"),
        "{all:?}"
    );
    e.set_caret(src.len());
    let vis = e.visible_diagnostics();
    assert!(vis.iter().all(|d| d.code != "unfinished-arg"), "{vis:?}");
    assert!(vis.iter().any(|d| d.code == "missing-close-paren"));
    e.set_suppress_at_caret(false);
    assert!(e
        .visible_diagnostics()
        .iter()
        .any(|d| d.code == "unfinished-arg"));
    // diagnostics() itself is never filtered
    assert_eq!(e.diagnostics().len(), all.len());
}

#[test]
fn map_references_carry_the_command_name() {
    let mut e = ScriptEditor::new();
    e.set_text_bytes(
        b"LEVEL_VERSION(1)\nZOOM_TO_LOCATION(PLAYER0, 3)\nIF_ACTION_POINT(4, PLAYER0)\n",
    );
    let refs = e.map_references();
    let names: Vec<&str> = refs.iter().map(|r| r.command.as_str()).collect();
    assert_eq!(names, vec!["ZOOM_TO_LOCATION", "IF_ACTION_POINT"]);
    let mut lua = ScriptEditor::with_language(Box::new(LuaLanguage::new()));
    lua.set_text_bytes(b"RegisterOnActionPointEvent(function() end, 3, PLAYER0)\nlocal x = 1\n");
    let refs = lua.map_references();
    assert!(
        refs.iter()
            .any(|r| r.command == "RegisterOnActionPointEvent"),
        "{refs:?}"
    );
    // the kfx_script level too
    let doc = kfx_script::Document::parse(b"LEVEL_VERSION(1)\nZOOM_TO_LOCATION(PLAYER0, 3)\n");
    assert_eq!(kfx_script::references(&doc)[0].command, "ZOOM_TO_LOCATION");
}

#[test]
fn workspace_makes_a_lua_draft_from_the_dk_script() {
    use kfx_script_ui::workspace::{ScriptFiles, ScriptWorkspace};
    struct F(std::cell::RefCell<std::collections::HashMap<String, Vec<u8>>>);
    impl ScriptFiles for F {
        fn read(&self, n: &str) -> Option<Vec<u8>> {
            self.0.borrow().get(n).cloned()
        }
        fn write(&self, n: &str, b: &[u8]) -> Result<(), String> {
            self.0.borrow_mut().insert(n.to_string(), b.to_vec());
            Ok(())
        }
    }
    let files = F(Default::default());
    files.0.borrow_mut().insert(
        "map00009.txt".into(),
        b"LEVEL_VERSION(1)\nSTART_MONEY(PLAYER0, 5000)\nIF(PLAYER0, \
            GAME_TURN >= 100)\n\tWIN_GAME\nENDIF\n"
            .to_vec(),
    );
    let mut ws = ScriptWorkspace::new(9);
    assert!(
        ws.create_lua_from_dk(&NullProvider).is_none(),
        "no DK script loaded yet"
    );
    ws.load(&files);
    let stats = ws.create_lua_from_dk(&NullProvider).expect("draft");
    assert_eq!((stats.commands, stats.converted), (5, 4));
    let lua = String::from_utf8(ws.editor("map00009.lua").unwrap().text_bytes()).unwrap();
    assert!(
        lua.contains("StartMoney(PLAYER0, 5000)")
            && lua.contains("RegisterTimerEvent(DkBlock_1, 100, false)"),
        "{lua}"
    );
    assert!(lua.contains("STARTING POINT, not a faithful translation"));
}
