//! Direct access to editor state and dialogs for the headless actions (`--actions`) and the UI
//! tests: palettes, thing selection, screen positions, properties, named dialogs, framing and
//! raw edits.
use super::panels::{RegenDialog, ResizeDialog};
use super::*;
use crate::document::Regen;
use kfx_formats::levelinfo::LevelKind;
use kfx_map::things::{class, thing_class, ItemRef};
use kfx_prefab::i18n::{trd, trf};

impl EditorApp {
    pub fn set_decoration(&mut self, on: bool) {
        self.settings.decoration = on;
        self.apply_decor_prefs();
    }

    pub fn set_palette_tab(&mut self, name: &str) -> Result<(), String> {
        self.category =
            Category::parse(name).ok_or_else(|| format!("unknown palette tab {name}"))?;
        Ok(())
    }

    pub fn set_free_place(&mut self, on: bool) {
        self.free_place = on;
    }

    pub fn set_style_brush(&mut self, v: Option<u8>) {
        self.style_brush = v;
    }

    /// Select every visible thing of a class (`creatures`, `objects`, ...), action points or
    /// lights.
    pub fn select_class(&mut self, name: &str) {
        let Some(doc) = &self.doc else { return };
        let want = match name {
            "creatures" => Some(class::CREATURE),
            "objects" => Some(class::OBJECT),
            "traps" => Some(class::TRAP),
            "doors" => Some(class::DOOR),
            "effects" => Some(class::EFFECT_GEN),
            _ => None,
        };
        self.selected.clear();
        match name {
            "action_points" => self
                .selected
                .extend((0..doc.doc.action_points.items.len()).map(ItemRef::ActionPoint)),
            "lights" => self
                .selected
                .extend((0..doc.doc.lights.items.len()).map(ItemRef::Light)),
            _ => {
                for (i, t) in doc.doc.things.items.iter().enumerate() {
                    if thing_class(t) == want && t.parent_tile.is_none() {
                        self.selected.insert(ItemRef::Thing(i));
                    }
                }
            }
        }
    }

    /// Pixel of a subtile position on the floor in a viewport of size `vp`.
    pub fn stl_to_px(&self, p: (f32, f32), vp: Vec2) -> Vec2 {
        let h = self
            .doc
            .as_ref()
            .map_or(0, |d| d.doc.floor_height(p.0 as i32, p.1 as i32)) as f32;
        self.camera
            .project(Vec3::new(p.0 + 0.5, h, p.1 + 0.5), vp)
            .unwrap_or(Vec2::ZERO)
    }

    /// Screen position (UI points) of a subtile position on the floor, in the pane of the 3D
    /// view or (`two_d`) of the 2D view, from the layout of the last frame; `None` when that
    /// view is not shown or the point is not inside its pane. Used by the headless recorder to
    /// move its pointer onto map positions.
    pub fn stl_screen_pos(&self, p: (f32, f32), two_d: bool) -> Option<egui::Pos2> {
        use crate::settings::ViewMode as M;
        let mode = if self.doc.is_some() {
            self.settings.layout.view_mode
        } else {
            M::ThreeD
        };
        let rect = match (mode, two_d) {
            (M::ThreeD, false) | (M::TwoD, true) => *self.panes.first()?,
            (M::Split, true) => *self.panes.first()?,
            (M::Split, false) => *self.panes.get(1)?,
            _ => return None,
        };
        let (cam, vp) = if two_d {
            (&self.camera2, self.last_vp2)
        } else {
            (&self.camera, self.last_vp)
        };
        let h = self
            .doc
            .as_ref()
            .map_or(0, |d| d.doc.floor_height(p.0 as i32, p.1 as i32)) as f32;
        let px = cam.project(Vec3::new(p.0 + 0.5, h, p.1 + 0.5), vp)?;
        let ppp = self.ctx.pixels_per_point();
        Some(rect.min + egui::vec2(px.x / ppp, px.y / ppp)).filter(|p| rect.contains(*p))
    }

    /// A pointer gesture of the thing tools: press at `a`, drag to `b`, release.
    pub fn mouse_things(&mut self, a: Vec2, b: Vec2, vp: Vec2, shift: bool, alt: bool) {
        self.things_press(a, vp, shift, alt);
        let n = 8;
        for k in 1..=n {
            let p = a + (b - a) * (k as f32 / n as f32);
            self.things_drag(p, vp);
        }
        self.things_release();
    }

    /// Hover a subtile (headless).
    pub fn hover_subtile(&mut self, p: (usize, usize)) {
        let h = self.terrain.top_height(p.0, p.1) as f32;
        self.hover = Some(HoverInfo {
            hit: Hit {
                stl_x: p.0,
                stl_y: p.1,
                point: Vec3::new(p.0 as f32 + 0.5, h, p.1 as f32 + 0.5),
                height: h,
            },
            slab: (p.0 / 3, p.1 / 3),
        });
    }

    /// Level properties from a JSON object: name, author, description, kind, players, ensign,
    /// texture (tmapa index); written like the dialog's Apply.
    pub fn set_properties(
        &mut self,
        v: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.open_properties();
        let mut d = self.props_dialog.take().ok_or("no map")?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        if let Some(x) = s("name") {
            d.info.name_text = Some(x);
        }
        if let Some(x) = s("author") {
            d.info.author = Some(x);
        }
        if let Some(x) = s("description") {
            d.info.description = Some(x);
        }
        if let Some(x) = s("ensign") {
            d.info.ensign = Some(x);
        }
        if let Some(x) = s("kind") {
            d.info.kinds = x
                .split_whitespace()
                .filter_map(LevelKind::from_name)
                .collect();
        }
        if let Some(x) = v.get("players").and_then(|x| x.as_i64()) {
            d.info.players = Some(x as i32);
        }
        if let Some(x) = v.get("texture").and_then(|x| x.as_u64()) {
            d.texture = x as u8;
        }
        if v.get("show").is_some_and(|x| x.as_bool() == Some(true)) {
            self.props_dialog = Some(d);
            return Ok(());
        }
        self.apply_properties(&d.info, d.texture, d.target.as_deref())
    }

    pub fn open_named_dialog(&mut self, name: &str) -> Result<(), String> {
        match name {
            "properties" => self.open_properties(),
            "resize" => {
                self.resize_dialog = Some(ResizeDialog {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                    fill: "EARTH".into(),
                })
            }
            "regenerate" => {
                self.regen_dialog = Some(RegenDialog {
                    what: Regen::default(),
                    selection: false,
                    confirm: false,
                })
            }
            "preferences" => self.show_prefs = true,
            "symmetry" => self.show_symmetry = true,
            "new" => self.open_new_dialog(),
            "shortcuts" => self.show_shortcuts = true,
            "backups" => self.open_backups(),
            "game_log" => {
                self.refresh_game_log();
                self.game_log.open = true;
            }
            "controls" => self.show_controls = true,
            "about" => self.show_about = true,
            "export" => self.open_export_dialog(),
            "image" => self.open_image_dialog(None),
            "browser" => self.browser.open = true,
            "config" => self.open_config_editor(None),
            "layers" => self.show_layers = true,
            "strings" => self.show_strings = true,
            "custom" => self.open_custom_dialog(),
            "columns" => self.open_columns_window(),
            "landview" => {
                self.open_properties();
                self.open_land_picker();
            }
            "column" => {
                let stl = self
                    .hover
                    .as_ref()
                    .map(|h| (h.hit.stl_x, h.hit.stl_y))
                    .ok_or("hover a subtile first")?;
                self.open_column_dialog(stl);
            }
            "note" => self.open_note_dialog(),
            "decoration" => self.open_decor_dialog(),
            "none" => {
                self.props_dialog = None;
                self.resize_dialog = None;
                self.regen_dialog = None;
                self.show_prefs = false;
                self.show_symmetry = false;
                self.new_dialog = None;
                self.show_shortcuts = false;
                self.backups = None;
                self.game_log.open = false;
                self.show_controls = false;
                self.show_about = false;
                self.export_dialog = None;
                self.image_dialog = None;
                self.browser.open = false;
                self.show_layers = false;
                self.show_strings = false;
                self.custom_dialog = None;
                self.column_dialog = None;
                self.note_dialog = None;
                self.decor_dialog = None;
                if let Some(c) = self.config.as_mut() {
                    c.open = false;
                }
            }
            _ => {
                return Err(trf(
                    "kfx_editor.unknown_dialog_name",
                    "unknown dialog {name}",
                    &[("name", &name)],
                ))
            }
        }
        Ok(())
    }
}

impl EditorApp {
    pub fn close_named_dialog(&mut self, name: &str) {
        match name {
            "image" => self.image_dialog = None,
            "export" => self.export_dialog = None,
            _ => {}
        }
    }

    pub fn set_prefs_tab(&mut self, t: &str) {
        self.prefs_tab = t.to_string();
    }

    pub fn set_script_max(&mut self, on: bool) {
        self.script_max = on;
    }

    /// File > Save (map and scripts).
    pub fn do_save_public(&mut self) {
        self.do_save();
    }

    pub fn browser_random(&mut self) -> Option<PathBuf> {
        if self.browser.packs.is_none() {
            self.browser.packs = self.install().map(crate::browser::scan);
        }
        self.browser.random()
    }

    pub fn browser_select(&mut self, p: PathBuf) {
        if self.browser.packs.is_none() {
            self.browser.packs = self.install().map(crate::browser::scan);
        }
        self.browser.select(Some(p));
    }

    /// Point the camera at a subtile rectangle and zoom so that it fills the view (keeps yaw and
    /// pitch). Uses the size of the last rendered viewport.
    pub fn frame_subtiles(&mut self, a: (f32, f32), b: (f32, f32)) {
        if self.camera_pending_home {
            let vp = self.last_vp;
            self.reset_camera(vp);
            self.camera_pending_home = false;
        }
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0) + 1.0);
        let (z0, z1) = (a.1.min(b.1), a.1.max(b.1) + 1.0);
        let h = self.doc.as_ref().map_or(1.0, |d| {
            d.doc
                .floor_height(((x0 + x1) / 2.0) as i32, ((z0 + z1) / 2.0) as i32) as f32
        });
        self.camera.target = Vec3::new((x0 + x1) / 2.0, h, (z0 + z1) / 2.0);
        let vp = self.last_vp;
        let mut c = self.camera;
        c.zoom = 1.0;
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in [
            Vec3::new(x0, h, z0),
            Vec3::new(x1, h, z0),
            Vec3::new(x0, h, z1),
            Vec3::new(x1, h, z1),
            Vec3::new(x0, h + 3.0, z0),
            Vec3::new(x1, h + 3.0, z1),
        ] {
            if let Some(q) = c.project(p, vp) {
                lo = lo.min(q);
                hi = hi.max(q);
            }
        }
        let size = (hi - lo).max(Vec2::ONE);
        self.camera.zoom = ((vp.x / size.x).min(vp.y / size.y) * 0.9).clamp(MIN_ZOOM, MAX_ZOOM);
    }
}

impl EditorApp {
    /// Test aid (actions only): make an inconsistent map on purpose, bypassing the door and slab
    /// rules: set a slab kind as is, or add a door thing without touching the slab.
    pub fn raw_edit(
        &mut self,
        v: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        let doc = self.doc.as_mut().ok_or("no map")?;
        let gd = doc.gd.clone();
        doc.end_stroke();
        let p = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_array())
                .and_then(|a| Some((a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32)))
        };
        let mut rf = Refresh::default();
        if let Some((x, y)) = p("slab") {
            let kind = v
                .get("kind")
                .and_then(|k| k.as_str())
                .and_then(|k| gd.slab_id(k))
                .ok_or("raw slab needs a known kind")? as u16;
            rf = rf.merge(
                doc.edit(&trd("kfx_editor.raw_slab", "Raw slab"), |d, g, _| {
                    let _ = d.set_slab(g, x as usize, y as usize, kind);
                })
                .1,
            );
        }
        if let Some((sx, sy)) = p("door_at") {
            let model = gd
                .door_names()
                .into_iter()
                .find(|(i, _)| *i > 0)
                .map(|x| x.0 as i64)
                .unwrap_or(1);
            let t = kfx_map::things::new_thing(
                class::DOOR,
                model,
                0,
                kfx_map::things::centred(sx),
                kfx_map::things::centred(sy),
                kfx_formats::level_fx::StlCoord::new(1, 0),
            );
            rf = rf.merge(
                doc.edit(&trd("kfx_editor.raw_door", "Raw door"), |d, _, _| {
                    d.things.items.push(t)
                })
                .1,
            );
        }
        self.after_change(rf);
        Ok(())
    }
}
