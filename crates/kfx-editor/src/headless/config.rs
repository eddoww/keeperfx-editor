//! Actions of the configuration editor, mods, custom slabs, column edits, locks, notes, string
//! tables, the save format and the 2D view.
use super::*;

/// Configuration, mods, custom slabs, locks, notes, strings, formats, 2D view (see the usage
/// text).
pub(super) fn run_config_and_view_actions(
    hl: &mut Headless,
    app: &mut EditorApp,
    o: &Options,
    obj: &serde_json::Map<String, serde_json::Value>,
    rel: &dyn Fn(&str) -> PathBuf,
) -> Result<(), String> {
    let get = |k: &str| obj.get(k);
    let b = |v: &serde_json::Value| v.as_bool().unwrap_or(true);
    let s = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
    let pair = |v: &serde_json::Value| point(v);
    if let Some(v) = get("ignore_mods") {
        app.settings.ignore_mods = b(v);
        app.reload_mods();
    }
    if let Some(v) = get("config").and_then(|v| v.as_object()) {
        use kfx_config_ui::ui::Tab as T;
        let tab = v.get("tab").and_then(|x| x.as_str()).map(|t| match t {
            "rules" => T::Rules,
            "creatures" => T::Creatures,
            "objects" => T::Objects,
            "terrain" => T::Terrain,
            "slabset" => T::Slabset,
            "columns" => T::Columns,
            "cubes" => T::Cubes,
            "files" => T::Files,
            _ => T::Generic,
        });
        app.open_config_editor(tab);
        if let Some(t) = v.get("target").and_then(|x| x.as_str()) {
            if let Some(m) = t.strip_prefix("mod:") {
                app.edit_mod_folder(m)?;
            } else {
                app.set_config_target(t)?;
            }
        }
        let c = app.config.as_mut().ok_or("no configuration editor")?;
        if let Some(d) = v.get("docked") {
            c.docked = b(d);
        }
        if let Some(d) = v.get("live") {
            c.live_regen = b(d);
        }
        if let Some(a) = v.get("set").and_then(|x| x.as_array()) {
            let t: Vec<String> = a.iter().map(s).collect();
            if t.len() != 4 {
                return Err("config set needs [file, section, key, value]".into());
            }
            c.editor.session.set_cfg(&t[0], &t[1], &t[2], &t[3]);
            if t[0] == "rules.cfg" {
                c.editor.rules.section = t[1].clone();
            }
            c.editor.generic = kfx_config_ui::ui::form::FormState::new(&t[0]);
            c.editor.generic.section = Some(t[1].clone());
        }
        if let Some(x) = v.get("slabset").and_then(|x| x.as_object()) {
            let n = |k: &str| x.get(k).and_then(|y| y.as_u64()).unwrap_or(0) as usize;
            let (kind, style, sub) = (n("kind"), n("style"), n("subtile"));
            // a column id, or the column of another slab kind's same style and subtile
            let col = match x.get("column_from").and_then(|y| y.as_u64()) {
                Some(k) => c.editor.session.slabset().style(k as usize, style).columns[sub.min(8)],
                None => x.get("column").and_then(|y| y.as_i64()).unwrap_or(1),
            };
            let mut st = c.editor.session.slabset().style(kind, style);
            st.columns[sub.min(8)] = col;
            c.editor.session.slabset().set_style(kind, style, st);
            c.editor.session.touched("slabset.toml");
            c.editor.slabset.kind = kind;
            c.editor.slabset.style = style;
            c.editor.slabset.subtile = sub.min(8);
        }
        if v.get("save").is_some_and(b) {
            let saved = c.editor.session.save()?;
            eprintln!("config saved: {saved:?}");
            c.editor
                .session
                .push_event(kfx_config_ui::ConfigEvent::Saved {
                    file: String::new(),
                    path: saved.first().cloned().unwrap_or_default(),
                    deleted: false,
                });
        }
        // the events of the scripted edits, like a frame of the editor would deliver them
        let events = app
            .config
            .as_mut()
            .map(|c| c.editor.session.take_events())
            .unwrap_or_default();
        if !events.is_empty() {
            app.apply_config_preview();
            if events
                .iter()
                .any(|e| matches!(e, kfx_config_ui::ConfigEvent::Saved { .. }))
            {
                app.status = "configuration saved".into();
            }
        }
        if let Some(k) = v.get("regen") {
            let kinds: Vec<u16> = match k.as_str() {
                Some("all") => app
                    .config
                    .as_ref()
                    .map(|c| c.affected.iter().map(|x| x.0).collect())
                    .unwrap_or_default(),
                _ => k.as_u64().map(|x| vec![x as u16]).unwrap_or_default(),
            };
            app.regenerate_kinds(&kinds);
        }
        if v.get("print").is_some() {
            let c = app.config.as_ref().unwrap();
            eprintln!(
                "config target {:?}, affected {:?}, pending {:?}",
                c.editor.session.target_kind(),
                c.affected,
                c.editor
                    .session
                    .pending()
                    .iter()
                    .map(|p| (p.0.clone(), p.1.clone()))
                    .collect::<Vec<_>>()
            );
        }
        if v.get("close").is_some_and(b) {
            if let Some(c) = app.config.as_mut() {
                c.open = false;
            }
        }
    }
    if get("layers_window").is_some() {
        app.open_named_dialog("layers")?;
        if let Some(l) = app.layers_summary() {
            eprintln!("{l}");
        }
    }
    if let Some(v) = get("custom_from").and_then(pair) {
        let id = app.create_custom_from_slab(v.0, v.1, get("name").and_then(|x| x.as_str()))?;
        eprintln!("custom slab {id}");
    }
    if let Some(v) = get("custom_columns").and_then(|v| v.as_object()) {
        let cols: Vec<usize> = v
            .get("columns")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_u64())
                    .map(|c| c as usize)
                    .collect()
            })
            .unwrap_or_default();
        let cols: [usize; 9] = cols
            .try_into()
            .map_err(|_| "custom_columns needs 9 columns")?;
        let kind = v
            .get("recognized_as")
            .and_then(|x| x.as_u64())
            .unwrap_or(10) as u16;
        let id = app.create_custom_from_columns(
            v.get("name").and_then(|x| x.as_str()).unwrap_or("custom"),
            kind,
            cols,
        )?;
        eprintln!("custom slab {id}");
    }
    if let Some(v) = get("custom_brush").and_then(|v| v.as_u64()) {
        app.select_custom_brush(v as u16);
    }
    if let Some(v) = get("edit_column").and_then(|v| v.as_object()) {
        let stl = v
            .get("stl")
            .and_then(point)
            .ok_or("edit_column needs stl [x,y]")?;
        let cubes: Vec<u16> = v
            .get("cubes")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_u64())
                    .map(|c| c as u16)
                    .collect()
            })
            .unwrap_or_default();
        let mut c8 = [0u16; 8];
        for (i, c) in cubes.iter().take(8).enumerate() {
            c8[i] = *c;
        }
        let id = app.edit_column(
            (stl.0 as usize, stl.1 as usize),
            c8,
            v.get("floor").and_then(|x| x.as_u64()).unwrap_or(0) as u16,
        )?;
        eprintln!("column edited: custom slab {id}");
    }
    if let Some(v) = get("lock") {
        let a = v
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or("lock needs [[x0,y0],[x1,y1]]")?;
        app.select_rect(
            point(&a[0]).ok_or("bad point")?,
            point(&a[1]).ok_or("bad point")?,
        );
        app.lock_selection(get("note").and_then(|x| x.as_str()).unwrap_or(""))?;
    }
    if get("unlock").is_some() {
        eprintln!("unlocked {}", app.unlock_selection()?);
    }
    if let Some(v) = get("add_note").and_then(|v| v.as_object()) {
        let at = v.get("at").and_then(point).ok_or("add_note needs at")?;
        app.add_note(at, v.get("text").and_then(|x| x.as_str()).unwrap_or("note"))?;
    }
    if let Some(v) = get("map_decoration").and_then(|v| v.as_object()) {
        let seed = v.get("seed").and_then(|x| x.as_u64());
        let dis: Vec<String> = v
            .get("disabled")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().map(s).collect())
            .unwrap_or_default();
        app.set_map_decoration(seed, v.get("enabled").map(b).unwrap_or(true), dis);
    }
    if get("strings_window").is_some() {
        app.open_named_dialog("strings")?;
    }
    if let Some(v) = get("string_set").and_then(|v| v.as_object()) {
        let id = v
            .get("id")
            .and_then(|x| x.as_u64())
            .ok_or("string_set needs id")? as usize;
        let target = if v.get("campaign").is_some_and(b) {
            crate::app::StringTarget::Campaign
        } else {
            crate::app::StringTarget::Level
        };
        app.set_string(
            id,
            v.get("text").and_then(|x| x.as_str()).unwrap_or(""),
            target,
        )?;
    }
    if let Some(v) = get("string_new").and_then(|v| v.as_str()) {
        eprintln!("new string {}", app.new_level_string(v)?);
    }
    if let Some(v) = get("string_insert").and_then(|v| v.as_str()) {
        eprintln!("new string {} inserted", app.insert_new_string(v)?);
    }
    if get("strings_save").is_some() {
        eprintln!("strings saved: {:?}", app.save_strings()?);
    }
    if let Some(v) = get("print_string").and_then(|v| v.as_i64()) {
        eprintln!("string {v}: {:?}", app.string_text(v));
    }
    if get("print_caret_messages").is_some() {
        eprintln!("caret messages: {:?}", app.caret_messages());
    }
    if let Some(v) = get("save_format").and_then(|v| v.as_str()) {
        app.set_save_format(v == "legacy");
    }
    if let Some(v) = get("view_mode").and_then(|v| v.as_str()) {
        use crate::settings::ViewMode as M;
        app.set_view_mode(match v {
            "2d" => M::TwoD,
            "split" => M::Split,
            _ => M::ThreeD,
        });
    }
    if let Some(v) = get("flat") {
        app.settings.layout.flat_colors = b(v);
    }
    if let Some(v) = get("camera2") {
        let f: Vec<f32> = v
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_f64())
                    .map(|x| x as f32)
                    .collect()
            })
            .unwrap_or_default();
        if f.len() != 3 {
            return Err("camera2 needs [x, y, zoom]".into());
        }
        app.camera2 = crate::camera::Camera {
            target: glam::Vec3::new(f[0], 0.0, f[1]),
            zoom: f[2] / 100.0,
            yaw: 0.0,
            top_down: true,
            ..Default::default()
        };
    }
    if let Some(v) = get("graphics").and_then(|v| v.as_object()) {
        let g = &mut app.settings.graphics;
        if let Some(x) = v.get("overview_boost") {
            g.overview_boost = b(x);
        }
        if let Some(x) = v.get("local_lights") {
            g.local_lights = b(x);
        }
        if let Some(x) = v.get("declutter") {
            g.declutter = b(x);
        }
    }
    let _ = (hl, o, rel);
    Ok(())
}
