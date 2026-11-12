//! The fly view (V): free camera movement over the terrain, mouse capture, collision.
use super::*;

impl EditorApp {
    /// Height of the terrain (cubes) at subtile position (x, z); 0 outside the map.
    pub(super) fn terrain_height(&self, x: f32, z: f32) -> f32 {
        let t = &self.terrain;
        if x < 0.0 || z < 0.0 || x >= t.stl_w as f32 || z >= t.stl_h as f32 {
            return 0.0;
        }
        t.top_height(x as usize, z as usize) as f32
    }

    /// Enter or leave the first-person fly view (the 3D pane; from the 2D view it switches to 3D).
    pub fn toggle_fly(&mut self) {
        if self.doc.is_none() {
            return;
        }
        if self.camera.fly {
            self.camera.leave_fly();
            self.set_fly_capture(false);
        } else {
            if self.settings.layout.view_mode == crate::settings::ViewMode::TwoD {
                self.set_view_mode(crate::settings::ViewMode::ThreeD);
            }
            let t = self.camera.target;
            let ground = self.terrain_height(t.x, t.z).min(2.0);
            self.camera.enter_fly(ground);
            // start over open ground if the eye ended inside rock
            if self.settings.camera.fly_collision && self.fly_blocked(self.camera.target) {
                self.camera.target.y = self
                    .terrain_height(self.camera.target.x, self.camera.target.z)
                    + crate::camera::FLY_EYE_HEIGHT;
            }
        }
    }

    /// Capture (hide and lock) the mouse for the fly view's mouse-look, or free it.
    pub fn set_fly_capture(&mut self, on: bool) {
        let on = on && self.camera.fly;
        if on != self.fly_capture {
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::CursorGrab(if on {
                    egui::CursorGrab::Locked
                } else {
                    egui::CursorGrab::None
                }));
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::CursorVisible(!on));
        }
        self.fly_capture = on;
    }

    /// Move the fly view eye by `local` (right, up, forward) with the collision setting.
    pub fn fly_move(&mut self, local: Vec3) {
        let collide = self.settings.camera.fly_collision;
        let mut cam = self.camera;
        cam.fly_move(local, |_, to| collide && self.fly_blocked(to));
        self.camera = cam;
    }

    /// Fly view collision: outside the map, or less than half a cube above the terrain.
    pub(super) fn fly_blocked(&self, p: Vec3) -> bool {
        let t = &self.terrain;
        if p.x < 0.0 || p.z < 0.0 || p.x >= t.stl_w as f32 || p.z >= t.stl_h as f32 || p.y > 40.0 {
            return true;
        }
        p.y < self.terrain_height(p.x, p.z) + 0.5
    }

    /// Fly view input: WASD / arrows move, Q/E down/up, Shift faster, the wheel moves forward;
    /// right-drag, or the mouse while captured, looks around.
    pub(super) fn fly_input(&mut self, ui: &egui::Ui, resp: &egui::Response, ppp: f32) {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        if self.fly_capture && !ui.input(|i| i.viewport().focused.unwrap_or(true)) {
            self.set_fly_capture(false);
        }
        let c = &self.settings.camera;
        let k = c.orbit_speed * if c.invert_orbit { -1.0 } else { 1.0 };
        let mut look = Vec2::ZERO;
        if resp.dragged_by(egui::PointerButton::Secondary) {
            let d = resp.drag_delta() * ppp;
            look += Vec2::new(d.x, d.y);
        }
        if self.fly_capture {
            let m = ui.input(|i| i.pointer.motion().unwrap_or(i.pointer.delta()));
            look += Vec2::new(m.x, m.y);
        }
        if look != Vec2::ZERO {
            self.camera.orbit(-look.x * 0.004 * k, -look.y * 0.004 * k);
        }
        let mut mv = Vec3::ZERO;
        let mut fast = false;
        if !ui.ctx().egui_wants_keyboard_input() {
            let km = &self.keymap;
            let key = |id: &str| km.get(id).map(|s| s.key);
            ui.input(|i| {
                let down = |id: &str, alt: egui::Key| {
                    key(id).is_some_and(|k| i.key_down(k))
                        || (!i.modifiers.command && i.key_down(alt))
                };
                if down("cam.up", egui::Key::ArrowUp) {
                    mv.z += 1.0;
                }
                if down("cam.down", egui::Key::ArrowDown) {
                    mv.z -= 1.0;
                }
                if down("cam.left", egui::Key::ArrowLeft) {
                    mv.x -= 1.0;
                }
                if down("cam.right", egui::Key::ArrowRight) {
                    mv.x += 1.0;
                }
                if key("cam.rot_left").is_some_and(|k| i.key_down(k)) {
                    mv.y -= 1.0;
                }
                if key("cam.rot_right").is_some_and(|k| i.key_down(k)) {
                    mv.y += 1.0;
                }
                fast = i.modifiers.shift;
            });
        }
        let speed = self.settings.camera.fly_speed.max(0.5) * if fast { 3.0 } else { 1.0 };
        let mut local = mv * speed * dt;
        if resp.hovered() {
            local.z += ui.input(|i| i.smooth_scroll_delta.y) * 0.02;
        }
        if local != Vec3::ZERO {
            self.fly_move(local);
            ui.ctx().request_repaint();
        }
        if mv != Vec3::ZERO || self.fly_capture {
            ui.ctx().request_repaint();
        }
    }
}
