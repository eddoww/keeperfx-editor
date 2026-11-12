//! Packaging a shipped level and a shipped campaign (from private copies) and feeding the zips
//! back through the classifier and installer.
mod common;
use common::*;
use kfx_workshop::archive::Limits;
use kfx_workshop::classify::*;
use kfx_workshop::install::*;
use kfx_workshop::package::*;
use std::path::{Path, PathBuf};

fn real_install() -> Option<PathBuf> {
    let p = kfx_formats::testutil::install_dir()?;
    p.join("campgns/ami2019.cfg").is_file().then_some(p)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

fn install_zip_into_fresh_install(
    zip: &Path,
    s: &Scratch,
    name: &str,
) -> (PathBuf, Manifest, InstallPlan) {
    let root = s.join(&format!("install-{name}"));
    private_install(&root);
    let insp = classify_archive(zip, &Limits::default()).unwrap();
    let plan = plan(&root, zip, &insp, &PlanOptions::new(name)).unwrap();
    assert!(!plan.is_blocked(), "{:?}", plan.blocked);
    let store = Store::new(s.join(&format!("store-{name}")));
    let m = apply(&root, &plan, &store, &Limits::default()).unwrap();
    (root, m, plan)
}

#[test]
fn package_a_shipped_level_and_install_it_back() {
    let s = Scratch::new("pk-level");
    let folder = s.join("work");
    std::fs::create_dir_all(&folder).unwrap();
    // a shipped level when the test install is there, else a synthetic one
    let (number, from_real) = match real_install() {
        Some(r) => {
            let src = r.join("levels/classic");
            let mut n = 0;
            for e in std::fs::read_dir(&src).unwrap().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.to_ascii_lowercase().starts_with("map00206.") {
                    std::fs::copy(e.path(), folder.join(&name)).unwrap();
                    n = 206;
                }
            }
            (n, n == 206)
        }
        None => (0, false),
    };
    let number = if from_real {
        number
    } else {
        for (n, b) in level_files(206, true) {
            std::fs::write(folder.join(n), b).unwrap();
        }
        206
    };
    // extras: sidecar, backup, scratch, a config with an absolute path, a stray file
    std::fs::write(folder.join("map00206.kfxed.toml"), b"[notes]\n").unwrap();
    std::fs::write(folder.join("map00206.txt.bak"), b"old").unwrap();
    std::fs::write(
        folder.join("map00206.rules.cfg"),
        b"[game]\nPATH = C:\\Users\\me\\kfx\\x.png\n",
    )
    .unwrap();
    std::fs::write(folder.join("map00206.weird"), b"?").unwrap();
    std::fs::write(folder.join("map00207.slb"), b"another level, not packaged").unwrap();
    let subject = Subject::Level {
        folder: folder.clone(),
        number,
    };
    let mut opts = PackageOptions::new("Classic Test");
    opts.author = "Tester".into();
    opts.description = "A level for the packaging test.".into();
    opts.host_issues = vec![Issue::new(
        Severity::Warning,
        "validator: 2 unreachable rooms",
    )];
    let plan = gather(&subject, &opts).unwrap();
    let names: Vec<&str> = plan.included().map(|e| e.path.as_str()).collect();
    assert!(
        names.contains(&"map00206.slb") && names.contains(&"map00206_readme.txt"),
        "{names:?}"
    );
    assert!(
        !names.contains(&"map00206.kfxed.toml"),
        "sidecar excluded by default"
    );
    assert!(
        !names.contains(&"map00206.txt.bak"),
        "backups never packaged"
    );
    assert!(
        !names.contains(&"map00207.slb"),
        "other levels are not packaged"
    );
    let pf = &plan.preflight;
    assert!(
        pf.issues
            .iter()
            .any(|i| i.message.contains("validator: 2 unreachable")),
        "host issues are passed through"
    );
    assert!(
        pf.issues
            .iter()
            .any(|i| i.severity == Severity::Error && i.message.contains("absolute path")),
        "{:?}",
        pf.issues
    );
    assert!(pf
        .issues
        .iter()
        .any(|i| i.message.contains("does not read") && i.message.contains("weird")));
    assert!(
        pf.issues
            .iter()
            .any(|i| i.message.contains("files,") && i.message.contains("unpacked")),
        "size report"
    );
    assert!(pf.has_errors());
    // the pre-flight blocks writing until forced
    let zip = s.join("out/level.zip");
    assert!(write_zip(&plan, &zip, false).is_err());
    // fix the config: no more errors
    std::fs::write(folder.join("map00206.rules.cfg"), b"[game]\n").unwrap();
    let mut plan = gather(&subject, &opts).unwrap();
    assert!(!plan.preflight.has_errors(), "{:?}", plan.preflight.issues);
    // the user unchecks the weird file
    plan.opts.excluded.insert("map00206.weird".into());
    plan.refresh();
    assert!(!plan.included().any(|e| e.path == "map00206.weird"));
    let pk = write_zip(&plan, &zip, false).unwrap();
    assert_eq!(
        pk.kind,
        Kind::Level,
        "the installer reads the zip back as a level"
    );
    // deterministic
    let zip2 = s.join("out/level2.zip");
    assert_eq!(write_zip(&plan, &zip2, false).unwrap().sha256, pk.sha256);
    // with the sidecar chosen
    let mut o2 = opts.clone();
    o2.include_sidecar = true;
    assert!(gather(&subject, &o2)
        .unwrap()
        .included()
        .any(|e| e.path == "map00206.kfxed.toml"));
    // back through the installer into a fresh private install: files byte-identical
    let (root, m, _) = install_zip_into_fresh_install(&zip, &s, "lvl");
    for e in plan
        .included()
        .filter(|e| matches!(e.source, EntrySource::File(_)))
    {
        let EntrySource::File(src) = &e.source else {
            continue;
        };
        let dest = root.join("levels/personal").join(&e.path);
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            std::fs::read(src).unwrap(),
            "{}",
            e.path
        );
    }
    assert_eq!(m.levels[0].number, 206);
    // layouts
    let mut o3 = opts.clone();
    o3.layout = PackLayout::Rooted("levels/standard".into());
    let p3 = gather(&subject, &o3).unwrap();
    assert!(
        p3.included()
            .all(|e| e.path.starts_with("levels/standard/")),
        "{:?}",
        p3.included().map(|e| &e.path).collect::<Vec<_>>()
    );
    let z3 = s.join("out/rooted.zip");
    write_zip(&p3, &z3, true).unwrap();
    let i3 = classify_archive(&z3, &Limits::default()).unwrap();
    assert_eq!(
        (i3.class.kind, i3.class.layout),
        (Kind::Level, Layout::Rooted)
    );
    // preview image slot
    let img = s.join("shot.png");
    std::fs::write(&img, b"\x89PNG fake").unwrap();
    let mut o4 = opts.clone();
    o4.preview_image = Some(img);
    let pk4 = write_zip(
        &gather(&subject, &o4).unwrap(),
        &s.join("out/with_preview.zip"),
        true,
    )
    .unwrap();
    assert!(pk4
        .preview
        .as_ref()
        .is_some_and(|p| p.is_file() && p.to_string_lossy().ends_with(".preview.png")));
    // upload helper text
    assert!(upload_form_text(&plan).contains("Category: Map"));
    assert!(upload_page_url().starts_with("https://keeperfx.net/"));
}

#[test]
fn missing_files_and_multiplayer_layout() {
    let s = Scratch::new("pk-missing");
    let folder = s.join("w");
    std::fs::create_dir_all(&folder).unwrap();
    for (n, b) in level_files(1234, false) {
        if !n.ends_with(".slb") && !n.ends_with(".lif") {
            std::fs::write(folder.join(n), b).unwrap();
        }
    }
    let plan = gather(
        &Subject::Level {
            folder: folder.clone(),
            number: 1234,
        },
        &PackageOptions::new("T"),
    )
    .unwrap();
    assert!(plan
        .preflight
        .issues
        .iter()
        .any(|i| i.severity == Severity::Error && i.message.contains("no slabs")));
    assert!(
        plan.included().any(|e| e.path == "map01234.lif"),
        "an overview is generated when missing"
    );
    // multiplayer: .lof KIND = MULTI -> multiplayer/modern
    std::fs::write(
        folder.join("map01234.lof"),
        b"KIND = MULTI\nNAME_TEXT = Duel\nPLAYERS = 2\n",
    )
    .unwrap();
    std::fs::write(folder.join("map01234.slb"), b"x").unwrap();
    let plan = gather(
        &Subject::Level {
            folder,
            number: 1234,
        },
        &PackageOptions::new("T"),
    )
    .unwrap();
    assert!(
        plan.included()
            .all(|e| e.path.starts_with("multiplayer/modern/")),
        "{:?}",
        plan.included().map(|e| &e.path).collect::<Vec<_>>()
    );
    assert!(upload_form_text(&plan).contains("Multiplayer Map"));
}

#[test]
fn package_a_shipped_campaign_and_install_it_back() {
    let Some(real) = real_install() else {
        eprintln!("no test install; skipped");
        return;
    };
    let s = Scratch::new("pk-camp");
    let work = s.join("work");
    private_install(&work);
    // a private copy of one shipped campaign (cfg + the folders it names)
    std::fs::copy(
        real.join("campgns/ami2019.cfg"),
        work.join("campgns/ami2019.cfg"),
    )
    .unwrap();
    let mut copied = vec![];
    for d in ["ami2019", "ami2019_lnd", "ami2019_crtr", "ami2019_cfgs"] {
        if real.join("campgns").join(d).is_dir() {
            copy_dir(&real.join("campgns").join(d), &work.join("campgns").join(d));
            copied.push(d);
        }
    }
    assert!(
        copied.contains(&"ami2019") && copied.contains(&"ami2019_lnd"),
        "{copied:?}"
    );
    // editor leftovers that must not ship
    std::fs::write(work.join("campgns/ami2019/map00001.kfxed.toml"), b"x").unwrap();
    std::fs::write(work.join("campgns/ami2019/map00001.txt.bak"), b"x").unwrap();
    let subject = Subject::Campaign {
        install_root: work.clone(),
        cfg: work.join("campgns/ami2019.cfg"),
    };
    let mut opts = PackageOptions::new("Assmist Isle (test)");
    opts.author = "Test".into();
    let plan = gather(&subject, &opts).unwrap();
    let paths: Vec<&str> = plan.included().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"campgns/ami2019.cfg"));
    assert!(paths
        .iter()
        .any(|p| p.starts_with("campgns/ami2019/map00001.")));
    assert!(
        paths.iter().any(|p| p.starts_with("campgns/ami2019_lnd/")),
        "land view files are packaged"
    );
    assert!(!paths
        .iter()
        .any(|p| p.contains("kfxed.toml") || p.ends_with(".bak")));
    assert!(
        !paths.iter().any(|p| p.contains("keeporig")),
        "the shipped campaign's speech is not bundled"
    );
    assert!(
        plan.entries.iter().any(|e| e.role == "note"
            && e.note
                .as_deref()
                .is_some_and(|n| n.contains("shipped with the game"))),
        "the skipped shared folder is reported"
    );
    assert!(plan.included().any(|e| e.role == "readme"));
    let zip = s.join("out/ami.zip");
    let pk = write_zip(&plan, &zip, true).unwrap();
    assert_eq!(pk.kind, Kind::Campaign);
    println!(
        "campaign zip: {} files, {} bytes; preflight: {:?}",
        pk.files,
        pk.size,
        plan.preflight
            .issues
            .iter()
            .map(|i| format!("{:?}: {}", i.severity, i.message))
            .collect::<Vec<_>>()
    );
    // back through the installer
    let (root, m, p) = install_zip_into_fresh_install(&zip, &s, "ami");
    assert!(p.campaigns.iter().any(|c| c == "campgns/ami2019.cfg"));
    let resolved =
        kfx_formats::campaign::resolve(&root, &root.join("campgns/ami2019.cfg")).unwrap();
    assert_eq!(
        resolved.levels.iter().filter(|l| l.slb.is_some()).count(),
        resolved.levels.len(),
        "every level of the campaign is present after the round trip"
    );
    for e in plan
        .included()
        .filter(|e| matches!(e.source, EntrySource::File(_)) && !e.path.is_empty())
    {
        let EntrySource::File(src) = &e.source else {
            continue;
        };
        assert_eq!(
            std::fs::read(root.join(&e.path)).unwrap(),
            std::fs::read(src).unwrap(),
            "{}",
            e.path
        );
    }
    assert!(m.files.len() >= plan.included().count() - 1);
    // and uninstalls cleanly
    let before_files = m.files.len();
    let r = uninstall(&root, &m.id, &Store::new(s.join("store-ami")), false).unwrap();
    assert_eq!(r.removed.len(), before_files);
    assert!(!root.join("campgns/ami2019").exists());
}
