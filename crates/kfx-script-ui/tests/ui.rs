//! Headless egui tests of the widget (no GPU needed).
use egui::{vec2, Event, Key, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use kfx_script::StaticProvider;
use kfx_script_ui::{MapTarget, PickKind, ScriptEditor, ScriptEvent, SidePanel};

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
fn typing_opens_completion_and_tab_accepts() {
    let mut h = harness(b"LEVEL_VERSION(1)\n");
    focus_end(&mut h);
    type_text(&mut h, "ADD_CR");
    assert_eq!(
        h.state().ed.completion_labels(),
        vec!["ADD_CREATURE_TO_LEVEL", "ADD_CREATURE_TO_POOL"]
    );
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::NONE, Key::ArrowUp);
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(text(&h), "LEVEL_VERSION(1)\nADD_CREATURE_TO_LEVEL(");
    // ... and the popup moved on to the first argument (a player), without inserting a newline
    assert!(
        h.state()
            .ed
            .completion_labels()
            .iter()
            .any(|l| l == "PLAYER0"),
        "{:?}",
        h.state().ed.completion_labels()
    );
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert!(
        text(&h).starts_with("LEVEL_VERSION(1)\nADD_CREATURE_TO_LEVEL(")
            && !text(&h).ends_with('('),
        "{}",
        text(&h)
    );
}

#[test]
fn escape_closes_popup_without_editing() {
    let mut h = harness(b"");
    focus_end(&mut h);
    type_text(&mut h, "ADD_CR");
    assert!(!h.state().ed.completion_labels().is_empty());
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state().ed.completion_labels().is_empty());
    assert_eq!(text(&h), "ADD_CR");
}

#[test]
fn ctrl_space_opens_completion() {
    let mut h = harness(b"");
    focus_end(&mut h);
    key(&mut h, Modifiers::COMMAND, Key::Space);
    assert!(h
        .state()
        .ed
        .completion_labels()
        .iter()
        .any(|l| l == "WIN_GAME"));
}

#[test]
fn enter_auto_indents_inside_if_and_dedents_endif() {
    let mut h = harness(b"IF(PLAYER0, FLAG0 == 1)");
    focus_end(&mut h);
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(text(&h), "IF(PLAYER0, FLAG0 == 1)\n\t");
    type_text(&mut h, "WIN_GAME");
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(text(&h), "IF(PLAYER0, FLAG0 == 1)\n\tWIN_GAME\n\t");
    type_text(&mut h, "ENDIF");
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(text(&h), "IF(PLAYER0, FLAG0 == 1)\n\tWIN_GAME\nENDIF\n");
}

#[test]
fn undo_and_redo_work_with_the_highlighting_layouter() {
    let mut h = harness(b"LEVEL_VERSION(1)\n");
    focus_end(&mut h);
    h.step(); // let the undoer record the initial state
    type_text(&mut h, "WIN_GAME");
    assert!(text(&h).ends_with("WIN_GAME"));
    // a pause makes the typing one undo step
    for _ in 0..40 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), "LEVEL_VERSION(1)\n", "undo restored the text");
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    assert!(text(&h).ends_with("WIN_GAME"), "redo: {:?}", text(&h));
}

#[test]
fn ctrl_slash_toggles_comment_in_the_ui() {
    let mut h = harness(b"WIN_GAME\n");
    h.state_mut().ed.goto_line(1);
    h.run_steps(3);
    key(&mut h, Modifiers::COMMAND, Key::Slash);
    assert_eq!(text(&h), "REM WIN_GAME\n");
    key(&mut h, Modifiers::COMMAND, Key::Slash);
    assert_eq!(text(&h), "WIN_GAME\n");
}

#[test]
fn tab_moves_between_arguments_after_inserting_a_command() {
    let mut h = harness(b"\n");
    h.state_mut().ed.goto_line(1);
    h.run_steps(2);
    h.state_mut().ed.insert_command("ADD_CREATURE_TO_LEVEL");
    h.run_steps(3);
    assert_eq!(
        text(&h),
        "ADD_CREATURE_TO_LEVEL(player, creature, location, copies, level, gold)\n".to_string()
    );
    let sel = h.state_mut().ed.selection_span();
    assert_eq!(&text(&h)[sel.start..sel.end], "player");
    type_text(&mut h, "PLAYER0");
    assert!(h.state().ed.completion_labels().is_empty());
    key(&mut h, Modifiers::NONE, Key::Tab);
    let sel = h.state_mut().ed.selection_span();
    assert_eq!(&text(&h)[sel.start..sel.end], "creature");
    key(&mut h, Modifiers::SHIFT, Key::Tab);
    let sel = h.state_mut().ed.selection_span();
    assert_eq!(&text(&h)[sel.start..sel.end], "PLAYER0");
}

#[test]
fn find_bar_selects_and_steps() {
    let mut h = harness(b"A(1)\nB(1)\nA(2)\n");
    h.state_mut().ed.goto_line(1);
    h.run_steps(3);
    key(&mut h, Modifiers::COMMAND, Key::F);
    type_text(&mut h, "a(");
    assert_eq!(h.state_mut().ed.find_matches().len(), 2);
    let s = h.state_mut().ed.selection_span();
    assert_eq!((s.start, s.end), (0, 2));
    key(&mut h, Modifiers::NONE, Key::F3);
    let s = h.state_mut().ed.selection_span();
    assert_eq!((s.start, s.end), (10, 12));
    key(&mut h, Modifiers::NONE, Key::Escape);
}

#[test]
fn changed_event_on_typing_but_not_on_load() {
    let mut h = harness(b"A\n");
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);
    focus_end(&mut h);
    type_text(&mut h, "X");
    assert!(h.state().events.contains(&ScriptEvent::Changed));
    assert!(h.state().ed.is_dirty());
}

#[test]
fn pick_from_map_and_jump_events() {
    let mut h =
        harness(b"LEVEL_VERSION(1)\nADD_CREATURE_TO_LEVEL(PLAYER_GOOD, DRAGON, 3, 1, 1, 0)\n");
    // caret inside the action point argument
    let off = h.state().ed.test_offset_of(b"3, 1");
    h.state_mut()
        .ed
        .reveal_span(kfx_script::Span::new(off, off));
    h.run_steps(4);
    assert_eq!(
        h.state_mut().ed.pick_kind_at_caret(),
        Some(PickKind::Location)
    );
    h.get_by_label("Pick from map").click();
    h.run_steps(3);
    assert!(
        h.state().events.contains(&ScriptEvent::RequestPickFromMap {
            kind: PickKind::Location
        }),
        "{:?}",
        h.state().events
    );
    // the host answers: the argument is rewritten
    h.state_mut()
        .ed
        .insert_pick(kfx_script_ui::PickValue::ActionPoint(9));
    h.run_steps(3);
    assert!(text(&h).contains("DRAGON, 9, 1"), "{}", text(&h));
    // "Show on map"
    h.get_by_label("Show on map").click();
    h.run_steps(3);
    assert!(
        h.state()
            .events
            .contains(&ScriptEvent::JumpToMapLocation(MapTarget::ActionPoint(9))),
        "{:?}",
        h.state().events
    );
    let _ = (PointerButton::Primary, Event::Copy);
}

#[test]
fn non_utf8_and_crlf_scripts_render_edit_and_save_cleanly() {
    let mut src: Vec<u8> = b"REM caf\xe9 \x80\x9f \xff\r\nLEVEL_VERSION(1)\r\n".to_vec();
    src.extend((0x80..=0xffu8).chain(b"\r\n".iter().copied()));
    let mut h = harness(&src);
    assert_eq!(text_bytes(&h), src);
    focus_end(&mut h);
    type_text(&mut h, "WIN_GAME");
    key(&mut h, Modifiers::NONE, Key::Enter);
    let out = text_bytes(&h);
    assert!(out.starts_with(&src));
    assert!(
        out.ends_with(b"WIN_GAME\r\n"),
        "{:?}",
        String::from_utf8_lossy(&out)
    );
    assert!(
        !out.windows(2).any(|w| w[1] == b'\n' && w[0] != b'\r'),
        "every LF must stay preceded by CR"
    );
}

fn text_bytes(h: &Harness<'static, State>) -> Vec<u8> {
    h.state().ed.text_bytes()
}

/// Ctrl as winit reports it on Windows/Linux: both `ctrl` and `command` set.
const CTRL: Modifiers = Modifiers {
    alt: false,
    ctrl: true,
    shift: false,
    mac_cmd: false,
    command: true,
};
const CTRL_SHIFT: Modifiers = Modifiers {
    alt: false,
    ctrl: true,
    shift: true,
    mac_cmd: false,
    command: true,
};

#[test]
fn ctrl_home_and_ctrl_end_move_to_document_start_and_end() {
    let mut script = String::from("LEVEL_VERSION(1)\n");
    for i in 0..80 {
        script.push_str(&format!("SET_FLAG(PLAYER0, FLAG0, {i})\n"));
    }
    script.push_str("REM last line");
    let n = script.len();
    let mut h = harness(script.as_bytes());
    h.state_mut().ed.goto_line(5);
    h.run_steps(3);
    key(&mut h, CTRL, Key::End);
    assert_eq!(
        h.state_mut().ed.selection_span(),
        kfx_script::Span::new(n, n),
        "Ctrl+End"
    );
    assert_eq!(h.state_mut().ed.caret_line_col().0, 82);
    // and the view scrolled there
    h.run_steps(3);
    let p = h.state().ed.test_char_pos(n).expect("caret laid out");
    assert!(p.y > 0.0 && p.y < 500.0, "last line visible: {p:?}");
    key(&mut h, CTRL, Key::Home);
    assert_eq!(
        h.state_mut().ed.selection_span(),
        kfx_script::Span::new(0, 0),
        "Ctrl+Home"
    );
    // with Shift the selection grows from the caret
    h.state_mut().ed.goto_line(2);
    h.run_steps(3);
    let line2 = h.state_mut().ed.caret_file_offset();
    key(&mut h, CTRL_SHIFT, Key::End);
    assert_eq!(
        h.state_mut().ed.selection_span(),
        kfx_script::Span::new(line2, n),
        "Ctrl+Shift+End"
    );
    key(&mut h, CTRL_SHIFT, Key::Home);
    assert_eq!(
        h.state_mut().ed.selection_span(),
        kfx_script::Span::new(0, line2),
        "Ctrl+Shift+Home"
    );
    // the command modifier alone (macOS Cmd, or a backend that only sets `command`) works too
    key(&mut h, Modifiers::COMMAND, Key::End);
    assert_eq!(
        h.state_mut().ed.selection_span(),
        kfx_script::Span::new(n, n),
        "Cmd+End"
    );
}

struct WinState {
    ed: ScriptEditor,
    host: StaticProvider,
    other_on_top: bool,
    script_win: Option<(egui::LayerId, egui::Rect)>,
    other_layer: Option<egui::LayerId>,
}

/// The signature help belongs to the editor's own window: same layer, inside its text view,
/// gone when the editor loses focus. It must never be a foreground area over other windows.
#[test]
fn signature_help_stays_in_the_editor_window_layer() {
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(b"LEVEL_VERSION(1)\n\n\n\n\nSET_FLAG(PLAYER0, ");
    ed.set_side_panel(SidePanel::None);
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1000.0, 600.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, s: &mut WinState| {
                let r = egui::Window::new("Script")
                    .default_pos([10.0, 10.0])
                    .default_size([700.0, 400.0])
                    .show(ui.ctx(), |ui| {
                        s.ed.show(ui, &s.host);
                        ui.layer_id()
                    });
                if let Some(r) = r {
                    s.script_win = r.inner.map(|l| (l, r.response.rect));
                }
                if s.other_on_top {
                    let r = egui::Window::new("Other")
                        .default_pos([60.0, 60.0])
                        .default_size([300.0, 200.0])
                        .show(ui.ctx(), |ui| {
                            ui.label("another window");
                            ui.layer_id()
                        });
                    s.other_layer = r.and_then(|r| r.inner);
                }
            },
            WinState {
                ed,
                host: StaticProvider::default(),
                other_on_top: false,
                script_win: None,
                other_layer: None,
            },
        );
    h.run_steps(3);
    let n = h.state().ed.text_bytes().len();
    h.state_mut().ed.set_caret(n);
    h.state_mut().ed.reveal_span(kfx_script::Span::new(n, n));
    h.run_steps(4);
    type_text_w(&mut h, "F");
    let (layer, rect) = h
        .state()
        .ed
        .signature_popup_for_test()
        .expect("signature help shown while typing in a call");
    let (script_layer, win) = h.state().script_win.expect("script window shown");
    assert_eq!(script_layer.order, egui::Order::Middle);
    assert_eq!(
        layer, script_layer,
        "drawn on the editor window's layer, not in a foreground area"
    );
    assert!(
        win.contains_rect(rect),
        "inside the editor window: {rect:?} vs {win:?}"
    );
    // another window raised above: the popup stays on the editor layer, so the other window covers
    // it
    h.state_mut().other_on_top = true;
    h.run_steps(3);
    let other = h.state().other_layer.expect("other window shown");
    h.ctx.move_to_top(other);
    h.run_steps(3);
    let order = h.ctx.memory(|m| m.layer_ids().collect::<Vec<_>>());
    let pos = |l: egui::LayerId| order.iter().position(|x| *x == l);
    assert!(pos(other) > pos(script_layer), "{order:?}");
    if let Some((l, _)) = h.state().ed.signature_popup_for_test() {
        assert_eq!(l, script_layer);
    }
    // focus moves away from the editor: no signature help at all
    h.ctx
        .memory_mut(|m| m.surrender_focus(m.focused().unwrap_or(egui::Id::NULL)));
    h.run_steps(3);
    assert!(
        h.state().ed.signature_popup_for_test().is_none(),
        "hidden when the editor is not focused"
    );
}

fn type_text_w(h: &mut Harness<'static, WinState>, text: &str) {
    for ch in text.chars() {
        h.event(Event::Text(ch.to_string()));
        h.run_steps(2);
    }
}
