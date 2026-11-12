//! Persistent editor settings (install folder, recent maps, window layout), stored as JSON in
//! the platform config directory (`~/.config/kfx-editor` on Linux,
//! `%APPDATA%\KeeperFX\kfx-editor\config` on Windows). `KFX_EDITOR_CONFIG_DIR` overrides the
//! directory (tests, portable use).
use kfx_prefab::i18n::trf;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_RECENT: usize = 10;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Layout {
    pub window_w: f32,
    pub window_h: f32,
    pub left_panel_w: f32,
    pub bottom_panel_h: f32,
    pub perspective: bool,
    pub show_things: bool,
    pub show_grid: bool,
    pub show_ownership: bool,
    pub lighting: bool,
    pub show_minimap: bool,
    pub show_history: bool,
    /// Tab of the right-hand panel (`history`, `inspector`, `things`, `layers`, `validate`,
    /// `notes`, `files`).
    pub right_tab: String,
    pub layers: Layers,
    /// 3D, top-down 2D, or both side by side.
    pub view_mode: ViewMode,
    /// 2D view: flat slab-kind / owner colours instead of the textures.
    pub flat_colors: bool,
    /// View > Grid data: the raw grid written over the map.
    pub grid_data: crate::griddata::GridData,
    /// Slab palette: tile size, ids on the tiles, one list in id order.
    pub palette_tile: f32,
    pub palette_ids: bool,
    pub palette_by_id: bool,
    /// The panel tabs (history, inspector, things, layers, validate, notes, files) sit beside
    /// the player tree in the left panel (the default, like the DK2 editor), else in a right
    /// panel of their own.
    pub tabs_left: bool,
    /// Width of the right panel (history, inspector, ...).
    pub right_panel_w: f32,
    /// Width of the docked configuration editor.
    pub config_dock_w: f32,
    /// Campaign editor and Workshop windows.
    pub campaign: crate::app::CampaignLayout,
    /// The Analysis panel (open, layer, options).
    #[serde(default)]
    pub analysis: crate::analysis_view::AnalysisSettings,
}

/// How the map is shown.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// The game's isometric / perspective view.
    #[default]
    ThreeD,
    /// Top-down orthographic, north up, slab aligned.
    TwoD,
    /// 2D and 3D side by side (shared selection and hover).
    Split,
}

/// What the 3D view shows.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Layers {
    pub terrain: bool,
    pub creatures: bool,
    pub objects: bool,
    pub traps: bool,
    pub doors: bool,
    pub effects: bool,
    /// Things and lights created by slabs (torches, room furniture; `ParentTile` set).
    pub slab_things: bool,
    pub action_points: bool,
    pub lights: bool,
    /// Range circles of every effect generator, action point and light (else only selected).
    pub ranges: bool,
    /// Tint creature sprites with their owner's colour.
    pub owner_tint: bool,
    pub level_badges: bool,
    pub gate_numbers: bool,
    /// Script markers (hook for the script panel).
    pub script_markers: bool,
    /// Animate sprites.
    pub animate: bool,
    /// Facing arrows on creatures, objects and traps (always on selected things).
    pub facing_arrows: bool,
}

impl Default for Layers {
    fn default() -> Self {
        Layers {
            terrain: true,
            creatures: true,
            objects: true,
            traps: true,
            doors: true,
            effects: true,
            slab_things: true,
            action_points: true,
            lights: true,
            ranges: false,
            owner_tint: false,
            level_badges: true,
            gate_numbers: true,
            script_markers: true,
            animate: true,
            facing_arrows: false,
        }
    }
}

/// Names of the layer toggles (actions, menus).
pub const LAYER_NAMES: [&str; 16] = [
    "terrain",
    "creatures",
    "objects",
    "traps",
    "doors",
    "effects",
    "slab_things",
    "action_points",
    "lights",
    "ranges",
    "owner_tint",
    "level_badges",
    "gate_numbers",
    "script_markers",
    "animate",
    "facing_arrows",
];

impl Layers {
    pub fn get_mut(&mut self, name: &str) -> Option<&mut bool> {
        Some(match name {
            "terrain" => &mut self.terrain,
            "creatures" => &mut self.creatures,
            "objects" => &mut self.objects,
            "traps" => &mut self.traps,
            "doors" => &mut self.doors,
            "effects" => &mut self.effects,
            "slab_things" => &mut self.slab_things,
            "action_points" => &mut self.action_points,
            "lights" => &mut self.lights,
            "ranges" => &mut self.ranges,
            "owner_tint" => &mut self.owner_tint,
            "level_badges" => &mut self.level_badges,
            "gate_numbers" => &mut self.gate_numbers,
            "script_markers" => &mut self.script_markers,
            "animate" => &mut self.animate,
            "facing_arrows" => &mut self.facing_arrows,
            _ => return None,
        })
    }
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            window_w: 1600.0,
            window_h: 1000.0,
            left_panel_w: 280.0,
            bottom_panel_h: 170.0,
            perspective: false,
            show_things: true,
            show_grid: false,
            show_ownership: false,
            lighting: true,
            show_minimap: true,
            show_history: true,
            right_tab: "tree".into(),
            layers: Layers::default(),
            view_mode: ViewMode::ThreeD,
            flat_colors: false,
            grid_data: Default::default(),
            tabs_left: true,
            palette_tile: 52.0,
            palette_ids: false,
            palette_by_id: false,
            right_panel_w: 300.0,
            config_dock_w: 640.0,
            campaign: Default::default(),
            analysis: Default::default(),
        }
    }
}

/// Play-testing (Preferences > Files).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlaySettings {
    /// Game executable; `None` = detected in the install (`keeperfx` / `keeperfx.exe`).
    pub exe: Option<PathBuf>,
    /// Wrapper command put before the executable (`wine` for `keeperfx.exe` on Linux).
    /// Empty: none, or `wine` automatically for a `.exe` on a non-Windows system.
    pub wrapper: String,
    /// Extra command-line arguments (split on whitespace, `"..."` keeps spaces).
    pub extra_args: String,
    /// `-nointro` (main.cpp:1746).
    pub no_intro: bool,
    /// Windowed: `-cfg INGAME_RES=<res>w32` and `FRONTEND_RES` (main.cpp:1968, src/bflib_video.c:905).
    pub windowed: bool,
    pub window_res: String,
    /// `-alex`: the game's cheat / easter egg flag (main.cpp:1893).
    pub cheats: bool,
    /// `-skipheartzoom` (main.cpp:1750).
    pub skip_heart_zoom: bool,
    /// `-nosound` (main.cpp:1777).
    pub no_sound: bool,
    /// `-1player` for multiplayer maps (computer keepers for the other players, main.cpp:1772).
    pub one_player_mp: bool,
    /// Save the map (and scripts) before Play when there are changes.
    pub save_before_play: bool,
    /// Further game flags (main.cpp `process_command_line`): `-altinput`, `-show_game_turns`,
    /// `-framestep`, `-timer`, `-dbgpathfind`, `-dbgshots`, `-nomods`.
    pub alt_input: bool,
    pub show_game_turns: bool,
    pub frame_step: bool,
    pub timer: bool,
    pub dbg_pathfind: bool,
    pub dbg_shots: bool,
    pub no_mods: bool,
}

impl Default for PlaySettings {
    fn default() -> Self {
        PlaySettings {
            exe: None,
            wrapper: String::new(),
            extra_args: String::new(),
            no_intro: true,
            windowed: true,
            window_res: "1280x800".into(),
            cheats: false,
            skip_heart_zoom: true,
            no_sound: false,
            one_player_mp: true,
            save_before_play: false,
            alt_input: false,
            show_game_turns: false,
            frame_step: false,
            timer: false,
            dbg_pathfind: false,
            dbg_shots: false,
            no_mods: false,
        }
    }
}

/// Interface (Preferences > UI).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiSettings {
    /// Zoom of the whole interface (egui points per pixel factor).
    pub scale: f32,
    /// Body text size in points.
    pub font_size: f32,
    pub script_font_size: f32,
    /// Light theme for the script editor.
    pub script_light: bool,
    /// Language of the editor's texts (a catalogue code, `en` = English; see `crate::lang`).
    pub language: String,
}

impl Default for UiSettings {
    fn default() -> Self {
        UiSettings {
            scale: 1.0,
            font_size: 13.0,
            script_font_size: 14.0,
            script_light: false,
            language: "en".into(),
        }
    }
}

/// Camera controls (Preferences > Camera).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CameraSettings {
    /// Keyboard / edge pan speed in pixels per second.
    pub pan_speed: f32,
    /// Wheel zoom factor per notch (1 = default).
    pub zoom_speed: f32,
    /// Right-drag orbit sensitivity (1 = default).
    pub orbit_speed: f32,
    pub invert_orbit: bool,
    pub invert_zoom: bool,
    /// Pan when the pointer is at the edge of the view.
    pub edge_pan: bool,
    /// Smoothing of keyboard pans and zoom (0 = none, 0.9 = very smooth).
    pub smoothing: f32,
    /// Fly view speed in subtiles per second (Shift: three times faster).
    pub fly_speed: f32,
    /// Fly view: the eye cannot pass through walls, rock and earth or go below the floor.
    pub fly_collision: bool,
}

impl Default for CameraSettings {
    fn default() -> Self {
        CameraSettings {
            pan_speed: 700.0,
            zoom_speed: 1.0,
            orbit_speed: 1.0,
            invert_orbit: false,
            invert_zoom: false,
            edge_pan: false,
            smoothing: 0.0,
            fly_speed: 12.0,
            fly_collision: true,
        }
    }
}

/// Rendering (Preferences > Graphics). Lighting and animation are in [`Layout`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GraphicsSettings {
    pub msaa: bool,
    /// Linear filtering when the 3D view and icons are scaled (else nearest).
    pub smooth_scaling: bool,
    /// Brighter tone curve when zoomed out, so whole-map overviews read clearly.
    pub overview_boost: bool,
    /// Light the terrain with the map's lights near the camera when zoomed in.
    pub local_lights: bool,
    /// Script markers and badges cluster / fade when zoomed out.
    pub declutter: bool,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        GraphicsSettings {
            msaa: true,
            smooth_scaling: false,
            overview_boost: true,
            local_lights: true,
            declutter: true,
        }
    }
}

/// Where the script workspace is shown.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ScriptDock {
    /// Not shown.
    #[default]
    Hidden,
    /// A panel beside the map.
    Docked,
    /// Its own window (an OS window where the platform allows it).
    Window,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub install: Option<PathBuf>,
    pub recent: Vec<PathBuf>,
    /// Display names of recent maps (level name from the campaign / `.lof`).
    pub recent_names: Vec<(PathBuf, String)>,
    pub layout: Layout,
    /// Decorate placed / regenerated slabs (`kfx_map::decorate`).
    pub decoration: bool,
    /// Per-family overrides of the decoration defaults (family name, on).
    pub decor_families: Vec<(String, bool)>,
    pub play: PlaySettings,
    pub ui: UiSettings,
    pub camera: CameraSettings,
    pub graphics: GraphicsSettings,
    /// Crash-recovery snapshot interval in seconds while there are unsaved changes (0 = off).
    pub autosave_secs: u32,
    /// Total size of the backups folder in MiB; the oldest backups are removed beyond it
    /// (0 = keep everything).
    pub backup_limit_mb: u32,
    /// Keyboard shortcut overrides: (action id, shortcut text such as `Ctrl+Shift+S`).
    pub shortcuts: Vec<(String, String)>,
    /// Values new things get (Place tool).
    pub place: crate::app::PlaceDefaults,
    pub script_dock: ScriptDock,
    pub script_panel_w: f32,
    /// Ignore `mods/load_order.cfg` (the game ignores mods in multiplayer).
    pub ignore_mods: bool,
    /// Language code of the string tables (`ENG`, `FRE`, ...).
    pub language: String,
    /// Things may be placed inside walls and rock (else refused on subtiles whose column is 5
    /// or more cubes tall; lights, action points
    /// and doors are always allowed).
    pub place_anywhere: bool,
    /// Natural terrain (earth, rock, gold, liquids) takes the painted owner
    /// (Map > Ownable natural terrain); else it stays neutral.
    pub ownable_natural: bool,
    /// Update checks and installs (Preferences > Updates).
    pub updates: crate::updates::UpdateSettings,
    /// Rounded corners (one option per terrain pair).
    pub rounding: crate::rounding::Rounding,
    /// Wall art of Fortify: the game's rule (grouped) or random.
    pub wall_art: crate::fortify::WallArt,
    /// Tell when an opened map has RNC-packed files.
    pub rnc_notice: bool,
    /// Window positions and sizes (egui memory, JSON) of the last session.
    pub ui_memory: String,
    /// Version whose changelog the user has been pointed at.
    pub last_seen_version: String,
    /// Save format per map (`.slb` path, `fx` or `legacy`) from older versions: read when the
    /// map's sidecar has no `save_format`, and moved into the sidecar when the format is chosen.
    pub save_formats: Vec<(PathBuf, String)>,
    /// The Events editor tab and view.
    #[serde(default)]
    pub logic: crate::app::logic::LogicSettings,
    /// The Prefabs panel (open, docked, width).
    #[serde(default)]
    pub prefabs: crate::prefabs::PrefabSettings,
    /// Workshop preferences.
    pub workshop: crate::app::WorkshopSettings,
    /// Sound previews and the live link with a running game.
    #[serde(default)]
    pub audiolive: AudioLiveSettings,
}

/// Sound previews and the live link (Preferences > Sound, the Live link panel).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioLiveSettings {
    /// Never play sound (the output device is not opened).
    pub mute: bool,
    /// The Live link panel is shown.
    pub live_open: bool,
    /// Play / Save and play start the game with its API on (`-cfg API_ENABLED=ON -cfg
    /// API_PORT=<port>`, main.cpp:1968; the user's keeperfx.cfg is not touched).
    pub api_on_play: bool,
    /// `API_PORT` given to the game and the port the panel connects to.
    pub port: u16,
    /// Connect when a game is started with the API on.
    pub auto_connect: bool,
    /// Draw the in-game creature positions on the map.
    pub overlay: bool,
}

impl Default for AudioLiveSettings {
    fn default() -> Self {
        AudioLiveSettings {
            mute: false,
            live_open: false,
            api_on_play: false,
            port: 5599,
            auto_connect: true,
            overlay: true,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            install: None,
            recent: vec![],
            recent_names: vec![],
            layout: Layout::default(),
            decoration: true,
            decor_families: vec![],
            play: PlaySettings::default(),
            ui: UiSettings::default(),
            camera: CameraSettings::default(),
            graphics: GraphicsSettings::default(),
            autosave_secs: 60,
            backup_limit_mb: 512,
            shortcuts: vec![],
            place: Default::default(),
            script_dock: ScriptDock::Hidden,
            script_panel_w: 620.0,
            ignore_mods: false,
            language: "ENG".into(),
            save_formats: vec![],
            place_anywhere: false,
            ownable_natural: false,
            updates: Default::default(),
            rnc_notice: true,
            wall_art: Default::default(),
            rounding: Default::default(),
            ui_memory: String::new(),
            last_seen_version: String::new(),
            logic: Default::default(),
            prefabs: Default::default(),
            workshop: Default::default(),
            audiolive: Default::default(),
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("KFX_EDITOR_CONFIG_DIR") {
        return Some(PathBuf::from(d));
    }
    directories::ProjectDirs::from("net", "KeeperFX", "kfx-editor")
        .map(|p| p.config_dir().to_path_buf())
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("settings.json"))
    }

    /// Folder for map backups: `backups/` in the config directory (never in the game folder).
    pub fn backup_dir() -> Option<PathBuf> {
        config_dir().map(|d| d.join("backups"))
    }

    /// Folder for crash-recovery snapshots: `autosave/` in the config directory.
    pub fn autosave_dir() -> Option<PathBuf> {
        config_dir().map(|d| d.join("autosave"))
    }

    /// Defaults for everything except the install folder, the recent maps and the window size.
    pub fn reset(&mut self) {
        let keep = (
            self.install.clone(),
            self.recent.clone(),
            self.recent_names.clone(),
            self.layout.window_w,
            self.layout.window_h,
            self.updates.notice_acknowledged,
        );
        *self = Settings::default();
        self.install = keep.0;
        self.recent = keep.1;
        self.recent_names = keep.2;
        self.layout.window_w = keep.3;
        self.layout.window_h = keep.4;
        // the first-run notice about update checks is not shown again
        self.updates.notice_acknowledged = keep.5;
    }

    /// Display name of a recent map, when known.
    pub fn recent_name(&self, p: &Path) -> Option<&str> {
        self.recent_names
            .iter()
            .find(|(x, _)| x == p)
            .map(|(_, n)| n.as_str())
            .filter(|n| !n.is_empty())
    }

    pub fn set_recent_name(&mut self, p: &Path, name: &str) {
        let recent = self.recent.clone();
        self.recent_names
            .retain(|(x, _)| x != p && recent.contains(x));
        self.recent_names.push((p.to_path_buf(), name.to_string()));
    }

    /// Load the settings; missing or unreadable files give defaults.
    pub fn load() -> Settings {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = Self::path().ok_or("no config directory on this platform")?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let t = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        kfx_formats::atomic::atomic_write(&p, t.as_bytes())
            .map_err(|e| format!("{}: {e}", p.display()))
    }

    pub fn push_recent(&mut self, p: &Path) {
        self.recent.retain(|r| r != p);
        self.recent.insert(0, p.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }
}

/// Check that `dir` looks like a KeeperFX install: it must contain `fxdata/` and `data/`
/// (any letter case).
pub fn validate_install(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(trf(
            "kfx_editor.dir_is_not_a_folder",
            "{dir} is not a folder",
            &[("dir", &dir.display())],
        ));
    }
    let mut missing = vec![];
    for sub in ["fxdata", "data"] {
        if !kfx_gfx::install::resolve(dir, sub).is_some_and(|p| p.is_dir()) {
            missing.push(format!("{sub}/"));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(trf(
            "kfx_editor.dir_has_no_missing",
            "{dir} has no {missing}",
            &[
                ("dir", &dir.display()),
                ("missing", &missing.join(" and no ")),
            ],
        ))
    }
}

/// The nearest ancestor of `p` that is a valid install.
pub fn install_above(p: &Path) -> Option<PathBuf> {
    p.ancestors()
        .skip(1)
        .find(|d| validate_install(d).is_ok())
        .map(Path::to_path_buf)
}

/// Install folder to use: `KFX_INSTALL` env, else the saved setting, else one found above
/// `map` (when given). Only valid installs are returned.
pub fn pick_install(settings: &Settings, map: Option<&Path>) -> Option<PathBuf> {
    if let Some(e) = std::env::var_os("KFX_INSTALL") {
        let p = PathBuf::from(e);
        if validate_install(&p).is_ok() {
            return Some(p);
        }
    }
    if let Some(p) = settings
        .install
        .as_ref()
        .filter(|p| validate_install(p).is_ok())
    {
        return Some(p.clone());
    }
    map.and_then(install_above)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate() {
        let d = kfx_formats::testutil::scratch_dir(&format!(
            "kfx-editor/settings_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("FXDATA")).unwrap();
        assert!(validate_install(&d).unwrap_err().contains("data/"));
        std::fs::create_dir_all(d.join("data")).unwrap();
        assert!(validate_install(&d).is_ok());
        std::fs::create_dir_all(d.join("levels/x")).unwrap();
        assert_eq!(
            install_above(&d.join("levels/x/map00001.slb")),
            Some(d.clone())
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn recent_and_roundtrip() {
        let mut s = Settings::default();
        for i in 0..12 {
            s.push_recent(Path::new(&format!("/m/{i}")));
        }
        s.push_recent(Path::new("/m/5"));
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], PathBuf::from("/m/5"));
        let t = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&t).unwrap(), s);
        // unknown / missing fields fall back to defaults
        let s2: Settings = serde_json::from_str("{\"install\":\"/x\"}").unwrap();
        assert_eq!(s2.layout, Layout::default());
    }
}
