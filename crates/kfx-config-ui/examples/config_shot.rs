//! Headless renders of the config editors, to look at the widgets without the editor
//! (egui_kittest + wgpu offscreen; nothing is shown on the desktop).
//!
//!   KFX_INSTALL=<install> cargo run -p kfx-config-ui --example config_shot -- [FILTER...]
//!
//! Writes PNGs to `$KFX_SHOT_DIR`, default `config-ui/shots` of the test scratch folder
//! ([`kfx_formats::testutil::scratch_dir`]: `$KFX_TEST_TMP`, else `target/tmp`). Arguments
//! filter the shots by name part (`09 11`). Reads the KeeperFX install named by `KFX_INSTALL`;
//! every write goes to an in-memory file system.
use egui::vec2;
use egui_kittest::Harness;
use kfx_config_ui::assets::Assets;
use kfx_config_ui::ui::{form::FormState, Tab};
use kfx_config_ui::{ConfigEditor, ConfigFs, ConfigSession, LayerStack, MemFs, StdFs};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The KeeperFX install named by `KFX_INSTALL`; exits with a message when it is not set.
fn install() -> PathBuf {
    kfx_formats::testutil::install_dir().unwrap_or_else(|| {
        eprintln!("set KFX_INSTALL to the folder of a KeeperFX install");
        std::process::exit(2)
    })
}

/// The editors' live-preview scratch folder.
fn preview_dir() -> PathBuf {
    kfx_formats::testutil::scratch_dir("config-ui/shot-preview")
}

fn editor(inst: &Path) -> ConfigEditor {
    let fs = MemFs::over(StdFs);
    // a campaign layer with one override, so badges show three layers
    let camp = PathBuf::from("/virtual/campaign");
    fs.write(
        &camp.join("rules.cfg"),
        b"; campaign tweaks\n[game]\nPayDayGap = 7000\n",
    )
    .unwrap();
    let camp_cr = PathBuf::from("/virtual/campaign/creatrs");
    fs.write(&camp_cr.join("imp.cfg"), b"[attributes]\nPay = 60\n")
        .unwrap();
    let levels = PathBuf::from("/virtual/levels");
    let layers = LayerStack::for_level(
        &fs,
        inst,
        Some(&camp),
        Some(&camp_cr),
        Some(&levels),
        Some(12),
    );
    let session = ConfigSession::new(Arc::new(fs), layers);
    let assets = Assets::load_install(inst, None, Some(12), &[], &[]).ok();
    ConfigEditor::new(session, assets, preview_dir())
}

fn harness(ed: ConfigEditor, w: f32, h: f32) -> Harness<'static, ConfigEditor> {
    Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(w, h))
        .with_pixels_per_point(1.0)
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(
            |ui, ed: &mut ConfigEditor| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
                let _ = ed.show(ui);
            },
            ed,
        )
}

fn save(h: &mut Harness<'static, ConfigEditor>, out: &Path, name: &str) {
    h.run_steps(6);
    let img = h.render().expect("render");
    let p = out.join(name);
    img.save(&p).expect("save png");
    println!("wrote {} ({}x{})", p.display(), img.width(), img.height());
}

fn main() {
    let out = std::env::var_os("KFX_SHOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| kfx_formats::testutil::scratch_dir("config-ui/shots"));
    std::fs::create_dir_all(&out).unwrap();
    let inst = install();
    let only: Vec<String> = std::env::args().skip(1).collect();
    let want = |n: &str| only.is_empty() || only.iter().any(|o| n.contains(o.as_str()));

    if want("01") {
        // generic editor on rules.cfg: a map override, a campaign value, base values
        let mut ed = editor(&inst);
        ed.session
            .set_cfg("rules.cfg", "game", "GoldPerHoard", "2600");
        ed.session
            .set_cfg("rules.cfg", "game", "MapCreatureLimit", "999");
        ed.tab = Tab::Generic;
        ed.generic = FormState::new("rules.cfg");
        ed.generic.section = Some("game".into());
        let mut h = harness(ed, 1400.0, 900.0);
        save(&mut h, &out, "01-generic-rules.png");
    }
    if want("02") {
        let mut ed = editor(&inst);
        ed.session
            .set_cfg("rules.cfg", "game", "GoldPerHoard", "2600");
        ed.session.set_cfg("rules.cfg", "game", "PayDayGap", "6000");
        ed.tab = Tab::Generic;
        ed.generic = FormState::new("rules.cfg");
        ed.generic.raw = true;
        let mut h = harness(ed, 1400.0, 700.0);
        save(&mut h, &out, "02-raw-diff.png");
    }
    if want("03") {
        let mut ed = editor(&inst);
        ed.tab = Tab::Creatures;
        ed.creature.name = "DRAGON".into();
        ed.creature.page = kfx_config_ui::ui::creature::Page::Sprites;
        ed.creature.anim = Some("Ambulate".into());
        ed.creature.quarter = 1;
        let mut h = harness(ed, 1400.0, 900.0);
        save(&mut h, &out, "03-creature-dragon-sprites.png");
        h.state_mut().creature.page = kfx_config_ui::ui::creature::Page::Growth;
        save(&mut h, &out, "03b-creature-dragon-growth.png");
        h.state_mut().creature.page = kfx_config_ui::ui::creature::Page::Stats;
        save(&mut h, &out, "03c-creature-dragon-stats.png");
    }
    if want("04") {
        let mut ed = editor(&inst);
        ed.tab = Tab::Slabset;
        ed.slabset.kind = 18;
        ed.slabset.style = 8;
        ed.slabset.neighbour = 10;
        ed.slabset.subtile = 4;
        // an edit so the override marker and live preview show it
        let mut st = ed.session.slabset().style(18, 8);
        st.columns[4] = 1;
        ed.session.slabset().set_style(18, 8, st);
        let mut h = harness(ed, 1500.0, 1100.0);
        save(&mut h, &out, "04-slabset-library.png");
    }
    if want("05") {
        let mut ed = editor(&inst);
        ed.tab = Tab::Columns;
        ed.columns.id = 2;
        let mut h = harness(ed, 1300.0, 900.0);
        save(&mut h, &out, "05-column-editor.png");
    }
    if want("06") {
        let mut ed = editor(&inst);
        ed.tab = Tab::Cubes;
        ed.cubes.id = 99;
        let mut h = harness(ed, 1300.0, 900.0);
        save(&mut h, &out, "06-cube-editor.png");
        h.state_mut().cubes.picking = Some(4);
        save(&mut h, &out, "06b-cube-face-picker.png");
    }
    if want("07") {
        let mut ed = editor(&inst);
        ed.tab = Tab::Tileset;
        ed.tileset.tile = 61;
        ed.tileset.anim_slot = Some(546);
        let mut h = harness(ed, 1300.0, 900.0);
        save(&mut h, &out, "07-tileset-sheet.png");
        // the campaign holds a texture file: imports of tmapa go there
        let campfile = PathBuf::from("/virtual/campaign/tmapb000.dat");
        let b = h
            .state()
            .assets
            .as_ref()
            .and_then(|a| a.tiles.as_ref())
            .map(|t| t.set.atlas_indexed(kfx_gfx::Part::B).to_vec())
            .unwrap_or_default();
        h.state_mut().session.fs.write(&campfile, &b).unwrap();
        h.state_mut().tileset.set = Some(1);
        h.state_mut().tileset.part = kfx_gfx::Part::B;
        save(&mut h, &out, "07b-tileset-set1-campaign-rule.png");
    }
    if want("08") {
        let mut ed = editor(&inst);
        ed.tab = Tab::Sprites;
        ed.sprites.page = 1;
        let mut h = harness(ed, 1300.0, 900.0);
        save(&mut h, &out, "08-sprites-builtin.png");
        h.state_mut().sprites.view = kfx_config_ui::ui::sprites::View::Custom;
        h.state_mut().sprites.page = 0;
        save(&mut h, &out, "08b-sprites-custom.png");
    }
    if want("09") {
        let mut ed = editor(&inst);
        // edits: the last research item and a recipe with an unknown victim, so the changed
        // rows and a warning show
        ed.session.set_cfg(
            "rules.cfg",
            "research",
            "Research",
            "MAGIC POWER_ARMAGEDDON 99999",
        );
        ed.session.set_cfg(
            "rules.cfg",
            "sacrifices",
            "POSSPELLALL",
            "SPELL_SLOW VAMPIRE NOBODY",
        );
        ed.tab = Tab::Rules;
        ed.rules.section = "research".into();
        let mut h = harness(ed, 1300.0, 900.0);
        save(&mut h, &out, "09-rules-research.png");
        h.state_mut().rules.section = "sacrifices".into();
        save(&mut h, &out, "09b-rules-sacrifices.png");
    }
    if want("11") {
        // stats comparison: map overrides (Dragon health, Wizard research), a campaign one (Imp
        // pay)
        let mut ed = editor(&inst);
        ed.session
            .set_cfg("creatrs/dragon.cfg", "attributes", "Health", "1200");
        ed.session
            .set_cfg("creatrs/wizard.cfg", "jobs", "ResearchValue", "9");
        ed.stats_open = true;
        ed.stats.level = 4;
        let mut h = Harness::builder()
            .with_context_setup(kfx_fonts::install)
            .with_size(vec2(1300.0, 760.0))
            .with_pixels_per_point(1.0)
            .with_accessibility_check(false)
            .wgpu()
            .build_ui_state(
                |ui, ed: &mut ConfigEditor| {
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
                    let _ = ed.creature_stats_window(ui.ctx());
                },
                ed,
            );
        save(&mut h, &out, "11-creature-stats.png");
        h.state_mut()
            .stats
            .click_header(kfx_config_ui::ui::stats::SortBy::Stat(0));
        save(&mut h, &out, "11b-creature-stats-by-health.png");
        h.state_mut().stats.filter = "imp".into();
        h.state_mut().stats.show_unlisted = true;
        save(&mut h, &out, "11c-creature-stats-filter-imp.png");
        // 1280x720 at UI scale 1.5, with a host style that shrinks sliders
        let mut ed = editor(&inst);
        ed.session
            .set_cfg("creatrs/dragon.cfg", "attributes", "Health", "1200");
        ed.stats_open = true;
        let mut h = Harness::builder()
            .with_context_setup(kfx_fonts::install)
            .with_size(vec2(1280.0 / 1.5, 720.0 / 1.5))
            .with_pixels_per_point(1.5)
            .with_accessibility_check(false)
            .wgpu()
            .build_ui_state(
                |ui, ed: &mut ConfigEditor| {
                    ui.ctx().global_style_mut(|s| s.spacing.slider_width = 10.0);
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
                    let _ = ed.creature_stats_window(ui.ctx());
                },
                ed,
            );
        save(&mut h, &out, "11d-creature-stats-1280x720-scale1.5.png");
    }
    if want("10") {
        let mut ed = editor(&inst);
        ed.tab = Tab::Objects;
        ed.objects.section = Some("object5".into());
        let mut h = harness(ed, 1300.0, 900.0);
        save(&mut h, &out, "10-objects.png");
        h.state_mut().tab = Tab::Files;
        save(&mut h, &out, "10b-files.png");
    }
    // the editors' live-preview scratch folder
    let _ = std::fs::remove_dir_all(preview_dir());
}
