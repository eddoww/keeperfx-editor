//! Unit tests of the Lua tooling: diagnostics, completions, signature help, hover, outline,
//! definitions, map references, formatter, template and robustness.
use kfx_script::lua::diagnostics::diagnostics;
use kfx_script::lua::doc::LuaDoc;
use kfx_script::lua::index::ApiIndex;
use kfx_script::lua::nav::{
    definition, format, outline, positions, references, LuaFormatOptions, LuaOutlineKind,
};
use kfx_script::lua::services::{
    completions, highlight, hover, signature_help, LuaCompletionKind, LuaTokenClass,
};
use kfx_script::lua::template::{
    dk_to_lua_name, new_script, new_script_with_hints, TemplateOptions,
};
use kfx_script::services::RefKind;
use kfx_script::{ConstKind, NullProvider, Severity, StaticProvider};

fn ix() -> ApiIndex {
    let ix = ApiIndex::stock();
    assert!(
        ix.functions.len() > 100,
        "data/lua_api.json must be generated (see tests/lua_index_gen.rs)"
    );
    ix
}

/// Split `src` at the `|` cursor marker.
fn cur(src: &str) -> (String, usize) {
    let i = src.find('|').expect("cursor marker");
    (format!("{}{}", &src[..i], &src[i + 1..]), i)
}

fn diags(src: &str) -> Vec<(String, String)> {
    let d = LuaDoc::parse(src);
    diagnostics(&d, &ix(), &NullProvider)
        .into_iter()
        .map(|x| (x.code.to_string(), x.message))
        .collect()
}

fn codes(src: &str) -> Vec<String> {
    diags(src).into_iter().map(|d| d.0).collect()
}

fn labels(src_with_cursor: &str) -> Vec<String> {
    let (s, o) = cur(src_with_cursor);
    let d = LuaDoc::parse(&s);
    completions(&d, o, &ix(), &NullProvider)
        .into_iter()
        .map(|c| c.label)
        .collect()
}

// ---- syntax --------------------------------------------------------------------------------

#[test]
fn syntax_errors_have_precise_spans() {
    let src = "local x = 1\nfunction f(\n  x = = 2\nend\ny = 3 +\n";
    let d = LuaDoc::parse(src);
    let all = diagnostics(&d, &ix(), &NullProvider);
    let errs: Vec<_> = all
        .iter()
        .filter(|x| x.severity == Severity::Error)
        .collect();
    assert!(errs.len() >= 2, "{errs:?}");
    for e in &errs {
        assert_eq!(e.code, "lua-syntax");
        assert!(e.span.start <= src.len() && e.span.end <= src.len());
        assert!(e.line >= 1);
    }
    // the `= =` error is on line 3 and points at the second `=`
    let eq = errs
        .iter()
        .find(|e| &src[e.span.start..e.span.end] == "=")
        .expect("error on '='");
    assert_eq!(eq.line, 3);
    // statements after the error are still parsed: `y = 3 +` is seen
    assert!(d.analysis.global_assigns.iter().any(|g| g.name == "y"));
}

#[test]
fn unfinished_string_and_comment_are_errors() {
    assert!(codes("x = \"abc\n").contains(&"lua-syntax".to_string()));
    assert!(codes("--[[ never closed").contains(&"lua-syntax".to_string()));
    assert!(codes("x = 3abc").contains(&"lua-syntax".to_string()));
    assert!(codes("x = 1 // 2").contains(&"lua-syntax".to_string())); // `//` is Lua 5.3, not LuaJIT
    assert!(codes("x = 5 & 3").contains(&"lua-syntax".to_string()));
    let ok = "x = 0xFFULL + 1e3 + 0x.8p1 + [==[a]]b]==] .. 'q'\ngoto done\n::done::\n";
    assert!(
        !codes(ok).contains(&"lua-syntax".to_string()),
        "{:?}",
        diags(ok)
    );
}

// ---- undefined globals /
// assignments ---------------------------------------------------------------

#[test]
fn undefined_global_with_suggestion_and_escape_hatches() {
    let d = diags("print(PLAYR0)\n");
    let (c, m) = &d[0];
    assert_eq!(c, "lua-undefined-global");
    assert!(m.contains("PLAYER0"), "{m}");
    assert!(codes("print(PLAYER0, string.format('%d', 1), Game)\n").is_empty());
    // escape hatches
    assert!(codes("---@diagnostic disable-next-line: undefined-global\nprint(foo)\n").is_empty());
    assert!(codes("print(foo) ---@diagnostic disable-line: lua-undefined-global\n").is_empty());
    assert!(
        codes(
            "---@diagnostic disable: undefined-global\nprint(foo)\n---@diagnostic enable: \
        undefined-global\nprint(bar)\n"
        )
        .iter()
        .filter(|c| *c == "lua-undefined-global")
        .count()
            == 1
    );
    assert!(codes("-- luacheck: globals foo\nprint(foo)\n").is_empty());
    // defined in this file (even later), via _G, or dynamic globals
    assert!(codes("function g() return h() end\nfunction h() end\n").is_empty());
    assert!(codes("_G.foo = 1\nprint(foo)\n").is_empty());
    assert!(codes("setmetatable(_G, {})\nprint(foo)\n").is_empty());
    // locals shadow, and declared locals are not globals
    assert!(codes("local a = 1\nprint(a)\n").is_empty());
}

#[test]
fn assignment_to_undeclared_global_inside_function() {
    let c = diags("function f()\n  leaked = 1\nend\n");
    assert!(
        c.iter()
            .any(|d| d.0 == "lua-global-assign" && d.1.contains("leaked")),
        "{c:?}"
    );
    // top-level assignment is a definition
    assert!(
        codes("counter = 0\nfunction f() counter = counter + 1 end\n")
            .iter()
            .all(|c| c != "lua-global-assign")
    );
    // known library globals may be assigned anywhere
    assert!(codes("function f() Game.x = 1 Game = {} end\n")
        .iter()
        .all(|c| c != "lua-global-assign"));
    assert!(codes(
        "function f()\n  ---@diagnostic disable-next-line: lua-global-assign\n  leaked = 1\nend\n"
    )
    .iter()
    .all(|c| c != "lua-global-assign"));
}

// ----
// calls -----------------------------------------------------------------------------------------

#[test]
fn argument_counts() {
    // WinGame(player?) takes at most one
    let d = diags("WinGame(PLAYER0, 1)\n");
    assert!(
        d.iter()
            .any(|d| d.0 == "lua-arg-count" && d.1.contains("at most 1")),
        "{d:?}"
    );
    // CreateEffect(effect, location, height) needs three
    let d = diags("CreateEffect(\"EFFECT_EXPLOSION_1\")\n");
    assert!(
        d.iter()
            .any(|d| d.0 == "lua-arg-count" && d.1.contains("at least 3")),
        "{d:?}"
    );
    assert!(codes("CreateEffect(\"EFFECT_EXPLOSION_1\", PLAYER0, 0)\n").is_empty());
    // a multi-value last argument can satisfy the count; half-typed calls are silent
    assert!(codes("CreateEffect(\"EFFECT_EXPLOSION_1\", unpack(t))\n")
        .iter()
        .all(|c| c != "lua-arg-count"));
    assert!(codes("CreateEffect(\n")
        .iter()
        .all(|c| c != "lua-arg-count"));
    // methods: Creature:level_up(levels?)
    let d = diags("local c = GetCreatureNear(1, 2)\nc:level_up(1, 2)\n");
    assert!(d.iter().any(|d| d.0 == "lua-arg-count"), "{d:?}");
    assert!(codes("local c = GetCreatureNear(1, 2)\nc:level_up(1)\nc:kill()\n").is_empty());
}

#[test]
fn bad_string_literal_with_suggestion() {
    let src = "AddCreatureToLevel(PLAYER0, \"KNIGTH\", 1, 1, 0)\n";
    let d = LuaDoc::parse(src);
    let out = diagnostics(&d, &ix(), &NullProvider);
    let bad = out
        .iter()
        .find(|x| x.code == "lua-bad-literal")
        .expect("bad literal");
    assert_eq!(&src[bad.span.start..bad.span.end], "\"KNIGTH\"");
    assert_eq!(bad.suggestion.as_deref(), Some("KNIGHT"));
    // case-insensitive like the game's get_rid, RANDOM accepted, numbers never flagged
    assert!(codes(
        "AddCreatureToLevel(PLAYER0, \"knight\", 1, 1, \
        0)\nAddCreatureToLevel(PLAYER0, \"RANDOM\", 1, 1, 0)\nAddCreatureToLevel(PLAYER0, 3, 1, 1, \
        0)\n"
    )
    .iter()
    .all(|c| c != "lua-bad-literal"));
    // the provider extends the union with custom creatures
    let mut p = StaticProvider::default();
    p.names.insert(ConstKind::Creature, vec!["MYCUSTOM".into()]);
    let d = LuaDoc::parse("AddCreatureToLevel(PLAYER0, \"MYCUSTOM\", 1, 1, 0)\n");
    assert!(diagnostics(&d, &ix(), &p)
        .iter()
        .all(|x| x.code != "lua-bad-literal"));
    let d = LuaDoc::parse("AddCreatureToLevel(PLAYER0, \"MYCUSTOM\", 1, 1, 0)\n");
    assert!(diagnostics(&d, &ix(), &NullProvider)
        .iter()
        .any(|x| x.code == "lua-bad-literal"));
}

// ---- KeeperFX
// lints --------------------------------------------------------------------------------

#[test]
fn trigger_callback_lints() {
    // local function by value
    let c = codes(
        "local function cb() end\nfunction OnGameStart()\n  RegisterTimerEvent(cb, 100, \
        false)\nend\n",
    );
    assert!(c.contains(&"lua-trigger-local-fn".to_string()), "{c:?}");
    // global named function: fine
    let c = codes(
        "function cb() end\nfunction OnGameStart()\n  RegisterTimerEvent(cb, 100, false)\nend\n",
    );
    assert!(!c.contains(&"lua-trigger-local-fn".to_string()), "{c:?}");
    // closure capturing a table
    let src = "local data = {}\nfunction OnGameStart()\n  RegisterTimerEvent(\n    \
        function()\n      print(data)\n    end,\n    100, false)\nend\n";
    let d = diags(src);
    assert!(
        d.iter()
            .any(|d| d.0 == "lua-trigger-capture" && d.1.contains("data")),
        "{d:?}"
    );
    // capturing a number/string/boolean is allowed
    let src = "local n = 5\nfunction OnGameStart()\n  RegisterTimerEvent(\n    function()\n      \
        print(n)\n    end,\n    100, false)\nend\n";
    assert!(!codes(src).contains(&"lua-trigger-capture".to_string()));
    // one-liner anonymous callback: portability note
    let c =
        codes("function OnGameStart()\n  RegisterTimerEvent(function() end, 100, false)\nend\n");
    assert!(c.contains(&"lua-anon-callback-layout".to_string()), "{c:?}");
    let c = codes(
        "function OnGameStart()\n  RegisterTimerEvent(\n    function()\n    end,\n    \
        100, false)\nend\n",
    );
    assert!(
        !c.contains(&"lua-anon-callback-layout".to_string()),
        "{c:?}"
    );
}

#[test]
fn state_outside_game_table_mutated_in_hooks() {
    let src = "local counter = 0\nfunction OnGameTick()\n  counter = counter + 1\nend\n";
    let d = diags(src);
    assert!(
        d.iter()
            .any(|d| d.0 == "lua-hook-state" && d.1.contains("counter")),
        "{d:?}"
    );
    // table fields of file-level locals and user globals
    assert!(
        codes("local st = {}\nfunction OnGameTick()\n  st.n = 1\nend\n")
            .contains(&"lua-hook-state".to_string())
    );
    assert!(
        codes("Stats = {}\nfunction OnGameTick()\n  Stats.n = 1\nend\n")
            .contains(&"lua-hook-state".to_string())
    );
    assert!(
        codes("local l = {}\nfunction OnGameTick()\n  table.insert(l, 1)\nend\n")
            .contains(&"lua-hook-state".to_string())
    );
    // Game table, API globals and locals of the hook itself are fine; non-hook functions too
    assert!(!codes(
        "function OnGameTick()\n  Game.n = (Game.n or 0) + 1\n  local x = 1\n  x = 2\n  \
        PLAYER0.FLAG0 = 1\nend\n"
    )
    .contains(&"lua-hook-state".to_string()));
    assert!(!codes("local c = 0\nfunction helper() c = c + 1 end\n")
        .contains(&"lua-hook-state".to_string()));
    // trigger callbacks count as hooks
    assert!(codes(
        "local c = 0\nfunction cb() c = c + 1 end\nfunction OnGameStart()\n  \
        RegisterTimerEvent(cb, 5, true)\nend\n"
    )
    .contains(&"lua-hook-state".to_string()));
}

#[test]
fn sandbox_and_multiplayer_lints() {
    let d = diags("local t = os.time()\n");
    assert!(
        d.iter()
            .any(|d| d.0 == "lua-sandbox-removed" && d.1.contains("GetGameTimestamp")),
        "{d:?}"
    );
    assert!(
        codes("os.execute('x')\nio.write('x')\nprint(ffi)\n")
            .iter()
            .filter(|c| *c == "lua-sandbox-removed")
            .count()
            == 3
    );
    assert!(
        codes("math.randomseed(5)\nlocal f = io.open('x')\n")
            .iter()
            .filter(|c| *c == "lua-mp-unsafe")
            .count()
            == 2
    );
    // a local named os is not the library
    assert!(codes("local os = {time = function() end}\nos.time()\n")
        .iter()
        .all(|c| c != "lua-sandbox-removed"));
    assert!(codes("math.random(1, 2)\nstring.format('x')\nbit.band(1, 2)\n").is_empty());
}

#[test]
fn deprecated_and_unknown_hook() {
    let c = codes("function OnGameStrat() end\n");
    assert!(c.contains(&"lua-unknown-hook".to_string()), "{c:?}");
    let d = diags("function OnGameStrat() end\n");
    assert!(d[0].1.contains("OnGameStart"), "{d:?}");
    assert!(
        !codes("function OnGameStart() end\nfunction OnModGameStart_foo() end\n")
            .contains(&"lua-unknown-hook".to_string())
    );
    let mut ix2 = ix();
    ix2.add_source("x.lua", "---@deprecated use New\nfunction Old() end\n");
    let d = LuaDoc::parse("Old()\n");
    assert!(diagnostics(&d, &ix2, &NullProvider)
        .iter()
        .any(|x| x.code == "lua-deprecated"));
}

// ----
// completions -------------------------------------------------------------------------------------

#[test]
fn completion_global_context() {
    let l = labels("function OnGameStart()\n  local mycounter = 1\n  Create|\nend\n");
    assert!(
        l.contains(&"CreateEffect".to_string()) && l.contains(&"CreateTrigger".to_string()),
        "{l:?}"
    );
    assert!(!l.contains(&"WinGame".to_string()));
    let l = labels("local mycounter = 1\nmyc|\n");
    assert!(l.contains(&"mycounter".to_string()));
    let l = labels("PLAY|\n");
    assert!(l.contains(&"PLAYER0".to_string()) && l.contains(&"PLAYER_GOOD".to_string()));
    // after `function` the hooks are offered
    let l = labels("function On|\n");
    assert!(l.contains(&"OnGameStart".to_string()) && l.contains(&"OnPowerCast".to_string()));
    // trigger registration snippets
    let l = labels("RegisterTim|\n");
    assert!(l.iter().any(|x| x.starts_with("RegisterTimerEvent")));
}

#[test]
fn completion_members_after_dot_and_colon() {
    let l = labels("local c = GetCreatureNear(1, 2)\nc:|\n");
    assert!(
        l.contains(&"level_up".to_string())
            && l.contains(&"kill".to_string())
            && l.contains(&"isValid".to_string()),
        "{l:?}"
    );
    let l = labels("local c = GetCreatureNear(1, 2)\nc.|\n");
    assert!(
        l.contains(&"level".to_string())
            && l.contains(&"health".to_string())
            && l.contains(&"pos".to_string()),
        "{l:?}"
    );
    // prefix filtering and the `.`-after-global-class case
    let l = labels("local c = GetCreatureNear(1, 2)\nc:lev|\n");
    assert_eq!(l, vec!["level_up".to_string()]);
    let l = labels("PLAYER0.came|\n");
    assert_eq!(l, vec!["camera".to_string()]);
    let l = labels("PLAYER0:|\n");
    assert!(
        l.contains(&"add_gold".to_string()) && l.contains(&"available".to_string()),
        "{l:?}"
    );
    // typed through ---@type, ---@param and for-loops over typed arrays
    let l = labels("---@type Room\nlocal r = nil\nr.|\n");
    assert!(l.contains(&"owner".to_string()), "{l:?}");
    let l = labels("---@param t Thing\nfunction f(t)\n  t.|\nend\n");
    assert!(l.contains(&"owner".to_string()), "{l:?}");
    let l = labels("for _, cr in ipairs(GetCreatures()) do\n  cr:|\nend\n");
    assert!(l.contains(&"level_up".to_string()), "{l:?}");
    // hook parameters take their types from the library hook
    let l = labels("function OnCreatureDeath(unit)\n  unit.|\nend\n");
    assert!(l.contains(&"level".to_string()), "{l:?}");
    // standard library members and string methods
    let l = labels("string.fo|\n");
    assert_eq!(l, vec!["format".to_string()]);
    let l = labels("local s = 'abc'\ns:up|\n");
    assert_eq!(l, vec!["upper".to_string()]);
    // unknown base: nothing
    assert!(labels("local u = foo()\nu.|\n").is_empty());
}

#[test]
fn completion_inside_string_argument() {
    let (s, o) = cur("AddCreatureToLevel(PLAYER0, \"KN|\", 1, 1, 0)\n");
    let d = LuaDoc::parse(&s);
    let c = completions(&d, o, &ix(), &NullProvider);
    let labels: Vec<_> = c.iter().map(|c| c.label.as_str()).collect();
    assert!(
        labels.contains(&"KNIGHT") && labels.contains(&"WIZARD"),
        "{labels:?}"
    );
    assert!(c.iter().all(|c| c.kind == LuaCompletionKind::Constant));
    // replacement covers the string contents, not the quotes
    assert_eq!(&s[c[0].replace.start..c[0].replace.end], "KN");
    // second argument of another function; per-literal comments are carried
    let (s, o) = cur("PlaySound(PLAYER0, \"|\")\nPlayMusic(1)\n");
    let d = LuaDoc::parse(&s);
    let _ = completions(&d, o, &ix(), &NullProvider);
    // config-driven names are added
    let mut p = StaticProvider::default();
    p.names.insert(ConstKind::Creature, vec!["MYCUSTOM".into()]);
    let (s, o) = cur("AddCreatureToLevel(PLAYER0, \"MY|\", 1, 1, 0)\n");
    let d = LuaDoc::parse(&s);
    assert!(completions(&d, o, &ix(), &p)
        .iter()
        .any(|c| c.label == "MYCUSTOM"));
    // comparison with a typed field
    let (s, o) = cur("local c = GetCreatureNear(1, 2)\nif c.state == \"|\" then end\n");
    let d = LuaDoc::parse(&s);
    assert!(!completions(&d, o, &ix(), &NullProvider).is_empty());
    // not a call argument and no expected type: nothing
    assert!(labels_of("local s = \"|\"\n").is_empty());
}

fn labels_of(src: &str) -> Vec<String> {
    labels(src)
}

#[test]
fn completion_inside_table_constructor() {
    let mut ix2 = ix();
    ix2.add_source(
        "o.lua",
        "---@class Opts\n---@field speed integer the speed\n---@field name \
        string\n\n---@param o Opts\nfunction Configure(o) end\n",
    );
    let (s, o) = cur("Configure({ speed = 1, | })\n");
    let d = LuaDoc::parse(&s);
    let c = completions(&d, o, &ix2, &NullProvider);
    let l: Vec<_> = c.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(l, vec!["name"], "{l:?}");
    assert_eq!(c[0].insert, "name = ");
    // outside any typed table the global list is offered
    let (s, o) = cur("local t = { Wi| }\n");
    let d = LuaDoc::parse(&s);
    assert!(completions(&d, o, &ix2, &NullProvider)
        .iter()
        .any(|c| c.label == "WinGame"));
}

#[test]
fn completion_in_annotations() {
    let l = labels("---@pa|\nfunction f() end\n");
    assert!(l.contains(&"param".to_string()));
    let l = labels("---@param x cre|\nfunction f(x) end\n");
    assert!(
        l.contains(&"creature_type".to_string()) && l.contains(&"Creature".to_string()),
        "{l:?}"
    );
    let l = labels("---@return Cr|\n");
    assert!(l.contains(&"Creature".to_string()));
}

// ---- signature
// help ---------------------------------------------------------------------------------

#[test]
fn signature_help_active_parameter() {
    let (s, o) = cur("CreateEffect(\"EFFECT_EXPLOSION_1\", PLAYER0, |)\n");
    let d = LuaDoc::parse(&s);
    let h = signature_help(&d, o, &ix()).expect("help");
    assert_eq!(h.name, "CreateEffect");
    assert_eq!(h.active, 2);
    assert_eq!(h.params.len(), 3);
    assert_eq!(
        &h.label[h.params[2].label_range.0..h.params[2].label_range.1],
        "height: integer"
    );
    assert!(
        h.label
            .starts_with("CreateEffect(effect: effect_or_effelem_type, location: location"),
        "{}",
        h.label
    );
    // first parameter, unclosed call, nested call
    let (s, o) = cur("CreateEffect(|\n");
    let d = LuaDoc::parse(&s);
    assert_eq!(signature_help(&d, o, &ix()).unwrap().active, 0);
    let (s, o) = cur("CreateEffect(\"x\", PLAYER0:available(|))\n");
    let d = LuaDoc::parse(&s);
    assert_eq!(
        signature_help(&d, o, &ix()).unwrap().name,
        "Player:available"
    );
    // methods and the standard library
    let (s, o) = cur("local c = GetCreatureNear(1, 2)\nc:teleport(PLAYER0, |)\n");
    let d = LuaDoc::parse(&s);
    let h = signature_help(&d, o, &ix()).unwrap();
    assert_eq!(h.name, "Creature:teleport");
    assert_eq!(h.active, 1);
    let (s, o) = cur("string.format('%d', |)\n");
    let d = LuaDoc::parse(&s);
    assert_eq!(signature_help(&d, o, &ix()).unwrap().params[0].name, "fmt");
    // outside a call
    let (s, o) = cur("local x = 1 |\n");
    assert!(signature_help(&LuaDoc::parse(&s), o, &ix()).is_none());
}

// ----
// hover -------------------------------------------------------------------------------------------

#[test]
fn hover_shows_docs_and_types() {
    let (s, o) = cur("CreateEff|ect(\"EFFECT_EXPLOSION_1\", PLAYER0, 0)\n");
    let d = LuaDoc::parse(&s);
    let h = hover(&d, o, &ix()).expect("hover");
    assert!(
        h.markdown
            .contains("CreateEffect(effect: effect_or_effelem_type"),
        "{}",
        h.markdown
    );
    assert!(h.markdown.contains("Create an Effect at a location"));
    assert!(h.markdown.contains("bindings/effects.lua"));
    // locals with inferred types
    let (s, o) = cur("local c = GetCreatureNear(1, 2)\nprint(|c)\n");
    let d = LuaDoc::parse(&s);
    assert!(hover(&d, o, &ix())
        .unwrap()
        .markdown
        .contains("c: Creature"));
    // members
    let (s, o) = cur("local c = GetCreatureNear(1, 2)\nc:level_|up(1)\n");
    let h = hover(&LuaDoc::parse(&s), o, &ix()).unwrap();
    assert!(h.markdown.contains("Creature:level_up"), "{}", h.markdown);
    let (s, o) = cur("local c = GetCreatureNear(1, 2)\nprint(c.lev|el)\n");
    assert!(hover(&LuaDoc::parse(&s), o, &ix())
        .unwrap()
        .markdown
        .contains("integer"));
    // alias literal comment
    let mut ix2 = ix();
    ix2.add_source(
        "a.lua",
        "---@alias mood\n---| \"HAPPY\" # smiling\n---| \"SAD\" # \
        crying\n\n---@param m mood\nfunction SetMood(m) end\n",
    );
    let (s, o) = cur("SetMood(\"HA|PPY\")\n");
    let h = hover(&LuaDoc::parse(&s), o, &ix2).unwrap();
    assert!(h.markdown.contains("smiling"), "{}", h.markdown);
    // hooks
    let (s, o) = cur("function OnGame|Start() end\n");
    assert!(hover(&LuaDoc::parse(&s), o, &ix())
        .unwrap()
        .markdown
        .contains("lua_triggers.c"));
    // nothing on whitespace/keywords
    let (s, o) = cur("local x = 1 |\n");
    assert!(hover(&LuaDoc::parse(&s), o, &ix()).is_none());
}

// ---- outline / definition /
// references ------------------------------------------------------------------

#[test]
fn outline_lists_functions_hooks_and_triggers() {
    let src = "---@class Foo\nlocal function helper(a) end\nfunction OnGameStart()\n  \
        RegisterTimerEvent(cb, 100, false)\n  local function inner() end\nend\nfunction Foo:bar(x) \
        end\nfunction cb() end\n";
    let d = LuaDoc::parse(src);
    let o = outline(&d);
    let kinds: Vec<_> = o.iter().map(|i| (i.kind, i.label.as_str())).collect();
    assert!(kinds.contains(&(LuaOutlineKind::Class, "Foo")), "{kinds:?}");
    assert!(kinds.contains(&(LuaOutlineKind::LocalFunction, "helper(a)")));
    assert!(kinds.contains(&(LuaOutlineKind::Hook, "OnGameStart()")));
    assert!(kinds.contains(&(LuaOutlineKind::Method, "Foo:bar(x)")));
    let hook = o.iter().find(|i| i.kind == LuaOutlineKind::Hook).unwrap();
    assert!(hook
        .children
        .iter()
        .any(|c| c.kind == LuaOutlineKind::Trigger && c.label == "RegisterTimerEvent"));
    assert!(hook
        .children
        .iter()
        .any(|c| c.kind == LuaOutlineKind::LocalFunction));
    assert_eq!(hook.line, 3);
}

#[test]
fn go_to_definition_in_file_and_library() {
    let (s, o) = cur("local value = 1\nfunction mine() end\nprint(val|ue)\nmine()\n");
    let d = LuaDoc::parse(&s);
    let l = definition(&d, o, &ix()).unwrap();
    assert!(l.file.is_none() && &s[l.span.start..l.span.end] == "value");
    let l = definition(&d, s.rfind("mine").unwrap() + 1, &ix()).unwrap();
    assert!(l.file.is_none());
    // library function
    let (s, o) = cur("Create|Effect(1, 2)\n");
    let d = LuaDoc::parse(&s);
    let l = definition(&d, o, &ix()).unwrap();
    assert_eq!(l.file.as_deref(), Some("bindings/effects.lua"));
    // library method through type inference
    let (s, o) = cur("local c = GetCreatureNear(1, 2)\nc:kil|l()\n");
    let l = definition(&LuaDoc::parse(&s), o, &ix()).unwrap();
    assert_eq!(l.file.as_deref(), Some("classes/Creature.lua"));
    // unknown
    let (s, o) = cur("nothing|here()\n");
    assert!(definition(&LuaDoc::parse(&s), o, &ix()).is_none());
}

#[test]
fn map_references() {
    let src = "RegisterOnActionPointEvent(function() end, 3, PLAYER0)\nZoomToLocation(PLAYER0, \
        7)\nlocal c = GetCreatureNear(10, 20)\nlocal s = GetSlab(4, 5)\nGetCreatureNear(x, \
        3)\nCreateEffect(\"EFFECT_EXPLOSION_1\", 9)\n";
    let d = LuaDoc::parse(src);
    let r = references(&d, &ix());
    let kinds: Vec<(RefKind, i32)> = r.iter().map(|r| (r.kind, r.value)).collect();
    assert!(kinds.contains(&(RefKind::ActionPoint, 3)), "{kinds:?}");
    assert!(kinds.contains(&(RefKind::ActionPoint, 7)), "{kinds:?}");
    assert!(kinds.contains(&(RefKind::ActionPoint, 9)), "{kinds:?}");
    assert!(
        kinds.contains(&(RefKind::SubtileX, 10)) && kinds.contains(&(RefKind::SubtileY, 20)),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&(RefKind::SlabX, 4)) && kinds.contains(&(RefKind::SlabY, 5)),
        "{kinds:?}"
    );
    let ap = r
        .iter()
        .find(|r| r.kind == RefKind::ActionPoint && r.value == 3)
        .unwrap();
    assert_eq!(&src[ap.span.start..ap.span.end], "3");
    let p = positions(&d, &ix());
    assert!(
        p.iter()
            .any(|p| p.x == 10 && p.y == 20 && !p.slab && p.command == "GetCreatureNear"),
        "{p:?}"
    );
    assert!(p.iter().any(|p| p.x == 4 && p.y == 5 && p.slab), "{p:?}");
    // a non-literal coordinate yields no pair
    assert!(!p.iter().any(|p| p.y == 3));
}

// ---- highlight / format /
// template ---------------------------------------------------------------------

#[test]
fn highlight_classes() {
    let src = "---@param x integer the x\nlocal function f(x)\n  WinGame(PLAYER0, \"a\", 12) -- \
        hi\n  return print(nope, true)\nend\n";
    let d = LuaDoc::parse(src);
    let h = highlight(&d, &ix());
    let at = |needle: &str| {
        let i = src.find(needle).unwrap();
        h.iter().find(|(s, _)| s.start == i).map(|x| x.1)
    };
    assert_eq!(at("local"), Some(LuaTokenClass::Keyword));
    assert_eq!(at("WinGame"), Some(LuaTokenClass::ApiFunction));
    assert_eq!(at("PLAYER0"), Some(LuaTokenClass::ApiGlobal));
    assert_eq!(at("\"a\""), Some(LuaTokenClass::String));
    assert_eq!(at("12"), Some(LuaTokenClass::Number));
    assert_eq!(at("-- hi"), Some(LuaTokenClass::Comment));
    assert_eq!(at("print"), Some(LuaTokenClass::Std));
    assert_eq!(at("nope"), Some(LuaTokenClass::Unknown));
    assert_eq!(at("true"), Some(LuaTokenClass::Constant));
    assert_eq!(at("@param"), Some(LuaTokenClass::Annotation));
    assert_eq!(at("integer"), Some(LuaTokenClass::AnnotationType));
    // spans are ordered and disjoint
    for w in h.windows(2) {
        assert!(w[0].0.end <= w[1].0.start, "{:?} {:?}", w[0], w[1]);
    }
    // errors are classified
    let d = LuaDoc::parse("x = 'oops\n");
    assert!(highlight(&d, &ix())
        .iter()
        .any(|x| x.1 == LuaTokenClass::Error));
}

#[test]
fn formatter_reindents_only() {
    let src =
        "function f(a)\nif a then\nx = 1\nelseif b then\nx = 2\nelse\nx = 3\nend\nfor i = 1, \
        3 do\nprint(i)\nend\nreturn \
        function()\nend\nend\n\nRegisterTimerEvent(\nfunction()\nreturn 1\nend,\n100, \
        false)\ncall(function()\nbody()\nend)\nlocal t = {\na = 1,\nb = {\nc = 2,\n},\n}\n  -- \
        trailing comment\nx = [[\nraw\n   text]]\n";
    let d = LuaDoc::parse(src);
    let out = format(&d, &LuaFormatOptions::default());
    let expect = "function f(a)\n    if a then\n        x = 1\n    elseif b then\n        x = \
        2\n    else\n        x = 3\n    end\n    for i = 1, 3 do\n        print(i)\n    end\n    \
        return function()\n    end\nend\n\nRegisterTimerEvent(\n    function()\n        return \
        1\n    end,\n    100, false)\ncall(function()\n    body()\nend)\nlocal t = {\n    a = \
        1,\n    b = {\n        c = 2,\n    },\n}\n-- trailing comment\nx = [[\nraw\n   text]]\n";
    assert_eq!(out, expect);
    // idempotent and only whitespace changes
    let d2 = LuaDoc::parse(&out);
    assert_eq!(format(&d2, &LuaFormatOptions::default()), out);
    let strip = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(strip(&out), strip(src));
    // tabs
    let out = format(
        &LuaDoc::parse("if x then\ny()\nend\n"),
        &LuaFormatOptions {
            indent: "\t".into(),
        },
    );
    assert_eq!(out, "if x then\n\ty()\nend\n");
}

#[test]
fn template_generator() {
    let t = new_script(&TemplateOptions {
        map_number: 42,
        title: Some("Test".into()),
        examples: true,
    });
    assert!(t.starts_with("-- map00042.lua: Test"));
    assert!(t.contains("function OnGameStart()"));
    let d = LuaDoc::parse(&t);
    assert!(d.syntax_errors.is_empty() && d.lex_errors.is_empty());
    let diag = diagnostics(&d, &ix(), &NullProvider);
    assert!(
        diag.iter().all(|x| x.severity != Severity::Error),
        "{diag:?}"
    );
    assert_eq!(dk_to_lua_name("SET_GENERATE_SPEED"), "SetGenerateSpeed");
    let dk = kfx_script::Document::parse(
        b"LEVEL_VERSION(1)\nSET_GENERATE_SPEED(500)\n\
        ADD_CREATURE_TO_LEVEL(PLAYER1, KNIGHT, 1, 2, 3, 100)\nNOPE_COMMAND(1)\n",
    );
    let t = new_script_with_hints(
        &TemplateOptions {
            map_number: 7,
            title: None,
            examples: false,
        },
        &dk,
        &ix(),
    );
    assert!(
        t.contains("-- DK: SET_GENERATE_SPEED(500)  ->  SetGenerateSpeed("),
        "{t}"
    );
    assert!(
        t.contains("-- DK: ADD_CREATURE_TO_LEVEL(") && t.contains("AddCreatureToLevel("),
        "{t}"
    );
    assert!(t.contains("no Lua function named"));
    assert!(LuaDoc::parse(&t).syntax_errors.is_empty());
}

// ----
// robustness --------------------------------------------------------------------------------------

fn services_do_not_panic(src: &str, ix: &ApiIndex, offsets: &[usize]) {
    let d = LuaDoc::parse(src);
    let _ = diagnostics(&d, ix, &NullProvider);
    let _ = highlight(&d, ix);
    let _ = outline(&d);
    let _ = references(&d, ix);
    let _ = positions(&d, ix);
    let _ = format(&d, &LuaFormatOptions::default());
    for &o in offsets {
        let mut o = o.min(src.len());
        while !src.is_char_boundary(o) {
            o -= 1;
        }
        let _ = completions(&d, o, ix, &NullProvider);
        let _ = signature_help(&d, o, ix);
        let _ = hover(&d, o, ix);
        let _ = definition(&d, o, ix);
    }
}

#[test]
fn half_typed_code_does_not_panic_or_hang() {
    let ix = ix();
    let samples = [
        "local x = ",
        "function f(",
        "if x then\n  foo(",
        "t = { a = 1,\n b = ",
        "x = \"abc",
        "c:",
        "a.b.",
        "for i = 1,",
        "local function",
        "return",
        "::",
        "x = [==[ abc",
        "RegisterTimerEvent(function()\n  local c = GetCreatureNear(1,2)\n  c.",
    ];
    for s in samples {
        let offs: Vec<usize> = (0..=s.len()).collect();
        services_do_not_panic(s, &ix, &offs);
    }
    // stray bytes and non-ASCII
    services_do_not_panic(
        "x = 'é' .. $ ` @ !\n\u{1F600} = 1\n",
        &ix,
        &(0..40).collect::<Vec<_>>(),
    );
    // deep nesting stays bounded
    let deep = "x = ".to_string() + &"(".repeat(3000) + &"{".repeat(3000);
    let t = std::time::Instant::now();
    services_do_not_panic(&deep, &ix, &[10, 100]);
    assert!(t.elapsed().as_secs() < 5);
}

/// Snippets of `Lua-level-script.md` (wiki) are known-good: no warning may fire on them.
#[test]
fn wiki_examples_have_no_false_positives() {
    let src = r#"
function ThingToDoWhenSlapIsCast(eventData, triggerData)
  PLAYER0.MONEY = PLAYER0.MONEY + 1
end

function OnGameStart()
  RegisterPowerCastEvent(ThingToDoWhenSlapIsCast, "POWER_SLAP")
  SetGenerateSpeed(300)
  StartMoney(PLAYER0, 7500)
  CreatureAvailable(PLAYER1, "TROLL", true, 2)
  local dragon = AddCreatureToLevel(PLAYER0, "DRAGON", 1, 3, 200)
  dragon:level_up(2)
  CreateParty("RAIDERS")
  AddToParty("RAIDERS", "TUNNELLER", 5, 0, "ATTACK_DUNGEON_HEART", 0)
  AddTunnellerPartyToLevel(PLAYER0, "RAIDERS", 2, "DUNGEON", 1, 5, 0)
  RegisterOnActionPointEvent(function() DisplayInformation(42, PLAYER0) end, 3, PLAYER0)
  PLAYER0.FLAG0 = 1
  PLAYER0.CAMPAIGN_FLAG2 = PLAYER0.MONEY + 75 * PLAYER0.IMP
  local imp = GetCreatureByCriterion(PLAYER0, "IMP", "MOST_EXPERIENCED")
  imp:level_up(2)
  imp:kill()
  if PLAYER0:available("WORKSHOP") and PLAYER0:controls("IMP") and PLAYER0:allied_with(PLAYER2) then
  end
  local slab = GetSlab(3, 4)
  print(slab.owner, slab.type, slab.style)
  Game.hasTriggeredEvent = true
  Game.playerScore = 1234
end
"#;
    let d = diags(src);
    let bad: Vec<_> = d
        .iter()
        .filter(|x| x.0 != "lua-anon-callback-layout")
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn index_extraction_of_annotations() {
    let src = r#"---@meta
---@alias mood
---| "HAPPY" # smiling
---|> "SAD" # crying

---@enum Level
local Level = { LOW = 1, HIGH = 2 }

---@class Animal
---@field name string the name
---@field legs? integer

---@class Dog: Animal
---@field good boolean
Dog = {}

---Barks loudly.
---@param times integer how often
---@param style? "LOUD"|"SOFT"
---@return boolean ok
---@overload fun(): boolean
---@deprecated use Howl
function Dog:bark(times, style) end

function Dog.new(n) end

---@type Dog
Pet = nil

function plain(a, b, ...) end
Counter = 0
"#;
    let mut ix = ApiIndex::default();
    ix.add_source("t.lua", src);
    let a = &ix.aliases["mood"];
    assert_eq!(
        a.literals
            .iter()
            .map(|l| (l.value.as_str(), l.doc.as_str()))
            .collect::<Vec<_>>(),
        vec![("HAPPY", "smiling"), ("SAD", "crying")]
    );
    assert_eq!(
        ix.enums["Level"].members,
        vec!["LOW".to_string(), "HIGH".to_string()]
    );
    let dog = &ix.classes["Dog"];
    assert_eq!(dog.parents, vec!["Animal".to_string()]);
    assert!(dog.fields.contains_key("good") && ix.classes["Animal"].fields["legs"].optional);
    let bark = &dog.methods["bark"];
    assert!(bark.method && bark.annotated && bark.deprecated.as_deref() == Some("use Howl"));
    assert_eq!(bark.doc, "Barks loudly.");
    assert_eq!(bark.params[0].doc, "how often");
    assert!(bark.params[1].optional && bark.params[1].ty == "\"LOUD\"|\"SOFT\"");
    assert_eq!(bark.returns[0].ty, "boolean");
    assert_eq!(bark.overloads.len(), 1);
    assert_eq!(
        bark.signature(),
        "bark(times: integer, style?: \"LOUD\"|\"SOFT\"): boolean"
    );
    // static member and constructor convention
    assert!(!dog.methods["new"].method);
    assert_eq!(dog.methods["new"].returns[0].ty, "Dog");
    // inheritance through the chain
    assert!(
        ix.find_member("Dog", "name").is_some()
            && ix.find_member("Dog", "bark").is_some()
            && ix.find_member("Animal", "bark").is_none()
    );
    // globals: annotated, inferred and unannotated functions
    assert_eq!(ix.globals["Pet"].ty, "Dog");
    assert_eq!(ix.globals["Counter"].ty, "integer");
    let p = &ix.functions["plain"];
    assert!(!p.annotated && p.vararg.is_some() && p.params.len() == 2);
    // alias resolution follows references
    let mut ix2 = ix.clone();
    ix2.add_source("u.lua", "---@alias feeling mood|\"ANGRY\"\n");
    let v = ix2.alias_values("feeling").unwrap();
    assert_eq!(v.literals.len(), 3);
    assert!(!v.open);
    // JSON round trip
    assert_eq!(ApiIndex::from_json(&ix2.to_json()).unwrap(), ix2);
    // merging keeps a documented stub over an unannotated redefinition
    ix2.add_source("v.lua", "function Dog:bark() end\n");
    assert!(ix2.classes["Dog"].methods["bark"].annotated);
}

// ---- argument type checking of
// literals -------------------------------------------------------------

fn type_msgs(src: &str) -> Vec<String> {
    diags(src)
        .into_iter()
        .filter(|d| d.0 == "lua-arg-type")
        .map(|d| d.1)
        .collect()
}

#[test]
fn literal_argument_types_are_checked() {
    // a string where an integer is declared
    let m = type_msgs("StartMoney(PLAYER0, \"lots\")\n");
    assert_eq!(m.len(), 1, "{m:?}");
    assert!(
        m[0].contains("'gold'") && m[0].contains("integer") && m[0].contains("string"),
        "{m:?}"
    );
    // a numeric string coerces at run time, plain numbers are fine
    assert!(type_msgs(
        "StartMoney(PLAYER0, \"500\")\nStartMoney(PLAYER0, 500)\nStartMoney(PLAYER0, -5)\n"
    )
    .is_empty());
    // a number where only string literals of an alias are expected
    let m = type_msgs("AddCreatureToLevel(PLAYER0, true, 1, 1, 0)\n");
    assert_eq!(m.len(), 1, "{m:?}");
    // a boolean where an integer is expected, nil for a non-optional parameter
    assert_eq!(type_msgs("StartMoney(PLAYER0, true)\n").len(), 1);
    assert_eq!(type_msgs("StartMoney(PLAYER0, nil)\n").len(), 1);
    // a table where a number is expected
    assert_eq!(type_msgs("StartMoney(PLAYER0, {})\n").len(), 1);
    // locals initialised from a literal count, reassigned ones and unknown values do not
    assert_eq!(
        type_msgs("local g = \"many\"\nStartMoney(PLAYER0, g)\n").len(),
        1
    );
    assert!(type_msgs("local g = \"many\"\ng = 5\nStartMoney(PLAYER0, g)\n").is_empty());
    assert!(
        type_msgs("StartMoney(PLAYER0, something.unknown)\nStartMoney(PLAYER0, f())\n").is_empty()
    );
    // unions with any/unknown names pass; optional parameters take nil
    assert!(type_msgs("print(nil)\nSetGenerateSpeed(nil)\n").len() <= 1);
}

#[test]
fn literal_type_check_ignores_unannotated_and_unions() {
    // Player alias admits strings, class values and userdata
    assert!(type_msgs(
        "CreatureAvailable(PLAYER1, \"TROLL\", true, \
        2)\nCreatureAvailable(\"PLAYER1\", \"TROLL\", true, 2)\n"
    )
    .is_empty());
    // a function or a name for trigger callbacks
    assert!(type_msgs(
        "RegisterTimerEvent(function() end, 10, true)\nRegisterTimerEvent(\"cb\", 10, true)\n"
    )
    .is_empty());
}

#[test]
fn completions_carry_the_callable_flag_from_the_index() {
    let (s, o) = cur("StartMo|");
    let d = LuaDoc::parse(&s);
    let c = completions(&d, o, &ix(), &NullProvider);
    let sm = c.iter().find(|c| c.label == "StartMoney").unwrap();
    assert_eq!(sm.callable, Some(true));
    // a function without parameters
    let (s, o) = cur("local c = GetCreatureNear(1, 2)\nc:in_enemy_c|");
    let d = LuaDoc::parse(&s);
    let c = completions(&d, o, &ix(), &NullProvider);
    let k = c
        .iter()
        .find(|c| c.label == "in_enemy_custody")
        .expect("in_enemy_custody");
    assert_eq!(k.callable, Some(false));
    let (s, o) = cur("local c = GetCreatureNear(1, 2)\nc:level_|");
    let d = LuaDoc::parse(&s);
    let c = completions(&d, o, &ix(), &NullProvider);
    assert_eq!(
        c.iter().find(|c| c.label == "level_up").unwrap().callable,
        Some(true)
    );
    // plain variables are not callable
    let (s, o) = cur("local counter = 1\ncoun|");
    let d = LuaDoc::parse(&s);
    assert!(completions(&d, o, &ix(), &NullProvider)
        .iter()
        .find(|c| c.label == "counter")
        .unwrap()
        .callable
        .is_none());
}
