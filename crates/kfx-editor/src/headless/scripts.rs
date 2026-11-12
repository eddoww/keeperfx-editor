//! Actions of the script workspace, play-testing, files, the map browser, the New map dialog,
//! image import / export and the preferences.
use super::*;

pub(super) fn pick_kind(s: &str) -> Option<kfx_script_ui::PickKind> {
    use kfx_script_ui::PickKind as K;
    Some(match s.to_ascii_lowercase().as_str() {
        "action_point" | "ap" => K::ActionPoint,
        "hero_gate" | "gate" => K::HeroGate,
        "location" => K::Location,
        "subtile" => K::Subtile,
        "slab" => K::Slab,
        _ => return None,
    })
}

/// Scripts, play-testing, files, browser, new map, image import / export, preferences (see
/// the usage text).
pub(super) fn run_script_play_file_actions(
    hl: &mut Headless,
    app: &mut EditorApp,
    o: &Options,
    obj: &serde_json::Map<String, serde_json::Value>,
    rel: &dyn Fn(&str) -> PathBuf,
) -> Result<(), String> {
    use crate::settings::ScriptDock;
    let get = |k: &str| obj.get(k);
    let b = |v: &serde_json::Value| v.as_bool().unwrap_or(true);
    let s = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
    if let Some(v) = get("pointer") {
        hl.pointer = fpair(v);
    }
    if let Some(v) = get("script").and_then(|v| v.as_str()) {
        match v {
            "docked" => {
                app.settings.script_dock = ScriptDock::Docked;
                app.set_script_max(false);
            }
            "window" => {
                app.settings.script_dock = ScriptDock::Window;
                app.set_script_max(false);
            }
            "max" => app.set_script_max(true),
            "hidden" => {
                app.settings.script_dock = ScriptDock::Hidden;
                app.set_script_max(false);
            }
            x => return Err(format!("unknown script mode {x}")),
        }
    }
    if let Some(v) = get("script_width").and_then(|v| v.as_f64()) {
        app.settings.script_panel_w = v as f32;
    }
    if let Some(v) = get("script_create").and_then(|v| v.as_str()) {
        let ws = app.script_ws().ok_or("no scripts")?;
        match v {
            "dk" => ws.create_dk_script(),
            "lua" => ws.create_lua_script(get("hints").is_some_and(b), true),
            x => return Err(format!("unknown script kind {x}")),
        }
    }
    if let Some(v) = get("script_tab").and_then(|v| v.as_str()) {
        let ws = app.script_ws().ok_or("no scripts")?;
        if !ws.activate(v) {
            return Err(format!("no script tab {v}"));
        }
    }
    if let Some(v) = get("script_goto").and_then(|v| v.as_u64()) {
        let ws = app.script_ws().ok_or("no scripts")?;
        ws.active_editor_mut().goto_line(v as usize);
    }
    if let Some(v) = get("script_caret").and_then(fpair) {
        // 1-based line and column
        let ws = app.script_ws().ok_or("no scripts")?;
        let e = ws.active_editor_mut();
        let bytes = e.text_bytes();
        let mut off = 0usize;
        for _ in 1..(v.0 as usize) {
            off = bytes[off..]
                .iter()
                .position(|&c| c == b'\n')
                .map(|i| off + i + 1)
                .unwrap_or(bytes.len());
        }
        let off = (off + (v.1 as usize).saturating_sub(1)).min(bytes.len());
        e.select_span(kfx_script::Span::new(off, off));
    }
    if let Some(v) = get("script_end").filter(|v| b(v)) {
        let _ = v;
        let ws = app.script_ws().ok_or("no scripts")?;
        let e = ws.active_editor_mut();
        let n = e.text_bytes().len();
        e.select_span(kfx_script::Span::new(n, n));
    }
    if let Some(v) = get("script_text").and_then(|v| v.as_str()) {
        let ws = app.script_ws().ok_or("no scripts")?;
        let e = ws.active_editor_mut();
        e.insert_at_caret(v);
        e.apply_pending();
    }
    if get("script_complete").is_some() {
        // a frame first, so that the editor has seen the caret move of the typed text
        hl.capture_ui(app, o.size, None)?;
        let consts = app.script_consts().cloned().ok_or("no scripts")?;
        let ws = app.script_ws().ok_or("no scripts")?;
        let e = ws.active_editor_mut();
        e.refresh_completions(&consts);
        eprintln!(
            "completions: {}",
            e.completion_labels()
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if get("script_analyze").is_some() {
        let consts = app.script_consts().cloned().ok_or("no scripts")?;
        let ws = app.script_ws().ok_or("no scripts")?;
        ws.analyze_now(&consts);
        for p in ws.problems() {
            eprintln!(
                "  {}:{} {:?} {} [{}]",
                p.tab,
                p.diagnostic.line,
                p.diagnostic.severity,
                p.diagnostic.message,
                p.diagnostic.code
            );
        }
    }
    if get("script_save").is_some() {
        app.do_save_public();
    }
    if let Some(v) = get("script_pick").and_then(|v| v.as_str()) {
        app.start_pick(pick_kind(v).ok_or_else(|| format!("unknown pick kind {v}"))?);
    }
    if let Some(n) = get("pick_ap").and_then(|v| v.as_i64()) {
        let stl = app
            .doc
            .as_ref()
            .and_then(|d| {
                d.doc
                    .action_points
                    .items
                    .iter()
                    .find(|a| a.number == Some(n))
            })
            .and_then(|a| Some((a.x?.stl, a.y?.stl)))
            .ok_or_else(|| format!("no action point {n}"))?;
        eprintln!("pick at action point {n}: subtile {stl:?}");
        app.pick_subtile(stl);
    }
    if let Some(p) = get("pick").and_then(fpair) {
        app.pick_subtile((p.0 as i32, p.1 as i32));
    }
    if let Some(v) = get("jump").and_then(|v| v.as_str()) {
        use kfx_script_ui::MapTarget as T;
        let (k, n) = v
            .split_once(':')
            .ok_or("jump needs ap:N, gate:N, stl:X,Y or slab:X,Y")?;
        let num = |t: &str| {
            t.trim()
                .parse::<i32>()
                .map_err(|_| format!("bad number {t}"))
        };
        let pair = |t: &str| -> Result<(i32, i32), String> {
            let (a, b) = t.split_once(',').ok_or("x,y")?;
            Ok((num(a)?, num(b)?))
        };
        let t = match k {
            "ap" => T::ActionPoint(num(n)?),
            "gate" => T::HeroGate(num(n)?),
            "stl" => {
                let (x, y) = pair(n)?;
                T::Subtile { x, y }
            }
            "slab" => {
                let (x, y) = pair(n)?;
                T::Slab { x, y }
            }
            x => return Err(format!("unknown jump target {x}")),
        };
        app.jump_to_target(t);
    }
    if let Some(v) = get("answer_scripts") {
        let n = app.answer_number_prompt(b(v));
        eprintln!("script references updated: {n}");
    }
    if get("script_print").is_some() {
        let ws = app.script_ws().ok_or("no scripts")?;
        for t in ws.tab_infos() {
            eprintln!(
                "  tab {} exists {} dirty {} errors {} warnings {}",
                t.name, t.exists, t.dirty, t.errors, t.warnings
            );
        }
        for r in ws.map_references() {
            eprintln!(
                "  ref {} line {}: {:?} ({})",
                r.tab, r.reference.line, r.reference.target, r.reference.command
            );
        }
    }
    if let Some(v) = get("game_exe").and_then(|v| v.as_str()) {
        app.settings.play.exe = Some(rel(v));
    }
    if get("game_log").is_some() {
        app.refresh_game_log();
        app.game_log.open = true;
        for e in app.game_log.entries.iter().filter(|e| e.script.is_some()) {
            eprintln!("  log {:?} {:?}: {}", e.level, e.script, e.text);
        }
    }
    if get("print_markers").is_some() {
        for (r, t) in app.marker_rects() {
            eprintln!("  marker at ({:.0},{:.0}): {t}", r.center().x, r.center().y);
        }
    }
    if let Some(n) = get("game_log_go").and_then(|v| v.as_u64()) {
        app.game_log_go(n as usize)?;
    }
    if get("print_launch").is_some() {
        match app.play_launch() {
            Ok((l, _)) => eprintln!("launch: {}", l.display()),
            Err(e) => eprintln!("launch: ({e})"),
        }
    }
    if let Some(v) = get("browser").and_then(|v| v.as_object()) {
        app.show_browser(true, v.get("filter").and_then(|x| x.as_str()).unwrap_or(""));
        match v.get("select").and_then(|x| x.as_str()) {
            Some("random") => {
                app.browser_random();
            }
            Some(p) => app.browser_select(rel(p)),
            None => {}
        }
    }
    if let Some(v) = get("new_dialog").and_then(|v| v.as_object()) {
        app.open_named_dialog("new")?;
        let spec = app.new_dialog_spec().ok_or("no dialog")?;
        apply_spec(spec, v)?;
    }
    if let Some(v) = get("new_map").and_then(|v| v.as_object()) {
        let mut spec = crate::newmap::NewMapSpec::default();
        apply_spec(&mut spec, v)?;
        let folder = v
            .get("folder")
            .and_then(|x| x.as_str())
            .ok_or("new_map needs folder")?;
        let level = v.get("level").and_then(|x| x.as_u64()).unwrap_or(1) as u32;
        app.new_map_spec(&rel(folder), level, &spec)?;
    }
    if let Some(v) = get("image_dialog").and_then(|v| v.as_str()) {
        app.open_image_dialog(Some(&rel(v)));
    }
    if get("image_apply").is_some() {
        app.apply_image()?;
        app.close_named_dialog("image");
    }
    if let Some(v) = get("export_image").and_then(|v| v.as_object()) {
        let out = v
            .get("out")
            .and_then(|x| x.as_str())
            .ok_or("export_image needs out")?;
        let iso = v.get("iso").and_then(|x| x.as_bool()).unwrap_or(true);
        let scale = v.get("scale").and_then(|x| x.as_u64()).unwrap_or(16) as u32;
        let things = v.get("things").and_then(|x| x.as_bool()).unwrap_or(true);
        let (w, h, t) = app.export_image(&rel(out), iso, scale, things, false, false)?;
        eprintln!("exported {out}: {w} x {h} in {t} tiles");
    }
    if let Some(v) = get("prefs_tab") {
        app.open_named_dialog("preferences")?;
        app.set_prefs_tab(&s(v));
    }
    if get("autosave").is_some() {
        let d = app.write_snapshot()?;
        eprintln!("snapshot: {}", d.display());
    }
    if get("recover").is_some() {
        let root = crate::settings::Settings::autosave_dir().ok_or("no config dir")?;
        let snap = crate::files::list_snapshots(&root)
            .into_iter()
            .next()
            .ok_or("no snapshot")?;
        app.recover(&snap)?;
    }
    if get("show_recovery").is_some() {
        app.show_recovery_prompt();
    }
    if get("check_external").is_some() {
        app.check_external_changes();
    }
    let _ = o;
    Ok(())
}

pub(super) fn apply_spec(
    spec: &mut crate::newmap::NewMapSpec,
    v: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    use kfx_map::transform::SymmetryMode as M;
    for (k, val) in v {
        let u = || val.as_u64().unwrap_or(0);
        match k.as_str() {
            "w" => spec.w = u() as u32,
            "h" => spec.h = u() as u32,
            "border" => spec.border_rock = val.as_bool().unwrap_or(true),
            "fill" => spec.fill = val.as_str().unwrap_or("EARTH").to_string(),
            "texture" => spec.texture = u() as u8,
            "generate" => spec.generate = val.as_bool().unwrap_or(true),
            "seed" => spec.seed = u(),
            "players" => spec.players = u() as u8,
            "kind" => {
                spec.kind = kfx_formats::levelinfo::LevelKind::from_name(val.as_str().unwrap_or(""))
            }
            "name" => spec.name = val.as_str().unwrap_or("").to_string(),
            "imps" => spec.imps = u() as u8,
            "portals" => spec.portals = val.as_bool().unwrap_or(true),
            "gold_near" => spec.gold_near = val.as_bool().unwrap_or(true),
            "hero_gates" => spec.hero_gates = u() as u8,
            "script" => {
                spec.script = match val.as_str().unwrap_or("none") {
                    "dk" => crate::newmap::ScriptChoice::Dk,
                    "lua" => crate::newmap::ScriptChoice::Lua,
                    _ => crate::newmap::ScriptChoice::None,
                }
            }
            "symmetry" => {
                spec.symmetry = match val.as_str().unwrap_or("none") {
                    "ns" => Some(M::NorthSouth),
                    "ew" => Some(M::EastWest),
                    "fourway" => Some(M::FourWay),
                    "rot2" => Some(M::Rot2),
                    "rot4" => Some(M::Rot4),
                    _ => None,
                }
            }
            "folder" | "level" => {}
            x => {
                if !crate::newmap::apply_gen_key(spec, x, val)? {
                    return Err(format!("unknown new map field {x}"));
                }
            }
        }
    }
    Ok(())
}
