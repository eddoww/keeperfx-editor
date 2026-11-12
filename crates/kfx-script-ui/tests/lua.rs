//! Lua language behaviour through the editor (no UI).
use kfx_script::lua::ApiIndex;
use kfx_script::{NullProvider, Severity, StaticProvider};
use kfx_script_ui::*;
use std::sync::Arc;

fn lua_editor(text: &[u8]) -> ScriptEditor {
    let mut e = ScriptEditor::with_language(Box::new(LuaLanguage::new()));
    e.set_text_bytes(text);
    e
}

fn at_end(e: &mut ScriptEditor) {
    let n = e.text_bytes().len();
    e.set_caret(n);
}

fn labels(e: &mut ScriptEditor, host: &dyn kfx_script::ConstantProvider) -> Vec<String> {
    e.completions_at_caret(host)
        .into_iter()
        .map(|c| c.label)
        .collect()
}

fn text(e: &ScriptEditor) -> String {
    e.text_bytes().iter().map(|&b| b as char).collect()
}

#[test]
fn highlighting_marks_api_user_and_annotation_tokens() {
    let src = "---@param n integer\nfunction Helper(n) return n end\n\
        function OnGameStart()\n\tStartMoney(PLAYER0, 3000)\n\tHelper(1)\n\tBogus()\nend\n";
    let a = LuaLanguage::new().analyze(src.as_bytes());
    let class_of = |needle: &str| {
        let at = src.find(needle).unwrap();
        a.tokens()
            .iter()
            .find(|(s, _)| s.start == at)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("no token at {needle}"))
    };
    assert_eq!(class_of("function Helper"), HlClass::Keyword);
    assert_eq!(class_of("StartMoney"), HlClass::ApiFunction);
    assert_eq!(class_of("PLAYER0"), HlClass::ApiGlobal);
    assert_eq!(class_of("Helper(1)"), HlClass::UserGlobal);
    assert_eq!(class_of("Bogus"), HlClass::Unknown);
    assert_eq!(class_of("3000"), HlClass::Number);
    // annotation: the tag and the type get their own classes
    assert!(a.tokens().iter().any(|(_, c)| *c == HlClass::Annotation));
    assert!(a
        .tokens()
        .iter()
        .any(|(_, c)| *c == HlClass::AnnotationType));
    // spans are ordered and non-overlapping
    let t = a.tokens();
    assert!(t.windows(2).all(|w| w[0].0.end <= w[1].0.start), "{t:?}");
}

#[test]
fn diagnostics_cover_syntax_unknown_literal_and_local_callback() {
    let src = "function OnGameStart()\n\
        \tAddCreatureToLevel(PLAYER_GOOD, \"DRAGONN\", 1, 1, 0)\n\tlocal function cb() \
        end\n\tRegisterTimerEvent(cb, 100, false)\n\tif x then\nend\n\tlocal = 3\nend\n";
    let mut e = lua_editor(src.as_bytes());
    let mut host = StaticProvider::default();
    host.names.insert(
        kfx_script::ConstKind::Creature,
        ["DRAGON", "KNIGHT"].iter().map(|s| s.to_string()).collect(),
    );
    e.analyze_now(&host);
    let d = e.diagnostics();
    let codes: Vec<&str> = d.iter().map(|x| x.code).collect();
    assert!(
        d.iter().any(|x| x.severity == Severity::Error
            && x.message.to_lowercase().contains("syntax")
            || x.code.contains("syntax")),
        "{d:#?}"
    );
    assert!(
        d.iter()
            .any(|x| x.code.contains("creature") || x.message.contains("DRAGONN")),
        "{codes:?} {d:#?}"
    );
    assert!(
        d.iter()
            .any(|x| x.code.contains("local") || x.message.contains("local function")),
        "{codes:?} {d:#?}"
    );
    // every span is inside the text
    let n = src.len();
    assert!(d.iter().all(|x| x.span.end <= n), "{d:#?}");
    // the unknown creature squiggle sits on the literal
    let lit = src.find("\"DRAGONN\"").unwrap();
    assert!(
        d.iter()
            .any(|x| x.span.start >= lit && x.span.end <= lit + 9),
        "{d:#?}"
    );
}

#[test]
fn spans_survive_non_utf8_bytes() {
    // Latin-1 bytes before the code shift UTF-8 offsets; spans must still be byte offsets
    let mut src = b"-- caf\xe9 \xff\xfe\nlocal s = \"\xe9\xe9\"\nBogusCall()\n".to_vec();
    let mut e = lua_editor(&src);
    e.analyze_now(&NullProvider);
    let at = src.windows(9).position(|w| w == b"BogusCall").unwrap();
    let d = e.diagnostics();
    assert!(
        d.iter().any(|x| x.span.start == at && x.span.end == at + 9),
        "{d:#?} at {at}"
    );
    // the file bytes come back unchanged, also after an edit elsewhere
    assert_eq!(e.text_bytes(), src);
    e.set_caret(src.len());
    e.insert_at_caret("x = 1\n");
    e.apply_pending();
    src.extend_from_slice(b"x = 1\n");
    assert_eq!(e.text_bytes(), src);
    // highlighting works across the non-ASCII region
    let a = LuaLanguage::new().analyze(&src);
    assert!(
        a.tokens()
            .iter()
            .any(|(s, c)| s.start == at && *c == HlClass::Unknown),
        "{:?}",
        a.tokens()
    );
}

#[test]
fn member_completion_after_colon_and_dot() {
    let mut e = lua_editor(b"PLAYER0:");
    at_end(&mut e);
    let l = labels(&mut e, &NullProvider);
    assert!(!l.is_empty(), "no members after `PLAYER0:`");
    let mut e = lua_editor(b"function OnCreatureDeath(unit)\n\tlocal c = unit\n\tc:");
    at_end(&mut e);
    let l = labels(&mut e, &NullProvider);
    assert!(
        l.iter().any(|x| x.to_lowercase().contains("level")),
        "{l:?}"
    );
    // a partial word after the dot filters
    let mut e = lua_editor(b"function OnCreatureDeath(unit)\n\tunit.mo");
    at_end(&mut e);
    let l = labels(&mut e, &NullProvider);
    assert!(
        l.iter().all(|x| x.to_lowercase().starts_with("mo")),
        "{l:?}"
    );
}

#[test]
fn accepting_a_method_completion_writes_the_call() {
    let mut e = lua_editor(b"function OnCreatureDeath(unit)\n\tunit:lev");
    at_end(&mut e);
    e.refresh_completions_for_test(&NullProvider);
    let all = e.completion_labels();
    assert!(!all.is_empty(), "no popup");
    let i = all
        .iter()
        .position(|l| l.to_lowercase().starts_with("level"))
        .unwrap();
    e.accept_completion(i);
    e.apply_pending();
    let t = text(&e);
    assert!(
        t.contains("unit:level") && t.trim_end().ends_with(')'),
        "{t}"
    );
    // the caret is between the parentheses when the method takes arguments, after them when not
    let caret = e.caret_file_offset();
    assert!(
        t.as_bytes()[caret - 1] == b'(' || t.as_bytes()[caret - 1] == b')',
        "{t} {caret}"
    );
}

#[test]
fn global_function_completion_and_acceptance() {
    let mut e = lua_editor(b"Start");
    at_end(&mut e);
    e.refresh_completions_for_test(&NullProvider);
    let l = e.completion_labels();
    let i = l
        .iter()
        .position(|x| x == "StartMoney")
        .unwrap_or_else(|| panic!("{l:?}"));
    e.accept_completion(i);
    e.apply_pending();
    assert_eq!(text(&e), "StartMoney()");
    assert_eq!(e.caret_file_offset(), "StartMoney(".len());
}

#[test]
fn alias_values_complete_inside_a_string_argument() {
    let mut e = lua_editor(b"AddCreatureToLevel(PLAYER_GOOD, \"");
    at_end(&mut e);
    let mut host = StaticProvider::default();
    host.names.insert(
        kfx_script::ConstKind::Creature,
        ["DRAGON", "KNIGHT", "WIZARD"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    );
    let l = labels(&mut e, &host);
    assert!(l.iter().any(|x| x == "KNIGHT"), "{l:?}");
    // with an auto-closed quote and a partial value
    let mut e = lua_editor(b"AddCreatureToLevel(PLAYER_GOOD, \"KN\")");
    e.set_caret("AddCreatureToLevel(PLAYER_GOOD, \"KN".len());
    e.refresh_completions_for_test(&host);
    let l = e.completion_labels();
    let i = l
        .iter()
        .position(|x| x == "KNIGHT")
        .unwrap_or_else(|| panic!("{l:?}"));
    e.accept_completion(i);
    e.apply_pending();
    assert_eq!(text(&e), "AddCreatureToLevel(PLAYER_GOOD, \"KNIGHT\")");
}

#[test]
fn signature_help_names_the_active_parameter() {
    let mut e = lua_editor(b"AddCreatureToLevel(PLAYER_GOOD, \"KNIGHT\", ");
    at_end(&mut e);
    let s = e.signature_help_at_caret().expect("signature help");
    assert_eq!(s.command, "AddCreatureToLevel");
    assert_eq!(s.active, 2);
    assert_eq!(s.params[2].name, "location");
    let (a, b) = s.params[2].label_range;
    assert!(s.label[a..b].starts_with("location"));
}

#[test]
fn go_to_definition_in_the_same_file() {
    let src = "function Helper()\nend\nHelper()\n";
    let mut e = lua_editor(src.as_bytes());
    e.set_caret(src.rfind("Helper").unwrap() + 2);
    assert!(e.goto_definition());
    assert!(e.take_events_for_test().is_empty());
    let sel = e.selection_span();
    assert_eq!(&src[sel.start..sel.end], "Helper");
    assert_eq!(sel.start, src.find("Helper").unwrap());
    // a local
    let src = "local count = 1\nprint(count)\n";
    let mut e = lua_editor(src.as_bytes());
    e.set_caret(src.rfind("count").unwrap() + 1);
    assert!(e.goto_definition());
    assert_eq!(e.selection_span().start, src.find("count").unwrap());
    // nothing to find
    let mut e = lua_editor(b"x = 12\n");
    e.set_caret(5);
    assert!(!e.goto_definition());
}

#[test]
fn go_to_definition_across_files_emits_an_event() {
    // a fake library: one file with a helper
    let mut ix = ApiIndex::stock();
    ix.add_source(
        "campaign/util.lua",
        "---Shout\nfunction CampaignHelper(a)\nend\n",
    );
    let mut e = ScriptEditor::with_language(Box::new(LuaLanguage::with_index(Arc::new(ix))));
    let src = "function OnGameStart()\n\tCampaignHelper(1)\nend\n";
    e.set_text_bytes(src.as_bytes());
    e.set_caret(src.find("CampaignHelper").unwrap() + 3);
    assert!(e.goto_definition());
    let ev = e.take_events_for_test();
    match ev.as_slice() {
        [ScriptEvent::GotoDefinition { file, span }] => {
            assert_eq!(file, "campaign/util.lua");
            let lib = "---Shout\nfunction CampaignHelper(a)\nend\n";
            assert_eq!(&lib[span.start..span.end], "CampaignHelper");
        }
        other => panic!("{other:?}"),
    }
    // a stock API function points into the stock library
    let mut e = lua_editor(b"StartMoney(PLAYER0, 1)\n");
    e.set_caret(3);
    assert!(e.goto_definition());
    match e.take_events_for_test().as_slice() {
        [ScriptEvent::GotoDefinition { file, .. }] => assert_eq!(file, "bindings/setup.lua"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn comment_toggle_uses_lua_markers() {
    let mut e = lua_editor(b"x = 1\n\ty = 2\n");
    e.set_caret(0);
    e.select_span(kfx_script::Span::new(0, 12));
    e.toggle_comment();
    e.apply_pending();
    assert_eq!(text(&e), "-- x = 1\n\t-- y = 2\n");
    e.toggle_comment();
    e.apply_pending();
    assert_eq!(text(&e), "x = 1\n\ty = 2\n");
}

#[test]
fn format_reindents_blocks() {
    let mut e =
        lua_editor(b"function OnGameStart()\nif true then\nStartMoney(PLAYER0, 1)\nend\nend\n");
    e.options.indent = "    ".into();
    e.format_document();
    e.apply_pending();
    assert_eq!(
        text(&e),
        "function OnGameStart()\n    if true then\n        StartMoney(PLAYER0, 1)\n    end\nend\n"
    );
}

#[test]
fn map_references_find_action_points_and_positions() {
    let src = "function OnGameStart()\n\
        \tRegisterOnActionPointEvent(function() end, 3, PLAYER0)\n\
        \tRevealMapRect(PLAYER0, 10, 12, 5, 5)\nend\n";
    let mut e = lua_editor(src.as_bytes());
    let refs = e.map_references();
    assert!(
        refs.iter()
            .any(|r| r.target == MapTarget::ActionPoint(3)
                && r.command == "RegisterOnActionPointEvent"),
        "{refs:#?}"
    );
    assert!(
        refs.iter()
            .any(|r| r.target == MapTarget::Subtile { x: 10, y: 12 }),
        "{refs:#?}"
    );
    let r = refs
        .iter()
        .find(|r| r.target == MapTarget::ActionPoint(3))
        .unwrap();
    assert_eq!(&src[r.span.start..r.span.end], "3");
    assert_eq!(r.line, 2);
}

#[test]
fn pick_from_map_writes_into_lua_arguments() {
    let src =
        "function OnGameStart()\n\tRegisterOnActionPointEvent(function() end, 3, PLAYER0)\nend\n";
    let mut e = lua_editor(src.as_bytes());
    e.set_caret(src.find(", 3,").unwrap() + 2);
    assert_eq!(e.pick_kind_at_caret(), Some(PickKind::ActionPoint));
    e.insert_pick(PickValue::ActionPoint(9));
    e.apply_pending();
    assert!(text(&e).contains("end, 9, PLAYER0"), "{}", text(&e));
}

#[test]
fn block_match_pairs_function_and_end() {
    let src = "function F()\n\tfor i = 1, 2 do\n\t\tif i then\n\t\tend\n\tend\nend\n";
    let a = LuaLanguage::new().analyze(src.as_bytes());
    let (o, c) = a.block_match(src.find("for").unwrap() + 1).expect("pair");
    assert_eq!(&src[o.start..o.end], "for");
    assert_eq!(c.start, src.find("\tend\nend").unwrap() + 1);
    let (o, c) = a.block_match(src.rfind("end").unwrap() + 1).unwrap();
    assert_eq!(&src[o.start..o.end], "function");
    assert_eq!(c.start, src.rfind("end").unwrap());
}

#[test]
fn status_line_lists_hook_functions() {
    let mut e = lua_editor(
        b"function OnGameStart()\nend\nfunction OnGameTick()\nend\n\
            function Helper()\nend\nRegisterTimerEvent(Helper, 10, true)\n",
    );
    let items = e.status_items();
    let t: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
    assert!(t.iter().any(|x| x.starts_with("OnGameStart")), "{t:?}");
    assert!(t.iter().any(|x| x.starts_with("OnGameTick")), "{t:?}");
    assert!(!t.iter().any(|x| x.starts_with("Helper")), "{t:?}");
    assert!(t.iter().any(|x| x.contains("1 trigger")), "{t:?}");
    assert!(e.meters().is_empty());
    let mut e = lua_editor(b"x = 1\n");
    assert_eq!(e.status_items()[0].text, "no hook functions");
}

#[test]
fn outline_lists_functions_hooks_and_triggers() {
    let mut e = lua_editor(
        b"function OnGameStart()\n\tRegisterTimerEvent(function() \
            end, 10, true)\nend\nlocal function helper()\nend\n",
    );
    let o = e.outline();
    let tags: Vec<&str> = o.iter().map(|i| i.tag).collect();
    assert_eq!(tags, vec!["hook", "local"]);
    assert_eq!(o[0].children.len(), 1);
    assert_eq!(o[0].children[0].tag, "trig");
}

#[test]
fn hover_documents_an_api_function() {
    let a = LuaLanguage::new().analyze(b"StartMoney(PLAYER0, 5)\n");
    let h = a.hover(3).expect("hover");
    assert!(
        h.title.as_deref().unwrap_or("").contains("StartMoney"),
        "{h:?}"
    );
    assert!(!h.summary.is_empty() || !h.params.is_empty(), "{h:?}");
}

#[test]
fn lua_reference_lists_the_library() {
    let r = LuaLanguage::new().reference();
    assert!(r.len() > 100);
    let e = r.iter().find(|e| e.name == "AddCreatureToLevel").unwrap();
    assert_eq!(e.snippet_params.len(), 5);
    assert!(!e.category.is_empty());
    assert!(!r.iter().any(|e| e.name.ends_with("_template")));
}

#[test]
fn insert_command_makes_a_call_with_placeholders() {
    let mut e = lua_editor(b"\n");
    e.set_caret(0);
    e.insert_command("StartMoney");
    e.apply_pending();
    assert_eq!(text(&e), "StartMoney(player, gold)\n");
    assert_eq!(e.snippet_stops_for_test().len(), 2);
}

#[test]
fn snippet_placeholders_follow_edits() {
    let mut e = lua_editor(b"\n");
    e.set_caret(0);
    e.insert_snippet("Foo(${1:alpha}, ${2:beta})$0");
    e.apply_pending();
    assert_eq!(text(&e), "Foo(alpha, beta)\n");
    assert_eq!(e.selection_span(), kfx_script::Span::new(4, 9));
    // typing over the first placeholder moves the second
    e.insert_at_caret("a_much_longer_name");
    e.apply_pending();
    let stops = e.snippet_stops_for_test();
    assert_eq!(stops.len(), 2);
    assert_eq!(&text(&e)[stops[1].0..stops[1].1], "beta");
    assert_eq!(&text(&e)[stops[0].0..stops[0].1], "a_much_longer_name");
}

#[test]
fn library_snippet_goes_into_the_existing_game_start() {
    let snip = kfx_script_ui::snippets::LIBRARY
        .iter()
        .find(|s| s.id == "setup")
        .unwrap();
    // with a hook: into the body, wherever the caret is
    let mut e = lua_editor(b"-- top\nfunction OnGameStart()\n\tPrint()\nend\n");
    e.set_caret(0);
    assert!(e.insert_library_snippet(snip));
    e.apply_pending();
    let t = text(&e);
    assert!(
        t.starts_with(
            "-- top\nfunction OnGameStart()\n\tPrint()\n\t-- Initial \
                setup\n\tStartMoney(PLAYER0, 3000)"
        ),
        "{t}"
    );
    assert!(t.trim_end().ends_with("end"), "{t}");
    // without one: a new function wraps it
    let mut e = lua_editor(b"");
    assert!(e.insert_library_snippet(snip));
    e.apply_pending();
    let t = text(&e);
    assert!(
        t.starts_with("function OnGameStart()\n\t-- Initial setup\n\tStartMoney(PLAYER0, 3000)"),
        "{t}"
    );
    assert!(t.trim_end().ends_with("end"), "{t}");
    e.analyze_now(&NullProvider);
    assert!(
        e.diagnostics()
            .iter()
            .all(|d| d.severity != Severity::Error),
        "{:#?}",
        e.diagnostics()
    );
}

#[test]
fn dk_language_gets_the_dk_form_of_a_snippet() {
    let snip = kfx_script_ui::snippets::LIBRARY
        .iter()
        .find(|s| s.id == "win-lose")
        .unwrap();
    let mut e = ScriptEditor::new();
    e.set_text_bytes(b"LEVEL_VERSION(1)\n");
    e.set_caret(17);
    assert!(e.insert_library_snippet(snip));
    e.apply_pending();
    assert!(
        text(&e).contains("IF(PLAYER0, ALL_DUNGEONS_DESTROYED == 1)\n\tWIN_GAME\nENDIF"),
        "{}",
        text(&e)
    );
    // the Lua-only hook snippet has no DK form
    let hook = kfx_script_ui::snippets::LIBRARY
        .iter()
        .find(|s| s.id == "hook-skeleton")
        .unwrap();
    assert!(!e.insert_library_snippet(hook));
}

#[test]
fn unknown_literal_is_flagged_negative_control_for_the_snippet_tests() {
    let mut e = lua_editor(
        b"function OnGameStart()\n\tAddCreatureToLevel(PLAYER_GOOD, \"NOPE\", 1, 1, 0)\nend\n",
    );
    let mut host = StaticProvider::default();
    host.names
        .insert(kfx_script::ConstKind::Creature, vec!["KNIGHT".to_string()]);
    e.analyze_now(&host);
    assert!(e.diagnostic_counts().1 > 0, "{:#?}", e.diagnostics());
}
