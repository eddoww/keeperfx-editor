//! String tables and translation.toml: exact round trips on the install, game fallbacks.
use kfx_formats::strings::*;
use kfx_formats::translation::*;
use kfx_formats::{campaign, rnc, testutil};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn internal_codepage_is_injective_and_round_trips() {
    let mut seen = std::collections::HashSet::new();
    let (mut mapped, mut unmapped) = (0, vec![]);
    for b in 1..=255u8 {
        let s = decode_internal(&[b]);
        let c = s.chars().next().unwrap();
        if c == '?' && b != b'?' {
            unmapped.push(b);
            assert_eq!(encode_internal_char('?'), Some(b'?'));
            continue;
        }
        mapped += 1;
        assert!(seen.insert(c), "byte {b:#x}: {c:?} mapped twice");
        assert_eq!(encode_internal_char(c), Some(b), "byte {b:#x}");
    }
    println!("{mapped} mapped bytes, unmapped (decode to '?'): {unmapped:02X?}");
    assert_eq!(
        unmapped,
        vec![0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD, 0xBE, 0xBF, 0xC0, 0xC1, 0xFE, 0xFF]
    );
    assert_eq!(decode_internal(b"a\0b"), "a\0b");
    assert_eq!(decode_internal(&[0x82, 0xA4, 0xEF, 0x41]), "éñĄA");
    assert!(encode_internal("€").is_err());
}

#[test]
fn table_parse_edit_and_exactness() {
    let data = b"one\0\0three\0";
    let mut t = StringTable::parse(data, Lang::English);
    assert_eq!(t.strings(), vec!["one", "", "three"]);
    assert_eq!(t.to_bytes(), data);
    // no final NUL, empty file, lone NUL
    for d in [&b"abc\0def"[..], b"", b"\0", b"\0\0", b"x"] {
        assert_eq!(StringTable::parse(d, Lang::German).to_bytes(), d, "{d:?}");
    }
    t.set(1, "two é").unwrap();
    assert_eq!(t.get(1).as_deref(), Some("two é"));
    assert_eq!(t.raw(1).unwrap(), b"two \x82");
    assert_eq!(t.add("four").unwrap(), 3);
    t.set(6, "seven").unwrap();
    assert_eq!(t.len(), 7);
    assert_eq!(t.first_free(), 4);
    assert!(t.set(0, "bad \0 nul").is_err());
    assert!(t.set(0, "snowman \u{2603}").is_err());
    assert_eq!(StringTable::parse(&t.to_bytes(), Lang::English), t);
    // double-byte languages: real code pages (Shift-JIS "ア" is 83 41, "あ" 82 A0)
    let jp = StringTable::parse(b"ab\x83\x41\0", Lang::Japanese);
    assert_eq!(jp.to_bytes(), b"ab\x83\x41\0");
    assert_eq!(jp.get(0).unwrap(), "ab\u{30A2}");
    let mut jp = jp;
    jp.set(1, "\u{3042}x").unwrap();
    assert_eq!(jp.raw(1).unwrap(), b"\x82\xA0x");
    assert!(jp.set(2, "\u{4E9C}").is_ok(), "kanji exist in CP932");
    assert!(jp.set(3, "\u{AC00}").is_err(), "hangul does not");
    // simplified and traditional Chinese both use GBK (src/bflib_text.c:177-241)
    for l in [Lang::ChineseInt, Lang::ChineseTra] {
        let mut t = StringTable::new(l);
        t.set(0, "\u{4F60}\u{597D}").unwrap();
        assert_eq!(t.raw(0).unwrap(), b"\xC4\xE3\xBA\xC3");
        assert_eq!(t.get(0).unwrap(), "\u{4F60}\u{597D}");
    }
    // Korean EUC-KR
    let mut ko = StringTable::new(Lang::Korean);
    ko.set(0, "\u{AC00}").unwrap();
    assert_eq!(ko.raw(0).unwrap(), b"\xB0\xA1");
    assert!(ko.set(1, "\u{3042}").is_ok(), "kana are in KS X 1001");
    // a string whose bytes are not a valid sequence keeps them when set to its own text
    let mut bad = StringTable::parse(b"a\x81\0", Lang::Japanese);
    assert!(bad.get(0).unwrap().contains('\u{F781}'));
    let same = bad.get(0).unwrap();
    bad.set(0, &same).unwrap();
    assert_eq!(bad.raw(0).unwrap(), b"a\x81");
    assert!(!decodes_cleanly(Lang::Japanese, b"a\x81"));
    assert_eq!(decode_game(Lang::Japanese, b"a\x81"), "a\u{FFFD}");
    assert_eq!(decode_game(Lang::English, &[0xB8]), "?");
    assert_eq!(
        decode(Lang::English, &[0xB8]),
        "\u{F7B8}",
        "lossless escape"
    );
    // the unencodable character is located
    let e = encode(Lang::Japanese, "ab\u{AC00}").unwrap_err();
    assert_eq!((e.ch, e.at), ('\u{AC00}', 2));
}

/// Every strings file of the install (`gtext_*.dat`, `text_*.dat`): parse + to_bytes is the file.
#[test]
fn install_string_tables_round_trip() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let mut files = vec![];
    walk(&root, &mut files);
    // (files, strings, bytes, files whose text round-trips through decode/encode) per language
    let mut per: BTreeMap<String, (usize, usize, usize, usize, usize)> = BTreeMap::new();
    let (mut failures, mut invalid) = (Vec::<String>::new(), 0usize);
    for p in files {
        let name = p
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        let lang = name
            .strip_prefix("gtext_")
            .or_else(|| name.strip_prefix("text_"))
            .and_then(|r| r.strip_suffix(".dat"))
            .and_then(Lang::from_code);
        let Some(lang) = lang else { continue };
        let data = rnc::load_file(&p).unwrap();
        let t = StringTable::parse(&data, lang);
        assert_eq!(
            t.to_bytes(),
            data,
            "{}: parse + to_bytes differs",
            p.display()
        );
        // decode -> encode must give the raw bytes of every string back
        let mut exact = 0usize;
        for i in 0..t.len() {
            let raw = t.raw(i).unwrap();
            let s = t.get(i).unwrap();
            match encode(lang, &s) {
                Ok(b) if b == raw => exact += 1,
                Ok(b) => failures.push(format!(
                    "{} #{i}: re-encodes to {b:02X?}, file has {raw:02X?}",
                    p.display()
                )),
                Err(e) => failures.push(format!("{} #{i}: {e}; raw {raw:02X?}", p.display())),
            }
            if !decodes_cleanly(lang, raw) {
                invalid += 1;
            }
        }
        let text_ok = (exact == t.len()) as usize;
        let strings_ok = exact;
        let e = per.entry(lang.code().to_string()).or_default();
        e.0 += 1;
        e.1 += t.len();
        e.2 += data.len();
        e.3 += text_ok;
        e.4 += strings_ok;
    }
    println!(
        "{:<5} {:>6} {:>9} {:>10} {:>12} {:>16}",
        "lang", "files", "strings", "bytes", "files exact", "strings exact"
    );
    let mut files = 0;
    for (l, (f, s, b, ok, sok)) in &per {
        println!("{l:<5} {f:>6} {s:>9} {b:>10} {ok:>12} {sok:>16}");
        files += f;
    }
    println!(
        "total {files} files, all byte-exact at file level; {invalid} strings contain invalid \
        sequences; {} strings not text-exact",
        failures.len()
    );
    for f in failures.iter().take(40) {
        println!("  {f}");
    }
    assert!(
        failures.is_empty(),
        "{} strings do not survive decode -> encode",
        failures.len()
    );
    assert!(
        files >= 90,
        "expected the install's 17 gtext + 78 text files, got {files}"
    );
}

#[test]
fn translation_resolution_and_layering() {
    let p = Path::new("t.toml");
    let base = TranslationFile::parse(
        "[B_ALIAS]\nENG = \"Beholder\"\n_CHI = \"chi\"\nGER = \"Betrachter\"\n_GER = \"ignored \
            fuzzy\"\n[A_ALIAS]\n_FRE = \"fuzzy fr\"\n[NOENG]\nGER = \"nur\"\n",
        p,
    )
    .unwrap();
    let b = base.get("B_ALIAS").unwrap();
    assert_eq!(b.resolve("GER"), "Betrachter");
    assert!(!b.texts["GER"].fuzzy && b.texts["CHI"].fuzzy);
    assert_eq!(b.resolve("CHT"), "chi", "CHT falls back to CHI");
    assert_eq!(b.resolve("FRE"), "Beholder", "then English");
    assert_eq!(base.get("A_ALIAS").unwrap().resolve("FRE"), "fuzzy fr");
    assert_eq!(
        base.get("NOENG").unwrap().resolve("ENG"),
        "NOENG",
        "last resort: the alias"
    );
    assert_eq!(b.fallback_of("FRE"), Some("ENG"));
    // sorted order, layering: later file replaces the text of an alias and appends new ones
    let dir = kfx_formats::testutil::scratch_dir(&format!(
        "kfx-formats/kfx_transl_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let (f1, f2) = (dir.join("a.toml"), dir.join("b.toml"));
    std::fs::write(&f1, base.to_toml()).unwrap();
    let mut over = TranslationFile::default();
    over.set("B_ALIAS", "ENG", "Eye of doom", false);
    over.set("C_NEW", "ENG", "new", false);
    over.save(&f2).unwrap();
    let t =
        TranslationTable::load(&[f1.clone(), f2.clone(), dir.join("missing.toml")], "eng").unwrap();
    let aliases: Vec<_> = t.entries.iter().map(|e| e.alias.as_str()).collect();
    assert_eq!(aliases, vec!["A_ALIAS", "B_ALIAS", "NOENG", "C_NEW"]);
    assert_eq!(
        t.id_of_alias("B_ALIAS"),
        Some(TRANSLATION_STRINGS_START + 1)
    );
    assert_eq!(t.id_of_alias("C_NEW"), Some(TRANSLATION_STRINGS_START + 3));
    assert_eq!(t.get(TRANSLATION_STRINGS_START + 1), "Eye of doom");
    assert_eq!(t.entry("B_ALIAS").unwrap().overridden_from, vec![0]);
    assert_eq!(t.entry("B_ALIAS").unwrap().file, 1);
    assert_eq!(t.id_of_alias("1011"), Some(1011));
    assert_eq!(t.id_of_alias("2001"), None);
    assert_eq!(t.id_of_alias("nope"), None);
    assert_eq!(t.get(5), "oh_crap_invalid_string_id");
    // to_toml round trips
    let re = TranslationFile::parse(&base.to_toml(), p).unwrap();
    assert_eq!(re, base);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn shipped_translation_toml_loads() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let f = testutil::resolve_ci(&root, "fxdata/translation.toml").unwrap();
    let tf = TranslationFile::load(&f).unwrap();
    assert!(!tf.entries.is_empty());
    let mut langs: BTreeMap<String, usize> = BTreeMap::new();
    for e in &tf.entries {
        for k in e.texts.keys() {
            *langs.entry(k.clone()).or_default() += 1;
        }
        assert!(e.texts.contains_key("ENG"), "{} has no ENG", e.alias);
    }
    println!(
        "translation.toml: {} aliases, texts per language {langs:?}",
        tf.entries.len()
    );
    let t = TranslationTable::load(&[f], "GER").unwrap();
    assert_eq!(t.entry("BEHOLDER_NAME").unwrap().text, "Betrachter");
    assert!(t.id_of_alias("BEHOLDER_NAME").is_some());
}

#[test]
fn campaign_string_file_selection_and_fallbacks() {
    let e = |c: &str, f: &str| (c.to_string(), f.to_string());
    let entries = vec![
        e("ENG", "x/eng.dat"),
        e("FRE", "x/fre.dat"),
        e("GER", "x/ger.dat"),
    ];
    let c = campaign_strings_files(&entries, Lang::German);
    assert_eq!(
        (c.fname.as_deref(), c.fname_lang),
        (Some("x/ger.dat"), Some(Lang::German))
    );
    assert_eq!(c.fname_eng.as_deref(), Some("x/eng.dat"));
    assert_eq!(c.default_language, Some(Lang::English));
    // a language the campaign lacks: the first entry is the default ("If there's no file for
    // current language, then first entry is selected")
    let c = campaign_strings_files(&entries, Lang::Polish);
    assert_eq!(
        (c.fname.as_deref(), c.fname_lang),
        (Some("x/eng.dat"), Some(Lang::English))
    );
    // first entry French, no English entry: eng falls back to the first entry
    let c = campaign_strings_files(&[e("FRE", "f.dat"), e("GER", "g.dat")], Lang::Polish);
    assert_eq!(
        (
            c.fname.as_deref(),
            c.fname_eng.as_deref(),
            c.default_language
        ),
        (Some("f.dat"), Some("f.dat"), Some(Lang::French))
    );
}

/// A level with strings in all four places: the game's id resolution.
#[test]
fn resolver_follows_the_games_order() {
    let dir = kfx_formats::testutil::scratch_dir(&format!(
        "kfx-formats/kfx_resolver_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let (fx, camp, lv, md) = (
        dir.join("fxdata"),
        dir.join("camp"),
        dir.join("levels"),
        dir.join("mods/m1"),
    );
    for d in [&fx, &camp, &lv, &md.join("fxdata")] {
        std::fs::create_dir_all(d).unwrap();
    }
    let table = |v: &[(usize, &str)]| {
        let mut t = StringTable::new(Lang::English);
        for (i, s) in v {
            t.set(*i, s).unwrap();
        }
        // padded so the file is at least 16 bytes
        t.set(30, "padding padding").unwrap();
        t.to_bytes()
    };
    // GUI: slot 5 and 7 in the base, slot 5 replaced by the mod (mod has no empty string rule)
    std::fs::write(
        fx.join("gtext_eng.dat"),
        table(&[(0, "g0"), (1, "g1"), (5, "gui5"), (7, "gui7")]),
    )
    .unwrap();
    std::fs::write(
        md.join("fxdata/gtext_eng.dat"),
        table(&[(5, "mod5"), (6, "")]),
    )
    .unwrap();
    std::fs::write(
        camp.join("text_eng.dat"),
        table(&[(0, "c0"), (1, ""), (2, "c2")]),
    )
    .unwrap();
    std::fs::write(lv.join("map00007.eng.dat"), table(&[(0, "L0"), (3, "L3")])).unwrap();
    std::fs::write(
        fx.join("translation.toml"),
        "[HELLO]\nENG = \"hello\"\nGER = \"hallo\"\n",
    )
    .unwrap();
    let src = StringSources::resolve(
        &dir,
        &[md.clone()],
        &[("ENG".into(), "camp/text_eng.dat".into())],
        Some(&lv),
        Some(7),
        Lang::English,
    )
    .unwrap();
    assert_eq!(src.gui.len(), 2);
    assert_eq!(src.campaign.len(), 1);
    assert!(src.level.is_some());
    let mut src = src;
    src.translation = vec![fx.join("translation.toml")];
    let r = StringResolver::load(&src).unwrap();
    assert_eq!(r.get(0), "L0", "level string wins");
    assert_eq!(r.source_of(0), StringSource::Level);
    assert_eq!(r.get(2), "c2", "campaign string");
    assert_eq!(
        r.get(1),
        "g1",
        "empty campaign string falls to the GUI string of the same index"
    );
    assert_eq!(r.source_of(1), StringSource::Gui);
    assert_eq!(r.get(3), "L3");
    assert_eq!(
        r.get((GUI_STRINGS_START + 5) as i64),
        "mod5",
        "a mod's GUI string replaces the base one"
    );
    assert_eq!(r.get((GUI_STRINGS_START + 7) as i64), "gui7");
    assert_eq!(
        r.get((GUI_STRINGS_START + 6) as i64),
        "",
        "an empty string does not replace"
    );
    assert_eq!(r.get(TRANSLATION_STRINGS_START as i64), "hello");
    assert_eq!(r.get(-3), "invalid string id");
    assert!(r
        .get((GUI_STRINGS_START + 100_000) as i64)
        .starts_with("untranslated"));
    // missing level language: the campaign's default language file is used
    std::fs::rename(lv.join("map00007.eng.dat"), lv.join("map00007.ger.dat")).unwrap();
    let src = StringSources::resolve(
        &dir,
        &[],
        &[("GER".into(), "camp/text_eng.dat".into())],
        Some(&lv),
        Some(7),
        Lang::English,
    )
    .unwrap();
    assert_eq!(src.level.as_ref().map(|(_, l)| *l), Some(Lang::German));
    // no GUI base file: error
    std::fs::remove_file(fx.join("gtext_eng.dat")).unwrap();
    assert!(StringSources::resolve(&dir, &[], &[], None, None, Lang::English).is_err());
    let _ = campaign::Campaign::load; // the [strings] entries come from Campaign::strings()
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every one- and two-byte sequence (and ASCII-padded ones) survives decode -> encode in every
/// language: invalid bytes become reversible escapes. Reports the sequences that decode to a
/// character the encoder maps to other bytes (duplicate code points of the code page).
#[test]
fn every_byte_sequence_round_trips_through_text() {
    for lang in [
        Lang::English,
        Lang::Russian,
        Lang::Japanese,
        Lang::ChineseInt,
        Lang::ChineseTra,
        Lang::Korean,
    ] {
        let mut dup: Vec<(Vec<u8>, Vec<u8>)> = vec![];
        let mut check = |seq: &[u8]| {
            let text = decode(lang, seq);
            match encode(lang, &text) {
                Ok(b) if b == seq => {}
                Ok(b) => dup.push((seq.to_vec(), b)),
                Err(e) => panic!("{} {seq:02X?}: {e}", lang.code()),
            }
        };
        for a in 1..=255u8 {
            check(&[a]);
            for b in 1..=255u8 {
                check(&[a, b]);
                check(&[b'x', a, b, b'y']);
            }
        }
        let sample: Vec<String> = dup
            .iter()
            .take(6)
            .map(|(s, b)| format!("{s:02X?} -> {b:02X?}"))
            .collect();
        println!(
            "{}: {} sequences decode to characters that re-encode to other bytes {sample:?}",
            lang.code(),
            dup.len()
        );
        // the table keeps those bytes when the string is not edited
        if let Some((seq, _)) = dup.first() {
            let mut t = StringTable::parse(&[seq.as_slice(), b"\0"].concat(), lang);
            let text = t.get(0).unwrap();
            t.set(0, &text).unwrap();
            assert_eq!(t.raw(0).unwrap(), seq.as_slice());
        }
    }
}
