//! egui_kittest tests of the creature stats window and the rules research / sacrifices lists,
//! over a small in-memory install (no game data needed).
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use kfx_config_ui::cfgedit::Pending;
use kfx_config_ui::creature::{ExpRates, Growth, STATS};
use kfx_config_ui::ui::stats::{SortBy, StatsAction};
use kfx_config_ui::ui::Tab;
use kfx_config_ui::{
    ConfigEditor, ConfigFs, ConfigSession, CreatureStats, Layer, LayerStack, MemFs,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CREATURE_CFG: &str = "[common]\nCreatures = WIZARD DRAGON IMP\n\n[experience]\n\
    HealthIncreaseOnExp = 35\nPayIncreaseOnExp = 35\n";
const RULES_CFG: &str = "[game]\nGoldPerHoard = 2000\n\n[research]\nResearch = MAGIC \
    POWER_HAND 10\nResearch = ROOM TREASURE 20\nResearch = MAGIC POWER_SLAP 30\n\n\
    [sacrifices]\nMKCREATURE = DRAGON FLY SPIDER\n";

fn fs() -> MemFs {
    let fs = MemFs::new();
    let w = |p: &str, t: &str| fs.write(Path::new(p), t.as_bytes()).unwrap();
    w("/inst/fxdata/creature.cfg", CREATURE_CFG);
    w("/inst/fxdata/rules.cfg", RULES_CFG);
    w(
        "/inst/fxdata/magic.cfg",
        "[power1]\nName = POWER_HAND\n[power2]\nName = POWER_SLAP\n[spell1]\nName = SPELL_SPEED\n",
    );
    w(
        "/inst/fxdata/terrain.cfg",
        "[room1]\nName = TREASURE\n[room2]\nName = LAIR\n",
    );
    w(
        "/inst/creatrs/wizard.cfg",
        "[attributes]\nHealth = 350\nStrength = 20\nPay = 200\nLuck \
            = 10\n[jobs]\nResearchValue = 5\n",
    );
    w(
        "/inst/creatrs/dragon.cfg",
        "[attributes]\nHealth = 900\nStrength = 90\nPay = 350\nLuck \
            = 18\n[jobs]\nResearchValue = 4\n",
    );
    w(
        "/inst/creatrs/imp.cfg",
        "[attributes]\nHealth = 75\nStrength = 5\nPay = 0\nLuck = 0\n",
    );
    // a model file creature.cfg does not list
    w("/inst/creatrs/ghost.cfg", "[attributes]\nHealth = 10\n");
    // the map layer overrides the dragon's health
    w("/lv/map00001.dragon.cfg", "[attributes]\nHealth = 1000\n");
    fs
}

fn session(fs: &MemFs) -> ConfigSession {
    let layers = LayerStack {
        layers: vec![
            Layer::base("/inst/fxdata", Some(PathBuf::from("/inst/creatrs"))),
            Layer::map("/lv", 1),
        ],
    };
    ConfigSession::new(Arc::new(fs.clone()), layers)
}

struct Stats {
    s: ConfigSession,
    w: CreatureStats,
    actions: Vec<StatsAction>,
}

fn stats_harness() -> Harness<'static, Stats> {
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1500.0, 500.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, st: &mut Stats| {
                if let Some(a) = st.w.show(ui, &mut st.s) {
                    st.actions.push(a);
                }
            },
            Stats {
                s: session(&fs()),
                w: CreatureStats::default(),
                actions: vec![],
            },
        );
    h.run_steps(3);
    h
}

fn names(h: &mut Harness<'static, Stats>) -> Vec<String> {
    let st = h.state_mut();
    st.w.rows(&mut st.s).into_iter().map(|r| r.name).collect()
}

fn health(h: &mut Harness<'static, Stats>, name: &str) -> (Option<i64>, Option<usize>) {
    let st = h.state_mut();
    let r =
        st.w.rows(&mut st.s)
            .into_iter()
            .find(|r| r.name == name)
            .unwrap();
    (r.cells[0].value, r.cells[0].layer)
}

#[test]
fn stats_sort_by_header_click() {
    let mut h = stats_harness();
    assert_eq!(STATS[0].key, "Health");
    assert_eq!(
        names(&mut h),
        ["WIZARD", "DRAGON", "IMP"],
        "load order first"
    );
    h.get_by_label("Health").click();
    h.run_steps(3);
    assert_eq!(h.state().w.sort, SortBy::Stat(0));
    assert_eq!(
        names(&mut h),
        ["DRAGON", "WIZARD", "IMP"],
        "numbers sort biggest first"
    );
    h.get_by_label("Health v").click();
    h.run_steps(3);
    assert_eq!(names(&mut h), ["IMP", "WIZARD", "DRAGON"]);
    h.get_by_label("Creature").click();
    h.run_steps(3);
    assert_eq!(names(&mut h), ["DRAGON", "IMP", "WIZARD"]);
    // filter
    h.state_mut().w.filter = "ar".into();
    assert_eq!(names(&mut h), ["WIZARD"]);
}

#[test]
fn stats_level_slider_changes_values() {
    let mut h = stats_harness();
    assert_eq!(health(&mut h, "DRAGON").0, Some(1000));
    assert!(h.query_all_by_label("1000").next().is_some());
    // click the middle of the slider
    h.get_by_role(Role::Slider).click();
    h.run_steps(3);
    let level = h.state().w.level;
    assert!(level > 1, "the slider moved ({level})");
    let expected = Growth::Health.at_level(1000, level as i64 - 1, &ExpRates::default());
    assert_eq!(expected, 1000 + 35 * 1000 * (level as i64 - 1) / 100);
    assert_eq!(health(&mut h, "DRAGON").0, Some(expected));
    assert!(
        h.query_all_by_label(&expected.to_string()).next().is_some(),
        "the table shows the level {level} value"
    );
    // luck does not grow, pay is capped at 16 bits
    let st = h.state_mut();
    st.w.level = 10;
    let rows = st.w.rows(&mut st.s);
    let d = rows.iter().find(|r| r.name == "DRAGON").unwrap();
    let luck = STATS.iter().position(|s| s.key == "Luck").unwrap();
    assert_eq!(d.cells[luck].value, Some(18));
    let pay = STATS.iter().position(|s| s.key == "Pay").unwrap();
    assert_eq!(d.cells[pay].value, Some(350 + 35 * 350 * 9 / 100));
    assert_eq!(Growth::Pay.at_level(100000, 9, &ExpRates::default()), 32767);
}

#[test]
fn stats_override_layer_and_actions() {
    let mut h = stats_harness();
    assert_eq!(
        health(&mut h, "DRAGON"),
        (Some(1000), Some(1)),
        "map layer value"
    );
    assert_eq!(health(&mut h, "WIZARD"), (Some(350), Some(0)), "base value");
    // an unsaved edit at the target layer shows at once
    h.state_mut()
        .s
        .set_cfg("creatrs/wizard.cfg", "attributes", "Health", "400");
    h.run_steps(2);
    assert_eq!(health(&mut h, "WIZARD"), (Some(400), Some(1)));
    assert!(h.query_all_by_label("400").next().is_some());
    // unlisted creature files
    assert!(!names(&mut h).contains(&"GHOST".to_string()));
    h.get_by_label("Unlisted").click();
    h.run_steps(3);
    assert!(names(&mut h).contains(&"GHOST".to_string()));
    // a click on a name asks to edit it
    h.get_by_label("IMP").click();
    h.run_steps(2);
    assert_eq!(h.state().actions, [StatsAction::Edit("IMP".into())]);
}

#[test]
fn stats_window_edit_switches_editor() {
    let fs = fs();
    let mut ed = ConfigEditor::new(
        session(&fs),
        None,
        std::env::temp_dir().join(format!("kfx-config-ui-uitest-{}", std::process::id())),
    );
    ed.stats_open = true;
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1400.0, 700.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, ed: &mut ConfigEditor| {
                let f = ed.creature_stats_window(ui.ctx());
                assert!(f.events.is_empty());
                if f.show_editor {
                    ed.time = -1.0;
                }
            },
            ed,
        );
    h.run_steps(3);
    h.get_by_label("DRAGON").click();
    h.run_steps(2);
    assert_eq!(h.state().tab, Tab::Creatures);
    assert_eq!(h.state().creature.name, "DRAGON");
    assert_eq!(h.state().time, -1.0, "show_editor was set");
}

fn rules_harness(section: &str) -> (Harness<'static, ConfigEditor>, MemFs) {
    let fs = fs();
    let mut ed = ConfigEditor::new(
        session(&fs),
        None,
        std::env::temp_dir().join(format!("kfx-config-ui-uitest-{}", std::process::id())),
    );
    ed.tab = Tab::Rules;
    ed.rules.section = section.into();
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1400.0, 800.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, ed: &mut ConfigEditor| {
                let _ = ed.show(ui);
            },
            ed,
        );
    h.run_steps(3);
    (h, fs)
}

fn pending(h: &mut Harness<'static, ConfigEditor>) -> String {
    match h.state_mut().session.cfg("rules.cfg").unwrap().pending() {
        Pending::Write(b) => String::from_utf8(b).unwrap(),
        Pending::Delete => "<delete>".into(),
        Pending::Unchanged => String::new(),
    }
}

#[test]
fn rules_research_reorder_insert_remove() {
    let (mut h, fs) = rules_harness("research");
    h.get_all_by_label("Down").next().unwrap().click();
    h.run_steps(3);
    assert_eq!(
        pending(&mut h),
        "[research]\nResearch = ROOM TREASURE 20\nResearch = MAGIC \
            POWER_HAND 10\nResearch = MAGIC POWER_SLAP 30\n"
    );
    // moving it back gives the inherited list: nothing to write
    h.get_all_by_label("Up").nth(1).unwrap().click();
    h.run_steps(3);
    assert_eq!(pending(&mut h), "");
    // insert a copy of the last item, then remove the first
    h.get_all_by_label("+").nth(2).unwrap().click();
    h.run_steps(3);
    h.get_all_by_label("x").next().unwrap().click();
    h.run_steps(3);
    let want = "[research]\nResearch = ROOM TREASURE 20\nResearch = MAGIC \
        POWER_SLAP 30\nResearch = MAGIC POWER_SLAP 30\n";
    assert_eq!(pending(&mut h), want);
    h.state_mut().session.save().unwrap();
    assert_eq!(
        fs.read(Path::new("/lv/map00001.rules.cfg"))
            .unwrap()
            .unwrap(),
        want.as_bytes()
    );
    // research points through the session's whole-block edit
    h.state_mut()
        .session
        .set_cfg("rules.cfg", "research", "Research", "MAGIC POWER_SLAP 45");
    h.run_steps(2);
    assert!(pending(&mut h).ends_with("Research = MAGIC POWER_SLAP 45\n"));
    // revert the whole list
    h.get_by_label("Revert list").click();
    h.run_steps(3);
    assert_eq!(pending(&mut h), "<delete>");
}

#[test]
fn rules_sacrifices_edit() {
    let (mut h, _fs) = rules_harness("sacrifices");
    h.get_by_label("+ victim").click();
    h.run_steps(3);
    assert_eq!(
        pending(&mut h),
        "[sacrifices]\nMKCREATURE = DRAGON FLY SPIDER FLY\n"
    );
    h.get_by_label("Add recipe").click();
    h.run_steps(3);
    assert_eq!(
        pending(&mut h),
        "[sacrifices]\nMKCREATURE = DRAGON FLY SPIDER FLY\nMKCREATURE = DRAGON FLY\n"
    );
    // the second victim of the first recipe goes
    h.get_all_by_label("-").nth(1).unwrap().click();
    h.run_steps(3);
    assert_eq!(
        pending(&mut h),
        "[sacrifices]\nMKCREATURE = DRAGON FLY FLY\nMKCREATURE = DRAGON FLY\n"
    );
    h.get_all_by_label("Down").next().unwrap().click();
    h.run_steps(3);
    assert_eq!(
        pending(&mut h),
        "[sacrifices]\nMKCREATURE = DRAGON FLY\nMKCREATURE = DRAGON FLY FLY\n"
    );
    // flagged: unknown victim names (no FLY/SPIDER creature in this install)
    assert!(h
        .query_all_by_label_contains("unknown creature")
        .next()
        .is_some());
}

fn tileset_harness() -> (Harness<'static, ConfigEditor>, MemFs) {
    use kfx_config_ui::assets::Assets;
    use kfx_config_ui::raster::TileCache;
    use kfx_gfx::{CustomSet, Palette, TextureAnim, TextureSet};
    let fs = fs();
    let w = |p: &str, b: u8| fs.write(Path::new(p), &[b; 2048]).unwrap();
    w("/inst/data/tmapa000.dat", 1);
    w("/inst/data/tmapa001.dat", 2);
    // the campaign already has texture files (of another part), the map has none
    w("/camp/tmapb000.dat", 3);
    let layers = LayerStack {
        layers: vec![
            Layer::base("/inst/fxdata", Some(PathBuf::from("/inst/creatrs"))),
            Layer::campaign(Some(PathBuf::from("/camp")), None),
            Layer::map("/lv", 1),
        ],
    };
    let palette = Palette::from_bytes(&[0u8; 768]).unwrap();
    // the host loaded something else: the editor reloads what the layers give
    let tiles = TileCache::new(
        palette.clone(),
        TextureSet::from_bytes(0, &[9; 1024], None).unwrap(),
        TextureAnim::default(),
    );
    let assets = Assets {
        palette,
        tiles: Some(tiles),
        keeper: None,
        gui2: None,
        gui1: None,
        custom: CustomSet::new(),
    };
    let mut ed = ConfigEditor::new(
        ConfigSession::new(Arc::new(fs.clone()), layers),
        Some(assets),
        std::env::temp_dir().join(format!("kfx-config-ui-uitest-{}", std::process::id())),
    );
    ed.tab = Tab::Tileset;
    ed.tileset.path = "/x/new.dat".into();
    let mut h = Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(1600.0, 900.0))
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, ed: &mut ConfigEditor| {
                let _ = ed.show(ui);
            },
            ed,
        );
    h.run_steps(3);
    (h, fs)
}

fn shown(h: &Harness<'static, ConfigEditor>) -> (u32, u8) {
    let t = h.state().assets.as_ref().unwrap().tiles.as_ref().unwrap();
    (t.set.index, t.set.atlas_indexed(kfx_gfx::Part::A)[0])
}

#[test]
fn tileset_effective_view_save_rule_and_picker() {
    let (mut h, fs) = tileset_harness();
    assert_eq!(shown(&h), (0, 1), "reloaded from the layers");
    // joined part by part like the editor does, so the separators match on Windows too
    let base_a = Path::new("/inst")
        .join("data")
        .join("tmapa000.dat")
        .display()
        .to_string();
    assert!(
        h.query_all_by_label(&base_a).next().is_some(),
        "source shown"
    );
    // the campaign has its own texture files: the import goes there
    fs.write(Path::new("/x/new.dat"), &[5; 2048]).unwrap();
    h.get_by_label("Import tmapa").click();
    h.run_steps(3);
    assert!(fs.read(Path::new("/camp/tmapa000.dat")).unwrap().is_some());
    assert_eq!(shown(&h), (0, 5));
    // map-local instead
    h.get_by_label("Map-local").click();
    h.run_steps(2);
    fs.write(Path::new("/x/new.dat"), &[6; 2048]).unwrap();
    h.get_by_label("Import tmapa").click();
    h.run_steps(3);
    assert!(fs
        .read(Path::new("/lv/map00001.tmapa000.dat"))
        .unwrap()
        .is_some());
    assert_eq!(shown(&h), (0, 6));
    // revert the map file: the campaign's applies again
    h.get_by_label("Revert tmapa").click();
    h.run_steps(3);
    assert_eq!(
        fs.read(Path::new("/lv/map00001.tmapa000.dat")).unwrap(),
        None
    );
    assert_eq!(shown(&h), (0, 5));
    // another set from the picker's list
    h.state_mut().tileset.set = Some(1);
    h.run_steps(3);
    assert_eq!(shown(&h), (1, 2));
}

#[test]
fn tileset_upper_layer_override_is_shown_and_flagged() {
    let (mut h, fs) = tileset_harness();
    // target the campaign while the map overrides tmapa
    fs.write(Path::new("/lv/map00001.tmapa000.dat"), &[7; 2048])
        .unwrap();
    h.state_mut().session.set_target(1).unwrap();
    h.run_steps(3);
    assert_eq!(shown(&h), (0, 7), "the game's view: the map file");
    assert!(h
        .query_all_by_label_contains("overrides this file")
        .next()
        .is_some());
}
