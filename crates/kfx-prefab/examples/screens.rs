//! Offscreen screenshots (the crate's own top-down renderer, no game graphics): the library
//! browser with the starter set and a contact sheet of every prefab.
//!
//!   KFX_INSTALL=<install> cargo run -p kfx-prefab --example screens -- [STARTER_DIR] [OUT_DIR]
//!
//! `STARTER_DIR` defaults to the workspace's `data/prefabs`, `OUT_DIR` to `prefab/shots` of the
//! test scratch folder ([`kfx_formats::testutil::scratch_dir`]: `$KFX_TEST_TMP`, else
//! `target/tmp`). The game data comes from the KeeperFX install named by `KFX_INSTALL`.
use egui::{vec2, Color32, RichText};
use egui_kittest::Harness;
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_prefab::library::{Library, Origin};
use kfx_prefab::render;
use kfx_prefab::ui::{BrowserContext, PrefabBrowser};
use kfx_prefab::{OwnerMap, Selection};

fn save(h: &mut Harness<'_, impl Sized>, path: &std::path::Path) {
    let img = h.render().expect("render");
    img.save(path).expect("save png");
    println!(
        "wrote {} ({}x{})",
        path.display(),
        img.width(),
        img.height()
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args
        .next()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/prefabs")
        });
    let out = args
        .next()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| kfx_formats::testutil::scratch_dir("prefab/shots"));
    std::fs::create_dir_all(&out).unwrap();
    let Some(install) = kfx_formats::testutil::install_dir() else {
        eprintln!("set KFX_INSTALL to the folder of a KeeperFX install");
        std::process::exit(2)
    };
    let gd = GameData::load(&ConfigLayers::for_level(&install, None, None, None, None)).unwrap();

    let mut lib = Library::new();
    lib.add_root(Origin::Starter, &dir, true);
    lib.scan();
    println!(
        "{} prefabs, {} problems",
        lib.entries.len(),
        lib.problems.len()
    );

    // ---- contact sheet
    let mut entries: Vec<_> = lib.entries.iter().map(|e| e.prefab.clone()).collect();
    entries.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));
    let cols = 6usize;
    let cell = 230.0f32;
    let rows = entries.len().div_ceil(cols);
    let size = vec2(
        cols as f32 * (cell + 8.0) + 24.0,
        rows as f32 * (cell + 34.0) + 24.0,
    );
    let owners = OwnerMap::default();
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(size)
        .wgpu()
        .with_accessibility_check(false)
        .build_ui_state(
            move |ui, texs: &mut Vec<Option<egui::TextureHandle>>| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, Color32::from_gray(34));
                ui.add_space(8.0);
                let mut n = 0;
                for chunk in entries.chunks(cols) {
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        for p in chunk {
                            if texs.len() <= n {
                                let px = ((cell - 20.0) as u32 / p.w.max(p.h)).clamp(4, 40);
                                let img = render::render(p, px, &owners);
                                let ci = egui::ColorImage::from_rgba_unmultiplied(
                                    [img.w as usize, img.h as usize],
                                    &img.rgba,
                                );
                                texs.push(Some(ui.ctx().load_texture(
                                    format!("c-{}", p.meta.name),
                                    ci,
                                    egui::TextureOptions::NEAREST,
                                )));
                            }
                            let tex = texs[n].clone().unwrap();
                            n += 1;
                            ui.allocate_ui_with_layout(
                                vec2(cell, cell + 30.0),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    let (rect, _) = ui.allocate_exact_size(
                                        vec2(cell - 10.0, cell - 10.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(rect, 3.0, Color32::from_gray(20));
                                    let ts = tex.size_vec2();
                                    let r = egui::Rect::from_center_size(rect.center(), ts);
                                    ui.painter().image(
                                        tex.id(),
                                        r,
                                        egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        Color32::WHITE,
                                    );
                                    ui.label(
                                        RichText::new(format!("{} ({}x{})", p.meta.name, p.w, p.h))
                                            .small()
                                            .color(Color32::from_gray(220)),
                                    );
                                },
                            );
                        }
                    });
                }
            },
            Vec::new(),
        );
    h.run_steps(3);
    save(&mut h, &out.join("contact-sheet.png"));

    // ---- browser
    let mut doc = kfx_map::mapdoc::MapDoc::new(
        std::path::Path::new("/nonexistent"),
        1,
        kfx_formats::level_bin::MapDims::new(40, 30),
        0,
        &gd,
    );
    for y in 1..29usize {
        for x in 1..39usize {
            doc.place_slab(&gd, x, y, gd.slab_id("DIRT").unwrap() as u16, 5)
                .unwrap();
        }
    }
    // a prefab that needs a creature the map does not have
    let mut odd = lib
        .entries
        .iter()
        .find(|e| e.prefab.meta.name.contains("Treasure vault"))
        .unwrap()
        .prefab
        .clone();
    odd.meta.name = "Vault of the Fenrir (needs a mod)".into();
    odd.meta.tags = vec!["mod".into(), "vault".into()];
    odd.things.push(kfx_prefab::model::PrefabThing::new(
        kfx_prefab::model::ThingClass::Creature,
        "FENRIR_WOLF",
        kfx_prefab::model::Owner::Neutral,
        3 * 768 + 384,
        6 * 768 + 384,
    ));
    lib.entries.push(kfx_prefab::library::Entry {
        origin: Origin::User,
        path: None,
        prefab: odd,
        read_only: false,
    });
    let mut browser = PrefabBrowser::new(lib);
    browser.select_by_name("Ambush corridor");
    let gd2 = gd;
    let doc2 = doc;
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1280.0, 860.0))
        .wgpu()
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, b: &mut PrefabBrowser| {
                let ctx = BrowserContext {
                    gd: Some(&gd2),
                    doc: Some(&doc2),
                    selection: Some(Selection::Rect(kfx_map::autotile::SlabRect {
                        x0: 2,
                        y0: 2,
                        x1: 8,
                        y1: 6,
                    })),
                };
                b.show(ui, &ctx);
            },
            browser,
        );
    h.run_steps(4);
    save(&mut h, &out.join("browser-ambush.png"));
    h.state_mut().select_by_name("Vault of the Fenrir");
    h.state_mut().orientation.rotation = kfx_prefab::Rotation::R90;
    h.run_steps(4);
    save(&mut h, &out.join("browser-missing-names.png"));
    h.state_mut().select_by_name("Hero fortress");
    h.state_mut().orientation = kfx_prefab::Orientation::default();
    h.state_mut().query = "hero".into();
    h.run_steps(4);
    save(&mut h, &out.join("browser-search-hero.png"));
}
