//! Generator tests: constraint success over many seeds, determinism, a contact sheet of
//! generated maps under `target/tmp/kfx-analysis/` (skipped without the test install).
mod common;

use kfx_analysis::analysis::Analysis;
use kfx_analysis::generate::{generate, GenParams, Style, SymmetryChoice, ALL_STYLES};
use kfx_analysis::layer::{render_overview, RgbaImage};
use kfx_map::transform::SymmetryMode;
use kfx_map::validate::{validate, Severity};
use std::time::Instant;

/// Parameters of run `i` of the sweep: styles, sizes and keeper counts in rotation.
fn sweep_params(i: u64) -> GenParams {
    let style = ALL_STYLES[(i % 5) as usize];
    let players = [2u8, 4, 1, 3, 2, 4, 6, 5, 7, 2][(i / 5 % 10) as usize];
    let (w, h) = match players {
        1 | 2 => [(64u16, 64u16), (85, 85), (96, 72)][(i % 3) as usize],
        3 | 4 => [(85, 85), (110, 110), (120, 96)][(i % 3) as usize],
        _ => [(130, 130), (150, 150), (170, 170)][(i % 3) as usize],
    };
    GenParams {
        seed: 1000 + i,
        width: w,
        height: h,
        style,
        players,
        symmetry: SymmetryChoice::Auto,
        richness: (20 + (i * 37) % 80) as u8,
        water: ((i * 13) % 60) as u8,
        lava: ((i * 29) % 60) as u8,
        heroes: i % 3 != 2,
        gems: (i % 4) as u8,
        neutral_rooms: (i % 3) as u8 + 1,
        neutral_creatures: (i % 5) as u8 + 2,
        special_boxes: (i % 3) as u8,
        decorate: i % 2 == 0,
        folder: common::out_dir().join("gen"),
        level: 2,
        ..Default::default()
    }
}

#[test]
fn many_seeds_meet_the_constraints() {
    let Some(gd) = common::base_gd() else { return };
    let n: u64 = std::env::var("KFX_GEN_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let mut fails = vec![];
    let mut times = vec![];
    let mut attempts = 0u64;
    let mut by_style: std::collections::BTreeMap<String, (u32, u32)> = Default::default();
    for i in 0..n {
        let p = sweep_params(i);
        let t0 = Instant::now();
        let r = generate(&gd, &p);
        let ms = t0.elapsed().as_millis() as u64;
        let e = by_style.entry(format!("{:?}", p.style)).or_default();
        e.0 += 1;
        match r {
            Ok(g) => {
                attempts += g.report.attempts as u64;
                times.push((ms, i, p.width, p.height, p.players));
                let errors: Vec<_> = validate(&g.doc, &gd)
                    .into_iter()
                    .filter(|x| x.severity == Severity::Error)
                    .collect();
                assert!(errors.is_empty(), "seed {i}: validator errors {errors:?}");
                assert!(
                    g.report.checks.iter().all(|c| c.ok),
                    "seed {i}: {:?}",
                    g.report.checks
                );
                if !g.report.warnings.is_empty() {
                    println!(
                        "run {i} {:?} {}p {}x{}: warnings {:?}",
                        p.style, p.players, p.width, p.height, g.report.warnings
                    );
                }
            }
            Err(f) => {
                e.1 += 1;
                fails.push(format!(
                    "run {i} {:?} {}p {}x{} {:?}: {f}",
                    p.style, p.players, p.width, p.height, p.symmetry
                ));
            }
        }
    }
    times.sort();
    let total: u64 = times.iter().map(|t| t.0).sum();
    println!(
        "{} of {n} generated ({} failed, {:.1}%); attempts per success {:.2}; time per map mean \
            {:.0} ms, median {} ms, max {} ms ({:?})",
        times.len(),
        fails.len(),
        fails.len() as f64 * 100.0 / n as f64,
        attempts as f64 / times.len().max(1) as f64,
        total as f64 / times.len().max(1) as f64,
        times.get(times.len() / 2).map(|t| t.0).unwrap_or(0),
        times.last().map(|t| t.0).unwrap_or(0),
        times.last()
    );
    for (s, (runs, f)) in &by_style {
        println!("  {s}: {runs} runs, {f} failed");
    }
    for f in &fails {
        println!("FAIL {f}");
    }
    assert!(fails.is_empty(), "{} failures", fails.len());
}

#[test]
fn same_seed_same_map() {
    let Some(gd) = common::base_gd() else { return };
    for i in [3u64, 17, 41] {
        let p = sweep_params(i);
        let a = generate(&gd, &p).expect("generates");
        let b = generate(&gd, &p).expect("generates");
        assert_eq!(a.doc.kinds, b.doc.kinds);
        assert_eq!(a.doc.owners, b.doc.owners);
        assert_eq!(a.doc.columns, b.doc.columns);
        assert_eq!(a.doc.things.items, b.doc.things.items);
        assert_eq!(a.plan, b.plan);
        // a fingerprint to compare across machines
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for (&k, &o) in a.doc.kinds.iter().zip(&a.doc.owners) {
            h = (h ^ (k as u64 * 31 + o as u64)).wrapping_mul(0x100_0000_01b3);
        }
        println!(
            "run {i}: fingerprint {h:016x}, {} things",
            a.doc.things.items.len()
        );
        let other = generate(
            &gd,
            &GenParams {
                seed: p.seed + 1,
                ..p.clone()
            },
        )
        .expect("generates");
        assert_ne!(
            a.doc.kinds, other.doc.kinds,
            "another seed gives another map"
        );
    }
}

#[test]
fn symmetric_modes_are_exact() {
    let Some(gd) = common::base_gd() else { return };
    for (mode, players, w, h) in [
        (SymmetryMode::NorthSouth, 2u8, 80u16, 70u16),
        (SymmetryMode::EastWest, 2, 81, 64),
        (SymmetryMode::Rot2, 2, 85, 85),
        (SymmetryMode::FourWay, 4, 100, 90),
        (SymmetryMode::Rot4, 4, 101, 101),
    ] {
        for style in ALL_STYLES {
            let p = GenParams {
                seed: 77,
                width: w,
                height: h,
                players,
                style,
                symmetry: SymmetryChoice::Mode(mode),
                ..Default::default()
            };
            let g = match generate(&gd, &p) {
                Ok(g) => g,
                Err(e) => panic!("{mode:?} {style:?}: {e}"),
            };
            let a = Analysis::of_doc(&g.doc, &gd);
            let c = a.symmetry.iter().find(|c| c.mode == mode).unwrap();
            assert!(
                c.terrain_exact(),
                "{mode:?} {style:?}: {} slabs break it, e.g. {:?}",
                c.slab_breaks.len(),
                &c.slab_breaks[..c.slab_breaks.len().min(6)]
            );
            println!(
                "{mode:?} {style:?}: exact terrain, {} thing breaks, imbalance {:.3}",
                c.thing_breaks.len(),
                a.fairness.as_ref().map(|f| f.score).unwrap_or(0.0)
            );
        }
    }
    let bad = generate(
        &gd,
        &GenParams {
            players: 3,
            symmetry: SymmetryChoice::Mode(SymmetryMode::Rot2),
            ..Default::default()
        },
    );
    let e = bad.expect_err("3 keepers cannot be half-turn symmetric");
    assert!(e.reason.contains("multiple of 2"), "{e}");
}

/// Twelve generated maps side by side.
#[test]
fn contact_sheet() {
    let Some(gd) = common::base_gd() else { return };
    let mut tiles: Vec<RgbaImage> = vec![];
    let specs: [(Style, u8, u16, SymmetryChoice); 12] = [
        (Style::OpenCave, 2, 85, SymmetryChoice::Auto),
        (Style::OpenCave, 4, 100, SymmetryChoice::Auto),
        (Style::Chambers, 2, 85, SymmetryChoice::Auto),
        (
            Style::Chambers,
            4,
            110,
            SymmetryChoice::Mode(SymmetryMode::FourWay),
        ),
        (Style::Islands, 2, 85, SymmetryChoice::Auto),
        (Style::Islands, 4, 110, SymmetryChoice::Auto),
        (
            Style::River,
            2,
            85,
            SymmetryChoice::Mode(SymmetryMode::NorthSouth),
        ),
        (Style::River, 3, 110, SymmetryChoice::Auto),
        (Style::Fortress, 2, 85, SymmetryChoice::Auto),
        (Style::Fortress, 4, 110, SymmetryChoice::Auto),
        (Style::OpenCave, 5, 140, SymmetryChoice::Auto),
        (Style::Chambers, 1, 64, SymmetryChoice::Auto),
    ];
    for (k, (style, players, size, sym)) in specs.into_iter().enumerate() {
        let p = GenParams {
            seed: 500 + k as u64,
            width: size,
            height: size,
            style,
            players,
            symmetry: sym,
            lava: 30,
            water: 30,
            ..Default::default()
        };
        let g = generate(&gd, &p).unwrap_or_else(|e| panic!("{style:?}: {e}"));
        let a = Analysis::of_doc(&g.doc, &gd);
        println!(
            "tile {k}: {style:?} {players}p {size}x{size} {:?}: {} attempts, imbalance {:.3} {:?}, \
                repairs {:?}",
            g.report.symmetry,
            g.report.attempts,
            g.report.imbalance.unwrap_or(0.0),
            a.fairness.as_ref().map(|f| f.top(2)).unwrap_or_default(),
            g.report.repairs
        );
        let scale = (340 / size as u32).max(2);
        let img = render_overview(&a, scale);
        common::write_png(
            &common::out_dir().join(format!("gen_tile_{k:02}.png")),
            &img,
        );
        tiles.push(img);
    }
    let cell = 345u32;
    let mut sheet = RgbaImage::new(cell * 4, cell * 3, [20, 20, 20, 255]);
    for (k, t) in tiles.iter().enumerate() {
        sheet.blit(
            t,
            (k as u32 % 4 * cell) as i32,
            (k as u32 / 4 * cell) as i32,
        );
    }
    common::write_png(&common::out_dir().join("gen_contact_sheet.png"), &sheet);
}

/// A rectangle of a shipped-like map is regenerated as one undoable patch.
#[test]
fn patch_a_rectangle() {
    let Some(gd) = common::base_gd() else { return };
    let base = generate(
        &gd,
        &GenParams {
            seed: 9,
            width: 85,
            height: 85,
            players: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let mut doc = base.doc.clone();
    let rect = kfx_map::autotile::SlabRect {
        x0: 5,
        y0: 30,
        x1: 30,
        y1: 55,
    };
    let p = GenParams {
        seed: 10,
        players: 0,
        heroes: false,
        style: Style::Chambers,
        neutral_rooms: 1,
        neutral_creatures: 2,
        special_boxes: 1,
        ..Default::default()
    };
    let (patch, report) = kfx_analysis::generate::generate_patch(&mut doc, &gd, rect, &p)
        .unwrap_or_else(|e| panic!("{e}"));
    println!(
        "patch: {} changes, {} attempts",
        patch.changes.len(),
        report.attempts
    );
    assert_ne!(doc.kinds, base.doc.kinds);
    // outside the rectangle (and its one-slab ring) nothing changed
    for y in 0..85usize {
        for x in 0..85usize {
            let inside = (4..=31).contains(&x) && (29..=56).contains(&y);
            if !inside {
                let i = doc.dims.slab_index(x, y);
                assert_eq!(doc.kinds[i], base.doc.kinds[i], "({x},{y}) changed");
            }
        }
    }
    let errors: Vec<_> = validate(&doc, &gd)
        .into_iter()
        .filter(|x| x.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    patch.revert(&mut doc);
    assert_eq!(doc.kinds, base.doc.kinds);
    assert_eq!(doc.things.items, base.doc.things.items);
    assert_eq!(doc.columns, base.doc.columns);
}

/// Analysis time on a generated 170x170 map with 7 keepers.
#[test]
fn analysis_time_on_170() {
    let Some(gd) = common::base_gd() else { return };
    let g = generate(
        &gd,
        &GenParams {
            seed: 3,
            width: 170,
            height: 170,
            players: 7,
            style: Style::OpenCave,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let mut best = u64::MAX;
    let mut a = None;
    for _ in 0..3 {
        let x = Analysis::of_doc(&g.doc, &gd);
        best = best.min(x.timings.total_us);
        a = Some(x);
    }
    let mut a = a.unwrap();
    let t = a.timings;
    println!(
        "170x170, 7 keepers, {} gates: full analysis {:.1} ms (classify {:.1}, fields+routes \
            {:.1}, economy {:.1}, heroes+fairness {:.1}, symmetry {:.1})",
        a.sites.gates.len(),
        best as f64 / 1000.0,
        t.classify_us as f64 / 1000.0,
        t.fields_us as f64 / 1000.0,
        t.economy_us as f64 / 1000.0,
        t.heroes_us as f64 / 1000.0,
        t.symmetry_us as f64 / 1000.0
    );
    let t0 = Instant::now();
    a.update(
        &g.doc,
        &gd,
        kfx_map::autotile::SlabRect {
            x0: 80,
            y0: 80,
            x1: 84,
            y1: 84,
        },
    );
    println!(
        "update after a 5x5 edit: {:.1} ms; generation took {:.1} ms",
        t0.elapsed().as_micros() as f64 / 1000.0,
        g.report.time_us as f64 / 1000.0
    );
}
