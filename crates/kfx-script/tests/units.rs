use kfx_script::*;
use std::collections::HashMap;

fn parse(s: &str) -> Document {
    Document::parse(s.as_bytes())
}
fn prov() -> StaticProvider {
    let mut names: HashMap<ConstKind, Vec<String>> = HashMap::new();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    names.insert(
        ConstKind::Creature,
        s(&["IMP", "KNIGHT", "BARBARIAN", "DRAGON", "TROLL"]),
    );
    names.insert(ConstKind::Room, s(&["TREASURE", "LAIR", "HATCHERY"]));
    names.insert(ConstKind::Slab, s(&["HARD", "CLAIMED", "PATH"]));
    names.insert(ConstKind::Trap, s(&["BOULDER", "ALARM"]));
    names.insert(ConstKind::Door, s(&["WOOD", "STEEL"]));
    names.insert(ConstKind::Power, s(&["POWER_HAND", "POWER_IMP"]));
    StaticProvider {
        names,
        action_points: vec![1, 2, 5],
        hero_gates: vec![1],
        map_known: true,
    }
}
fn diags(s: &str) -> Vec<Diagnostic> {
    validate(&parse(s), &prov()).diagnostics
}
fn codes(s: &str) -> Vec<&'static str> {
    diags(s).iter().map(|d| d.code).collect()
}
fn has(s: &str, code: &str) -> bool {
    codes(s).contains(&code)
}
fn lacks(s: &str, code: &str) -> bool {
    !has(s, code)
}
const V1: &str = "LEVEL_VERSION(1)\n";

#[test]
fn lossless_incl_latin1_and_odd_bytes() {
    let mut data: Vec<u8> =
        b"REM caf\xe9 \xff\nADD_CREATURE_TO_LEVEL(PLAYER0,IMP,1,1,1,0)\r\n  /* \
        a\nmulti */ WIN_GAME\n\"unterminated\n\x00\x01 ??? ~ , ( ) <= != -\n"
            .to_vec();
    data.extend_from_slice(b"no newline at end");
    let d = Document::parse(&data);
    assert_eq!(d.reassemble(), data);
    // 256 byte values
    let all: Vec<u8> = (0..=255u8).collect();
    assert_eq!(Document::parse(&all).reassemble(), all);
    assert_eq!(Document::parse(b"").reassemble(), b"");
}

#[test]
fn parse_basic_command_and_spans() {
    let src = "SET_FLAG(PLAYER0, flag1, 5)\n";
    let d = parse(src);
    let c = d.lines[0].cmd.as_ref().unwrap();
    assert_eq!(c.name, "SET_FLAG");
    assert_eq!(d.text_lossy(c.name_span), "SET_FLAG");
    assert_eq!(c.args.len(), 3);
    assert_eq!(c.args[1].text, "FLAG1"); // upper-cased
    assert_eq!(d.text_lossy(c.args[1].span), "flag1");
    assert_eq!(c.arg_count, 3);
    assert_eq!(d.text_lossy(c.open.unwrap()), "(");
    assert_eq!(d.text_lossy(c.close.unwrap()), ")");
}

#[test]
fn rem_comments_and_block_comments() {
    let d =
        parse("REM hello\n  rem x\n/* block */ LEVEL_VERSION(1)\n   /* multi\nline */\nENDIF\n");
    assert_eq!(d.lines[0].kind, LineKind::Comment);
    assert_eq!(d.lines[1].kind, LineKind::Comment);
    let c = d.lines[2].cmd.as_ref().unwrap();
    assert_eq!(c.name, "LEVEL_VERSION");
    assert!(d.lines[2].block.is_some());
    // multi-line block comment line holds the ENDIF that follows it on the closing line? no: ENDIF
    // is on its own line
    assert_eq!(d.version, 1);
    // block comment only at line start
    let d = parse("WIN_GAME /* x */\n");
    assert!(
        d.diagnostics.iter().any(|x| x.code == "expected-paren"),
        "{:?}",
        d.diagnostics
    );
}

#[test]
fn operator_form_if() {
    let d = parse("LEVEL_VERSION(1)\nIF(PLAYER0,MONEY >= 100)\nENDIF\n");
    let c = d.lines[1].cmd.as_ref().unwrap();
    let texts: Vec<_> = c.args.iter().map(|a| a.text.as_str()).collect();
    assert_eq!(texts, ["PLAYER0", "MONEY", ">=", "100"]);
    assert!(c.syntax_ok);
    assert_eq!(c.args[2].tok, ArgTok::Operator);
    assert!(
        validate(&d, &NullProvider)
            .diagnostics
            .iter()
            .all(|x| x.severity != Severity::Error),
        "{:?}",
        validate(&d, &NullProvider).diagnostics
    );
    // comma-separated operator form is not what the game reads
    let d = parse("LEVEL_VERSION(1)\nIF(PLAYER0,MONEY,>=,100)\nENDIF\n");
    assert!(
        d.diagnostics.iter().any(|x| x.code == "excess-token"),
        "{:?}",
        d.diagnostics
    );
}

#[test]
fn drawfrom_random_import() {
    let d = parse(
        "LEVEL_VERSION(1)\nADD_CREATURE_TO_LEVEL(PLAYER0, DRAWFROM(IMP, KNIGHT), 1, 2, \
        3, 4)\nSET_FLAG(PLAYER0, FLAG0, DRAWFROM(1~3, 7))\nSET_FLAG(PLAYER0, FLAG1, \
        IMPORT(PLAYER0, CAMPAIGN_FLAG2))\n",
    );
    let c = d.lines[1].cmd.as_ref().unwrap();
    let f = c.args[1].func.as_ref().unwrap();
    assert_eq!(f.name, "DRAWFROM");
    assert_eq!(
        f.args.iter().map(|a| a.text.as_str()).collect::<Vec<_>>(),
        ["IMP", "KNIGHT"]
    );
    assert_eq!(c.args.len(), 6);
    let f2 = d.lines[2].cmd.as_ref().unwrap().args[2]
        .func
        .as_ref()
        .unwrap();
    assert_eq!(
        f2.args.iter().map(|a| a.text.as_str()).collect::<Vec<_>>(),
        ["1", "~", "3", "7"]
    );
    assert_eq!(f2.args[1].tok, ArgTok::Tilde);
    let f3 = d.lines[3].cmd.as_ref().unwrap().args[2]
        .func
        .as_ref()
        .unwrap();
    assert_eq!(f3.name, "IMPORT");
    let r = validate(&d, &prov());
    assert!(
        r.diagnostics.iter().all(|x| x.severity != Severity::Error),
        "{:?}",
        r.diagnostics
    );
    // RANDOM: level version 0 only; DRAWFROM: version 1 only
    assert!(has(
        "SET_FLAG(PLAYER0, FLAG0, RANDOM(1,5))\nLEVEL_VERSION(1)\n",
        "function-version"
    ));
    assert!(lacks(
        "SET_FLAG(PLAYER0, FLAG0, RANDOM(1,5))\n",
        "function-version"
    ));
    assert!(has(
        "LEVEL_VERSION(0)\nSET_FLAG(PLAYER0, FLAG0, DRAWFROM(1,5))\n",
        "function-version"
    ));
    // import of a player that has no campaign flags, unknown flag
    assert!(has(
        &format!("{V1}SET_FLAG(PLAYER0, FLAG0, IMPORT(PLAYER_NEUTRAL, CAMPAIGN_FLAG0))\n"),
        "bad-import"
    ));
    let n_campaign_flags = Tables::get().named("campaign_flag_desc").len();
    assert!(has(
        &format!(
            "{V1}SET_FLAG(PLAYER0, FLAG0, IMPORT(PLAYER0, CAMPAIGN_FLAG{n_campaign_flags}))\n"
        ),
        "unknown-flag"
    ));
    // unknown creature inside DRAWFROM is reported
    assert!(has(
        &format!("{V1}ADD_CREATURE_TO_LEVEL(PLAYER0, DRAWFROM(IMP, UNICORN), 1, 2, 3, 4)\n"),
        "unknown-creature"
    ));
    assert!(has(
        &format!("{V1}SET_FLAG(PLAYER0, FLAG0, DRAWFROM(5~2))\n"),
        "range-order"
    ));
}

#[test]
fn code_unknown_command_with_suggestion() {
    let d = diags(&format!(
        "{V1}ADD_CREATURE_TO_LEVL(PLAYER0, IMP, 1, 1, 1, 0)\n"
    ));
    let e = d.iter().find(|x| x.code == "unknown-command").unwrap();
    assert!(
        e.message.contains("did you mean ADD_CREATURE_TO_LEVEL"),
        "{}",
        e.message
    );
    assert_eq!(e.severity, Severity::Error);
    assert_eq!(e.line, 2);
}

#[test]
fn code_dk1_restrictions_and_level_version() {
    // full-table command without LEVEL_VERSION
    assert!(has("SET_TEXTURE(PLAYER0, WINTER)\n", "needs-level-version"));
    assert!(lacks(
        &format!("{V1}SET_TEXTURE(PLAYER0, WINTER)\n"),
        "needs-level-version"
    ));
    // LEVEL_VERSION at the end still enables the full table for the main pass but pass 1 complains
    let c = codes("SET_TEXTURE(PLAYER0, WINTER)\nLEVEL_VERSION(1)\n");
    assert!(c.contains(&"command-before-level-version"), "{c:?}");
    assert!(!c.contains(&"needs-level-version"));
    assert!(c.contains(&"level-version-late"));
    // preloaded commands need the version at their own position
    assert!(has(
        "NEW_TRAP_TYPE(MYTRAP)\nLEVEL_VERSION(1)\n",
        "needs-level-version"
    ));
    assert!(lacks(
        &format!("{V1}NEW_TRAP_TYPE(MYTRAP)\nPLACE_TRAP(PLAYER0, MYTRAP, 3, 3)\n"),
        "unknown-trap"
    ));
    assert!(has(
        &format!("{V1}LEVEL_VERSION(1)\n"),
        "level-version-repeated"
    ));
    // COMPUTER_PLAYER with PLAYER_GOOD is a level version 1 feature
    assert!(has(
        "COMPUTER_PLAYER(PLAYER_GOOD, 0)\n",
        "level-version-feature"
    ));
}

#[test]
fn code_argument_counts_and_syntax() {
    assert!(has(&format!("{V1}ADD_TO_PARTY(P)\n"), "too-few-args"));
    assert!(has(&format!("{V1}CREATE_PARTY\n"), "too-few-args"));
    assert!(lacks(&format!("{V1}WIN_GAME\n"), "too-few-args"));
    assert!(has(
        &format!("{V1}MAX_CREATURES(PLAYER0, 5, 6, 7)\n"),
        "excess-args"
    ));
    assert!(has(
        &format!("{V1}MAX_CREATURES PLAYER0, 5\n"),
        "expected-paren"
    ));
    assert!(has(
        &format!("{V1}MAX_CREATURES(PLAYER0,, 5)\n"),
        "bad-token"
    ));
    assert!(has(
        &format!("{V1}MAX_CREATURES(PLAYER0, 5\n"),
        "missing-close-paren"
    ));
    assert!(has(&format!("{V1}PRINT(\"abc)\n"), "unterminated-string"));
    assert!(has(&format!("{V1}$$$\n"), "syntax"));
    assert!(has(
        &format!("{V1}MAX_CREATURES(PLAYER0, 5) junk\n"),
        "trailing-tokens"
    ));
    assert!(has(
        &format!("{V1}MAX_CREATURES(PLAYER0 5)\n"),
        "bad-arg-separator"
    ));
    // short args string (DISPLAY_VARIABLE_WITH_LABEL has only 7 letters in the C source)
    assert!(!has(
        &format!("{V1}DISPLAY_VARIABLE_WITH_LABEL(PLAYER0, MONEY)\n"),
        "too-few-args"
    ));
}

#[test]
fn code_wrong_types_and_constants() {
    let c = |l: &str| codes(&format!("{V1}{l}\n"));
    assert!(c("MAX_CREATURES(NOBODY, 5)").contains(&"unknown-player"));
    assert!(c("ADD_CREATURE_TO_LEVEL(PLAYER0, UNICORN, 1, 1, 1, 0)").contains(&"unknown-creature"));
    assert!(
        c("ADD_CREATURE_TO_LEVEL(PLAYER0, ANY_CREATURE, 1, 1, 1, 0)").contains(&"unknown-creature")
    ); // not extended here
       // C!
    assert!(!c("SET_CREATURE_MAX_LEVEL(PLAYER0, ANY_CREATURE, 5)").contains(&"unknown-creature"));
    assert!(c("ROOM_AVAILABLE(PLAYER0, DUNGEON, 1, 1)").contains(&"unknown-room"));
    assert!(c("CHANGE_SLAB_TYPE(1, 1, LAVAX)").contains(&"unknown-slab"));
    assert!(c("IF(PLAYER0, MONEY ?? 5)").len() > 0);
    assert!(c("IF(PLAYER0, MONEY === 5)\nENDIF").contains(&"excess-token"));
    assert!(c("IF(PLAYER0, MONEY ~= 5)\nENDIF").len() > 0);
    assert!(c("DEAD_CREATURES_RETURN_TO_POOL(MAYBE)").contains(&"unknown-boolean"));
    assert!(!c("DEAD_CREATURES_RETURN_TO_POOL(ENABLE)").contains(&"unknown-boolean"));
    // one past the last timer / flag of the tables (TIMER8 and FLAG8 with today's eight)
    let n_timers = Tables::get().named("timer_desc").len();
    let n_flags = Tables::get().named("flag_desc").len();
    assert!(n_timers > 0 && n_flags > 0);
    assert!(c(&format!("SET_TIMER(PLAYER0, TIMER{n_timers})")).contains(&"unknown-timer"));
    assert!(c(&format!("SET_FLAG(PLAYER0, FLAG{n_flags}, 1)")).contains(&"unknown-flag"));
    assert!(c("SET_CAMPAIGN_FLAG(PLAYER0, FLAG1, 1)").contains(&"unknown-flag"));
    assert!(c("MAGIC_AVAILABLE(PLAYER0, POWER_FOO, 1, 1)").contains(&"unknown-power"));
    assert!(c("TRAP_AVAILABLE(PLAYER0, SPIKE, 1, 1)").contains(&"unknown-trap"));
    assert!(c("DOOR_AVAILABLE(PLAYER0, GOLD, 1, 1)").contains(&"unknown-door"));
    assert!(c("IF(PLAYER0, UNKNOWN_VAR >= 1)\nENDIF").contains(&"unknown-variable"));
    // creature as variable
    assert!(!c("IF(PLAYER0, DRAGON >= 1)\nENDIF").contains(&"unknown-variable"));
    assert!(c("SET_TEXTURE(PLAYER0, MARBLE)").contains(&"unknown-constant"));
    assert!(c("SET_PLAYER_MODIFIER(PLAYER0, Wisdom, 5)").contains(&"unknown-constant"));
    assert!(c("KILL_CREATURE(PLAYER0, IMP, SOMEWHERE, 1)").contains(&"unknown-constant"));
    assert!(c("ADD_TO_PARTY(X, IMP, 1, 0, ATTACK_ENEMIES, 0)").contains(&"unknown-party"));
    assert!(c("MAX_CREATURES(PLAYER0, 5x)").contains(&"number-format"));
    // NullProvider: config-driven kinds stay silent
    let d = validate(
        &parse(&format!(
            "{V1}ADD_CREATURE_TO_LEVEL(PLAYER0, UNICORN, 1, 1, 1, 0)\n"
        )),
        &NullProvider,
    );
    assert!(d.diagnostics.iter().all(|x| x.code != "unknown-creature"));
    // the script's own NEW_*_TYPE names are known
    assert!(lacks(
        &format!(
            "{V1}NEW_CREATURE_TYPE(UNICORN)\nADD_CREATURE_TO_LEVEL(PLAYER0, UNICORN, 1, 1, 1, 0)\n"
        ),
        "unknown-creature"
    ));
}

#[test]
fn code_locations() {
    let c = |l: &str| codes(&format!("{V1}{l}\n"));
    assert!(!c("ZOOM_TO_LOCATION(PLAYER0, 1)").contains(&"missing-action-point"));
    assert!(c("ZOOM_TO_LOCATION(PLAYER0, 9)").contains(&"missing-action-point"));
    assert!(c("ZOOM_TO_LOCATION(PLAYER0, -9)").contains(&"missing-hero-gate"));
    assert!(!c("ZOOM_TO_LOCATION(PLAYER0, -1)").contains(&"missing-hero-gate"));
    assert!(c("ZOOM_TO_LOCATION(PLAYER0, 0)").contains(&"bad-location"));
    assert!(c("ZOOM_TO_LOCATION(PLAYER0, SOMEWHERE)").contains(&"bad-location"));
    for ok in [
        "PLAYER1",
        "LAST_EVENT",
        "COMBAT",
        "COMBAT[PLAYER1]",
        "CTA[PLAYER0]",
        "LAST_DEATH_EVENT",
        "LAST_TRAP_EVENT[PLAYER2]",
        "TREASURE",
        "IMP",
    ] {
        assert!(
            c(&format!("ZOOM_TO_LOCATION(PLAYER0, {ok})"))
                .iter()
                .all(|x| !x.contains("location") && !x.contains("missing")),
            "{ok}"
        );
    }
    assert!(c("ZOOM_TO_LOCATION(PLAYER0, COMBAT[NOBODY])").contains(&"bad-location"));
    assert!(c("IF_ACTION_POINT(7, PLAYER0)\nENDIF").contains(&"missing-action-point"));
    assert!(!c("IF_ACTION_POINT(5, PLAYER0)\nENDIF").contains(&"missing-action-point"));
}

#[test]
fn code_if_endif_and_limits() {
    assert!(has(&format!("{V1}ENDIF\n"), "unexpected-endif"));
    assert!(has(
        &format!("{V1}IF(PLAYER0, MONEY >= 1)\nWIN_GAME\n"),
        "missing-endif"
    ));
    assert!(has(&format!("{V1}WIN_GAME\n"), "win-without-condition"));
    assert!(lacks(
        &format!("{V1}IF(PLAYER0, MONEY >= 1)\nWIN_GAME\nENDIF\n"),
        "win-without-condition"
    ));
    assert!(has(
        &format!("{V1}CREATE_PARTY(A)\nCREATE_PARTY(a)\n"),
        "duplicate-party"
    ));
    assert!(has(
        &format!("{V1}IF(PLAYER0, MONEY >= 1)\nCREATE_PARTY(A)\nENDIF\n"),
        "party-in-if"
    ));
    assert!(has(
        &format!("{V1}IF(PLAYER0, MONEY >= 1)\nSTART_MONEY(PLAYER0, 5)\nENDIF\n"),
        "start-money-in-if"
    ));
    // used before created
    let d = diags(&format!(
        "{V1}ADD_TO_PARTY(LATE, IMP, 1, 0, ATTACK_ENEMIES, 0)\nCREATE_PARTY(LATE)\n"
    ));
    let e = d.iter().find(|x| x.code == "unknown-party").unwrap();
    assert!(e.message.contains("created later"), "{}", e.message);
    assert!(lacks(
        &format!(
            "{V1}CREATE_PARTY(A)\nADD_TO_PARTY(a, IMP, 1, 0, ATTACK_ENEMIES, \
        0)\nADD_PARTY_TO_LEVEL(PLAYER_GOOD, A, -1, 1)\n"
        ),
        "unknown-party"
    ));
    // limits
    let mut s = String::from(V1);
    for _ in 0..513 {
        s.push_str("IF(PLAYER0, MONEY >= 1)\nENDIF\n");
    }
    let r = validate(&parse(&s), &prov());
    assert_eq!(r.stats.conditions, 512);
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d.code == "too-many-conditions"));
    let mut s = String::from(V1);
    s.push_str("IF(PLAYER0, MONEY >= 1)\n");
    for _ in 0..2049 {
        s.push_str("SET_FLAG(PLAYER0, FLAG0, 1)\n");
    }
    s.push_str("ENDIF\n");
    let r = validate(&parse(&s), &prov());
    assert_eq!(r.stats.values, 2049);
    assert!(r.diagnostics.iter().any(|d| d.code == "too-many-values"));
    let mut s = String::from(V1);
    s.push_str("IF(PLAYER0, MONEY >= 1)\n");
    for _ in 0..13 {
        s.push_str("WIN_GAME\n");
    }
    s.push_str("ENDIF\n");
    assert!(has(&s, "too-many-win-conditions"));
    let mut s = String::from(V1);
    s.push_str("IF(PLAYER0, MONEY >= 1)\n");
    for _ in 0..257 {
        s.push_str(
            "ADD_CREATURE_TO_LEVEL(PLAYER0, IMP, 1, 1, 1, \
            0)\nADD_TUNNELLER_TO_LEVEL(PLAYER0, 1, ACTION_POINT, 1, 1, 0)\n",
        );
    }
    s.push_str("ENDIF\n");
    let r = validate(&parse(&s), &prov());
    assert_eq!(r.stats.party_triggers, 256);
    assert_eq!(r.stats.tunneller_triggers, 256);
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d.code == "too-many-party-triggers"));
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d.code == "too-many-tunneller-triggers"));
    let mut s = String::from(V1);
    for i in 0..257 {
        s.push_str(&format!("CREATE_PARTY(P{i})\n"));
    }
    assert!(has(&s, "too-many-parties"));
    let mut s = format!("{V1}CREATE_PARTY(A)\n");
    for _ in 0..31 {
        s.push_str("ADD_TO_PARTY(A, IMP, 1, 0, ATTACK_ENEMIES, 0)\n");
    }
    assert!(has(&s, "party-too-big"));
}

#[test]
fn code_ranges_and_players() {
    let c = |l: &str| codes(&format!("{V1}{l}\n"));
    assert!(c("ADD_CREATURE_TO_LEVEL(PLAYER0, IMP, 1, 1, 11, 0)").contains(&"level-range"));
    assert!(c("ADD_CREATURE_TO_LEVEL(PLAYER0, IMP, 1, 0, 1, 0)").contains(&"count-range"));
    assert!(c("ADD_CREATURE_TO_LEVEL(ALL_PLAYERS, IMP, 1, 1, 1, 0)").contains(&"player-not-single"));
    assert!(
        c("CREATE_PARTY(A)\nADD_TO_PARTY(A, IMP, 0, 0, ATTACK_ENEMIES, 0)")
            .contains(&"level-range")
    );
    assert!(c("CREATE_PARTY(A)\nADD_PARTY_TO_LEVEL(PLAYER0, A, 1, 0)").contains(&"count-range"));
}

#[test]
fn code_reusable_lost() {
    assert!(has(
        &format!(
            "{V1}IF(PLAYER0, MONEY >= 1)\nNEXT_COMMAND_REUSABLE\n\nSET_FLAG(PLAYER0, \
        FLAG0, 1)\nENDIF\n"
        ),
        "reusable-lost"
    ));
    assert!(lacks(
        &format!(
            "{V1}IF(PLAYER0, MONEY >= 1)\nNEXT_COMMAND_REUSABLE\nSET_FLAG(PLAYER0, \
        FLAG0, 1)\nENDIF\n"
        ),
        "reusable-lost"
    ));
    // reusable commands count as values even outside IF
    let r = validate(
        &parse(&format!(
            "{V1}NEXT_COMMAND_REUSABLE\nSET_FLAG(PLAYER0, FLAG0, 1)\n"
        )),
        &prov(),
    );
    assert_eq!(r.stats.values, 1);
}

#[test]
fn discarded_command_does_not_change_state() {
    // IF with an unknown variable is discarded by the game, so the ENDIF is unexpected
    let c = codes(&format!("{V1}IF(PLAYER0, NOPE >= 1)\nENDIF\n"));
    assert!(
        c.contains(&"unknown-variable") && c.contains(&"unexpected-endif"),
        "{c:?}"
    );
}

#[test]
fn completions_at_cursors() {
    let p = prov();
    let src = "LEVEL_VERSION(1)\nADD_CR\nSET_FLAG(PLAYER0, \
        F)\nMAX_CREATURES(P\n\nADD_CREATURE_TO_LEVEL(PLAYER0, K";
    let d = parse(src);
    // command prefix
    let off = src.find("ADD_CR").unwrap() + 6;
    let cs = completions(&d, off, &p);
    assert!(cs
        .iter()
        .any(|c| c.label == "ADD_CREATURE_TO_LEVEL" && c.kind == CompletionKind::Command));
    assert!(cs
        .iter()
        .all(|c| c.label.starts_with("ADD_CR") || c.kind == CompletionKind::Snippet));
    assert_eq!(&src[cs[0].replace.start..cs[0].replace.end], "ADD_CR");
    // flag argument
    let off = src.find("(PLAYER0, F").unwrap() + 11;
    let cs = completions(&d, off, &p);
    let labels: Vec<_> = cs.iter().map(|c| c.label.as_str()).collect();
    assert!(
        labels.contains(&"FLAG0") && labels.contains(&"FLAG7"),
        "{labels:?}"
    );
    assert!(!labels.contains(&"TIMER0"));
    // player argument
    let off = src.find("MAX_CREATURES(P").unwrap() + 15;
    let labels: Vec<_> = completions(&d, off, &p)
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert!(labels.contains(&"PLAYER0".to_string()) && labels.contains(&"PLAYER_GOOD".to_string()));
    // empty line offers commands and IF snippets
    let off = src.find("\n\nADD_CREATURE").unwrap() + 1;
    let cs = completions(&d, off, &p);
    assert!(cs
        .iter()
        .any(|c| c.kind == CompletionKind::Snippet && c.insert.contains("ENDIF")));
    assert!(cs.iter().any(|c| c.label == "WIN_GAME"));
    // creature argument comes from the provider
    let labels: Vec<_> = completions(&d, src.len(), &p)
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert_eq!(labels, ["KNIGHT"]);
    // inside a comment: nothing
    let d2 = parse("REM some text here\n");
    assert!(completions(&d2, 8, &p).is_empty());
    // location argument offers action points of the map
    let d3 = parse("LEVEL_VERSION(1)\nZOOM_TO_LOCATION(PLAYER0, ");
    let labels: Vec<_> = completions(&d3, d3.src.len(), &p)
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert!(
        labels.contains(&"5".to_string())
            && labels.contains(&"LAST_EVENT".to_string())
            && labels.contains(&"-1".to_string())
    );
    // operator slot of IF after the variable
    let d4 = parse("LEVEL_VERSION(1)\nIF(PLAYER0, MONEY ");
    let labels: Vec<_> = completions(&d4, d4.src.len(), &p)
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert!(labels.contains(&">=".to_string()), "{labels:?}");
}

#[test]
fn signature_help_active_param() {
    let src = "LEVEL_VERSION(1)\nADD_TO_PARTY(ORCS, KNIGHT, 3, ";
    let d = parse(src);
    let sh = signature_help(&d, src.len()).unwrap();
    assert_eq!(sh.command, "ADD_TO_PARTY");
    assert_eq!(sh.active, 3);
    assert_eq!(sh.params[sh.active].name, "gold");
    assert!(sh
        .label
        .starts_with("ADD_TO_PARTY(party, creature, level, gold"));
    let (a, b) = sh.params[0].label_range;
    assert_eq!(&sh.label[a..b], "party");
    // operator form: IF(PLAYER0, MONEY >= |
    let src = "LEVEL_VERSION(1)\nIF(PLAYER0, MONEY >= ";
    let d = parse(src);
    let sh = signature_help(&d, src.len()).unwrap();
    assert_eq!(sh.params[sh.active].name, "value");
    // not inside parentheses
    assert!(signature_help(&parse("WIN_GAME\n"), 3).is_none());
    // optional parameters are marked
    let sh = signature_of("ADD_CREATURE_TO_LEVEL").unwrap();
    assert!(sh.params.last().unwrap().optional);
    assert!(sh.label.contains("[spawn_type]"));
    // repeating commands
    let src = "LEVEL_VERSION(1)\nSET_SACRIFICE_RECIPE(MKCREATURE, IMP, BILE_DEMON, TROLL, ";
    let d = parse(src);
    let sh = signature_help(&d, src.len()).unwrap();
    assert_eq!(sh.active, 2); // clamped to the repeating parameter
}

#[test]
fn hover_and_highlight() {
    let src = "LEVEL_VERSION(1)\nIF(PLAYER0, MONEY >= 100)\n\tZOOM_TO_LOCATION(PLAYER0, \
        3)\nENDIF\nBOGUS(1)\nREM c\nPRINT(\"hi\")\n";
    let d = parse(src);
    let h = hover(&d, src.find("ZOOM").unwrap() + 2).unwrap();
    assert!(h.markdown.contains("ZOOM_TO_LOCATION(player, location)"));
    let h = hover(&d, src.find(", 3").unwrap() + 2).unwrap();
    assert!(h.markdown.contains("action point 3"), "{}", h.markdown);
    let hl = highlight(&d);
    let class_of = |needle: &str| {
        let o = src.find(needle).unwrap();
        hl.iter().find(|(s, _)| s.start == o).unwrap().1
    };
    assert_eq!(class_of("IF("), TokenClass::Keyword);
    assert_eq!(class_of("ZOOM"), TokenClass::Command);
    assert_eq!(class_of("PLAYER0"), TokenClass::Constant);
    assert_eq!(class_of(">="), TokenClass::Operator);
    assert_eq!(class_of("100"), TokenClass::Number);
    assert_eq!(class_of("BOGUS"), TokenClass::Error);
    assert_eq!(class_of("REM"), TokenClass::Comment);
    assert_eq!(class_of("\"hi\""), TokenClass::String);
    // highlight covers every non-whitespace token exactly once, in order
    assert!(hl.windows(2).all(|w| w[0].0.end <= w[1].0.start));
}

#[test]
fn outline_tree() {
    let src = "LEVEL_VERSION(1)\nCREATE_PARTY(A)\nIF(PLAYER0, MONEY >= 1)\n IF(PLAYER0, FLAG0 == \
        1)\n  WIN_GAME\n ENDIF\n LOSE_GAME\nENDIF\nNEW_TRAP_TYPE(X)\n";
    let o = outline(&parse(src));
    let kinds: Vec<_> = o.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        [
            OutlineKind::LevelVersion,
            OutlineKind::Party,
            OutlineKind::IfBlock,
            OutlineKind::NewType
        ]
    );
    let outer = &o[2];
    assert_eq!(outer.children.len(), 2);
    assert_eq!(outer.children[0].kind, OutlineKind::IfBlock);
    assert_eq!(outer.children[0].children[0].kind, OutlineKind::Win);
    assert_eq!(outer.children[1].kind, OutlineKind::Lose);
    assert_eq!(outer.line, 3);
}

#[test]
fn references_to_map_objects() {
    let src = "LEVEL_VERSION(1)\nIF_ACTION_POINT(4, \
        PLAYER0)\nENDIF\nADD_PARTY_TO_LEVEL(PLAYER_GOOD, A, -2, \
        1)\nADD_PARTY_TO_LEVEL(PLAYER_GOOD, A, 7, 1)\nPLACE_TRAP(PLAYER0, ALARM, 12, \
        30)\nCHANGE_SLAB_TYPE(3, 4, HARD)\nKILL_CREATURE(PLAYER0, IMP, AT_ACTION_POINT[6], \
        1)\nADD_TUNNELLER_TO_LEVEL(PLAYER0, 1, ACTION_POINT, 9, 1, 0)\n";
    let d = parse(src);
    let r = references(&d);
    let get = |k: RefKind| {
        r.iter()
            .filter(|x| x.kind == k)
            .map(|x| x.value)
            .collect::<Vec<_>>()
    };
    let aps = get(RefKind::ActionPoint);
    for v in [4, 7, 6, 9, 1] {
        assert!(aps.contains(&v), "{aps:?}");
    }
    assert_eq!(get(RefKind::HeroGate), [2]);
    let first = r
        .iter()
        .find(|x| x.kind == RefKind::ActionPoint && x.value == 4)
        .unwrap();
    assert_eq!(d.text_lossy(first.span), "4");
    let pos = positions(&d);
    assert!(pos.iter().any(|p| p.x == 12 && p.y == 30 && !p.slab));
    assert!(pos.iter().any(|p| p.x == 3 && p.y == 4 && p.slab));
}

#[test]
fn formatter_reindents_only_leading_whitespace() {
    let src = "LEVEL_VERSION(1)\r\nIF(PLAYER0, MONEY >= 1)\r\nSET_FLAG(PLAYER0, FLAG0, \
        1)  \r\n        IF(PLAYER0, FLAG0 == 1)\r\nREM keep\r\nWIN_GAME\r\n  \
        ENDIF\r\n\r\nENDIF\r\nSET_FLAG(PLAYER0,FLAG1,2) REM x\r\n";
    let d = parse(src);
    let out = String::from_utf8(format(
        &d,
        &FormatOptions {
            indent: "  ".into(),
            indent_rem: false,
        },
    ))
    .unwrap();
    let expect = "LEVEL_VERSION(1)\r\nIF(PLAYER0, MONEY >= 1)\r\n  SET_FLAG(PLAYER0, FLAG0, \
        1)  \r\n  IF(PLAYER0, FLAG0 == 1)\r\nREM keep\r\n    WIN_GAME\r\n  \
        ENDIF\r\n\r\nENDIF\r\nSET_FLAG(PLAYER0,FLAG1,2) REM x\r\n";
    assert_eq!(out, expect);
    // idempotent
    let again = format(
        &parse(&out),
        &FormatOptions {
            indent: "  ".into(),
            indent_rem: false,
        },
    );
    assert_eq!(again, out.as_bytes());
    // REM lines can follow the block depth
    let out2 = String::from_utf8(format(
        &d,
        &FormatOptions {
            indent: "\t".into(),
            indent_rem: true,
        },
    ))
    .unwrap();
    assert!(out2.contains("\t\tREM keep\r\n"), "{out2:?}");
    // binary-safe
    let bin = b"IF(PLAYER0,MONEY >= 1)\n\xe9\xff WIN_GAME\nENDIF\n";
    let o = format(&Document::parse(bin), &FormatOptions::default());
    assert!(o.windows(3).any(|w| w == b"\xe9\xff "));
}

#[test]
fn tables_and_overlay_are_consistent() {
    // No pinned counts: `tools/sync_keeperfx.py` regenerates the tables when KeeperFX changes.
    // What must hold is that the tables are not empty, that the overlay covers every command
    // with matching arguments, and that long-standing commands keep their shape.
    let t = Tables::get();
    assert!(t.commands.len() > 100, "command table lost rows");
    assert!(t.dk1.len() > 20, "dk1 command table lost rows");
    assert!(!t.subfunctions.is_empty());
    for (name, args) in [
        ("ADD_CREATURE_TO_LEVEL", "PCANNNa"),
        ("IF", "PAOAa"),
        ("WIN_GAME", ""),
        ("LEVEL_VERSION", "N"),
    ] {
        let c = t
            .find_full(name)
            .unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(
            std::str::from_utf8(&c.args).unwrap().trim_end(),
            args,
            "{name}"
        );
    }
    for c in t.commands.iter().chain(&t.dk1).chain(&t.subfunctions) {
        assert!(!c.name.is_empty(), "nameless command row");
        assert!(
            c.args.iter().all(|b| b" !+ANCPRLOSBanprlosb".contains(b)),
            "{}: unexpected argument letter in {:?}",
            c.name,
            String::from_utf8_lossy(&c.args)
        );
    }
    let ov = Overlay::get();
    for name in ov.commands.keys() {
        assert!(
            t.find_full(name).is_some(),
            "overlay has {name}, the table not"
        );
    }
    for name in ov.removed.keys() {
        assert!(
            t.find_full(name).is_none(),
            "{name} is in the table and in the overlay's removed section"
        );
    }
    for c in &t.commands {
        let info = ov
            .cmd(&c.name)
            .unwrap_or_else(|| panic!("overlay misses {}", c.name));
        assert_eq!(info.args.len(), c.arity(), "{}", c.name);
        // letters agree with typed overlay kinds
        for (a, l) in info.args.iter().zip(c.arg_letters()) {
            let lk = ArgKind::from_letter(l);
            let refined_bool = lk == ArgKind::Bool
                && matches!(
                    a.kind,
                    ArgKind::ConcealMode | ArgKind::LockState | ArgKind::FreeState
                );
            if !matches!(lk, ArgKind::Text | ArgKind::Number) && !refined_bool {
                assert_eq!(a.kind, lk, "{} {}", c.name, a.name);
            }
        }
    }
    assert_eq!(
        t.find_full("DISPLAY_VARIABLE_WITH_LABEL").unwrap().args,
        *b"PAa     "
    );
}

#[test]
fn services_never_panic_on_garbage() {
    let frags: [&str; 28] = [
        "IF(",
        "ENDIF",
        "\n",
        "\r\n",
        "PLAYER0",
        ",",
        ")",
        "(",
        "\"",
        "REM ",
        "/*",
        "*/",
        "~",
        ">=",
        "-",
        "DRAWFROM(",
        "RANDOM(",
        "IMPORT(",
        "ADD_TO_PARTY",
        "LEVEL_VERSION(1)",
        " ",
        "\t",
        "\u{e9}",
        "12",
        "-3",
        "MONEY",
        "WIN_GAME",
        "[",
    ];
    let mut seed: u64 = 0x1234_5678_9abc_def0;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for _ in 0..300 {
        let mut s = Vec::new();
        for _ in 0..(next() % 40) {
            if next() % 11 == 0 {
                s.push((next() % 256) as u8);
            } else {
                s.extend_from_slice(frags[next() % frags.len()].as_bytes());
            }
        }
        let d = Document::parse(&s);
        assert_eq!(d.reassemble(), s);
        let _ = validate(&d, &prov());
        let _ = highlight(&d);
        let _ = outline(&d);
        let _ = references(&d);
        let _ = positions(&d);
        let f = format(&d, &FormatOptions::default());
        assert_eq!(
            Document::parse(&f)
                .tokens
                .iter()
                .filter(|t| t.kind != TokKind::Whitespace)
                .count(),
            d.tokens
                .iter()
                .filter(|t| t.kind != TokKind::Whitespace)
                .count()
        );
        for off in 0..=s.len() {
            let _ = completions(&d, off, &prov());
            let _ = signature_help(&d, off);
            let _ = hover(&d, off);
        }
    }
}

// ---- unfinished
// lines ----------------------------------------------------------------------------

#[test]
fn unfinished_line_reads_naturally_and_keeps_the_games_verdict() {
    let src = format!("{V1}CREATURE_AVAILABLE(PLAYER0,DR");
    let d = diags(&src);
    // the game: "Unexpected token after parameter 2 ... discarding command" = an error
    let e = d
        .iter()
        .find(|d| d.code == "missing-close-paren")
        .expect("missing paren diagnostic");
    assert_eq!(e.severity, Severity::Error);
    assert!(
        e.message.starts_with("Missing closing parenthesis")
            && !e.message.contains("Unexpected token"),
        "{}",
        e.message
    );
    assert!(!d.iter().any(|d| d.code == "bad-arg-separator"));
    // the half typed name is only a hint
    let h = d
        .iter()
        .find(|d| d.code == "unfinished-arg")
        .expect("hint for DR");
    assert_eq!(h.severity, Severity::Info);
    assert!(
        h.message.to_lowercase().contains("unknown creature") && h.message.contains("DR"),
        "{}",
        h.message
    );
    assert_eq!(&src[h.span.start..h.span.end], "DR");
    // a complete creature name gives no hint, the error stays
    let d = diags(&format!("{V1}CREATURE_AVAILABLE(PLAYER0,DRAGON"));
    assert!(
        d.iter().any(|d| d.code == "missing-close-paren")
            && d.iter().all(|d| d.code != "unfinished-arg")
    );
    // after a comma the argument is missing; the game says "Invalid token"
    let d = diags(&format!("{V1}CREATURE_AVAILABLE(PLAYER0,"));
    let e = d.iter().find(|d| d.code == "unexpected-eol").expect("eol");
    assert_eq!(e.severity, Severity::Error);
    assert!(
        e.message.contains("Unfinished") && e.message.contains("comma"),
        "{}",
        e.message
    );
    // the same line, finished: the real error
    let d = diags(&format!("{V1}CREATURE_AVAILABLE(PLAYER0,DR,1,1)\n"));
    assert!(d.iter().any(|d| d.code == "unknown-creature"));
}

#[test]
fn value_diagnostics_hide_at_the_caret_but_structure_stays() {
    let src = format!("{V1}CREATURE_AVAILABLE(PLAYER0,DR");
    let d = diags(&src);
    let caret = src.len();
    let shown = suppress_at_caret(&d, caret);
    assert!(shown.iter().all(|x| x.code != "unfinished-arg"));
    assert!(
        shown.iter().any(|x| x.code == "missing-close-paren"),
        "the structural error stays"
    );
    // caret elsewhere: nothing hidden
    assert_eq!(suppress_at_caret(&d, 0).len(), d.len());
    // a finished line with an unknown creature: hidden only while the caret is on it
    let src = format!("{V1}CREATURE_AVAILABLE(PLAYER0,DR,1,1)\n");
    let d = diags(&src);
    let at = src.find("DR").unwrap();
    assert!(suppress_at_caret(&d, at + 2)
        .iter()
        .all(|x| x.code != "unknown-creature"));
    assert!(suppress_at_caret(&d, src.len())
        .iter()
        .any(|x| x.code == "unknown-creature"));
    assert_eq!(
        token_at(src.as_bytes(), at + 1),
        Some(Span::new(at, at + 2))
    );
}

// ---- schema driven checks: rules, configuration properties, computer
// names -----------------------------

fn prov2() -> StaticProvider {
    let mut p = prov();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    p.names.insert(
        ConstKind::ComputerProcess,
        s(&["ATTACK PLAN 1", "BUILD A GRAVEYARD ROOM"]),
    );
    p.names
        .insert(ConstKind::ComputerCheck, s(&["CHECK FOR QUICK ATTACK"]));
    p.names
        .insert(ConstKind::ComputerEvent, s(&["EVENT ENEMY DOOR"]));
    p.names
        .insert(ConstKind::Instance, s(&["FIREBALL", "TELEPORT"]));
    p
}
fn d2(s: &str) -> Vec<Diagnostic> {
    validate(&parse(&format!("{V1}{s}\n")), &prov2()).diagnostics
}
fn c2(s: &str) -> Vec<&'static str> {
    d2(s).iter().map(|d| d.code).collect()
}

#[test]
fn game_rule_names_and_values() {
    // the names the shipped scripts use, any case
    for ok in [
        "SET_GAME_RULE(StunGoodEnemyChance,0)",
        "SET_GAME_RULE(PRISONBREAKCHANCE, 50)",
        "SET_GAME_RULE(PayDayProgress,10)",
        "SET_GAME_RULE(PreserveClassicBugs,128)",
    ] {
        assert!(
            c2(ok)
                .iter()
                .all(|c| !c.starts_with("unknown-rule") && !c.starts_with("rule-")),
            "{ok}: {:?}",
            c2(ok)
        );
    }
    let d = d2("SET_GAME_RULE(PrisonBreakChanse, 5)");
    let e = d
        .iter()
        .find(|d| d.code == "unknown-rule")
        .expect("unknown rule");
    assert_eq!(e.severity, Severity::Warning);
    assert!(
        e.message.contains("did you mean PRISONBREAKCHANCE"),
        "{}",
        e.message
    );
    // research and sacrifice settings are not game rules (ruleblocks, src/config_rules.c:220)
    assert!(has(
        &format!("{V1}SET_GAME_RULE(MKCREATURE, 1)\n"),
        "unknown-rule"
    ));
    // values: clamped numbers, words without a name list
    let d = d2("SET_GAME_RULE(BarrackMaxPartySize, 99)");
    let r = d
        .iter()
        .find(|d| d.code == "rule-value-range")
        .expect("range");
    assert!(
        r.message.contains("maximum 30") && r.message.contains("clamps"),
        "{}",
        r.message
    );
    assert!(c2("SET_GAME_RULE(GameTurnsPerHungerHealthLoss, -4)").contains(&"rule-value-range"));
    assert!(c2("SET_GAME_RULE(PayDayGap, lots)").contains(&"rule-value"));
    // inside the range: nothing
    assert!(c2("SET_GAME_RULE(BarrackMaxPartySize, 30)")
        .iter()
        .all(|c| !c.starts_with("rule")));
    // flags rule: a number or the C names
    assert!(c2("SET_GAME_RULE(PreserveClassicBugs, \"NOT_A_BUG\")").contains(&"rule-value"));
    assert!(c2("SET_GAME_RULE(PreserveClassicBugs, RESURRECT_FOREVER)")
        .iter()
        .all(|c| !c.starts_with("rule")));
}

#[test]
fn configuration_property_names_per_target() {
    // shipped uses
    for ok in [
        "SET_CREATURE_CONFIGURATION(IMP, CorpseVanishEffect, 64)",
        "SET_TRAP_CONFIGURATION(BOULDER, Hidden, 0)",
        "SET_ROOM_CONFIGURATION(TREASURE, PROPERTIES, CANNOT_BE_SOLD)",
        "SET_POWER_CONFIGURATION(POWER_HAND, PanelTabIndex, 16)",
        "SET_OBJECT_CONFIGURATION(GOLD, AnimationID, 5)",
    ] {
        assert!(
            !c2(ok).contains(&"unknown-config-property"),
            "{ok}: {:?}",
            d2(ok)
        );
    }
    let d = d2("SET_TRAP_CONFIGURATION(BOULDER, Hiden, 0)");
    let e = d
        .iter()
        .find(|d| d.code == "unknown-config-property")
        .expect("property");
    assert_eq!(e.severity, Severity::Warning);
    assert!(
        e.message.contains("trap") && e.message.contains("did you mean HIDDEN"),
        "{}",
        e.message
    );
    // properties are per target: a room property is not a door property
    assert!(c2("SET_ROOM_CONFIGURATION(TREASURE, Cost, 5)")
        .iter()
        .all(|c| *c != "unknown-config-property"));
    assert!(c2("SET_DOOR_CONFIGURATION(WOOD, SlabAssign, 1)").contains(&"unknown-config-property"));
    assert!(c2("SET_CREATURE_CONFIGURATION(IMP, Health, 5)")
        .iter()
        .all(|c| *c != "unknown-config-property"));
    assert!(c2("SET_CREATURE_CONFIGURATION(IMP, Haelth, 5)").contains(&"unknown-config-property"));
    assert!(
        c2("SET_POWER_CONFIGURATION(POWER_HAND, Colour, 5)").contains(&"unknown-config-property")
    );
    // value ranges of numeric properties
    let d = d2("SET_TRAP_CONFIGURATION(BOULDER, Hidden, 7)");
    assert!(
        d.iter()
            .any(|d| d.code == "config-value" && d.message.contains("maximum 1")),
        "{d:?}"
    );
    assert!(!c2("SET_TRAP_CONFIGURATION(BOULDER, Hidden, 1)").contains(&"config-value"));
}

#[test]
fn computer_names_come_from_the_provider() {
    assert!(
        c2("SET_COMPUTER_PROCESS(PLAYER1,\"ATTACK PLAN 1\",-1,100,335,100,0)")
            .iter()
            .all(|c| *c != "unknown-computer-name")
    );
    assert!(
        c2("SET_COMPUTER_CHECKS(PLAYER1,\"CHECK FOR QUICK ATTACK\",700,70,3000,8,12500)")
            .iter()
            .all(|c| *c != "unknown-computer-name")
    );
    assert!(
        c2("SET_COMPUTER_EVENT(PLAYER1,\"EVENT ENEMY DOOR\", 400, 17, 2500, 2, 0)")
            .iter()
            .all(|c| *c != "unknown-computer-name")
    );
    let d = d2("SET_COMPUTER_PROCESS(PLAYER1,\"ATTACK PLAN 7\",-1,100,335,100,0)");
    let e = d
        .iter()
        .find(|d| d.code == "unknown-computer-name")
        .expect("name");
    assert!(
        e.message.contains("process") && e.message.contains("did you mean ATTACK PLAN 1"),
        "{}",
        e.message
    );
    // a process name is not a check name
    assert!(
        c2("SET_COMPUTER_CHECKS(PLAYER1,\"ATTACK PLAN 1\",700,70,3000,8,12500)")
            .contains(&"unknown-computer-name")
    );
    // a provider without the lists stays silent
    assert!(!has(
        &format!("{V1}SET_COMPUTER_PROCESS(PLAYER1,\"NOPE\",-1,100,335,100,0)\n"),
        "unknown-computer-name"
    ));
}

#[test]
fn instances_are_checked_when_the_provider_knows_them() {
    assert!(!c2("SET_CREATURE_INSTANCE(IMP, 1, FIREBALL, 3)").contains(&"unknown-instance"));
    assert!(c2("SET_CREATURE_INSTANCE(IMP, 1, FIREBAL, 3)").contains(&"unknown-instance"));
    assert!(!has(
        &format!("{V1}SET_CREATURE_INSTANCE(IMP, 1, NOPE, 3)\n"),
        "unknown-instance"
    ));
}

#[test]
fn schema_completions_carry_docs() {
    let src = format!("{V1}SET_GAME_RULE(PrisonBr");
    let doc = parse(&src);
    let c = completions(&doc, src.len(), &prov2());
    let e = c
        .iter()
        .find(|c| c.label == "PRISONBREAKCHANCE")
        .expect("rule offered");
    assert!(
        e.doc.to_lowercase().contains("escape") && !e.detail.is_empty(),
        "{e:?}"
    );
    let src = format!("{V1}SET_TRAP_CONFIGURATION(BOULDER, Shot");
    let c = completions(&parse(&src), src.len(), &prov2());
    let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
    assert!(
        labels.contains(&"SHOTS")
            && labels.contains(&"SHOTORIGIN")
            && !labels.contains(&"SLABASSIGN"),
        "{labels:?}"
    );
    assert!(c
        .iter()
        .find(|c| c.label == "SHOTS")
        .unwrap()
        .doc
        .contains("trigger"));
    // value names of an enum property
    let src = format!("{V1}SET_TRAP_CONFIGURATION(BOULDER, TriggerType, ");
    let c = completions(&parse(&src), src.len(), &prov2());
    assert!(!c.is_empty(), "trigger types offered");
    // computer names from the provider
    let src = format!("{V1}SET_COMPUTER_PROCESS(PLAYER1, ");
    let c = completions(&parse(&src), src.len(), &prov2());
    assert!(c.iter().any(|c| c.label == "ATTACK PLAN 1"));
    // hover on a rule name shows its documentation
    let src = format!("{V1}SET_GAME_RULE(PrisonBreakChance, 5)\n");
    let h = hover(&parse(&src), src.find("PrisonB").unwrap() + 3).unwrap();
    assert!(
        h.markdown.to_lowercase().contains("escape"),
        "{}",
        h.markdown
    );
}
