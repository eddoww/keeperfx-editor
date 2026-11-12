//! Viewport camera.
//!
//! World space: X = map east (subtile x), Y = up (one unit per cube), Z = map south (subtile y).
//! One world unit is one subtile wide and one cube tall. Right-handed: looking north (-Z), east
//! is to the right.
//!
//! With yaw 0 the camera stands south of its target looking north, so north is at the top of
//! the screen; positive yaw moves the eye towards the east (the view turns anticlockwise).
//!
//! # The game's default view, derived from the C code
//!
//! Angles in the game are 2048 per turn, compass-like: `ANGLE_NORTH` 0, `ANGLE_EAST` 512
//! (src/globals.h:170-188). A step of `d` in direction `a` moves the map position by
//! `(+d sin a, -d cos a)` (`move_coord_with_angle_x/_y`, src/bflib_planar.c:160-175; map y grows to
//! the south), so angle `a` points along `(sin a, -cos a)` in map coordinates.
//!
//! The isometric camera starts with `rotation_angle_x = DEGREES_45` = 256
//! (`src/local_camera.c:240`). The engine builds the view matrix from the identity
//! (`init_coords_and_rotation`, src/engine_render.c:451) with `rotate_base_axis(m, rotation_angle_x,
//! 2)` (src/engine_render.c:6799), a rotation about the up row `r[1]` = (0, 1, 0): with Rodrigues'
//! formula (src/engine_render.c:661-676) the rotation matrix is `[[c,0,s],[0,1,0],[-s,0,c]]`
//! (`c = cos a`, `s = sin a`), applied to the rows by `matrix_transform` (out_i = sum_j
//! M_ij v_j, src/engine_render.c:605), giving `r[0] = (c, 0, -s)` and `r[2] = (s, 0, c)`; the
//! following cross products (src/engine_render.c:688-698) leave them unchanged. The tilt
//! (`rotate_base_axis(.., -rotation_angle_y, 1)`, about `r[0]`) does not change `r[0]`.
//! Engine coordinates of a map point are `ex = x - cam_x`, `ey = height`, `ez = cam_y - y`
//! (`do_map_who_for_thing`, src/engine_render.c:8862-8864), and the screen position is
//! `sx = width/2 + r[0]·e`, `sy = height/2 - r[1]·e` (`rotpers_parallel_3`,
//! src/engine_render.c:541-565, which computes the dot products with Winograd's trick). Hence
//!
//! * screen right: `r[0]·e = c·dx + s·dy`, i.e. the map direction `(c, s)` = angle `a + 512`;
//! * screen up (into the screen on the ground): `r[2]` = `(s, -c)` in map coordinates = angle
//!   `a`, which is also what the camera's keyboard moves use: "down" is `a + 1024`
//!   (`view_move_camera_y`, src/engine_camera.c:428-434), "right" is `a + 512` (:420-426).
//!
//! At the default `a = 256`: map +x (east) goes up-right on the screen, map +y (south) goes
//! down-right, north-east is "up" (the north-east corner of the map is at the top of the
//! screen, the north-west corner on the left).
//!
//! The editor's eye is at `back() = (sin yaw, -, cos yaw)` from the target, so it looks along
//! map `(-sin yaw, -cos yaw)` = angle `-yaw`: **game angle `a` = `-yaw`**. The default view
//! is therefore yaw = -45 degrees (stored as 315). Sprites use the game's rule `sprite angle =
//! move_angle_xy - rotation_angle_x` (src/engine_render.c:4954) = facing + yaw (in 2048 units).
use glam::{Mat4, Vec2, Vec3, Vec4Swizzles};

/// Screen pixels per world unit at 100% zoom (one 32 px texture tile per subtile).
pub const PX_PER_UNIT: f32 = 32.0;
/// The game's default `rotation_angle_x` 256 (45 degrees) as an editor yaw (see the module
/// documentation): -45 degrees.
pub const DEFAULT_YAW_DEG: f32 = 315.0;
pub const DEFAULT_PITCH_DEG: f32 = -266.0 * 360.0 / 2048.0;
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 8.0;
const ORTHO_DIST: f32 = 1500.0;
const FOV_Y_DEG: f32 = 50.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    /// Radians. 0 = looking north.
    pub yaw: f32,
    /// Radians, negative looks down.
    pub pitch: f32,
    /// 1.0 = 100% = [`PX_PER_UNIT`] pixels per world unit at the target.
    pub zoom: f32,
    pub perspective: bool,
    /// Top-down orthographic (the 2D view): looking straight down, `yaw` turns the map
    /// (0 = north up), `pitch` and `perspective` are ignored.
    pub top_down: bool,
    /// First-person fly view: `target` is the eye, `yaw` / `pitch` the look direction (pitch
    /// may look up), perspective projection; `zoom` and `perspective` are ignored.
    pub fly: bool,
}

/// Height of the fly view's eye above the ground it starts over, in cubes.
pub const FLY_EYE_HEIGHT: f32 = 2.5;
const FLY_FOV_Y_DEG: f32 = 70.0;

/// A ray in world space (`dir` normalised).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
    /// Intersection with the horizontal plane `y = h`.
    pub fn hit_plane_y(&self, h: f32) -> Option<Vec3> {
        if self.dir.y.abs() < 1e-6 {
            return None;
        }
        let t = (h - self.origin.y) / self.dir.y;
        (t > 0.0).then(|| self.at(t))
    }
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: Vec3::ZERO,
            yaw: DEFAULT_YAW_DEG.to_radians(),
            pitch: DEFAULT_PITCH_DEG.to_radians(),
            zoom: 1.0,
            perspective: false,
            top_down: false,
            fly: false,
        }
    }
}

impl Camera {
    /// Unit vector from the target towards the eye.
    pub fn back(&self) -> Vec3 {
        if self.top_down && !self.fly {
            return Vec3::Y;
        }
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, -sp, cy * cp)
    }

    fn persp_dist(&self, viewport_h: f32) -> f32 {
        viewport_h / (2.0 * PX_PER_UNIT * self.zoom * (FOV_Y_DEG.to_radians() * 0.5).tan())
    }

    fn is_persp(&self) -> bool {
        self.perspective && !self.top_down
    }

    /// Unit look direction of the fly view.
    pub fn forward(&self) -> Vec3 {
        -self.back()
    }

    pub fn eye(&self, viewport: Vec2) -> Vec3 {
        if self.fly {
            return self.target;
        }
        let d = if self.is_persp() {
            self.persp_dist(viewport.y)
        } else {
            ORTHO_DIST
        };
        self.target + self.back() * d
    }

    /// Screen "up" on the ground in the top-down view: north at yaw 0.
    fn top_up(&self) -> Vec3 {
        let (s, c) = self.yaw.sin_cos();
        Vec3::new(-s, 0.0, -c)
    }

    pub fn view(&self, viewport: Vec2) -> Mat4 {
        if self.fly {
            return Mat4::look_to_rh(self.target, self.forward(), Vec3::Y);
        }
        Mat4::look_at_rh(
            self.eye(viewport),
            self.target,
            if self.top_down {
                self.top_up()
            } else {
                Vec3::Y
            },
        )
    }

    /// World directions of screen right and screen up (billboards).
    pub fn screen_axes(&self) -> (Vec3, Vec3) {
        if self.top_down && !self.fly {
            let (s, c) = self.yaw.sin_cos();
            return (Vec3::new(c, 0.0, -s), self.top_up());
        }
        let back = self.back();
        let right = Vec3::Y.cross(back).normalize_or(Vec3::X);
        (right, back.cross(right))
    }

    /// wgpu clip space (depth 0..1).
    pub fn proj(&self, viewport: Vec2) -> Mat4 {
        let vp = viewport.max(Vec2::ONE);
        if self.fly {
            return Mat4::perspective_rh(FLY_FOV_Y_DEG.to_radians(), vp.x / vp.y, 0.05, 600.0);
        }
        if self.is_persp() {
            let d = self.persp_dist(vp.y);
            Mat4::perspective_rh(
                FOV_Y_DEG.to_radians(),
                vp.x / vp.y,
                (d * 0.02).max(0.05),
                d + 2000.0,
            )
        } else {
            let hw = vp.x / (2.0 * PX_PER_UNIT * self.zoom);
            let hh = vp.y / (2.0 * PX_PER_UNIT * self.zoom);
            Mat4::orthographic_rh(-hw, hw, -hh, hh, 1.0, ORTHO_DIST * 2.0)
        }
    }

    pub fn view_proj(&self, viewport: Vec2) -> Mat4 {
        self.proj(viewport) * self.view(viewport)
    }

    /// Ray through pixel `p` (origin top-left) of a viewport of size `viewport`.
    pub fn ray(&self, p: Vec2, viewport: Vec2) -> Ray {
        let vp = viewport.max(Vec2::ONE);
        let ndc = Vec2::new(p.x / vp.x * 2.0 - 1.0, 1.0 - p.y / vp.y * 2.0);
        let inv = self.view_proj(vp).inverse();
        let near = inv * ndc.extend(0.0).extend(1.0);
        let far = inv * ndc.extend(1.0).extend(1.0);
        let (n, f) = (near.xyz() / near.w, far.xyz() / far.w);
        Ray {
            origin: n,
            dir: (f - n).normalize(),
        }
    }

    /// World point -> pixel (origin top-left). `None` behind the camera.
    pub fn project(&self, w: Vec3, viewport: Vec2) -> Option<Vec2> {
        let c = self.view_proj(viewport) * w.extend(1.0);
        if c.w <= 1e-6 {
            return None;
        }
        let n = c.xyz() / c.w;
        Some(Vec2::new(
            (n.x + 1.0) * 0.5 * viewport.x,
            (1.0 - n.y) * 0.5 * viewport.y,
        ))
    }

    /// Drag-pan: keep the ground point under `from` under `to`.
    pub fn pan_pixels(&mut self, from: Vec2, to: Vec2, viewport: Vec2) {
        let h = self.target.y;
        if let (Some(a), Some(b)) = (
            self.ray(from, viewport).hit_plane_y(h),
            self.ray(to, viewport).hit_plane_y(h),
        ) {
            self.target += a - b;
            self.target.y = h;
        }
    }

    /// Set the zoom keeping the world point `anchor` under pixel `p`.
    pub fn zoom_at(&mut self, new_zoom: f32, p: Vec2, anchor: Vec3, viewport: Vec2) {
        self.zoom = new_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if let Some(now) = self.ray(p, viewport).hit_plane_y(anchor.y) {
            let d = anchor - now;
            self.target.x += d.x;
            self.target.z += d.z;
        }
    }

    pub fn orbit(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw = (self.yaw + dyaw).rem_euclid(std::f32::consts::TAU);
        if self.fly {
            self.pitch = (self.pitch + dpitch).clamp(-89f32.to_radians(), 89f32.to_radians());
        } else if !self.top_down {
            self.pitch = (self.pitch + dpitch).clamp(-89f32.to_radians(), -5f32.to_radians());
        }
    }

    /// Enter the fly view from an orbit camera: the eye goes where the orbit camera's eye looks
    /// at the target from, but close to the ground (`ground` = height at the target), looking
    /// the same way, slightly down.
    pub fn enter_fly(&mut self, ground: f32) {
        let f = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        self.target = Vec3::new(self.target.x, ground + FLY_EYE_HEIGHT, self.target.z) - f * 6.0;
        self.pitch = -15f32.to_radians();
        self.top_down = false;
        self.fly = true;
    }

    /// Leave the fly view: orbit around the ground point in front of the eye.
    pub fn leave_fly(&mut self) {
        let f = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        self.target = Vec3::new(self.target.x, 0.0, self.target.z) + f * 6.0;
        self.pitch = DEFAULT_PITCH_DEG.to_radians();
        self.fly = false;
    }

    /// Fly view movement: `local` = (right, up, forward) in world units; forward and right
    /// stay horizontal (WASD walks over the map, Q/E move down/up). `blocked(from, to)` says
    /// whether the eye may not move from `from` to `to` (collision); each axis is tried on its
    /// own so the eye slides along walls.
    pub fn fly_move(&mut self, local: Vec3, blocked: impl Fn(Vec3, Vec3) -> bool) {
        let f = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let r = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let d = r * local.x + Vec3::Y * local.y + f * local.z;
        for axis in [Vec3::X, Vec3::Z, Vec3::Y] {
            let to = self.target + d * axis;
            if to != self.target && !blocked(self.target, to) {
                self.target = to;
            }
        }
    }

    /// The 2D view preset: top-down, north up, fitted to a `w x h` subtile map.
    pub fn top_down_home(stl_w: usize, stl_h: usize, viewport: Vec2) -> Camera {
        let mut c = Camera {
            target: Vec3::new(stl_w as f32 * 0.5, 0.0, stl_h as f32 * 0.5),
            yaw: 0.0,
            top_down: true,
            ..Default::default()
        };
        let (sx, sy) = (
            viewport.x / (stl_w.max(1) as f32 * PX_PER_UNIT),
            viewport.y / (stl_h.max(1) as f32 * PX_PER_UNIT),
        );
        c.zoom = (sx.min(sy) * 0.95).clamp(MIN_ZOOM, MAX_ZOOM);
        c
    }

    /// Rotate by 45 degree steps around the target (Q/E), snapping to the nearest step.
    pub fn rotate_step(&mut self, steps: i32) {
        let step = std::f32::consts::FRAC_PI_4;
        let k = (self.yaw / step).round() as i32 + steps;
        self.yaw = (k as f32 * step).rem_euclid(std::f32::consts::TAU);
    }

    /// The game's isometric view centred on a `w x h` subtile map, zoomed to fit `viewport`.
    pub fn home(stl_w: usize, stl_h: usize, viewport: Vec2, perspective: bool) -> Camera {
        let mut c = Camera {
            target: Vec3::new(stl_w as f32 * 0.5, 0.0, stl_h as f32 * 0.5),
            perspective,
            ..Default::default()
        };
        c.zoom = 1.0;
        c.fit(stl_w as f32, stl_h as f32, viewport);
        c
    }

    /// Choose the zoom at which the map rectangle fits the viewport (orthographic estimate).
    pub fn fit(&mut self, w: f32, h: f32, viewport: Vec2) {
        let mut c = *self;
        c.perspective = false;
        c.zoom = 1.0;
        let corners = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(w, 0.0, 0.0),
            Vec3::new(0.0, 0.0, h),
            Vec3::new(w, 0.0, h),
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::new(w, 5.0, h),
        ];
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for k in corners {
            if let Some(p) = c.project(k, viewport) {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        let size = (hi - lo).max(Vec2::ONE);
        let z = (viewport.x / size.x).min(viewport.y / size.y) * 0.95;
        self.zoom = z.clamp(MIN_ZOOM, MAX_ZOOM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VP: Vec2 = Vec2::new(1600.0, 1000.0);

    fn approx(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 0.05
    }

    #[test]
    fn target_projects_to_centre() {
        for persp in [false, true] {
            let c = Camera {
                target: Vec3::new(10.0, 1.0, 20.0),
                perspective: persp,
                ..Default::default()
            };
            assert!(
                approx(c.project(c.target, VP).unwrap(), VP * 0.5),
                "persp {persp}"
            );
        }
    }

    #[test]
    fn yaw0_north_is_up_east_is_right() {
        let c = Camera {
            yaw: 0.0,
            target: Vec3::new(50.0, 0.0, 50.0),
            ..Default::default()
        };
        let centre = c.project(c.target, VP).unwrap();
        let north = c.project(c.target - Vec3::Z, VP).unwrap();
        let east = c.project(c.target + Vec3::X, VP).unwrap();
        assert!(north.y < centre.y && (north.x - centre.x).abs() < 1e-3);
        assert!(east.x > centre.x && (east.y - centre.y).abs() < 1e-3);
        // 100% zoom: one unit east = 32 px
        assert!((east.x - centre.x - PX_PER_UNIT).abs() < 1e-3);
    }

    /// The editor's game angle of a camera (`a = -yaw`, 2048 per turn).
    pub fn game_angle(c: &Camera) -> i32 {
        ((-c.yaw.to_degrees() * 2048.0 / 360.0).round() as i32).rem_euclid(2048)
    }

    /// Pins the default view to the game's (`rotation_angle_x` 256, see the module docs): map +x
    /// points up-right and +y down-right on the screen, the north-east corner is at the top, and
    /// a creature with orientation 0 (facing north) is drawn with sprite view 3 mirrored, the
    /// same view the game picks for `move_angle_xy - rotation_angle_x` = -256.
    #[test]
    fn default_iso_view_like_game() {
        let c = Camera {
            target: Vec3::new(50.0, 0.0, 50.0),
            ..Default::default()
        };
        assert_eq!(game_angle(&c), 256);
        let o = c.project(c.target, VP).unwrap();
        let ex = c.project(c.target + Vec3::X, VP).unwrap();
        let ey = c.project(c.target + Vec3::Z, VP).unwrap();
        // +x: right and up; +y: right and down (screen y grows downwards)
        assert!(ex.x > o.x && ex.y < o.y, "+x {ex} from {o}");
        assert!(ey.x > o.x && ey.y > o.y, "+y {ey} from {o}");
        // the game's screen-right is the map direction (cos a, sin a), screen-up (sin a, -cos a)
        let a = 256f32 * std::f32::consts::TAU / 2048.0;
        let right = c
            .project(c.target + Vec3::new(a.cos(), 0.0, a.sin()), VP)
            .unwrap()
            - o;
        let up = c
            .project(c.target + Vec3::new(a.sin(), 0.0, -a.cos()), VP)
            .unwrap()
            - o;
        assert!(right.y.abs() < 1e-3 && right.x > 0.0, "{right}");
        assert!(up.x.abs() < 1e-3 && up.y < 0.0, "{up}");
        // corners of a 255x255 map: NE on top, SW at the bottom, NW left, SE right
        let p = |x: f32, z: f32| c.project(Vec3::new(x, 0.0, z), VP).unwrap();
        let (nw, ne, sw, se) = (p(0.0, 0.0), p(255.0, 0.0), p(0.0, 255.0), p(255.0, 255.0));
        assert!(ne.y < nw.y && ne.y < se.y && ne.y < sw.y);
        assert!(sw.y > nw.y && sw.y > se.y);
        assert!(nw.x < ne.x && nw.x < sw.x && nw.x < se.x);
        assert!(se.x > ne.x && se.x > sw.x);
        assert!((c.pitch.to_degrees() + 46.76).abs() < 0.01);
        // sprite view of a creature facing north: angle = facing - rotation_angle_x
        let angle = 0 - game_angle(&c);
        let v = kfx_gfx::keeper::view_for_angle(angle);
        assert_eq!((v.quarter, v.flip), (3, true));
        // the editor's sprite rule (app/things.rs): facing + yaw in 2048 units
        let editor_angle = 0 + (c.yaw.to_degrees() * 2048.0 / 360.0).round() as i32;
        let w = kfx_gfx::keeper::view_for_angle(editor_angle);
        assert_eq!((w.quarter, w.flip), (v.quarter, v.flip));
        // facing south-west (1280) points straight at the viewer: the front view 0
        assert_eq!(
            kfx_gfx::keeper::view_for_angle(1280 - game_angle(&c)).quarter,
            0
        );
    }

    #[test]
    fn ray_roundtrip() {
        for persp in [false, true] {
            let c = Camera {
                target: Vec3::new(30.0, 0.0, 40.0),
                zoom: 0.7,
                perspective: persp,
                ..Default::default()
            };
            let p = Vec2::new(300.0, 700.0);
            let hit = c.ray(p, VP).hit_plane_y(0.0).unwrap();
            assert!(approx(c.project(hit, VP).unwrap(), p), "persp {persp}");
        }
    }

    #[test]
    fn zoom_keeps_anchor_under_cursor() {
        for persp in [false, true] {
            let mut c = Camera {
                target: Vec3::new(30.0, 0.0, 40.0),
                perspective: persp,
                ..Default::default()
            };
            let p = Vec2::new(1200.0, 200.0);
            let anchor = c.ray(p, VP).hit_plane_y(0.0).unwrap();
            c.zoom_at(2.5, p, anchor, VP);
            assert!((c.zoom - 2.5).abs() < 1e-6);
            assert!(approx(c.project(anchor, VP).unwrap(), p), "persp {persp}");
        }
    }

    #[test]
    fn pan_moves_ground_with_cursor() {
        let mut c = Camera {
            target: Vec3::new(30.0, 0.0, 40.0),
            ..Default::default()
        };
        let (a, b) = (Vec2::new(500.0, 500.0), Vec2::new(650.0, 420.0));
        let g = c.ray(a, VP).hit_plane_y(0.0).unwrap();
        c.pan_pixels(a, b, VP);
        assert!(approx(c.project(g, VP).unwrap(), b));
    }

    #[test]
    fn rotate_steps_snap() {
        let mut c = Camera {
            yaw: 0.3,
            ..Default::default()
        };
        // 0.3 rad snaps to 0, then one step
        c.rotate_step(1);
        assert!((c.yaw - std::f32::consts::FRAC_PI_4).abs() < 1e-5);
        // 45 - 135 = -90 = 270 degrees
        c.rotate_step(-3);
        assert!((c.yaw - 6.0 * std::f32::consts::FRAC_PI_4).abs() < 1e-5);
    }

    #[test]
    fn top_down_north_up_and_picks() {
        let c = Camera {
            target: Vec3::new(50.0, 0.0, 50.0),
            yaw: 0.0,
            top_down: true,
            ..Default::default()
        };
        let o = c.project(c.target, VP).unwrap();
        let n = c.project(c.target - Vec3::Z, VP).unwrap();
        let e = c.project(c.target + Vec3::X, VP).unwrap();
        assert!(approx(o, VP * 0.5));
        assert!(n.y < o.y && (n.x - o.x).abs() < 1e-3, "north {n} from {o}");
        assert!(e.x > o.x && (e.y - o.y).abs() < 1e-3, "east {e} from {o}");
        // a slab is axis aligned: 3 subtiles = 96 px at 100%
        assert!((e.x - o.x - PX_PER_UNIT).abs() < 1e-3);
        let hit = c.ray(Vec2::new(300.0, 200.0), VP).hit_plane_y(0.0).unwrap();
        assert!(approx(c.project(hit, VP).unwrap(), Vec2::new(300.0, 200.0)));
        let (r, u) = c.screen_axes();
        assert!((r - Vec3::X).length() < 1e-5 && (u + Vec3::Z).length() < 1e-5);
        let h = Camera::top_down_home(255, 255, VP);
        for k in [Vec3::ZERO, Vec3::new(255.0, 0.0, 255.0)] {
            let p = h.project(k, VP).unwrap();
            assert!(
                p.x >= 0.0 && p.y >= 0.0 && p.x <= VP.x && p.y <= VP.y,
                "{k} -> {p}"
            );
        }
    }

    #[test]
    fn fly_view_looks_forward_and_moves() {
        let mut c = Camera {
            target: Vec3::new(30.0, 0.0, 40.0),
            yaw: 0.0,
            ..Default::default()
        };
        c.enter_fly(1.0);
        assert!(c.fly && (c.target.y - (1.0 + FLY_EYE_HEIGHT)).abs() < 1e-5);
        // looking north (yaw 0): a point straight ahead projects on the vertical centre line
        let ahead = c.target + Vec3::new(0.0, 0.0, -10.0);
        let p = c.project(ahead, VP).unwrap();
        assert!((p.x - VP.x * 0.5).abs() < 1e-2, "{p}");
        // the eye's ray through the centre points along forward
        let r = c.ray(VP * 0.5, VP);
        assert!(
            (r.dir - c.forward()).length() < 1e-3,
            "{:?} vs {}",
            r.dir,
            c.forward()
        );
        // W moves north, D east, E up
        let start = c.target;
        c.fly_move(Vec3::new(1.0, 2.0, 3.0), |_, _| false);
        assert!(
            (c.target - (start + Vec3::new(1.0, 2.0, -3.0))).length() < 1e-4,
            "{}",
            c.target
        );
        // collision blocks one axis only (slide)
        let start = c.target;
        c.fly_move(Vec3::new(1.0, 0.0, 1.0), |_, to: Vec3| to.x > start.x);
        assert!((c.target.x - start.x).abs() < 1e-5 && c.target.z < start.z);
        // looking up is allowed in the fly view, unlike the orbit camera
        c.orbit(0.0, 1.0);
        assert!(c.pitch > 0.5);
        c.leave_fly();
        assert!(!c.fly && c.pitch < 0.0);
    }

    #[test]
    fn home_fits_map() {
        let c = Camera::home(255, 255, VP, false);
        for k in [
            Vec3::ZERO,
            Vec3::new(255.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 255.0),
            Vec3::new(255.0, 0.0, 255.0),
        ] {
            let p = c.project(k, VP).unwrap();
            assert!(
                p.x >= 0.0 && p.y >= 0.0 && p.x <= VP.x && p.y <= VP.y,
                "{k} -> {p}"
            );
        }
    }
}
