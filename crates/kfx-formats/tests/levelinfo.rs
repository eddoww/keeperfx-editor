use kfx_formats::levelinfo::*;
use kfx_formats::rnc;
use kfx_formats::testutil::install_dir;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn walk(dir: &std::path::Path, ext: &str, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, ext, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
                out.push(p);
            }
        }
    }
}

#[test]
fn all_lof_roundtrip_and_typed() {
    let Some(root) = install_dir() else { return };
    let mut files = vec![];
    walk(&root, "lof", &mut files);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut sizes: BTreeMap<(i32, i32), usize> = BTreeMap::new();
    for f in &files {
        let raw = rnc::load_file(f).unwrap();
        let lof = Lof::parse(&raw);
        assert_eq!(lof.to_bytes(), raw, "{}", f.display());
        let info = lof.info();
        // re-applying the parsed info must not change a byte
        let mut l2 = lof.clone();
        l2.set_info(&info);
        assert_eq!(l2.to_bytes(), raw, "{}", f.display());
        // number in the file name
        assert!(level_number_from_file_name(&f.file_name().unwrap().to_string_lossy()).is_some());
        *sizes.entry(info.mapsize_or_default()).or_default() += 1;
        for k in &info.kinds {
            *kinds.entry(k.name().into()).or_default() += 1;
        }
        if info.kinds.is_empty() {
            *kinds.entry("(none)".into()).or_default() += 1;
        }
    }
    println!(
        "lof: {} files byte-exact; kinds {kinds:?}; mapsizes {sizes:?}",
        files.len()
    );
}

#[test]
fn all_lif_roundtrip() {
    let Some(root) = install_dir() else { return };
    let mut files = vec![];
    walk(&root, "lif", &mut files);
    let (mut entries, mut sid, mut odd) = (0, 0, vec![]);
    for f in &files {
        let raw = rnc::load_file(f).unwrap();
        let lif = Lif::parse(&raw);
        assert_eq!(lif.to_bytes(), raw, "{}", f.display());
        for e in lif.entries() {
            entries += 1;
            if e.string_id.is_some() {
                sid += 1;
            }
        }
        for o in lif.odd_lines() {
            odd.push(format!(
                "{}: {:?}",
                f.file_name().unwrap().to_string_lossy(),
                o
            ));
        }
    }
    println!(
        "lif: {} files byte-exact, {entries} entries ({sid} with #string id), {} odd lines {odd:?}",
        files.len(),
        odd.len()
    );
}

#[test]
fn lof_edit_and_new() {
    let src = b"; c\r\nNAME_TEXT = A\r\nOPTIONS = \r\nFOO = bar\r\nMAPSIZE = 85 85\r\n";
    let mut l = Lof::parse(src);
    let mut i = l.info();
    assert_eq!(i.name_text.as_deref(), Some("A"));
    assert_eq!(i.mapsize, Some((85, 85)));
    i.mapsize = Some((101, 99));
    i.ensign = Some("TUTORIAL".into());
    i.kinds = vec![LevelKind::Multi, LevelKind::Free];
    l.set_info(&i);
    let t = String::from_utf8(l.to_bytes()).unwrap();
    assert!(
        t.contains("OPTIONS = TUTORIAL\r\n")
            && t.contains("MAPSIZE = 101 99\r\n")
            && t.contains("FOO = bar\r\n"),
        "{t:?}"
    );
    assert!(t.contains("KIND = MULTI FREE"));
    assert_eq!(Lof::parse(&l.to_bytes()).info(), i);
    let n = Lof::new(&LevelInfo {
        name_text: Some("X".into()),
        mapsize: Some((85, 85)),
        ..Default::default()
    });
    assert_eq!(n.info().name_text.as_deref(), Some("X"));
    assert_eq!(n.info().mapsize, Some((85, 85)));
}

#[test]
fn lif_semantics() {
    let l =
        Lif::parse(b"01, Red Carpet\r\n; c\r\n190, #222\r\n; Armageddon\r\n  7 : x, y\r\nbad\r\n");
    let e = l.entries();
    assert_eq!(e.len(), 3);
    assert_eq!((e[0].number, e[0].name.as_str()), (1, "Red Carpet"));
    assert_eq!((e[1].number, e[1].string_id), (190, Some(222)));
    assert_eq!(e[2].name, "x, y");
    assert_eq!(l.odd_lines(), vec!["bad"]);
    let mut l2 = l.clone();
    l2.set_entry(7, "Seven");
    l2.set_entry(9, "Nine");
    assert!(l2.to_string().ends_with("bad\r\n9, Nine\r\n"));
    assert_eq!(l2.remove_entry(1), 1);
}
