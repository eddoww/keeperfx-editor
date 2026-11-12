//! `kfx-editor`: the map editor for KeeperFX (Dungeon Keeper 1), as a desktop window (eframe /
//! egui with a wgpu 3D view) and as headless tools for scripts and CI.
//!
//! The library behind two executables (see [`main_entry`] and `--help`): `kfx-editor`, which
//! on Windows is built for the GUI subsystem (no console window), and `kfx-editor-cli`, a
//! console program for the command line on Windows. Elsewhere both behave the same and the
//! release archives ship only `kfx-editor`.
//!
//! Entry points:
//!
//! * `kfx-editor [MAP]`: the editor window ([`app::EditorApp`] wrapped in [`app::EframeApp`]).
//! * `kfx-editor update [--check]`: look for, download and install a newer release
//!   ([`update_cli`], on top of the `kfx-update` crate); `kfx-editor --version`.
//! * `kfx-editor validate | convert | regenerate | export-image | script-check | info MAP`: the
//!   commands of [`cli`] (no window, no GPU except `export-image`).
//! * `kfx-editor --screenshot OUT.png --map MAP [--actions FILE.json] [--record DIR]`: the
//!   offscreen editor of [`headless`]: renders the UI or the 3D view to a PNG, optionally after
//!   a JSON script of editor actions and as a frame-by-frame recording ([`record`]).
//!
//! Modules at a glance:
//!
//! * The application: [`app`] (the `EditorApp` state and its UI, split by area into `app/*`),
//!   [`settings`], [`shortcuts`], [`theme`], [`lang`] (UI language catalogues).
//! * The edited map: [`document`] (map, journal with undo / redo, saving), [`files`] (backups,
//!   crash-recovery snapshots, external changes), [`tools`] (brush geometry, clipboard),
//!   [`newmap`] and [`imageio`] (new maps, image to map), [`doodads`], [`fortify`],
//!   [`rounding`], [`prefabs`], [`scripts`] (level scripts), [`playtest`] (running the game).
//! * Rendering: [`render`] (GPU), [`terrain`], [`mesh`], [`atlas`], [`sprites`], [`assets`],
//!   [`camera`], [`pick`], [`minimap`], [`griddata`], [`fptd`], [`analysis_view`].
//! * Browsing: [`browser`] (the install's campaigns and map packs).
//!
//! The game data and file formats come from the workspace crates (`kfx-formats`, `kfx-map`,
//! `kfx-gfx`); the script, configuration, events, campaign, Workshop, audio, live link,
//! analysis and prefab features from their own `kfx-*` crates.
mod analysis_view;
mod app;
mod assets;
mod atlas;
mod browser;
mod camera;
mod cli;
mod console;
mod crash;
mod document;
mod doodads;
mod files;
mod fortify;
mod fptd;
#[cfg(all(test, unix))]
mod game_test;
#[cfg(all(test, unix))]
mod game_test_e2e;
#[cfg(all(test, unix))]
mod game_test_live;
mod griddata;
mod headless;
mod imageio;
mod lang;
mod mesh;
mod minimap;
mod newmap;
mod pick;
mod playtest;
mod prefabs;
mod record;
mod render;
mod rounding;
mod scripts;
mod settings;
mod shortcuts;
mod sprites;
#[cfg(test)]
mod sweep_tests;
mod terrain;
mod theme;
mod tools;
#[cfg(test)]
mod ui_tests;
#[cfg(test)]
mod ui_tests_analysis;
#[cfg(test)]
mod ui_tests_audio;
#[cfg(test)]
mod ui_tests_campaign;
#[cfg(test)]
mod ui_tests_columns_palette;
#[cfg(test)]
mod ui_tests_editing;
#[cfg(test)]
mod ui_tests_events;
#[cfg(test)]
mod ui_tests_fonts;
#[cfg(test)]
mod ui_tests_i18n;
#[cfg(test)]
mod ui_tests_live;
#[cfg(test)]
mod ui_tests_prefab;
#[cfg(test)]
mod ui_tests_safety;
#[cfg(test)]
mod ui_tests_updates;
#[cfg(test)]
mod ui_tests_views;
#[cfg(test)]
mod ui_tests_workshop;
mod update_cli;
mod updates;

use std::path::PathBuf;

const USAGE: &str = r#"usage:
  kfx-editor [MAP]
      open the editor window (MAP: path to a mapNNNNN.slb)
  kfx-editor --screenshot OUT.png --map MAP [options]
      render without a window and write a PNG
      --size WxH              image size (default 1600x1000)
      --camera x,y,zoom,yaw,pitch
                              target subtile x,y; zoom in percent; yaw and pitch in degrees
      --perspective           perspective instead of the isometric (orthographic) view
      --viewport-only         only the 3D view, no UI
      --pointer px,py         simulate the mouse at this pixel (hover readout + outline)
      --frame N               animation frame 0..7 (default 0)
      --bench N               also time N viewport frames and print the result
      --show-browser          open the map browser window
      --browser-filter TEXT   open the map browser with this filter
      --rooms-palette         show the Rooms palette instead of Terrain
      --slab-image OUT.png    also write a top-down debug image of the slab kinds
      --actions FILE.json     run scripted editor actions first (see below); OUT.png is
                              written after the last action
      --map -                 start with no map open
      --record DIR|-          recording: write every frame of the recording actions as
                              DIR/frame_NNNNNN.png, or as raw RGBA on standard output (-)
                              (pipe into: ffmpeg -f rawvideo -pix_fmt rgba -s WxH -r FPS -i -)
      --record-captions FILE  write the captions of a recording (frame ranges) as JSON
      --fps N                 frames per second of a recording (default 30)
      --forbid-text TEXT      fail if a frame or screenshot shows TEXT (case-insensitive;
                              repeatable), e.g. a home folder before publishing pictures
recording actions (simulated input, one or more frames each; docs/manual/cli.md):
  {"hold": secs, "realtime": false, "write": true}  {"rec": frames}  {"auto_frames": n}
  {"stride": n}  {"cursor": false}  {"anim": true}  {"view_only": true}
  {"move": {"to": T, "secs": 0.6, "ease": "inout|linear|in|out"}}  {"tap": "left"}
  {"tap": {"to": T, "count": 2}}  {"down": "left"}  {"up": "left"}
  {"pdrag": {"to": T, "button": "right", "secs": 1}}
  {"stroke": {"slabs": [[x,y], ...], "secs": 2, "two_d": false}}
  {"type": {"text": "..", "cps": 14}}  {"keys": "ctrl+z", "times": 3, "secs": 0.3}
  {"wheel": {"dy": -300, "secs": 0.6}}  {"camera_to": {"cam": [x,y,zoom,yaw,pitch], "secs": 2}}
  {"orbit": {"deg": 360, "secs": 20}}  {"caption": "text"|null}
  {"fly_path": {"move": [right,up,forward], "look": [yaw,pitch], "secs": 4}}
  targets T: [px,py] | {"slab": [x,y]} | {"stl": [x,y]}
             | {"text": "Label", "nth": 0, "in": [x0,y0,x1,y1], "dx": 0}
             | {"word": "WORD"} (a word inside any drawn text, e.g. the script editor)
scripted actions (a JSON array, one object per action, slab coordinates):
  {"palette": "TREASURE"}  {"owner": 0}  {"tool": "paint|line|box|circle|fill|eyedropper|select"}
  {"brush": 3, "round": true}  {"filled": false}  {"owner_only": true}  {"border": true}
  {"press": [x,y], "alt": false}  {"drag": [[x,y], ...]}  {"release": [x,y]}  {"click": [x,y]}
  {"hover": [x,y]}  {"key": "Escape|R|F|Delete"}  {"undo": n}  {"redo": n}  {"undo_all": true}
  {"select": [[x0,y0],[x1,y1]]}  {"copy": true}  {"cut": true}  {"paste": true}
  {"save": true}  {"save_as": {"folder": "DIR", "level": n}}  {"open": "MAP.slb"}
  {"new": {"folder": "DIR", "level": n, "w": 85, "h": 85, "border": true, "earth": true,
      "texture": 0}}
  {"view": {"ownership": true, "grid": true, "lighting": true}}
  {"camera": [x,y,zoom,yaw,pitch]}  {"screenshot": "OUT.png", "ui": false}  {"print": true}
things and map tools (subtile coordinates unless noted):
  {"tool": "place|things"}
  {"thing_palette": "creature:IMP|object:GOLD|trap:BOULDER|door:WOOD|effect:NAME|light|
      action_point"}
  {"place_defaults": {"level": 5, "orientation": 768, "locked": true, "light_range": 6, ...}}
  {"free_place": true}  {"place": [x,y], "sub": [128,128]}
  {"select_things": [[x0,y0],[x1,y1]], "add": false}  {"select_class": "creatures"}
  {"select_none": true}  {"mouse": {"press": [x,y], "to": [x,y], "shift": false, "alt": false}}
      (real pointer gesture)
  {"move_things": [dx,dy]}  {"delete_things": true}  {"duplicate": true}  {"rotate_things": n}
  {"paste_at": [x,y]}  {"set": {"level": 7, "owner": 1, ...}} (inspector fields)
  {"list_things": true}  {"print_things": true}  {"focus": [x,y]}
  {"resize": {"left": 0, "top": 5, "right": 10, "bottom": 0, "fill": "EARTH"}} (slabs)
  {"transform": "flipx|flipy|rot90|rot180|rot270", "selection": false}
  {"symmetry": "off|ns|ew|fourway|rot2|rot4", "sym_owners": [[0,1,..8], ...]}  {"style": n|null}
  {"regenerate": {"columns": true, "decoration": true, "objects": true, "liquid": true,
      "selection": false}}
  {"decoration": true}  {"validate": true}  {"layers": {"creatures": false, ...}}
  {"tab": "objects"}
  {"right_tab": "history|inspector|things|layers|validate"}
  {"frame": [[x0,y0],[x1,y1]], "ui": true}
  {"properties": {"name": "..", "author": "..", "texture": 3, "show": true}}
  {"open_dialog": "properties|resize|regenerate|preferences|symmetry|new|shortcuts|backups|game_log|
      controls|about|export|image|browser|columns|landview|none"}
scripts, play-testing, files, map browser, new map:
  {"script": "docked|window|max|hidden"}  {"script_width": 640}
  {"script_create": "dk|lua", "hints": true}  {"script_tab": "map00001.lua"}  {"script_goto": line}
  {"script_caret": [line,col]}  {"script_end": true}  {"script_text": ".."}
  {"script_complete": true}  {"script_analyze": true}  {"script_print": true}  {"script_save": true}
  {"script_pick": "action_point|hero_gate|location|subtile|slab"}
  {"pick": [stl x,y]}  {"pick_ap": n}  {"jump": "ap:3|gate:1|stl:x,y|slab:x,y"}
  {"answer_scripts": true}
  {"pointer": [px,py]}  {"print_markers": true}  {"game_exe": "PATH"}  {"game_log": true}
  {"game_log_go": n}  {"print_launch": true}  {"browser": {"filter": "..", "select": "MAP|random"}}
  {"recover": true}  {"show_recovery": true}
  {"new_dialog": {spec}}
  {"new_map": {"folder": .., "level": n, spec}} (spec: w h border fill texture
      generate seed symmetry players imps portals gold_near hero_gates script kind name;
      generator: style gen_symmetry richness water lava heroes gems neutral_rooms
      neutral_creatures special_boxes min_gold max_imbalance max_attempts)
  {"new_dialog_folder": "DIR"}  {"new_dialog_seed": n}  {"new_dialog_regenerate": true}
  {"new_dialog_wait": true}  {"new_dialog_create": true}
analysis, generator, images, preferences:
  {"analysis": true}  {"analysis_tab": "layers|fairness|economy|routes|symmetry|heroes"}
  {"analysis_wait": true}
  {"analysis_layer": "off|reach|dig|walk|territory|heroes|gold|class|symmetry"}
  {"analysis_player": 0}  {"analysis_opacity": 120}
  {"analysis_symmetry": "auto|ns|ew|fourway|rot2|rot4"}
  {"analysis_route": n|null}  {"analysis_breaker": n}  {"analysis_print": true}
  {"analysis_bench": n}
  {"generate_selection": {"seed": n, "style": "chambers", "players": 0, ...}} (needs a selection)
  {"image_dialog": "IN.png"}  {"image_apply": true}
  {"export_image": {"out": .., "iso": true, "scale": 16}}
  {"prefs_tab": "Play"}  {"autosave": true}  {"check_external": true}
  {"raw": {"slab": [x,y], "kind": "DOOR_WOODEN"}} / {"raw": {"door_at": [x,y]}}
      (test aid: inconsistent maps)
configuration, mods, custom slabs, strings, formats, views:
  {"config": {"tab": "rules|slabset|..", "target": "map|campaign|base|mod:NAME",
              "set": [file, section, key, value],
              "slabset": {"kind": 10, "style": 8, "subtile": 4, "column": 1}, "save": true,
              "regen": KIND|"all", "live": true, "docked": true, "print": true, "close": true}}
  {"layers_window": true}  {"ignore_mods": true}  {"custom_from": [x,y], "name": ..}
  {"custom_brush": id}
  {"custom_columns": {"name": .., "recognized_as": 10, "columns": [9 ids]}}
  {"edit_column": {"stl": [x,y], "cubes": [..8], "floor": n}}
  {"lock": [[x0,y0],[x1,y1]], "note": ..}
  {"unlock": true}  {"add_note": {"at": [x,y], "text": ..}}
  {"map_decoration": {"seed": n, "enabled": true, "disabled": [..]}}
  {"strings_window": true}  {"string_set": {"id": n, "text": .., "campaign": false}}
  {"string_new": ..}
  {"string_insert": ..}  {"strings_save": true}  {"print_string": n}  {"print_caret_messages": true}
  {"save_format": "fx|legacy"}  {"view_mode": "3d|2d|split"}  {"flat": true}
  {"camera2": [x,y,zoom%]}
  {"graphics": {"overview_boost": true, "local_lights": true, "declutter": true}}
fly view, grid data, doodads, windows, script wizard, placement options:
  {"fly": true}  {"fly_look": [yaw_deg, pitch_deg]}  {"fly_move": [right, up, forward]}
  {"fly_collision": false}
  {"grid_data": "off|kind|owner|wlb|style|custom|column|wibble|floor|flags"}
  {"ui_scale": 1.5}
  {"doodads": {"seed": n, "rule": [{"slab": "PATH", "kind": "object|effect|light",
      "model": "BARREL", "chance": 2.5}]}}
  {"open_window": "doodads|stats|wizard|lua_wizard"}
  {"wizard_set": {"player": 0, "start_money": n, "max_creatures": n, "win": true, "lose": true,
      "rooms": {"TREASURE": "yes|research|no"}, "generate_speed": n}}  {"wizard_apply": true}
  {"fortify": true}  {"wall_art": "grouped|random"}  {"place_anywhere": true}
  {"ownable_natural": true}  {"door_lock": true}
  benchmarks: {"bench_populate": n}
  {"bench_stroke": {"from": [x,y], "steps": 60, "brush": 3, "ui": true}}
      {"bench_undo": n}  {"bench_ui_frames": n}  {"bench_save": "DIR"}  {"print_mem": true}
events editor (docs/manual/cli.md):
  {"events": "open|text|toggle"}  {"events_view": "cards|graph|timeline|parties|variables|lua"}
  {"events_select": n|"text"}  {"events_party": "NAME"}  {"events_undo": n}  {"events_redo": n}
  {"events_print": true}
  {"events_add_wave": {"party": "RAIDERS", "creatures": [["KNIGHT", 4, 300], ..],
      "owner": "PLAYER_GOOD", "spawn": {"hero_gate": 1}|{"action_point": 2},
      "turn": 6000 | "condition": {"player", "variable", "op", "value"}, "copies": 1, "count": 1,
      "interval": 3000, "copies_step": 0, "objective": .., "tunneller": {"head_for", "target"}}}
prefabs and translations (docs/manual/cli.md):
  {"prefabs": true, "docked": true}  {"prefab_select": "NAME"}  {"prefab_rotate": 90}
  {"prefab_flip": true}  {"prefab_owners": [1, 0]}
  {"prefab_mask": "replace|skip_empty|keep_existing"}
  {"prefab_ghost": true}  {"prefab_place": [x,y], "centre": true}  {"prefab_save": {"name": .., ..}}
  {"prefab_save_dialog": true}  {"prefab_print": true}
  {"texts": "level|campaign"}  {"texts_add_lang": ..}  {"texts_set": ..}  {"texts_save": true}
  {"texts_print": true}  {"texts_export_csv": ..}  {"texts_import_csv": ..}  {"texts_export_po": ..}
  {"texts_import_po": ..}
campaigns, Workshop, sounds, live link, language (docs/manual/cli.md and the feature pages):
  {"campaign": {..}}  {"campaign_wizard": {..}}  {"workshop": {..}} (always offline)
  {"sounds": {"tab": "sounds|creature", "creature": "IMP", "filter": .., "category": ..,
      "play": "fx:32"}}  {"mute": true}  {"speech_preview": "intro|outro|FILE"}
  {"live": {"open": true, "connect": "HOST:PORT", "tab": .., "api_on_play": true, "port": n,
      "overlay": true, "disconnect": true}}  {"live_demo": true}  {"print_audiolive": true}
  {"language": "fr"}
environment:
  KFX_INSTALL             KeeperFX folder (overrides the saved setting)
  KFX_EDITOR_CONFIG_DIR   settings folder (default: the platform config dir)"#;

/// The version of this build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which executable is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exe {
    /// `kfx-editor`: on Windows a GUI-subsystem program, which has no console to print to.
    Gui,
    /// `kfx-editor-cli`: a console program (Windows only in the release archives).
    Cli,
}

/// The `main` of both executables.
pub fn main_entry(exe: Exe) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // A GUI-subsystem program started from a terminal cannot print to it: run the command
    // through kfx-editor-cli (hidden), keep its output in a log file and show it in a window.
    if exe == Exe::Gui && cfg!(windows) && console::wants_console(&args) {
        std::process::exit(console::run_for_gui(&args));
    }
    std::process::exit(run(&args));
}

/// Run the command line `args` (without the program name); returns the exit code.
pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}\n\n{}\n\n{}", cli::USAGE, update_cli::USAGE);
        return 0;
    }
    if args.first().is_some_and(|a| a == "--version" || a == "-V") {
        println!("{}", cli::version_text());
        return 0;
    }
    if args.first().is_some_and(|a| a == "update") {
        return update_cli::run(&args[1..]);
    }
    if args
        .first()
        .is_some_and(|a| cli::COMMANDS.contains(&a.as_str()))
    {
        return cli::run(args);
    }
    if args.iter().any(|a| a == "--screenshot") {
        // list the system fonts in the background while the GPU and the data load
        kfx_fonts::warm_up();
        return match headless::Options::parse(args).and_then(|o| headless::run(&o)) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("kfx-editor: {e}");
                1
            }
        };
    }
    // a map file or a level folder (file association, drag onto the program)
    let map = match window_map_arg(args) {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "kfx-editor: {e}\n\n{USAGE}\n\n{}\n\n{}",
                cli::USAGE,
                update_cli::USAGE
            );
            return 2;
        }
    };
    crash::install_hook(settings::config_dir());
    if let Err(e) = run_window(map) {
        eprintln!("kfx-editor: {e}");
        return 1;
    }
    0
}

/// The map argument of a window start: none, or an existing map file / level folder. An unknown
/// subcommand, an unknown option or a path that does not exist is an error (exit code 2), so a
/// typo never silently opens the window.
fn window_map_arg(args: &[String]) -> Result<Option<PathBuf>, String> {
    let mut map = None;
    for a in args {
        if a.starts_with('-') {
            return Err(format!("unknown option {a:?}"));
        }
        if map.is_some() {
            return Err(format!("unexpected argument {a:?}"));
        }
        let p = PathBuf::from(a);
        if !p.exists() {
            let bare = !a.contains(['/', '\\', '.']);
            return Err(if bare {
                format!("unknown command {a:?}")
            } else {
                format!("{a}: no such file or folder")
            });
        }
        map = Some(files::map_for_path(&p).map(|m| m.0).unwrap_or(p));
    }
    Ok(map)
}

fn run_window(map: Option<PathBuf>) -> Result<(), String> {
    // list the system fonts in the background while the window opens (`EditorApp::new` installs
    // them as fallbacks)
    kfx_fonts::warm_up();
    let settings = settings::Settings::load();
    let l = settings.layout.clone();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("KeeperFX Map Editor")
            .with_icon(window_icon())
            .with_app_id("kfx-editor")
            .with_inner_size([l.window_w.max(640.0), l.window_h.max(480.0)])
            .with_min_inner_size([640.0, 480.0]),
        // the editor installs its own system font fallback (kfx_fonts), for every context
        system_font_fallback: false,
        ..Default::default()
    };
    eframe::run_native(
        "kfx-editor",
        options,
        Box::new(move |cc| {
            let rs = cc
                .wgpu_render_state
                .as_ref()
                .ok_or("the wgpu renderer is not available")?;
            let gpu = app::Gpu {
                device: rs.device.clone(),
                queue: rs.queue.clone(),
                egui_renderer: rs.renderer.clone(),
            };
            eprintln!(
                "kfx-editor: adapter {}",
                egui_wgpu::adapter_info_summary(&rs.adapter.get_info())
            );
            Ok(Box::new(app::EframeApp(app::EditorApp::new(
                &cc.egui_ctx,
                gpu,
                settings,
                true,
                map,
            ))))
        }),
    )
    .map_err(|e| e.to_string())
}

/// The application icon for the window (title bar, task bar, window switcher), from the PNG
/// made by `tools/make_icon.py`.
pub fn window_icon() -> eframe::egui::IconData {
    const PNG: &[u8] = include_bytes!("../../../packaging/icons/kfx-editor-256.png");
    match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            eframe::egui::IconData {
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
            }
        }
        Err(_) => eframe::egui::IconData::default(),
    }
}

#[cfg(test)]
mod cli_args {
    use super::window_map_arg;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unknown_words_and_missing_paths_are_errors() {
        assert_eq!(window_map_arg(&[]).unwrap(), None);
        let e = window_map_arg(&args(&["valdate"])).unwrap_err();
        assert!(e.contains("unknown command"), "{e}");
        let e = window_map_arg(&args(&["no/such/map00001.slb"])).unwrap_err();
        assert!(e.contains("no such file"), "{e}");
        let e = window_map_arg(&args(&["--frobnicate"])).unwrap_err();
        assert!(e.contains("unknown option"), "{e}");
        // an existing folder is accepted (the window opens on it)
        let d = std::env::temp_dir();
        assert!(window_map_arg(&args(&[d.to_str().unwrap()]))
            .unwrap()
            .is_some());
    }

    #[test]
    fn version_and_icon() {
        assert_eq!(super::run(&args(&["--version"])), 0);
        let i = super::window_icon();
        assert_eq!((i.width, i.height), (256, 256));
        assert_eq!(i.rgba.len(), 256 * 256 * 4);
    }
}
