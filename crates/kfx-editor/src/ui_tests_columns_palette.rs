//! Interaction tests of Map > Columns of this map (the `.clm` editor),
//! the land view / ensign picker of Level properties, workshop boxes and spellbooks in the thing
//! palette, and modded things from the map's own config layer. Same harness as `ui_tests`.
use crate::ui_tests::{level_copy, Ui};
use eframe::egui::{self, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use kfx_formats::testutil;
use kfx_map::column::ColumnKey;

/// Grid cells holding a column with content `k`.
fn cells_with(u: &mut Ui, k: ColumnKey) -> usize {
    let d = u.app().doc.as_ref().unwrap();
    let n = d.dims().stl_grid();
    d.doc
        .columns
        .iter()
        .take(n)
        .filter(|c| ColumnKey::from(*c) == k)
        .count()
}

/// Replace the text of the columns window's filter field.
fn set_filter(u: &mut Ui, text: &str) {
    let f = u.h.get_by_label("Filter").rect();
    u.click_at(f.right_center() + egui::vec2(80.0, 0.0));
    u.key(Modifiers::COMMAND, Key::A);
    u.text(text);
    u.steps(2);
}

/// Columns window: open from the Map menu, sort and filter, copy the cubes of one column and
/// paste them into another, apply (one undo step changing every use), undo, jump to a use.
#[test]
fn ui_columns_window() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("columns") else {
        return;
    };
    let dir = level_copy(&inst, 1, "columns");
    u.open(&dir.join("map00001.slb"));
    u.menu("Map", "Columns of this map…");
    let st = u.app().columns_window_state().expect("the window opens");
    let total = st.rows.len();
    assert!(total > 10, "{total} distinct columns");
    assert_eq!(
        st.edit.map(|e| e.index),
        Some(1),
        "the first real column is selected"
    );
    // pool size and limit line
    assert!(
        u.h.query_all_by_label_contains("columns in the saved .clm")
            .count()
            > 0
    );
    assert!(
        u.h.query_all_by_label_contains("clear unused").count() > 0,
        "the window says why there is no clear unused"
    );
    u.shot("opened");

    // sort by uses, descending: the most used column comes first
    u.click_first("by uses");
    let gd = u.app().doc.as_ref().unwrap().gd.clone();
    let st = u.app().columns_window_state().unwrap();
    let vis = st.visible(&gd);
    let max = st.rows.iter().map(|r| r.uses).max().unwrap();
    assert_eq!(st.rows[vis[0]].uses, max, "sorted by uses, most used first");
    u.click_first("by index");

    // filter by index
    set_filter(&mut u, "#3");
    let st = u.app().columns_window_state().unwrap();
    assert_eq!(st.visible(&gd).len(), 1, "#3 filters one row");
    let src = st.rows.iter().find(|r| r.index == 3).copied().unwrap();
    u.click("#3");
    assert_eq!(
        u.app()
            .columns_window_state()
            .unwrap()
            .edit
            .map(|e| e.index),
        Some(3)
    );
    u.click("Copy cubes");
    assert_eq!(
        u.app().columns_window_state().unwrap().clipboard,
        Some((src.col.cubes, src.col.floor_texture))
    );
    // pick another column (#2)
    set_filter(&mut u, "#2");
    u.click("#2");
    u.key(Modifiers::NONE, Key::Escape);
    let target = u
        .app()
        .columns_window_state()
        .unwrap()
        .rows
        .iter()
        .find(|r| r.index == 2)
        .copied()
        .unwrap();
    let old_key = ColumnKey::from(&target.col);
    let uses = cells_with(&mut u, old_key);
    assert_eq!(uses as u32, target.uses);
    u.click("Paste cubes");
    let e = u.app().columns_window_state().unwrap().edit.unwrap();
    assert_eq!(
        (e.cubes, e.floor),
        (src.col.cubes, src.col.floor_texture),
        "pasted into the edit"
    );
    let before_hist = u.history_len();
    let src_uses = cells_with(&mut u, ColumnKey::from(&src.col));
    u.click("Apply to all uses");
    assert_eq!(u.history_len(), before_hist + 1, "one undo step");
    assert_eq!(
        cells_with(&mut u, old_key),
        0,
        "no subtile keeps the old column"
    );
    assert_eq!(
        cells_with(&mut u, ColumnKey::from(&src.col)),
        src_uses + uses,
        "every use now holds the pasted column (merged with #3)"
    );
    let after = u.app().columns_window_state().unwrap().rows.len();
    assert_eq!(after, total - 1, "the two columns merged in the pool");
    u.shot("applied");
    // undo restores every subtile
    u.key(Modifiers::COMMAND, Key::Z);
    assert_eq!(
        cells_with(&mut u, old_key),
        uses,
        "Ctrl+Z restores the column"
    );
    u.steps(2);
    assert_eq!(u.app().columns_window_state().unwrap().rows.len(), total);

    // jump to a use of column #1
    set_filter(&mut u, "#1");
    u.click("#1");
    let t0 = u.app().camera.target;
    u.click("Jump to a use");
    let t1 = u.app().camera.target;
    assert!(t0 != t1, "the camera moved");
    let one = u
        .app()
        .columns_window_state()
        .unwrap()
        .rows
        .iter()
        .find(|r| r.index == 1)
        .copied()
        .unwrap();
    let d = u.app().doc.as_ref().unwrap();
    let w = d.dims().stl_w() + 1;
    let i = (t1.z.floor() as usize) * w + t1.x.floor() as usize;
    assert_eq!(
        ColumnKey::from(&d.doc.columns[i]),
        ColumnKey::from(&one.col),
        "the camera is on a subtile using it"
    );
    u.shot("jumped");
}

/// Land view picker: Level properties > Pick on the land view, choose a picture, click on it to
/// place the ensign, use, apply; the `.lof` gets LAND_VIEW, ENSIGN_POS and ENSIGN_ZOOM.
#[test]
fn ui_land_view_picker() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("landview") else {
        return;
    };
    let dir = level_copy(&inst, 1, "landview");
    u.open(&dir.join("map00001.slb"));
    u.menu("Level", "Level properties…");
    u.click("Pick on the land view…");
    let p = u.app().land_picker().expect("the picker opens");
    assert!(p.views.len() > 5, "pictures listed: {:?}", p.views);
    assert!(p.views.contains(&"rgmap05".to_string()));
    u.click("rgmap05");
    assert_eq!(
        u.app().land_picker().unwrap().window,
        "viframe05",
        "the matching window frame follows"
    );
    u.steps(2);
    // the picture is drawn at half size: (300, 200) on screen is pixel (600, 400)
    let r = u.h.get_by_label("land view picture").rect();
    assert_eq!((r.width(), r.height()), (640.0, 480.0));
    u.click_at(r.min + egui::vec2(300.0, 200.0));
    let p = u.app().land_picker().unwrap();
    assert_eq!(p.pos, (600, 400));
    assert_eq!(p.zoom, (600, 400), "the zoom point follows the ensign");
    // outside the allowed area it is clamped
    u.click_at(r.min + egui::vec2(10.0, 10.0));
    assert_eq!(u.app().land_picker().unwrap().pos, (160, 120));
    u.click_at(r.min + egui::vec2(300.0, 200.0));
    u.shot("picked");
    u.click("Use these");
    assert!(u.app().land_picker().is_none(), "the picker closes");
    let info = u.app().props_info().unwrap().clone();
    assert_eq!(info.ensign_pos, Some((600, 400)));
    assert_eq!(info.ensign_zoom, Some((600, 400)));
    assert_eq!(info.land_view, Some(("rgmap05".into(), "viframe05".into())));
    u.shot("properties");
    u.click("Apply");
    let lof = std::fs::read_to_string(dir.join("map00001.lof")).expect("the .lof is written");
    let norm = lof.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(norm.contains("ENSIGN_POS = 600 400"), "{lof}");
    assert!(norm.contains("ENSIGN_ZOOM = 600 400"), "{lof}");
    assert!(norm.contains("LAND_VIEW = rgmap05 viframe05"), "{lof}");
}

/// Show a palette category filtered by `search` (nothing chosen).
fn show_in_palette(u: &mut Ui, category: &str, search: &str) {
    u.click(category);
    let lens = u.rect_of("🔍");
    u.click_at(lens.right_center() + egui::vec2(60.0, 0.0));
    u.key(Modifiers::COMMAND, Key::A);
    u.text(search);
    u.steps(2);
    u.key(Modifiers::NONE, Key::Escape);
}

/// Workshop boxes show their trap / door icon; spellbooks name their power (magic.cfg).
#[test]
fn ui_workshop_boxes() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("workshop-boxes") else {
        return;
    };
    let dir = level_copy(&inst, 1, "workshop-boxes");
    u.open(&dir.join("map00001.slb"));
    let (tip, size) = u
        .app()
        .thing_palette_info("WRKBOX_BOULDER")
        .expect("boulder box in the palette");
    assert!(
        tip.contains("workshop box of trap") && tip.contains("BOULDER"),
        "{tip}"
    );
    assert!(
        size[0] >= 48 && size[0] == size[1],
        "the icon canvas: {size:?}"
    );
    let (tip, _) = u
        .app()
        .thing_palette_info("WRKBOX_WOOD")
        .expect("wooden door box");
    assert!(tip.contains("workshop box of door"), "{tip}");
    let (tip, _) = u
        .app()
        .thing_palette_info("SPELLBOOK_HOE")
        .expect("hand of evil book");
    assert!(tip.contains("spellbook of POWER_HAND"), "{tip}");
    show_in_palette(&mut u, "📦 Objects", "WRKBOX");
    assert!(
        u.h.query_by_label("WRKBOX_BOULDER").is_some(),
        "the box tile is shown"
    );
    u.shot("boxes");
}

/// Things defined in the map's own config layer (`mapNNNNN.objects.cfg`, `.effects.toml`,
/// `.magic.cfg`) reach the palette and the inspector.
#[test]
fn ui_map_layer_things() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("modded") else {
        return;
    };
    let dir = level_copy(&inst, 1, "modded");
    let base = kfx_map::gamedata::GameData::load(&crate::document::base_layers(&inst)).unwrap();
    let obj = base.object_names().iter().map(|(i, _)| *i).max().unwrap() + 1;
    let eff = base
        .effect_generator_names()
        .iter()
        .map(|(i, _)| *i)
        .max()
        .unwrap()
        + 1;
    let hoe = base.object_id("SPELLBOOK_HOE").unwrap();
    let anim = base
        .objects_cfg
        .get(&format!("object{hoe}"), "AnimationID")
        .unwrap_or("0")
        .to_string();
    std::fs::write(
        dir.join("map00001.objects.cfg"),
        format!(
            "[object{obj}]\nName = SPELLBOOK_MODDED\nGenre = SPELLBOOK\nAnimationID = {anim}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("map00001.effects.toml"),
        format!("[effectGenerator{eff}]\nName = \"EFFECTGENERATOR_MODDED\"\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("map00001.magic.cfg"),
        "[power1]\nArtifact = SPELLBOOK_MODDED\n",
    )
    .unwrap();
    u.open(&dir.join("map00001.slb"));
    let (tip, size) = u
        .app()
        .thing_palette_info("SPELLBOOK_MODDED")
        .expect("map-layer object in the palette");
    assert!(tip.contains(&format!("object {obj}")), "{tip}");
    assert!(
        tip.contains("spellbook of POWER_HAND"),
        "map-layer magic.cfg links the book: {tip}"
    );
    assert!(size[0] > 0, "it has a sprite");
    let (tip, _) = u
        .app()
        .thing_palette_info("MODDED")
        .expect("map-layer effect generator in the palette");
    assert!(tip.contains(&format!("effect generator {eff}")), "{tip}");
    // place it and read its name back the way the inspector shows it
    u.app()
        .select_thing_palette(&format!("effect:{eff}"))
        .unwrap();
    let at = u.app().find_floor_subtile(40, 40).expect("a free floor");
    let placed = u.app().place_thing_at(at, (128, 128)).unwrap();
    let it = *placed.iter().next().unwrap();
    let label = u.app().item_label(it);
    assert!(label.contains("EFFECTGENERATOR_MODDED"), "{label}");
    show_in_palette(&mut u, "✨ Effects", "MODDED");
    assert!(
        u.h.query_by_label("MODDED").is_some(),
        "the effect tile is shown"
    );
    u.shot("modded-effect");
}
