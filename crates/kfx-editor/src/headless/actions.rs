//! `--actions FILE.json`: the action loop (one JSON object per action) and the basic map,
//! file and camera actions; the other action groups live in the sibling modules.
use super::*;

pub(super) fn point(v: &serde_json::Value) -> Option<(i32, i32)> {
    let a = v.as_array()?;
    Some((a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32))
}

/// Run a JSON action script (see the usage text).
pub(super) fn run_actions(
    hl: &mut Headless,
    app: &mut EditorApp,
    o: &Options,
    file: &PathBuf,
) -> Result<(), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let list: Vec<serde_json::Value> =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    let base = file.parent().map(PathBuf::from).unwrap_or_default();
    let rel = |s: &str| {
        let p = PathBuf::from(s);
        if p.is_absolute() {
            p
        } else {
            base.join(p)
        }
    };
    for (n, a) in list.iter().enumerate() {
        let err = |m: String| format!("action {n} {a}: {m}");
        let obj = a.as_object().ok_or_else(|| err("not an object".into()))?;
        let get = |k: &str| obj.get(k);
        if let Some(v) = get("palette") {
            app.select_palette(v.as_str().unwrap_or("")).map_err(err)?;
        }
        if let Some(v) = get("owner").and_then(|v| v.as_u64()) {
            app.set_active_owner(v as u8);
        }
        if let Some(v) = get("tool") {
            let t = crate::tools::Tool::parse(v.as_str().unwrap_or(""))
                .ok_or_else(|| err("unknown tool".into()))?;
            app.set_tool(t);
        }
        if let Some(v) = get("brush").and_then(|v| v.as_u64()) {
            app.set_brush(
                v as u32,
                get("round").and_then(|r| r.as_bool()).unwrap_or(false),
            );
        }
        if let Some(v) = get("filled").and_then(|v| v.as_bool()) {
            app.set_shape_filled(v);
        }
        if let Some(v) = get("owner_only").and_then(|v| v.as_bool()) {
            app.set_owner_only(v);
        }
        if let Some(v) = get("border").and_then(|v| v.as_bool()) {
            app.set_editable_border(v);
        }
        if let Some(v) = get("view") {
            let b = |k: &str| v.get(k).and_then(|x| x.as_bool());
            app.set_view_flags(b("ownership"), b("grid"), b("lighting"));
        }
        if let Some(v) = get("camera") {
            let f: Vec<f32> = v
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_f64())
                        .map(|x| x as f32)
                        .collect()
                })
                .unwrap_or_default();
            if f.len() != 5 {
                return Err(err("camera needs [x, y, zoom, yaw, pitch]".into()));
            }
            app.set_camera_spec(CameraSpec {
                x: f[0],
                y: f[1],
                zoom_pct: f[2],
                yaw_deg: f[3],
                pitch_deg: f[4],
            });
        }
        if let Some(p) = get("hover").and_then(point) {
            app.hover_slab(p);
        }
        if let Some(p) = get("press").and_then(point) {
            app.hover_slab(p);
            app.tool_press(p, get("alt").and_then(|x| x.as_bool()).unwrap_or(false));
        }
        if let Some(v) = get("drag") {
            let pts: Vec<(i32, i32)> = match point(v) {
                Some(p) => vec![p],
                None => v
                    .as_array()
                    .map(|a| a.iter().filter_map(point).collect())
                    .unwrap_or_default(),
            };
            for p in pts {
                app.hover_slab(p);
                app.tool_drag(p);
            }
        }
        if let Some(p) = get("release").and_then(point) {
            app.hover_slab(p);
            app.tool_release(p);
        }
        if let Some(p) = get("click").and_then(point) {
            app.hover_slab(p);
            app.tool_press(p, get("alt").and_then(|x| x.as_bool()).unwrap_or(false));
            app.tool_release(p);
        }
        if let Some(v) = get("key").and_then(|v| v.as_str()) {
            match v.to_ascii_lowercase().as_str() {
                "escape" | "esc" => app.cancel_tool(),
                "r" => app.rotate_paste(),
                "f" => app.flip_paste(),
                "delete" | "del" => app.delete_selection(),
                other => return Err(err(format!("unknown key {other}"))),
            }
        }
        if let Some(v) = get("select") {
            let a = v
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or_else(|| err("select needs [[x0,y0],[x1,y1]]".into()))?;
            app.select_rect(
                point(&a[0]).ok_or_else(|| err("bad point".into()))?,
                point(&a[1]).ok_or_else(|| err("bad point".into()))?,
            );
        }
        if get("copy").is_some() {
            app.copy();
        }
        if get("cut").is_some() {
            app.cut();
        }
        if get("paste").is_some() {
            app.paste();
        }
        if let Some(v) = get("undo").and_then(|v| v.as_u64()) {
            for _ in 0..v {
                app.undo();
            }
        }
        if let Some(v) = get("redo").and_then(|v| v.as_u64()) {
            for _ in 0..v {
                app.redo();
            }
        }
        if get("undo_all").is_some() {
            app.undo_all();
        }
        if get("save").is_some() {
            app.save().map_err(err)?;
        }
        if let Some(v) = get("save_as") {
            let folder = v
                .get("folder")
                .and_then(|x| x.as_str())
                .ok_or_else(|| err("save_as needs folder".into()))?;
            let level = v
                .get("level")
                .and_then(|x| x.as_u64())
                .ok_or_else(|| err("save_as needs level".into()))?;
            app.save_as(&rel(folder), level as u32).map_err(err)?;
        }
        if let Some(v) = get("new") {
            let folder = v
                .get("folder")
                .and_then(|x| x.as_str())
                .ok_or_else(|| err("new needs folder".into()))?;
            let n = |k: &str, d: u64| v.get(k).and_then(|x| x.as_u64()).unwrap_or(d);
            let b = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(true);
            app.new_map(
                &rel(folder),
                n("level", 1) as u32,
                n("w", 85) as u32,
                n("h", 85) as u32,
                b("border"),
                b("earth"),
                n("texture", 0) as u8,
            )
            .map_err(err)?;
        }
        if let Some(v) = get("open").and_then(|v| v.as_str()) {
            app.open_map(&rel(v)).map_err(err)?;
        }
        if let Some(v) = get("screenshot").and_then(|v| v.as_str()) {
            let ui = get("ui")
                .and_then(|x| x.as_bool())
                .unwrap_or(!o.viewport_only);
            hl.capture(app, o, ui, &rel(v))?;
        }
        run_thing_and_map_actions(hl, app, o, obj, &rel).map_err(err)?;
        run_script_play_file_actions(hl, app, o, obj, &rel).map_err(err)?;
        run_config_and_view_actions(hl, app, o, obj, &rel).map_err(err)?;
        run_view_and_tool_actions(hl, app, o, obj, &rel).map_err(err)?;
        super::events::run_logic(app, obj).map_err(err)?;
        crate::app::prefab::run_actions(app, obj, &rel).map_err(err)?;
        crate::app::run_campaign_actions(app, obj, &rel).map_err(err)?;
        super::audiolive::run_audiolive(app, obj).map_err(err)?;
        app.run_analysis_actions(obj).map_err(err)?;
        crate::lang::run_actions(app, obj).map_err(err)?;
        let framed = run_record(hl, app, o, obj).map_err(err)?;
        if !framed && hl.rec.auto_frames > 0 {
            hl.steps(app, o, hl.rec.auto_frames).map_err(err)?;
        }
        if get("print").is_some() {
            let d = app.doc.as_ref();
            eprintln!(
                "[{n}] status: {} | dirty {} | history {} (at {}) | {}",
                app.status,
                d.is_some_and(|d| d.is_dirty()),
                d.map(|d| d.history().len()).unwrap_or(0),
                d.map(|d| d.cursor()).unwrap_or(0),
                d.map(|d| d
                    .history()
                    .iter()
                    .map(|c| c.name.clone())
                    .collect::<Vec<_>>()
                    .join(", "))
                    .unwrap_or_default()
            );
        }
    }
    Ok(())
}

pub(super) fn fpair(v: &serde_json::Value) -> Option<(f32, f32)> {
    let a = v.as_array()?;
    Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
}

/// Subtile centre to a viewport pixel of the full-size view (after rendering it once, so the
/// thing hit boxes match).
pub(super) fn stl_px(
    hl: &mut Headless,
    app: &mut EditorApp,
    o: &Options,
    p: (f32, f32),
) -> glam::Vec2 {
    let _ = hl;
    app.render_view(o.size);
    app.stl_to_px(p, (o.size.0 as f32, o.size.1 as f32).into())
}
