//! Headless renders of the script editor, to look at the widgets without the editor.
//!
//! Renders with `egui_kittest` + wgpu (an offscreen render pass; nothing is shown on the
//! desktop) and writes PNGs to `script-ui-shots` of `$KFX_TEST_TMP`, else of `target/tmp`.
//!
//!   KFX_INSTALL=<install> cargo run -p kfx-script-ui --example shot -- [MODE]
//!
//! `MODE`:
//! * none: the DK script editor on a shipped level script;
//! * `lua`: the Lua editor and the script workspace (into `lua/`);
//! * `wizard [OUT_DIR]`: the script wizard, one PNG per tab (into `wizard/`; works without an
//!   install);
//! * `wizard-sample FILE`: write the DK script of a sample two-keeper wizard setup to `FILE`.
//!
//! The KeeperFX install named by `KFX_INSTALL` provides the scripts and the command
//! constants; it is only read.
use egui::{vec2, Event, Key, Modifiers, PointerButton};
use egui_kittest::Harness;
use kfx_script::InstallProvider;
use kfx_script_ui::workspace::{ScriptFiles, ScriptWorkspace};
use kfx_script_ui::{LuaLanguage, ScriptEditor, SidePanel};
use std::path::{Path, PathBuf};

struct State {
    ed: ScriptEditor,
    host: InstallProvider,
}

/// `script-ui-shots` in `$KFX_TEST_TMP`, else in `target/tmp` of the build that made this
/// example (`target/<profile>/examples/shot`).
fn shots_dir() -> PathBuf {
    let root = std::env::var_os("KFX_TEST_TMP")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            let exe = std::env::current_exe().ok()?;
            Some(exe.parent()?.parent()?.parent()?.join("tmp"))
        })
        .expect("set KFX_TEST_TMP to an output folder");
    root.join("script-ui-shots")
}

/// The KeeperFX install named by `KFX_INSTALL`, when it is set.
fn install_opt() -> Option<PathBuf> {
    std::env::var_os("KFX_INSTALL")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// The KeeperFX install named by `KFX_INSTALL`; exits with a message when it is not set.
fn install() -> PathBuf {
    install_opt().unwrap_or_else(|| {
        eprintln!("set KFX_INSTALL to the folder of a KeeperFX install");
        std::process::exit(2)
    })
}

fn harness(state: State, w: f32, h: f32) -> Harness<'static, State> {
    Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(w, h))
        .with_pixels_per_point(1.25)
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(
            |ui, s: &mut State| {
                let State { ed, host } = s;
                ed.show(ui, host);
            },
            state,
        )
}

fn save(h: &mut Harness<'static, State>, out: &Path, name: &str) {
    h.run_steps(4);
    let img = h.render().expect("render");
    let p = out.join(name);
    img.save(&p).expect("save png");
    println!("wrote {} ({}x{})", p.display(), img.width(), img.height());
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

fn main() {
    if std::env::args().any(|a| a == "lua") {
        lua_shots();
        return;
    }
    if std::env::args().any(|a| a == "wizard-sample") {
        // `shot wizard-sample <file>`: a 2-keeper setup (PLAYER0 human, PLAYER1 computer)
        let out = std::env::args()
            .skip_while(|a| a != "wizard-sample")
            .nth(1)
            .expect("output file");
        wizard_sample(Path::new(&out));
        return;
    }
    if std::env::args().any(|a| a == "wizard") {
        // `shot wizard [out-dir]`
        let out = std::env::args()
            .skip_while(|a| a != "wizard")
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| shots_dir().join("wizard"));
        wizard_shots(&out);
        return;
    }
    let out = shots_dir();
    std::fs::create_dir_all(&out).unwrap();
    let inst = install();
    let mut host = InstallProvider::new(&inst);
    host.set_map((1..=12).collect(), Some((1..=9).collect()));
    let script = std::fs::read(inst.join("levels/standard/map00405.txt")).expect("shipped script");

    // 1. highlighting, current line, matching IF/ENDIF, outline, meters
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(&script);
    ed.analyze_now(&host);
    let lines = String::from_utf8_lossy(&script)
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let if_line = lines
        .iter()
        .position(|l| l.trim_start().to_ascii_uppercase().starts_with("IF"))
        .unwrap_or(60);
    let off: usize = lines
        .iter()
        .take(if_line)
        .map(|l| l.len() + 1)
        .sum::<usize>();
    let mut h = harness(
        State {
            ed,
            host: host.clone(),
        },
        1180.0,
        720.0,
    );
    h.state_mut().ed.goto_line(if_line.saturating_sub(8).max(1));
    h.run_steps(3);
    h.state_mut()
        .ed
        .reveal_span(kfx_script::Span::new(off, off + 2));
    save(&mut h, &out, "01-highlight.png");

    // 2. completion popup after typing ADD_CR
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(
        b"LEVEL_VERSION(1)\n\nCREATE_PARTY(RAIDERS)\n\
            \tADD_TO_PARTY(RAIDERS, KNIGHT, 1, 0, ATTACK_ENEMIES, 0)\n\nIF(PLAYER0, FLAG0 \
            == 1)\n\t\nENDIF\n",
    );
    let mut h = harness(
        State {
            ed,
            host: host.clone(),
        },
        1180.0,
        520.0,
    );
    h.state_mut().ed.set_side_panel(SidePanel::None);
    let at = 6 + 11 + 1 + 18 + 1 + 55 + 2 + 1 + 1;
    let _ = at;
    // caret on the empty indented line inside the IF block (line 7)
    h.state_mut().ed.goto_line(7);
    h.run_steps(2);
    key(&mut h, Modifiers::NONE, Key::End);
    type_text(&mut h, "ADD_CR");
    save(&mut h, &out, "02-completion.png");
    let acc_items = h.state().ed.completion_labels();
    println!("completion items: {:?}", acc_items);
    // accept with Tab: partial word replaced, parenthesis added, first argument offered
    key(&mut h, Modifiers::NONE, Key::Tab);
    println!(
        "after accept: {:?}",
        String::from_utf8_lossy(&h.state().ed.text_bytes())
    );
    save(&mut h, &out, "02b-completion-accepted.png");

    // 3. signature help inside ADD_CREATURE_TO_LEVEL(
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(b"LEVEL_VERSION(1)\n\nIF(PLAYER0, FLAG0 == 1)\n\t\nENDIF\n");
    let mut h = harness(
        State {
            ed,
            host: host.clone(),
        },
        1180.0,
        420.0,
    );
    h.state_mut().ed.set_side_panel(SidePanel::None);
    h.state_mut().ed.goto_line(4);
    h.run_steps(2);
    key(&mut h, Modifiers::NONE, Key::End);
    type_text(&mut h, "ADD_CREATURE_TO_LEVEL(PLAYER_GOOD, DRAGON, ");
    h.state_mut().ed.completion_clear_for_test();
    h.run_steps(2);
    save(&mut h, &out, "03-signature-help.png");

    // 4. errors: squiggles, gutter markers, problems panel
    let bad = b"LEVEL_VERSION(1)\n\nSTART_MONEY(PLAYER0, 5000)\n\
        ADD_CREATURE_TO_LEVEL(PLAYER0, DRAGONN, 3, 1, 1, 0)\nSET_FLAG(PLAYER0, FLAG9, \
        1)\nBOGUS_COMMAND(1, 2)\nIF(PLAYER0, FLAG0 == 1)\n\
        \tADD_PARTY_TO_LEVEL(PLAYER_GOOD, NO_SUCH_PARTY, 4, 1)\n\
        \tROOM_AVAILABLE(PLAYER0, NOTAROOM, 1, 1)\n\tWIN_GAME\nREM the IF above is \
        never closed\n";
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(bad);
    ed.analyze_now(&host);
    let mut h = harness(
        State {
            ed,
            host: host.clone(),
        },
        1180.0,
        600.0,
    );
    h.state_mut().ed.set_side_panel(SidePanel::None);
    h.state_mut().ed.goto_line(4);
    h.run_steps(3);
    // hover the unknown creature to get the diagnostic card
    save(&mut h, &out, "04-errors.png");
    let pos = h
        .state()
        .ed
        .test_char_pos(h.state().ed.test_offset_of(b"DRAGONN"));
    if let Some(p) = pos {
        h.event(Event::PointerMoved(p));
        h.run_steps(3);
        std::thread::sleep(std::time::Duration::from_millis(500));
        h.event(Event::PointerMoved(p + vec2(1.0, 0.0)));
        h.run_steps(6);
        save(&mut h, &out, "05-hover-diagnostic.png");
    }

    // 5. command hover docs
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(
        b"LEVEL_VERSION(1)\n\nADD_CREATURE_TO_LEVEL(PLAYER_GOOD, DRAGON, 3, 1, 1, 0)\n",
    );
    let mut h = harness(
        State {
            ed,
            host: host.clone(),
        },
        1180.0,
        560.0,
    );
    h.state_mut().ed.set_side_panel(SidePanel::None);
    h.state_mut().ed.set_problems_panel(false);
    h.run_steps(3);
    if let Some(p) = h
        .state()
        .ed
        .test_char_pos(h.state().ed.test_offset_of(b"ADD_CREATURE") + 3)
    {
        h.event(Event::PointerMoved(p));
        h.run_steps(3);
        std::thread::sleep(std::time::Duration::from_millis(500));
        h.event(Event::PointerMoved(p + vec2(1.0, 0.0)));
        h.run_steps(6);
        save(&mut h, &out, "06-hover-docs.png");
    }

    // 6. command reference panel
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(&script);
    ed.analyze_now(&host);
    let mut h = harness(
        State {
            ed,
            host: host.clone(),
        },
        1280.0,
        760.0,
    );
    h.state_mut().ed.set_side_panel(SidePanel::Commands);
    h.state_mut().ed.set_reference_search_for_test("creature");
    h.state_mut()
        .ed
        .select_reference_for_test("ADD_CREATURE_TO_LEVEL");
    h.state_mut().ed.goto_line(30);
    save(&mut h, &out, "07-command-reference.png");

    // 7. find / replace bar and outline panel
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(&script);
    ed.analyze_now(&host);
    let mut h = harness(
        State {
            ed,
            host: host.clone(),
        },
        1280.0,
        700.0,
    );
    h.state_mut().ed.set_problems_panel(false);
    h.state_mut().ed.goto_line(14);
    h.run_steps(2);
    key(&mut h, Modifiers::COMMAND, Key::H);
    type_text(&mut h, "CREATURE_AVAILABLE");
    h.state_mut().ed.set_replacement_for_test("CREATURE_AVAIL");
    save(&mut h, &out, "08-find-replace-outline.png");

    // keep PointerButton import used
    let _ = PointerButton::Primary;
}

// ---------------------------------------------------------------------------------------------
// Lua

const LEVEL_LUA: &str = "-- map00042.lua: the siege of Stonegate
---Called once when the level starts.
function OnGameStart()
\tStartMoney(PLAYER0, 3000)
\tMaxCreatures(PLAYER0, 25)
\tRoomAvailable(PLAYER0, \"TREASURE\", 1, true)
\tRoomAvailable(PLAYER0, \"LAIR\", 1, true)
\tAddCreatureToLevel(PLAYER_GOOD, \"KNIGHT\", 3, 4, 0)
\tGame.waves = 0
\t-- a wave every minute (20 turns per second)
\tRegisterTimerEvent(SpawnWave, 20 * 60, true)
\tRegisterOnActionPointEvent(
\t\tfunction(eventData, triggerData)
\t\t\tRevealMapLocation(PLAYER0, 2, 9)
\t\t\tQuickObjective(\"Find the hidden vault\", 2)
\t\tend,
\t\t2, PLAYER0)
end

---@param eventData table
---@param triggerData table
function SpawnWave(eventData, triggerData)
\tGame.waves = Game.waves + 1
\tlocal count = 2 + Game.waves
\tfor i = 1, count do
\t\tAddCreatureToLevel(PLAYER_GOOD, \"ARCHER\", -1, 1, 0)
\tend
end

function OnCreatureDeath(unit)
\tif unit.model == \"KNIGHT\" then
\t\tQuickObjective(\"A knight has fallen\")
\tend
end
";

const BAD_LUA: &str = "function OnGameStart()
\tAddCreatureToLevel(PLAYER_GOOD, \"DRAGONN\", 1, 3, 0)
\tlocal function onWave()
\t\tQuickObjective(\"Wave\")
\tend
\tRegisterTimerEvent(onWave, 400, true)
\tif Game.waves then
\t\tStartMoney(PLAYER0, 5000
end
\tBogusFunction(1)
end
";

struct LuaState {
    ed: ScriptEditor,
    host: InstallProvider,
}

fn lua_harness(
    ed: ScriptEditor,
    host: InstallProvider,
    w: f32,
    h: f32,
) -> Harness<'static, LuaState> {
    Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(w, h))
        .with_pixels_per_point(1.25)
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(
            |ui, s: &mut LuaState| {
                let LuaState { ed, host } = s;
                ed.show(ui, host);
            },
            LuaState { ed, host },
        )
}

fn lua_save<S>(h: &mut Harness<'static, S>, out: &Path, name: &str) {
    h.run_steps(4);
    let img = h.render().expect("render");
    let p = out.join(name);
    img.save(&p).expect("save png");
    println!("wrote {} ({}x{})", p.display(), img.width(), img.height());
}

fn lua_key(h: &mut Harness<'static, LuaState>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run_steps(3);
}

fn lua_type(h: &mut Harness<'static, LuaState>, text: &str) {
    for ch in text.chars() {
        h.event(Event::Text(ch.to_string()));
        h.run_steps(2);
    }
}

fn lua_editor(text: &str) -> ScriptEditor {
    let mut ed = ScriptEditor::with_language(Box::new(LuaLanguage::new()));
    ed.set_text_bytes(text.as_bytes());
    ed
}

struct DiskFiles {
    dir: PathBuf,
}

impl ScriptFiles for DiskFiles {
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join(name)).ok()
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        std::fs::write(self.dir.join(name), bytes).map_err(|e| e.to_string())
    }
    fn read_library(&self, name: &str) -> Option<Vec<u8>> {
        if name == "bindings/setup.lua" {
            Some(
                b"---Set the starting gold of a player.\n---@param player \
                    playerrange\n---@param gold integer\nfunction StartMoney(player, gold) end\n"
                    .to_vec(),
            )
        } else {
            None
        }
    }
}

struct WsState {
    ws: ScriptWorkspace,
    host: InstallProvider,
    files: DiskFiles,
}

fn lua_shots() {
    let out = shots_dir().join("lua");
    std::fs::create_dir_all(&out).unwrap();
    let inst = install();
    let mut host = InstallProvider::new(&inst);
    host.set_map((1..=12).collect(), Some((1..=9).collect()));

    // 1. highlighting of a realistic level script
    let mut ed = lua_editor(LEVEL_LUA);
    ed.analyze_now(&host);
    ed.set_problems_panel(false);
    let mut h = lua_harness(ed, host.clone(), 1180.0, 780.0);
    h.state_mut().ed.goto_line(3);
    h.run_steps(3);
    lua_save(&mut h, &out, "01-highlight.png");

    // 2. member completion after `PLAYER0:`
    let mut ed = lua_editor("function OnGameStart()\n\tStartMoney(PLAYER0, 3000)\n\t\nend\n");
    ed.set_side_panel(SidePanel::None);
    ed.set_problems_panel(false);
    let mut h = lua_harness(ed, host.clone(), 1000.0, 520.0);
    h.state_mut().ed.goto_line(3);
    h.run_steps(3);
    lua_key(&mut h, Modifiers::NONE, Key::End);
    lua_type(&mut h, "PLAYER0:");
    println!(
        "PLAYER0: members: {:?}",
        h.state()
            .ed
            .completion_labels()
            .iter()
            .take(12)
            .collect::<Vec<_>>()
    );
    lua_save(&mut h, &out, "02-member-completion-player.png");

    // 3. member completion on a Creature variable
    let mut ed = lua_editor("function OnCreatureDeath(unit)\n\tlocal fallen = unit\n\t\nend\n");
    ed.set_side_panel(SidePanel::None);
    ed.set_problems_panel(false);
    let mut h = lua_harness(ed, host.clone(), 1000.0, 520.0);
    h.state_mut().ed.goto_line(3);
    h.run_steps(3);
    lua_key(&mut h, Modifiers::NONE, Key::End);
    lua_type(&mut h, "fallen:");
    println!(
        "Creature members: {:?}",
        h.state()
            .ed
            .completion_labels()
            .iter()
            .take(12)
            .collect::<Vec<_>>()
    );
    h.run_steps(12);
    lua_save(&mut h, &out, "03-member-completion-creature.png");

    // 4. alias literal completion inside a call
    let mut ed = lua_editor("function OnGameStart()\n\t\nend\n");
    ed.set_side_panel(SidePanel::None);
    ed.set_problems_panel(false);
    let mut h = lua_harness(ed, host.clone(), 1000.0, 520.0);
    h.state_mut().ed.goto_line(2);
    h.run_steps(3);
    lua_key(&mut h, Modifiers::NONE, Key::End);
    lua_type(&mut h, "AddCreatureToLevel(PLAYER_GOOD, \"DR");
    println!("alias items: {:?}", h.state().ed.completion_labels());
    lua_save(&mut h, &out, "04-alias-literal-completion.png");

    // 5. signature help
    let mut ed = lua_editor("function OnGameStart()\n\t\nend\n");
    ed.set_side_panel(SidePanel::None);
    ed.set_problems_panel(false);
    let mut h = lua_harness(ed, host.clone(), 1000.0, 420.0);
    h.state_mut().ed.goto_line(2);
    h.run_steps(3);
    lua_key(&mut h, Modifiers::NONE, Key::End);
    lua_type(&mut h, "AddCreatureToLevel(PLAYER_GOOD, \"KNIGHT\", ");
    h.state_mut().ed.completion_clear_for_test();
    lua_save(&mut h, &out, "05-signature-help.png");

    // 6. errors: squiggles, gutter, problems panel
    let mut ed = lua_editor(BAD_LUA);
    ed.analyze_now(&host);
    ed.set_side_panel(SidePanel::None);
    let mut h = lua_harness(ed, host.clone(), 1180.0, 600.0);
    h.state_mut().ed.goto_line(1);
    h.run_steps(3);
    for d in h.state().ed.diagnostics() {
        println!(
            "diag line {} {:?} {}: {}",
            d.line, d.severity, d.code, d.message
        );
    }
    lua_save(&mut h, &out, "06-errors.png");

    // 7. hover on an API function
    let mut ed = lua_editor(LEVEL_LUA);
    ed.set_side_panel(SidePanel::None);
    ed.set_problems_panel(false);
    let mut h = lua_harness(ed, host.clone(), 1180.0, 640.0);
    h.state_mut().ed.goto_line(5);
    h.run_steps(3);
    if let Some(p) = h
        .state()
        .ed
        .test_char_pos(h.state().ed.test_offset_of(b"AddCreatureToLevel") + 4)
    {
        h.event(Event::PointerMoved(p));
        h.run_steps(3);
        std::thread::sleep(std::time::Duration::from_millis(500));
        h.event(Event::PointerMoved(p + vec2(1.0, 0.0)));
        h.run_steps(6);
        lua_save(&mut h, &out, "07-hover-api.png");
    }

    // 8. the workspace with both tabs, a library tab, combined problems
    let dir = shots_dir().join("lua").join("ws-files");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("map00042.txt"),
        std::fs::read(inst.join("levels/standard/map00405.txt")).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("map00042.lua"), BAD_LUA).unwrap();
    let files = DiskFiles { dir: dir.clone() };
    let mut ws = ScriptWorkspace::new(42);
    ws.load(&files);
    ws.analyze_now(&host);
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(1.25)
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(
            |ui, s: &mut WsState| {
                let WsState { ws, host, files } = s;
                ws.show(ui, host, files);
            },
            WsState {
                ws,
                host: host.clone(),
                files,
            },
        );
    h.run_steps(4);
    lua_save(&mut h, &out, "08a-workspace-dk-tab.png");
    h.state_mut().ws.activate("map00042.lua");
    h.state_mut().ws.active_editor_mut().goto_line(1);
    h.run_steps(4);
    // make the DK script dirty too
    h.state_mut()
        .ws
        .editor_mut("map00042.txt")
        .unwrap()
        .insert_at_caret("REM edited\n");
    h.run_steps(4);
    lua_save(&mut h, &out, "08-workspace.png");

    // 8b. a library tab opened by go to definition
    {
        let st = h.state_mut();
        st.ws.open_library(
            "bindings/setup.lua",
            Some(kfx_script::Span::new(92, 102)),
            &st.files,
        );
    }
    h.run_steps(6);
    lua_save(&mut h, &out, "08b-workspace-library-tab.png");

    // 8c. a missing Lua file: the create panel
    let dir2 = shots_dir().join("lua").join("ws-files-missing");
    std::fs::create_dir_all(&dir2).unwrap();
    std::fs::write(
        dir2.join("map00043.txt"),
        std::fs::read(inst.join("levels/standard/map00405.txt")).unwrap(),
    )
    .unwrap();
    let _ = std::fs::remove_file(dir2.join("map00043.lua"));
    let files = DiskFiles { dir: dir2 };
    let mut ws = ScriptWorkspace::new(43);
    ws.load(&files);
    ws.activate("map00043.lua");
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1000.0, 560.0))
        .with_pixels_per_point(1.25)
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(
            |ui, s: &mut WsState| {
                let WsState { ws, host, files } = s;
                ws.show(ui, host, files);
            },
            WsState {
                ws,
                host: host.clone(),
                files,
            },
        );
    lua_save(&mut h, &out, "08c-workspace-missing-lua.png");

    // 9. snippet panel (Lua)
    let mut ed = lua_editor(LEVEL_LUA);
    ed.set_side_panel(SidePanel::Snippets);
    ed.set_problems_panel(false);
    let mut h = lua_harness(ed, host.clone(), 1280.0, 780.0);
    h.state_mut().ed.select_snippet_for_test("hero-waves");
    h.state_mut().ed.goto_line(3);
    lua_save(&mut h, &out, "09-snippets.png");

    // 9b. snippet inserted, first placeholder selected
    let mut ed =
        lua_editor("-- map00042.lua\n\nfunction OnGameStart()\n\tStartMoney(PLAYER0, 3000)\nend\n");
    ed.set_side_panel(SidePanel::None);
    ed.set_problems_panel(false);
    let mut h = lua_harness(ed, host.clone(), 1100.0, 560.0);
    h.state_mut().ed.goto_line(4);
    h.run_steps(3);
    let snip = kfx_script_ui::snippets::LIBRARY
        .iter()
        .find(|s| s.id == "ambush")
        .unwrap();
    h.state_mut().ed.insert_library_snippet(snip);
    h.run_steps(5);
    lua_save(&mut h, &out, "09b-snippet-inserted.png");

    // 10. light theme
    let mut ed = lua_editor(LEVEL_LUA);
    ed.set_light(true);
    ed.analyze_now(&host);
    let mut h = lua_harness(ed, host.clone(), 1180.0, 780.0);
    h.state_mut().ed.goto_line(14);
    h.run_steps(3);
    lua_save(&mut h, &out, "10-light-theme.png");

    // 10b. light theme with the errors and a bigger font
    let mut ed = lua_editor(BAD_LUA);
    ed.set_light(true);
    ed.set_font_size(17.0);
    ed.analyze_now(&host);
    ed.set_side_panel(SidePanel::None);
    let mut h = lua_harness(ed, host.clone(), 1180.0, 600.0);
    h.state_mut().ed.goto_line(1);
    lua_save(&mut h, &out, "10b-light-errors-large-font.png");

    // 10c. a long completion list (DK) scrolls
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(b"LEVEL_VERSION(1)\n\n");
    ed.set_side_panel(SidePanel::None);
    ed.set_problems_panel(false);
    let mut h = lua_harness(ed, host.clone(), 1000.0, 520.0);
    h.state_mut().ed.goto_line(3);
    h.run_steps(3);
    lua_type(&mut h, "ADD_");
    println!("DK items: {}", h.state().ed.completion_labels().len());
    lua_save(&mut h, &out, "10c-dk-long-completion.png");

    // 11. DK editor still fine: light theme + snippet panel in DK form
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(&std::fs::read(inst.join("levels/standard/map00405.txt")).unwrap());
    ed.set_light(true);
    ed.set_side_panel(SidePanel::Snippets);
    ed.select_snippet_for_test("win-lose");
    let mut h = lua_harness(ed, host.clone(), 1280.0, 760.0);
    lua_save(&mut h, &out, "11-dk-light-snippets.png");
}

struct WizState {
    ws: ScriptWorkspace,
    files: MemFiles,
    host: InstallProvider,
}

#[derive(Default)]
struct MemFiles(std::cell::RefCell<std::collections::HashMap<String, Vec<u8>>>);

impl ScriptFiles for MemFiles {
    fn read(&self, n: &str) -> Option<Vec<u8>> {
        self.0.borrow().get(n).cloned()
    }
    fn write(&self, n: &str, b: &[u8]) -> Result<(), String> {
        self.0.borrow_mut().insert(n.into(), b.to_vec());
        Ok(())
    }
}

/// The script wizard window over the workspace: one PNG per tab.
fn wizard_shots(out: &Path) {
    use kfx_script_ui::wizard::{Availability, Computer, Player, ScriptLang, WizardTab};
    std::fs::create_dir_all(out).unwrap();
    let host = if let Some(inst) = install_opt() {
        InstallProvider::new(&inst)
    } else {
        InstallProvider::default()
    };
    let files = MemFiles::default();
    let script = "REM map00007.txt: level script\nLEVEL_VERSION(1)\n\n\
        START_MONEY(PLAYER0, 3000)\nROOM_AVAILABLE(ALL_PLAYERS, TEMPLE, 1, 0)\n\
        COMPUTER_PLAYER(PLAYER1, 0)\nADD_CREATURE_TO_POOL(TROLL, 5)\n";
    files
        .0
        .borrow_mut()
        .insert("map00007.txt".into(), script.as_bytes().to_vec());
    let mut ws = ScriptWorkspace::new(7);
    ws.load(&files);
    ws.open_wizard(ScriptLang::Dk);
    {
        let w = ws.wizard_mut().unwrap();
        let m = w.model_mut();
        m.player_mut(Player::P0).start_money = Some(12345);
        m.player_mut(Player::P0)
            .rooms
            .insert("TREASURE".into(), Availability::Available);
        m.player_mut(Player::P0)
            .rooms
            .insert("LAIR".into(), Availability::Available);
        m.player_mut(Player::P0)
            .rooms
            .insert("TEMPLE".into(), Availability::Available);
        m.player_mut(Player::P1)
            .rooms
            .insert("PRISON".into(), Availability::Unavailable);
        m.player_mut(Player::P0)
            .powers
            .insert("POWER_IMP".into(), Availability::Available);
        m.player_mut(Player::P0)
            .powers
            .insert("POWER_LIGHTNING".into(), Availability::Researchable);
        m.player_mut(Player::P1).computer = Some(Computer::Model(4));
        m.player_mut(Player::P0).win_when_enemies_destroyed = true;
        m.player_mut(Player::P0).lose_when_heart_destroyed = true;
        m.player_mut(Player::P0).creatures.insert(
            "DRAGON".into(),
            kfx_script_ui::wizard::Attraction {
                allowed: true,
                forced: 1,
            },
        );
        m.pool.insert("DRAGON".into(), 2);
        m.player_mut(Player::P0).traps.insert(
            "BOULDER".into(),
            kfx_script_ui::wizard::Stock {
                buildable: true,
                amount: 2,
            },
        );
        m.alliances.insert((Player::P1, Player::P2), 3);
        w.set_player_shown(Player::Good, true);
        w.set_player_shown(Player::P2, true);
    }
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1200.0, 800.0))
        .with_pixels_per_point(1.25)
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(
            |ui, s: &mut WizState| {
                s.ws.show(ui, &s.host, &s.files);
            },
            WizState { ws, files, host },
        );
    for (i, t) in WizardTab::ALL.iter().enumerate() {
        h.state_mut().ws.wizard_mut().unwrap().set_tab(*t);
        h.run_steps(6);
        let img = h.render().expect("render");
        let p = out.join(format!(
            "wizard-{:02}-{}.png",
            i + 1,
            t.title().to_ascii_lowercase().replace([' ', '&'], "")
        ));
        img.save(&p).expect("save png");
        println!("wrote {} ({}x{})", p.display(), img.width(), img.height());
    }
}

fn wizard_sample(out: &Path) {
    use kfx_script_ui::wizard::{Computer, Player, WizardModel};
    let script = "LEVEL_VERSION(1)\n";
    let mut m = WizardModel::read(script);
    m.player_mut(Player::P0).start_money = Some(12345);
    m.player_mut(Player::P1).computer = Some(Computer::Model(0));
    m.player_mut(Player::P0).win_when_enemies_destroyed = true;
    m.player_mut(Player::P0).lose_when_heart_destroyed = true;
    let plan = m.plan(script.as_bytes());
    assert!(plan.conflicts.is_empty() && plan.notes.is_empty());
    std::fs::write(out, plan.apply_to(script.as_bytes())).expect("write sample");
    println!("wrote {}", out.display());
}
