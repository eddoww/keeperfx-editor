//! Interaction tests through the real UI: the whole `EditorApp` in an `egui_kittest` harness
//! (wgpu renderer, offscreen, the same device renders the 3D view), driven by pointer and
//! keyboard events only (widgets found through their accessibility labels, the map through the
//! camera projection), with the document checked after every interaction and a screenshot per
//! scenario in the test scratch folder `kfx-editor/ui-shots/` (or `KFX_SHOTS_DIR`).
//!
//! Needs the test install (skipped without it). Settings and backups go to a private folder
//! (`KFX_EDITOR_CONFIG_DIR`); maps are edited in private copies in the test scratch folder
//! (`kfx_formats::testutil::scratch_dir`).
use crate::app::{EditorApp, EframeApp, Gpu};
use crate::settings::Settings;
use eframe::egui::{self, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use glam::{Vec2, Vec3};
use kfx_formats::testutil;
use std::path::{Path, PathBuf};

/// Where the scenario screenshots go (`KFX_SHOTS_DIR` overrides).
pub fn shots_dir() -> PathBuf {
    std::env::var_os("KFX_SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| testutil::scratch_dir("kfx-editor/ui-shots"))
}

fn config_dir() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let d = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/uitest-config-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&d);
        // the tests never write the user's settings or backups
        std::env::set_var("KFX_EDITOR_CONFIG_DIR", &d);
    });
}

/// A temporary folder removed when dropped.
pub struct Tmp(pub PathBuf);

impl std::ops::Deref for Tmp {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A private copy of a shipped campaign level (all its `mapNNNNN.*` files) in a fresh folder.
pub fn level_copy(install: &Path, level: u32, tag: &str) -> Tmp {
    let src = testutil::resolve_ci(install, "campgns/keeporig").expect("keeporig");
    let dir = kfx_formats::testutil::scratch_dir(&format!(
        "kfx-editor/uitest-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for f in crate::document::level_files(&src, level) {
        let name = f
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        std::fs::copy(&f, dir.join(name)).unwrap();
    }
    Tmp(dir)
}

/// Whether egui_kittest can create its wgpu renderer here (probed once per process, the same
/// way the harness does). Without one, the UI tests skip with a message instead of panicking.
pub fn gpu_available() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        let ok = std::panic::catch_unwind(egui_kittest::wgpu::WgpuTestRenderer::new).is_ok();
        if !ok {
            eprintln!("no wgpu adapter: the egui_kittest UI tests that render are skipped");
        }
        ok
    })
}

pub struct Ui {
    pub h: Harness<'static, EframeApp>,
    pub name: String,
    shots: usize,
}

impl Ui {
    /// The editor at 1500x950 with the test install, no map open. `None` without an install or
    /// without a GPU adapter (CI runners may have none).
    pub fn new(name: &str) -> Option<Ui> {
        let install = testutil::install_dir()?;
        if !gpu_available() {
            return None;
        }
        Some(Ui::with_settings(
            name,
            Settings {
                install: Some(install),
                ..Default::default()
            },
            egui::vec2(1500.0, 950.0),
        ))
    }

    /// [`Ui::new`] with persistence on (settings, autosave snapshots, config backups), all in
    /// the private config folder.
    pub fn persistent(name: &str) -> Option<Ui> {
        let install = testutil::install_dir()?;
        if !gpu_available() {
            return None;
        }
        Some(Ui::build(
            name,
            Settings {
                install: Some(install),
                ..Default::default()
            },
            egui::vec2(1500.0, 950.0),
            true,
        ))
    }

    /// The editor with these settings at `size` (no install check).
    pub fn with_settings(name: &str, settings: Settings, size: egui::Vec2) -> Ui {
        Ui::build(name, settings, size, false)
    }

    fn build(name: &str, settings: Settings, size: egui::Vec2, persist: bool) -> Ui {
        config_dir();
        let h = Harness::builder()
            .with_size(size)
            .with_pixels_per_point(1.0)
            .with_max_steps(400)
            .wgpu()
            .build_eframe(move |cc| {
                let rs = cc.wgpu_render_state.clone().expect("wgpu render state");
                let gpu = Gpu {
                    device: rs.device.clone(),
                    queue: rs.queue.clone(),
                    egui_renderer: rs.renderer.clone(),
                };
                let mut app = EditorApp::new(&cc.egui_ctx, gpu, settings.clone(), persist, None);
                app.fixed_frame = Some(0);
                EframeApp(app)
            });
        let mut u = Ui {
            h,
            name: name.to_string(),
            shots: 0,
        };
        u.steps(2);
        u
    }

    pub fn app(&mut self) -> &mut EditorApp {
        &mut self.h.state_mut().0
    }

    pub fn steps(&mut self, n: usize) {
        for _ in 0..n {
            self.h.step();
        }
    }

    /// Open a map directly (the browser scenario opens one by clicking).
    pub fn open(&mut self, slb: &Path) {
        self.app().open_map(slb).unwrap();
        self.steps(3);
    }

    pub fn shot(&mut self, what: &str) -> PathBuf {
        self.steps(1);
        let img = self.h.render().expect("render");
        let dir = shots_dir();
        let _ = std::fs::create_dir_all(&dir);
        self.shots += 1;
        let p = dir.join(format!("{}_{:02}_{what}.png", self.name, self.shots));
        // a full disk must not fail the interaction test itself
        if let Err(e) = img.save(&p) {
            eprintln!("screenshot {} not written: {e}", p.display());
        }
        p
    }

    // ---------------------------------------------------------------- pointer / keys

    fn ev(&mut self, e: egui::Event) {
        self.h.event(e);
        self.h.step();
    }
    pub fn move_to(&mut self, p: Pos2) {
        self.ev(egui::Event::PointerMoved(p));
    }
    pub fn button(&mut self, p: Pos2, b: PointerButton, pressed: bool, m: Modifiers) {
        self.ev(egui::Event::PointerButton {
            pos: p,
            button: b,
            pressed,
            modifiers: m,
        });
    }
    pub fn click_at(&mut self, p: Pos2) {
        self.move_to(p);
        self.button(p, PointerButton::Primary, true, Modifiers::NONE);
        self.button(p, PointerButton::Primary, false, Modifiers::NONE);
        self.steps(1);
    }
    /// Press at `a`, move in `n` steps to `b`, release there.
    pub fn drag(&mut self, a: Pos2, b: Pos2, n: usize, m: Modifiers) {
        self.move_to(a);
        self.button(a, PointerButton::Primary, true, m);
        for k in 1..=n {
            let t = k as f32 / n as f32;
            self.move_to(a + (b - a) * t);
        }
        self.button(b, PointerButton::Primary, false, m);
        self.steps(1);
    }
    pub fn key(&mut self, m: Modifiers, k: Key) {
        self.h.key_press_modifiers(m, k);
        self.steps(1);
    }
    pub fn text(&mut self, t: &str) {
        self.ev(egui::Event::Text(t.to_string()));
    }

    /// Click the widget with this accessibility label (exactly one must match).
    pub fn click(&mut self, label: &str) {
        let r = self.h.get_by_label(label).rect();
        self.click_at(r.center());
    }
    /// Click the first widget with this label (several may match, e.g. one per pack).
    pub fn click_first(&mut self, label: &str) {
        let r = self
            .h
            .query_all_by_label(label)
            .next()
            .or_else(|| self.h.query_all_by_label_contains(label).next())
            .map(|n| n.rect());
        let Some(r) = r else {
            self.shot("missing-widget");
            panic!("no widget {label:?}");
        };
        self.click_at(r.center());
    }
    pub fn has(&self, label: &str) -> bool {
        self.h.query_by_label(label).is_some()
    }
    pub fn rect_of(&self, label: &str) -> egui::Rect {
        self.h.get_by_label(label).rect()
    }
    /// Open a top menu and click one of its items.
    pub fn menu(&mut self, top: &str, item: &str) {
        self.click_top(top);
        self.steps(1);
        self.click_first(item);
        self.steps(1);
    }
    /// Click the highest widget with this label (the menu bar over a toolbar group title).
    pub fn click_top(&mut self, label: &str) {
        let r = self
            .h
            .get_all_by_label(label)
            .map(|n| n.rect())
            .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
            .unwrap_or_else(|| panic!("no widget {label:?}"));
        self.click_at(r.center());
    }
    /// Choose a palette tile like a user: search for it, then click it.
    pub fn pick_palette(&mut self, category: &str, name: &str) {
        self.click(category);
        let lens = self.rect_of("🔍");
        self.click_at(lens.right_center() + egui::vec2(60.0, 0.0));
        self.key(Modifiers::COMMAND, Key::A);
        self.text(name);
        self.steps(2);
        self.click(name);
        // leave the search field so that map keys work again
        self.key(Modifiers::NONE, Key::Escape);
    }

    // ---------------------------------------------------------------- map positions

    /// Screen position of a world point in the main pane.
    pub fn world_px(&mut self, w: Vec3) -> Pos2 {
        let app = self.app();
        let r = app
            .pane_rects()
            .first()
            .copied()
            .expect("viewport laid out");
        let vp = Vec2::new(r.width(), r.height());
        let p = app.camera.project(w, vp).expect("in front of the camera");
        r.min + egui::vec2(p.x, p.y)
    }
    /// Centre of slab (x, y) on its top.
    pub fn slab_px(&mut self, x: i32, y: i32) -> Pos2 {
        let h = self.app().slab_top(x, y);
        self.world_px(Vec3::new(x as f32 * 3.0 + 1.5, h, y as f32 * 3.0 + 1.5))
    }
    /// A subtile centre on the floor.
    pub fn stl_px(&mut self, x: i32, y: i32) -> Pos2 {
        let h = self
            .app()
            .doc
            .as_ref()
            .map_or(0, |d| d.doc.floor_height(x, y)) as f32;
        self.world_px(Vec3::new(x as f32 + 0.5, h, y as f32 + 0.5))
    }
    /// Zoom the main camera onto a slab so that one slab is about `px` pixels.
    pub fn look_at(&mut self, x: i32, y: i32, zoom: f32) {
        let app = self.app();
        app.camera.target = Vec3::new(x as f32 * 3.0 + 1.5, 0.0, y as f32 * 3.0 + 1.5);
        app.camera.zoom = zoom;
        self.steps(2);
    }

    pub fn kind_at(&mut self, x: i32, y: i32) -> u16 {
        self.app()
            .doc
            .as_ref()
            .unwrap()
            .state_at(x, y)
            .unwrap()
            .kind
    }
    pub fn history_len(&mut self) -> usize {
        self.app().doc.as_ref().map_or(0, |d| d.history().len())
    }
}

fn install() -> Option<PathBuf> {
    testutil::install_dir()
}

// ------------------------------------------------------------------------------- scenarios

/// Map browser: open it from the toolbar, type a filter, pick a level, Edit.
#[test]
fn ui_open_map_via_browser() {
    let Some(mut u) = Ui::new("browser") else {
        return;
    };
    u.click("🗺 Maps");
    assert!(
        u.app().browser_open(),
        "the toolbar button opens the browser"
    );
    // the filter field is right of its label
    let f = u.rect_of("Filter:");
    u.click_at(f.right_center() + egui::vec2(60.0, 0.0));
    u.text("Flowerhat");
    u.steps(2);
    u.shot("filtered");
    u.click_first("map00004");
    u.steps(2);
    u.click("✏ Edit");
    u.steps(3);
    let d = u.app().doc.as_ref().expect("a map is open");
    assert_eq!(d.doc.level, 4);
    assert!(!u.app().browser_open(), "Edit closes the browser");
    u.shot("opened");
}

/// Palette tile by click, paint stroke by pointer drag, Ctrl+Z.
#[test]
fn ui_palette_paint_undo() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("paint") else {
        return;
    };
    let dir = level_copy(&inst, 1, "paint");
    u.open(&dir.join("map00001.slb"));
    u.look_at(40, 40, 1.0);
    u.pick_palette("🗻 Terrain", "PATH");
    assert_eq!(
        u.app().brush_kind(),
        u.app()
            .doc
            .as_ref()
            .unwrap()
            .gd
            .slab_id("PATH")
            .map(|k| k as u16)
    );
    let path = u.app().doc.as_ref().unwrap().gd.slab_id("PATH").unwrap() as u16;
    let (a, b) = (u.slab_px(38, 40), u.slab_px(42, 40));
    let before: Vec<u16> = (38..=42).map(|x| u.kind_at(x, 40)).collect();
    u.drag(a, b, 8, Modifiers::NONE);
    for x in 38..=42 {
        assert_eq!(u.kind_at(x, 40), path, "slab ({x},40) painted");
    }
    assert_eq!(u.history_len(), 1, "one stroke = one undo step");
    u.shot("painted");
    u.key(Modifiers::COMMAND, Key::Z);
    let after: Vec<u16> = (38..=42).map(|x| u.kind_at(x, 40)).collect();
    assert_eq!(after, before, "Ctrl+Z reverts the stroke");
    // redo with Ctrl+Y
    u.key(Modifiers::COMMAND, Key::Y);
    assert_eq!(u.kind_at(40, 40), path);
}

/// Box tool by toolbar click and a drag; the preview is not applied until release.
#[test]
fn ui_box_tool() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("box") else { return };
    let dir = level_copy(&inst, 1, "box");
    u.open(&dir.join("map00001.slb"));
    u.look_at(30, 30, 0.9);
    u.pick_palette("🗻 Terrain", "WATER");
    u.click("☐ Box");
    let water = u.app().doc.as_ref().unwrap().gd.slab_id("WATER").unwrap() as u16;
    let (a, b) = (u.slab_px(28, 28), u.slab_px(31, 30));
    u.move_to(a);
    // the slab under the pointer (a taller slab in front may hide the one aimed at)
    let s0 = u
        .app()
        .hover
        .as_ref()
        .map(|h| (h.slab.0 as i32, h.slab.1 as i32))
        .expect("hover");
    u.button(a, PointerButton::Primary, true, Modifiers::NONE);
    u.move_to(b);
    let s1 = u
        .app()
        .hover
        .as_ref()
        .map(|h| (h.slab.0 as i32, h.slab.1 as i32))
        .expect("hover");
    assert_ne!(
        u.kind_at(s1.0, s1.1),
        water,
        "nothing changes while dragging"
    );
    u.shot("box-preview");
    u.button(b, PointerButton::Primary, false, Modifiers::NONE);
    u.steps(1);
    for y in s0.1.min(s1.1)..=s0.1.max(s1.1) {
        for x in s0.0.min(s1.0)..=s0.0.max(s1.0) {
            assert_eq!(u.kind_at(x, y), water, "({x},{y}) in the box {s0:?}-{s1:?}");
        }
    }
    assert_eq!(u.history_len(), 1);
    u.shot("box-done");
}

/// Place a creature from the palette, select it with the Things tool, drag it, edit a field in
/// the inspector by typing.
#[test]
fn ui_place_select_drag_inspect() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("things") else {
        return;
    };
    let dir = level_copy(&inst, 1, "things");
    u.open(&dir.join("map00001.slb"));
    // a big open floor area of map 1 near the heart
    u.look_at(40, 40, 1.4);
    u.pick_palette("👹 Creatures", "TROLL");
    let n0 = u.app().doc.as_ref().unwrap().doc.things.items.len();
    // the dungeon heart room of map00001 is around slab (40..45, 40..45); use a floor subtile
    let (sx, sy) = u
        .app()
        .find_floor_subtile(40, 40)
        .expect("floor near (40,40)");
    u.look_at(sx / 3, sy / 3, 1.4);
    let p = u.stl_px(sx, sy);
    u.move_to(p);
    // the subtile actually under the pointer
    let (sx, sy) = u
        .app()
        .hover
        .as_ref()
        .map(|h| (h.hit.stl_x as i32, h.hit.stl_y as i32))
        .expect("hover");
    u.click_at(p);
    let tn = u.app().tool_name();
    eprintln!("place: status {:?} tool {tn:?}", u.app().status);
    u.shot("place-click");
    let d = u.app().doc.as_ref().unwrap();
    assert_eq!(d.doc.things.items.len(), n0 + 1, "a click places one thing");
    let i = d.doc.things.items.len() - 1;
    let t = d.doc.things.items[i].clone();
    assert_eq!(t.x.unwrap().stl, sx);
    u.shot("placed");
    // Things tool: click selects, drag moves
    u.click("👆 Things");
    u.app().selected.clear();
    let p = u.stl_px(sx, sy);
    u.click_at(p - egui::vec2(0.0, 8.0));
    assert!(
        u.app()
            .selected
            .contains(&kfx_map::things::ItemRef::Thing(i)),
        "click on the sprite selects it: {:?}",
        u.app().selected
    );
    let (tx, ty) = (sx + 3, sy + 3);
    let a = u.stl_px(sx, sy) - egui::vec2(0.0, 8.0);
    let b = u.stl_px(tx, ty) - egui::vec2(0.0, 8.0);
    u.move_to(a);
    let g0 = u
        .app()
        .hover
        .as_ref()
        .map(|h| (h.hit.stl_x as i32, h.hit.stl_y as i32))
        .unwrap();
    u.drag(a, b, 10, Modifiers::NONE);
    let g1 = u
        .app()
        .hover
        .as_ref()
        .map(|h| (h.hit.stl_x as i32, h.hit.stl_y as i32))
        .unwrap();
    // (the things list may be reordered by a move: follow the selection)
    let i = match u.app().selected.iter().next() {
        Some(kfx_map::things::ItemRef::Thing(k)) => *k,
        x => panic!("the moved thing is not selected: {x:?}"),
    };
    let t2 = u.app().doc.as_ref().unwrap().doc.things.items[i].clone();
    let moved = (t2.x.unwrap().stl, t2.y.unwrap().stl);
    // the thing follows the pointer on the plane of its floor (whole subtiles); the ground
    // under the pointer may differ where walls are in front
    assert_eq!(
        moved,
        (tx, ty),
        "dragged to {moved:?} (pointer ground {g0:?} -> {g1:?})"
    );
    u.shot("dragged");
    // inspector: type a gold value
    u.click("Inspector");
    u.steps(1);
    let field = u.h.get_by_label("creature_gold").rect();
    let r =
        u.h.get_all_by_role(egui::Role::TextInput)
            .map(|n| n.rect())
            .find(|r| (r.center().y - field.center().y).abs() < 6.0)
            .expect("gold field");
    u.click_at(r.center());
    u.key(Modifiers::COMMAND, Key::A);
    u.text("777");
    u.key(Modifiers::NONE, Key::Enter);
    u.steps(2);
    let t3 = u.app().doc.as_ref().unwrap().doc.things.items[i].clone();
    assert_eq!(t3.creature_gold, Some(777), "typed into the inspector");
    // typing in the field must not have moved the camera or run map shortcuts (A = pan left,
    // Ctrl+A = select all slabs)
    assert!(
        u.app().selection_rect().is_none(),
        "Ctrl+A in a text field selected the map"
    );
    u.shot("inspector");
}

/// The script panel: F3 opens it, typing with completion.
#[test]
fn ui_script_typing_completion() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("script") else {
        return;
    };
    let dir = level_copy(&inst, 4, "script");
    u.open(&dir.join("map00004.slb"));
    u.key(Modifiers::NONE, Key::F3);
    u.steps(2);
    assert!(u.app().script_panel_open(), "F3 opens the script panel");
    // click into the editor text, go to the end, type a command start and accept completion
    let r = u.app().script_text_rect().expect("script editor laid out");
    u.click_at(r.center());
    u.key(Modifiers::COMMAND, Key::End);
    u.key(Modifiers::NONE, Key::Enter);
    let hist = u.history_len();
    for c in "DISPLAY_OBJ".chars() {
        u.text(&c.to_string());
    }
    u.steps(2);
    u.shot("completion");
    u.key(Modifiers::NONE, Key::Tab);
    u.steps(1);
    let text = u.app().script_active_text();
    assert!(
        text.contains("DISPLAY_OBJECTIVE"),
        "completion accepted: ...{}",
        &text[text.len().saturating_sub(80)..]
    );
    // typed letters went to the script, not to the map tools (W/A/S/D pan, Q/E rotate)
    assert_eq!(u.history_len(), hist, "typing changed the map");
    u.text("(");
    u.text("1");
    u.steps(2);
    u.shot("typed");
}

/// Every menu opens; every enabled item that does not leave the editor (file dialogs, the
/// game, the file manager, exit) runs without a panic. Dialogs are closed after each.
#[test]
fn ui_menus() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("menus") else {
        return;
    };
    let dir = level_copy(&inst, 2, "menus");
    u.open(&dir.join("map00002.slb"));
    let menus: &[(&str, &[&str])] = &[
        (
            "File",
            &[
                "Map browser…",
                "Level files…",
                "Backups…",
                "Export image…",
                "Image to map…",
            ],
        ),
        (
            "Edit",
            &[
                "Select all",
                "Copy",
                "Fill selection with palette slab",
                "Undo",
            ],
        ),
        (
            "View",
            &[
                "2D view (top-down)",
                "Split view (2D + 3D)",
                "3D view",
                "Script panel",
                "Reset camera (game view)",
                "Rotate left",
                "Rotate right",
                "Maximise scripts over the map",
            ],
        ),
        (
            "Level",
            &[
                "Map browser…",
                "Level properties…",
                "Scripts",
                "Validate",
                "Game log",
            ],
        ),
        (
            "Map",
            &[
                "Resize…",
                "Symmetry…",
                "Decoration of this map…",
                "📝 Add note…",
                "🔓 Unlock selection",
            ],
        ),
        (
            "Tools",
            &["Regenerate all slabs…", "Paint", "Box", "Things"],
        ),
        (
            "Configuration",
            &[
                "Configuration editor…",
                "Rules…",
                "Slab sets…",
                "Layers & mods…",
                "Strings…",
                "Preferences…",
                "Keyboard shortcuts…",
                "KeeperFX folder…",
            ],
        ),
        (
            "Window",
            &[
                "Right panel",
                "Layers & mods",
                "Strings",
                "Level files",
                "Notes",
            ],
        ),
        ("Help", &["Controls", "About"]),
    ];
    let mut done = 0;
    for (top, items) in menus {
        u.click_top(top);
        u.steps(1);
        if u.h.query_all_by_label_contains(items[0]).count() == 0 {
            u.shot(&format!("menu-{top}-missing"));
            panic!("menu {top} does not open with one click");
        }
        if done == 0 {
            u.shot("file-menu");
        }
        u.key(Modifiers::NONE, Key::Escape);
        for item in *items {
            u.click_top(top);
            u.steps(1);
            let target =
                u.h.query_all_by_label_contains(item)
                    .next()
                    .filter(|n| !n.accesskit_node().is_disabled())
                    .map(|n| n.rect());
            match target {
                Some(r) => {
                    u.click_at(r.center());
                    u.steps(2);
                    done += 1;
                }
                None => u.key(Modifiers::NONE, Key::Escape),
            }
            // close what opened
            let app = u.app();
            app.open_named_dialog("none").unwrap();
            app.close_first_run();
            u.key(Modifiers::NONE, Key::Escape);
            u.steps(1);
        }
    }
    assert!(done >= 30, "{done} menu items run");
    assert!(u.app().doc.is_some());
    u.shot("after-menus");
}

/// Keyboard shortcuts, rebinding Undo to Ctrl+U through the shortcuts window.
#[test]
fn ui_shortcuts_rebind() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("shortcuts") else {
        return;
    };
    let dir = level_copy(&inst, 1, "shortcuts");
    u.open(&dir.join("map00001.slb"));
    // brush size with the bracket keys
    let b0 = u.app().brush_size();
    u.key(Modifiers::NONE, Key::CloseBracket);
    assert_eq!(u.app().brush_size(), b0 + 1);
    // a stroke to undo
    u.look_at(40, 40, 1.0);
    u.pick_palette("🗻 Terrain", "PATH");
    let (a, b) = (u.slab_px(39, 41), u.slab_px(41, 41));
    u.drag(a, b, 4, Modifiers::NONE);
    assert_eq!(u.history_len(), 1);
    u.menu("Configuration", "Keyboard shortcuts…");
    u.steps(2);
    u.shot("shortcuts-window");
    // the button showing Undo's current key
    let row = u.rect_of("Undo");
    let btn =
        u.h.get_all_by_label("Ctrl+Z")
            .map(|n| n.rect())
            .find(|r| (r.center().y - row.center().y).abs() < 8.0)
            .expect("Ctrl+Z button on the Undo row");
    u.click_at(btn.center());
    assert!(u.app().capturing_shortcut(), "waiting for the new key");
    u.key(Modifiers::COMMAND, Key::U);
    assert!(!u.app().capturing_shortcut());
    assert_eq!(u.app().keymap.label("edit.undo"), "Ctrl+U");
    u.app().open_named_dialog("none").unwrap();
    u.steps(1);
    u.key(Modifiers::COMMAND, Key::Z);
    assert_eq!(
        u.app().doc.as_ref().unwrap().cursor(),
        1,
        "Ctrl+Z no longer undoes"
    );
    u.key(Modifiers::COMMAND, Key::U);
    assert_eq!(u.app().doc.as_ref().unwrap().cursor(), 0, "Ctrl+U undoes");
}

/// Dialogs: New map (Cancel, then Create), Level properties (Cancel, Apply), Preferences,
/// Resize (Cancel, Resize).
#[test]
fn ui_dialogs() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("dialogs") else {
        return;
    };
    let dir = level_copy(&inst, 3, "dialogs");
    u.open(&dir.join("map00003.slb"));
    // resize: cancel, then grow by 2 on the east
    u.menu("Map", "Resize…");
    assert!(u.has("Resize map"));
    u.shot("resize");
    u.click("Cancel");
    assert!(!u.has("Resize map"));
    assert_eq!(u.app().doc.as_ref().unwrap().dims().w, 85);
    u.menu("Map", "Resize…");
    let east = u.rect_of("East");
    let field =
        u.h.get_all_by_role(egui::Role::SpinButton)
            .map(|n| n.rect())
            .find(|r| (r.center().y - east.center().y).abs() < 8.0)
            .expect("East value");
    // a DragValue takes typed text after a click
    u.click_at(field.center());
    u.key(Modifiers::COMMAND, Key::A);
    u.text("2");
    u.key(Modifiers::NONE, Key::Enter);
    u.shot("resize-typed");
    u.click("Resize");
    u.steps(2);
    eprintln!("resize: {}", u.app().status);
    assert_eq!(
        u.app().doc.as_ref().unwrap().dims().w,
        87,
        "resized through the dialog"
    );
    // properties: cancel, then apply a name
    u.menu("Level", "Level properties…");
    assert!(u.has("Level properties"));
    u.shot("properties");
    u.click("Cancel");
    assert!(!u.has("Level properties"));
    // preferences window opens and closes
    u.menu("Configuration", "Preferences…");
    assert!(u.has("Preferences"));
    u.shot("preferences");
    u.app().open_named_dialog("none").unwrap();
    u.steps(1);
    // new map: cancel keeps the map; create opens a new unsaved one
    u.app().discard_changes_for_test();
    u.menu("File", "New map…");
    assert!(u.has("Create"));
    u.shot("new-map");
    let c = u.rect_of("Cancel");
    eprintln!("cancel at {c:?}");
    u.click("Cancel");
    eprintln!(
        "after cancel: dialog {} prompt {}",
        u.app().new_dialog_open(),
        u.app().pending_prompt()
    );
    assert!(
        !u.app().new_dialog_open(),
        "Cancel closes the New map dialog"
    );
    assert_eq!(u.app().doc.as_ref().unwrap().doc.level, 3);
    u.menu("File", "New map…");
    u.click("Create");
    u.steps(3);
    let d = u.app().doc.as_ref().unwrap();
    assert!(d.is_dirty() || !u.app().on_disk(), "a new unsaved map");
    u.shot("new-map-created");
}

/// Save As through the dialog into a new folder, then File > Reload: the same map comes back.
#[test]
fn ui_save_reload_roundtrip() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("save") else { return };
    let dir = level_copy(&inst, 5, "save");
    // a per-map override and a level string travel with Save As
    std::fs::write(
        dir.join("map00005.rules.cfg"),
        "[game]\nGoldPerHoard = 1234\n",
    )
    .unwrap();
    u.open(&dir.join("map00005.slb"));
    u.look_at(42, 42, 1.0);
    u.pick_palette("🗻 Terrain", "LAVA");
    let p = u.slab_px(42, 42);
    u.click_at(p);
    let lava = u.app().doc.as_ref().unwrap().gd.slab_id("LAVA").unwrap() as u16;
    assert_eq!(u.kind_at(42, 42), lava);
    let out = Tmp(kfx_formats::testutil::scratch_dir(&format!(
        "kfx-editor/uitest-saveas-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&*out);
    std::fs::create_dir_all(&*out).unwrap();
    u.key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
    assert!(u.has("Save map as"));
    let lab = u.rect_of("Folder");
    let folder =
        u.h.get_all_by_role(egui::Role::TextInput)
            .map(|n| n.rect())
            .find(|r| (r.center().y - lab.center().y).abs() < 8.0)
            .expect("folder field");
    u.click_at(folder.center());
    u.key(Modifiers::COMMAND, Key::A);
    u.text(&out.display().to_string());
    u.shot("save-as");
    u.click("Save");
    u.steps(2);
    assert!(
        out.join("map00005.slb").is_file(),
        "saved there: {}",
        u.app().status
    );
    assert!(
        out.join("map00005.rules.cfg").is_file(),
        "the per-map override went along"
    );
    assert!(!u.app().doc.as_ref().unwrap().is_dirty());
    let kinds = u.app().doc.as_ref().unwrap().doc.kinds.clone();
    u.menu("File", "Reload");
    u.steps(2);
    let d = u.app().doc.as_ref().unwrap();
    assert_eq!(d.doc.folder, out.0);
    assert_eq!(d.doc.kinds, kinds, "reloaded map equals the saved one");
    u.shot("reloaded");
}

/// Tools must not fire through windows, a drag started on a panel must not paint when it
/// crosses the viewport, and the wheel over a panel must not zoom the map.
#[test]
fn ui_no_click_through() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("through") else {
        return;
    };
    let dir = level_copy(&inst, 1, "through");
    u.open(&dir.join("map00001.slb"));
    u.look_at(40, 40, 1.0);
    u.pick_palette("🗻 Terrain", "PATH");
    // a window over the map: the Layers & mods window
    u.app().open_named_dialog("layers").unwrap();
    u.steps(2);
    let w = u.rect_of("Ignore mods");
    let h0 = u.history_len();
    u.click_at(w.center() + egui::vec2(120.0, 0.0));
    u.drag(
        w.center() + egui::vec2(120.0, 20.0),
        w.center() + egui::vec2(220.0, 30.0),
        4,
        Modifiers::NONE,
    );
    assert_eq!(
        u.history_len(),
        h0,
        "a click / drag in a window painted the map below"
    );
    u.app().open_named_dialog("none").unwrap();
    u.steps(1);
    // drag from the left panel onto the map
    let tree = u.rect_of("Keeper 0 (Red)");
    let target = u.slab_px(40, 40);
    u.drag(tree.center(), target, 8, Modifiers::NONE);
    assert_eq!(
        u.history_len(),
        h0,
        "a drag started on a panel painted when it reached the map"
    );
    // wheel over the palette panel
    let z = u.app().camera.zoom;
    let pal = u.rect_of("🗻 Terrain");
    u.move_to(pal.center());
    u.ev_wheel(pal.center(), -120.0);
    assert!(
        (u.app().camera.zoom - z).abs() < 1e-6,
        "the wheel over a panel zoomed the map"
    );
    // and over the map it does zoom
    u.move_to(target);
    u.ev_wheel(target, 120.0);
    assert!(
        (u.app().camera.zoom - z).abs() > 1e-4,
        "the wheel over the map zooms"
    );
}

impl Ui {
    fn ev_wheel(&mut self, p: Pos2, dy: f32) {
        self.ev(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, dy),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        });
        let _ = p;
        self.steps(3);
    }
}

/// 2D view: F2 switches, the tools work identically (paint by pointer), the split view shares
/// the hover.
#[test]
fn ui_2d_and_split() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("view2d") else {
        return;
    };
    let dir = level_copy(&inst, 1, "view2d");
    u.open(&dir.join("map00001.slb"));
    u.key(Modifiers::NONE, Key::F2);
    assert_eq!(
        u.app().settings.layout.view_mode,
        crate::settings::ViewMode::TwoD
    );
    u.steps(2);
    u.pick_palette("🗻 Terrain", "PATH");
    let path = u.app().doc.as_ref().unwrap().gd.slab_id("PATH").unwrap() as u16;
    // in 2D the pane's camera is the top-down one
    let (a, b) = (u.slab_px_2d(36, 44), u.slab_px_2d(39, 44));
    u.drag(a, b, 6, Modifiers::NONE);
    for x in 36..=39 {
        assert_eq!(u.kind_at(x, 44), path, "2D paint at ({x},44)");
    }
    u.shot("2d");
    u.key(Modifiers::SHIFT, Key::F2);
    assert_eq!(
        u.app().settings.layout.view_mode,
        crate::settings::ViewMode::Split
    );
    u.steps(2);
    let p = u.slab_px_2d(20, 20);
    u.move_to(p);
    u.steps(1);
    assert_eq!(
        u.app().hover.as_ref().map(|h| h.slab),
        Some((20, 20)),
        "hover from the 2D pane"
    );
    u.shot("split");
}

impl Ui {
    /// A slab centre in the 2D pane (the first pane in 2D / split mode).
    fn slab_px_2d(&mut self, x: i32, y: i32) -> Pos2 {
        let app = self.app();
        let r = app.pane_rects().first().copied().unwrap();
        let vp = Vec2::new(r.width(), r.height());
        let h = app.slab_top(x, y);
        let p = app
            .camera2
            .project(Vec3::new(x as f32 * 3.0 + 1.5, h, y as f32 * 3.0 + 1.5), vp)
            .unwrap();
        r.min + egui::vec2(p.x, p.y)
    }
}

/// Every non-ASCII character of the crate's string literals has a glyph in the UI fonts (a
/// missing one shows as a box in the editor and panics the test harness).
#[test]
fn ui_glyphs_exist() {
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    // fonts are built on the first frame
    let mut out = ctx.run_ui(Default::default(), |_| {});
    out.textures_delta.clear();
    let mut missing: Vec<(char, String)> = vec![];
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // every source file, in every folder below src/
    let mut files = vec![];
    let mut dirs = vec![dir];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else {
                files.push(p);
            }
        }
    }
    for f in files
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
    {
        let text = std::fs::read_to_string(&f).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            // characters inside "..." literals of the line
            let mut inside = false;
            let mut prev = ' ';
            for c in line.chars() {
                if c == '"' && prev != '\\' {
                    inside = !inside;
                } else if inside
                    && !c.is_ascii()
                    && !ctx.fonts_mut(|fo| fo.has_glyph(&egui::FontId::proportional(13.0), c))
                    && !missing.iter().any(|m| m.0 == c)
                {
                    missing.push((c, format!("{}:{}", f.display(), n + 1)));
                }
                prev = c;
            }
        }
    }
    assert!(
        missing.is_empty(),
        "characters without a glyph: {missing:?}"
    );
}

/// The script editor shows the text of a message id on hover (the editor's string lookup,
/// `kfx_script_ui::StringLookup`), and "New string…" creates a level string and writes its id.
#[test]
fn ui_script_string_hover_and_new() {
    let Some(inst) = install() else { return };
    let Some(mut u) = Ui::new("strings") else {
        return;
    };
    let dir = level_copy(&inst, 4, "strings");
    u.open(&dir.join("map00004.slb"));
    u.key(Modifiers::NONE, Key::F3);
    u.steps(2);
    let expected = u.app().string_text(45).expect("string 45");
    assert!(!expected.is_empty());
    {
        let ws = u.app().script_ws().unwrap();
        ws.active_editor_mut().goto_line(96);
    }
    u.steps(3);
    let pos = {
        let ws = u.app().script_ws().unwrap();
        let e = ws.active_editor_mut();
        let off = e.test_offset_of(b"DISPLAY_OBJECTIVE(45");
        e.test_char_pos(off + "DISPLAY_OBJECTIVE(4".len())
    }
    .expect("the id is on screen");
    u.move_to(pos + egui::vec2(2.0, 6.0));
    for _ in 0..12 {
        u.move_to(pos + egui::vec2(3.0, 6.0));
    }
    u.steps(4);
    let words: String = expected
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    let shown = u.h.query_all_by_label_contains(&words).count() > 0;
    u.shot("hover");
    assert!(shown, "hover shows {words:?}");
    // "New string…": request from the editor, answered by the editor app
    let tab = u.app().script_ws().unwrap().active_name().to_string();
    {
        // the caret inside the message argument: the new id replaces it
        let e = u.app().script_ws().unwrap().active_editor_mut();
        let off = e.test_offset_of(b"DISPLAY_OBJECTIVE(45") + "DISPLAY_OBJECTIVE(4".len();
        e.select_span(kfx_script::Span::new(off, off));
        e.request_new_string();
    }
    u.steps(3);
    let text = u.app().script_active_text();
    assert!(u.app().strings_dirty(), "a new level string");
    assert!(u.has("Strings"), "the Strings window opens on it");
    let id = u.app().selected_string().expect("selected");
    assert!(
        text.contains(&format!("DISPLAY_OBJECTIVE({id},PLAYER0)")),
        "the new id {id} replaced the argument in {tab}: {:?}",
        text.lines().nth(95)
    );
    u.shot("new-string");
}
