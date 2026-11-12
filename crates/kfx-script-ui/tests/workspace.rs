//! ScriptWorkspace: tabs, dirty state, reload logic, creation, combined problems.
use egui::vec2;
use egui_kittest::Harness;
use kfx_script::{NullProvider, Severity};
use kfx_script_ui::workspace::*;
use kfx_script_ui::{MapTarget, ScriptEvent};
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Default)]
struct Fake {
    files: RefCell<HashMap<String, Vec<u8>>>,
    lib: RefCell<HashMap<String, Vec<u8>>>,
    writes: RefCell<Vec<String>>,
    fail: RefCell<bool>,
}

impl Fake {
    fn put(&self, n: &str, b: &[u8]) {
        self.files.borrow_mut().insert(n.to_string(), b.to_vec());
    }
    fn get(&self, n: &str) -> Option<Vec<u8>> {
        self.files.borrow().get(n).cloned()
    }
}

impl ScriptFiles for Fake {
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.files.borrow().get(name).cloned()
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if *self.fail.borrow() {
            return Err("disk full".into());
        }
        self.files
            .borrow_mut()
            .insert(name.to_string(), bytes.to_vec());
        self.writes.borrow_mut().push(name.to_string());
        Ok(())
    }
    fn read_library(&self, name: &str) -> Option<Vec<u8>> {
        self.lib.borrow().get(name).cloned()
    }
}

const DK: &str = "LEVEL_VERSION(1)\nIF_ACTION_POINT(2, PLAYER0)\n\tWIN_GAME\nENDIF\n";
const LUA: &str =
    "function OnGameStart()\n\tRegisterOnActionPointEvent(function() end, 5, PLAYER0)\nend\n";

fn loaded(fake: &Fake) -> ScriptWorkspace {
    fake.put("map00007.txt", DK.as_bytes());
    fake.put("map00007.lua", LUA.as_bytes());
    let mut ws = ScriptWorkspace::new(7);
    ws.load(fake);
    ws
}

#[test]
fn names_and_loading() {
    let fake = Fake::default();
    let ws = loaded(&fake);
    assert_eq!(ws.dk_name(), "map00007.txt");
    assert_eq!(ws.lua_name(), "map00007.lua");
    let t = ws.tab_infos();
    assert_eq!(t.len(), 2);
    assert!(t.iter().all(|t| t.exists && !t.dirty && !t.read_only));
    assert_eq!(
        ws.editor("map00007.lua").unwrap().text_bytes(),
        LUA.as_bytes()
    );
    assert!(!ws.is_dirty());
}

#[test]
fn dirty_state_is_per_tab_and_save_clears_it() {
    let fake = Fake::default();
    let mut ws = loaded(&fake);
    let e = ws.editor_mut("map00007.lua").unwrap();
    e.set_caret(0);
    e.insert_at_caret("-- hi\n");
    e.apply_pending();
    assert_eq!(ws.dirty_tabs(), vec!["map00007.lua".to_string()]);
    assert!(
        ws.tab_infos()
            .iter()
            .find(|t| t.name == "map00007.lua")
            .unwrap()
            .dirty
    );
    assert!(
        !ws.tab_infos()
            .iter()
            .find(|t| t.name == "map00007.txt")
            .unwrap()
            .dirty
    );
    ws.save("map00007.lua", &fake).unwrap();
    assert!(!ws.is_dirty());
    assert_eq!(
        fake.get("map00007.lua").unwrap(),
        format!("-- hi\n{LUA}").as_bytes()
    );
    assert_eq!(fake.writes.borrow().as_slice(), ["map00007.lua"]);
    // editing back to the saved text is clean, editing away is dirty again
    let e = ws.editor_mut("map00007.lua").unwrap();
    e.insert_at_caret("x");
    e.apply_pending();
    assert!(ws.is_dirty());
    // a failing write keeps the tab dirty and reports it
    *fake.fail.borrow_mut() = true;
    assert_eq!(ws.save("map00007.lua", &fake), Err("disk full".to_string()));
    assert!(ws.is_dirty());
    assert!(ws
        .take_events()
        .iter()
        .any(|e| matches!(e, WorkspaceEvent::SaveFailed { .. })));
    *fake.fail.borrow_mut() = false;
    assert!(ws.save_all(&fake).is_empty());
    assert!(!ws.is_dirty());
}

#[test]
fn external_change_reloads_a_clean_tab_silently() {
    let fake = Fake::default();
    let mut ws = loaded(&fake);
    fake.put("map00007.lua", b"-- edited elsewhere\n");
    assert_eq!(
        ws.file_changed_on_disk("map00007.lua", &fake),
        ExternalChange::Reloaded
    );
    assert_eq!(
        ws.editor("map00007.lua").unwrap().text_bytes(),
        b"-- edited elsewhere\n"
    );
    assert!(!ws.is_dirty());
    // the same bytes again: nothing to do
    assert_eq!(
        ws.file_changed_on_disk("map00007.lua", &fake),
        ExternalChange::Unchanged
    );
    // our own save is not a change either
    let e = ws.editor_mut("map00007.lua").unwrap();
    e.insert_at_caret("y");
    e.apply_pending();
    ws.save("map00007.lua", &fake).unwrap();
    assert_eq!(
        ws.file_changed_on_disk("map00007.lua", &fake),
        ExternalChange::Unchanged
    );
    assert_eq!(
        ws.file_changed_on_disk("nothing.lua", &fake),
        ExternalChange::Unchanged
    );
}

#[test]
fn external_change_with_edits_asks_and_the_answer_is_honoured() {
    let fake = Fake::default();
    let mut ws = loaded(&fake);
    let e = ws.editor_mut("map00007.txt").unwrap();
    e.set_caret(0);
    e.insert_at_caret("REM mine\n");
    e.apply_pending();
    fake.put("map00007.txt", b"LEVEL_VERSION(1)\nREM theirs\n");
    assert_eq!(
        ws.file_changed_on_disk("map00007.txt", &fake),
        ExternalChange::Conflict
    );
    // the user's text is untouched while the question is open
    assert!(ws
        .editor("map00007.txt")
        .unwrap()
        .text_bytes()
        .starts_with(b"REM mine"));
    assert_eq!(
        ws.pending_external("map00007.txt"),
        Some(&b"LEVEL_VERSION(1)\nREM theirs\n"[..])
    );
    // keep: still dirty, saving overwrites
    ws.resolve_external("map00007.txt", false);
    assert!(ws.pending_external("map00007.txt").is_none());
    assert!(ws.is_dirty());
    // another change, this time reload
    fake.put("map00007.txt", b"LEVEL_VERSION(1)\nREM third\n");
    assert_eq!(
        ws.file_changed_on_disk("map00007.txt", &fake),
        ExternalChange::Conflict
    );
    ws.resolve_external("map00007.txt", true);
    assert_eq!(
        ws.editor("map00007.txt").unwrap().text_bytes(),
        b"LEVEL_VERSION(1)\nREM third\n"
    );
    assert!(!ws.is_dirty());
}

#[test]
fn missing_files_are_created_from_templates() {
    let fake = Fake::default();
    fake.put("map00007.txt", DK.as_bytes());
    let mut ws = ScriptWorkspace::new(7);
    ws.load(&fake);
    let t = ws.tab_infos();
    assert!(t[0].exists && !t[1].exists);
    ws.create_lua_script(true, true);
    assert!(ws.tab_infos()[1].exists);
    // a created file counts as unsaved even though the editor holds its "original" text
    assert!(ws.is_dirty());
    let text = String::from_utf8(ws.editor("map00007.lua").unwrap().text_bytes()).unwrap();
    assert!(text.contains("function OnGameStart()"), "{text}");
    assert!(
        text.contains("-- DK: LEVEL_VERSION(1)"),
        "hints missing: {text}"
    );
    assert!(
        text.contains("-- DK: IF_ACTION_POINT(2, PLAYER0)"),
        "{text}"
    );
    assert!(fake.get("map00007.lua").is_none());
    ws.save_all(&fake);
    assert_eq!(fake.get("map00007.lua").unwrap(), text.as_bytes());
    assert!(!ws.is_dirty());
    // no hints
    let mut ws = ScriptWorkspace::new(7);
    ws.load(&Fake::default());
    ws.create_lua_script(false, false);
    let text = String::from_utf8(ws.editor("map00007.lua").unwrap().text_bytes()).unwrap();
    assert!(!text.contains("-- DK:"));
    // the generated script has no errors
    ws.analyze_now(&NullProvider);
    assert!(
        ws.problems()
            .iter()
            .all(|p| p.diagnostic.severity != Severity::Error),
        "{:#?}",
        ws.problems()
    );
}

#[test]
fn default_dk_script_is_valid() {
    let text = default_dk_script(12);
    let r = kfx_script::validate(&kfx_script::Document::parse(text.as_bytes()), &NullProvider);
    assert!(
        r.diagnostics.iter().all(|d| d.severity == Severity::Info),
        "{text}\n{:#?}",
        r.diagnostics
    );
    let mut ws = ScriptWorkspace::new(12);
    ws.load(&Fake::default());
    ws.create_dk_script();
    assert!(ws.is_dirty());
    assert_eq!(
        ws.editor("map00012.txt").unwrap().text_bytes(),
        text.as_bytes()
    );
}

#[test]
fn combined_problems_and_map_references_span_both_scripts() {
    let fake = Fake::default();
    fake.put(
        "map00007.txt",
        b"LEVEL_VERSION(1)\nBOGUS_COMMAND(1)\nIF_ACTION_POINT(2, PLAYER0)\nENDIF\n",
    );
    fake.put(
        "map00007.lua",
        b"function OnGameStart()\n\tBogusCall(\nend\n\
            \tRegisterOnActionPointEvent(function() end, 5, PLAYER0)\n",
    );
    let mut ws = ScriptWorkspace::new(7);
    ws.load(&fake);
    ws.analyze_now(&NullProvider);
    let p = ws.problems();
    assert!(
        p.iter()
            .any(|x| x.tab == "map00007.txt" && x.diagnostic.code == "unknown-command"),
        "{p:#?}"
    );
    assert!(
        p.iter()
            .any(|x| x.tab == "map00007.lua" && x.diagnostic.severity == Severity::Error),
        "{p:#?}"
    );
    let refs = ws.map_references();
    let aps: Vec<(&str, i32)> = refs
        .iter()
        .filter_map(|r| {
            if let MapTarget::ActionPoint(n) = r.reference.target {
                Some((r.tab.as_str(), n))
            } else {
                None
            }
        })
        .collect();
    assert!(aps.contains(&("map00007.txt", 2)), "{aps:?}");
    assert!(aps.contains(&("map00007.lua", 5)), "{aps:?}");
}

#[test]
fn library_files_open_read_only_through_the_host() {
    let fake = Fake::default();
    fake.lib.borrow_mut().insert(
        "bindings/setup.lua".into(),
        b"---@param p integer\nfunction StartMoney(p) end\n".to_vec(),
    );
    let mut ws = loaded(&fake);
    assert!(ws.open_library(
        "bindings/setup.lua",
        Some(kfx_script::Span::new(31, 41)),
        &fake
    ));
    assert_eq!(ws.active_name(), "bindings/setup.lua");
    assert!(ws.editor("bindings/setup.lua").unwrap().is_read_only());
    // a read-only tab is never dirty, refuses edits and cannot be saved
    let e = ws.editor_mut("bindings/setup.lua").unwrap();
    e.insert_at_caret("x");
    e.apply_pending();
    assert_eq!(
        e.text_bytes(),
        b"---@param p integer\nfunction StartMoney(p) end\n"
    );
    assert!(!ws.is_dirty());
    assert!(ws.save("bindings/setup.lua", &fake).is_err());
    // opening it again switches to the same tab
    ws.activate("map00007.txt");
    assert!(ws.open_library("bindings/setup.lua", None, &fake));
    assert_eq!(ws.tab_infos().len(), 3);
    // an unknown file is reported
    assert!(!ws.open_library("nope.lua", None, &fake));
    assert!(ws
        .take_events()
        .iter()
        .any(|e| matches!(e, WorkspaceEvent::OpenFailed { .. })));
    assert!(ws.notice().is_some());
    ws.close_tab("bindings/setup.lua");
    assert_eq!(ws.tab_infos().len(), 2);
    ws.close_tab("map00007.lua");
    assert_eq!(ws.tab_infos().len(), 2, "level tabs cannot be closed");
}

#[test]
fn lua_files_round_trip_byte_for_byte_through_the_workspace() {
    let fake = Fake::default();
    let src: Vec<u8> =
        b"-- caf\xe9 \xff\r\nfunction OnGameStart()\r\n\tprint(\"\xe9\")\r\nend\r\n".to_vec();
    fake.put("map00007.lua", &src);
    let mut ws = ScriptWorkspace::new(7);
    ws.load(&fake);
    assert_eq!(ws.editor("map00007.lua").unwrap().text_bytes(), src);
    // an untouched save writes the same bytes
    let e = ws.editor_mut("map00007.lua").unwrap();
    e.set_caret(0);
    e.insert_at_caret("x");
    e.apply_pending();
    let e = ws.editor_mut("map00007.lua").unwrap();
    e.apply_pending();
    ws.save("map00007.lua", &fake).unwrap();
    let mut expect = b"x".to_vec();
    expect.extend_from_slice(&src);
    assert_eq!(fake.get("map00007.lua").unwrap(), expect);
}

fn shown() -> (
    Harness<'static, (ScriptWorkspace, Fake, Vec<WorkspaceEvent>)>,
    (),
) {
    let fake = Fake::default();
    fake.put("map00007.txt", DK.as_bytes());
    fake.put("map00007.lua", LUA.as_bytes());
    let mut ws = ScriptWorkspace::new(7);
    ws.load(&fake);
    let h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1000.0, 600.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, s: &mut (ScriptWorkspace, Fake, Vec<WorkspaceEvent>)| {
                let ev = s.0.show(ui, &NullProvider, &s.1);
                s.2.extend(ev);
            },
            (ws, fake, vec![]),
        );
    (h, ())
}

#[test]
fn the_workspace_renders_switches_tabs_and_saves_with_ctrl_s() {
    let (mut h, _) = shown();
    h.run_steps(4);
    assert_eq!(h.state().0.active_name(), "map00007.txt");
    h.state_mut().0.activate("map00007.lua");
    h.run_steps(4);
    let e = h.state_mut().0.editor_mut("map00007.lua").unwrap();
    e.set_caret(0);
    e.insert_at_caret("-- x\n");
    h.run_steps(4);
    assert!(h.state().0.is_dirty());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run_steps(4);
    assert!(!h.state().0.is_dirty());
    assert!(h
        .state()
        .1
        .get("map00007.lua")
        .unwrap()
        .starts_with(b"-- x\n"));
    assert!(
        h.state()
            .2
            .iter()
            .any(|e| matches!(e, WorkspaceEvent::Saved { tab } if tab == "map00007.lua")),
        "{:?}",
        h.state().2
    );
}

#[test]
fn definition_events_open_library_tabs() {
    let (mut h, _) = shown();
    h.state().1.lib.borrow_mut().insert(
        "bindings/setup.lua".into(),
        b"function StartMoney(p) end\n".to_vec(),
    );
    h.state_mut().0.activate("map00007.lua");
    h.run_steps(3);
    let e = h.state_mut().0.editor_mut("map00007.lua").unwrap();
    e.set_caret(0);
    e.insert_at_caret("StartMoney(PLAYER0, 1)\n");
    h.run_steps(4);
    let e = h.state_mut().0.editor_mut("map00007.lua").unwrap();
    e.set_caret(3);
    assert!(e.goto_definition());
    h.run_steps(4);
    assert_eq!(h.state().0.active_name(), "bindings/setup.lua");
    assert!(h
        .state()
        .0
        .editor("bindings/setup.lua")
        .unwrap()
        .is_read_only());
    let _ = ScriptEvent::Changed;
}
