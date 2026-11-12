//! Hostile archives: escapes, links, special files, bombs, lying sizes, duplicates.
use kfx_archive::*;
use std::io::Write;
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = kfx_formats::testutil::scratch_dir("archive")
            .join(format!("{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
    fn join(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A raw tar entry: the name and link name are written into the header as they are (the `tar`
/// crate's own setters refuse hostile names, which is what is being tested here).
struct Raw<'a> {
    name: &'a str,
    kind: tar::EntryType,
    data: &'a [u8],
    link: &'a str,
    mode: u32,
}

fn raw(name: &str, data: &'static [u8]) -> Raw<'static> {
    Raw {
        name: Box::leak(name.to_string().into_boxed_str()),
        kind: tar::EntryType::Regular,
        data,
        link: "",
        mode: 0o644,
    }
}

fn tar_gz(path: &Path, entries: &[Raw]) {
    let f = std::fs::File::create(path).unwrap();
    let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
    let mut b = tar::Builder::new(gz);
    for e in entries {
        let mut h = tar::Header::new_gnu();
        {
            let g = h.as_gnu_mut().unwrap();
            g.name[..e.name.len()].copy_from_slice(e.name.as_bytes());
            g.linkname[..e.link.len()].copy_from_slice(e.link.as_bytes());
        }
        h.set_entry_type(e.kind);
        h.set_size(e.data.len() as u64);
        h.set_mode(e.mode);
        h.set_cksum();
        b.append(&h, e.data).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap();
}

fn zip_file(path: &Path, files: &[(&str, &[u8])], symlink: Option<(&str, &str)>) {
    let f = std::fs::File::create(path).unwrap();
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default();
    for (n, b) in files {
        z.start_file(*n, o).unwrap();
        z.write_all(b).unwrap();
    }
    if let Some((l, t)) = symlink {
        z.add_symlink(l, t, o).unwrap();
    }
    z.finish().unwrap();
}

#[test]
fn good_tar_gz_extracts_with_exec_bits() {
    let s = Scratch::new("good");
    let a = s.join("good.tar.gz");
    tar_gz(
        &a,
        &[
            Raw {
                kind: tar::EntryType::Directory,
                ..raw("pkg/", b"")
            },
            Raw {
                mode: 0o755,
                ..raw("pkg/kfx-editor", b"#!/bin/sh\n")
            },
            raw("pkg/docs/manual/README.md", b"manual"),
        ],
    );
    assert_eq!(detect_file(&a).unwrap(), Format::TarGz);
    let l = list(&a, &Limits::default()).unwrap();
    assert!(l.problems().is_empty(), "{:?}", l.problems());
    assert_eq!(l.files().count(), 2);
    let out = s.join("out");
    let files = extract_all(&a, &out, &Limits::default()).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(
        std::fs::read(out.join("pkg/docs/manual/README.md")).unwrap(),
        b"manual"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = |p: &str| std::fs::metadata(out.join(p)).unwrap().permissions().mode();
        assert_eq!(m("pkg/kfx-editor") & 0o111, 0o111);
        assert_eq!(m("pkg/docs/manual/README.md") & 0o111, 0);
    }
    assert_eq!(
        read_one(&a, "pkg/kfx-editor", &Limits::default()).unwrap(),
        Some(b"#!/bin/sh\n".to_vec())
    );
}

#[test]
fn hostile_tar_entries_refuse_the_whole_archive() {
    let s = Scratch::new("evil");
    let cases: Vec<(&str, Raw)> = vec![
        ("parent", raw("pkg/../../evil.txt", b"x")),
        ("absolute", raw("/tmp/evil.txt", b"x")),
        ("drive", raw("C:\\evil.txt", b"x")),
        (
            "symlink",
            Raw {
                kind: tar::EntryType::Symlink,
                link: "/etc/passwd",
                ..raw("pkg/link", b"")
            },
        ),
        (
            "hardlink",
            Raw {
                kind: tar::EntryType::Link,
                link: "../../outside",
                ..raw("pkg/hard", b"")
            },
        ),
        (
            "fifo",
            Raw {
                kind: tar::EntryType::Fifo,
                ..raw("pkg/fifo", b"")
            },
        ),
        (
            "device",
            Raw {
                kind: tar::EntryType::Char,
                ..raw("pkg/dev", b"")
            },
        ),
    ];
    for (tag, bad) in cases {
        let a = s.join(&format!("{tag}.tar.gz"));
        tar_gz(&a, &[raw("pkg/ok.txt", b"ok"), bad]);
        let l = list(&a, &Limits::default()).unwrap();
        assert_eq!(l.problems().len(), 1, "{tag}: {:?}", l.problems());
        // reading never yields the flagged entry
        let mut got = vec![];
        read_entries(&a, &|_| true, &Limits::default(), &mut |n, _| {
            got.push(n.to_string());
            Ok(())
        })
        .unwrap();
        assert_eq!(got, vec!["pkg/ok.txt"], "{tag}");
        let out = s.join(&format!("out-{tag}"));
        let e = extract_all(&a, &out, &Limits::default()).unwrap_err();
        assert!(matches!(e, Error::Unsafe(_)), "{tag}: {e}");
        assert!(!out.exists(), "{tag}: nothing written");
    }
    assert!(!s.join("evil.txt").exists() && !s.0.parent().unwrap().join("evil.txt").exists());
}

#[test]
fn hostile_zip_entries_refuse_the_whole_archive() {
    let s = Scratch::new("zip");
    let a = s.join("evil.zip");
    zip_file(&a, &[("pkg/ok.txt", b"ok"), ("../evil.txt", b"x")], None);
    let out = s.join("out");
    assert!(matches!(
        extract_all(&a, &out, &Limits::default()),
        Err(Error::Unsafe(_))
    ));
    assert!(!out.exists() && !s.0.join("evil.txt").exists());
    let b = s.join("link.zip");
    zip_file(
        &b,
        &[("pkg/ok.txt", b"ok")],
        Some(("pkg/link", "/etc/passwd")),
    );
    let e = extract_all(&b, &out, &Limits::default()).unwrap_err();
    assert!(e.to_string().contains("symbolic link"), "{e}");
    // a good zip extracts
    let c = s.join("good.zip");
    zip_file(&c, &[("pkg/a.txt", b"a"), ("pkg/sub/b.txt", b"b")], None);
    assert_eq!(extract_all(&c, &out, &Limits::default()).unwrap().len(), 2);
    assert_eq!(std::fs::read(out.join("pkg/sub/b.txt")).unwrap(), b"b");
}

#[test]
fn limits_and_duplicates() {
    let s = Scratch::new("limits");
    // oversize entry (declared size), refused while listing
    let a = s.join("big.tar.gz");
    tar_gz(&a, &[raw("pkg/big.bin", &[0u8; 4096])]);
    let small = Limits {
        max_file: 1024,
        ..Limits::default()
    };
    assert!(matches!(list(&a, &small), Err(Error::Unsafe(_))));
    // too many entries
    let many: Vec<Raw> = (0..20).map(|i| raw(&format!("pkg/f{i}"), b"x")).collect();
    let b = s.join("many.tar.gz");
    tar_gz(&b, &many);
    let few = Limits {
        max_entries: 10,
        ..Limits::default()
    };
    assert!(matches!(list(&b, &few), Err(Error::Unsafe(_))));
    // total size
    let total = Limits {
        max_total: 10,
        ..Limits::default()
    };
    assert!(matches!(list(&b, &total), Err(Error::Unsafe(_))));
    // the same file twice: the second write is refused
    let c = s.join("dup.tar.gz");
    tar_gz(&c, &[raw("pkg/a", b"1"), raw("pkg/./a", b"2")]);
    let e = extract_all(&c, &s.join("out"), &Limits::default()).unwrap_err();
    assert!(e.to_string().contains("written twice"), "{e}");
    // a file where a folder is needed
    let d = s.join("clash.tar.gz");
    tar_gz(&d, &[raw("pkg/a", b"1"), raw("pkg/a/b", b"2")]);
    assert!(extract_all(&d, &s.join("out2"), &Limits::default()).is_err());
    // not an archive
    let n = s.join("not.tar.gz");
    std::fs::write(&n, b"hello").unwrap();
    assert!(matches!(
        list(&n, &Limits::default()),
        Err(Error::Archive { .. })
    ));
    // a truncated archive is an error, not a panic
    let t = s.join("trunc.tar.gz");
    let full = std::fs::read(&a).unwrap();
    std::fs::write(&t, &full[..full.len() / 2]).unwrap();
    assert!(extract_all(&t, &s.join("out3"), &Limits::default()).is_err());
}
