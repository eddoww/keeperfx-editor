//! Actions of the fly view, the grid data overlay, the UI scale, random doodads, the tool
//! windows, the script wizard, the placement options and the benchmarks.
use super::*;

/// Fly view, grid data overlay, doodads, save format, UI scale, windows, the script wizard,
/// placement options and benchmarks (see the usage text).
pub(super) fn run_view_and_tool_actions(
    hl: &mut Headless,
    app: &mut EditorApp,
    o: &Options,
    obj: &serde_json::Map<String, serde_json::Value>,
    rel: &dyn Fn(&str) -> PathBuf,
) -> Result<(), String> {
    let get = |k: &str| obj.get(k);
    let floats = |v: &serde_json::Value| -> Vec<f32> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_f64())
                    .map(|x| x as f32)
                    .collect()
            })
            .unwrap_or_default()
    };
    if let Some(v) = get("ui_scale").and_then(|v| v.as_f64()) {
        app.settings.ui.scale = v as f32;
        app.apply_ui_settings();
    }
    if let Some(v) = get("fly").and_then(|v| v.as_bool()) {
        if v != app.camera.fly {
            app.toggle_fly();
        }
    }
    if let Some(v) = get("fortify").and_then(|v| v.as_bool()) {
        app.fortify = v;
    }
    if let Some(v) = get("wall_art").and_then(|v| v.as_str()) {
        app.settings.wall_art = if v == "random" {
            crate::fortify::WallArt::Random
        } else {
            crate::fortify::WallArt::Grouped
        };
    }
    if let Some(v) = get("place_anywhere").and_then(|v| v.as_bool()) {
        app.settings.place_anywhere = v;
    }
    if let Some(v) = get("ownable_natural").and_then(|v| v.as_bool()) {
        app.settings.ownable_natural = v;
    }
    if get("door_lock").is_some() {
        app.toggle_door_lock();
    }
    if let Some(v) = get("fly_collision").and_then(|v| v.as_bool()) {
        app.settings.camera.fly_collision = v;
    }
    if let Some(v) = get("fly_look") {
        let f = floats(v);
        if f.len() != 2 {
            return Err("fly_look needs [yaw_deg, pitch_deg]".into());
        }
        app.camera.orbit(f[0].to_radians(), f[1].to_radians());
    }
    if let Some(v) = get("fly_move") {
        let f = floats(v);
        if f.len() != 3 {
            return Err("fly_move needs [right, up, forward]".into());
        }
        app.fly_move(glam::Vec3::new(f[0], f[1], f[2]));
    }
    if let Some(v) = get("grid_data").and_then(|v| v.as_str()) {
        app.settings.layout.grid_data = crate::griddata::GridData::parse(v)
            .ok_or_else(|| format!("unknown grid data {v:?}"))?;
    }
    if let Some(v) = get("doodads") {
        let set: crate::doodads::Doodads =
            serde_json::from_value(v.clone()).map_err(|e| format!("doodads: {e}"))?;
        let n = app.apply_doodads(set)?;
        eprintln!("doodads: {n} placed");
    }
    if let Some(v) = get("save_format").and_then(|v| v.as_str()) {
        app.set_save_format(v == "legacy");
    }
    // benchmarks (task: scale check on a 170x170 map with ~10k things)
    let stats = |name: &str, mut t: Vec<f64>| {
        if t.is_empty() {
            return;
        }
        t.sort_by(|a, b| a.total_cmp(b));
        let mean = t.iter().sum::<f64>() / t.len() as f64;
        eprintln!(
            "bench {name}: n {} mean {mean:.2} ms, median {:.2} ms, p95 {:.2} ms, max {:.2} ms",
            t.len(),
            t[t.len() / 2],
            t[(t.len() * 95 / 100).min(t.len() - 1)],
            t[t.len() - 1]
        );
    };
    if let Some(n) = get("bench_populate").and_then(|v| v.as_u64()) {
        let d = app.doc.as_mut().ok_or("no map")?;
        let dm = d.dims();
        let models: Vec<(u8, i64)> = {
            let gd = &d.gd;
            let mut m: Vec<(u8, i64)> = ["IMP", "TROLL", "DRAGON", "WIZARD", "KNIGHT"]
                .iter()
                .filter_map(|c| gd.creature_id(c))
                .map(|i| (kfx_map::things::class::CREATURE, i as i64))
                .collect();
            m.extend(
                ["GOLD", "BARREL", "TORCH", "CANDLESTICK", "POTION1"]
                    .iter()
                    .filter_map(|o| gd.object_id(o))
                    .map(|i| (kfx_map::things::class::OBJECT, i as i64)),
            );
            m
        };
        let t = Instant::now();
        let (_, rf) = d.edit("Populate", |m, _, _| {
            m.touch_things();
            let mut h: u64 = 0x1234_5678;
            for k in 0..n {
                h = h
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let x = 3 + (h >> 33) as usize % (dm.stl_w() - 6);
                let y = 3 + (h >> 13) as usize % (dm.stl_h() - 6);
                let (c, model) = models[k as usize % models.len()];
                m.things.items.push(kfx_map::things::new_thing(
                    c,
                    model,
                    (k % 6) as u8,
                    kfx_map::things::centred(x as i32),
                    kfx_map::things::centred(y as i32),
                    kfx_formats::level_fx::StlCoord::new(1, 0),
                ));
            }
        });
        app.after_change(rf);
        eprintln!(
            "bench populate: {n} things in {:.1} ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
    if let Some(v) = get("bench_stroke") {
        let from = v
            .get("from")
            .and_then(point)
            .ok_or("bench_stroke needs from")?;
        let steps = v.get("steps").and_then(|x| x.as_u64()).unwrap_or(60) as i32;
        let ui = v.get("ui").and_then(|x| x.as_bool()).unwrap_or(true);
        app.select_palette(v.get("palette").and_then(|x| x.as_str()).unwrap_or("PATH"))?;
        app.set_tool(crate::tools::Tool::Paint);
        app.set_brush(
            v.get("brush").and_then(|x| x.as_u64()).unwrap_or(3) as u32,
            false,
        );
        hl.frame_ui(app, o.size);
        app.hover_slab(from);
        app.tool_press(from, false);
        let (mut t_doc, mut t_all) = (vec![], vec![]);
        for k in 1..=steps {
            let p = (from.0 + k, from.1 + (k / 7) % 3);
            let t = Instant::now();
            app.hover_slab(p);
            app.tool_drag(p);
            t_doc.push(t.elapsed().as_secs_f64() * 1000.0);
            if ui {
                hl.frame_ui(app, o.size);
            }
            t_all.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let last = (from.0 + steps, from.1 + (steps / 7) % 3);
        app.tool_release(last);
        stats(
            "stroke edit (document + view refresh) per pointer move",
            t_doc,
        );
        if ui {
            stats("stroke with the UI frame per pointer move", t_all);
        }
    }
    if let Some(n) = get("bench_undo").and_then(|v| v.as_u64()) {
        // undo and redo the last step n times
        let (mut u, mut r) = (vec![], vec![]);
        for _ in 0..n {
            let t = Instant::now();
            app.undo();
            u.push(t.elapsed().as_secs_f64() * 1000.0);
            let t = Instant::now();
            app.redo();
            r.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        stats("undo", u);
        stats("redo", r);
    }
    if let Some(n) = get("bench_ui_frames").and_then(|v| v.as_u64()) {
        let mut t = vec![];
        for _ in 0..n {
            let s = Instant::now();
            hl.frame_ui(app, o.size);
            t.push(s.elapsed().as_secs_f64() * 1000.0);
        }
        let mode = format!(
            "{:?} UI frame {}x{} (zoom {:.0}%; last: things {:.2} ms, view encode \
            {:.2} ms, {} billboards)",
            app.settings.layout.view_mode,
            o.size.0,
            o.size.1,
            app.camera.zoom * 100.0,
            app.perf.things_ms,
            app.perf.frame_cpu_ms,
            app.perf.billboards
        );
        stats(&mode, t);
    }
    if let Some(v) = get("bench_save").and_then(|v| v.as_str()) {
        let t = Instant::now();
        let level = app.doc.as_ref().map(|d| d.doc.level).unwrap_or(1);
        app.save_as(&rel(v), level)?;
        eprintln!("bench save: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    }
    if get("print_mem").is_some() {
        let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let m: Vec<&str> = s
            .lines()
            .filter(|l| l.starts_with("VmRSS") || l.starts_with("VmHWM"))
            .collect();
        eprintln!(
            "memory: {}",
            m.join(", ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    // the script wizard: {"wizard_set": {"player": 0, "start_money": n, "max_creatures": n,
    // "win": true, "lose": true, "rooms": {"TREASURE": "yes|research|no"}, "generate_speed": n}}
    if let Some(v) = get("wizard_set").and_then(|v| v.as_object()) {
        use kfx_script_ui::wizard::{Availability, Player};
        let ws = app.script_ws().ok_or("no scripts")?;
        let w = ws
            .wizard_mut()
            .ok_or("the wizard is not open (open_window: wizard)")?;
        let m = w.model_mut();
        if let Some(n) = v.get("generate_speed").and_then(|x| x.as_i64()) {
            m.generate_speed = Some(n);
        }
        let p = v.get("player").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let pl = *Player::ALL.get(p).ok_or("player 0..7")?;
        let ps = m.player_mut(pl);
        if let Some(n) = v.get("start_money").and_then(|x| x.as_i64()) {
            ps.start_money = Some(n);
        }
        if let Some(n) = v.get("max_creatures").and_then(|x| x.as_i64()) {
            ps.max_creatures = Some(n);
        }
        if let Some(b) = v.get("win").and_then(|x| x.as_bool()) {
            ps.win_when_enemies_destroyed = b;
        }
        if let Some(b) = v.get("lose").and_then(|x| x.as_bool()) {
            ps.lose_when_heart_destroyed = b;
        }
        if let Some(r) = v.get("rooms").and_then(|x| x.as_object()) {
            for (k, a) in r {
                let a = match a.as_str().unwrap_or("") {
                    "yes" => Availability::Available,
                    "research" => Availability::Researchable,
                    _ => Availability::Unavailable,
                };
                ps.rooms.insert(k.clone(), a);
            }
        }
    }
    if get("wizard_apply").is_some() {
        let ws = app.script_ws().ok_or("no scripts")?;
        let plan = ws.apply_wizard().ok_or("the wizard is not open")?;
        for c in &plan.conflicts {
            eprintln!("wizard conflict line {}: {}", c.line, c.message);
        }
        for n in &plan.notes {
            eprintln!("wizard note: {n}");
        }
    }
    if let Some(v) = get("open_window").and_then(|v| v.as_str()) {
        match v {
            "doodads" => app.open_doodad_dialog(),
            "stats" => app.open_creature_stats(),
            "wizard" => app.open_script_wizard(kfx_script_ui::wizard::ScriptLang::Dk),
            "lua_wizard" => app.open_script_wizard(kfx_script_ui::wizard::ScriptLang::Lua),
            other => return Err(format!("unknown window {other:?}")),
        }
    }
    Ok(())
}
