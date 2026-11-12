//! Import / export of each construct, analysis, Lua export and templates.
use kfx_logic_ui::analysis::*;
use kfx_logic_ui::lua::{export_lua, LuaFlagKind};
use kfx_logic_ui::waves::*;
use kfx_logic_ui::*;
use kfx_script::lua::ApiIndex;
use kfx_script::NullProvider;

fn rt(src: &str) -> LogicDoc {
    let doc = import(src.as_bytes());
    assert!(doc.read_only.is_none(), "read-only: {:?}", doc.read_only);
    assert_eq!(export(&doc), src, "byte round trip");
    let mut regen = doc.clone();
    regen.forget_source();
    let out = export(&regen);
    compare(src.as_bytes(), out.as_bytes())
        .unwrap_or_else(|e| panic!("regenerated differs: {e}\n{out}"));
    doc
}

fn events(doc: &LogicDoc) -> Vec<&Event> {
    doc.nodes
        .iter()
        .filter_map(|n| {
            if let Node::Event(e) = n {
                Some(e)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn simple_if_block() {
    let src = "LEVEL_VERSION(1)\nREM bile demon\nIF(PLAYER0,BILE_DEMON > \
        0)\n\tCREATURE_AVAILABLE(PLAYER0,BILE_DEMON,1,0)\n\
        \tDISPLAY_INFORMATION(59,ALL_PLAYERS)\nENDIF\n";
    let doc = rt(src);
    let ev = events(&doc);
    assert_eq!(ev.len(), 2);
    assert!(ev[0].conditions.is_empty());
    assert_eq!(ev[1].conditions.len(), 1);
    assert_eq!(
        ev[1].conditions[0].cmd.args,
        vec!["PLAYER0", "BILE_DEMON", ">", "0"]
    );
    assert_eq!(ev[1].body.len(), 2);
    assert_eq!(notes_of(&ev[1].pre), "bile demon");
    assert_eq!(
        condition_text(&doc, &ev[1].conditions[0]),
        "PLAYER0: BILE_DEMON > 0"
    );
}

#[test]
fn nested_chain_is_one_event() {
    let src = "LEVEL_VERSION(1)\nIF(PLAYER1,DUNGEON_DESTROYED == 1)\n\
        \tIF(PLAYER2,DUNGEON_DESTROYED == 1)\n\
        \t\tIF_CONTROLS(PLAYER_GOOD,TOTAL_CREATURES <= 0)\n\t\t\tWIN_GAME\n\t\tENDIF\n\
        \tENDIF\nENDIF\n";
    let doc = rt(src);
    let ev = events(&doc);
    assert_eq!(ev[1].conditions.len(), 3);
    assert_eq!(ev[1].body.len(), 1);
}

#[test]
fn nested_with_actions_between() {
    let src = "LEVEL_VERSION(1)\nIF(PLAYER_GOOD,TIMER2 >= 1500)\n\
        \tADD_TUNNELLER_PARTY_TO_LEVEL(PLAYER_GOOD,THREE,-2,DUNGEON_HEART,0,6,250)\n\
        \tIF_CONTROLS(PLAYER_GOOD,DWARFA <= 0)\n\t\tSET_TIMER(PLAYER_GOOD,TIMER3)\n\
        \tENDIF\n\tWIN_GAME\nENDIF\nIF(PLAYER0,FLAG1 == 1)\n\tIF(PLAYER0,FLAG2 == 1)\n\
        \t\tLOSE_GAME\n\tENDIF\n\tREM after\nENDIF\n";
    let doc = rt(src);
    let ev = events(&doc);
    assert_eq!(ev[1].conditions.len(), 1);
    assert_eq!(ev[1].body.len(), 3);
    assert!(matches!(ev[1].body[1], Node::Event(_)));
    // comment between the ENDIFs splits the chain
    assert_eq!(ev[2].conditions.len(), 1);
    assert!(matches!(ev[2].body[0], Node::Event(_)));
    assert_eq!(ev[2].tail, vec!["\tREM after".to_string()]);
}

#[test]
fn parties() {
    let src = "LEVEL_VERSION(1)\nCREATE_PARTY(ONE)\n\
        \tADD_TO_PARTY(ONE,THIEF,2,300,ATTACK_ENEMIES,0)\n\
        \tADD_TO_PARTY(ONE,ARCHER,4,400,ATTACK_ENEMIES,0)\nCREATE_PARTY(TWO)\n\
        \tADD_TO_PARTY(TWO,ARCHER,3,400,ATTACK_ENEMIES,0)\nIF_ACTION_POINT(2,PLAYER0)\n\
        \tADD_PARTY_TO_LEVEL(PLAYER_GOOD,ONE,-3,1)\n\
        \tADD_TO_PARTY(TWO,KNIGHT,9,0,ATTACK_ENEMIES,0)\nENDIF\n";
    let doc = rt(src);
    let ps: Vec<&Party> = doc.parties().collect();
    assert_eq!(ps.len(), 2);
    assert_eq!(ps[0].members.len(), 2);
    assert_eq!(ps[0].members[1].creature, "ARCHER");
    assert_eq!(
        ps[1].members.len(),
        1,
        "a conditional ADD_TO_PARTY stays an action"
    );
    let routes = party_routes(&doc, "ONE");
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].1, Some(MapLoc::HeroGate(3)));
    let (s, unknown) = party_strength(ps[0], &FlatStats);
    assert_eq!((s, unknown), (6.0, 0));
}

#[test]
fn reusable() {
    let src = "LEVEL_VERSION(1)\nIF(PLAYER0,FLAG0 >= 1)\n\
        \tNEXT_COMMAND_REUSABLE\n\tADD_GOLD_TO_PLAYER(PLAYER0,10)\n\
        \tNEXT_COMMAND_REUSABLE\n\n\tWIN_GAME\nENDIF\n";
    let doc = rt(src);
    let ev = events(&doc);
    let Node::Action(a) = &ev[1].body[0] else {
        panic!()
    };
    assert!(a.reusable);
    assert_eq!(a.cmd.name, "ADD_GOLD_TO_PLAYER");
    let Node::Action(b) = &ev[1].body[1] else {
        panic!()
    };
    assert_eq!(
        b.cmd.name, "NEXT_COMMAND_REUSABLE",
        "NCR followed by a blank line does nothing and stays"
    );
    // toggling reusable off changes the semantics as expected
    let mut d2 = doc.clone();
    if let Some(a) = d2.action_mut(a.id) {
        a.reusable = false;
    }
    assert!(compare(src.as_bytes(), export(&d2).as_bytes()).is_err());
}

#[test]
fn disabled_event_round_trip() {
    let src = "LEVEL_VERSION(1)\nREM the ambush\n\
        IF_ACTION_POINT(6,PLAYER0)\n\tREM comment inside\n\
        \tADD_CREATURE_TO_LEVEL(PLAYER_GOOD,ARCHER,6,6,6,500)\nENDIF\nWIN_GAME\n";
    let mut doc = rt(src);
    let id = events(&doc)[1].id;
    doc.event_mut(id).unwrap().enabled = false;
    let off = export(&doc);
    assert!(
        off.contains("REM kfx-off IF_ACTION_POINT(6,PLAYER0)"),
        "{off}"
    );
    let seq = semantic_sequence(off.as_bytes());
    assert!(!seq.iter().any(|s| s.contains("ARCHER")), "{seq:?}");
    let mut back = import(off.as_bytes());
    assert!(back.read_only.is_none());

    assert_eq!(export(&back), off, "disabled export is stable");
    let e = events(&back)
        .into_iter()
        .find(|e| !e.conditions.is_empty())
        .unwrap()
        .clone();
    assert!(!e.enabled);
    assert_eq!(notes_of(&e.pre), "the ambush");
    back.event_mut(e.id).unwrap().enabled = true;
    assert_eq!(export(&back), src);
}

#[test]
fn variable_aliases() {
    let src = "REM header\nLEVEL_VERSION(1)\nIF(PLAYER0,FLAG1 == 1)\n\
        \tSET_FLAG(PLAYER0,FLAG2,1)\nENDIF\n";
    let mut doc = rt(src);
    doc.variables.push(VarAlias {
        name: "Alarm".into(),
        player: "PLAYER0".into(),
        family: VarFamily::Flag,
        index: 1,
    });
    let out = export(&doc);
    assert!(
        out.starts_with("REM header\nREM kfx-var Alarm = PLAYER0.FLAG1\nLEVEL_VERSION(1)"),
        "{out}"
    );
    let back = import(out.as_bytes());
    assert_eq!(back.variables, doc.variables);
    assert_eq!(export(&back), out);
    let ev = events(&back);
    assert_eq!(
        condition_text(&back, &ev[1].conditions[0]),
        "PLAYER0: Alarm (FLAG1) == 1"
    );
    let t = var_table(&back);
    let f1 = t.iter().find(|r| r.var == "FLAG1").unwrap();
    assert_eq!(f1.alias.as_deref(), Some("Alarm"));
    assert_eq!(f1.readers.len(), 1);
    let f2 = t.iter().find(|r| r.var == "FLAG2").unwrap();
    assert_eq!(f2.writers.len(), 1);
    assert_eq!(free_slot(&back, "PLAYER0", VarFamily::Flag), Some(0));
}

#[test]
fn broken_structure_is_read_only() {
    for (src, what) in [
        ("IF(PLAYER0,FLAG1 == 1)\nWIN_GAME\n", "never closed"),
        ("WIN_GAME\nENDIF\n", "ENDIF without IF"),
        ("IF(PLAYER0,FLAG1 ==)\nWIN_GAME\nENDIF\n", "discards"),
    ] {
        let doc = import(src.as_bytes());
        let r = doc.read_only.clone().unwrap_or_default();
        assert!(r.contains(what), "{src:?}: {r}");
        assert_eq!(
            export(&doc),
            src,
            "view-only documents still export verbatim"
        );
    }
}

#[test]
fn opaque_lines_kept() {
    let src = "LEVEL_VERSION(1)\n\
        FROBNICATE(1,2)\n  ADD_TO_PARTY(KNIGHTS,KNIGHT),10,0,ATTACK_ENEMIES,0)\n";
    let doc = rt(src);
    assert_eq!(doc.opaque_count(), 2);
}

#[test]
fn edits_regenerate() {
    let src = "LEVEL_VERSION(1)\nIF(PLAYER0, GAME_TURN >= 1200) REM after \
        a minute\n    ADD_PARTY_TO_LEVEL(PLAYER_GOOD, ONE, -1, 1)\nENDIF\n";
    let mut doc = rt(src);
    let ev = events(&doc)[1].clone();
    let c = doc.condition_mut(ev.conditions[0].id).unwrap();
    c.cmd.set_arg(3, "2400");
    c.touch();
    let Node::Action(a) = &ev.body[0] else {
        panic!()
    };
    let a = doc.action_mut(a.id).unwrap();
    a.cmd.set_arg(3, "2");
    a.touch();
    assert_eq!(
        export(&doc),
        "LEVEL_VERSION(1)\nIF(PLAYER0,GAME_TURN >= 2400) REM after a \
            minute\n\tADD_PARTY_TO_LEVEL(PLAYER_GOOD,ONE,-1,2)\nENDIF\n"
    );
}

#[test]
fn block_comment_and_crlf() {
    let src = "LEVEL_VERSION(1)\r\n/* a\r\n b */ WIN_GAME\r\n\
        IF(PLAYER0,FLAG0 == 0)\r\n\tLOSE_GAME\r\nENDIF\r\n";
    let doc = import(src.as_bytes());
    assert!(doc.read_only.is_none());
    compare(src.as_bytes(), &export_bytes(&doc)).unwrap();
    let mut r = doc.clone();
    r.forget_source();
    compare(src.as_bytes(), &export_bytes(&r)).unwrap();
    assert!(export(&r).contains("\r\n"));
}

#[test]
fn locations_and_timing() {
    let src = "LEVEL_VERSION(1)\nSET_TIMER(PLAYER0,TIMER1)\n\
        IF(PLAYER0,GAME_TURN >= 600)\n\
        \tADD_TUNNELLER_PARTY_TO_LEVEL(PLAYER_GOOD,ONE,-2,ACTION_POINT,5,3,100)\n\
        \tSET_TIMER(PLAYER0,TIMER2)\nENDIF\nIF(PLAYER0,TIMER1 >= 100)\n\
        \tREVEAL_MAP_LOCATION(PLAYER0,3,10)\nENDIF\nIF(PLAYER0,TIMER2 > 50)\n\
        \tWIN_GAME\nENDIF\nIF_SLAB_OWNER(10,12,PLAYER0)\n\tWIN_GAME\nENDIF\n";
    let doc = rt(src);
    let ev = events(&doc);
    let l = event_locations(&doc, ev[1]);
    assert!(l.contains(&LocRef {
        loc: MapLoc::HeroGate(2),
        role: LocRole::SpawnAt,
        node: l[0].node,
        command: "ADD_TUNNELLER_PARTY_TO_LEVEL".into()
    }));
    assert!(l
        .iter()
        .any(|r| r.loc == MapLoc::ActionPoint(5) && r.role == LocRole::HeadFor));
    let l4 = event_locations(&doc, ev[4]);
    assert_eq!(l4[0].loc, MapLoc::Slab { x: 10, y: 12 });
    assert_eq!(l4[0].role, LocRole::Trigger);
    assert!(matches!(
        time_gate(&doc, ev[1]),
        Some(TimeGate::GameTurn { turn: 600, .. })
    ));
    let t = earliest_turns(&doc);
    assert_eq!(t.get(&ev[0].id), Some(&0));
    assert_eq!(t.get(&ev[1].id), Some(&600));
    assert_eq!(t.get(&ev[2].id), Some(&100));
    assert_eq!(t.get(&ev[3].id), Some(&651));
    assert_eq!(t.get(&ev[4].id), None);
    assert_eq!(locations(&doc).len(), 3);
}

#[test]
fn node_lines_match_export() {
    let src = "REM h\nLEVEL_VERSION(1)\n\nCREATE_PARTY(A)\n\
        \tADD_TO_PARTY(A,THIEF,2,300,ATTACK_ENEMIES,0)\nIF(PLAYER0,FLAG0 == 0)\n\tREM \
        x\n\tIF(PLAYER0,FLAG1 == 0)\n\t\tNEXT_COMMAND_REUSABLE\n\
        \t\tADD_GOLD_TO_PLAYER(PLAYER0,1)\n\tENDIF\n\tWIN_GAME\nENDIF\n";
    let mut doc = rt(src);
    doc.variables.push(VarAlias {
        name: "V".into(),
        player: "PLAYER0".into(),
        family: VarFamily::Flag,
        index: 0,
    });
    let text = export(&doc);
    let lines: Vec<&str> = text.lines().collect();
    let nl = node_lines(&doc);
    for (e, _) in doc.events() {
        for n in &e.body {
            if let Node::Action(a) = n {
                let (_, s, end) = nl.iter().find(|x| x.0 == a.id).unwrap();
                assert!(
                    lines[*end - 1].contains(&a.cmd.name),
                    "{} at {s}-{end}: {:?}",
                    a.cmd.name,
                    &lines[*s - 1..*end]
                );
            }
        }
        for c in &e.conditions {
            let (_, s, _) = nl.iter().find(|x| x.0 == c.id).unwrap();
            assert!(
                lines[*s - 1].trim_start().starts_with(&c.cmd.name),
                "{}: {}",
                c.cmd.name,
                lines[*s - 1]
            );
        }
    }
}

#[test]
fn lua_export_triggers() {
    let src = "LEVEL_VERSION(1)\nCREATE_PARTY(ONE)\n\
        \tADD_TO_PARTY(ONE,THIEF,2,300,ATTACK_ENEMIES,0)\nSTART_MONEY(PLAYER0,3000)\n\
        IF(PLAYER0,GAME_TURN >= 600)\n\tADD_PARTY_TO_LEVEL(PLAYER_GOOD,ONE,-1,1)\n\
        \tSET_FLAG(PLAYER0,FLAG1,1)\nENDIF\nIF_ACTION_POINT(3,PLAYER0)\n\tWIN_GAME\n\
        ENDIF\nIF(PLAYER0,FLAG1 == 1)\n\tIF(PLAYER0,FLAG2 == 0)\n\
        \t\tNEXT_COMMAND_REUSABLE\n\t\tADD_GOLD_TO_PLAYER(PLAYER0,5)\n\tENDIF\nENDIF\n\
        IF_AVAILABLE(PLAYER0,POWER_SIGHT == 1)\n\tDISPLAY_INFORMATION(60,ALL_PLAYERS)\nENDIF\n";
    let doc = rt(src);
    let idx = ApiIndex::stock();
    let l = export_lua(&doc, &idx, &NullProvider);
    let lua = &l.lua;
    println!("{lua}");
    assert!(lua.contains("CreateParty(\"ONE\")"));
    assert!(lua.contains("RegisterTimerEvent(KfxEvent_1, 600, false)"));
    assert!(lua.contains("PLAYER0.FLAG1 = 1"));
    assert!(lua.contains("RegisterOnActionPointEvent(KfxEvent_2, 3, PLAYER0)"));
    assert!(lua.contains("return PLAYER0.FLAG1 == 1 and PLAYER0.FLAG2 == 0"));
    assert!(lua.contains("CreateTrigger(\"GameTick\""));
    assert!(l
        .flags
        .iter()
        .any(|f| f.kind == LuaFlagKind::NoEquivalent && f.what == "IF_AVAILABLE"));
    let d = kfx_script::lua::LuaDoc::parse(lua);
    assert!(d.syntax_errors.is_empty(), "{:?}", d.syntax_errors);
}

#[test]
fn templates_validate() {
    let mut doc = import(b"LEVEL_VERSION(1)\n");
    let mut p = doc.new_party("RAIDERS");
    p.members
        .push(doc.new_member("KNIGHT", 5, 500, "ATTACK_DUNGEON_HEART", 0));
    p.members
        .push(doc.new_member("ARCHER", 3, 100, "ATTACK_ENEMIES", 0));
    doc.nodes.push(Node::Party(p));
    let w = EscalatingWaves {
        party: "RAIDERS".into(),
        count: 3,
        ..Default::default()
    };
    for e in w.build(&mut doc) {
        doc.nodes.push(Node::Event(e));
    }
    let a = Ambush {
        owner: "PLAYER_GOOD".into(),
        party: "RAIDERS".into(),
        action_point: 2,
        player: "PLAYER0".into(),
        location: String::new(),
        copies: 1,
        message: Some("Ambush!".into()),
    };
    let e = a.build(&mut doc);
    doc.nodes.push(Node::Event(e));
    let b = BossOnCondition {
        owner: "PLAYER_GOOD".into(),
        party: "RAIDERS".into(),
        location: "-1".into(),
        player: "PLAYER0".into(),
        variable: "FLAG3".into(),
        op: "==".into(),
        value: "1".into(),
        objective: Some("Defeat the lord".into()),
    };
    let e = b.build(&mut doc);
    doc.nodes.push(Node::Event(e));
    let text = export(&doc);
    println!("{text}");
    let c = check(&doc, &NullProvider);
    let errs: Vec<_> = c
        .diagnostics
        .iter()
        .filter(|d| d.severity == kfx_script::Severity::Error)
        .collect();
    assert!(errs.is_empty(), "{errs:?}");
    assert!(text.contains(
        "IF(PLAYER0,GAME_TURN >= 12000)\n\tADD_PARTY_TO_LEVEL(PLAYER_GOOD,RAIDERS,-1,3)\nENDIF"
    ));
    assert!(text.contains("QUICK_INFORMATION(0,\"Ambush!\",2)"));
    assert!(text.contains("QUICK_OBJECTIVE(1,\"Defeat the lord\",-1)"));
    let back = import(text.as_bytes());
    assert!(back.read_only.is_none());
    assert_eq!(export(&back), text);
    assert_eq!(
        c.meters.iter().find(|m| m.label == "Parties").unwrap().used,
        1
    );
}
