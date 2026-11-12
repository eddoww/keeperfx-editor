//! Picking: the subtile under the cursor, by casting the view ray against the column height
//! field (each subtile a box from 0 to the top of its highest solid cube).
use crate::camera::Ray;
use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub stl_x: usize,
    pub stl_y: usize,
    pub point: Vec3,
    /// Height of the box that was hit.
    pub height: f32,
}

/// Cast `ray` against a `w x h` grid of boxes whose heights `height(x, y)` returns.
/// Grid DDA over the cells the ray crosses (Amanatides & Woo).
pub fn pick(
    ray: &Ray,
    w: usize,
    h: usize,
    max_h: f32,
    height: impl Fn(usize, usize) -> f32,
) -> Option<Hit> {
    let (wf, hf) = (w as f32, h as f32);
    // Clip the ray to the grid's bounding box.
    let (mut t0, mut t1) = (0.0f32, f32::INFINITY);
    for (o, d, lo, hi) in [
        (ray.origin.x, ray.dir.x, 0.0, wf),
        (ray.origin.y, ray.dir.y, -1.0, max_h),
        (ray.origin.z, ray.dir.z, 0.0, hf),
    ] {
        if d.abs() < 1e-9 {
            if o < lo || o > hi {
                return None;
            }
        } else {
            let (a, b) = ((lo - o) / d, (hi - o) / d);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
    }
    if t0 > t1 {
        return None;
    }
    let p = ray.at(t0);
    let mut cx = (p.x.floor() as isize).clamp(0, w as isize - 1);
    let mut cz = (p.z.floor() as isize).clamp(0, h as isize - 1);
    let step_x: isize = if ray.dir.x >= 0.0 { 1 } else { -1 };
    let step_z: isize = if ray.dir.z >= 0.0 { 1 } else { -1 };
    let next = |c: isize, step: isize, o: f32, d: f32| -> f32 {
        if d.abs() < 1e-9 {
            f32::INFINITY
        } else {
            let edge = if step > 0 { (c + 1) as f32 } else { c as f32 };
            (edge - o) / d
        }
    };
    let dtx = if ray.dir.x.abs() < 1e-9 {
        f32::INFINITY
    } else {
        1.0 / ray.dir.x.abs()
    };
    let dtz = if ray.dir.z.abs() < 1e-9 {
        f32::INFINITY
    } else {
        1.0 / ray.dir.z.abs()
    };
    let mut tx = next(cx, step_x, ray.origin.x, ray.dir.x);
    let mut tz = next(cz, step_z, ray.origin.z, ray.dir.z);
    let mut t_in = t0;
    loop {
        let t_out = tx.min(tz).min(t1);
        let ch = height(cx as usize, cz as usize);
        let y_in = ray.at(t_in).y;
        let y_out = ray.at(t_out).y;
        if y_in <= ch {
            return Some(Hit {
                stl_x: cx as usize,
                stl_y: cz as usize,
                point: ray.at(t_in),
                height: ch,
            });
        }
        if y_out <= ch {
            let t = t_in + (ch - y_in) / (y_out - y_in) * (t_out - t_in);
            return Some(Hit {
                stl_x: cx as usize,
                stl_y: cz as usize,
                point: ray.at(t),
                height: ch,
            });
        }
        if t_out >= t1 {
            return None;
        }
        if tx < tz {
            cx += step_x;
            tx += dtx;
        } else {
            cz += step_z;
            tz += dtz;
        }
        if cx < 0 || cz < 0 || cx >= w as isize || cz >= h as isize {
            return None;
        }
        t_in = t_out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use glam::Vec2;

    fn field(x: usize, _y: usize) -> f32 {
        // a 5-high wall along x = 10, everything else floor
        if x == 10 {
            5.0
        } else {
            0.0
        }
    }

    #[test]
    fn straight_down_hits_cell() {
        let r = Ray {
            origin: Vec3::new(3.5, 50.0, 7.25),
            dir: -Vec3::Y,
        };
        let h = pick(&r, 20, 20, 8.0, field).unwrap();
        assert_eq!((h.stl_x, h.stl_y), (3, 7));
        assert!((h.point.y).abs() < 1e-4);
        let r = Ray {
            origin: Vec3::new(10.5, 50.0, 2.5),
            dir: -Vec3::Y,
        };
        let h = pick(&r, 20, 20, 8.0, field).unwrap();
        assert_eq!((h.stl_x, h.stl_y, h.height), (10, 2, 5.0));
        assert!((h.point.y - 5.0).abs() < 1e-4);
    }

    #[test]
    fn wall_blocks_ground_behind_it() {
        // Looking east and down: the ground point behind the wall is hidden by the wall side.
        let target = Vec3::new(11.5, 0.0, 4.5);
        let origin = Vec3::new(2.0, 6.0, 4.5);
        let r = Ray {
            origin,
            dir: (target - origin).normalize(),
        };
        let h = pick(&r, 20, 20, 8.0, field).unwrap();
        assert_eq!((h.stl_x, h.stl_y), (10, 4));
        assert!((h.point.x - 10.0).abs() < 1e-3, "{:?}", h.point);
        // ...but a ray passing above the wall reaches the floor behind.
        let origin = Vec3::new(2.0, 30.0, 4.5);
        let r = Ray {
            origin,
            dir: (Vec3::new(14.5, 0.0, 4.5) - origin).normalize(),
        };
        let h = pick(&r, 20, 20, 8.0, field).unwrap();
        assert_eq!((h.stl_x, h.stl_y), (14, 4));
    }

    #[test]
    fn camera_ray_picks_projected_point() {
        let c = Camera {
            target: Vec3::new(5.0, 0.0, 5.0),
            ..Default::default()
        };
        let vp = Vec2::new(800.0, 600.0);
        for (x, y) in [(2usize, 3usize), (15, 17), (10, 6)] {
            let top = Vec3::new(x as f32 + 0.5, field(x, y), y as f32 + 0.5);
            let px = c.project(top, vp).unwrap();
            let h = pick(&c.ray(px, vp), 20, 20, 8.0, field).unwrap();
            assert_eq!((h.stl_x, h.stl_y), (x, y));
        }
    }

    #[test]
    fn misses_outside_grid() {
        let r = Ray {
            origin: Vec3::new(-5.0, 10.0, -5.0),
            dir: Vec3::new(-1.0, -1.0, 0.0).normalize(),
        };
        assert!(pick(&r, 20, 20, 8.0, field).is_none());
    }
}
