//! Offscreen screenshot of the translation grid on a private copy of the `ancntkpr` campaign
//! of a KeeperFX install (only its texts; the install named by `KFX_INSTALL` is only read).
//!
//!   KFX_INSTALL=<install> cargo run -p kfx-prefab --example screens_texts -- [OUT_DIR]
//!
//! `OUT_DIR` defaults to `prefab/shots` of the test scratch folder
//! ([`kfx_formats::testutil::scratch_dir`]: `$KFX_TEST_TMP`, else `target/tmp`); the copy is made
//! in `prefab/shot-texts-copy` next to it.
use egui::vec2;
use egui_kittest::Harness;
use kfx_formats::strings::Lang;
use kfx_prefab::texts::widget::TranslationGrid;
use kfx_prefab::texts::*;
use std::path::PathBuf;

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| kfx_formats::testutil::scratch_dir("prefab/shots"));
    std::fs::create_dir_all(&out).unwrap();
    let Some(install) = kfx_formats::testutil::install_dir() else {
        eprintln!("set KFX_INSTALL to the folder of a KeeperFX install");
        std::process::exit(2)
    };
    let root = kfx_formats::testutil::scratch_dir("prefab/shot-texts-copy");
    let _ = std::fs::remove_dir_all(&root);
    let camp = root.join("campgns/ancntkpr");
    std::fs::create_dir_all(&camp).unwrap();
    let src = kfx_formats::testutil::resolve_ci(&install, "campgns").unwrap();
    std::fs::copy(src.join("ancntkpr.cfg"), root.join("campgns/ancntkpr.cfg")).unwrap();
    for e in std::fs::read_dir(src.join("ancntkpr")).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if (n.ends_with(".txt") || n.ends_with(".lif") || n.starts_with("text_"))
            && e.path().is_file()
        {
            std::fs::copy(e.path(), camp.join(&n)).unwrap();
        }
    }
    kfx_formats::testutil::symlink(
        &kfx_formats::testutil::resolve_ci(&install, "fxdata").unwrap(),
        &root.join("fxdata"),
    )
    .unwrap();
    let spec = ProjectSpec::for_campaign(
        &root,
        &root.join("campgns/ancntkpr.cfg"),
        Some((1..=4).collect()),
    )
    .unwrap();
    let mut p = TextProject::open(spec).unwrap();
    // show every state: a translator edited the English source of row 2, row 3 of Dutch is a copy
    p.add_language(Lang::Dutch);
    p.create_campaign_file(Lang::Dutch).unwrap();
    p.set_text(2, Lang::French, "Texte fran\u{e7}ais de test")
        .unwrap();
    p.set_text(2, Lang::English, &format!("{} (edited)", p.source_text(2)))
        .unwrap();
    p.copy_source_to_missing(Some(&[Lang::Dutch]));
    p.set_text(5, Lang::Dutch, "Een echte vertaling").unwrap();
    let mut keep = vec![Lang::English, Lang::French, Lang::German, Lang::Dutch];
    keep.dedup();
    p.langs.retain(|l| keep.contains(l));
    let grid = TranslationGrid::new(p);
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1500.0, 800.0))
        .wgpu()
        .with_accessibility_check(false)
        .build_ui_state(|ui, g: &mut TranslationGrid| g.show(ui), grid);
    h.run_steps(4);
    let img = h.render().expect("render");
    img.save(out.join("translation-grid.png")).unwrap();
    println!("wrote {}", out.join("translation-grid.png").display());
    // cleanup of the private copy (the fxdata link first, never followed)
    let _ = std::fs::remove_file(root.join("fxdata"));
    let _ = std::fs::remove_dir_all(&root);
}
