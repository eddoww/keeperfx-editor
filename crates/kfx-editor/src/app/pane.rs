//! One viewport pane: pointer and keyboard interaction with the map, the 2D / 3D drawing of the
//! pane, and the grid data overlay (View > Grid data).
use super::*;

impl EditorApp {
    /// One view pane at `rect` (`slot` 1 = the 2D pane, its camera swapped in).
    pub(super) fn pane_ui(&mut self, ui: &mut egui::Ui, rect: egui::Rect, slot: usize) {
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        // (clamped to the texture limit: a transient frame after a UI scale change can be huge)
        let size = (
            (rect.width() * ppp).round().clamp(1.0, 8192.0) as u32,
            (rect.height() * ppp).round().clamp(1.0, 8192.0) as u32,
        );
        let vp = Vec2::new(size.0 as f32, size.1 as f32);
        let to_px = |p: egui::Pos2| Vec2::new((p.x - rect.min.x) * ppp, (p.y - rect.min.y) * ppp);
        if self.camera_pending_home {
            self.reset_camera(vp);
            self.camera_pending_home = false;
        }
        let shift = ui.input(|i| i.modifiers.shift);
        let thing_tool = matches!(self.tool, Tool::Place | Tool::Things) && !self.pasting;

        if self.camera.fly {
            self.fly_input(ui, &resp, ppp);
        } else {
            // Camera input: middle / shift+left pan, right orbit (also while a tool is active)
            if let Some(pos) = resp.interact_pointer_pos().or(resp.hover_pos()) {
                let d = resp.drag_delta() * ppp;
                if resp.dragged_by(egui::PointerButton::Middle)
                    || (resp.dragged_by(egui::PointerButton::Primary)
                        && shift
                        && self.drag.is_none()
                        && !thing_tool)
                {
                    let cur = to_px(pos);
                    self.camera.pan_pixels(cur - Vec2::new(d.x, d.y), cur, vp);
                } else if resp.dragged_by(egui::PointerButton::Secondary) {
                    let c = &self.settings.camera;
                    let k = c.orbit_speed * if c.invert_orbit { -1.0 } else { 1.0 };
                    self.camera.orbit(-d.x * 0.006 * k, -d.y * 0.004 * k);
                }
            }
            if resp.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    if let Some(pos) = resp.hover_pos() {
                        let p = to_px(pos);
                        let anchor = self
                            .pick_at(p, vp)
                            .map(|h| h.hit.point)
                            .or_else(|| self.camera.ray(p, vp).hit_plane_y(0.0))
                            .unwrap_or(self.camera.target);
                        let c = &self.settings.camera;
                        let k = 0.0025 * c.zoom_speed * if c.invert_zoom { -1.0 } else { 1.0 };
                        let z = self.camera.zoom * (scroll * k).exp();
                        self.camera.zoom_at(z, p, anchor, vp);
                    }
                }
            }
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            let mut want = Vec2::ZERO;
            // keyboard camera moves drive one pane only (the 3D one in the split view)
            let keyboard = slot == 0 || self.panes.len() == 1;
            if keyboard && !ui.ctx().egui_wants_keyboard_input() {
                let km = &self.keymap;
                ui.input(|i| {
                    let arrows = !i.modifiers.command && !i.modifiers.alt;
                    if km.held(i, "cam.up") || (arrows && i.key_down(egui::Key::ArrowUp)) {
                        want.y -= 1.0;
                    }
                    if km.held(i, "cam.down") || (arrows && i.key_down(egui::Key::ArrowDown)) {
                        want.y += 1.0;
                    }
                    if km.held(i, "cam.left") || (arrows && i.key_down(egui::Key::ArrowLeft)) {
                        want.x -= 1.0;
                    }
                    if km.held(i, "cam.right") || (arrows && i.key_down(egui::Key::ArrowRight)) {
                        want.x += 1.0;
                    }
                });
                if self.keymap.pressed(ui.ctx(), "cam.rot_left") {
                    self.camera.rotate_step(-1);
                }
                if self.keymap.pressed(ui.ctx(), "cam.rot_right") {
                    self.camera.rotate_step(1);
                }
                if self.keymap.pressed(ui.ctx(), "cam.home") {
                    self.reset_camera(vp);
                }
            }
            // edge panning (pointer within 12 points of the view's border)
            if self.settings.camera.edge_pan
                && self.drag.is_none()
                && self.thing_drag.is_none()
                && self.fixed_frame.is_none()
            {
                if let Some(p) = resp.hover_pos() {
                    let m = 12.0;
                    if p.x < rect.min.x + m {
                        want.x -= 1.0;
                    } else if p.x > rect.max.x - m {
                        want.x += 1.0;
                    }
                    if p.y < rect.min.y + m {
                        want.y -= 1.0;
                    } else if p.y > rect.max.y - m {
                        want.y += 1.0;
                    }
                }
            }
            let sm = self.settings.camera.smoothing.clamp(0.0, 0.95);
            let a = 1.0 - sm.powf(dt * 60.0);
            self.pan_vel += (want - self.pan_vel) * a;
            if self.pan_vel.length() < 0.01 && want == Vec2::ZERO {
                self.pan_vel = Vec2::ZERO;
            }
            if self.pan_vel != Vec2::ZERO {
                let c = vp * 0.5;
                let speed = self.settings.camera.pan_speed * dt;
                self.camera.pan_pixels(c + self.pan_vel * speed, c, vp);
                ui.ctx().request_repaint();
            }
        }

        // Picking (keeps following the pointer outside the view while a tool drags; at the
        // crosshair while the fly view has the mouse)
        let pointer = if self.camera.fly && self.fly_capture {
            Some(rect.center())
        } else if self.drag.is_some() {
            ui.input(|i| i.pointer.latest_pos())
        } else {
            resp.hover_pos()
        };
        if let Some(pos) = pointer {
            if let Some(h) = self.pick_at(to_px(pos), vp) {
                self.hover = Some(h);
            } else if self.drag.is_none() {
                self.hover = None;
            }
        } else if self.fixed_frame.is_none()
            && !ui
                .input(|i| i.pointer.latest_pos())
                .is_some_and(|p| self.panes.iter().any(|r| *r != rect && r.contains(p)))
        {
            // (the other pane of the split view keeps its hover)
            self.hover = None;
        }

        // Tools
        let slab = self
            .hover
            .as_ref()
            .map(|h| (h.slab.0 as i32, h.slab.1 as i32));
        let (pressed, down, released, alt) = ui.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Primary),
                i.pointer.button_down(egui::PointerButton::Primary),
                i.pointer.button_released(egui::PointerButton::Primary),
                i.modifiers.alt,
            )
        });
        // script markers and pick mode come first
        let over_marker = resp.hover_pos().and_then(|p| {
            self.markers
                .iter()
                .position(|m| m.rect.is_some_and(|r| r.expand(2.0).contains(p)))
        });
        let pressed = if pressed && resp.hovered() && self.drag.is_none() && !self.pointer_drag {
            if self.pick.is_some() {
                if let Some(pos) = resp.hover_pos() {
                    self.pick_click(to_px(pos), vp);
                }
                false
            } else if let Some(i) = over_marker {
                self.click_marker(i);
                false
            } else {
                true
            }
        } else {
            pressed && self.pick.is_none()
        };
        if thing_tool {
            if pressed && resp.hovered() {
                if let Some(pos) = resp.hover_pos() {
                    self.things_press(to_px(pos), vp, shift, alt);
                    self.pointer_drag = true;
                }
            }
            if self.pointer_drag {
                if down {
                    if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
                        self.things_drag(to_px(pos), vp);
                    }
                }
                if released || !down {
                    self.things_release();
                    self.pointer_drag = false;
                }
            }
        } else if pressed && resp.hovered() && !shift {
            if let Some(p) = slab {
                self.tool_press(p, alt);
                self.pointer_drag = self.drag.is_some();
            }
        }
        if let (Some(d), true, false) = (self.drag, self.pointer_drag, thing_tool) {
            if down {
                if let Some(p) = slab.filter(|&p| p != d.cur) {
                    self.tool_drag(p);
                }
            }
            if released || !down {
                self.tool_release(slab.unwrap_or(d.cur));
                self.pointer_drag = false;
            }
        }

        let painter = ui.painter_at(rect);
        if self.doc.is_some() {
            if self.camera.top_down && self.settings.layout.flat_colors {
                self.flat_colors(&painter, rect, vp, ppp);
            }
            if let Some(id) = self.render_view(size) {
                painter.image(
                    id,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        } else {
            painter.rect_filled(rect, 0.0, Color32::from_rgb(9, 8, 7));
            let msg = if self.assets.is_none() {
                trd(
                    "kfx_editor.choose_your_keeperfx_folder_to_start",
                    "Choose your KeeperFX folder to start.",
                )
                .to_string()
            } else {
                trf(
                    "kfx_editor.open_a_map_file_open_keymap_the_map_browser_keym",
                    "Open a map: File > Open ({key}), the map browser ({key2}),\nor \
                    start one with File > New map ({key3}).",
                    &[
                        ("key", &self.keymap.label("file.open")),
                        ("key2", &self.keymap.label("file.browser")),
                        ("key3", &self.keymap.label("file.new")),
                    ],
                )
            };
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                msg,
                egui::FontId::proportional(18.0),
                TEXT_DIM,
            );
        }

        // badges (creature levels, hero gate and action point numbers) and the selection box
        if self.doc.is_some() {
            let to_ui = |p: Vec2| rect.min + EVec2::new(p.x / ppp, p.y / ppp);
            // badges fade out when zoomed out (declutter): creature levels below 45%, the
            // others (action points, gates) below 25%
            let z = self.camera.zoom;
            let fade = |lo: f32, hi: f32| {
                if self.settings.graphics.declutter {
                    ((z - lo) / (hi - lo)).clamp(0.0, 1.0)
                } else {
                    1.0
                }
            };
            let (fa_level, fa_other) = (fade(0.3, 0.45), fade(0.12, 0.25));
            for b in &self.badges {
                let a = if b.text.chars().all(|c| c.is_ascii_digit()) {
                    fa_level
                } else {
                    fa_other
                };
                if a <= 0.02 {
                    continue;
                }
                let Some(p) = self.camera.project(b.pos, vp) else {
                    continue;
                };
                let at = to_ui(p) + EVec2::new(0.0, 6.0);
                if !rect.contains(at) {
                    continue;
                }
                let galley = painter.layout_no_wrap(
                    b.text.clone(),
                    egui::FontId::proportional(10.0),
                    Color32::WHITE.gamma_multiply(a),
                );
                let r = egui::Rect::from_center_size(at, galley.size() + EVec2::new(6.0, 2.0));
                painter.rect(
                    r,
                    3.0,
                    Color32::from_black_alpha((190.0 * a) as u8),
                    egui::Stroke::new(
                        1.0,
                        Color32::from_rgb(b.color[0], b.color[1], b.color[2]).gamma_multiply(a),
                    ),
                    egui::StrokeKind::Inside,
                );
                painter.galley(
                    r.min + EVec2::new(3.0, 1.0),
                    galley,
                    Color32::WHITE.gamma_multiply(a),
                );
            }
            if let Some(r) = &self.rect_sel {
                let sr = egui::Rect::from_two_pos(to_ui(r.start), to_ui(r.cur));
                painter.rect(
                    sr,
                    0.0,
                    Color32::from_rgba_unmultiplied(90, 170, 255, 30),
                    egui::Stroke::new(1.0, Color32::from_rgb(110, 190, 255)),
                    egui::StrokeKind::Inside,
                );
            }
            self.draw_grid_data(&painter, rect, vp, ppp);
            self.draw_analysis(&painter, rect, vp, ppp);
            self.build_markers();
            let pointer = resp.hover_pos();
            let note = self.draw_notes(&painter, rect, vp, ppp, pointer);
            self.draw_logic_marks(&painter, rect, vp, ppp);
            self.live_overlay(&painter, rect, vp, ppp);
            if let Some(i) = self.draw_markers(&painter, rect, vp, ppp, pointer) {
                if let Some(p) = pointer {
                    self.marker_tooltip(&painter, i, p);
                }
            } else if let (Some(t), Some(p)) = (note, pointer) {
                let galley =
                    painter.layout(t, egui::FontId::proportional(12.5), theme::TEXT, 420.0);
                let r = egui::Rect::from_min_size(
                    p + EVec2::new(14.0, 14.0),
                    galley.size() + EVec2::new(14.0, 10.0),
                );
                painter.rect(
                    r,
                    4.0,
                    Color32::from_rgba_unmultiplied(40, 32, 10, 240),
                    egui::Stroke::new(1.0, Color32::from_rgb(240, 200, 80)),
                    egui::StrokeKind::Inside,
                );
                painter.galley(r.min + EVec2::new(7.0, 5.0), galley, theme::TEXT);
            }
        }

        // Tool hint (top-left)
        let hint = if let Some(k) = self.pick {
            trf(
                "kfx_editor.pick_from_map_click_k_esc_cancels",
                "Pick from map: click {k} · Esc cancels",
                &[(
                    "k",
                    &(match k {
                        kfx_script_ui::PickKind::ActionPoint => {
                            trd("kfx_editor.an_action_point", "an action point")
                        }
                        kfx_script_ui::PickKind::HeroGate => {
                            trd("kfx_editor.a_hero_gate", "a hero gate")
                        }
                        kfx_script_ui::PickKind::Location => trd(
                            "kfx_editor.an_action_point_or_a_hero_gate",
                            "an action point or a hero gate",
                        ),
                        kfx_script_ui::PickKind::Subtile => {
                            trd("kfx_editor.a_subtile", "a subtile")
                        }
                        kfx_script_ui::PickKind::Slab => trd("kfx_editor.a_slab", "a slab"),
                    }),
                )],
            )
        } else if self.pasting {
            trd(
                "kfx_editor.paste_click_to_stamp_r_rotate_f_flip_esc_stop",
                "Paste: click to stamp · R rotate · F flip · Esc stops",
            )
            .to_string()
        } else {
            let what = if self.owner_only {
                trf(
                    "kfx_editor.owner_active_owner",
                    "owner {active_owner}",
                    &[("active_owner", &PLAYERS[self.active_owner as usize].0)],
                )
            } else {
                self.brush_name()
            };
            match self.tool {
                Tool::Select => trd(
                    "kfx_editor.select_drag_a_rectangle_ctrl_c_x_v_del_fills_wit",
                    "Select: drag a rectangle · Ctrl+C/X/V · Del fills with the \
                    palette slab",
                )
                .into(),
                Tool::Eyedropper => trd(
                    "kfx_editor.eyedropper_click_a_slab_to_pick_its_kind_and_own",
                    "Eyedropper: click a slab to pick its kind and owner",
                )
                .into(),
                Tool::Place => trf(
                    "kfx_editor.place_gd_click_shift_click_keeps_placing_alt_cli",
                    "Place {gd}: click · Shift+click keeps placing · Alt+click picks \
                    a thing's settings{free_place}",
                    &[
                        (
                            "gd",
                            &self
                                .place
                                .as_ref()
                                .zip(self.gd())
                                .map(|(k, gd)| k.label(gd))
                                .unwrap_or_else(|| "(choose on the palette)".into()),
                        ),
                        (
                            "free_place",
                            &(if self.free_place {
                                trd("kfx_editor.free_sub_position_0a5c", " · free sub-position")
                            } else {
                                String::new()
                            }),
                        ),
                    ],
                ),
                Tool::Things => trd(
                    "kfx_editor.things_click_selects_again_next_in_the_stack_shi",
                    "Things: click selects (again: next in the stack) · Shift+click \
                    adds · drag moves / boxes · Del · Ctrl+D · R turns",
                )
                .into(),
                t => trf(
                    "kfx_editor.t_what_alt_click_picks_right_drag_orbits_esc_can",
                    "{t}: {what} · Alt+click picks · right-drag orbits · Esc cancels",
                    &[("t", &t.label()), ("what", &what)],
                ),
            }
        };
        let hint = if self.camera.top_down {
            format!("2D · {hint}")
        } else if self.camera.fly {
            let look = if self.fly_capture {
                trd(
                    "kfx_editor.m_or_esc_frees_the_mouse",
                    "M or Esc frees the mouse",
                )
            } else {
                trd(
                    "kfx_editor.m_captures_the_mouse_or_right_drag_looks",
                    "M captures the mouse, or right-drag looks",
                )
            };
            trf(
                "kfx_editor.fly_wasd_move_q_e_down_up_shift_faster_look_v_le",
                "Fly: WASD move · Q/E down/up · Shift faster · {look} · V leaves",
                &[("look", &look)],
            )
        } else {
            hint
        };
        if self.camera.fly && self.fly_capture {
            let c = rect.center();
            let st = egui::Stroke::new(1.5, Color32::from_white_alpha(200));
            painter.line_segment([c - EVec2::new(8.0, 0.0), c + EVec2::new(8.0, 0.0)], st);
            painter.line_segment([c - EVec2::new(0.0, 8.0), c + EVec2::new(0.0, 8.0)], st);
        }
        if self.doc.is_some() {
            let hint_pos = rect.min + EVec2::new(10.0, 10.0);
            let galley =
                painter.layout_no_wrap(hint, egui::FontId::proportional(13.0), theme::TEXT);
            painter.rect_filled(
                egui::Rect::from_min_size(hint_pos, galley.size() + EVec2::new(12.0, 8.0)),
                3.0,
                Color32::from_black_alpha(170),
            );
            painter.galley(hint_pos + EVec2::new(6.0, 4.0), galley, theme::TEXT);
        }

        // Hover readout (top-right)
        if let Some(text) = self.readout() {
            let galley =
                painter.layout_no_wrap(text, egui::FontId::proportional(14.0), theme::TEXT);
            let sz = galley.size() + EVec2::new(16.0, 10.0);
            // narrow panes (split view): below the tool hint instead of over it
            let y = if rect.width() < 1000.0 {
                rect.min.y + 44.0
            } else {
                rect.min.y + 10.0
            };
            let r = egui::Rect::from_min_size(egui::pos2(rect.max.x - sz.x - 10.0, y), sz);
            painter.rect(
                r,
                3.0,
                Color32::from_black_alpha(180),
                egui::Stroke::new(1.0, GOLD_DIM),
                egui::StrokeKind::Inside,
            );
            painter.galley(r.min + EVec2::new(8.0, 5.0), galley, theme::TEXT);
        }
        if self.fixed_frame.is_none() && self.doc.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    /// View > Grid data: the chosen raw grid written over each visible slab / subtile, when
    /// zoomed in far enough for the numbers to be readable.
    pub(super) fn draw_grid_data(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        vp: Vec2,
        ppp: f32,
    ) {
        use crate::griddata::{self, GridData};
        let which = self.settings.layout.grid_data;
        let Some(doc) = &self.doc else { return };
        if which == GridData::Off {
            return;
        }
        if which == GridData::Column && self.grid_cols.is_none() {
            self.grid_cols = griddata::column_indices(&doc.doc);
        }
        let sub = which.per_subtile();
        let t = &self.terrain;
        let step = if sub { 1.0 } else { 3.0 };
        let (cw, ch) = if sub {
            (t.stl_w, t.stl_h)
        } else {
            (t.stl_w / 3, t.stl_h / 3)
        };
        let pos = |x: usize, y: usize| -> Vec3 {
            if sub {
                Vec3::new(
                    x as f32 + 0.5,
                    t.top_height(x.min(t.stl_w - 1), y.min(t.stl_h - 1)) as f32,
                    y as f32 + 0.5,
                )
            } else {
                Vec3::new(
                    x as f32 * 3.0 + 1.5,
                    t.slab_top(x, y) as f32,
                    y as f32 * 3.0 + 1.5,
                )
            }
        };
        let to_ui = |p: Vec2| rect.min + EVec2::new(p.x / ppp, p.y / ppp);
        // readable only when a cell is wide enough on screen
        let c = self.camera.target;
        let (cx, cy) = (
            ((c.x / step) as usize).min(cw.saturating_sub(2)),
            ((c.z / step) as usize).min(ch.saturating_sub(2)),
        );
        let spacing = match (
            self.camera.project(pos(cx, cy), vp),
            self.camera.project(pos(cx + 1, cy), vp),
        ) {
            (Some(a), Some(b)) => (a - b).length() / ppp,
            _ => 0.0,
        };
        let msg_at = rect.left_bottom() + EVec2::new(10.0, -30.0);
        if spacing < if sub { 16.0 } else { 20.0 } && !self.camera.fly {
            painter.text(
                msg_at,
                egui::Align2::LEFT_BOTTOM,
                trf(
                    "kfx_editor.grid_data_which_zoom_in_to_read_the_values",
                    "Grid data: {which} · zoom in to read the values",
                    &[("which", &which.label())],
                ),
                egui::FontId::proportional(12.0),
                theme::TEXT,
            );
            return;
        }
        // visible cells: the bounding box of the ground under the view
        let f = minimap::frustum(&self.camera, vp);
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in f {
            lo = lo.min(p);
            hi = hi.max(p);
        }
        if self.camera.fly {
            let e = Vec2::new(c.x, c.z);
            lo = lo.min(e - Vec2::splat(40.0));
            hi = hi.max(e + Vec2::splat(40.0));
        }
        let x0 = ((lo.x / step).floor().max(0.0) as usize).min(cw);
        let y0 = ((lo.y / step).floor().max(0.0) as usize).min(ch);
        let x1 = ((hi.x / step).ceil().max(0.0) as usize + 1).min(cw);
        let y1 = ((hi.y / step).ceil().max(0.0) as usize + 1).min(ch);
        let font = egui::FontId::monospace(if sub { 10.0 } else { 11.0 });
        let cols = self.grid_cols.as_deref();
        let mut n = 0;
        'rows: for y in y0..y1 {
            for x in x0..x1 {
                let Some(text) = griddata::value(&doc.doc, which, x, y, cols) else {
                    continue;
                };
                let Some(p) = self.camera.project(pos(x, y), vp) else {
                    continue;
                };
                let at = to_ui(p);
                if !rect.shrink(4.0).contains(at) {
                    continue;
                }
                let galley = painter.layout_no_wrap(text, font.clone(), Color32::WHITE);
                let r = egui::Rect::from_center_size(at, galley.size() + EVec2::new(4.0, 0.0));
                painter.rect_filled(r, 2.0, Color32::from_black_alpha(150));
                painter.galley(r.min + EVec2::new(2.0, 0.0), galley, Color32::WHITE);
                n += 1;
                if n >= 6000 {
                    break 'rows;
                }
            }
        }
        let note = if which == GridData::Flags && doc.doc.flg.is_none() {
            trd(
                "kfx_editor.this_map_has_no_flg_file",
                " · this map has no .flg file",
            )
        } else {
            String::new()
        };
        painter.text(
            msg_at,
            egui::Align2::LEFT_BOTTOM,
            trf(
                "kfx_editor.grid_data_which_note",
                "Grid data: {which}{note}",
                &[("which", &which.label()), ("note", &note)],
            ),
            egui::FontId::proportional(12.0),
            theme::TEXT,
        );
    }
}
