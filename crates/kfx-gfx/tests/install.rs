//! Tests against a real install (`KFX_INSTALL`); skipped when absent.
use kfx_gfx::custom::{CustomSet, REQUIRED_SPRITE_ZIPS};
use kfx_gfx::keeper::{view_for_angle, KeeperSprites, ANIMATION_COUNT};
use kfx_gfx::sheet::{panel_sprite_index, SpriteSheet};
use kfx_gfx::textures::*;
use kfx_gfx::{Install, Palette};

fn install() -> Option<Install> {
    kfx_formats::testutil::install_dir().map(Install::new)
}

#[test]
fn palette_loads() {
    let Some(i) = install() else { return };
    let p = Palette::load(&i).unwrap();
    println!(
        "palette max raw channel {}, idx255 {:?}",
        p.max_raw(),
        p.rgba(255)
    );
    assert!(p.max_raw() <= 63);
}

#[test]
fn keeper_sprites_all_decode() {
    let Some(i) = install() else { return };
    let pal = Palette::load(&i).unwrap();
    let ks = KeeperSprites::load(&i).unwrap();
    let (mut ok, mut overflow, mut trailing, mut empty) = (0, 0usize, 0usize, 0);
    let mut rot = [0usize; 4];
    for n in 0..ks.len() {
        let f = ks
            .decode(n, &pal)
            .unwrap_or_else(|e| panic!("frame {n}: {e}"));
        let e = f.entry;
        assert_eq!(
            (f.image.w, f.image.h),
            (e.s_width as u32, e.s_height as u32)
        );
        if e.s_width == 0 || e.s_height == 0 {
            empty += 1;
        }
        overflow += f.stats.overflow_px;
        trailing += f.stats.trailing;
        rot[(e.rotable as usize).min(3)] += 1;
        ok += 1;
    }
    println!(
        "keeper frames decoded {ok}/{}; empty {empty}; overflow px {overflow}; trailing bytes \
        {trailing}; rotable 0/1/2/other entries {rot:?}",
        ks.len()
    );
    let mut bad = 0;
    let mut no_first = 0;
    for id in 0..ANIMATION_COUNT as u16 {
        match ks.animation(id) {
            None => no_first += 1,
            Some(a) => {
                let last = a.tab_index(a.frames as usize + 5, 4);
                if ks.entry(last).is_none() || a.frames == 0 || a.rotable == 1 {
                    bad += 1;
                }
            }
        }
    }
    println!(
        "animations: {ANIMATION_COUNT}, without tab entry {no_first}, odd (0 frames/rotable \
        1/out of range) {bad}"
    );
    assert_eq!(no_first, 0);
    // imp stand = 556 (creatrs/imp.cfg), dragon stand = 688
    for id in [556u16, 688] {
        let a = ks.animation(id).unwrap();
        println!(
            "anim {id}: first {} frames {} rotable {} dirs {}",
            a.first_frame, a.frames, a.rotable, a.directions
        );
    }
    assert_eq!(view_for_angle(0).quarter, 4);
}

#[test]
fn sheets_all_decode() {
    let Some(i) = install() else { return };
    let pal = Palette::load(&i).unwrap();
    for stem in ["gui2-64", "gui1-64", "pointer64"] {
        let s = SpriteSheet::load(&i, stem).unwrap();
        let (mut zero, mut overflow, mut dup) = (0, 0, 0);
        let mut offs: Vec<u32> = s.entries().iter().map(|e| e.offset).collect();
        offs.sort();
        dup += offs.windows(2).filter(|w| w[0] == w[1]).count();
        for n in 0..s.len() {
            let (ix, st) = s
                .decode_indexed(n)
                .unwrap_or_else(|e| panic!("{stem}[{n}]: {e}"));
            overflow += st.overflow_px;
            if ix.w == 0 || ix.h == 0 {
                zero += 1;
            }
            s.decode(n, &pal).unwrap();
        }
        println!(
            "{stem}: {} entries decoded, zero-size {zero}, overflow px {overflow}, duplicate \
            offsets {dup}",
            s.len()
        );
    }
    assert!(panel_sprite_index("GPS_portrt_qmark").is_some());
}

#[test]
fn all_texture_files_load() {
    let Some(i) = install() else { return };
    let pal = Palette::load(&i).unwrap();
    let anim = TextureAnim::load(&i).unwrap();
    let data = i.data_dir();
    let (mut n, mut odd) = (0, 0);
    for e in std::fs::read_dir(&data).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if !(name.starts_with("tmap") && name.ends_with(".dat")) {
            continue;
        }
        let len = kfx_gfx::install::load_file(&e.path()).unwrap().len();
        if len != FILE_BYTES {
            odd += 1;
            println!("odd size {name}: {len}");
        }
        n += 1;
    }
    println!("tmap files in data: {n}, size != 557056: {odd}");
    let mut sets = 0;
    for idx in 0..TEXTURE_VARIATIONS as u32 {
        if let Ok(ts) = TextureSet::load_std(&i, idx) {
            sets += 1;
            if !ts.has_b {
                println!("set {idx}: no tmapb");
            }
            assert_eq!(
                ts.atlas_rgba(Part::A, &pal).data.len(),
                ATLAS_W * ATLAS_H * 4
            );
        }
    }
    println!("texture sets loaded: {sets}");
    let ts = TextureSet::load_std(&i, 0).unwrap();
    // water (545) frames resolve into tmapa
    for f in 0..8 {
        let t = anim.resolve(545, f).unwrap();
        assert!(ts.tile_rgba(t, &pal).is_some());
    }
    println!("texture 545 frames: {:?}", anim.frames(545).unwrap());
    // tiles that are one flat index, and use of palette index 0
    let (mut flat0, mut with0) = (0, 0);
    for t in 0..TILES_PER_FILE as u16 {
        let d = ts
            .tile_indexed(TileRef {
                part: Part::A,
                tile: t,
            })
            .unwrap();
        if d.iter().all(|&v| v == 0) {
            flat0 += 1;
        } else if d.contains(&0) {
            with0 += 1;
        }
    }
    println!(
        "tmapa000: tiles entirely index 0: {flat0}, tiles containing index 0 among others: {with0}"
    );
}

#[test]
fn texture_search_order() {
    let mut s = TextureSearch::new("/d");
    s.level_dir = Some("/lv".into());
    s.campaign_config = Some("/cc".into());
    s.mods_after_base = vec![
        ModDirs {
            data: Some("/m1".into()),
            ..Default::default()
        },
        ModDirs {
            data: Some("/m2".into()),
            ..Default::default()
        },
    ];
    let c: Vec<String> = s
        .candidates('a', 3, Some(7))
        .iter()
        .map(|(d, n)| format!("{}/{n}", d.display()))
        .collect();
    assert_eq!(
        c,
        [
            "/lv/map00007.tmapa003.dat",
            "/cc/tmapa003.dat",
            "/m2/tmapa003.dat",
            "/m1/tmapa003.dat",
            "/d/tmapa003.dat"
        ]
    );
}

#[test]
fn fxdata_zips_parse() {
    let Some(i) = install() else { return };
    let fx = i.fxdata_dir();
    let mut total_anims = 0;
    for e in std::fs::read_dir(&fx).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "zip") {
            let mut s = CustomSet::new();
            let sum = s.load_zip(&p).unwrap();
            println!(
                "{}: {} animations, {} icons, {} slots, {} warnings",
                sum.file,
                sum.animations,
                sum.icons,
                s.slots_used(),
                sum.warnings.len()
            );
            for w in sum.warnings.iter().take(5) {
                println!("   warning: {w}");
            }
        }
    }
    let mut set = CustomSet::new();
    let sums = set.load_fxdata(&fx);
    assert_eq!(sums.len(), REQUIRED_SPRITE_ZIPS.len());
    for s in &sums {
        total_anims += s.animations;
    }
    println!(
        "combined fxdata: {total_anims} animations, {} icons, {} slots, {} warnings",
        set.icons().len(),
        set.slots_used(),
        set.warnings.len()
    );
    let a = set.animation("ORC_EAT").unwrap();
    assert_eq!(set.anim_id("ORC_EAT").unwrap().id, a.id);
    assert_eq!(set.anim_id("ORC_EAT:EAST").unwrap().direction, Some(2));
    assert!(set.frame(a.id).is_some());
    let ic = set.icons().first().map(|i| (i.name.clone(), i.id));
    println!(
        "first anim {} id {}, first icon {:?}",
        set.animations()[0].name,
        set.animations()[0].id,
        ic
    );
    if let Some((n, id)) = ic {
        assert!(set.icon_image(id).is_some());
        assert_eq!(set.icon_id(&n), Some(id));
    }
}
