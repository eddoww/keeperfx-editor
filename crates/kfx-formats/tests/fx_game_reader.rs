//! FX files and the game's TOML reader (`deps/centitoml`, ported as `kfx_formats::game_toml`):
//! what the editor writes must load in the game and mean the same there, and what the game
//! loads must open in the editor: parser strictness, quoting of unknown keys, empty parent
//! table headers, Latin-1 files and strings with newlines.
use kfx_formats::game_toml;
use kfx_formats::level_fx::*;
use toml::Value;

fn thing() -> FxThing {
    FxThing {
        class: Some(FxClass::from_id(5)),
        subtype: Some(1),
        owner: Some(0),
        x: Some(StlCoord::new(10, 128)),
        y: Some(StlCoord::new(10, 128)),
        z: Some(StlCoord::new(1, 0)),
        creature_level: Some(1),
        ..Default::default()
    }
}

/// An unknown key that is not a bare key is written quoted: written bare (`My Key = 1`) neither
/// the game nor the editor could read it back.
#[test]
fn unknown_keys_needing_quotes_are_quoted() {
    let mut f = TngFx::new();
    let mut t = thing();
    t.extra.insert("My Key".into(), Value::Integer(1));
    t.extra.insert("a.b".into(), Value::Integer(2));
    t.extra.insert("".into(), Value::Integer(3));
    let mut sub = toml::Table::new();
    sub.insert("x y".into(), Value::Integer(1));
    t.extra.insert("Nested".into(), Value::Table(sub));
    f.items.push(t);
    let text = f.to_text();
    assert_eq!(game_toml::check(&text), Ok(()), "{text}");
    let back = TngFx::parse(&text).unwrap();
    assert_eq!(back.items, f.items, "{text}");
    assert_eq!(back.to_text(), text, "stable");
}

/// A hand-written quoted key in a real file. The game keeps the quotes as part of the key; the
/// editor must load it and write something the game reads.
#[test]
fn quoted_key_in_a_file_survives_a_save() {
    let src = "[common]\nThingsCount = 1\n\n[thing0]\n\"Foo Bar\" = 1\nThingType \
        = \"Object\"\nSubtype = 1\nOwnership = 0\nSubtileX = [1, 128]\nSubtileY = [1, \
        128]\nSubtileZ = [1, 0]";
    assert_eq!(game_toml::check(src), Ok(()));
    let f = TngFx::parse(src).unwrap();
    assert_eq!(f.items.len(), 1);
    let text = f.to_text();
    assert_eq!(game_toml::check(&text), Ok(()), "{text}");
    assert!(text.contains("\"Foo Bar\" = 1"), "{text}");
    assert_eq!(TngFx::parse(&text).unwrap().to_text(), text);
}

/// Preserved unknown tables keep an empty parent header (`[foo]` before `[foo.bar]`, which
/// `toml::to_string` drops): without it the game fails the whole file ("out of memory",
/// toml_api.c:702). Deeper tables must not get a header through an explicit middle table
/// (toml_api.c:1294).
#[test]
fn preserved_tables_keep_the_headers_the_game_needs() {
    for (name, src) in [
        (
            "empty parent",
            "[common]\nLightsCount = 0\n\n[foo]\n\n[foo.bar]\ny = 2\n",
        ),
        (
            "three levels",
            "[common]\nLightsCount = 0\n\n[deep]\nx = 1\n\n[deep.a]\nz = 1\n\n[deep.a.b]\nw = 1\n",
        ),
        (
            "three levels, implicit middle",
            "[common]\nLightsCount = 0\n\n[deep]\n\n[deep.a.b]\nw = 1\n\n[deep.a.c]\nv = 1\n",
        ),
        ("top-level value", "top = 5\n[common]\nLightsCount = 0\n"),
    ] {
        let l = LgtFx::parse(src).unwrap_or_else(|e| panic!("{name}: {e}"));
        let out = l.to_text();
        assert_eq!(game_toml::same_as_toml(&out), Ok(()), "{name}:\n{out}");
        assert_eq!(LgtFx::parse(&out).unwrap().other, l.other, "{name}");
    }
}

/// A file that is not UTF-8 is read as Latin-1 and written back as Latin-1: as UTF-8 the bytes
/// of every non-ASCII creature name would change (the game copies the bytes as they are).
#[test]
fn latin1_files_keep_their_bytes() {
    let mut raw = b"[common]\nThingsCount = 1\n\n[thing0]\nThingType = \"Creature\"\nSubtype = \
        1\nOwnership = 0\nCreatureName = \"Bj"
        .to_vec();
    raw.push(0xF8);
    raw.extend_from_slice(b"rn\"\nSubtileX = [1, 128]\nSubtileY = [1, 128]\nSubtileZ = [1, 0]");
    let f = TngFx::from_bytes(&raw).unwrap();
    assert_eq!(f.items[0].creature_name.as_deref(), Some("Bj\u{f8}rn"));
    assert_eq!(f.to_bytes(), raw);
}

/// Strings with a newline are not written as `"""` multi-line strings: the game keeps the
/// newline after the opening quotes, so the name would change. Also no `'''` inside a basic
/// string and no lower-case `\u` hex, which the game rejects.
#[test]
fn strings_mean_the_same_to_the_game() {
    for name in [
        "two\nlines",
        "x'''y",
        "tab\tand\u{1}ctrl",
        "quote\"back\\slash",
    ] {
        let mut f = TngFx::new();
        let mut t = thing();
        t.creature_name = Some(name.into());
        f.items.push(t);
        let text = f.to_text();
        assert!(!text.contains("\"\"\""), "{text}");
        assert_eq!(game_toml::same_as_toml(&text), Ok(()), "{name:?}: {text}");
        let g = game_toml::parse(&text).unwrap();
        let thing0 = g
            .iter()
            .find(|(k, _)| k == "thing0")
            .map(|(_, v)| v.clone())
            .unwrap();
        let game_name = match thing0 {
            game_toml::GValue::Table(t) => t
                .into_iter()
                .find(|(k, _)| k == "CreatureName")
                .map(|(_, v)| v),
            _ => None,
        };
        assert_eq!(game_name, Some(game_toml::GValue::Str(name.into())));
    }
}

/// The editor's parser is not stricter than the game's. These load in the game
/// (toml_api.c) but not in a TOML reader.
#[test]
fn files_the_game_reads_open_in_the_editor() {
    for (name, body) in [
        ("raw and quoted duplicate", "\"Foo\" = 1\nFoo = 2\n"),
        ("trailing comma in an inline table", "Extra = {a = 1,}\n"),
        ("duplicate key with an empty array", "Junk = 1\nJunk = []\n"),
    ] {
        let src = format!(
            "[common]\nThingsCount = 1\n\n[thing0]\nThingType = \"Object\"\nSubtype \
            = 1\nOwnership = 0\nSubtileX = [1, 128]\nSubtileY = [1, 128]\nSubtileZ = [1, \
            0]\n{body}"
        );
        assert_eq!(game_toml::check(&src), Ok(()), "{name}: the game reads it");
        let f = TngFx::parse(&src).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(f.items.len(), 1, "{name}");
        let out = f.to_text();
        assert_eq!(game_toml::check(&out), Ok(()), "{name}: {out}");
    }
}

/// A file neither reader accepts reports where.
#[test]
fn unreadable_file_error_has_line_and_column() {
    let e = TngFx::parse("[common]\nThingsCount = 1\n\n[thing0]\nThingType = = 1\n")
        .unwrap_err()
        .to_string();
    assert!(e.contains("line 5"), "{e}");
    assert!(e.contains("column"), "{e}");
}
