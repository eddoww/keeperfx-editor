//! Headless egui tests of the Lua editor (typing, auto-close, indentation, snippets).
use egui::{vec2, Event, Key, Modifiers};
use egui_kittest::Harness;
use kfx_script::StaticProvider;
use kfx_script_ui::{LuaLanguage, ScriptEditor, ScriptEvent, SidePanel};

struct State {
    ed: ScriptEditor,
    host: StaticProvider,
    events: Vec<ScriptEvent>,
}

fn harness(script: &str) -> Harness<'static, State> {
    let mut ed = ScriptEditor::with_language(Box::new(LuaLanguage::new()));
    ed.set_text_bytes(script.as_bytes());
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

fn type_text(h: &mut Harness<'static, State>, text: &str) {
    for ch in text.chars() {
        h.event(Event::Text(ch.to_string()));
        h.run_steps(2);
    }
}

fn key(h: &mut Harness<'static, State>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run_steps(3);
}

fn text(h: &Harness<'static, State>) -> String {
    h.state()
        .ed
        .text_bytes()
        .iter()
        .map(|&b| b as char)
        .collect()
}

fn focus_end(h: &mut Harness<'static, State>) {
    let n = h.state().ed.text_bytes().len();
    h.state_mut().ed.set_caret(n);
    h.state_mut().ed.reveal_span(kfx_script::Span::new(n, n));
    h.run_steps(3);
}

#[test]
fn brackets_and_quotes_close_and_step_over() {
    let mut h = harness("");
    focus_end(&mut h);
    type_text(&mut h, "print(");
    assert_eq!(text(&h), "print()");
    type_text(&mut h, "\"hi");
    assert_eq!(text(&h), "print(\"hi\")");
    // typing the closing quote and the parenthesis steps over them
    type_text(&mut h, "\")");
    assert_eq!(text(&h), "print(\"hi\")");
    assert_eq!(h.state_mut().ed.caret_file_offset(), 11);
    type_text(&mut h, " x");
    assert_eq!(text(&h), "print(\"hi\") x");
}

#[test]
fn backspace_removes_both_halves_of_an_empty_pair() {
    let mut h = harness("");
    focus_end(&mut h);
    type_text(&mut h, "f({");
    assert_eq!(text(&h), "f({})");
    key(&mut h, Modifiers::NONE, Key::Backspace);
    assert_eq!(text(&h), "f()");
    key(&mut h, Modifiers::NONE, Key::Backspace);
    assert_eq!(text(&h), "f");
}

#[test]
fn apostrophes_in_words_are_not_closed() {
    let mut h = harness("");
    focus_end(&mut h);
    type_text(&mut h, "-- don't");
    assert_eq!(text(&h), "-- don't");
}

#[test]
fn enter_indents_and_end_dedents() {
    let mut h = harness("");
    focus_end(&mut h);
    type_text(&mut h, "function OnGameStart()");
    key(&mut h, Modifiers::NONE, Key::Enter);
    type_text(&mut h, "if x then");
    key(&mut h, Modifiers::NONE, Key::Enter);
    type_text(&mut h, "foo()");
    key(&mut h, Modifiers::NONE, Key::Enter);
    type_text(&mut h, "end");
    // `end` moved back to the `if` level
    assert_eq!(
        text(&h),
        "function OnGameStart()\n\tif x then\n\t\tfoo()\n\tend"
    );
    key(&mut h, Modifiers::NONE, Key::Enter);
    type_text(&mut h, "end");
    assert_eq!(
        text(&h),
        "function OnGameStart()\n\tif x then\n\t\tfoo()\n\tend\nend"
    );
}

#[test]
fn else_and_until_dedent_too() {
    let mut h = harness("");
    focus_end(&mut h);
    type_text(&mut h, "if x then");
    key(&mut h, Modifiers::NONE, Key::Enter);
    type_text(&mut h, "a()");
    key(&mut h, Modifiers::NONE, Key::Enter);
    type_text(&mut h, "else");
    assert_eq!(text(&h), "if x then\n\ta()\nelse");
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(text(&h).ends_with("else\n\t"), "{:?}", text(&h));
}

#[test]
fn typing_a_function_name_offers_completion_and_tab_writes_the_call() {
    let mut h = harness("");
    focus_end(&mut h);
    type_text(&mut h, "StartMo");
    assert!(
        h.state()
            .ed
            .completion_labels()
            .iter()
            .any(|l| l == "StartMoney"),
        "{:?}",
        h.state().ed.completion_labels()
    );
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(text(&h), "StartMoney()");
    assert_eq!(h.state_mut().ed.caret_file_offset(), 11);
    // signature help is up for the new call
    assert_eq!(
        h.state_mut().ed.signature_help_at_caret().unwrap().command,
        "StartMoney"
    );
}

#[test]
fn dot_and_colon_open_member_completion() {
    let mut h = harness("function OnCreatureDeath(unit)\n\tunit");
    focus_end(&mut h);
    type_text(&mut h, ":");
    assert!(!h.state().ed.completion_labels().is_empty());
    assert!(
        h.state()
            .ed
            .completion_labels()
            .iter()
            .any(|l| l.to_lowercase().contains("level")),
        "{:?}",
        h.state().ed.completion_labels()
    );
    key(&mut h, Modifiers::NONE, Key::Escape);
}

#[test]
fn quote_inside_a_call_offers_alias_values() {
    let mut h = harness("function OnGameStart()\n\tAddCreatureToLevel(PLAYER_GOOD, ");
    h.state_mut().host.names.insert(
        kfx_script::ConstKind::Creature,
        ["DRAGON", "KNIGHT"].iter().map(|s| s.to_string()).collect(),
    );
    h.state_mut().ed.invalidate();
    focus_end(&mut h);
    type_text(&mut h, "\"");
    assert_eq!(
        text(&h).lines().last().unwrap(),
        "\tAddCreatureToLevel(PLAYER_GOOD, \"\""
    );
    assert!(
        h.state()
            .ed
            .completion_labels()
            .iter()
            .any(|l| l == "KNIGHT"),
        "{:?}",
        h.state().ed.completion_labels()
    );
    type_text(&mut h, "KN");
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(
        text(&h).lines().last().unwrap(),
        "\tAddCreatureToLevel(PLAYER_GOOD, \"KNIGHT\""
    );
}

#[test]
fn tab_walks_snippet_placeholders_and_ends_after_the_last() {
    let mut h = harness("function OnGameStart()\n\t\nend\n");
    h.state_mut().ed.goto_line(2);
    h.run_steps(3);
    key(&mut h, Modifiers::NONE, Key::End);
    h.state_mut()
        .ed
        .insert_snippet("StartMoney(${1:PLAYER0}, ${2:3000})\nMaxCreatures(PLAYER0, ${3:20})$0");
    h.run_steps(3);
    assert_eq!(
        text(&h),
        "function OnGameStart()\n\tStartMoney(PLAYER0, 3000)\n\tMaxCreatures(PLAYER0, 20)\nend\n"
    );
    let sel = |h: &mut Harness<'static, State>| {
        let s = h.state_mut().ed.selection_span();
        text(h)[s.start..s.end].to_string()
    };
    assert_eq!(sel(&mut h), "PLAYER0");
    type_text(&mut h, "PLAYER1");
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(sel(&mut h), "3000");
    type_text(&mut h, "5000");
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(sel(&mut h), "20");
    key(&mut h, Modifiers::SHIFT, Key::Tab);
    assert_eq!(sel(&mut h), "5000");
    key(&mut h, Modifiers::NONE, Key::Tab);
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert!(!h.state().ed.snippet_active());
    assert_eq!(
        text(&h),
        "function OnGameStart()\n\tStartMoney(PLAYER1, 5000)\n\tMaxCreatures(PLAYER0, 20)\nend\n"
    );
    // the caret sits at the end of the snippet
    let c = h.state_mut().ed.caret_file_offset();
    assert_eq!(
        &text(&h)[..c],
        "function OnGameStart()\n\tStartMoney(PLAYER1, 5000)\n\tMaxCreatures(PLAYER0, 20)"
    );
}

#[test]
fn moving_the_caret_away_ends_the_snippet_session() {
    let mut h = harness("\n");
    h.state_mut().ed.goto_line(1);
    h.run_steps(3);
    h.state_mut().ed.insert_snippet("A(${1:x}, ${2:y})");
    h.run_steps(3);
    assert!(h.state().ed.snippet_active());
    h.state_mut().ed.goto_line(2);
    h.run_steps(3);
    assert!(!h.state().ed.snippet_active());
}

#[test]
fn f12_selects_a_definition_in_the_file() {
    let mut h = harness("function Helper()\nend\nHelper()\n");
    let off = h.state().ed.test_offset_of(b"Helper()\n");
    let call = "function Helper()\nend\n".len() + 2;
    let _ = off;
    h.state_mut()
        .ed
        .reveal_span(kfx_script::Span::new(call, call));
    h.run_steps(4);
    key(&mut h, Modifiers::NONE, Key::F12);
    let s = h.state_mut().ed.selection_span();
    assert_eq!((s.start, s.end), (9, 15));
}

#[test]
fn f12_on_a_library_function_tells_the_host() {
    let mut h = harness("StartMoney(PLAYER0, 1)\n");
    h.state_mut().ed.reveal_span(kfx_script::Span::new(3, 3));
    h.run_steps(4);
    key(&mut h, Modifiers::NONE, Key::F12);
    assert!(h.state().events.iter().any(|e| matches!(e, ScriptEvent::GotoDefinition { file, .. } if file == "bindings/setup.lua")), "{:?}", h.state().events);
}

#[test]
fn ctrl_slash_toggles_a_lua_comment() {
    let mut h = harness("x = 1\n");
    h.state_mut().ed.goto_line(1);
    h.run_steps(3);
    key(&mut h, Modifiers::COMMAND, Key::Slash);
    assert_eq!(text(&h), "-- x = 1\n");
}

#[test]
fn the_light_theme_and_font_size_apply() {
    let mut h = harness("function OnGameStart()\nend\n");
    h.state_mut().ed.set_light(true);
    h.state_mut().ed.set_font_size(18.0);
    h.run_steps(4);
    assert!(!h.state().ed.theme().is_dark);
    assert_eq!(h.state().ed.font_size(), 18.0);
    h.state_mut().ed.set_font_size(100.0);
    assert_eq!(h.state().ed.font_size(), 32.0);
}

/// After a long line scrolled the text sideways, a caret back at the start of a line scrolls
/// the text back: the line-number gutter is drawn over the left edge of the text, so a caret
/// in the first columns must not stay hidden under it.
#[test]
fn caret_at_line_start_scrolls_back_from_under_the_gutter() {
    let mut h = harness("x = 1\n");
    focus_end(&mut h);
    let x0 = h.state().ed.galley_screen_pos().expect("geometry").x;
    type_text(
        &mut h,
        "local s = \"a long line that is much wider than the editor \
            so that it scrolls sideways, and it keeps going on\"",
    );
    let scrolled = h.state().ed.galley_screen_pos().unwrap().x;
    assert!(
        scrolled < x0 - 50.0,
        "typing to the right scrolls: {x0} -> {scrolled}"
    );
    key(&mut h, Modifiers::NONE, Key::Home);
    h.run_steps(3);
    let back = h.state().ed.galley_screen_pos().unwrap().x;
    assert!(
        (back - x0).abs() < 1.0,
        "Home scrolls back to the start: {x0} -> {back}"
    );
}

/// The same after Enter at the end of a long line (the editor moves the caret itself there, for
/// the indentation).
#[test]
fn enter_after_a_long_line_scrolls_back() {
    let mut h = harness("x = 1\n");
    focus_end(&mut h);
    let x0 = h.state().ed.galley_screen_pos().expect("geometry").x;
    type_text(
        &mut h,
        "local s = \"a long line that is much wider than the editor \
            so that it scrolls sideways, and it keeps going on\"",
    );
    assert!(h.state().ed.galley_screen_pos().unwrap().x < x0 - 50.0);
    key(&mut h, Modifiers::NONE, Key::Enter);
    h.run_steps(3);
    let back = h.state().ed.galley_screen_pos().unwrap().x;
    assert!(
        (back - x0).abs() < 1.0,
        "Enter scrolls back to the start: {x0} -> {back}"
    );
}
