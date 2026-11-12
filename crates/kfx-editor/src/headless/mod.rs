//! The screenshot mode (`--screenshot`): render the UI (or only the 3D view) offscreen and write
//! a PNG, no window; optionally after a script of editor actions (`--actions`) and as a recording
//! of frames (`--record`).
use crate::app::{CameraSpec, EditorApp, Gpu};
use crate::render::{read_texture, COLOR_FORMAT};
use crate::settings::Settings;
use eframe::egui;
use egui_wgpu::wgpu;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

mod actions;
mod audiolive;
mod config;
mod events;
mod harness;
mod options;
mod recording;
mod scripts;
mod things;
mod views_tools;

use actions::*;
use config::*;
use harness::*;
use recording::*;
use scripts::*;
use things::*;
use views_tools::*;

pub use harness::create_device;
pub use options::Options;

/// The editor without a window, with a map open (CLI `export-image`).
pub fn export(
    map: &std::path::Path,
    out: &std::path::Path,
    iso: bool,
    scale: u32,
    things: bool,
) -> Result<(u32, u32, usize), String> {
    let (_adapter, device, queue) = create_device()?;
    let renderer = egui_wgpu::Renderer::new(
        &device,
        COLOR_FORMAT,
        egui_wgpu::RendererOptions {
            dithering: false,
            ..Default::default()
        },
    );
    let gpu = Gpu {
        device,
        queue,
        egui_renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
    };
    let ctx = egui::Context::default();
    let mut settings = Settings::load();
    settings.install = crate::settings::pick_install(&settings, Some(map));
    settings.layout = crate::settings::Layout::default();
    let mut app = EditorApp::new(&ctx, gpu, settings, false, None);
    if app.install().is_none() {
        return Err(
            "no KeeperFX install: set KFX_INSTALL, or keep the map inside an install".into(),
        );
    }
    app.fixed_frame = Some(0);
    app.open_map(map)?;
    app.export_image(out, iso, scale, things, false, false)
}

pub fn run(o: &Options) -> Result<(), String> {
    let (adapter, device, queue) = create_device()?;
    let info = adapter.get_info();
    eprintln!("adapter: {}", egui_wgpu::adapter_info_summary(&info));
    let renderer = egui_wgpu::Renderer::new(
        &device,
        COLOR_FORMAT,
        egui_wgpu::RendererOptions {
            dithering: false,
            ..Default::default()
        },
    );
    let gpu = Gpu {
        device: device.clone(),
        queue: queue.clone(),
        egui_renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
    };
    let ctx = egui::Context::default();
    let mut settings = Settings::load();
    // `--map -`: start with no map open (the editor as it starts without one)
    let no_map = o.map.as_os_str() == "-";
    settings.install =
        crate::settings::pick_install(&settings, (!no_map).then_some(o.map.as_path()));
    // screenshots use the default view options, not the user's saved ones
    settings.layout = crate::settings::Layout::default();
    let mut app = EditorApp::new(&ctx, gpu.clone(), settings, false, None);
    if app.install().is_none() {
        return Err(
            "no KeeperFX install: set KFX_INSTALL, or put the map inside an install".into(),
        );
    }
    app.fixed_frame = Some(o.frame % 8);
    app.show_browser(o.show_browser, &o.browser_filter);
    if o.rooms_palette {
        app.show_rooms_palette();
    }
    app.set_perspective(o.perspective);
    if let Some(c) = o.camera {
        app.set_camera_spec(c);
    }
    let t = Instant::now();
    if !no_map {
        app.open_map(&o.map)?;
    }
    eprintln!("open: {:.0} ms total", t.elapsed().as_secs_f32() * 1000.0);
    let p = app.perf;
    eprintln!(
        "atlas {:.1} ms, mesh build {:.1} ms ({} chunks), upload {:.1} ms, {} triangles",
        p.atlas_ms,
        p.mesh_ms,
        p.chunks,
        p.upload_ms,
        app.view_renderer().triangles
    );
    if let Some(p) = &o.slab_image {
        slab_image(&app, p)?;
    }
    eprintln!(
        "sprites: {:.0} ms to load ({} things)",
        app.perf.sprites_ms,
        app.doc.as_ref().map_or(0, |d| d.doc.things.items.len())
    );
    let mut rec =
        crate::record::Recorder::new(o.record.as_deref().map(crate::record::Sink::parse), o.fps);
    rec.forbid = o.forbid.clone();
    rec.captions_file = o.record_captions.clone();
    let mut hl = Headless {
        device: device.clone(),
        queue,
        gpu,
        ctx,
        time: 0.0,
        pointer: None,
        rec,
    };
    if let Some(f) = &o.actions {
        run_actions(&mut hl, &mut app, o, f)?;
    }
    if o.record.is_some() {
        return hl.rec.finish();
    }

    let (w, h) = o.size;
    if o.viewport_only {
        if let Some((x, y)) = o.pointer {
            app.hover_pixel(glam::Vec2::new(x, y), (w, h));
        }
        app.render_view((w, h));
        bench(&mut app, &device, (w, h), o.bench);
        if let Some(r) = app.readout() {
            eprintln!("hover: {}", r.replace('\n', " | "));
        }
        return hl.capture(&mut app, o, false, &o.out);
    }
    let (rw, rh, px) = hl.capture_ui(&mut app, (w, h), o.pointer)?;
    let vs = app_view_size(&app);
    bench(&mut app, &device, vs, o.bench);
    if let Some(r) = app.readout() {
        eprintln!("hover: {}", r.replace('\n', " | "));
    }
    write_png(&o.out, rw, rh, px)
}

fn app_view_size(app: &EditorApp) -> (u32, u32) {
    app.view_renderer()
        .output()
        .map(|(t, _)| (t.width(), t.height()))
        .unwrap_or((800, 600))
}

/// Time `n` viewport frames (CPU encode + GPU completion).
fn bench(app: &mut EditorApp, device: &wgpu::Device, size: (u32, u32), n: u32) {
    if n == 0 {
        return;
    }
    let (cam, frame) = (app.camera, app.fixed_frame);
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let t = Instant::now();
    for i in 0..n {
        app.fixed_frame = Some(i % 8);
        app.camera.yaw += 0.01;
        app.render_view(size);
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }
    let ms = t.elapsed().as_secs_f32() * 1000.0 / n as f32;
    app.camera = cam;
    app.fixed_frame = frame;
    app.render_view(size);
    eprintln!(
        "bench: {n} frames at {}x{}: {ms:.2} ms/frame (render + wait for GPU); {} \
        billboards, last thing build {:.2} ms",
        size.0, size.1, app.perf.billboards, app.perf.things_ms
    );
}
