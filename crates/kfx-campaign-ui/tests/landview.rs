use kfx_campaign_ui::landview::*;
use kfx_formats::testutil::install_dir;

#[test]
fn palette_scaling() {
    let mut pal = vec![0u8; 768];
    pal[3..6].copy_from_slice(&[63, 32, 0]);
    pal[6..9].copy_from_slice(&[64, 1, 2]);
    let rgb = palette_rgb(&pal);
    assert_eq!(rgb[1], [255, 130, 0]);
    assert_eq!(rgb[2], [255, 4, 8], "an out of range 64 is clamped to 63");
    let img = decode(&[1, 2, 0], &pal);
    assert_eq!(
        (img.width, img.height, img.rgba.len(), img.raw_len),
        (1280, 960, 1280 * 960 * 4, 3)
    );
    assert_eq!(&img.rgba[0..4], &[255, 130, 0, 255]);
    assert_eq!(
        &img.rgba[12..16],
        &[0, 0, 0, 255],
        "pixels past a short file are black"
    );
}

#[test]
fn view_rect_is_clamped_like_the_game() {
    assert_eq!(view_rect((640, 480)), (320, 240, 640, 480));
    assert_eq!(view_rect((10, 10)), (0, 0, 640, 480));
    assert_eq!(view_rect((1270, 950)), (640, 480, 640, 480));
}

#[test]
fn every_shipped_land_view_decodes() {
    let Some(inst) = install_dir() else { return };
    let mut n = 0;
    for e in std::fs::read_dir(inst.join("campgns")).unwrap().flatten() {
        let p = e.path();
        if !p.is_dir() || !p.file_name().unwrap().to_string_lossy().ends_with("_lnd") {
            continue;
        }
        for v in available_views(&p) {
            let img = load(&p, &v).unwrap_or_else(|e| panic!("{}/{v}: {e}", p.display()));
            assert_eq!((img.width, img.height), (1280, 960));
            assert!(
                img.rgba
                    .chunks(4)
                    .any(|px| px[0] > 40 || px[1] > 40 || px[2] > 40),
                "{v} is not all black"
            );
            n += 1;
            assert!(
                check_files(&p, &v, &v.replace("rgmap", "viframe"))
                    .iter()
                    .all(|(m, _)| !m.contains(".raw") && !m.contains(".pal")),
                "{v}"
            );
        }
    }
    assert!(n >= 20, "decoded {n} pictures");
    println!("decoded {n} land view pictures");
}
