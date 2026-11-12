//! Offscreen renders of every screen of the campaign editor, to look at the widgets without the
//! editor.
//!
//! Uses `egui_kittest` + wgpu (an offscreen render pass, nothing is shown on the desktop) and
//! writes PNGs into `campaign-ui/shots` of the test scratch folder
//! ([`kfx_formats::testutil::scratch_dir`]: `$KFX_TEST_TMP`, else `target/tmp`).
//!
//!   KFX_INSTALL=<install> cargo run -p kfx-campaign-ui --example campaign_shot -- [ARG]
//!
//! `ARG` is either
//! * a campaign stem (default `keeporig`): the campaign `campgns/<stem>.cfg` of the install.
//! * `broken`: instead, a campaign made by the editor in a private copy of the install (symbolic
//!   links) and damaged on purpose, so the health panel has something to show.
//!
//! The install (environment variable `KFX_INSTALL`) is only read.
use egui::vec2;
use egui_kittest::Harness;
use kfx_campaign_ui::ui::{CampaignEditor, KindTab, Tab};
use kfx_campaign_ui::Pack;
use std::path::{Path, PathBuf};

/// The KeeperFX install named by `KFX_INSTALL`; exits with a message when it is not set.
fn install() -> PathBuf {
    kfx_formats::testutil::install_dir().unwrap_or_else(|| {
        eprintln!("set KFX_INSTALL to the folder of a KeeperFX install");
        std::process::exit(2)
    })
}

fn out_dir() -> PathBuf {
    kfx_formats::testutil::scratch_dir("campaign-ui/shots")
}

fn harness(ed: CampaignEditor, w: f32, h: f32) -> Harness<'static, CampaignEditor> {
    let hh = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(w, h))
        .with_pixels_per_point(1.0)
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(
            |ui, ed: &mut CampaignEditor| {
                ed.show(ui);
            },
            ed,
        );
    // the default fonts have no CJK glyphs; the shipped string tables do
    hh.ctx
        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    hh
}

fn save(h: &mut Harness<'static, CampaignEditor>, out: &Path, name: &str) {
    h.run_steps(5);
    let img = h.render().expect("render");
    let p = out.join(name);
    img.save(&p).expect("save png");
    println!("wrote {} ({}x{})", p.display(), img.width(), img.height());
}

/// A private install in the scratch folder (symlinks to the real one) holding a campaign made by
/// the editor and then damaged on purpose, so the health panel has something to show.
fn broken_pack(real: &Path) -> Pack {
    use kfx_campaign_ui::pack::{AddLevel, LevelSource};
    use kfx_campaign_ui::{create_pack, NewPack};
    use kfx_formats::levelinfo::LevelKind;
    let dir = kfx_formats::testutil::scratch_dir("campaign-ui/shot-inst");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for e in std::fs::read_dir(real).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if ["campgns", "levels", "multiplayer"]
            .iter()
            .any(|g| g.eq_ignore_ascii_case(&n))
        {
            std::fs::create_dir_all(dir.join(&n)).unwrap();
            for c in std::fs::read_dir(e.path()).unwrap().flatten() {
                kfx_formats::testutil::symlink(&c.path(), &dir.join(&n).join(c.file_name()))
                    .unwrap();
            }
        } else {
            kfx_formats::testutil::symlink(&e.path(), &dir.join(&n)).unwrap();
        }
    }
    let mut spec = NewPack::campaign("mycamp", "My new campaign");
    spec.land_from = Some(dir.join("campgns/keeporig.cfg"));
    let mut pack = create_pack(&dir, &spec).unwrap();
    let orig = kfx_formats::campaign::Campaign::load(&dir.join("campgns/keeporig.cfg")).unwrap();
    for (n, kind) in [
        (1u32, LevelKind::Single),
        (2, LevelKind::Single),
        (3, LevelKind::Single),
        (4, LevelKind::Single),
        (100, LevelKind::Bonus),
        (105, LevelKind::Extra),
    ] {
        let mut s = LevelSource::new(&dir.join("campgns/keeporig"), n);
        s.info = orig.map_info(n);
        pack.add_level(&AddLevel::new(s, kind)).unwrap();
    }
    // the damage
    let mut i = pack.level_info(2);
    i.ensign_pos = Some((1300, 100));
    pack.set_level_info(2, &i);
    let mut i = pack.level_info(3);
    i.land_view = Some(("rgmap77".into(), "viframe03".into()));
    pack.set_level_info(3, &i);
    let mut i = pack.level_info(1);
    i.speech = Some(("intro_good.mp3".into(), "intro_bad.mp3".into()));
    pack.set_level_info(1, &i);
    let mut i = pack.level_info(4);
    i.mapsize = Some((60, 60));
    pack.set_level_info(4, &i);
    pack.update_common(|c| c.creatures_location = Some("campgns/mycamp_crtr".into()));
    pack
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("broken") {
        let out = out_dir();
        std::fs::create_dir_all(&out).unwrap();
        let mut ed = CampaignEditor::new(broken_pack(&install()));
        ed.selected = Some(2);
        ed.add_open = true;
        let mut h = harness(ed, 1280.0, 860.0);
        for (tab, name) in [
            (Tab::Levels, "levels-add"),
            (Tab::LandView, "landview"),
            (Tab::Health, "health"),
            (Tab::Overview, "overview"),
        ] {
            h.state_mut().tab = tab;
            save(&mut h, &out, &format!("broken-{name}.png"));
        }
        return;
    }
    let stem = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "keeporig".to_string());
    let inst = install();
    let out = out_dir();
    std::fs::create_dir_all(&out).unwrap();
    let pack = Pack::load(&inst, &inst.join(format!("campgns/{stem}.cfg"))).expect("campaign");
    let mut ed = CampaignEditor::new(pack);
    let sel: u32 = std::env::var("KFX_SHOT_LEVEL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    ed.selected = Some(sel);
    let mut h = harness(ed, 1280.0, 860.0);
    let tabs: [(Tab, &str); 5] = [
        (Tab::Overview, "overview"),
        (Tab::Levels, "levels"),
        (Tab::LandView, "landview"),
        (Tab::Texts, "texts"),
        (Tab::Health, "health"),
    ];
    for (tab, name) in tabs {
        h.state_mut().tab = tab;
        if tab == Tab::Levels {
            h.state_mut().kind_tab = KindTab::All;
        }
        save(&mut h, &out, &format!("{stem}-{name}.png"));
    }
}
