//! Script wizard: reading existing values, writing one block, conflicts, the form, the workspace.
use egui::{vec2, Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use kfx_script::{NullProvider, Span, StaticProvider};
use kfx_script_ui::wizard::*;
use kfx_script_ui::workspace::{ScriptFiles, ScriptWorkspace};
use kfx_script_ui::{ScriptEditor, SidePanel};
use std::cell::RefCell;
use std::collections::HashMap;

const SCRIPT: &str = "REM My level\nREM by someone\nLEVEL_VERSION(1)\n\n\
    SET_GENERATE_SPEED(500)\nSTART_MONEY(PLAYER0, 3000)\n\
    ROOM_AVAILABLE(ALL_PLAYERS, TEMPLE, 1, 0)\nMAGIC_AVAILABLE(PLAYER1, POWER_IMP, \
    1, 1)\nTRAP_AVAILABLE(PLAYER0, BOULDER, 1, 2)\nDOOR_AVAILABLE(PLAYER0, WOOD, 1, \
    0)\nCREATURE_AVAILABLE(PLAYER0, TROLL, 1, 2)\nADD_CREATURE_TO_POOL(TROLL, 3)\n\
    ADD_CREATURE_TO_POOL(TROLL, 4)\nCOMPUTER_PLAYER(PLAYER1, 4)\n\
    ALLY_PLAYERS(PLAYER1, PLAYER0, 3)\nMAX_CREATURES(PLAYER0, 30)\n\nIF(PLAYER0, \
    FLAG0 == 1)\n\tSTART_MONEY(PLAYER0, 99999)\n\tROOM_AVAILABLE(PLAYER0, LAIR, 1, \
    1)\nENDIF\nIF(PLAYER0, ALL_DUNGEONS_DESTROYED == 1)\n\tWIN_GAME\nENDIF\n";

fn s(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

#[test]
fn read_recognises_unconditional_values() {
    let m = WizardModel::read(SCRIPT);
    assert_eq!(m.version, 1);
    assert_eq!(m.generate_speed, Some(500));
    let p0 = m.player(Player::P0);
    assert_eq!(
        p0.start_money,
        Some(3000),
        "the START_MONEY inside IF is ignored"
    );
    assert_eq!(p0.max_creatures, Some(30));
    assert_eq!(p0.rooms.get("TEMPLE"), Some(&Availability::Researchable));
    assert_eq!(p0.rooms.get("LAIR"), None, "conditional");
    assert_eq!(
        p0.traps.get("BOULDER"),
        Some(&Stock {
            buildable: true,
            amount: 2
        })
    );
    assert_eq!(
        p0.doors.get("WOOD"),
        Some(&Stock {
            buildable: true,
            amount: 0
        })
    );
    assert_eq!(
        p0.creatures.get("TROLL"),
        Some(&Attraction {
            allowed: true,
            forced: 2
        })
    );
    assert!(p0.win_when_enemies_destroyed);
    assert!(!p0.lose_when_heart_destroyed);
    // ALL_PLAYERS reaches every slot
    for p in Player::ALL {
        assert_eq!(
            m.player(p).rooms.get("TEMPLE"),
            Some(&Availability::Researchable),
            "{p:?}"
        );
    }
    let p1 = m.player(Player::P1);
    assert_eq!(p1.powers.get("POWER_IMP"), Some(&Availability::Available));
    assert_eq!(p1.computer, Some(Computer::Model(4)));
    assert_eq!(m.pool.get("TROLL"), Some(&7), "pool additions add up");
    assert_eq!(
        m.alliances.get(&(Player::P0, Player::P1)),
        Some(&3),
        "pair normalised"
    );
    // the read model asks for no block: the script already says it all
    let plan = m.plan(SCRIPT.as_bytes());
    assert!(plan.lines.is_empty(), "{:?}", plan.lines);
    assert!(!plan.changes(SCRIPT.as_bytes()));
    assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
}

#[test]
fn level_version_0_argument_forms() {
    let src = "CREATURE_AVAILABLE(PLAYER0, TROLL, 0, 1)\n\
        ALLY_PLAYERS(PLAYER0, PLAYER1)\nCOMPUTER_PLAYER(PLAYER1, 0)\n";
    let m = WizardModel::read(src);
    assert_eq!(m.version, 0);
    assert_eq!(
        m.player(Player::P0).creatures.get("TROLL"),
        Some(&Attraction {
            allowed: true,
            forced: 0
        }),
        "v0 takes the 4th argument"
    );
    assert_eq!(m.alliances.get(&(Player::P0, Player::P1)), Some(&1));
    let mut m2 = m.clone();
    m2.player_mut(Player::P0).creatures.insert(
        "DRAGON".into(),
        Attraction {
            allowed: true,
            forced: 0,
        },
    );
    m2.alliances.insert((Player::P0, Player::P2), 1);
    m2.player_mut(Player::P2).computer = Some(Computer::Roaming);
    m2.player_mut(Player::P4).start_money = Some(10);
    let plan = m2.plan(src.as_bytes());
    let block = plan.lines.join("\n");
    assert!(
        block.contains("CREATURE_AVAILABLE(PLAYER0, DRAGON, 1, 1)"),
        "{block}"
    );
    assert!(
        block.contains("ALLY_PLAYERS(PLAYER0, PLAYER2)\n"),
        "{block}"
    );
    assert!(
        !block.contains("ROAMING") && !block.contains("PLAYER4"),
        "{block}"
    );
    assert!(
        plan.notes.iter().any(|n| n.contains("ROAMING")),
        "{:?}",
        plan.notes
    );
    assert!(
        plan.notes.iter().any(|n| n.contains("PLAYER4")),
        "{:?}",
        plan.notes
    );
}

#[test]
fn new_block_goes_after_the_header_and_nothing_else_changes() {
    let src = SCRIPT.as_bytes();
    let mut m = WizardModel::read(SCRIPT);
    m.player_mut(Player::P2).start_money = Some(12345);
    m.player_mut(Player::P2)
        .rooms
        .insert("GARDEN".into(), Availability::Available);
    m.player_mut(Player::P2).lose_when_heart_destroyed = true;
    let plan = m.plan(src);
    assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
    let header = "REM My level\nREM by someone\nLEVEL_VERSION(1)\n";
    assert_eq!(
        plan.span,
        Span::new(header.len(), header.len()),
        "after LEVEL_VERSION"
    );
    let out = plan.apply_to(src);
    let text = s(&out);
    let expected_block = "REM ---- kfx-editor wizard begin ----\n\
        REM Level setup from the script wizard. Lines between these markers are rewritten when it is applied again.\n\
        START_MONEY(PLAYER2, 12345)\n\
        ROOM_AVAILABLE(PLAYER2, GARDEN, 1, 1)\n\
        IF(PLAYER2, DUNGEON_DESTROYED == 1)\n\tLOSE_GAME\nENDIF\n\
        REM ---- kfx-editor wizard end ----\n";
    assert_eq!(
        text,
        format!("{header}{expected_block}{}", &SCRIPT[header.len()..])
    );
    // read back: the same model
    let back = WizardModel::read(&text);
    assert_eq!(back, m);
    // applying again changes nothing
    assert!(!m.plan(&out).changes(&out));
}

#[test]
fn block_is_replaced_in_place_and_the_rest_stays_byte_identical() {
    let mut m = WizardModel::read(SCRIPT);
    m.player_mut(Player::P2).start_money = Some(100);
    let once = m.plan(SCRIPT.as_bytes()).apply_to(SCRIPT.as_bytes());
    // the user edits around the block
    let edited = s(&once).replace(
        "SET_GENERATE_SPEED(500)",
        "SET_GENERATE_SPEED(500)\nREM user line",
    ) + "REM tail\n";
    let mut m2 = WizardModel::read(&edited);
    assert_eq!(
        m2.player(Player::P2).start_money,
        Some(100),
        "values inside the block are recognised"
    );
    m2.player_mut(Player::P2).start_money = Some(20000);
    m2.player_mut(Player::P3).max_creatures = Some(12);
    let plan = m2.plan(edited.as_bytes());
    let b0 = edited.find("REM ---- kfx-editor wizard begin").unwrap();
    let b1 = edited
        .find("REM ---- kfx-editor wizard end ----\n")
        .unwrap()
        + "REM ---- kfx-editor wizard end ----\n".len();
    assert_eq!(plan.span, Span::new(b0, b1), "the old block is the span");
    let out = s(&plan.apply_to(edited.as_bytes()));
    assert_eq!(&out[..b0], &edited[..b0]);
    let end_mark = "REM ---- kfx-editor wizard end ----\n";
    let tail_new = &out[out.find(end_mark).unwrap() + end_mark.len()..];
    assert_eq!(
        tail_new,
        &edited[b1..],
        "everything after the block is untouched"
    );
    assert!(
        out.contains("START_MONEY(PLAYER2, 20000)\nMAX_CREATURES(PLAYER3, 12)\n")
            || out.contains("START_MONEY(PLAYER2, 20000)"),
        "{out}"
    );
    assert!(!out.contains("START_MONEY(PLAYER2, 100)"));
    assert_eq!(out.matches("kfx-editor wizard begin").count(), 1);
    // unset everything the block set: the block goes away, the rest is the user's script
    let mut m3 = WizardModel::read(&out);
    m3.player_mut(Player::P2).start_money = None;
    m3.player_mut(Player::P3).max_creatures = None;
    let gone = s(&m3.plan(out.as_bytes()).apply_to(out.as_bytes()));
    assert_eq!(gone, edited[..b0].to_string() + &edited[b1..]);
}

#[test]
fn conflicts_are_reported_not_edited() {
    let mut m = WizardModel::read(SCRIPT);
    // START_MONEY adds up: the block writes the difference
    m.player_mut(Player::P0).start_money = Some(5000);
    // a set-type value also set outside (and after the block)
    m.player_mut(Player::P0)
        .rooms
        .insert("TEMPLE".into(), Availability::Available);
    // switched off in the form but set outside
    m.player_mut(Player::P1).computer = None;
    let plan = m.plan(SCRIPT.as_bytes());
    let block = plan.lines.join("\n");
    assert!(block.contains("START_MONEY(PLAYER0, 2000)"), "{block}");
    assert!(
        block.contains("ROOM_AVAILABLE(PLAYER0, TEMPLE, 1, 1)"),
        "{block}"
    );
    let lines: Vec<(usize, &str)> = plan
        .conflicts
        .iter()
        .map(|c| (c.line, c.what.as_str()))
        .collect();
    assert!(lines.contains(&(6, "START_MONEY for PLAYER0")), "{lines:?}");
    assert!(
        lines.contains(&(7, "ROOM_AVAILABLE TEMPLE for PLAYER0")),
        "{lines:?}"
    );
    assert!(
        lines.contains(&(14, "COMPUTER_PLAYER for PLAYER1")),
        "{lines:?}"
    );
    let room = plan.conflicts.iter().find(|c| c.line == 7).unwrap();
    assert!(room.message.contains("overrides"), "{}", room.message);
    assert_eq!(room.text, "ROOM_AVAILABLE(ALL_PLAYERS, TEMPLE, 1, 0)");
    // the conflicting lines are still there, unchanged
    let out = s(&plan.apply_to(SCRIPT.as_bytes()));
    for l in SCRIPT.lines() {
        assert!(out.lines().any(|o| o == l), "line {l:?} kept");
    }
    assert_eq!(
        out.lines().count(),
        SCRIPT.lines().count() + plan.lines.len()
    );
}

#[test]
fn crlf_files_keep_crlf_and_insert_after_a_last_line_without_newline() {
    let src = b"REM x\r\nLEVEL_VERSION(1)\r\nSTART_MONEY(PLAYER1, 1)\r\n".to_vec();
    let mut m = WizardModel::read_bytes(&src);
    m.player_mut(Player::P0).max_creatures = Some(5);
    let out = m.plan(&src).apply_to(&src);
    assert!(
        !out.windows(2).any(|w| w[1] == b'\n' && w[0] != b'\r'),
        "{}",
        s(&out)
    );
    assert!(s(&out)
        .starts_with("REM x\r\nLEVEL_VERSION(1)\r\nREM ---- kfx-editor wizard begin ----\r\n"));
    assert!(s(&out).ends_with(
        "REM ---- kfx-editor wizard end ----\r\n\
            START_MONEY(PLAYER1, 1)\r\n"
    ));
    // header only, no trailing newline
    let src = b"LEVEL_VERSION(1)";
    let out = s(&m.plan(src).apply_to(src));
    assert!(
        out.starts_with("LEVEL_VERSION(1)\nREM ---- kfx-editor wizard begin ----\n"),
        "{out}"
    );
    // empty script
    let out = s(&m.plan(b"").apply_to(b""));
    assert!(
        out.starts_with("REM ---- kfx-editor wizard begin ----\n")
            && out.contains("MAX_CREATURES(PLAYER0, 5)"),
        "{out}"
    );
}

#[test]
fn lua_block_round_trip() {
    let src = "-- map00007.lua\n-- my level\n\nfunction \
        OnGameStart()\n    StartMoney(PLAYER0, 3000)\nend\n";
    let mut m = WizardModel::read_lua(src);
    assert_eq!(m.player(Player::P0).start_money, Some(3000));
    m.player_mut(Player::P1).computer = Some(Computer::Model(0));
    m.player_mut(Player::P0)
        .rooms
        .insert("TEMPLE".into(), Availability::Researchable);
    m.player_mut(Player::P0)
        .powers
        .insert("POWER_IMP".into(), Availability::Available);
    m.player_mut(Player::P0).traps.insert(
        "BOULDER".into(),
        Stock {
            buildable: true,
            amount: 1,
        },
    );
    m.player_mut(Player::P0).creatures.insert(
        "TROLL".into(),
        Attraction {
            allowed: true,
            forced: 0,
        },
    );
    m.pool.insert("TROLL".into(), 4);
    m.alliances.insert((Player::P0, Player::P1), 1);
    m.player_mut(Player::P0).win_when_enemies_destroyed = true;
    m.player_mut(Player::P0).lose_when_heart_destroyed = true;
    let plan = m.plan_lua(src.as_bytes());
    let block = plan.lines.join("\n");
    for want in [
        "function KfxWizardSetup()",
        "    ComputerPlayer(PLAYER1, 0)",
        "    RoomAvailable(PLAYER0, \"TEMPLE\", 1, false)",
        "    MagicAvailable(PLAYER0, \"POWER_IMP\", true, true)",
        "    TrapAvailable(PLAYER0, \"BOULDER\", true, 1)",
        "    CreatureAvailable(PLAYER0, \"TROLL\", true, 0)",
        "    AddCreatureToPool(\"TROLL\", 4)",
        "    AllyPlayers(PLAYER0, PLAYER1, 1)",
        "    RegisterOnConditionEvent(\"KfxWizardWin_PLAYER0\", \"KfxWizardWinCheck_PLAYER0\")",
        "    RegisterDungeonDestroyedEvent(\"KfxWizardLose_PLAYER0\", PLAYER0)",
        "    return PLAYER0.ALL_DUNGEONS_DESTROYED == 1",
        "    WinGame(PLAYER0)",
        "    LoseGame(PLAYER0)",
    ] {
        assert!(block.contains(want), "missing {want:?} in\n{block}");
    }
    assert!(!block.contains("StartMoney"), "already set outside");
    assert!(!block.contains("function OnGameStart"), "the file has one");
    assert!(
        plan.notes.iter().any(|n| n.contains("KfxWizardSetup()")),
        "{:?}",
        plan.notes
    );
    let out = s(&plan.apply_to(src.as_bytes()));
    assert!(
        out.starts_with("-- map00007.lua\n-- my level\n-- ---- kfx-editor wizard begin ----\n"),
        "{out}"
    );
    assert!(
        out.ends_with(
            "-- ---- kfx-editor wizard end ----\n\nfunction \
                OnGameStart()\n    StartMoney(PLAYER0, 3000)\nend\n"
        ),
        "{out}"
    );
    assert_eq!(WizardModel::read_lua(&out), m, "read back");
    assert!(!m.plan_lua(out.as_bytes()).changes(out.as_bytes()));
    // without an OnGameStart, the block defines one
    let plan = m.plan_lua(b"-- empty\n");
    assert!(plan
        .lines
        .join("\n")
        .contains("function OnGameStart()\n    KfxWizardSetup()\nend"));
    assert!(plan.notes.is_empty(), "{:?}", plan.notes);
}

// ---- the form

struct Form {
    w: ScriptWizard,
    events: Vec<WizardEvent>,
    host: StaticProvider,
}

fn form(script: &str) -> Harness<'static, Form> {
    let w = ScriptWizard::new(ScriptLang::Dk, script.as_bytes());
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(900.0, 640.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, f: &mut Form| {
                if let Some(e) = f.w.show(ui, &f.host) {
                    f.events.push(e);
                }
            },
            Form {
                w,
                events: vec![],
                host: StaticProvider::default(),
            },
        );
    h.run_steps(3);
    h
}

#[test]
fn form_cycle_button_start_gold_and_apply() {
    let script = "REM test\nLEVEL_VERSION(1)\nCOMPUTER_PLAYER(PLAYER1, 0)\n";
    let mut h = form(script);
    assert!(h.state().w.player_shown(Player::P0) && h.state().w.player_shown(Player::P1));
    // start gold on the General tab
    h.get_by_label("Start gold PLAYER0")
        .set_value_accesskit(12345.0);
    h.run_steps(3);
    assert_eq!(
        h.state().w.model().player(Player::P0).start_money,
        Some(12345)
    );
    // cycle a room: (not set) -> No -> Research -> Yes
    h.get_by_label("Rooms").click();
    h.run_steps(3);
    h.get_by_label("TEMPLE PLAYER0").click();
    h.run_steps(2);
    assert_eq!(
        h.state().w.model().player(Player::P0).rooms.get("TEMPLE"),
        Some(&Availability::Unavailable)
    );
    h.get_by_label("TEMPLE PLAYER0").click();
    h.run_steps(2);
    h.get_by_label("TEMPLE PLAYER0").click();
    h.run_steps(2);
    assert_eq!(
        h.state().w.model().player(Player::P0).rooms.get("TEMPLE"),
        Some(&Availability::Available)
    );
    // victory checkbox
    h.get_by_label("Victory").click();
    h.run_steps(3);
    h.get_all_by_label("Win when all enemy dungeons are destroyed")
        .next()
        .unwrap()
        .click();
    h.run_steps(3);
    assert!(
        h.state()
            .w
            .model()
            .player(Player::P0)
            .win_when_enemies_destroyed
    );
    h.get_by_label("Apply").click();
    h.run_steps(2);
    assert_eq!(h.state().events, vec![WizardEvent::Apply]);
    let out = s(&h
        .state()
        .w
        .plan_for(script.as_bytes())
        .apply_to(script.as_bytes()));
    assert_eq!(
        out,
        "REM test\nLEVEL_VERSION(1)\nREM ---- kfx-editor wizard \
            begin ----\nREM Level setup from the script wizard. Lines between these markers \
            are rewritten when it is applied again.\nSTART_MONEY(PLAYER0, 12345)\n\
            ROOM_AVAILABLE(PLAYER0, TEMPLE, 1, 1)\nIF(PLAYER0, ALL_DUNGEONS_DESTROYED == \
            1)\n\tWIN_GAME\nENDIF\nREM ---- kfx-editor wizard end ----\n\
            COMPUTER_PLAYER(PLAYER1, 0)\n"
    );
    h.get_by_label("Close").click();
    h.run_steps(2);
    assert_eq!(h.state().events.last(), Some(&WizardEvent::Close));
}

#[test]
fn form_shows_conflicts_and_disables_apply_without_changes() {
    let mut h = form(SCRIPT);
    // nothing to change yet
    assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
    h.state_mut()
        .w
        .model_mut()
        .player_mut(Player::P0)
        .max_creatures = Some(10);
    h.run_steps(3);
    assert!(!h.get_by_label("Apply").accesskit_node().is_disabled());
    assert!(
        h.query_by_label_contains("MAX_CREATURES for PLAYER0")
            .is_some(),
        "conflict listed"
    );
}

// ---- applying to the editor: one undo step

#[test]
fn apply_to_editor_is_one_undo_step() {
    let src = "REM x\nLEVEL_VERSION(1)\nSTART_MONEY(PLAYER1, 1)\n";
    let mut ed = ScriptEditor::new();
    ed.set_text_bytes(src.as_bytes());
    ed.set_side_panel(SidePanel::None);
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(900.0, 500.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, (ed, host): &mut (ScriptEditor, StaticProvider)| {
                ed.show(ui, host);
            },
            (ed, StaticProvider::default()),
        );
    h.run_steps(3);
    h.state_mut().0.set_caret(src.len());
    h.state_mut().0.reveal_span(Span::new(src.len(), src.len()));
    h.run_steps(3);
    let mut m = WizardModel::read(src);
    m.player_mut(Player::P0).start_money = Some(12345);
    m.player_mut(Player::P0)
        .rooms
        .insert("LAIR".into(), Availability::Available);
    let bytes = h.state().0.text_bytes();
    let plan = m.plan(&bytes);
    assert!(plan.apply_to_editor(&mut h.state_mut().0));
    h.run_steps(3);
    let now = s(&h.state().0.text_bytes());
    assert_eq!(now, s(&plan.apply_to(src.as_bytes())));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(4);
    assert_eq!(
        s(&h.state().0.text_bytes()),
        src,
        "one Ctrl+Z reverts the whole block"
    );
}

// ---- the workspace hosts the wizard

#[derive(Default)]
struct Files(RefCell<HashMap<String, Vec<u8>>>);
impl ScriptFiles for Files {
    fn read(&self, n: &str) -> Option<Vec<u8>> {
        self.0.borrow().get(n).cloned()
    }
    fn write(&self, n: &str, b: &[u8]) -> Result<(), String> {
        self.0.borrow_mut().insert(n.into(), b.to_vec());
        Ok(())
    }
}

#[test]
fn workspace_opens_the_wizard_and_applies_its_block() {
    let files = Files::default();
    let src = "REM level\nLEVEL_VERSION(1)\n\nCOMPUTER_PLAYER(PLAYER1, 0)\n";
    files
        .0
        .borrow_mut()
        .insert("map00007.txt".into(), src.as_bytes().to_vec());
    let mut ws = ScriptWorkspace::new(7);
    ws.load(&files);
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1100.0, 760.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, (ws, files): &mut (ScriptWorkspace, Files)| {
                ws.show(ui, &NullProvider, files);
            },
            (ws, files),
        );
    h.run_steps(3);
    h.get_by_label("Wizard…").click();
    h.run_steps(4);
    assert!(h.state().0.wizard_open());
    assert_eq!(
        h.state_mut()
            .0
            .wizard_mut()
            .unwrap()
            .model()
            .player(Player::P1)
            .computer,
        Some(Computer::Model(0))
    );
    h.get_by_label("Start gold PLAYER0")
        .set_value_accesskit(12345.0);
    h.run_steps(3);
    h.get_by_label("Apply").click();
    h.run_steps(4);
    let text = s(&h.state().0.editor("map00007.txt").unwrap().text_bytes());
    assert!(
        text.starts_with("REM level\nLEVEL_VERSION(1)\nREM ---- kfx-editor wizard begin ----\n"),
        "{text}"
    );
    assert!(text.contains("\nSTART_MONEY(PLAYER0, 12345)\n"), "{text}");
    assert!(
        text.ends_with("REM ---- kfx-editor wizard end ----\n\nCOMPUTER_PLAYER(PLAYER1, 0)\n"),
        "{text}"
    );
    assert!(h.state().0.is_dirty());
    // the form now plans against the new text: nothing left to apply
    assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
    // a Lua script that does not exist yet is created from the template
    h.state_mut().0.open_wizard(ScriptLang::Lua);
    h.run_steps(3);
    assert!(h
        .state()
        .0
        .tab_infos()
        .iter()
        .any(|t| t.name == "map00007.lua" && t.exists));
    h.state_mut()
        .0
        .wizard_mut()
        .unwrap()
        .model_mut()
        .player_mut(Player::P0)
        .max_creatures = Some(9);
    let plan = h.state_mut().0.apply_wizard().expect("applied");
    assert!(
        plan.lines
            .iter()
            .any(|l| l == "    MaxCreatures(PLAYER0, 9)"),
        "{:?}",
        plan.lines
    );
    h.run_steps(3);
    let lua = s(&h.state().0.editor("map00007.lua").unwrap().text_bytes());
    assert!(
        lua.contains("MaxCreatures(PLAYER0, 9)") && lua.contains("KfxWizardSetup()"),
        "{lua}"
    );
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(!h.state().0.wizard_open());
}

/// Every command form the wizard writes passes the game-faithful validator, in both level
/// versions, with every option in use.
#[test]
fn generated_blocks_validate_cleanly() {
    for (version, header) in [(1, "LEVEL_VERSION(1)\n"), (0, "REM original rules\n")] {
        let mut m = WizardModel::read(header);
        assert_eq!(m.version, version);
        m.generate_speed = Some(400);
        for p in [Player::P0, Player::P1, Player::P2, Player::P3] {
            let s = m.player_mut(p);
            s.start_money = Some(2500);
            s.max_creatures = Some(25);
            s.rooms.insert("TREASURE".into(), Availability::Available);
            s.rooms.insert("TEMPLE".into(), Availability::Researchable);
            s.rooms.insert("PRISON".into(), Availability::Unavailable);
            s.powers.insert("POWER_IMP".into(), Availability::Available);
            s.traps.insert(
                "BOULDER".into(),
                Stock {
                    buildable: true,
                    amount: 1,
                },
            );
            s.doors.insert(
                "WOOD".into(),
                Stock {
                    buildable: true,
                    amount: 0,
                },
            );
            s.creatures.insert(
                "TROLL".into(),
                Attraction {
                    allowed: true,
                    forced: if version > 0 { 1 } else { 0 },
                },
            );
            s.win_when_enemies_destroyed = p == Player::P0;
            s.lose_when_heart_destroyed = p == Player::P0;
        }
        m.player_mut(Player::P1).computer = Some(Computer::Model(0));
        m.player_mut(Player::P2).computer = Some(if version > 0 {
            Computer::Roaming
        } else {
            Computer::Model(13)
        });
        m.pool.insert("TROLL".into(), 5);
        m.alliances.insert((Player::P1, Player::P2), 1);
        let out = m.plan(header.as_bytes()).apply_to(header.as_bytes());
        let report = kfx_script::validate(&kfx_script::Document::parse(&out), &NullProvider);
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity != kfx_script::Severity::Info)
            .collect();
        assert!(
            errors.is_empty(),
            "version {version}:\n{}\n{errors:#?}",
            s(&out)
        );
        assert_eq!(
            WizardModel::read_bytes(&out),
            m,
            "version {version} reads back"
        );
    }
}
