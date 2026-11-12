//! Drawing the things: billboards, ground lines, rings and ranges, hit boxes and badges of
//! everything visible, and the offset of a dragged selection.
use super::*;

impl EditorApp {
    /// Whether the layers show thing `t`.
    pub(in crate::app) fn thing_visible(&self, t: &FxThing) -> bool {
        thing_visible(&self.settings.layout.layers, t)
    }

    pub(super) fn thing_frame(&self) -> f32 {
        if !self.settings.layout.layers.animate {
            return 0.0;
        }
        match self.fixed_frame {
            Some(f) => f as f32,
            // 20 game turns per second; AnimationSpeed 256 = one frame per turn
            None => self.start.elapsed().as_secs_f32() * 20.0,
        }
    }

    /// Billboards, ground lines, hit boxes and badges of everything visible.
    pub(in crate::app) fn build_things(
        &mut self,
        vp: Vec2,
        depth: &mut Vec<LineVertex>,
    ) -> Vec<SpriteInstance> {
        let mut out = vec![];
        self.hitboxes.clear();
        self.badges.clear();
        let drag_off = self
            .thing_drag
            .map(|d| self.drag_offset(d))
            .unwrap_or(Vec3::ZERO);
        let frame_t = self.thing_frame();
        let Some(doc) = &self.doc else { return out };
        let Some(lib) = self.sprites.as_mut() else {
            return out;
        };
        let gd = doc.gd.clone();
        let cam = self.camera;
        let layers = self.settings.layout.layers.clone();
        // the game's sprite angle is facing minus the view rotation; the editor's yaw turns the
        // other way round (see sprites.rs)
        let cam_angle = -(cam.yaw.to_degrees() * 2048.0 / 360.0).round() as i32;
        let (right, up) = cam.screen_axes();
        let vpm = cam.view_proj(vp);
        // the 2D view draws things as icons: their sprite fitted into about one subtile,
        // centred on the thing
        let top_down = cam.top_down;
        let selected = self.selected.clone();
        let gold = self.gold_rules;
        let lv = |p: Vec3, c: [u8; 4]| LineVertex {
            pos: p.to_array(),
            color: c,
        };
        let circle = |out: &mut Vec<LineVertex>, c: Vec3, r: f32, col: [u8; 4]| {
            let n = ((r * 12.0) as usize).clamp(16, 96);
            for k in 0..n {
                let a0 = k as f32 / n as f32 * std::f32::consts::TAU;
                let a1 = (k + 1) as f32 / n as f32 * std::f32::consts::TAU;
                out.push(lv(c + Vec3::new(a0.cos() * r, 0.0, a0.sin() * r), col));
                out.push(lv(c + Vec3::new(a1.cos() * r, 0.0, a1.sin() * r), col));
            }
        };
        // world -> pixel with the matrix computed once (Camera::project rebuilds it per call)
        let to_px = |w: Vec3| -> Option<Vec2> {
            let c = vpm * w.extend(1.0);
            (c.w > 1e-6).then(|| {
                Vec2::new(
                    (c.x / c.w + 1.0) * 0.5 * vp.x,
                    (1.0 - c.y / c.w) * 0.5 * vp.y,
                )
            })
        };
        // things far outside the view are skipped (no billboard, no hit box): a margin of the
        // largest sprite at this zoom
        let margin = if cam.fly || cam.perspective {
            vp.x.max(vp.y)
        } else {
            96.0 + 4.0 * crate::camera::PX_PER_UNIT * cam.zoom
        };
        let on_screen = |p: Vec3| {
            to_px(p).is_some_and(|q| {
                q.x > -margin && q.y > -margin && q.x < vp.x + margin && q.y < vp.y + margin
            })
        };
        let atlas = &mut self.sprite_atlas;
        let push = |out: &mut Vec<SpriteInstance>,
                    hb: &mut Vec<HitBox>,
                    item: ItemRef,
                    pos: Vec3,
                    slot: Slot,
                    flip: bool,
                    scale: f32,
                    tint: [f32; 4],
                    top: bool| {
            let x = if flip {
                -(slot.ox + slot.w as i32)
            } else {
                slot.ox
            } as f32;
            let rect = [x, slot.oy as f32, slot.w as f32, slot.h as f32];
            out.push(SpriteInstance {
                pos: pos.to_array(),
                scale,
                rect,
                uv: [slot.x as f32, slot.y as f32, slot.w as f32, slot.h as f32],
                tint,
                flags: flip as u32 | (top as u32) << 1,
            });
            // screen box from the four corners
            let mut lo = Vec2::splat(f32::MAX);
            let mut hi = Vec2::splat(f32::MIN);
            for (cx, cy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let px = rect[0] + cx * rect[2];
                let py = rect[1] + cy * rect[3];
                let w = pos + right * px * scale - up * py * scale;
                if let Some(p) = to_px(w) {
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
            }
            let c = vpm * pos.extend(1.0);
            hb.push(HitBox {
                item,
                min: lo,
                max: hi,
                depth: c.z / c.w.max(1e-6),
            });
        };
        let sel_tint = [1.0, 1.0, 0.55, 0.55];
        let white = [1.0, 1.0, 1.0, 1.0];
        // icons keep at least ~0.6 of their pixel size on screen when zoomed out
        let icon_scale = 0.02f32.max(0.6 / (crate::camera::PX_PER_UNIT * cam.zoom));
        for (i, t) in doc.doc.things.items.iter().enumerate() {
            if !thing_visible(&layers, t) {
                continue;
            }
            let item = ItemRef::Thing(i);
            let sel = selected.contains(&item);
            let Some(mut pos) = world_of(t.x, t.y, t.z) else {
                continue;
            };
            if sel {
                pos += drag_off;
            }
            if !on_screen(pos) {
                continue;
            }
            let cls = thing_class(t);
            let model = t.subtype.unwrap_or(-1);
            let level = t.creature_level.unwrap_or(1);
            let owner = t.owner.unwrap_or(5);
            let tint = if sel { sel_tint } else { white };
            match cls {
                Some(class::DOOR) => {
                    // the door is the slab; a marker above it, and a lock when locked
                    let top = pos + Vec3::Y * 4.6;
                    if let Some(s) = atlas.icon(Icon::Door) {
                        push(
                            &mut out,
                            &mut self.hitboxes,
                            item,
                            top,
                            s,
                            false,
                            icon_scale,
                            tint,
                            false,
                        );
                    }
                    if t.door_locked.unwrap_or(0) > 0 {
                        if let Some(s) = atlas.icon(Icon::Lock) {
                            push(
                                &mut out,
                                &mut self.hitboxes,
                                item,
                                top + right * 0.45,
                                s,
                                false,
                                icon_scale,
                                tint,
                                false,
                            );
                        }
                    }
                    continue;
                }
                Some(class::EFFECT_GEN) => {
                    if let Some(s) = atlas.icon(Icon::EffectGen) {
                        push(
                            &mut out,
                            &mut self.hitboxes,
                            item,
                            pos,
                            s,
                            false,
                            icon_scale,
                            tint,
                            false,
                        );
                    }
                    if sel || layers.ranges {
                        let r = t
                            .effect_range
                            .map(|c| c.stl as f32 + c.sub as f32 / 256.0)
                            .unwrap_or(0.0);
                        if r > 0.0 {
                            circle(depth, pos + Vec3::Y * 0.05, r, [255, 150, 60, 220]);
                        }
                    }
                    continue;
                }
                _ => {}
            }
            let vis = match cls {
                Some(c) => lib.visual(&gd, c, model, level),
                None => Visual::Unknown,
            };
            match vis {
                Visual::Sprite {
                    anim,
                    size,
                    speed,
                    transparent,
                } => {
                    let frame = (frame_t * speed.max(0) as f32 / 256.0) as usize;
                    let angle = t.orientation.unwrap_or(0) as i32 - cam_angle;
                    match anim
                        .pick(angle, frame)
                        .and_then(|(key, flip)| atlas.frame(lib, &anim, key).map(|s| (s, flip)))
                    {
                        Some((slot, flip)) => {
                            // gold piles grow with their value like in the game; only valuables
                            // (and the spinning coin) take a GoldValue (src/thing_factory.c:285)
                            let size = if cls == Some(class::OBJECT)
                                && (model == 128
                                    || kthings::object_genre_is(&gd, model, "VALUABLE"))
                            {
                                gold.sprite_size(model, t.gold_value, size)
                            } else {
                                size
                            };
                            let (pos, scale) = if top_down {
                                let k = (size as f32 / SPRITE_UNITS)
                                    .min(1.15 / slot.w.max(slot.h).max(1) as f32);
                                let cx = slot.ox as f32 + slot.w as f32 / 2.0;
                                let cy = slot.oy as f32 + slot.h as f32 / 2.0;
                                (
                                    pos - right * cx * k * if flip { -1.0 } else { 1.0 }
                                        + up * cy * k
                                        + Vec3::Y * 0.5,
                                    k,
                                )
                            } else {
                                (pos, size as f32 / SPRITE_UNITS)
                            };
                            let tint = if layers.owner_tint && cls == Some(class::CREATURE) {
                                let c = owner_rgb(owner);
                                let m = |v: u8| 0.7 + 0.3 * v as f32 / 255.0;
                                if sel {
                                    sel_tint
                                } else {
                                    [m(c[0]), m(c[1]), m(c[2]), 1.0]
                                }
                            } else {
                                tint
                            };
                            push(
                                &mut out,
                                &mut self.hitboxes,
                                item,
                                pos,
                                slot,
                                flip,
                                scale,
                                tint,
                                false,
                            );
                            if transparent != 0 {
                                if let Some(last) = out.last_mut() {
                                    last.flags |= 4;
                                }
                            }
                        }
                        None => {
                            if let Some(s) = atlas.icon(Icon::Unknown) {
                                push(
                                    &mut out,
                                    &mut self.hitboxes,
                                    item,
                                    pos,
                                    s,
                                    false,
                                    icon_scale,
                                    tint,
                                    false,
                                );
                            }
                        }
                    }
                }
                Visual::Marker => {
                    if let Some(s) = atlas.icon(Icon::Marker) {
                        push(
                            &mut out,
                            &mut self.hitboxes,
                            item,
                            pos + Vec3::Y * 0.1,
                            s,
                            false,
                            icon_scale,
                            tint,
                            false,
                        );
                    }
                }
                Visual::Unknown => {
                    if let Some(s) = atlas.icon(Icon::Unknown) {
                        push(
                            &mut out,
                            &mut self.hitboxes,
                            item,
                            pos,
                            s,
                            false,
                            icon_scale,
                            tint,
                            false,
                        );
                    }
                }
            }
            if (sel || layers.facing_arrows)
                && matches!(
                    cls,
                    Some(class::CREATURE) | Some(class::OBJECT) | Some(class::TRAP)
                )
            {
                // facing: game angle a points along map (sin a, -cos a) (camera.rs module docs)
                let a = t.orientation.unwrap_or(0) as f32 * std::f32::consts::TAU / 2048.0;
                let dir = Vec3::new(a.sin(), 0.0, -a.cos());
                let side = Vec3::new(-dir.z, 0.0, dir.x);
                let base = pos + Vec3::Y * 0.07;
                let tip = base + dir * 0.85;
                let col = [255, 235, 120, 255];
                for (p0, p1) in [
                    (base, tip),
                    (tip, tip - dir * 0.28 + side * 0.18),
                    (tip, tip - dir * 0.28 - side * 0.18),
                ] {
                    depth.push(lv(p0, col));
                    depth.push(lv(p1, col));
                }
            }
            if cls == Some(class::CREATURE) {
                let c = owner_rgb(owner);
                circle(depth, pos + Vec3::Y * 0.04, 0.42, [c[0], c[1], c[2], 255]);
                circle(depth, pos + Vec3::Y * 0.04, 0.36, [c[0], c[1], c[2], 255]);
                if layers.level_badges {
                    self.badges.push(Badge {
                        pos,
                        text: level.to_string(),
                        color: c,
                    });
                }
            }
            if cls == Some(class::OBJECT) && layers.gate_numbers && kthings::is_hero_gate(&gd, t) {
                self.badges.push(Badge {
                    pos: pos + Vec3::Y * 2.5,
                    text: trf(
                        "kfx_editor.gate_herogate_number",
                        "gate {herogate_number}",
                        &[("herogate_number", &t.herogate_number.unwrap_or(0))],
                    ),
                    color: [255, 255, 255],
                });
            }
        }
        if layers.action_points {
            for (i, a) in doc.doc.action_points.items.iter().enumerate() {
                let item = ItemRef::ActionPoint(i);
                let sel = selected.contains(&item);
                let Some(p) = world_of(a.x, a.y, None) else {
                    continue;
                };
                let h = doc.doc.floor_height(p.x as i32, p.z as i32) as f32;
                let mut pos = Vec3::new(p.x, h, p.z);
                if sel {
                    pos += drag_off;
                }
                if let Some(s) = atlas.icon(Icon::ActionPoint) {
                    push(
                        &mut out,
                        &mut self.hitboxes,
                        item,
                        pos,
                        s,
                        false,
                        icon_scale,
                        if sel { sel_tint } else { white },
                        false,
                    );
                }
                self.badges.push(Badge {
                    pos: pos + Vec3::Y * 1.0,
                    text: trf(
                        "kfx_editor.ap_n",
                        "AP {n}",
                        &[("n", &a.number.unwrap_or(0))],
                    ),
                    color: [120, 210, 255],
                });
                if sel || layers.ranges {
                    let r = a
                        .range
                        .map(|c| c.stl as f32 + c.sub as f32 / 256.0)
                        .unwrap_or(0.0);
                    circle(depth, pos + Vec3::Y * 0.05, r.max(0.3), [80, 200, 255, 230]);
                }
            }
        }
        if layers.lights {
            for (i, l) in doc.doc.lights.items.iter().enumerate() {
                if l.parent_tile.is_some() && !layers.slab_things {
                    continue;
                }
                let item = ItemRef::Light(i);
                let sel = selected.contains(&item);
                let Some(mut pos) = world_of(l.x, l.y, l.z) else {
                    continue;
                };
                if sel {
                    pos += drag_off;
                }
                if !on_screen(pos) {
                    continue;
                }
                if let Some(s) = atlas.icon(Icon::Light) {
                    // lights are many (every wall lamp): smaller unless selected
                    push(
                        &mut out,
                        &mut self.hitboxes,
                        item,
                        pos,
                        s,
                        false,
                        if sel { icon_scale } else { icon_scale * 0.6 },
                        if sel { sel_tint } else { white },
                        false,
                    );
                }
                if sel || layers.ranges {
                    let r = l
                        .range
                        .map(|c| c.stl as f32 + c.sub as f32 / 256.0)
                        .unwrap_or(0.0);
                    let h = doc.doc.floor_height(pos.x as i32, pos.z as i32) as f32;
                    circle(
                        depth,
                        Vec3::new(pos.x, h + 0.05, pos.z),
                        r.max(0.3),
                        [255, 230, 120, 220],
                    );
                }
            }
        }
        // selection outlines on the ground
        for it in &selected {
            let p = match *it {
                ItemRef::Thing(i) => doc
                    .doc
                    .things
                    .items
                    .get(i)
                    .and_then(|t| world_of(t.x, t.y, t.z)),
                ItemRef::ActionPoint(i) => doc
                    .doc
                    .action_points
                    .items
                    .get(i)
                    .and_then(|a| world_of(a.x, a.y, None))
                    .map(|p| {
                        Vec3::new(
                            p.x,
                            doc.doc.floor_height(p.x as i32, p.z as i32) as f32,
                            p.z,
                        )
                    }),
                ItemRef::Light(i) => doc
                    .doc
                    .lights
                    .items
                    .get(i)
                    .and_then(|l| world_of(l.x, l.y, l.z)),
            };
            if let Some(p) = p {
                let p = p + drag_off;
                let h = doc.doc.floor_height(p.x as i32, p.z as i32) as f32;
                circle(
                    depth,
                    Vec3::new(p.x, h + 0.06, p.z),
                    0.55,
                    [255, 240, 120, 255],
                );
            }
        }
        out
    }

    /// World offset of the selection while it is dragged (whole subtiles unless free).
    pub(in crate::app) fn drag_offset(&self, d: ThingDrag) -> Vec3 {
        let dx = d.cur.x - d.start.x;
        let dz = d.cur.z - d.start.z;
        if self.free_place {
            Vec3::new(dx, 0.0, dz)
        } else {
            Vec3::new(dx.round(), 0.0, dz.round())
        }
    }
}
