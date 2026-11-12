//! The recording actions (simulated pointer, keys and camera moves over several frames) and
//! their pointer targets (see `crate::record`).
use super::*;

/// A pointer target of the recording actions, in points (see `record.rs`).
pub(super) fn rec_target(
    hl: &Headless,
    app: &EditorApp,
    v: &serde_json::Value,
) -> Result<egui::Pos2, String> {
    let zoom = hl.ctx.zoom_factor();
    if let Some((x, y)) = fpair(v) {
        return Ok(egui::pos2(x / zoom, y / zoom));
    }
    let o = v
        .as_object()
        .ok_or("a target is [x, y] pixels, {slab}, {stl}, {text} or {word}")?;
    let d = egui::vec2(
        o.get("dx").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        o.get("dy").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
    ) / zoom;
    let two_d = o.get("two_d").and_then(|x| x.as_bool()).unwrap_or(false);
    if let Some((x, y)) = o.get("slab").and_then(fpair) {
        return app
            .stl_screen_pos((x * 3.0 + 1.0, y * 3.0 + 1.0), two_d)
            .map(|p| p + d)
            .ok_or_else(|| format!("slab {x},{y} is not on screen"));
    }
    if let Some((x, y)) = o.get("stl").and_then(fpair) {
        return app
            .stl_screen_pos((x, y), two_d)
            .map(|p| p + d)
            .ok_or_else(|| format!("subtile {x},{y} is not on screen"));
    }
    if let Some(t) = o.get("text").and_then(|x| x.as_str()) {
        let nth = o.get("nth").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let within = o.get("in").and_then(|x| x.as_array()).and_then(|a| {
            let f: Vec<f32> = a
                .iter()
                .filter_map(|x| x.as_f64())
                .map(|x| x as f32 / zoom)
                .collect();
            (f.len() == 4)
                .then(|| egui::Rect::from_min_max(egui::pos2(f[0], f[1]), egui::pos2(f[2], f[3])))
        });
        return crate::record::find_text(&hl.rec.texts, t, nth, within)
            .map(|r| r.center() + d)
            .ok_or_else(|| format!("no text {t:?} (nth {nth}) on the last frame"));
    }
    if let Some(t) = o.get("word").and_then(|x| x.as_str()) {
        let nth = o.get("nth").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        let within = o.get("in").and_then(|x| x.as_array()).and_then(|a| {
            let f: Vec<f32> = a
                .iter()
                .filter_map(|x| x.as_f64())
                .map(|x| x as f32 / zoom)
                .collect();
            (f.len() == 4)
                .then(|| egui::Rect::from_min_max(egui::pos2(f[0], f[1]), egui::pos2(f[2], f[3])))
        });
        return crate::record::find_word(&hl.rec.texts, t, nth, within)
            .map(|r| r.center() + d)
            .ok_or_else(|| format!("no word {t:?} (nth {nth}) on the last frame"));
    }
    Err(format!("bad target {v}"))
}

/// The recording actions (see `record.rs`). Returns whether the action rendered frames.
pub(super) fn run_record(
    hl: &mut Headless,
    app: &mut EditorApp,
    o: &Options,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<bool, String> {
    use crate::record::{ease, frames_for, parse_button, parse_keys};
    let get = |k: &str| obj.get(k);
    let fps = hl.rec.fps;
    let secs = |v: &serde_json::Value, d: f32| {
        v.get("secs")
            .and_then(|x| x.as_f64())
            .map(|x| x as f32)
            .unwrap_or(d)
    };
    let ease_of = |v: &serde_json::Value| {
        v.get("ease")
            .and_then(|x| x.as_str())
            .unwrap_or("inout")
            .to_string()
    };
    let mut framed = false;
    if let Some(v) = get("auto_frames").and_then(|v| v.as_u64()) {
        hl.rec.auto_frames = v as u32;
    }
    if let Some(v) = get("stride").and_then(|v| v.as_u64()) {
        hl.rec.stride = (v as u32).max(1);
    }
    if let Some(v) = get("cursor").and_then(|v| v.as_bool()) {
        hl.rec.cursor = v;
    }
    if let Some(v) = get("view_only").and_then(|v| v.as_bool()) {
        hl.rec.view_only = v;
    }
    if let Some(v) = get("anim").and_then(|v| v.as_bool()) {
        hl.rec.anim = v;
        if !v {
            app.fixed_frame = Some(o.frame % 8);
        }
    }
    if let Some(v) = get("caption") {
        hl.rec.start_caption(v.as_str());
    }
    if let Some(v) = get("move") {
        let to = rec_target(hl, app, v.get("to").unwrap_or(v))?;
        hl.glide(app, o, to, frames_for(secs(v, 0.6), fps), &ease_of(v))?;
        framed = true;
    }
    if let Some(v) = get("tap") {
        let (button, count) = match v.as_str() {
            Some(b) => (b.to_string(), 1),
            None => (
                v.get("button")
                    .and_then(|x| x.as_str())
                    .unwrap_or("left")
                    .to_string(),
                v.get("count").and_then(|x| x.as_u64()).unwrap_or(1),
            ),
        };
        if let Some(t) = v.get("to") {
            let to = rec_target(hl, app, t)?;
            hl.glide(app, o, to, frames_for(secs(v, 0.6), fps), &ease_of(v))?;
        }
        let b = parse_button(&button)?;
        for _ in 0..count {
            let e = hl.button_event(b, true);
            hl.step(app, o, vec![e])?;
            let e = hl.button_event(b, false);
            hl.step(app, o, vec![e])?;
        }
        hl.steps(app, o, frames_for(0.15, fps))?;
        framed = true;
    }
    if let Some(v) = get("down").and_then(|v| v.as_str()) {
        let e = hl.button_event(parse_button(v)?, true);
        hl.step(app, o, vec![e])?;
        framed = true;
    }
    if let Some(v) = get("up").and_then(|v| v.as_str()) {
        let e = hl.button_event(parse_button(v)?, false);
        hl.step(app, o, vec![e])?;
        framed = true;
    }
    if let Some(v) = get("pdrag") {
        let b = parse_button(v.get("button").and_then(|x| x.as_str()).unwrap_or("left"))?;
        let to = rec_target(hl, app, v.get("to").ok_or("pdrag needs to")?)?;
        let e = hl.button_event(b, true);
        hl.step(app, o, vec![e])?;
        hl.glide(
            app,
            o,
            to,
            frames_for(secs(&v.clone(), 0.8), fps),
            &ease_of(v),
        )?;
        let e = hl.button_event(b, false);
        hl.step(app, o, vec![e])?;
        framed = true;
    }
    if let Some(v) = get("stroke") {
        let b = parse_button(v.get("button").and_then(|x| x.as_str()).unwrap_or("left"))?;
        let two_d = v.get("two_d").and_then(|x| x.as_bool()).unwrap_or(false);
        let slabs: Vec<(f32, f32)> = v
            .get("slabs")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(fpair).collect())
            .unwrap_or_default();
        if slabs.is_empty() {
            return Err("stroke needs slabs".into());
        }
        let mut pts = vec![];
        for (x, y) in &slabs {
            pts.push(
                app.stl_screen_pos((x * 3.0 + 1.0, y * 3.0 + 1.0), two_d)
                    .ok_or_else(|| format!("slab {x},{y} is not on screen"))?,
            );
        }
        let lead = v.get("lead").and_then(|x| x.as_f64()).unwrap_or(0.4) as f32;
        if lead > 0.0 {
            hl.glide(app, o, pts[0], frames_for(lead, fps), "inout")?;
        }
        let shift = v.get("shift").and_then(|x| x.as_bool()).unwrap_or(false);
        hl.rec.modifiers.shift = shift;
        let e = hl.button_event(b, true);
        hl.step(app, o, vec![e])?;
        let n = frames_for(secs(v, 1.5), fps);
        let kind = v
            .get("ease")
            .and_then(|x| x.as_str())
            .unwrap_or("linear")
            .to_string();
        for i in 1..=n {
            hl.rec.pointer = crate::record::along(&pts, ease(&kind, i as f32 / n as f32));
            hl.step(app, o, vec![])?;
        }
        let e = hl.button_event(b, false);
        hl.step(app, o, vec![e])?;
        hl.rec.modifiers.shift = false;
        framed = true;
    }
    if let Some(v) = get("type") {
        let (text, cps) = match v.as_str() {
            Some(t) => (t.to_string(), 14.0),
            None => (
                v.get("text")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                v.get("cps").and_then(|x| x.as_f64()).unwrap_or(14.0) as f32,
            ),
        };
        let per = ((fps / cps.max(0.5)).round() as u32).max(1);
        for c in text.chars() {
            let m = hl.rec.modifiers;
            let ev = match c {
                '\n' => vec![
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: m,
                    },
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: m,
                    },
                ],
                '\t' => vec![
                    egui::Event::Key {
                        key: egui::Key::Tab,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: m,
                    },
                    egui::Event::Key {
                        key: egui::Key::Tab,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: m,
                    },
                ],
                c => vec![egui::Event::Text(c.to_string())],
            };
            hl.step(app, o, ev)?;
            hl.steps(app, o, per - 1)?;
        }
        framed = true;
    }
    if let Some(v) = get("keys") {
        let (k, times, gap) = match v.as_str() {
            Some(k) => (k.to_string(), 1, 0.25),
            None => (
                v.get("key")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                v.get("times").and_then(|x| x.as_u64()).unwrap_or(1),
                secs(v, 0.25),
            ),
        };
        let times = get("times").and_then(|x| x.as_u64()).unwrap_or(times);
        let gap = get("secs")
            .and_then(|x| x.as_f64())
            .map(|x| x as f32)
            .unwrap_or(gap);
        let (m, key) = parse_keys(&k)?;
        for _ in 0..times {
            hl.rec.modifiers = m;
            let down = egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: m,
            };
            hl.step(app, o, vec![down])?;
            let up = egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: false,
                repeat: false,
                modifiers: m,
            };
            hl.rec.modifiers = egui::Modifiers::NONE;
            hl.step(app, o, vec![up])?;
            hl.steps(app, o, frames_for(gap, fps).saturating_sub(2))?;
        }
        framed = true;
    }
    if let Some(v) = get("wheel") {
        let dy = v.get("dy").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
        let n = frames_for(secs(v, 0.6), fps);
        for _ in 0..n {
            let e = egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, dy / n as f32),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: hl.rec.modifiers,
            };
            hl.step(app, o, vec![e])?;
        }
        framed = true;
    }
    if let Some(v) = get("camera_to") {
        let f: Vec<f32> = v
            .get("cam")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_f64())
                    .map(|x| x as f32)
                    .collect()
            })
            .unwrap_or_default();
        if f.len() != 5 {
            return Err("camera_to needs cam [x, y, zoom, yaw, pitch]".into());
        }
        let c0 = app.camera;
        let (x1, z1, zoom1, yaw1, pitch1) = (
            f[0],
            f[1],
            f[2] / 100.0,
            f[3].to_radians(),
            f[4].to_radians()
                .clamp(-89f32.to_radians(), -5f32.to_radians()),
        );
        // the shorter way round
        let mut dyaw = (yaw1 - c0.yaw).rem_euclid(std::f32::consts::TAU);
        if dyaw > std::f32::consts::PI {
            dyaw -= std::f32::consts::TAU;
        }
        let n = frames_for(secs(v, 2.0), fps);
        let k = ease_of(v);
        for i in 1..=n {
            let t = ease(&k, i as f32 / n as f32);
            app.camera.target.x = c0.target.x + (x1 - c0.target.x) * t;
            app.camera.target.z = c0.target.z + (z1 - c0.target.z) * t;
            // zoom moves on a log scale, so that zooming in and out look even
            app.camera.zoom = (c0.zoom.ln() + (zoom1.ln() - c0.zoom.ln()) * t).exp();
            app.camera.yaw = c0.yaw + dyaw * t;
            app.camera.pitch = c0.pitch + (pitch1 - c0.pitch) * t;
            hl.step(app, o, vec![])?;
        }
        framed = true;
    }
    if let Some(v) = get("orbit") {
        let deg = v.get("deg").and_then(|x| x.as_f64()).unwrap_or(360.0) as f32;
        let n = frames_for(secs(v, 12.0), fps);
        let k = v
            .get("ease")
            .and_then(|x| x.as_str())
            .unwrap_or("linear")
            .to_string();
        let y0 = app.camera.yaw;
        for i in 1..=n {
            app.camera.yaw = y0 + deg.to_radians() * ease(&k, i as f32 / n as f32);
            hl.step(app, o, vec![])?;
        }
        framed = true;
    }
    if let Some(v) = get("fly_path") {
        let f3 = |k: &str| -> Vec<f32> {
            v.get(k)
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_f64())
                        .map(|x| x as f32)
                        .collect()
                })
                .unwrap_or_default()
        };
        let (mv, look) = (f3("move"), f3("look"));
        let mv = if mv.len() == 3 {
            glam::Vec3::new(mv[0], mv[1], mv[2])
        } else {
            glam::Vec3::ZERO
        };
        let look = if look.len() == 2 {
            (look[0], look[1])
        } else {
            (0.0, 0.0)
        };
        let n = frames_for(secs(v, 4.0), fps);
        let k = ease_of(v);
        let mut done = 0.0;
        for i in 1..=n {
            let t = ease(&k, i as f32 / n as f32);
            let dt = t - done;
            done = t;
            app.camera
                .orbit((look.0 * dt).to_radians(), (look.1 * dt).to_radians());
            app.fly_move(mv * dt);
            hl.step(app, o, vec![])?;
        }
        framed = true;
    }
    if let Some(v) = get("rec").and_then(|v| v.as_u64()) {
        hl.steps(app, o, v as u32)?;
        framed = true;
    }
    if let Some(v) = get("hold").and_then(|v| v.as_f64()) {
        // `"write": false`: let the editor run (a toast fades, a preview finishes) without
        // writing these frames
        let write = get("write").and_then(|x| x.as_bool()).unwrap_or(true);
        hl.rec.paused = !write;
        if get("realtime").and_then(|x| x.as_bool()).unwrap_or(false) {
            // wall-clock pacing: for things outside the editor that run in real time (a game
            // connected through the live link); each frame waits for its moment
            let start = Instant::now();
            for i in 0..frames_for(v as f32, fps) {
                let due = std::time::Duration::from_secs_f64(i as f64 / fps as f64);
                if let Some(w) = due.checked_sub(start.elapsed()) {
                    std::thread::sleep(w);
                }
                hl.step(app, o, vec![])?;
            }
        } else {
            hl.steps(app, o, frames_for(v as f32, fps))?;
        }
        hl.rec.paused = false;
        framed = true;
    }
    Ok(framed)
}
