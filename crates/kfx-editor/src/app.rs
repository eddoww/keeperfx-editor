//! The editor application: UI shell, document handling, tools, viewport interaction.
use crate::assets::{self, Assets, CubeFaces, PLAYERS, PLAYER_TREE_ORDER};
use crate::atlas::{self, AtlasData, LAYER_TILES, LAYER_W, TILES_PER_ROW};
use crate::browser::MapBrowser;
use crate::camera::{Camera, MAX_ZOOM, MIN_ZOOM};
use crate::document::Refresh;
use crate::document::{self, Document, Edit};
use crate::mesh;
use crate::minimap;
use crate::pick::{pick, Hit};
use crate::render::{LineVertex, ViewFlags, ViewRenderer};
use crate::settings::{pick_install, validate_install, Settings};
use crate::sprites::{SpriteAtlas, SpriteLib};
use crate::terrain::Terrain;
use crate::theme::{self, GOLD, GOLD_DIM, TEXT_DIM};
use crate::tools::{self, Clipboard, Tool};
use kfx_formats::level_bin;
use kfx_formats::levelinfo::Lof;
use kfx_map::mapdoc::MapDoc;
use kfx_map::things::ItemRef;
use kfx_map::transform::Symmetry;
use kfx_map::validate::Issue;
use kfx_prefab::i18n::{trd, trf, tx, Tx};
use std::collections::BTreeSet;

mod campaign;
mod clm_editor;
mod config;
mod custom;
mod dialogs;
mod documents;
mod editing;
mod extras;
mod fly;
mod keys;
mod landview;
pub mod logic;
mod map_dialogs;
mod menu_bar;
mod palette;
mod pane;
mod panels;
mod play;
pub(crate) mod prefab;
mod prompts;
mod safety;
mod script_panel;
mod scripted;
mod strings;
#[cfg(test)]
mod testsupport;
mod texts;
mod things;
mod toolbar;
mod viewport;
mod workshop;
mod workshop_win;
pub use campaign::{run_actions as run_campaign_actions, CampaignLayout, WorkshopSettings};
mod analysis_panel;
pub mod audio;
mod generator;
mod live;
#[cfg(all(test, unix))]
pub use config::{ui_stack as config_ui_stack, EditorFsForTest};
use eframe::egui::{self, mutex::RwLock, Color32, RichText, Vec2 as EVec2};
use egui_wgpu::wgpu;
use glam::{Vec2, Vec3};
use kfx_formats::level_bin::MapDims;
use kfx_gfx::textures::TextureSearch;
use kfx_gfx::Install;
use kfx_map::autotile::{SlabRect, PLAYER_NEUTRAL};
use kfx_map::gamedata::{category, ConfigLayers, GameData};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
pub use strings::StringTarget;
pub use things::class_name as class_label;
pub use things::{PlaceDefaults, PlaceKind};

/// Narrowest the map view gets when side panels are docked beside it (points).
pub const MIN_MAP_W: f32 = 420.0;

/// The widest a right-hand panel may be so that the map keeps [`MIN_MAP_W`] (panels are laid
/// out one after the other, so this is what is left now).
pub(crate) fn side_panel_max(ui: &egui::Ui) -> f32 {
    (ui.available_width() - MIN_MAP_W).max(200.0)
}

/// Device, queue and the egui renderer (from eframe, or created by the headless mode).
#[derive(Clone)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub egui_renderer: Arc<RwLock<egui_wgpu::Renderer>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Terrain,
    Rooms,
    Doors,
    Creatures,
    Traps,
    Objects,
    ActionPoints,
    Lights,
    Effects,
    /// Custom slabs of the map (sidecar).
    Custom,
}

const CATEGORIES: [(Category, Tx); 10] = [
    (
        Category::Terrain,
        tx("kfx_editor.terrain_962f", "🗻 Terrain"),
    ),
    (Category::Rooms, tx("kfx_editor.rooms", "🏠 Rooms")),
    (
        Category::Creatures,
        tx("kfx_editor.creatures_bd20", "👹 Creatures"),
    ),
    (
        Category::Objects,
        tx("kfx_editor.objects_c265", "📦 Objects"),
    ),
    (Category::Traps, tx("kfx_editor.traps_3293", "⚠ Traps")),
    (Category::Doors, tx("kfx_editor.doors_3efd", "🚪 Doors")),
    (
        Category::Effects,
        tx("kfx_editor.effects_bd87", "✨ Effects"),
    ),
    (Category::Lights, tx("kfx_editor.lights_eb5e", "💡 Lights")),
    (
        Category::ActionPoints,
        tx("kfx_editor.action_points_62bd", "⚑ Action points"),
    ),
    (
        Category::Custom,
        tx("kfx_editor.custom_slabs", "⬛ Custom slabs"),
    ),
];

impl Category {
    pub fn parse(s: &str) -> Option<Category> {
        Some(match s.to_ascii_lowercase().replace(' ', "_").as_str() {
            "terrain" => Category::Terrain,
            "rooms" => Category::Rooms,
            "creatures" => Category::Creatures,
            "objects" => Category::Objects,
            "traps" => Category::Traps,
            "doors" => Category::Doors,
            "effects" => Category::Effects,
            "lights" => Category::Lights,
            "action_points" => Category::ActionPoints,
            "custom" => Category::Custom,
            _ => return None,
        })
    }
}

/// Filters of the things list.
#[derive(Clone, Debug)]
pub struct ListFilter {
    pub kind: String,
    pub owner: Option<u8>,
    pub text: String,
}

/// Palette groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Natural,
    Walls,
    Floors,
    RoomWalls,
    Other,
    Rooms,
}

const TERRAIN_GROUPS: [(Group, Tx); 5] = [
    (
        Group::Natural,
        tx("kfx_editor.rock_earth_and_gold", "Rock, earth and gold"),
    ),
    (
        Group::Walls,
        tx("kfx_editor.fortified_walls", "Fortified walls"),
    ),
    (
        Group::Floors,
        tx("kfx_editor.floors_and_liquids", "Floors and liquids"),
    ),
    (
        Group::RoomWalls,
        tx(
            "kfx_editor.room_walls_normally_drawn_automatically_next_to",
            "Room walls (normally drawn automatically next to rooms)",
        ),
    ),
    (Group::Other, tx("kfx_editor.other", "Other")),
];

struct PaletteTile {
    kind: u16,
    group: Group,
    label: String,
    tooltip: String,
    tex: Option<egui::TextureHandle>,
}

/// What is under the cursor.
#[derive(Clone, Debug)]
pub struct HoverInfo {
    pub hit: Hit,
    pub slab: (usize, usize),
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Perf {
    pub atlas_ms: f32,
    pub mesh_ms: f32,
    pub upload_ms: f32,
    pub chunks: usize,
    pub frame_cpu_ms: f32,
    pub sprites_ms: f32,
    /// CPU time to build the thing billboards of the last frame.
    pub things_ms: f32,
    pub billboards: usize,
}

#[derive(Default)]
struct FirstRun {
    open: bool,
    text: String,
    error: Option<String>,
}

/// Camera given on the command line: target x, y (subtiles), zoom %, yaw, pitch (degrees).
#[derive(Clone, Copy, Debug)]
pub struct CameraSpec {
    pub x: f32,
    pub y: f32,
    pub zoom_pct: f32,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
}

/// An action waiting for the "unsaved changes" answer.
#[derive(Clone, Debug)]
enum Pending {
    Open(PathBuf),
    New,
    Reload,
    Exit,
}

/// A tool interaction in progress (left button held).
#[derive(Clone, Copy, Debug)]
struct Drag {
    tool: Tool,
    start: (i32, i32),
    last: (i32, i32),
    cur: (i32, i32),
}

struct SaveAsDialog {
    folder: String,
    level: u32,
    confirm_overwrite: bool,
    error: Option<String>,
}

pub struct EditorApp {
    ctx: egui::Context,
    gpu: Gpu,
    pub settings: Settings,
    /// Write settings back to disk (off in screenshot mode).
    persist: bool,
    assets: Option<Assets>,
    data_error: Option<String>,
    gd_cache: HashMap<ConfigLayers, Arc<GameData>>,
    pub doc: Option<Document>,
    /// Whether the document has a file on disk to reload from.
    doc_on_disk: bool,
    cubes: CubeFaces,
    terrain: Terrain,
    pub status: String,
    view: ViewRenderer,
    view_tex: Option<egui::TextureId>,
    view_size: (u32, u32),
    pub camera: Camera,
    camera_pending_home: bool,
    camera_spec: Option<CameraSpec>,
    pub hover: Option<HoverInfo>,
    active_owner: u8,
    category: Category,
    brush: Option<u16>,
    palette: Vec<PaletteTile>,
    palette_search: String,
    tool: Tool,
    brush_size: u32,
    brush_round: bool,
    shape_filled: bool,
    owner_only: bool,
    editable_border: bool,
    drag: Option<Drag>,
    /// The drag was started by the real pointer (released when the button goes up).
    pointer_drag: bool,
    selection: Option<((i32, i32), (i32, i32))>,
    clipboard: Option<Clipboard>,
    pasting: bool,
    minimap_tex: Option<egui::TextureHandle>,
    minimap_dirty: bool,
    backed_up: HashSet<(PathBuf, u32)>,
    /// Copies of files kept before their first overwrite in this session (string tables,
    /// campaign files, `.lof` / `.lif`, scripts); shared with the documents and the scripts.
    pub(crate) session_backups: crate::files::SessionBackups,
    /// Files of the opened map that could not be read: ask whether to open read-only or to
    /// discard them.
    unreadable_prompt: Option<Vec<kfx_map::mapdoc::UnreadablePart>>,
    pending: Option<Pending>,
    new_dialog: Option<dialogs::NewMapDialog>,
    save_as_dialog: Option<SaveAsDialog>,
    allow_close: bool,
    browser: MapBrowser,
    first_run: FirstRun,
    show_about: bool,
    show_controls: bool,
    pub perf: Perf,
    start: Instant,
    /// Fixed animation frame (headless); `None` = from the clock.
    pub fixed_frame: Option<u32>,
    pending_open: Option<PathBuf>,
    quit_requested: bool,
    /// Last viewport size in pixels (for the minimap frustum and headless tools).
    last_vp: Vec2,
    /// Viewport of the 2D pane (swapped with `last_vp` with the cameras).
    last_vp2: Vec2,
    /// Widths of the toolbar groups in the last frame (wrapping).
    toolbar_widths: HashMap<&'static str, f32>,
    /// Map > Random doodads.
    doodad_dialog: Option<extras::DoodadDialog>,
    toasts: Vec<extras::Toast>,
    /// The open map was opened from the backups folder: Save is refused, Save as works.
    pub from_backup: bool,
    /// Fortify: placing a keeper's ground walls the earth around it.
    pub fortify: bool,
    /// Play with `-packetload` (Some(true)) or `-packetsave` (Some(false)) for the next launch.
    packet_mode: Option<bool>,
    last_status: String,
    show_changelog: bool,
    /// A newer release found by the optional release check (tag, page).
    release_note: Arc<std::sync::Mutex<Option<(String, String)>>>,
    /// Saved `.dat` column indices for the grid data overlay (recomputed after changes).
    grid_cols: Option<Vec<u16>>,
    /// Fly view: the mouse is captured for mouse-look (the tools act at the crosshair).
    pub fly_capture: bool,
    /// Texture sets loaded into the atlas.
    texture_sets: Vec<String>,
    // things
    sprites: Option<SpriteLib>,
    sprite_atlas: SpriteAtlas,
    /// Custom sprite zips loaded for the document.
    sprite_zips: usize,
    thing_palette: Vec<things::ThingEntry>,
    place: Option<PlaceKind>,
    pub place_defaults: things::PlaceDefaults,
    free_place: bool,
    pub selected: BTreeSet<ItemRef>,
    hitboxes: Vec<things::HitBox>,
    badges: Vec<things::Badge>,
    /// Hit boxes / badges / markers of the other pane of the split view (see `swap_pane`).
    hitboxes2: Vec<things::HitBox>,
    badges2: Vec<things::Badge>,
    markers2: Vec<script_panel::Marker>,
    last_click: Option<(Vec2, usize, Vec<ItemRef>)>,
    thing_drag: Option<things::ThingDrag>,
    rect_sel: Option<things::RectSel>,
    inspector_buf: HashMap<String, String>,
    model_search: String,
    inspector_rev: Option<(u64, BTreeSet<ItemRef>)>,
    list_filter: ListFilter,
    // map tools
    symmetry: Option<Symmetry>,
    /// Slab style paint mode: the `.slx` value painted instead of slabs.
    style_brush: Option<u8>,
    pub issues: Vec<Issue>,
    issues_rev: u64,
    issues_seen_rev: u64,
    issues_changed: Instant,
    props_dialog: Option<panels::PropsDialog>,
    resize_dialog: Option<panels::ResizeDialog>,
    regen_dialog: Option<panels::RegenDialog>,
    show_prefs: bool,
    show_symmetry: bool,
    // scripts
    scripts: Option<script_panel::ScriptState>,
    /// The script workspace is maximised over the map (F4).
    script_max: bool,
    /// Pick-from-map mode requested by a script editor.
    pick: Option<kfx_script_ui::PickKind>,
    /// A map position flashed after a jump from a script.
    flash: Option<(Vec3, Instant)>,
    number_prompt: Option<script_panel::NumberPrompt>,
    markers: Vec<script_panel::Marker>,
    /// Action point and hero gate numbers of the map as last seen.
    numbers: (Vec<i32>, Vec<i32>),
    /// Script references of `issues` (same order).
    issue_links: Vec<Option<(String, kfx_script::Span)>>,
    pub keymap: crate::shortcuts::Keymap,
    shortcut_capture: Option<&'static str>,
    show_shortcuts: bool,
    prefs_tab: String,
    // play
    pub game_log: play::GameLog,
    loose_prompt: Option<play::LoosePrompt>,
    // files
    last_autosave: Instant,
    recovery: Vec<crate::files::Snapshot>,
    map_fp: Vec<(String, Option<std::time::SystemTime>, u64)>,
    was_focused: bool,
    external_prompt: bool,
    backups: Option<safety::BackupsWindow>,
    image_dialog: Option<dialogs::ImageDialog>,
    export_dialog: Option<dialogs::ExportDialog>,
    /// Keyboard pan velocity (smoothing).
    pan_vel: Vec2,
    /// Gold values of `rules.cfg` for gold pile sizes.
    gold_rules: things::GoldRules,
    /// `mods/load_order.cfg` of the install.
    mods: Option<kfx_map::mods::ModsConfig>,
    /// The configuration editor bound to the open level.
    pub config: Option<config::ConfigState>,
    show_layers: bool,
    prov_file: String,
    prov_filter: String,
    /// Paint custom slab `id` instead of a palette slab.
    custom_brush: Option<u16>,
    custom_palette: Vec<custom::CustomTile>,
    custom_dialog: Option<custom::CustomDialog>,
    column_dialog: Option<custom::ColumnDialog>,
    note_dialog: Option<custom::NoteDialog>,
    decor_dialog: Option<custom::DecorDialog>,
    /// Map > Columns of this map.
    columns_win: Option<clm_editor::ColumnsWindow>,
    /// A one-time notice about the opened map (e.g. the custom slabs of its `.une` file were imported).
    notice: Option<String>,
    strings: Option<strings::StringsState>,
    show_strings: bool,
    /// The texture atlas of the open map (palette previews).
    atlas: Option<Arc<AtlasData>>,
    /// Camera of the 2D pane of the split view.
    pub camera2: Camera,
    camera2_home: bool,
    view_tex2: Option<egui::TextureId>,
    view_size2: (u32, u32),
    /// Rectangles of the viewport panes as last laid out (the split view has two).
    panes: Vec<egui::Rect>,
    /// Legacy save: the report shown before writing (warnings ask, errors block).
    legacy_prompt: Option<(PathBuf, u32, kfx_map::legacy::LegacyReport)>,
    /// The string tables the script editors' lookup reads (shared with its closure).
    string_shared: Arc<std::sync::RwLock<Option<kfx_formats::strings::StringResolver>>>,
    /// Where the docked script panel was last drawn (UI tests).
    // written by every frame, read only by the UI tests
    #[cfg_attr(not(test), allow(dead_code))]
    script_rect: Option<egui::Rect>,
    /// The Events editor (visual logic editor) of the open level's DK script.
    logic: Option<logic::LogicState>,
    /// The Prefabs panel, placing and the paired script fragments (created on first use).
    prefabs: Option<prefab::PrefabState>,
    /// The translation grid of the Strings window.
    texts: Option<texts::TextsState>,
    /// The docked Prefabs panel had no room this frame and is shown as a window.
    prefab_squeezed: bool,
    /// The campaign editor, its wizard and the Workshop window (`app/campaign.rs`,
    /// `app/workshop_win.rs`).
    camp: campaign::Hub,
    /// Game sounds: library, browser, lazy output device.
    audio: audio::AudioState,
    /// Live link with a running game.
    pub live: live::Live,
    /// The Analysis panel: background analysis, layers, findings.
    analysis: analysis_panel::AnalysisState,
    /// Map > Generate into selection.
    gen_patch: Option<generator::PatchDialog>,
}

fn img_to_color(img: &kfx_gfx::Rgba) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], &img.data)
}

fn ordered(a: (i32, i32), b: (i32, i32)) -> ((i32, i32), (i32, i32)) {
    ((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)))
}

impl EditorApp {
    pub fn new(
        ctx: &egui::Context,
        gpu: Gpu,
        settings: Settings,
        persist: bool,
        map: Option<PathBuf>,
    ) -> EditorApp {
        theme::apply(ctx);
        kfx_fonts::install(ctx);
        let view = ViewRenderer::with_samples(
            &gpu.device,
            if settings.graphics.msaa {
                crate::render::SAMPLES
            } else {
                1
            },
        );
        let keymap = crate::shortcuts::Keymap::new(&settings.shortcuts);
        let place_defaults = settings.place.clone();
        let mut app = EditorApp {
            ctx: ctx.clone(),
            gpu,
            settings,
            persist,
            assets: None,
            data_error: None,
            gd_cache: HashMap::new(),
            doc: None,
            doc_on_disk: false,
            cubes: CubeFaces::default(),
            terrain: Terrain::default(),
            status: String::new(),
            view,
            view_tex: None,
            view_size: (0, 0),
            camera: Camera::default(),
            camera_pending_home: true,
            camera_spec: None,
            hover: None,
            active_owner: 0,
            category: Category::Terrain,
            brush: None,
            palette: vec![],
            palette_search: String::new(),
            tool: Tool::Paint,
            brush_size: 1,
            brush_round: false,
            shape_filled: true,
            owner_only: false,
            editable_border: false,
            drag: None,
            pointer_drag: false,
            selection: None,
            clipboard: None,
            pasting: false,
            minimap_tex: None,
            minimap_dirty: true,
            backed_up: HashSet::new(),
            session_backups: crate::files::SessionBackups::new(Settings::backup_dir()),
            unreadable_prompt: None,
            pending: None,
            new_dialog: None,
            save_as_dialog: None,
            allow_close: false,
            browser: MapBrowser::default(),
            first_run: FirstRun::default(),
            show_about: false,
            show_controls: false,
            perf: Perf::default(),
            start: Instant::now(),
            fixed_frame: None,
            pending_open: map.clone(),
            quit_requested: false,
            last_vp: Vec2::new(1200.0, 800.0),
            last_vp2: Vec2::new(1200.0, 800.0),
            fly_capture: false,
            grid_cols: None,
            doodad_dialog: None,
            toolbar_widths: HashMap::new(),
            toasts: vec![],
            from_backup: false,
            packet_mode: None,
            fortify: false,
            last_status: String::new(),
            show_changelog: false,
            release_note: Default::default(),
            texture_sets: vec![],
            sprites: None,
            sprite_atlas: SpriteAtlas::default(),
            sprite_zips: 0,
            thing_palette: vec![],
            place: None,
            place_defaults,
            free_place: false,
            selected: BTreeSet::new(),
            hitboxes: vec![],
            badges: vec![],
            hitboxes2: vec![],
            badges2: vec![],
            markers2: vec![],
            last_click: None,
            thing_drag: None,
            rect_sel: None,
            inspector_buf: HashMap::new(),
            model_search: String::new(),
            inspector_rev: None,
            list_filter: ListFilter {
                kind: "All".into(),
                owner: None,
                text: String::new(),
            },
            symmetry: None,
            style_brush: None,
            issues: vec![],
            issues_rev: u64::MAX,
            issues_seen_rev: u64::MAX,
            issues_changed: Instant::now(),
            props_dialog: None,
            resize_dialog: None,
            regen_dialog: None,
            show_prefs: false,
            show_symmetry: false,
            scripts: None,
            script_max: false,
            pick: None,
            flash: None,
            number_prompt: None,
            markers: vec![],
            numbers: (vec![], vec![]),
            issue_links: vec![],
            keymap,
            shortcut_capture: None,
            show_shortcuts: false,
            prefs_tab: "Files".into(),
            game_log: play::GameLog {
                show_warnings: true,
                ..Default::default()
            },
            loose_prompt: None,
            last_autosave: Instant::now(),
            recovery: vec![],
            map_fp: vec![],
            was_focused: true,
            external_prompt: false,
            backups: None,
            image_dialog: None,
            export_dialog: None,
            pan_vel: Vec2::ZERO,
            gold_rules: things::GoldRules::default(),
            mods: None,
            config: None,
            show_layers: false,
            prov_file: "rules.cfg".into(),
            prov_filter: String::new(),
            custom_brush: None,
            custom_palette: vec![],
            custom_dialog: None,
            column_dialog: None,
            note_dialog: None,
            decor_dialog: None,
            columns_win: None,
            notice: None,
            strings: None,
            show_strings: false,
            atlas: None,
            camera2: Camera {
                top_down: true,
                yaw: 0.0,
                ..Default::default()
            },
            camera2_home: true,
            view_tex2: None,
            view_size2: (0, 0),
            panes: vec![],
            legacy_prompt: None,
            string_shared: Default::default(),
            script_rect: None,
            logic: None,
            prefabs: None,
            texts: None,
            prefab_squeezed: false,
            camp: Default::default(),
            // the output device only in the real window (not tests, not headless)
            audio: audio::AudioState::new(persist && !cfg!(test)),
            live: Default::default(),
            analysis: Default::default(),
            gen_patch: None,
        };
        app.apply_ui_settings();
        app.startup_extras();
        app.find_snapshots();
        app.camera.perspective = app.settings.layout.perspective;
        match pick_install(&app.settings, map.as_deref()) {
            Some(p) => app.set_install(p),
            None => {
                app.first_run.open = true;
                app.first_run.text = app
                    .settings
                    .install
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
            }
        }
        app
    }

    pub fn set_camera_spec(&mut self, spec: CameraSpec) {
        self.camera_spec = Some(spec);
        self.camera_pending_home = true;
    }

    pub fn show_browser(&mut self, open: bool, filter: &str) {
        self.browser.open = open;
        self.browser.filter = filter.to_string();
    }

    pub fn show_rooms_palette(&mut self) {
        self.category = Category::Rooms;
    }

    pub fn set_perspective(&mut self, p: bool) {
        self.camera.perspective = p;
        self.settings.layout.perspective = p;
    }

    pub fn install(&self) -> Option<&Path> {
        self.assets.as_ref().map(|d| d.install.root.as_path())
    }

    fn save_settings(&mut self) {
        if self.persist {
            if let Err(e) = self.settings.save() {
                self.status = trf(
                    "kfx_editor.could_not_save_settings_e",
                    "Could not save settings: {e}",
                    &[("e", &e)],
                );
            }
        }
    }

    fn set_install(&mut self, p: PathBuf) {
        match Assets::load(Install::new(&p)) {
            Ok(d) => {
                self.assets = Some(d);
                self.data_error = None;
                self.gd_cache.clear();
                self.mods = kfx_map::mods::ModsConfig::load(&p);
                if std::env::var_os("KFX_INSTALL").is_none()
                    && self.settings.install.as_deref() != Some(&p)
                {
                    self.settings.install = Some(p.clone());
                    self.save_settings();
                }
                self.browser.packs = None;
                self.status = trf(
                    "kfx_editor.install_p",
                    "Install: {p}",
                    &[("p", &p.display())],
                );
                if let Some(d) = self.doc.take() {
                    self.pending_open = Some(d.slb_path());
                }
            }
            Err(e) => {
                self.data_error = Some(format!("{}: {e}", p.display()));
                self.first_run.open = true;
            }
        }
    }

    /// Game configuration for a set of layers (cached).
    fn gamedata(&mut self, layers: &ConfigLayers) -> Result<Arc<GameData>, String> {
        if let Some(g) = self.gd_cache.get(layers) {
            return Ok(g.clone());
        }
        let g = Arc::new(GameData::load(layers).map_err(|e| {
            trf(
                "kfx_editor.game_configuration_e",
                "game configuration: {e}",
                &[("e", &e)],
            )
        })?);
        self.gd_cache.insert(layers.clone(), g.clone());
        Ok(g)
    }

    pub fn title(&self) -> String {
        match &self.doc {
            Some(d) => {
                let star = if d.is_dirty() || self.scripts_dirty() {
                    "*"
                } else {
                    ""
                };
                let where_ = if self.doc_on_disk {
                    d.slb_path().display().to_string()
                } else {
                    trf(
                        "kfx_editor.d_new_unsaved",
                        "{d} (new, unsaved)",
                        &[("d", &d.slb_path().display())],
                    )
                };
                trf(
                    "kfx_editor.star_keeperfx_map_editor_where",
                    "{star}KeeperFX Map Editor - [{where_}]",
                    &[("star", &star), ("where_", &where_)],
                )
            }
            None => trd("kfx_editor.keeperfx_map_editor_2ec4", "KeeperFX Map Editor").into(),
        }
    }

    /// The whole UI. `ui` is the root ui of the frame.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(p) = self.pending_open.take() {
            if self.assets.is_some() {
                self.request(Pending::Open(p));
            } else {
                self.pending_open = Some(p);
            }
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if let Some(p) = dropped.into_iter().next() {
            match crate::files::map_for_path(&p) {
                Some((m, n)) => {
                    if n > 1 {
                        self.status = trf(
                            "kfx_editor.n_maps_in_p_opening_the_first_use_the_map_browse",
                            "{n} maps in {p}: opening the first; use the map browser \
                            for the others",
                            &[("n", &n), ("p", &p.display())],
                        );
                    }
                    self.request(Pending::Open(m));
                }
                None => {
                    self.status = trf(
                        "kfx_editor.p_no_mapnnnnn_slb_there",
                        "{p}: no mapNNNNN.slb there",
                        &[("p", &p.display())],
                    )
                }
            }
        }
        let modal = self.pending.is_some()
            || self.new_dialog.is_some()
            || self.save_as_dialog.is_some()
            || self.props_dialog.is_some()
            || self.resize_dialog.is_some()
            || self.regen_dialog.is_some()
            || self.number_prompt.is_some()
            || self.loose_prompt.is_some()
            || self.external_prompt
            || self.shortcut_capture.is_some()
            || self.custom_dialog.is_some()
            || self.column_dialog.is_some()
            || self.note_dialog.is_some()
            || self.legacy_prompt.is_some()
            || self.unreadable_prompt.is_some()
            || (!self.recovery.is_empty() && self.assets.is_some());
        if !modal {
            self.global_shortcuts(&ctx);
            self.shortcuts(&ctx);
            self.analysis_shortcuts(&ctx);
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.anything_dirty()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Exit);
        }
        self.focus_tick(&ctx);
        self.update_scripts();

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(6, 4)),
            )
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if !self.script_max {
            let hmax = (ui.available_height() * 0.5).clamp(120.0, 600.0);
            let bottom = egui::Panel::bottom("palette")
                .resizable(true)
                .default_size(self.settings.layout.bottom_panel_h.min(hmax))
                .size_range(90.0..=hmax)
                .show(ui, |ui| self.palette_panel(ui));
            self.settings.layout.bottom_panel_h = bottom.response.rect.height();
            let tabs_left = self.settings.layout.tabs_left;
            let lmax = (side_panel_max(ui) - 200.0).clamp(180.0, 560.0);
            let left = egui::Panel::left("players")
                .resizable(true)
                .default_size(self.settings.layout.left_panel_w.min(lmax))
                .size_range(150f32.min(lmax)..=lmax)
                .show(ui, |ui| {
                    if tabs_left {
                        // the tree and the other panels as tabs (DK2 editor layout)
                        self.right_panel(ui);
                    } else {
                        self.players_panel(ui)
                    }
                });
            self.settings.layout.left_panel_w = left.response.rect.width();
            if self.settings.layout.show_history && !tabs_left {
                let max = side_panel_max(ui).min(640.0);
                let r = egui::Panel::right("history")
                    .resizable(true)
                    .default_size(self.settings.layout.right_panel_w.min(max))
                    .size_range(160f32.min(max)..=max)
                    .show(ui, |ui| self.right_panel(ui));
                self.settings.layout.right_panel_w = r.response.rect.width();
            }
            self.script_dock_ui(ui);
            self.config_dock_ui(ui);
            self.prefab_dock_ui(ui);
            egui::CentralPanel::no_frame().show(ui, |ui| self.viewport_ui(ui));
        } else {
            egui::CentralPanel::no_frame().show(ui, |ui| self.script_max_ui(ui));
        }
        self.script_window_ui(&ctx);

        let base_gd = self
            .install()
            .map(Path::to_path_buf)
            .and_then(|i| self.gamedata(&document::base_layers(&i)).ok());
        if let Some(a) = self.browser.show(
            &ctx,
            self.assets.as_ref().map(|d| d.install.root.as_path()),
            base_gd.as_deref(),
        ) {
            self.browser_action(a);
            ctx.request_repaint();
        }
        self.first_run_ui(&ctx);
        self.unsaved_prompt(&ctx);
        self.new_map_ui(&ctx);
        self.save_as_ui(&ctx);
        self.help_windows(&ctx);
        self.dialogs_extra(&ctx);
        self.prefs_window(&ctx);
        self.shortcuts_window(&ctx);
        self.script_dialogs(&ctx);
        self.logic_windows(&ctx);
        self.game_log_ui(&ctx);
        self.live_ui(&ctx);
        self.loose_prompt_ui(&ctx);
        self.backups_ui(&ctx);
        self.image_ui(&ctx);
        self.export_ui(&ctx);
        self.external_prompt_ui(&ctx);
        self.recovery_ui(&ctx);
        self.config_ui(&ctx);
        self.creature_stats_ui(&ctx);
        self.layers_window(&ctx);
        self.strings_window(&ctx);
        self.prefab_window_ui(&ctx);
        self.texts_window(&ctx);
        self.custom_dialog_ui(&ctx);
        self.column_dialog_ui(&ctx);
        self.note_dialog_ui(&ctx);
        self.decor_dialog_ui(&ctx);
        self.columns_window_ui(&ctx);
        self.doodad_dialog_ui(&ctx);
        self.legacy_prompt_ui(&ctx);
        self.unreadable_prompt_ui(&ctx);
        self.notice_ui(&ctx);
        self.changelog_ui(&ctx);
        self.campaign_ui(&ctx);
        self.toasts_ui(&ctx);
        self.analysis_window(&ctx);
        self.gen_patch_window(&ctx);
        self.analysis_tick();
        self.validate_tick();
        self.play_tick();
        self.audio_tick();
        self.live_tick();
        self.autosave_tick();

        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
        if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
            self.settings.layout.window_w = r.width();
            self.settings.layout.window_h = r.height();
        }
        if self.quit_requested {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn browser_action(&mut self, a: crate::browser::BrowserAction) {
        use crate::browser::BrowserAction as B;
        match a {
            B::Open(p) => self.request(Pending::Open(p)),
            B::Play(p) => self.play_file(&p),
            B::OpenFolder(d) => {
                if let Err(e) = crate::files::open_folder(&d) {
                    self.status = e;
                }
            }
            B::EditPack(cfg) => {
                if let Err(e) = self.open_campaign_cfg(&cfg) {
                    self.status = e;
                }
            }
            B::NewLevel { folder, level } => {
                if self.anything_dirty() {
                    self.status = trf(
                        "kfx_editor.map_level_lof_written_in_folder_save_or_discard",
                        "map{level}.lof written in {folder}; save or discard the \
                        open map, then File > New map",
                        &[
                            ("level", &format!("{:05}", level)),
                            ("folder", &folder.display()),
                        ],
                    );
                    return;
                }
                let mut d = dialogs::NewMapDialog::new(folder.display().to_string(), level);
                d.spec.players = 1;
                self.new_dialog = Some(d);
            }
        }
    }

    /// 3D, 2D or split view.
    pub fn set_view_mode(&mut self, m: crate::settings::ViewMode) {
        if m == crate::settings::ViewMode::TwoD {
            self.set_fly_capture(false);
        }
        if m != crate::settings::ViewMode::ThreeD
            && self.settings.layout.view_mode == crate::settings::ViewMode::ThreeD
        {
            self.camera2_home = true;
        }
        self.settings.layout.view_mode = m;
    }

    pub fn on_exit(&mut self) {
        self.remember_ui_memory();
        self.settings.shortcuts = self.keymap.overrides();
        self.settings.place = self.place_defaults.clone();
        if !self.anything_dirty() {
            self.drop_snapshot();
        }
        self.save_settings();
    }
}

/// eframe glue.
pub struct EframeApp(pub EditorApp);

impl eframe::App for EframeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
    fn on_exit(&mut self) {
        self.0.on_exit();
    }
}
