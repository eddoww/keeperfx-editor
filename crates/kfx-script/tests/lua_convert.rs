//! The DK to Lua assistant: conservative conversion, output must parse and raise no Lua error;
//! run over every shipped DK script for the coverage numbers.
use kfx_formats::{rnc, testutil};
use kfx_script::lua::convert::{convert, ConvertOptions, TODO_MARK};
use kfx_script::lua::diagnostics::diagnostics;
use kfx_script::lua::doc::LuaDoc;
use kfx_script::lua::index::ApiIndex;
use kfx_script::{Document, NullProvider, Severity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn conv(src: &str) -> kfx_script::lua::convert::Conversion {
    convert(
        &Document::parse(src.as_bytes()),
        &ApiIndex::stock(),
        &NullProvider,
        &ConvertOptions::default(),
    )
}

fn check_output(c: &kfx_script::lua::convert::Conversion) {
    let d = LuaDoc::parse(&c.lua);
    assert!(
        d.syntax_errors.is_empty() && d.lex_errors.is_empty(),
        "{:?}\n{}",
        d.syntax_errors,
        c.lua
    );
    let all = diagnostics(&d, &ApiIndex::stock(), &NullProvider);
    let bad: Vec<_> = all
        .iter()
        .filter(|x| x.severity != Severity::Info)
        .collect();
    assert!(bad.is_empty(), "{bad:#?}\n{}", c.lua);
}

#[test]
fn simple_commands_become_calls() {
    let c = conv(
        "LEVEL_VERSION(1)\nSTART_MONEY(PLAYER0, \
        7500)\nSET_GENERATE_SPEED(300)\nSET_FLAG(PLAYER0, FLAG1, 5)\nADD_TO_FLAG(PLAYER0, FLAG1, \
        -2)\nADD_GOLD_TO_PLAYER(PLAYER0, 100)\nMAGIC_AVAILABLE(PLAYER0, POWER_HAND, 1, \
        1)\nCREATURE_AVAILABLE(PLAYER1, TROLL, 1, 2)\nPLACE_DOOR(PLAYER0, WOOD, 3, 4, LOCKED, \
        FREE)\n",
    );
    assert!(c.lua.contains("StartMoney(PLAYER0, 7500)"), "{}", c.lua);
    assert!(c.lua.contains("SetGenerateSpeed(300)"));
    assert!(c.lua.contains("PLAYER0.FLAG1 = 5"));
    assert!(c.lua.contains("PLAYER0.FLAG1 = PLAYER0.FLAG1 + (-2)"));
    assert!(c.lua.contains("PLAYER0:add_gold(100)"));
    assert!(
        c.lua
            .contains("MagicAvailable(PLAYER0, \"POWER_HAND\", true, true)"),
        "{}",
        c.lua
    );
    assert!(c
        .lua
        .contains("CreatureAvailable(PLAYER1, \"TROLL\", true, 2)"));
    assert!(c
        .lua
        .contains("PlaceDoor(PLAYER0, \"WOOD\", 3, 4, true, true)"));
    // LEVEL_VERSION is a load time command: kept as a comment
    assert!(c.lua.contains(&format!("-- {TODO_MARK} LEVEL_VERSION(1)")));
    assert_eq!(
        (c.stats.commands, c.stats.converted, c.stats.commented),
        (9, 8, 1)
    );
    // labelled as an assistant draft
    assert!(c.lua.contains("STARTING POINT, not a faithful translation"));
    check_output(&c);
}

#[test]
fn doubtful_commands_stay_comments_with_a_reason() {
    let c = conv(
        "LEVEL_VERSION(1)\nADD_CREATURE_TO_LEVEL(PLAYER0, IMP, 1, 2, 3, \
        0)\nSET_COMPUTER_GLOBALS(PLAYER1, 1, 2, 3, 4, 5, 6)\nSWAP_CREATURE(KNIGHT, \
        TROLL)\nHIDE_VARIABLE(PLAYER0, MONEY)\nUSE_POWER_ON_CREATURE(PLAYER0, IMP, \
        MOST_EXPERIENCED, PLAYER0, POWER_HEAL_CREATURE, 1, 1)\nSET_CREATURE_CONFIGURATION(IMP, \
        Health, 5)\nSET_FLAG(ALL_PLAYERS, FLAG1, 5)\nSTART_MONEY(PLAYER0, RANDOM(5, 9))\nNOPE(1)\n",
    );
    for cmd in [
        "ADD_CREATURE_TO_LEVEL",
        "SET_COMPUTER_GLOBALS",
        "SWAP_CREATURE",
        "HIDE_VARIABLE",
        "USE_POWER_ON_CREATURE",
        "SET_CREATURE_CONFIGURATION",
        "SET_FLAG",
        "START_MONEY",
    ] {
        assert!(
            c.lua.contains(&format!("-- {TODO_MARK} {cmd}")),
            "{cmd} should be a comment:\n{}",
            c.lua
        );
        assert!(
            c.notes
                .iter()
                .any(|n| n.command == cmd && !n.reason.is_empty()),
            "{cmd}"
        );
    }
    assert!(c.lua.contains("-- TODO(dk2lua) NOPE(1)") || c.lua.contains("not valid DK script"));
    assert_eq!(c.stats.converted, 0, "{}", c.lua);
    check_output(&c);
}

#[test]
fn if_blocks_become_trigger_registrations_only_for_the_simple_patterns() {
    let src = "LEVEL_VERSION(1)\nIF_ACTION_POINT(3, PLAYER0)\n\tDISPLAY_INFORMATION(7, \
        3)\n\tSET_FLAG(PLAYER0, FLAG0, 1)\nENDIF\nIF(PLAYER0, GAME_TURN >= \
        600)\n\tADD_GOLD_TO_PLAYER(PLAYER0, 500)\nENDIF\nIF(PLAYER0, FLAG0 == \
        1)\n\tWIN_GAME\nENDIF\nIF(PLAYER0, MONEY != 5)\n\tSET_FLAG(PLAYER0, FLAG2, \
        1)\nENDIF\nIF_AVAILABLE(PLAYER0, TREASURE >= 1)\n\tSET_FLAG(PLAYER0, FLAG3, \
        1)\nENDIF\nIF(PLAYER0, FLAG0 == 1)\n\tIF(PLAYER0, FLAG1 == 1)\n\t\tSET_FLAG(PLAYER0, \
        FLAG4, 1)\n\tENDIF\n\tSET_FLAG(PLAYER0, FLAG5, 1)\nENDIF\nSTART_MONEY(PLAYER0, 5)\n";
    let c = conv(src);
    assert!(
        c.lua
            .contains("RegisterOnActionPointEvent(DkBlock_1, 3, PLAYER0)"),
        "{}",
        c.lua
    );
    assert!(c.lua.contains("DisplayInformation(7, 3)"));
    assert!(c.lua.contains("RegisterTimerEvent(DkBlock_2, 600, false)"));
    assert!(c
        .lua
        .contains("RegisterOnConditionEvent(DkBlock_3, DkCond_3)"));
    assert!(c.lua.contains("return PLAYER0.FLAG0 == 1"));
    assert!(c.lua.contains("return PLAYER0.MONEY ~= 5"));
    assert!(c.lua.contains("WinGame()"));
    // IF_AVAILABLE has no helper: block and body stay comments
    assert!(
        c.lua.contains(&format!(
            "-- {TODO_MARK} IF_AVAILABLE(PLAYER0, TREASURE >= 1)"
        )),
        "{}",
        c.lua
    );
    assert!(
        !c.lua.contains("FLAG3 = 1\n")
            || c.lua
                .contains("-- TODO(dk2lua) SET_FLAG(PLAYER0, FLAG3, 1)")
    );
    // the nested block keeps the outer one, the inner IF is commented
    assert!(
        c.lua.contains("RegisterOnConditionEvent(DkBlock_6"),
        "{}",
        c.lua
    );
    assert!(
        c.lua.contains("-- TODO(dk2lua) IF(PLAYER0, FLAG1 == 1)")
            && c.lua
                .contains("-- TODO(dk2lua) SET_FLAG(PLAYER0, FLAG4, 1)")
    );
    assert!(c.lua.contains("PLAYER0.FLAG5 = 1"));
    assert_eq!(c.stats.if_blocks, 7);
    assert_eq!(c.stats.if_blocks_converted, 5);
    // callbacks are global named functions, no closures
    assert!(!c.lua.contains("function()"));
    check_output(&c);
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            let n = p
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            if n.starts_with("map")
                && n.ends_with(".txt")
                && n[3..n.len() - 4].bytes().all(|b| b.is_ascii_digit())
            {
                out.push(p);
            }
        }
    }
}

/// Every shipped DK script converts to Lua that parses and raises no Lua error or warning.
#[test]
fn shipped_scripts_convert_to_clean_lua() {
    let Some(install) = testutil::install_dir() else {
        eprintln!("skip: no install");
        return;
    };
    let mut files = vec![];
    for d in ["campgns", "levels", "multiplayer"] {
        walk(&install.join(d), &mut files);
    }
    files.sort();
    let index = ApiIndex::stock();
    let (mut total, mut conv_lines, mut com_lines) = (0usize, 0usize, 0usize);
    let (mut if_total, mut if_conv) = (0usize, 0usize);
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_cmd: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut warn_count = 0usize;
    for f in &files {
        let data = rnc::load_file(f).unwrap();
        let doc = Document::parse(&data);
        let c = convert(
            &doc,
            &index,
            &NullProvider,
            &ConvertOptions {
                source_name: f.file_name().map(|n| n.to_string_lossy().into_owned()),
            },
        );
        let d = LuaDoc::parse(&c.lua);
        assert!(
            d.syntax_errors.is_empty() && d.lex_errors.is_empty(),
            "{}: {:?}",
            f.display(),
            d.syntax_errors
        );
        for x in diagnostics(&d, &index, &NullProvider) {
            assert_ne!(
                x.severity,
                Severity::Error,
                "{}: {} (line {})",
                f.display(),
                x.message,
                x.line
            );
            if x.severity == Severity::Warning {
                warn_count += 1;
                eprintln!(
                    "{}: warning {} {} (line {})",
                    f.display(),
                    x.code,
                    x.message,
                    x.line
                );
            }
        }
        assert_eq!(
            c.stats.converted + c.stats.commented,
            c.stats.commands,
            "{}",
            f.display()
        );
        total += c.stats.commands;
        conv_lines += c.stats.converted;
        com_lines += c.stats.commented;
        if_total += c.stats.if_blocks;
        if_conv += c.stats.if_blocks_converted;
        for n in &c.notes {
            *reasons
                .entry(format!(
                    "{}: {}",
                    n.command,
                    n.reason.split(':').next().unwrap_or("")
                ))
                .or_default() += 1;
        }
        for (_, cmd) in doc.commands() {
            by_cmd.entry(cmd.name.clone()).or_default().0 += 1;
        }
        for n in &c.notes {
            by_cmd.entry(n.command.clone()).or_default().1 += 1;
        }
    }
    println!(
        "{} scripts, {total} command lines: {conv_lines} converted ({:.1}%), {com_lines} \
        commented ({:.1}%)",
        files.len(),
        100.0 * conv_lines as f64 / total as f64,
        100.0 * com_lines as f64 / total as f64
    );
    println!(
        "IF blocks: {if_total}, converted to registrations: {if_conv} ({:.1}%)",
        100.0 * if_conv as f64 / if_total.max(1) as f64
    );
    println!("Lua warnings on the output: {warn_count}");
    let mut top: Vec<_> = by_cmd.iter().collect();
    top.sort_by_key(|(_, (n, c))| std::cmp::Reverse(*c * 1000 + *n));
    for (cmd, (n, commented)) in top.iter().take(25) {
        println!("  {cmd:<32} {n:>6} uses, {commented:>6} commented");
    }
    let mut rs: Vec<_> = reasons.iter().collect();
    rs.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (r, n) in rs.iter().take(15) {
        println!("  {n:>6}  {r}");
    }
    assert_eq!(warn_count, 0);
}
