//! Shared test helpers: game data, shipped maps, PNG output under the test scratch folder
//! (`kfx_formats::testutil::scratch_dir`, `target/tmp/kfx-analysis` by default).
// Shared by several test binaries; each uses a subset.
#![allow(dead_code)]
use kfx_analysis::layer::RgbaImage;
use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::{rnc, testutil};
use kfx_map::gamedata::{pack_folders, ConfigLayers, GameData};
use kfx_map::mapdoc::MapDoc;
use std::path::{Path, PathBuf};

/// Base game data (fxdata only), `None` without the test install.
pub fn base_gd() -> Option<GameData> {
    let root = testutil::install_dir()?;
    GameData::load(&ConfigLayers::for_level(&root, None, None, None, None)).ok()
}

/// Where test images go.
pub fn out_dir() -> PathBuf {
    testutil::scratch_dir("kfx-analysis")
}

pub fn write_png(path: &Path, img: &RgbaImage) {
    let f = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), img.w, img.h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    let mut w = e.write_header().unwrap();
    w.write_image_data(&img.pixels).unwrap();
}

/// A shipped map: (pack folder, level, game data of the level, document).
pub struct Shipped {
    pub folder: PathBuf,
    pub level: u32,
    pub gd: GameData,
    pub doc: MapDoc,
}

/// Every shipped map (each pack folder / level once), calling `f` for each.
pub fn for_each_shipped(mut f: impl FnMut(Shipped)) -> usize {
    let Some(root) = testutil::install_dir() else {
        return 0;
    };
    let mut seen = std::collections::HashSet::new();
    let mut n = 0;
    for (levels, cfgs, crtr) in pack_folders(&root) {
        let mut nums: Vec<u32> = std::fs::read_dir(&levels)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                n.strip_prefix("map")?.strip_suffix(".slb")?.parse().ok()
            })
            .collect();
        nums.sort();
        nums.dedup();
        for lv in nums {
            if !seen.insert((levels.clone(), lv)) {
                continue;
            }
            let Ok(gd) = GameData::load(&ConfigLayers::for_level(
                &root,
                cfgs.as_deref(),
                Some(&levels),
                crtr.as_deref(),
                Some(lv),
            )) else {
                continue;
            };
            let Some(slb) =
                level_bin::find_file(&levels, lv, "slb").and_then(|p| rnc::load_file(&p).ok())
            else {
                continue;
            };
            let Some(dims) = MapDims::from_slb_len(slb.len()) else {
                continue;
            };
            let Ok(doc) = MapDoc::load(&levels, lv, dims, &gd) else {
                continue;
            };
            n += 1;
            f(Shipped {
                folder: levels.clone(),
                level: lv,
                gd,
                doc,
            });
        }
    }
    n
}
