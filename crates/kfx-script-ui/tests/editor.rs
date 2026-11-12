use kfx_script::{NullProvider, Severity, Span};
use kfx_script_ui::*;

fn editor_with(bytes: &[u8]) -> ScriptEditor {
    let mut e = ScriptEditor::new();
    e.set_text_bytes(bytes);
    e
}

const SCRIPT: &str =
    "LEVEL_VERSION(1)\nIF(PLAYER0, FLAG0 == 1)\n\tSET_FLAG(PLAYER0, FLAG1, 2)\nENDIF\n";

#[test]
fn latin1_roundtrip_untouched_and_edited() {
    let mut all: Vec<u8> = (0..=255u8).collect();
    all.extend_from_slice(b"\nREM caf\xe9\n");
    for src in [
        all.clone(),
        b"A\r\nB\r\nC".to_vec(),
        b"A\r\nB\nC\r".to_vec(),
        b"".to_vec(),
        b"no newline".to_vec(),
    ] {
        let mut e = editor_with(&src);
        assert_eq!(e.text_bytes(), src, "untouched");
        assert!(!e.is_dirty());
        // an edit and its reverse: same bytes again, and the dirty flag follows the content
        e.set_caret(0);
        e.insert_at_caret("X");
        e.apply_pending();
        assert!(e.is_dirty());
        assert_ne!(e.text_bytes(), src);
        let mut back = e.text_bytes();
        assert_eq!(back.remove(0), b'X');
        assert_eq!(back, src, "only the inserted byte differs (CRLF restored)");
    }
}

#[test]
fn crlf_restored_after_edit_and_dirty_tracking() {
    let src = b"IF(A)\r\n\tFOO\r\nENDIF\r\n";
    let mut e = editor_with(src);
    assert_eq!(e.line_ending(), Eol::CrLf);
    e.set_caret(5);
    e.insert_at_caret("\n\tX"); // pasted LF text becomes CR LF on save
    e.apply_pending();
    assert_eq!(e.text_bytes(), b"IF(A)\r\n\tX\r\n\tFOO\r\nENDIF\r\n");
    e.mark_saved();
    assert!(!e.is_dirty());
}

#[test]
fn generation_counter_increases_on_every_change() {
    let mut e = editor_with(SCRIPT.as_bytes());
    let g0 = e.generation();
    e.set_caret(0);
    e.insert_at_caret("REM x\n");
    e.apply_pending();
    let g1 = e.generation();
    assert!(g1 > g0);
    e.toggle_comment();
    e.apply_pending();
    assert!(e.generation() > g1);
}

#[test]
fn spans_with_crlf_map_back_to_file_bytes() {
    let src = "LEVEL_VERSION(1)\r\nBOGUS_CMD(1)\r\nIF(PLAYER0, FLAG0 == \
        1)\r\n\tSET_FLAG(PLAYER0, FLAG1, 2)\r\nENDIF\r\n";
    let mut e = editor_with(src.as_bytes());
    e.analyze_now(&NullProvider);
    let d = e.diagnostics();
    let bogus = d
        .iter()
        .find(|d| d.code == "unknown-command")
        .expect("unknown command diagnostic");
    assert_eq!(
        &src.as_bytes()[bogus.span.start..bogus.span.end],
        b"BOGUS_CMD"
    );
    assert_eq!(bogus.line, 2);
    assert_eq!(e.file_offset_line_col(bogus.span.start), (2, 1));
    // reveal by file span puts the selection on the same bytes
    e.reveal_span(bogus.span);
    assert_eq!(e.selection_span(), bogus.span);
    // map refs / outline survive the mapping
    let src2 = "LEVEL_VERSION(1)\r\nADD_CREATURE_TO_LEVEL(PLAYER_GOOD, DRAGON, 7, 1, 1, 0)\r\n";
    let mut e = editor_with(src2.as_bytes());
    let refs = e.map_references();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].target, MapTarget::ActionPoint(7));
    assert_eq!(&src2.as_bytes()[refs[0].span.start..refs[0].span.end], b"7");
    assert_eq!(refs[0].line, 2);
}

#[test]
fn validation_is_debounced_and_only_on_changes() {
    let mut e = editor_with(SCRIPT.as_bytes());
    assert!(
        e.update(0.0, &NullProvider),
        "first validation is immediate"
    );
    assert!(!e.update(0.1, &NullProvider), "nothing changed");
    e.set_caret(0);
    e.insert_at_caret("REM a\n");
    e.apply_pending();
    assert!(!e.update(1.0, &NullProvider), "edit starts the debounce");
    assert!(!e.update(1.1, &NullProvider));
    assert!(e.update(1.4, &NullProvider), "ran after the quiet period");
    assert!(!e.update(1.5, &NullProvider));
}

#[test]
fn completion_accept_replaces_partial_word() {
    let mut e = editor_with(b"ADD_CR");
    e.set_caret(6);
    let items = e.completions_at_caret(&NullProvider);
    assert!(
        items.iter().any(|c| c.label == "ADD_CREATURE_TO_LEVEL"),
        "{:?}",
        items.iter().map(|c| &c.label).collect::<Vec<_>>()
    );
    assert!(items.iter().all(|c| c.label.starts_with("ADD_CR")));
    e.refresh_completions_for_test(&NullProvider);
    let i = e
        .completion_labels()
        .iter()
        .position(|l| l == "ADD_CREATURE_TO_LEVEL")
        .unwrap();
    e.accept_completion(i);
    e.apply_pending();
    assert_eq!(e.text_bytes(), b"ADD_CREATURE_TO_LEVEL(");
    // the first argument is offered right away
    e.refresh_completions_for_test(&NullProvider);
    assert!(e.completion_labels().iter().any(|l| l == "PLAYER0"));
}

#[test]
fn signature_help_tracks_the_active_argument() {
    let mut e = editor_with(b"ADD_CREATURE_TO_LEVEL(PLAYER0, DRAGON, ");
    e.set_caret(39);
    let s = e
        .signature_help_at_caret()
        .expect("signature help inside the parentheses");
    assert_eq!(s.command, "ADD_CREATURE_TO_LEVEL");
    assert_eq!(s.active, 2);
    let mut e = editor_with(b"ADD_CREATURE_TO_LEVEL(PLAYER0, DRAGON, 1)\nWIN_GAME");
    e.set_caret(49);
    assert!(e.signature_help_at_caret().is_none());
}

#[test]
fn pick_from_map_writes_into_the_argument() {
    let mut e = editor_with(b"ADD_CREATURE_TO_LEVEL(PLAYER_GOOD, DRAGON, 3, 1, 1, 0)");
    e.set_caret(43); // inside "3"
    e.insert_pick(PickValue::ActionPoint(12));
    e.apply_pending();
    assert_eq!(
        e.text_bytes(),
        b"ADD_CREATURE_TO_LEVEL(PLAYER_GOOD, DRAGON, 12, 1, 1, 0)"
    );
    // hero gate in a location argument is written negative
    e.set_caret(43);
    e.insert_pick(PickValue::HeroGate(4));
    e.apply_pending();
    assert_eq!(
        e.text_bytes(),
        b"ADD_CREATURE_TO_LEVEL(PLAYER_GOOD, DRAGON, -4, 1, 1, 0)"
    );
    // empty argument: inserted at the caret
    let mut e = editor_with(b"ADD_CREATURE_TO_LEVEL(PLAYER_GOOD, DRAGON, ");
    e.set_caret(43);
    e.insert_pick(PickValue::ActionPoint(5));
    e.apply_pending();
    assert_eq!(
        e.text_bytes(),
        b"ADD_CREATURE_TO_LEVEL(PLAYER_GOOD, DRAGON, 5"
    );
    // outside any argument: plain text insertion
    let mut e = editor_with(b"\n");
    e.set_caret(0);
    e.insert_pick(PickValue::Slab { x: 3, y: 4 });
    e.apply_pending();
    assert_eq!(e.text_bytes(), b"3, 4\n");
}

#[test]
fn pick_position_replaces_both_coordinates() {
    let src = b"LEVEL_VERSION(1)\nREVEAL_MAP_RECT(PLAYER0, 10, 20, 5, 5)";
    let mut e = editor_with(src);
    let pos = src.windows(2).position(|w| w == b"10").unwrap();
    e.set_caret(pos + 1);
    let kind = {
        e.set_caret(pos + 1);
        e.pick_kind_at_caret()
    };
    assert_eq!(kind, Some(PickKind::Subtile));
    e.insert_pick(PickValue::Subtile { x: 33, y: 44 });
    e.apply_pending();
    let out = String::from_utf8(e.text_bytes()).unwrap();
    assert_eq!(
        out,
        "LEVEL_VERSION(1)\nREVEAL_MAP_RECT(PLAYER0, 33, 44, 5, 5)"
    );
}

#[test]
fn comment_toggle_and_format_through_the_editor() {
    let mut e = editor_with(b"IF(PLAYER0, FLAG0 == 1)\nSET_FLAG(PLAYER0, FLAG1, 2)\nENDIF\n");
    e.format_document();
    e.apply_pending();
    assert_eq!(
        e.text_bytes(),
        b"IF(PLAYER0, FLAG0 == 1)\n\tSET_FLAG(PLAYER0, FLAG1, 2)\nENDIF\n"
    );
    e.set_caret(30);
    e.toggle_comment();
    e.apply_pending();
    assert_eq!(
        e.text_bytes(),
        b"IF(PLAYER0, FLAG0 == 1)\n\tREM SET_FLAG(PLAYER0, FLAG1, 2)\nENDIF\n"
    );
    e.toggle_comment();
    e.apply_pending();
    assert_eq!(
        e.text_bytes(),
        b"IF(PLAYER0, FLAG0 == 1)\n\tSET_FLAG(PLAYER0, FLAG1, 2)\nENDIF\n"
    );
}

#[test]
fn find_replace_through_the_editor() {
    let mut e = editor_with(b"SET_FLAG(P, FLAG0, 1)\nset_flag(P, FLAG1, 2)\n");
    e.open_find(true);
    e.find_query_for_test("set_flag");
    assert_eq!(e.find_matches().len(), 2);
    assert_eq!(e.replace_all_with_for_test("ADD_TO_FLAG"), 2);
    assert_eq!(
        e.text_bytes(),
        b"ADD_TO_FLAG(P, FLAG0, 1)\nADD_TO_FLAG(P, FLAG1, 2)\n"
    );
}

#[test]
fn map_references_cover_aps_gates_and_positions() {
    let src = "LEVEL_VERSION(1)\nADD_CREATURE_TO_LEVEL(PLAYER_GOOD, \
        DRAGON, -2, 1, 1, 0)\nIF_ACTION_POINT(4, PLAYER0)\nENDIF\n\
        REVEAL_MAP_RECT(PLAYER0, 10, 20, 5, 5)\n";
    let mut e = editor_with(src.as_bytes());
    let refs = e.map_references();
    assert!(refs.iter().any(|r| r.target == MapTarget::HeroGate(2)));
    assert!(
        refs.iter().any(|r| r.target == MapTarget::ActionPoint(4)),
        "{refs:?}"
    );
    assert!(
        refs.iter()
            .any(|r| matches!(r.target, MapTarget::Subtile { x: 10, y: 20 })),
        "{refs:?}"
    );
}

#[test]
fn problems_for_deliberate_errors() {
    let mut e = editor_with(b"LEVEL_VERSION(1)\nNOPE(1)\nIF(PLAYER0, FLAG0 == 1)\n");
    e.analyze_now(&NullProvider);
    let (errs, _, _) = e.diagnostic_counts();
    assert!(errs >= 1);
    assert!(e
        .diagnostics()
        .iter()
        .any(|d| d.severity == Severity::Error && d.span != Span::default()));
}

#[test]
fn meters_report_stats_with_game_limits() {
    let mut e = editor_with(SCRIPT.as_bytes());
    e.analyze_now(&NullProvider);
    let m = e.meters();
    let cond = m.iter().find(|m| m.label == "conditions").unwrap();
    assert_eq!((cond.used, cond.limit), (1, 512));
    assert_eq!(
        m.iter().find(|m| m.label == "script values").unwrap().limit,
        2048
    );
    assert_eq!(m.iter().find(|m| m.label == "parties").unwrap().limit, 256);
    let near = Meter {
        label: "x".into(),
        used: 410,
        limit: 512,
    };
    assert_eq!(near.level(), MeterLevel::Near);
    assert_eq!(
        Meter {
            label: "x".into(),
            used: 513,
            limit: 512
        }
        .level(),
        MeterLevel::Over
    );
    assert_eq!(
        Meter {
            label: "x".into(),
            used: 100,
            limit: 512
        }
        .level(),
        MeterLevel::Ok
    );
}

#[test]
fn shipped_scripts_roundtrip_byte_identical() {
    let Some(dir) = install() else { return };
    let mut n = 0;
    for sub in ["levels", "campgns", "multiplayer"] {
        walk(&dir.join(sub), &mut |p| {
            let name = p
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            if name.starts_with("map") && name.ends_with(".txt") {
                let bytes = std::fs::read(p).unwrap();
                let mut e = editor_with(&bytes);
                assert_eq!(e.text_bytes(), bytes, "{}", p.display());
                // also after a no-op trip through the buffer
                e.insert_at_caret("");
                n += 1;
            }
        });
    }
    eprintln!("checked {n} shipped scripts");
}

/// The KeeperFX install named by `KFX_INSTALL` (the test is skipped without it).
fn install() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(std::env::var_os("KFX_INSTALL")?);
    p.is_dir().then_some(p)
}

fn walk(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}

#[test]
fn block_match_pairs_if_and_endif() {
    use kfx_script_ui::Language;
    let src = b"IF(PLAYER0, FLAG0 == 1)\n\tIF(PLAYER0, FLAG1 == 1)\n\tENDIF\nENDIF\n";
    let a = DkLanguage.analyze(src);
    // caret on the outer IF
    let (o, c) = a.block_match(1).expect("outer pair");
    assert_eq!((o.start, c.start), (0, 56));
    // caret on the inner ENDIF
    let (o, c) = a.block_match(52).expect("inner pair");
    assert_eq!(o.start, 25);
    assert_eq!(c.start, 50);
    assert!(
        a.block_match(35).is_none(),
        "caret in the middle of a block touches neither keyword"
    );
}
