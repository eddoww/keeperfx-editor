//! Robustness: every map of the install opens, validates and renders a frame without a panic;
//! truncated, corrupted and missing map files give an error or a warning, never a panic.
//!
//! The full sweep (every map of every campaign / pack) runs with `KFX_SWEEP=1`; without it every
//! seventh map is opened.
use crate::app::{EditorApp, Gpu};
use crate::render::COLOR_FORMAT;
use crate::settings::Settings;
use eframe::egui;
use kfx_formats::testutil;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn app(install: &Path) -> Option<EditorApp> {
    let (_a, device, queue) = crate::headless::create_device().ok()?;
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
    let settings = Settings {
        install: Some(install.to_path_buf()),
        ..Default::default()
    };
    let mut a = EditorApp::new(&ctx, gpu, settings, false, None);
    a.fixed_frame = Some(0);
    a.install()?;
    Some(a)
}

/// Open, validate and render one frame; `Err` for a clean error, panics are caught by the caller.
fn open_render(a: &mut EditorApp, slb: &Path) -> Result<Vec<String>, String> {
    a.open_map(slb)?;
    a.revalidate();
    a.render_view((320, 200));
    Ok(a.doc
        .as_ref()
        .map(|d| d.doc.warnings.clone())
        .unwrap_or_default())
}

#[test]
fn sweep_install_maps() {
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let Some(mut a) = app(&install) else { return };
    let full = std::env::var("KFX_SWEEP").ok().as_deref() == Some("1");
    let packs = crate::browser::scan(&install);
    let mut maps: Vec<PathBuf> = packs
        .iter()
        .flat_map(|p| p.levels.iter().filter_map(|l| l.slb.clone()))
        .collect();
    maps.sort();
    maps.dedup();
    let total = maps.len();
    if !full {
        maps = maps.into_iter().step_by(7).collect();
    }
    let t0 = std::time::Instant::now();
    let (mut ok, mut errors, mut panics, mut warned) = (0, vec![], vec![], 0);
    for m in &maps {
        match catch_unwind(AssertUnwindSafe(|| open_render(&mut a, m))) {
            Ok(Ok(w)) => {
                ok += 1;
                if !w.is_empty() {
                    warned += 1;
                }
            }
            Ok(Err(e)) => errors.push(format!("{}: {e}", m.display())),
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                panics.push(format!("{}: PANIC {msg}", m.display()));
                // a panic may leave the app half-updated: start over
                if let Some(n) = app(&install) {
                    a = n;
                }
            }
        }
    }
    println!(
        "sweep: {} of {total} maps ({} packs) in {:.1} s: {ok} opened + validated + rendered \
        ({warned} with load warnings), {} errors, {} panics",
        maps.len(),
        packs.len(),
        t0.elapsed().as_secs_f32(),
        errors.len(),
        panics.len()
    );
    for e in errors.iter().chain(&panics) {
        println!("  {e}");
    }
    assert!(panics.is_empty(), "{} maps panicked", panics.len());
}

/// Truncations at several lengths, a corrupted copy and a missing copy of every file type of a
/// shipped map: a clear error or a warning, never a panic.
#[test]
fn corrupt_map_files() {
    let Some(install) = testutil::install_dir() else {
        return;
    };
    let Some(mut a) = app(&install) else { return };
    let src = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    let dir =
        kfx_formats::testutil::scratch_dir(&format!("kfx-editor/corrupt-{}", std::process::id()));
    let reset = |dir: &Path| {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        for f in crate::document::level_files(&src, 1) {
            std::fs::copy(
                &f,
                dir.join(
                    f.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_ascii_lowercase(),
                ),
            )
            .unwrap();
        }
    };
    reset(&dir);
    let files: Vec<PathBuf> = crate::document::level_files(&dir, 1);
    let slb = dir.join("map00001.slb");
    let (mut cases, mut errs, mut opened, mut panics) = (0, 0, 0, vec![]);
    let mut run = |a: &mut EditorApp, what: String| {
        cases += 1;
        match catch_unwind(AssertUnwindSafe(|| open_render(a, &slb))) {
            Ok(Ok(_)) => opened += 1,
            Ok(Err(_)) => errs += 1,
            Err(_) => {
                panics.push(what);
                if let Some(n) = app(&install) {
                    *a = n;
                }
            }
        }
    };
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let len = std::fs::metadata(f).map(|m| m.len() as usize).unwrap_or(0);
        let mut cuts: Vec<usize> = vec![0, 1, 2, 7, 16, len / 3, len / 2, len.saturating_sub(1)];
        cuts.sort();
        cuts.dedup();
        for cut in cuts.into_iter().filter(|&c| c < len) {
            reset(&dir);
            let b = std::fs::read(f).unwrap();
            std::fs::write(f, &b[..cut]).unwrap();
            run(&mut a, format!("{name} truncated to {cut}"));
        }
        // corrupted: every 7th byte flipped
        reset(&dir);
        let mut b = std::fs::read(f).unwrap();
        for i in (0..b.len()).step_by(7) {
            b[i] ^= 0xA5;
        }
        std::fs::write(f, &b).unwrap();
        run(&mut a, format!("{name} corrupted"));
        // missing
        reset(&dir);
        std::fs::remove_file(f).unwrap();
        run(&mut a, format!("{name} missing"));
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!(
        "corrupt files: {cases} cases over {} files: {opened} opened (with warnings where \
        data was missing), {errs} clean errors, {} panics",
        files.len(),
        panics.len()
    );
    for p in &panics {
        println!("  PANIC {p}");
    }
    assert!(
        panics.is_empty(),
        "{} cases panicked: {panics:?}",
        panics.len()
    );
}
