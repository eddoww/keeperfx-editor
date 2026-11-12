//! The viewport: camera reset, picking, the tool preview and line overlays, rendering the 3D
//! view, the 2D view's flat colours, the split view and the hover readout.
use super::*;

impl EditorApp {
    pub(super) fn anim_frame(&self) -> u32 {
        // 20 game turns per second, frame advances every 2 turns (src/engine_textures.c:92-96).
        self.fixed_frame
            .unwrap_or_else(|| ((self.start.elapsed().as_secs_f64() * 10.0) as u64 % 8) as u32)
    }

    pub(super) fn reset_camera(&mut self, vp: Vec2) {
        self.set_fly_capture(false);
        let persp = self.camera.perspective;
        if self.camera.top_down {
            // the 2D pane: top-down, north up, the whole map
            self.camera =
                Camera::top_down_home(self.terrain.stl_w.max(1), self.terrain.stl_h.max(1), vp);
            self.camera.perspective = persp;
            return;
        }
        if self.doc.is_some() {
            self.camera = Camera::home(self.terrain.stl_w, self.terrain.stl_h, vp, persp);
        } else {
            self.camera = Camera {
                perspective: persp,
                ..Default::default()
            };
        }
        if let Some(s) = self.camera_spec.take() {
            self.camera.target = Vec3::new(s.x, 0.0, s.y);
            self.camera.zoom = (s.zoom_pct / 100.0).clamp(MIN_ZOOM, MAX_ZOOM);
            self.camera.yaw = s.yaw_deg.to_radians();
            self.camera.pitch = s
                .pitch_deg
                .to_radians()
                .clamp(-89f32.to_radians(), -5f32.to_radians());
        }
    }

    pub(super) fn pick_at(&self, px: Vec2, vp: Vec2) -> Option<HoverInfo> {
        self.doc.as_ref()?;
        let t = &self.terrain;
        let ray = self.camera.ray(px, vp);
        let hit = pick(&ray, t.stl_w, t.stl_h, 8.0, |x, y| {
            t.top_height(x, y) as f32
        })?;
        Some(HoverInfo {
            slab: (hit.stl_x / 3, hit.stl_y / 3),
            hit,
        })
    }

    /// Cells to preview with their colours.
    pub(super) fn preview(&self) -> Vec<((i32, i32), [u8; 4])> {
        let (Some(doc), Some(gd)) = (&self.doc, self.gd()) else {
            return vec![];
        };
        let base = if self.owner_only {
            let c = PLAYERS[self.active_owner as usize].1;
            [c[0], c[1], c[2], 120]
        } else {
            match (self.custom_brush, self.brush) {
                (Some(_), _) => [230, 160, 255, 130],
                (None, Some(k)) => {
                    let c = assets::slab_color(gd, k, self.owner_for(k));
                    [c[0], c[1], c[2], 120]
                }
                (None, None) => [255, 255, 255, 50],
            }
        };
        let mark = |cells: Vec<(i32, i32)>, c: [u8; 4]| -> Vec<((i32, i32), [u8; 4])> {
            cells
                .into_iter()
                .filter(|&(x, y)| doc.in_map(x, y))
                .map(|p| {
                    (
                        p,
                        if self.editable(p.0, p.1) {
                            c
                        } else {
                            [255, 30, 30, 110]
                        },
                    )
                })
                .collect()
        };
        let hover = self
            .hover
            .as_ref()
            .map(|h| (h.slab.0 as i32, h.slab.1 as i32));
        if let Some(d) = &self.drag {
            return match d.tool {
                Tool::Paint => vec![],
                Tool::Select => mark(self.shape_cells(d), [90, 170, 255, 70]),
                _ => mark(self.shape_cells(d), base),
            };
        }
        let Some(p) = hover else { return vec![] };
        if self.pasting {
            if let Some(c) = &self.clipboard {
                return c
                    .stamp(p)
                    .into_iter()
                    .filter(|&(x, y, _, _)| doc.in_map(x, y))
                    .map(|(x, y, k, o)| {
                        let col = assets::slab_color(gd, k, o);
                        (
                            (x, y),
                            if self.editable(x, y) {
                                [col[0], col[1], col[2], 150]
                            } else {
                                [255, 30, 30, 110]
                            },
                        )
                    })
                    .collect();
            }
        }
        match self.tool {
            Tool::Paint | Tool::Line => {
                let cells = tools::brush_cells(self.brush_size, self.brush_round)
                    .into_iter()
                    .map(|(x, y)| (x + p.0, y + p.1))
                    .collect();
                mark(cells, base)
            }
            Tool::Eyedropper => vec![],
            _ => mark(vec![p], base),
        }
    }

    pub(super) fn overlay(&self) -> (Vec<LineVertex>, Vec<LineVertex>, Vec<LineVertex>) {
        let mut depth = vec![];
        let mut top = vec![];
        let mut tris = vec![];
        let Some(doc) = &self.doc else {
            return (depth, top, tris);
        };
        let t = &self.terrain;
        let lv = |p: Vec3, c: [u8; 4]| LineVertex {
            pos: p.to_array(),
            color: c,
        };
        let rect =
            |out: &mut Vec<LineVertex>, x0: f32, z0: f32, x1: f32, z1: f32, y: f32, c: [u8; 4]| {
                let pts = [
                    Vec3::new(x0, y, z0),
                    Vec3::new(x1, y, z0),
                    Vec3::new(x1, y, z1),
                    Vec3::new(x0, y, z1),
                ];
                for i in 0..4 {
                    out.push(lv(pts[i], c));
                    out.push(lv(pts[(i + 1) % 4], c));
                }
            };
        // map outline (the edge of the map, also where the border ring is not rock)
        {
            let dm = doc.dims();
            rect(
                &mut top,
                0.0,
                0.0,
                dm.w as f32 * 3.0,
                dm.h as f32 * 3.0,
                0.03,
                [200, 170, 90, 150],
            );
        }
        // symmetry guide lines
        if let Some(sym) = &self.symmetry {
            let dm = doc.dims();
            let (w, h) = (dm.w as f32 * 3.0, dm.h as f32 * 3.0);
            let c = [255, 120, 220, 200];
            let y = 0.08;
            use kfx_map::transform::SymmetryMode as M;
            if matches!(sym.mode, M::NorthSouth | M::FourWay | M::Rot2 | M::Rot4) {
                top.push(lv(Vec3::new(0.0, y, h / 2.0), c));
                top.push(lv(Vec3::new(w, y, h / 2.0), c));
            }
            if matches!(sym.mode, M::EastWest | M::FourWay | M::Rot4) {
                top.push(lv(Vec3::new(w / 2.0, y, 0.0), c));
                top.push(lv(Vec3::new(w / 2.0, y, h), c));
            }
        }
        self.locked_overlay(&mut top);
        self.logic_overlay(&mut top);
        // slabs whose definition the configuration editor changed flash
        if let Some(c) = self
            .config
            .as_ref()
            .filter(|c| c.open && !c.affected.is_empty())
        {
            let kinds: HashSet<u16> = c.affected.iter().map(|a| a.0).collect();
            let phase = (self.ctx.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
            let col = [255, 90, 200, (60.0 + 120.0 * phase) as u8];
            let dm = doc.dims();
            for sy in 0..dm.h as usize {
                for sx in 0..dm.w as usize {
                    let i = sy * dm.w as usize + sx;
                    if doc.doc.custom_cells[i] != 0 || !kinds.contains(&doc.doc.kinds[i]) {
                        continue;
                    }
                    let h = t.slab_top(sx, sy) as f32 + 0.07;
                    let (x0, z0) = (sx as f32 * 3.0, sy as f32 * 3.0);
                    let q = [
                        Vec3::new(x0, h, z0),
                        Vec3::new(x0, h, z0 + 3.0),
                        Vec3::new(x0 + 3.0, h, z0 + 3.0),
                        Vec3::new(x0 + 3.0, h, z0),
                    ];
                    for k in [0, 1, 2, 0, 2, 3] {
                        tris.push(lv(q[k], col));
                    }
                }
            }
            self.ctx.request_repaint_after(Duration::from_millis(50));
        }
        // tool preview: translucent slab tops
        for ((sx, sy), c) in self.preview() {
            let h = t.slab_top(sx as usize, sy as usize) as f32 + 0.06;
            let (x0, z0) = (sx as f32 * 3.0, sy as f32 * 3.0);
            let (x1, z1) = (x0 + 3.0, z0 + 3.0);
            let q = [
                Vec3::new(x0, h, z0),
                Vec3::new(x0, h, z1),
                Vec3::new(x1, h, z1),
                Vec3::new(x1, h, z0),
            ];
            for i in [0, 1, 2, 0, 2, 3] {
                tris.push(lv(q[i], c));
            }
            rect(
                &mut depth,
                x0 + 0.05,
                z0 + 0.05,
                x1 - 0.05,
                z1 - 0.05,
                h,
                [c[0], c[1], c[2], 230],
            );
        }
        // selection
        if let Some(((ax, ay), (bx, by))) = self.selection {
            let (x0, z0, x1, z1) = (
                ax as f32 * 3.0,
                ay as f32 * 3.0,
                (bx + 1) as f32 * 3.0,
                (by + 1) as f32 * 3.0,
            );
            let c = [110, 190, 255, 255];
            rect(&mut top, x0, z0, x1, z1, 0.05, c);
            rect(&mut top, x0, z0, x1, z1, 5.05, [110, 190, 255, 140]);
            for (cx, cz) in [(x0, z0), (x1, z0), (x1, z1), (x0, z1)] {
                top.push(lv(Vec3::new(cx, 0.05, cz), [110, 190, 255, 140]));
                top.push(lv(Vec3::new(cx, 5.05, cz), [110, 190, 255, 140]));
            }
        }
        if let Some(h) = &self.hover {
            let (sx, sy) = (h.slab.0 as f32 * 3.0, h.slab.1 as f32 * 3.0);
            let sh = t.slab_top(h.slab.0, h.slab.1) as f32;
            let gold = [255, 200, 80, 255];
            rect(&mut top, sx, sy, sx + 3.0, sy + 3.0, sh + 0.02, gold);
            rect(
                &mut top,
                sx,
                sy,
                sx + 3.0,
                sy + 3.0,
                0.02,
                [255, 200, 80, 140],
            );
            for (cx, cz) in [
                (sx, sy),
                (sx + 3.0, sy),
                (sx + 3.0, sy + 3.0),
                (sx, sy + 3.0),
            ] {
                top.push(lv(Vec3::new(cx, 0.02, cz), [255, 200, 80, 140]));
                top.push(lv(Vec3::new(cx, sh + 0.02, cz), [255, 200, 80, 140]));
            }
            let (x, z) = (h.hit.stl_x as f32, h.hit.stl_y as f32);
            rect(
                &mut top,
                x + 0.04,
                z + 0.04,
                x + 0.96,
                z + 0.96,
                h.hit.height + 0.03,
                [255, 255, 255, 230],
            );
        }
        (depth, top, tris)
    }

    /// Render the 3D view at `size` pixels; returns the texture id for egui.
    pub fn render_view(&mut self, size: (u32, u32)) -> Option<egui::TextureId> {
        let vp = Vec2::new(size.0 as f32, size.1 as f32);
        self.last_vp = vp;
        if self.camera_pending_home {
            self.reset_camera(vp);
            self.camera_pending_home = false;
        }
        let l = &self.settings.layout;
        let flat = self.camera.top_down && l.flat_colors;
        self.view.flags = ViewFlags {
            ownership: l.show_ownership,
            grid: l.show_grid,
            lighting: l.lighting,
            terrain: l.layers.terrain && !flat,
        };
        self.view.clear_alpha = flat;
        self.view.tone = self.tone();
        let (mut dl, tl, mut tris) = self.overlay();
        self.analysis_tris(&mut tris);
        let tt = Instant::now();
        let sprites = self.build_things(vp, &mut dl);
        self.perf.things_ms = tt.elapsed().as_secs_f32() * 1000.0;
        self.perf.billboards = sprites.len();
        self.view
            .upload_sprites(&self.gpu.queue, &mut self.sprite_atlas);
        let t0 = Instant::now();
        self.view.render(
            &self.gpu.device,
            &self.gpu.queue,
            &self.camera,
            size,
            self.anim_frame(),
            &dl,
            &tl,
            &tris,
            &sprites,
        );
        self.perf.frame_cpu_ms = t0.elapsed().as_secs_f32() * 1000.0;
        let (_, view) = self.view.output()?;
        let mut r = self.gpu.egui_renderer.write();
        match self.view_tex {
            Some(id) if self.view_size == size => Some(id),
            Some(id) => {
                let f = if self.settings.graphics.smooth_scaling {
                    wgpu::FilterMode::Linear
                } else {
                    wgpu::FilterMode::Nearest
                };
                r.update_egui_texture_from_wgpu_texture(&self.gpu.device, view, f, id);
                self.view_size = size;
                Some(id)
            }
            None => {
                let f = if self.settings.graphics.smooth_scaling {
                    wgpu::FilterMode::Linear
                } else {
                    wgpu::FilterMode::Nearest
                };
                let id = r.register_native_texture(&self.gpu.device, view, f);
                self.view_tex = Some(id);
                self.view_size = size;
                Some(id)
            }
        }
    }

    /// The view's tone: exposure lifted when zoomed out (whole-map overviews), the map's lights
    /// near the camera when zoomed in.
    pub(super) fn tone(&self) -> crate::render::Tone {
        let g = &self.settings.graphics;
        let z = self.camera.zoom;
        let exposure = if g.overview_boost && self.settings.layout.lighting {
            // 1 at 60% and closer, 2.4 at 15% and further
            let t = ((0.6 - z) / 0.45).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            1.0 + 1.4 * t
        } else {
            1.0
        };
        let mut lights = vec![];
        if g.local_lights && self.settings.layout.lighting && z >= 1.2 {
            if let Some(d) = &self.doc {
                let c = self.camera.target;
                let mut all: Vec<(f32, crate::render::LocalLight)> = d
                    .doc
                    .lights
                    .items
                    .iter()
                    .filter_map(|l| {
                        let x = l.x?.stl as f32 + l.x?.sub as f32 / 256.0;
                        let y = l.y?.stl as f32 + l.y?.sub as f32 / 256.0;
                        let z = l.z.map_or(0.0, |z| z.stl as f32 + z.sub as f32 / 256.0);
                        let range = l.range.map_or(0.0, |r| r.stl as f32 + r.sub as f32 / 256.0);
                        let pos = Vec3::new(x, z, y);
                        let dist = (Vec2::new(pos.x, pos.z) - Vec2::new(c.x, c.z)).length();
                        (range > 0.0).then_some((
                            dist,
                            crate::render::LocalLight {
                                pos,
                                range: range * 1.5,
                                power: (l.intensity.unwrap_or(32) as f32 / 64.0).clamp(0.1, 1.0),
                            },
                        ))
                    })
                    .collect();
                // objects that light up in the game (torches, candles: objects.cfg LightRadius /
                // LightIntensity, config_objects.c)
                for t in &d.doc.things.items {
                    if kfx_map::things::thing_class(t) != Some(kfx_map::things::class::OBJECT) {
                        continue;
                    }
                    let (Some(x), Some(y)) = (t.x, t.y) else {
                        continue;
                    };
                    let Some(sec) =
                        d.gd.objects_cfg
                            .section(&format!("object{}", t.subtype.unwrap_or(-1)))
                    else {
                        continue;
                    };
                    let num = |k: &str| {
                        sec.get(k)
                            .and_then(|v| v.split_whitespace().next())
                            .and_then(kfx_map::gamedata::parse_number)
                            .unwrap_or(0)
                    };
                    let (r, i) = (num("LightRadius"), num("LightIntensity"));
                    if r <= 0 || i <= 0 {
                        continue;
                    }
                    let pos = Vec3::new(
                        x.stl as f32 + x.sub as f32 / 256.0,
                        t.z.map_or(1.0, |z| z.stl as f32 + z.sub as f32 / 256.0),
                        y.stl as f32 + y.sub as f32 / 256.0,
                    );
                    let dist = (Vec2::new(pos.x, pos.z) - Vec2::new(c.x, c.z)).length();
                    all.push((
                        dist,
                        crate::render::LocalLight {
                            pos,
                            range: r as f32 * 1.2,
                            power: (i as f32 / 64.0).clamp(0.1, 1.0),
                        },
                    ));
                }
                all.sort_by(|a, b| a.0.total_cmp(&b.0));
                // fade in between 120% and 200%
                let k = ((z - 1.2) / 0.8).clamp(0.0, 1.0);
                lights = all
                    .into_iter()
                    .take(crate::render::MAX_LIGHTS)
                    .map(|(_, mut l)| {
                        l.power *= k * 0.6;
                        l
                    })
                    .collect();
            }
        }
        crate::render::Tone { exposure, lights }
    }

    /// 2D flat colours: the slab kind / owner colours (the minimap image) under the view.
    pub(super) fn flat_colors(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        vp: Vec2,
        ppp: f32,
    ) {
        let Some(doc) = &self.doc else { return };
        if self.minimap_dirty || self.minimap_tex.is_none() {
            let img = minimap::image(&doc.doc, &doc.gd);
            match &mut self.minimap_tex {
                Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                None => {
                    self.minimap_tex = Some(self.ctx.load_texture(
                        "minimap",
                        img,
                        egui::TextureOptions::NEAREST,
                    ))
                }
            }
            self.minimap_dirty = false;
        }
        let Some(tex) = &self.minimap_tex else { return };
        let Some(doc) = &self.doc else { return };
        let dm = doc.dims();
        let (w, h) = (dm.w as f32 * 3.0, dm.h as f32 * 3.0);
        let to_ui = |p: Vec2| rect.min + EVec2::new(p.x / ppp, p.y / ppp);
        let corners = [
            (Vec3::new(0.0, 0.0, 0.0), (0.0, 0.0)),
            (Vec3::new(w, 0.0, 0.0), (1.0, 0.0)),
            (Vec3::new(w, 0.0, h), (1.0, 1.0)),
            (Vec3::new(0.0, 0.0, h), (0.0, 1.0)),
        ];
        let mut mesh = egui::Mesh::with_texture(tex.id());
        for (p, uv) in corners {
            let Some(q) = self.camera.project(p, vp) else {
                return;
            };
            mesh.vertices.push(egui::epaint::Vertex {
                pos: to_ui(q),
                uv: egui::pos2(uv.0, uv.1),
                color: Color32::WHITE,
            });
        }
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        painter.rect_filled(rect, 0.0, Color32::from_rgb(9, 8, 7));
        painter.add(egui::Shape::mesh(mesh));
    }

    /// Hover at a pixel of a viewport of `size` (headless `--pointer`).
    pub fn hover_pixel(&mut self, px: Vec2, size: (u32, u32)) {
        let vp = Vec2::new(size.0 as f32, size.1 as f32);
        if self.camera_pending_home {
            self.reset_camera(vp);
            self.camera_pending_home = false;
        }
        self.hover = self.pick_at(px, vp);
    }

    pub fn view_renderer(&self) -> &ViewRenderer {
        &self.view
    }

    /// Exchange the main (3D) pane's camera, render slot and per-pane state with the 2D pane's.
    pub(super) fn swap_pane(&mut self) {
        std::mem::swap(&mut self.camera, &mut self.camera2);
        std::mem::swap(&mut self.last_vp, &mut self.last_vp2);
        std::mem::swap(&mut self.camera_pending_home, &mut self.camera2_home);
        std::mem::swap(&mut self.view_tex, &mut self.view_tex2);
        std::mem::swap(&mut self.view_size, &mut self.view_size2);
        std::mem::swap(&mut self.hitboxes, &mut self.hitboxes2);
        std::mem::swap(&mut self.badges, &mut self.badges2);
        std::mem::swap(&mut self.markers, &mut self.markers2);
        self.view.slot ^= 1;
    }

    /// The map view: 3D, 2D (top-down) or both side by side.
    pub(super) fn viewport_ui(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let mode = if self.doc.is_some() {
            self.settings.layout.view_mode
        } else {
            crate::settings::ViewMode::ThreeD
        };
        match mode {
            crate::settings::ViewMode::ThreeD => {
                self.panes = vec![full];
                self.pane_ui(ui, full, 0);
            }
            crate::settings::ViewMode::TwoD => {
                self.panes = vec![full];
                self.swap_pane();
                self.pane_ui(ui, full, 1);
                self.swap_pane();
            }
            crate::settings::ViewMode::Split => {
                let mid = full.center().x.round();
                let left = egui::Rect::from_min_max(full.min, egui::pos2(mid - 1.0, full.max.y));
                let right = egui::Rect::from_min_max(egui::pos2(mid + 1.0, full.min.y), full.max);
                self.panes = vec![left, right];
                self.swap_pane();
                self.pane_ui(ui, left, 1);
                self.swap_pane();
                self.pane_ui(ui, right, 0);
                ui.painter().line_segment(
                    [egui::pos2(mid, full.min.y), egui::pos2(mid, full.max.y)],
                    egui::Stroke::new(2.0, GOLD_DIM),
                );
            }
        }
    }

    /// The hover readout, e.g. `(72,69) id 2 DIRT [solid] owner 2`.
    pub fn readout(&self) -> Option<String> {
        let h = self.hover.as_ref()?;
        let doc = self.doc.as_ref()?;
        let s = doc.state_at(h.slab.0 as i32, h.slab.1 as i32)?;
        let gd = &doc.gd;
        let solid = if assets::is_solid(gd, s.kind as usize) {
            " [solid]"
        } else {
            ""
        };
        let solid = format!(
            "{solid}{}{}",
            doc.doc
                .custom_slab_at(h.slab.0 as i32, h.slab.1 as i32)
                .map(|c| trf(
                    "kfx_editor.custom_id_name",
                    " [custom {id}: {name}]",
                    &[("id", &c.id), ("name", &c.name)]
                ))
                .unwrap_or_default(),
            if self.is_locked(h.slab.0 as i32, h.slab.1 as i32) {
                " [locked]"
            } else {
                ""
            }
        );
        let col = self.terrain.column_at(h.hit.stl_x, h.hit.stl_y);
        Some(trf(
            "kfx_editor.slab_slab2_id_kind_kind2_solid_owner_owner_subti",
            "({slab},{slab2}) ID {kind} {kind2}{solid} owner {owner}\nsubtile \
            ({stl_x},{stl_y}) height {height} cubes {solidmask}",
            &[
                ("slab", &h.slab.0),
                ("slab2", &h.slab.1),
                ("kind", &s.kind),
                ("kind2", &gd.slab_name(s.kind as usize).unwrap_or("?")),
                ("solid", &solid),
                ("owner", &s.owner),
                ("stl_x", &h.hit.stl_x),
                ("stl_y", &h.hit.stl_y),
                ("height", &h.hit.height),
                (
                    "solidmask",
                    &format!(
                        "{:?}",
                        &col.cubes[..assets::floor_height(col.solidmask).max(1)]
                    ),
                ),
            ],
        ))
    }
}
