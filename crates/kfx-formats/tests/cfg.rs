use kfx_formats::cfg::*;
use kfx_formats::testutil::install_dir;

fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")) {
                out.push(p);
            }
        }
    }
}

#[test]
fn roundtrip_all_install_cfgs() {
    let Some(root) = install_dir() else { return };
    let mut files = vec![];
    walk(&root, &mut files);
    let (mut n, mut crlf, mut nonascii, mut dup_keys) = (0, 0, vec![], 0);
    for f in &files {
        let raw = std::fs::read(f).unwrap();
        let doc = CfgDoc::from_bytes(&raw);
        assert_eq!(doc.to_bytes(), raw, "{}", f.display());
        n += 1;
        if raw.windows(2).any(|w| w == b"\r\n") {
            crlf += 1;
        }
        if raw.iter().any(|&b| b >= 0x80) {
            nonascii.push(f.file_name().unwrap().to_string_lossy().to_string());
        }
        for s in doc.sections() {
            let mut seen = std::collections::HashSet::new();
            for e in &s.entries {
                if !seen.insert(e.key.to_ascii_lowercase()) {
                    dup_keys += 1;
                }
            }
        }
    }
    println!(
        "cfg round-trip: {n} files byte-exact ({crlf} with CRLF, {} with non-ASCII bytes: \
        {:?}, {dup_keys} repeated keys within a section)",
        nonascii.len(),
        nonascii
    );
    assert!(n > 0);
}

const SAMPLE: &str = "; head\r\n\r\nNAME = Foo Bar \r\n[ one ]\r\nKey=1 2  3\r\n  ; c\r\nkey = \
    9\r\n\r\n[Two]\r\nX\r\n";

#[test]
fn model_basics() {
    let mut d = CfgDoc::parse(SAMPLE);
    assert_eq!(d.to_string(), SAMPLE);
    assert_eq!(d.section_names(), vec!["one", "Two"]);
    assert_eq!(d.get("", "name").unwrap(), "Foo Bar");
    assert_eq!(d.get("ONE", "KEY").unwrap(), "9");
    assert_eq!(d.get_all("one", "key").len(), 2);
    assert_eq!(d.get_tokens("one", "key").unwrap(), vec!["9"]);
    assert_eq!(d.get_all("one", "key")[0].tokens, vec!["1", "2", "3"]);
    d.set("one", "key", "10");
    assert!(d.to_string().contains("key = 10\r\n"));
    d.set("Two", "X", "5");
    assert!(d.to_string().ends_with("X = 5\r\n"));
    d.add("one", "New", "a b");
    let t = d.to_string();
    assert!(t.contains("key = 10\r\nNew = a b\r\n\r\n[Two]"), "{t:?}");
    assert_eq!(d.remove("one", "key"), 2);
    d.add_section("Three");
    d.add("Three", "A", "1");
    assert!(
        d.to_string().ends_with("[Three]\r\nA = 1\r\n"),
        "{:?}",
        d.to_string()
    );
    assert!(d.remove_section("Two"));
    assert!(!d.has_section("Two"));
    // reparse of the output is stable
    let s = d.to_string();
    assert_eq!(CfgDoc::parse(&s).to_string(), s);
}

#[test]
fn bytes_and_eol_edge_cases() {
    let raw: Vec<u8> = b"A=\xe9\xff\r[x]\rB\x01C\n\n\x1a\n[y".to_vec();
    let d = CfgDoc::from_bytes(&raw);
    assert_eq!(d.to_bytes(), raw);
    let e = CfgDoc::from_bytes(b"");
    assert_eq!(e.to_bytes(), b"");
    assert_eq!(CfgDoc::from_bytes(b"no newline").to_bytes(), b"no newline");
}
