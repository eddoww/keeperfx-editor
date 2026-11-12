//! Extraction safety: zip-slip, absolute paths, symlinks, bombs.
mod common;
use common::*;
use kfx_workshop::archive::*;
use kfx_workshop::classify::*;
use kfx_workshop::install::*;
use std::io::Write;

fn evil_zip(path: &std::path::Path) {
    let f = std::fs::File::create(path).unwrap();
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default();
    for (n, b) in [
        ("map00001.slb", &b"ok"[..]),
        ("../evil.txt", b"x"),
        ("/abs/evil.txt", b"x"),
        ("C:\\win\\evil.txt", b"x"),
        ("a/../../evil2.txt", b"x"),
        ("ok\\..\\..\\evil3.txt", b"x"),
    ] {
        z.start_file(n, o).unwrap();
        z.write_all(b).unwrap();
    }
    z.add_symlink("link.txt", "/etc/passwd", o).unwrap();
    z.finish().unwrap();
}

#[test]
fn path_normalisation_rejects_escapes() {
    for bad in [
        "../x",
        "a/../../x",
        "/etc/passwd",
        "C:\\x",
        "c:/x",
        "a\\..\\..\\x",
        "x\0y",
        "",
        "./",
        "a/\u{7}b",
    ] {
        assert!(safe_rel_path(bad).is_err(), "{bad:?} must be rejected");
    }
    assert_eq!(safe_rel_path("./a//b\\c.txt").unwrap(), "a/b/c.txt");
    assert_eq!(
        safe_rel_path("levels/standard/MAP00001.SLB").unwrap(),
        "levels/standard/MAP00001.SLB"
    );
    assert_eq!(safe_rel_path("a/./b").unwrap(), "a/b");
    assert_eq!(
        safe_rel_path("..foo/bar").unwrap(),
        "..foo/bar",
        "a name that merely starts with dots is fine"
    );
}

#[test]
fn hostile_entries_are_flagged_never_read_never_installed() {
    let s = Scratch::new("sf-evil");
    let z = s.join("evil.zip");
    evil_zip(&z);
    let l = list(&z, &Limits::default()).unwrap();
    assert_eq!(l.problems().len(), 6, "{:?}", l.problems());
    assert!(l.problems().iter().any(|p| p.contains("symbolic link")));
    assert_eq!(
        l.files().count(),
        1,
        "only the harmless file is a candidate"
    );
    // reading never yields a flagged entry
    let mut got = vec![];
    read_entries(&z, &|_| true, &Limits::default(), &mut |n, _| {
        got.push(n.to_string());
        Ok(())
    })
    .unwrap();
    assert_eq!(got, vec!["map00001.slb"]);
    // the classifier reports them and the plan refuses
    let insp = classify_archive(&z, &Limits::default()).unwrap();
    assert_eq!(insp.class.unsafe_entries.len(), 6);
    let root = s.join("install");
    private_install(&root);
    let plan = plan(&root, &z, &insp, &PlanOptions::new("evil")).unwrap();
    assert!(plan.is_blocked() && plan.blocked.iter().any(|b| b.contains("unsafe")));
    let store = Store::new(s.join("store"));
    let before = snapshot(&root);
    assert!(apply(&root, &plan, &store, &Limits::default()).is_err());
    assert_eq!(snapshot(&root), before, "nothing written");
    assert!(
        !s.join("evil.txt").exists()
            && !s.join("evil2.txt").exists()
            && !root.join("evil.txt").exists()
    );
}

#[test]
fn bomb_limits() {
    let s = Scratch::new("sf-bomb");
    // too many entries
    let z = s.join("many.zip");
    let names: Vec<String> = (0..50).map(|i| format!("f{i}.txt")).collect();
    make_zip(
        &z,
        &names
            .iter()
            .map(|n| (n.as_str(), &b"x"[..]))
            .collect::<Vec<_>>(),
    );
    let e = list(
        &z,
        &Limits {
            max_entries: 10,
            ..Limits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(e, kfx_workshop::Error::Unsafe(_)), "{e}");
    // one huge file (declared size)
    let big = s.join("big.zip");
    make_zip(&big, &[("big.bin", &vec![0u8; 2_000_000][..])]);
    assert!(matches!(
        list(
            &big,
            &Limits {
                max_file: 1_000_000,
                ..Limits::default()
            }
        ),
        Err(kfx_workshop::Error::Unsafe(_))
    ));
    // total size
    assert!(matches!(
        list(
            &big,
            &Limits {
                max_total: 1_000_000,
                ..Limits::default()
            }
        ),
        Err(kfx_workshop::Error::Unsafe(_))
    ));
    // ratio (only applies above 64 MiB unpacked)
    let ratio = s.join("ratio.zip");
    make_zip(&ratio, &[("zeros.bin", &vec![0u8; 80 << 20][..])]);
    let e = list(
        &ratio,
        &Limits {
            max_ratio: 100,
            ..Limits::default()
        },
    )
    .unwrap_err();
    assert!(e.to_string().contains("ratio"), "{e}");
    // defaults accept a normal archive
    assert!(list(&big, &Limits::default()).is_ok());
    // reading respects the cap too
    let mut n = 0;
    assert!(read_entries(
        &big,
        &|_| true,
        &Limits {
            max_file: 1_000_000,
            ..Limits::default()
        },
        &mut |_, _| {
            n += 1;
            Ok(())
        }
    )
    .is_err());
    assert_eq!(n, 0);
}

#[test]
fn garbage_and_unknown_formats_are_errors_not_panics() {
    let s = Scratch::new("sf-garbage");
    let p = s.join("junk.zip");
    std::fs::write(&p, b"not an archive at all").unwrap();
    assert!(list(&p, &Limits::default()).is_err());
    let trunc = s.join("trunc.zip");
    make_zip(&s.join("ok.zip"), &[("a.txt", &b"hello world"[..])]);
    let b = std::fs::read(s.join("ok.zip")).unwrap();
    std::fs::write(&trunc, &b[..b.len() / 2]).unwrap();
    assert!(list(&trunc, &Limits::default()).is_err());
    let p7 = s.join("bad.7z");
    std::fs::write(
        &p7,
        b"7z\xbc\xaf\x27\x1c\x00\x04garbage garbage garbage garbage garbage",
    )
    .unwrap();
    assert!(list(&p7, &Limits::default()).is_err());
    assert!(list(&s.join("missing.zip"), &Limits::default()).is_err());
}

#[test]
fn staging_and_destinations_stay_inside_the_install() {
    // a name like "levels/personal/../../../x" is rejected at list time; a plan made from a
    // classification with a hostile wrapper cannot produce a destination outside the root
    let s = Scratch::new("sf-dest");
    let z = s.join("w.zip");
    make_zip(
        &z,
        &[
            ("..%2f..%2fx/map00001.slb", b"1"),
            ("..%2f..%2fx/map00001.own", b"1"),
        ],
    );
    let insp = classify_archive(&z, &Limits::default()).unwrap();
    let root = s.join("install");
    private_install(&root);
    let p = plan(&root, &z, &insp, &PlanOptions::new("w")).unwrap();
    for i in &p.items {
        assert!(!i.dest.contains(".."), "{}", i.dest);
        assert!(kfx_workshop::archive::safe_rel_path(&i.dest).is_ok());
    }
}
