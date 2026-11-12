//! Actions of the things and map tools: placing, selecting and editing things, resize,
//! transform, symmetry, regeneration, layers, dialogs.
use super::*;

/// The thing / map actions (see the usage text).
pub(super) fn run_thing_and_map_actions(
    hl: &mut Headless,
    app: &mut EditorApp,
    o: &Options,
    obj: &serde_json::Map<String, serde_json::Value>,
    rel: &dyn Fn(&str) -> PathBuf,
) -> Result<(), String> {
    let get = |k: &str| obj.get(k);
    let b = |v: &serde_json::Value| v.as_bool().unwrap_or(true);
    if let Some(v) = get("decoration") {
        app.set_decoration(b(v));
    }
    if let Some(v) = get("tab").and_then(|v| v.as_str()) {
        app.set_palette_tab(v)?;
    }
    if let Some(v) = get("right_tab").and_then(|v| v.as_str()) {
        app.settings.layout.show_history = true;
        app.settings.layout.right_tab = v.to_string();
    }
    if let Some(v) = get("layers").and_then(|v| v.as_object()) {
        for (k, val) in v {
            *app.settings
                .layout
                .layers
                .get_mut(k)
                .ok_or_else(|| format!("unknown layer {k}"))? = b(val);
        }
    }
    if let Some(v) = get("thing_palette").and_then(|v| v.as_str()) {
        app.select_thing_palette(v)?;
    }
    if let Some(v) = get("place_defaults").and_then(|v| v.as_object()) {
        let d = &mut app.place_defaults;
        for (k, val) in v {
            let i = || val.as_i64().unwrap_or(0);
            let f = || val.as_f64().unwrap_or(0.0) as f32;
            match k.as_str() {
                "level" => d.level = i().clamp(1, 10),
                "orientation" => d.orientation = i().rem_euclid(2048),
                "creature_gold" => d.creature_gold = i(),
                "health" => d.health = i(),
                "name" => d.name = val.as_str().unwrap_or("").to_string(),
                "gold_value" => d.gold_value = i(),
                "custom_box" => d.custom_box = i(),
                "effect_range" => d.effect_range = f(),
                "locked" => d.locked = b(val),
                "light_range" => d.light_range = f(),
                "light_intensity" => d.light_intensity = i(),
                "light_height" => d.light_height = f(),
                "ap_range" => d.ap_range = f(),
                _ => return Err(format!("unknown placement default {k}")),
            }
        }
    }
    if let Some(v) = get("free_place") {
        app.set_free_place(b(v));
    }
    if let Some(p) = get("place").and_then(fpair) {
        let sub = get("sub").and_then(fpair).unwrap_or((128.0, 128.0));
        let new = app.place_thing_at((p.0 as i32, p.1 as i32), (sub.0 as i32, sub.1 as i32))?;
        app.selected = new;
    }
    if let Some(v) = get("select_things") {
        let a = v
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or("select_things needs [[x0,y0],[x1,y1]]")?;
        let (p, q) = (
            fpair(&a[0]).ok_or("bad point")?,
            fpair(&a[1]).ok_or("bad point")?,
        );
        app.select_items_in(
            (p.0 as i32, p.1 as i32),
            (q.0 as i32, q.1 as i32),
            get("add").is_some_and(b),
        );
    }
    if get("select_none").is_some() {
        app.selected.clear();
    }
    if let Some(v) = get("select_class").and_then(|v| v.as_str()) {
        app.select_class(v);
    }
    if let Some(v) = get("mouse").and_then(|v| v.as_object()) {
        // a real pointer gesture of the thing tools, at subtile centres on the floor
        let from = v
            .get("press")
            .and_then(fpair)
            .ok_or("mouse needs press [x,y]")?;
        let to = v.get("to").and_then(fpair).unwrap_or(from);
        let shift = v.get("shift").is_some_and(b);
        let alt = v.get("alt").is_some_and(b);
        let vp: glam::Vec2 = (o.size.0 as f32, o.size.1 as f32).into();
        let p0 = stl_px(hl, app, o, from);
        let p1 = app.stl_to_px(to, vp);
        app.mouse_things(p0, p1, vp, shift, alt);
    }
    if let Some(v) = get("move_things").and_then(fpair) {
        app.move_things((v.0 * 256.0).round() as i64, (v.1 * 256.0).round() as i64);
    }
    if get("delete_things").is_some() {
        app.delete_things();
    }
    if get("duplicate").is_some() {
        app.duplicate_things();
    }
    if let Some(n) = get("rotate_things").and_then(|v| v.as_i64()) {
        app.rotate_things(n);
    }
    if let Some(p) = get("paste_at").and_then(fpair) {
        app.hover_subtile((p.0 as usize, p.1 as usize));
        let slab = ((p.0 / 3.0) as i32, (p.1 / 3.0) as i32);
        app.tool_press(slab, false);
        app.tool_release(slab);
    }
    if let Some(v) = get("set").and_then(|v| v.as_object()) {
        for (k, val) in v {
            let s = match val {
                serde_json::Value::String(s) => s.clone(),
                x => x.to_string(),
            };
            let n = app.set_selection_field(k, &s)?;
            eprintln!("set {k} = {s} on {n} items");
        }
    }
    if let Some(v) = get("resize").and_then(|v| v.as_object()) {
        let n = |k: &str| v.get(k).and_then(|x| x.as_i64()).unwrap_or(0) as i32;
        app.resize_map(
            n("left"),
            n("top"),
            n("right"),
            n("bottom"),
            v.get("fill").and_then(|x| x.as_str()).unwrap_or("EARTH"),
        )?;
    }
    if let Some(v) = get("transform").and_then(|v| v.as_str()) {
        use kfx_map::transform::Transform as T;
        let t = match v.to_ascii_lowercase().as_str() {
            "flipx" | "flip_ew" => T::FlipX,
            "flipy" | "flip_ns" => T::FlipY,
            "rot90" => T::Rot90,
            "rot180" => T::Rot180,
            "rot270" => T::Rot270,
            x => return Err(format!("unknown transform {x}")),
        };
        app.transform(t, get("selection").is_some_and(b))?;
    }
    if let Some(v) = get("symmetry") {
        use kfx_map::transform::{Symmetry, SymmetryMode as M};
        let mode = match v.as_str().unwrap_or("off").to_ascii_lowercase().as_str() {
            "off" | "none" => None,
            "ns" | "north_south" => Some(M::NorthSouth),
            "ew" | "east_west" => Some(M::EastWest),
            "fourway" | "4way" | "four_way" => Some(M::FourWay),
            "rot2" => Some(M::Rot2),
            "rot4" => Some(M::Rot4),
            x => return Err(format!("unknown symmetry {x}")),
        };
        let mut s = mode.map(Symmetry::new);
        if let (Some(s), Some(rows)) = (s.as_mut(), get("sym_owners").and_then(|v| v.as_array())) {
            for (k, row) in rows.iter().enumerate() {
                if let (Some(dst), Some(r)) = (s.owners.get_mut(k), row.as_array()) {
                    for (j, o) in r.iter().enumerate().take(9) {
                        dst[j] = o.as_u64().unwrap_or(j as u64) as u8;
                    }
                }
            }
        }
        app.set_symmetry(s);
    }
    if let Some(v) = get("style") {
        app.set_style_brush(v.as_u64().map(|x| x as u8));
    }
    if let Some(v) = get("regenerate").and_then(|v| v.as_object()) {
        let f = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(true);
        let what = crate::document::Regen {
            columns: f("columns"),
            decoration: f("decoration"),
            objects: f("objects"),
            liquid: f("liquid"),
            doodads: f("doodads"),
        };
        app.regenerate(
            what,
            v.get("selection")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
        )?;
    }
    if let Some(v) = get("properties").and_then(|v| v.as_object()) {
        app.set_properties(v)?;
    }
    if let Some(v) = get("raw").and_then(|v| v.as_object()) {
        app.raw_edit(v)?;
    }
    if let Some(v) = get("open_dialog").and_then(|v| v.as_str()) {
        app.open_named_dialog(v)?;
    }
    if let Some(v) = get("frame") {
        let a = v
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or("frame needs [[x0,y0],[x1,y1]] (subtiles)")?;
        let (p, q) = (
            fpair(&a[0]).ok_or("bad point")?,
            fpair(&a[1]).ok_or("bad point")?,
        );
        // the view size of the next capture: the full image, or the UI's viewport
        let ui = get("ui")
            .and_then(|x| x.as_bool())
            .unwrap_or(!o.viewport_only);
        if ui {
            hl.capture_ui(app, o.size, None)?;
        } else {
            app.render_view(o.size);
        }
        app.frame_subtiles(p, q);
    }
    if let Some(p) = get("focus").and_then(fpair) {
        app.camera.target = glam::Vec3::new(p.0, app.camera.target.y, p.1);
    }
    if get("validate").is_some() {
        app.revalidate();
        app.settings.layout.show_history = true;
        app.settings.layout.right_tab = "validate".into();
        for i in &app.issues {
            eprintln!("  {:?} [{}] {}", i.severity, i.code, i.message);
        }
    }
    if get("list_things").is_some() {
        if let Some(d) = &app.doc {
            for (i, t) in d.doc.things.items.iter().enumerate() {
                eprintln!(
                    "  #{i} class {:?} model {:?} owner {:?} at ({},{}) z {} parent {:?} lvl \
                    {:?} orient {:?} {}",
                    t.class.as_ref().and_then(|c| c.id()),
                    t.subtype,
                    t.owner,
                    t.x.map_or(-1, |c| c.stl),
                    t.y.map_or(-1, |c| c.stl),
                    t.z.map_or(-1, |c| c.stl),
                    t.parent_tile,
                    t.creature_level,
                    t.orientation,
                    app.item_label(kfx_map::things::ItemRef::Thing(i))
                );
            }
        }
    }
    if get("print_things").is_some() {
        if let Some(d) = &app.doc {
            let sel: Vec<String> = app.selected.iter().map(|it| app.item_label(*it)).collect();
            eprintln!(
                "things {} aps {} lights {} | selected {}: {}",
                d.doc.things.items.len(),
                d.doc.action_points.items.len(),
                d.doc.lights.items.len(),
                sel.len(),
                sel.join(", ")
            );
        }
    }
    let _ = rel;
    Ok(())
}
