//! Decodes a sample of the game's art and writes it as PNGs, to check the decoders by eye: the
//! first texture atlas (`tmapa000.png`), the first 200 GUI icons of `gui2-64`, the standing
//! animation of a few creatures (one row per stored view) and the first animation of
//! `creatures.zip` when the install has one. Prints each file written with its size.
//!
//! Usage: `cargo run -p kfx-gfx --example dump -- [<install dir> [<output dir>]]`. The install
//! defaults to `KFX_INSTALL`, the output folder to `target/tmp/kfx-gfx/dump`
//! (`kfx_formats::testutil::scratch_dir`).
use kfx_gfx::keeper::KeeperSprites;
use kfx_gfx::sheet::SpriteSheet;
use kfx_gfx::textures::{Part, TextureSet};
use kfx_gfx::{CustomSet, Install, Palette, Rgba};
use std::path::{Path, PathBuf};

/// Grid of images on a checkerboard background, `cols` per row, cells sized to the largest image.
fn contact_sheet(imgs: &[Rgba], cols: usize, pad: u32) -> Rgba {
    let cw = imgs.iter().map(|i| i.w).max().unwrap_or(1).max(1) + pad;
    let ch = imgs.iter().map(|i| i.h).max().unwrap_or(1).max(1) + pad;
    let rows = imgs.len().div_ceil(cols).max(1);
    let mut out = Rgba::new(cw * cols as u32, ch * rows as u32);
    for y in 0..out.h {
        for x in 0..out.w {
            let c = if ((x / 8) + (y / 8)) % 2 == 0 { 60 } else { 90 };
            out.put(x, y, [c, c, c, 255]);
        }
    }
    for (n, im) in imgs.iter().enumerate() {
        out.blit(
            im,
            ((n % cols) as u32 * cw + pad / 2) as i32,
            ((n / cols) as u32 * ch + pad / 2) as i32,
        );
    }
    out
}

fn save(dir: &Path, name: &str, img: &Rgba, report: &mut Vec<String>) {
    let p = dir.join(name);
    std::fs::write(&p, img.to_png().expect("png encode")).expect("write png");
    report.push(format!("{} {}x{}", p.display(), img.w, img.h));
}

/// `Stand = N` of the `[sprites]` section of a creature cfg.
fn stand_anim(install: &Install, cfg: &str) -> Option<u16> {
    let text = String::from_utf8_lossy(&install.load(&format!("creatrs/{cfg}")).ok()?).into_owned();
    let mut in_sprites = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_sprites = l.eq_ignore_ascii_case("[sprites]");
        } else if in_sprites {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case("Stand") {
                    return v.split(';').next()?.trim().parse().ok();
                }
            }
        }
    }
    None
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(install_dir) = args
        .next()
        .map(PathBuf::from)
        .or_else(kfx_formats::testutil::install_dir)
    else {
        eprintln!("usage: dump [<install dir> [<output dir>]] (or set KFX_INSTALL)");
        std::process::exit(2);
    };
    let install = Install::new(install_dir);
    let out: PathBuf = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| kfx_formats::testutil::scratch_dir("kfx-gfx/dump"));
    std::fs::create_dir_all(&out).expect("create output dir");
    let mut report = Vec::new();
    let pal = Palette::load(&install).expect("palette");

    let ts = TextureSet::load_std(&install, 0).expect("tmap 0");
    save(
        &out,
        "tmapa000.png",
        &ts.atlas_rgba(Part::A, &pal),
        &mut report,
    );

    let gui = SpriteSheet::load(&install, "gui2-64").expect("gui2-64");
    let icons: Vec<Rgba> = (0..gui.len().min(200))
        .map(|i| gui.decode(i, &pal).expect("icon"))
        .collect();
    save(
        &out,
        "gui2-64_first200.png",
        &contact_sheet(&icons, 20, 4),
        &mut report,
    );

    let ks = KeeperSprites::load(&install).expect("keeper sprites");
    for cfg in ["imp", "dragon", "knight", "bile_demon"] {
        let Some(id) = stand_anim(&install, &format!("{cfg}.cfg")) else {
            eprintln!("no Stand animation for {cfg}");
            continue;
        };
        let Some(a) = ks.animation(id) else { continue };
        // one row per stored view, `frames` columns
        let mut imgs = Vec::new();
        for q in 0..a.directions {
            for f in 0..a.frames as usize {
                imgs.push(ks.decode(a.tab_index(f, q), &pal).expect("frame").image);
            }
        }
        let cols = a.frames.max(1) as usize;
        save(
            &out,
            &format!("creature_{cfg}_anim{id}.png"),
            &contact_sheet(&imgs, cols, 4),
            &mut report,
        );
    }

    let mut set = CustomSet::new();
    let fx = install.fxdata_dir();
    if let Some(p) = kfx_gfx::install::find_in_dir(&fx, "creatures.zip") {
        set.load_zip(&p).expect("creatures.zip");
        if let Some(a) = set.animations().first() {
            let mut imgs = Vec::new();
            for d in &a.td {
                for &id in d {
                    imgs.push(set.frame(id).expect("slot").image.clone());
                }
            }
            let cols = a.frames().max(1);
            save(
                &out,
                &format!("custom_{}_td.png", a.name),
                &contact_sheet(&imgs, cols, 4),
                &mut report,
            );
        }
    }
    for l in report {
        println!("{l}");
    }
}
