//! The offscreen editor: a wgpu device without a surface, the egui context fed with
//! simulated input, and the frame capture that writes PNGs.
use super::*;

/// Create a device without a surface. Tries a hardware adapter first, then wgpu's fallback
/// (software) adapter.
pub fn create_device() -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env()
            .unwrap_or(wgpu::Backends::PRIMARY | wgpu::Backends::GL),
        flags: wgpu::InstanceFlags::from_build_config().with_env(),
        backend_options: wgpu::BackendOptions::from_env_or_default(),
        memory_budget_thresholds: Default::default(),
        display: None,
    });
    let mut adapter = None;
    let mut errs = vec![];
    for force_fallback_adapter in [false, true] {
        match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:
                wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance),
            compatible_surface: None,
            force_fallback_adapter,
            apply_limit_buckets: false,
        })) {
            Ok(a) => {
                adapter = Some(a);
                break;
            }
            Err(e) => errs.push(e.to_string()),
        }
    }
    let adapter = adapter.ok_or_else(|| format!("no wgpu adapter: {}", errs.join("; ")))?;
    let base = if adapter.get_info().backend == wgpu::Backend::Gl {
        wgpu::Limits::downlevel_webgl2_defaults()
    } else {
        wgpu::Limits::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("kfx-editor headless"),
        required_limits: wgpu::Limits {
            max_texture_dimension_2d: 8192,
            ..base
        },
        ..Default::default()
    }))
    .map_err(|e| e.to_string())?;
    Ok((adapter, device, queue))
}

pub(super) fn write_png(path: &PathBuf, w: u32, h: u32, data: Vec<u8>) -> Result<(), String> {
    let img = kfx_gfx::Rgba { w, h, data };
    let png = img.to_png().map_err(|e| e.to_string())?;
    if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(path, png).map_err(|e| format!("{}: {e}", path.display()))
}

/// Debug aid: a top-down image of the slabs (4 px per slab, north up, minimap colours), to
/// compare the layout of the 3D view with the `.slb`.
pub(super) fn slab_image(app: &EditorApp, path: &PathBuf) -> Result<(), String> {
    let doc = app.doc.as_ref().ok_or("no map")?;
    let img = crate::minimap::image(&doc.doc, &doc.gd);
    let (w, h) = (img.size[0] as u32, img.size[1] as u32);
    let s = 4u32;
    let mut out = kfx_gfx::Rgba::new(w * s, h * s);
    for y in 0..h {
        for x in 0..w {
            let c = img.pixels[(y * w + x) as usize];
            for dy in 0..s {
                for dx in 0..s {
                    out.put(x * s + dx, y * s + dy, [c.r(), c.g(), c.b(), 255]);
                }
            }
        }
    }
    write_png(path, out.w, out.h, out.data)
}

pub(super) struct Headless {
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) gpu: Gpu,
    pub(super) ctx: egui::Context,
    pub(super) time: f64,
    /// Pointer position for the next UI captures (`pointer` action), overrides `--pointer`.
    pub(super) pointer: Option<(f32, f32)>,
    /// The recording (`--record`) and the simulated input of the recording actions.
    pub(crate) rec: crate::record::Recorder,
}

impl Headless {
    /// Render the full UI at `size` and read it back.
    pub(super) fn capture_ui(
        &mut self,
        app: &mut EditorApp,
        size: (u32, u32),
        pointer: Option<(f32, f32)>,
    ) -> Result<(u32, u32, Vec<u8>), String> {
        let (w, h) = size;
        let mut out = None;
        for _ in 0..4 {
            // the UI scale (Preferences) zooms: the same pixels hold fewer points (read every
            // frame: a new zoom factor takes effect at the start of the next pass)
            let zoom = self.ctx.zoom_factor();
            let pointer = pointer.map(|(x, y)| (x / zoom, y / zoom));
            self.time += 1.0;
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(w as f32 / zoom, h as f32 / zoom),
                )),
                // large steps so that fade-in animations are finished in the captured frame
                time: Some(self.time),
                predicted_dt: 1.0 / 30.0,
                focused: true,
                ..Default::default()
            };
            if let Some(v) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
                v.native_pixels_per_point = Some(1.0);
            }
            if let Some((x, y)) = pointer {
                raw.events.push(egui::Event::PointerMoved(egui::pos2(x, y)));
            }
            let mut full = self.ctx.run_ui(raw, |ui| app.ui(ui));
            {
                let mut r = self.gpu.egui_renderer.write();
                for (id, deltas) in &full.textures_delta.set {
                    for delta in deltas {
                        r.update_texture(&self.device, &self.queue, *id, delta);
                    }
                }
                for id in &full.textures_delta.free {
                    r.free_texture(id);
                }
            }
            full.textures_delta.clear();
            out = Some(full);
        }
        let full = out.unwrap();
        self.check_forbidden(&full.shapes)?;
        self.render_output(full, size)
    }

    /// Fail when the frame shows one of the `--forbid-text` texts; keep its texts for the
    /// text targets of the recording actions.
    pub(super) fn check_forbidden(
        &mut self,
        shapes: &[egui::epaint::ClippedShape],
    ) -> Result<(), String> {
        let mut texts = vec![];
        crate::record::collect_texts(shapes, &mut texts);
        if let Some((f, t)) = crate::record::forbidden(&texts, &self.rec.forbid) {
            return Err(format!(
                "frame {} shows the forbidden text {f:?} in {t:?}",
                self.rec.written
            ));
        }
        self.rec.texts = texts;
        Ok(())
    }

    /// Tessellate and draw a UI frame, read it back.
    pub(super) fn render_output(
        &mut self,
        full: egui::FullOutput,
        size: (u32, u32),
    ) -> Result<(u32, u32, Vec<u8>), String> {
        let (w, h) = size;
        let prims = self.ctx.tessellate(full.shapes, full.pixels_per_point);
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let sd = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [w, h],
            pixels_per_point: full.pixels_per_point,
        };
        let mut enc = self.device.create_command_encoder(&Default::default());
        let mut r = self.gpu.egui_renderer.write();
        let cmds = r.update_buffers(&self.device, &self.queue, &mut enc, &prims, &sd);
        {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            r.render(&mut pass, &prims, &sd);
        }
        self.queue.submit(cmds.into_iter().chain([enc.finish()]));
        drop(r);
        read_texture(&self.device, &self.queue, &target, (w, h))
            .ok_or_else(|| "read back failed".into())
    }

    /// One UI frame at `size` without read-back (the 3D view renders inside it), waiting for
    /// the GPU. For the frame time benchmarks.
    pub(super) fn frame_ui(&mut self, app: &mut EditorApp, size: (u32, u32)) {
        let zoom = self.ctx.zoom_factor();
        self.time += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size.0 as f32 / zoom, size.1 as f32 / zoom),
            )),
            time: Some(self.time),
            predicted_dt: 1.0 / 60.0,
            focused: true,
            ..Default::default()
        };
        let mut full = self.ctx.run_ui(raw, |ui| app.ui(ui));
        {
            let mut r = self.gpu.egui_renderer.write();
            for (id, deltas) in &full.textures_delta.set {
                for delta in deltas {
                    r.update_texture(&self.device, &self.queue, *id, delta);
                }
            }
            for id in &full.textures_delta.free {
                r.free_texture(id);
            }
        }
        full.textures_delta.clear();
        let _ = self.ctx.tessellate(full.shapes, full.pixels_per_point);
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// One simulated frame of a recording: the pointer where the recorder has it, the given
    /// input events, time advancing by one frame; the frame is written when the recording
    /// writes this one (see `stride`).
    pub(super) fn step(
        &mut self,
        app: &mut EditorApp,
        o: &Options,
        events: Vec<egui::Event>,
    ) -> Result<(), String> {
        let (w, h) = o.size;
        let fps = self.rec.fps as f64;
        self.time += 1.0 / fps;
        if self.rec.anim {
            // the editor animates liquids at 10 frames per second
            app.fixed_frame = Some(((self.rec.sim as f64 / fps * 10.0) as u64 % 8) as u32);
        }
        if self.rec.view_only {
            // only the 3D view at the full frame size (no UI, no pointer)
            if self.rec.writes_next() {
                let (rw, rh, px) = self.capture_view(app, o.size)?;
                self.rec.write(rw, rh, px)?;
            }
            self.rec.sim += 1;
            return Ok(());
        }
        let zoom = self.ctx.zoom_factor();
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w as f32 / zoom, h as f32 / zoom),
            )),
            time: Some(self.time),
            predicted_dt: 1.0 / fps as f32,
            focused: true,
            ..Default::default()
        };
        if let Some(v) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
            v.native_pixels_per_point = Some(1.0);
        }
        raw.events
            .push(egui::Event::ModifiersChanged(self.rec.modifiers));
        raw.events.push(egui::Event::PointerMoved(self.rec.pointer));
        raw.events.extend(events);
        let (cursor, pointer, pressed, now) = (
            self.rec.cursor,
            self.rec.pointer,
            !self.rec.held.is_empty(),
            self.time,
        );
        self.rec.ripples.retain(|(_, t)| now - t < 1.0);
        let ripples = self.rec.ripples.clone();
        let mut full = self.ctx.run_ui(raw, |ui| {
            app.ui(ui);
            if cursor {
                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Debug,
                    egui::Id::unique("kfx_record_pointer"),
                ));
                crate::record::paint_cursor(&painter, pointer, pressed, &ripples, now);
            }
        });
        {
            let mut r = self.gpu.egui_renderer.write();
            for (id, deltas) in &full.textures_delta.set {
                for delta in deltas {
                    r.update_texture(&self.device, &self.queue, *id, delta);
                }
            }
            for id in &full.textures_delta.free {
                r.free_texture(id);
            }
        }
        full.textures_delta.clear();
        self.check_forbidden(&full.shapes)?;
        if self.rec.writes_next() {
            let (rw, rh, px) = self.render_output(full, o.size)?;
            self.rec.write(rw, rh, px)?;
        } else {
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        }
        self.rec.sim += 1;
        Ok(())
    }

    /// `n` frames without new input.
    pub(super) fn steps(&mut self, app: &mut EditorApp, o: &Options, n: u32) -> Result<(), String> {
        for _ in 0..n {
            self.step(app, o, vec![])?;
        }
        Ok(())
    }

    /// Move the pointer to `to` (points) over `n` frames.
    pub(super) fn glide(
        &mut self,
        app: &mut EditorApp,
        o: &Options,
        to: egui::Pos2,
        n: u32,
        ease: &str,
    ) -> Result<(), String> {
        let from = self.rec.pointer;
        for i in 1..=n {
            self.rec.pointer = from + (to - from) * crate::record::ease(ease, i as f32 / n as f32);
            self.step(app, o, vec![])?;
        }
        Ok(())
    }

    pub(super) fn button_event(&mut self, b: egui::PointerButton, pressed: bool) -> egui::Event {
        if pressed {
            if !self.rec.held.contains(&b) {
                self.rec.held.push(b);
            }
            self.rec
                .ripples
                .push((self.rec.pointer, self.time + 1.0 / self.rec.fps as f64));
        } else {
            self.rec.held.retain(|x| *x != b);
        }
        egui::Event::PointerButton {
            pos: self.rec.pointer,
            button: b,
            pressed,
            modifiers: self.rec.modifiers,
        }
    }

    pub(super) fn capture_view(
        &mut self,
        app: &mut EditorApp,
        size: (u32, u32),
    ) -> Result<(u32, u32, Vec<u8>), String> {
        app.render_view(size);
        app.view_renderer()
            .read_back(&self.device, &self.queue)
            .ok_or_else(|| "read back failed".into())
    }

    pub(super) fn capture(
        &mut self,
        app: &mut EditorApp,
        o: &Options,
        ui: bool,
        path: &PathBuf,
    ) -> Result<(), String> {
        let ptr = self.pointer.or(o.pointer);
        let (w, h, px) = if ui {
            self.capture_ui(app, o.size, ptr)?
        } else {
            self.capture_view(app, o.size)?
        };
        write_png(path, w, h, px)?;
        eprintln!("wrote {}", path.display());
        Ok(())
    }
}
