//! The script services on data as `tools/sync_keeperfx.py` leaves it after an upstream change.
//!
//! `tools/tests/fixtures/expected/` holds the command table and the reconciled overlay of a
//! synthetic KeeperFX tree (`tools/tests/test_sync_keeperfx.py` checks that the tool produces
//! exactly these files): `SET_FIXTURE_GLOW` is new and only has a generated (`"auto": true`)
//! overlay entry, `ADD_GOLD_TO_PLAYER` gained an optional argument, `FIXTURE_OLD_BEACON` was
//! removed. This test binary replaces the embedded data with them before anything reads it, so
//! it holds a single test.
use kfx_script::overlay::AUTO_DOC;
use kfx_script::*;

const TABLES: &str = include_str!("../../../tools/tests/fixtures/expected/script_tables.json");
const OVERLAY: &str =
    include_str!("../../../tools/tests/fixtures/expected/script_arg_overlay.json");

fn codes(src: &str) -> Vec<(&'static str, String)> {
    let doc = Document::parse(src.as_bytes());
    validate(&doc, &NullProvider)
        .diagnostics
        .into_iter()
        .map(|d| (d.code, d.message))
        .collect()
}

#[test]
fn services_on_reconciled_fixture_data() {
    Tables::install(Tables::from_json(TABLES).expect("fixture tables"))
        .unwrap_or_else(|_| panic!("tables already loaded"));
    Overlay::install(Overlay::from_json(OVERLAY).expect("fixture overlay"))
        .unwrap_or_else(|_| panic!("overlay already loaded"));
    assert!(Tables::get().find_full("SET_FIXTURE_GLOW").is_some());

    // A command known only through a generated overlay entry is valid, with or without its
    // optional argument; so is the changed command with its new optional argument.
    for src in [
        "LEVEL_VERSION(1)\nSET_FIXTURE_GLOW(PLAYER0, 5)\nWIN_GAME\n",
        "LEVEL_VERSION(1)\nSET_FIXTURE_GLOW(PLAYER1, 5, 7)\n",
        "LEVEL_VERSION(1)\nADD_GOLD_TO_PLAYER(PLAYER0, 100, 2)\n",
        "LEVEL_VERSION(1)\nADD_GOLD_TO_PLAYER(PLAYER0, 100)\n",
    ] {
        let d = codes(src);
        assert!(
            d.iter().all(|(c, _)| *c != "unknown-command"),
            "{src}: {d:?}"
        );
        assert!(
            !d.iter().any(|(c, _)| c.starts_with("unknown-")
                || *c == "too-few-args"
                || *c == "excess-args"),
            "{src}: {d:?}"
        );
    }

    // ... and its arguments are still checked against the type letters.
    let d = codes("LEVEL_VERSION(1)\nSET_FIXTURE_GLOW(NOBODY, 5)\n");
    assert!(d.iter().any(|(c, _)| c.starts_with("unknown")), "{d:?}");

    // Signature help: generic, guessed names and the note instead of an empty description.
    let sig = signature_of("SET_FIXTURE_GLOW").expect("signature");
    assert_eq!(sig.label, "SET_FIXTURE_GLOW(player, number2, [number3])");
    assert!(sig.params.iter().all(|p| p.guessed));
    assert_eq!(sig.params[0].kind, ArgKind::Player);
    assert_eq!(sig.doc, AUTO_DOC);
    assert!(Overlay::get().cmd("SET_FIXTURE_GLOW").unwrap().auto);

    // The changed command keeps its hand-written names.
    let sig = signature_of("ADD_GOLD_TO_PLAYER").expect("signature");
    assert_eq!(sig.label, "ADD_GOLD_TO_PLAYER(player, gold, [number3])");
    assert_eq!(sig.doc, "Gives gold to a player.");

    // A removed command is unknown, and the message says KeeperFX removed it.
    let d = codes("LEVEL_VERSION(1)\nFIXTURE_OLD_BEACON(PLAYER0, 1)\n");
    let (_, msg) = d
        .iter()
        .find(|(c, _)| *c == "unknown-command")
        .unwrap_or_else(|| panic!("{d:?}"));
    assert!(msg.contains("KeeperFX removed it"), "{msg}");

    // A command nobody knows names the revision of the data instead of calling it invalid.
    let d = codes("LEVEL_VERSION(1)\nSET_FIXTURE_SPARKLE(PLAYER0)\n");
    let (_, msg) = d
        .iter()
        .find(|(c, _)| *c == "unknown-command")
        .unwrap_or_else(|| panic!("{d:?}"));
    let rev = DataRevision::get();
    assert!(
        msg.contains("not known to this editor build's data")
            && msg.contains(rev.keeperfx_day())
            && msg.contains("a newer editor build may know it"),
        "{msg}"
    );
    assert!(!msg.contains("Invalid"), "{msg}");
}
