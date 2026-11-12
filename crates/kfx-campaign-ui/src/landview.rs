//! Land view images: `<name>.raw` (8 bit indexed picture), `<name>.pal` (palette) and the window
//! frame `<name>.dat`, all in the campaign's `LAND_LOCATION` folder.
//!
//! Format (src/front_landview.c, `load_map_and_window`, 940-1013):
//! * `.raw`: `LANDVIEW_MAP_WIDTH x LANDVIEW_MAP_HEIGHT` (1280 x 960, src/config.h:39-40) bytes, one
//!   palette index per pixel, rows top to bottom, no header. The game accepts any length from 1024
//!   to 1228997 bytes (965-974) and copies it over the picture buffer, so a shorter file shows a
//!   truncated picture. Files may be RNC packed (`LbFileLengthRnc` / `LbFileLoadAt`).
//! * `.pal`: exactly `PALETTE_SIZE` = 768 bytes, 256 x (r, g, b), 6 bit VGA values (0..63;
//!   one shipped palette has a 64). The game loads it straight into `frontend_palette` (1005).
//! * `.dat` (window): `WINDOW_Y_SIZE` (720) little-endian `i32` line offsets followed by the RLE
//!   data of a `WINDOW_X_SIZE x WINDOW_Y_SIZE` (960 x 720) sprite (989-1003). This crate checks
//!   the size only and does not decode the sprite; the frame is shown in the canvas as a
//!   rectangle.
//!
//! Ensign positions (`ENSIGN_POS`, `ENSIGN_ZOOM`) are pixels of the 1280 x 960 picture; the
//! ensign sprite is drawn with its bottom centre on `ENSIGN_POS` (src/front_landview.c:583,
//! `is_over_ensign`:241-250). At the 640 x 480 base resolution `units_per_pixel_landview` is 16
//! (src/bflib_video.c:1189) so the screen shows a 640 x 480 piece of the picture
//! (`set_map_info_screen_shift`:625 centres it on the zoom point, then clamps it inside the
//! picture, `set_map_info_screen_shift_raw`:597).
use kfx_formats::testutil::resolve_ci;
use kfx_formats::{rnc, Error, Result};
use std::path::{Path, PathBuf};

/// `LANDVIEW_MAP_WIDTH` (src/config.h:39).
pub const MAP_W: usize = 1280;
/// `LANDVIEW_MAP_HEIGHT` (src/config.h:40).
pub const MAP_H: usize = 960;
/// `WINDOW_X_SIZE` (src/front_landview.c:73).
pub const WINDOW_W: usize = 960;
/// `WINDOW_Y_SIZE` (src/front_landview.c:74).
pub const WINDOW_H: usize = 720;
/// Size of the part of the picture the game shows at 640 x 480.
pub const VIEW_W: i32 = 640;
pub const VIEW_H: i32 = 480;
/// Largest `.raw` the game accepts (src/front_landview.c:970).
pub const RAW_MAX: usize = 1_228_997;
/// Smallest `.raw` the game accepts (src/front_landview.c:966).
pub const RAW_MIN: usize = 1024;
/// `PALETTE_SIZE`.
pub const PAL_SIZE: usize = 768;

/// A decoded land view picture.
#[derive(Clone, Debug)]
pub struct LandImage {
    pub width: usize,
    pub height: usize,
    /// `width * height * 4` bytes, RGBA.
    pub rgba: Vec<u8>,
    /// Length of the unpacked `.raw` file.
    pub raw_len: usize,
}

/// Find `<name><ext>` (ext with the dot) in `dir`, ignoring case.
pub fn find_file(dir: &Path, name: &str, ext: &str) -> Option<PathBuf> {
    resolve_ci(dir, &format!("{name}{ext}")).filter(|p| p.is_file())
}

/// Decode 6 bit VGA palette bytes to RGB (`v << 2 | v >> 4`, values above 63 are clamped).
pub fn palette_rgb(pal: &[u8]) -> Vec<[u8; 3]> {
    (0..256)
        .map(|i| {
            let c = |k: usize| {
                let v = pal.get(i * 3 + k).copied().unwrap_or(0).min(63);
                (v << 2) | (v >> 4)
            };
            [c(0), c(1), c(2)]
        })
        .collect()
}

/// Load and decode `<name>.raw` + `<name>.pal` from `dir`.
pub fn load(dir: &Path, name: &str) -> Result<LandImage> {
    let raw_path = find_file(dir, name, ".raw").ok_or_else(|| {
        Error::format(
            "landview",
            format!("{name}.raw not found in {}", dir.display()),
        )
    })?;
    let pal_path = find_file(dir, name, ".pal").ok_or_else(|| {
        Error::format(
            "landview",
            format!("{name}.pal not found in {}", dir.display()),
        )
    })?;
    let raw = rnc::load_file(&raw_path)?;
    let pal = rnc::load_file(&pal_path)?;
    Ok(decode(&raw, &pal))
}

/// Decode raw + palette bytes. Missing pixels (a short file) are black.
pub fn decode(raw: &[u8], pal: &[u8]) -> LandImage {
    let rgb = palette_rgb(pal);
    let mut rgba = vec![0u8; MAP_W * MAP_H * 4];
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let idx = raw.get(i).copied();
        match idx {
            Some(v) => {
                let c = rgb[v as usize];
                px.copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
            None => px.copy_from_slice(&[0, 0, 0, 255]),
        }
    }
    LandImage {
        width: MAP_W,
        height: MAP_H,
        rgba,
        raw_len: raw.len(),
    }
}

/// The part of the picture the game shows when the camera is centred on `zoom` (clamped to the
/// picture): `(x, y, w, h)` in picture pixels.
pub fn view_rect(zoom: (i32, i32)) -> (i32, i32, i32, i32) {
    let x = (zoom.0 - VIEW_W / 2).clamp(0, MAP_W as i32 - VIEW_W);
    let y = (zoom.1 - VIEW_H / 2).clamp(0, MAP_H as i32 - VIEW_H);
    (x, y, VIEW_W, VIEW_H)
}

/// Problems with the three files of one land view, as `(message, rule)`.
pub fn check_files(dir: &Path, view: &str, window: &str) -> Vec<(String, &'static str)> {
    let mut out = vec![];
    match find_file(dir, view, ".raw") {
        None => out.push((
            format!(
                "land view picture {view}.raw is missing in {}",
                dir.display()
            ),
            "front_landview.c:965-970",
        )),
        Some(p) => match rnc::load_file(&p) {
            Err(e) => out.push((
                format!("{view}.raw cannot be read: {e}"),
                "front_landview.c:979",
            )),
            Ok(b) if b.len() < RAW_MIN => out.push((
                format!(
                    "{view}.raw is {} bytes, the game needs at least {RAW_MIN}",
                    b.len()
                ),
                "front_landview.c:966-970",
            )),
            Ok(b) if b.len() > RAW_MAX => out.push((
                format!(
                    "{view}.raw is {} bytes, the game accepts at most {RAW_MAX}",
                    b.len()
                ),
                "front_landview.c:971-975",
            )),
            Ok(b) if b.len() != MAP_W * MAP_H => out.push((
                format!(
                    "{view}.raw is {} bytes, a full picture is {} (1280x960)",
                    b.len(),
                    MAP_W * MAP_H
                ),
                "config.h:39-40",
            )),
            Ok(_) => {}
        },
    }
    match find_file(dir, view, ".pal") {
        None => out.push((
            format!("palette {view}.pal is missing in {}", dir.display()),
            "front_landview.c:1005-1010",
        )),
        Some(p) => match rnc::load_file(&p) {
            Ok(b) if b.len() == PAL_SIZE => {}
            Ok(b) => out.push((
                format!(
                    "{view}.pal is {} bytes, it must be exactly {PAL_SIZE}",
                    b.len()
                ),
                "front_landview.c:1005-1010",
            )),
            Err(e) => out.push((
                format!("{view}.pal cannot be read: {e}"),
                "front_landview.c:1005",
            )),
        },
    }
    match find_file(dir, window, ".dat") {
        None => out.push((
            format!("window frame {window}.dat is missing in {}", dir.display()),
            "front_landview.c:987-993",
        )),
        Some(p) => match std::fs::metadata(&p) {
            Ok(m) if (m.len() as usize) < WINDOW_H * 4 && !rnc_packed(&p) => out.push((
                format!(
                    "{window}.dat is {} bytes, the line table alone needs {}",
                    m.len(),
                    WINDOW_H * 4
                ),
                "front_landview.c:989-993",
            )),
            Ok(_) => {}
            Err(e) => out.push((
                format!("{window}.dat cannot be read: {e}"),
                "front_landview.c:988",
            )),
        },
    }
    out
}

fn rnc_packed(p: &Path) -> bool {
    std::fs::read(p).map(|b| rnc::is_rnc(&b)).unwrap_or(false)
}

/// The numbered land views present in `dir`: names `<stem>NN` that have a `.raw` and a `.pal`
/// (for example `rgmap00` ... `rgmap21`), sorted.
pub fn available_views(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    let lower = n.to_ascii_lowercase();
                    let stem = lower.strip_suffix(".raw")?;
                    find_file(dir, stem, ".pal").map(|_| stem.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The window frames present in `dir`: `viframeNN` style `.dat` names, sorted.
pub fn available_windows(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let lower = e.file_name().to_string_lossy().to_ascii_lowercase();
                    let stem = lower.strip_suffix(".dat")?;
                    stem.starts_with("viframe").then(|| stem.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}
