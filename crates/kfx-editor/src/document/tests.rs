//! Tests of the document: locating levels, saving and reloading, the journal (undo / redo over
//! random strokes), slab placement, backups.
use super::*;
use kfx_formats::testutil;

/// A loaded shipped map with its game data, or `None` without a test install.
pub fn load_shipped(level: u32) -> Option<(PathBuf, Document)> {
    let root = testutil::install_dir()?;
    let dir = testutil::resolve_ci(&root, "campgns/keeporig")?;
    let loc = locate(&root, &dir, level).ok()?;
    let gd = Arc::new(GameData::load(&loc.layers).ok()?);
    Some((root, Document::load(&loc, gd).unwrap()))
}

fn copy_level(src: &Path, level: u32, dst: &Path) {
    let _ = std::fs::remove_dir_all(dst);
    std::fs::create_dir_all(dst).unwrap();
    for f in level_files(src, level) {
        std::fs::copy(
            &f,
            dst.join(
                f.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_ascii_lowercase(),
            ),
        )
        .unwrap();
    }
}

/// The `.lof` MAPSIZE follows the map's size, also back to 85x85: a 101x101 map shrunk to
/// 85x85 must not keep `MAPSIZE = 101 101` (the game would read it with the wrong size).
#[test]
fn lof_mapsize_follows_the_map() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let Some(src) = testutil::resolve_ci(&root, "multiplayer/classic") else {
        return;
    };
    let tmp = kfx_formats::testutil::scratch_dir(&format!("kfx-editor/lof_{}", std::process::id()));
    copy_level(&src, 631, &tmp);
    let loc = locate(&root, &tmp, 631).unwrap();
    assert_eq!((loc.dims.w, loc.dims.h), (101, 101));
    let gd = Arc::new(GameData::load(&loc.layers).unwrap());
    let mut d = Document::load(&loc, gd).unwrap();
    d.edit("Resize", |m, gd, _| {
        m.resize(gd, 0, 0, -16, -16, 0, 5).unwrap()
    });
    d.save_to(&tmp, 631).unwrap();
    let lof = Lof::load(&level_bin::find_file(&tmp, 631, "lof").unwrap()).unwrap();
    assert_eq!(
        lof.info().mapsize.unwrap_or((85, 85)),
        (85, 85),
        "the .lof must declare the saved size"
    );
    let again = locate(&root, &tmp, 631).unwrap();
    assert_eq!((again.dims.w, again.dims.h), (85, 85));
    // and growing it again
    d.edit("Resize", |m, gd, _| m.resize(gd, 0, 0, 5, 0, 0, 5).unwrap());
    d.save_to(&tmp, 631).unwrap();
    let lof = Lof::load(&level_bin::find_file(&tmp, 631, "lof").unwrap()).unwrap();
    assert_eq!(lof.info().mapsize, Some((90, 85)));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// For a campaign level the game reads `MAPSIZE` from the campaign's
/// `[mapNNNNN]` block (the `.lof` files of a campaign are not loaded, src/config_campaigns.c:1326).
#[test]
fn campaign_mapsize_goes_to_the_campaign_cfg() {
    let Some((root, mut d)) = load_shipped(1) else {
        return;
    };
    let tmp =
        kfx_formats::testutil::scratch_dir(&format!("kfx-editor/campsize_{}", std::process::id()));
    copy_level(&d.doc.folder.clone(), 1, &tmp);
    let cfg = tmp.join("test.cfg");
    std::fs::copy(
        testutil::resolve_ci(&root, "campgns/keeporig.cfg").unwrap(),
        &cfg,
    )
    .unwrap();
    d.doc.folder = tmp.clone();
    d.pack_cfg = Some((cfg.clone(), PackKind::Campaign));
    d.edit("Resize", |m, gd, _| m.resize(gd, 0, 0, 5, 0, 0, 5).unwrap());
    d.save_to(&tmp, 1).unwrap();
    let c = kfx_formats::campaign::Campaign::load(&cfg).unwrap();
    assert_eq!(c.map_info(1).and_then(|i| i.mapsize), Some((90, 85)));
    assert!(
        level_bin::find_file(&tmp, 1, "lof").is_none(),
        "the game does not read a campaign's .lof"
    );
    d.edit("Resize", |m, gd, _| {
        m.resize(gd, 0, 0, -5, 0, 0, 5).unwrap()
    });
    d.save_to(&tmp, 1).unwrap();
    let c = kfx_formats::campaign::Campaign::load(&cfg).unwrap();
    assert_eq!(
        c.map_info(1).and_then(|i| i.mapsize).unwrap_or((85, 85)),
        (85, 85)
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn map_numbers() {
    assert_eq!(map_number(Path::new("/x/MAP00007.SLB")), Some(7));
    assert_eq!(map_number(Path::new("map04008.slb")), Some(4008));
    assert_eq!(map_number(Path::new("foo.slb")), None);
}

#[test]
fn timestamps_look_right() {
    let t = timestamp();
    assert_eq!(t.len(), 15);
    assert!(t.starts_with("20"));
}

#[test]
fn locate_campaign_and_loose_maps() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let Some(dir) = testutil::resolve_ci(&root, "campgns/keeporig") else {
        return;
    };
    let loc = locate(&root, &dir, 1).unwrap();
    assert_eq!((loc.dims.w, loc.dims.h), (85, 85));
    assert!(loc.pack.is_some());
    assert!(loc.layers.layers.len() >= 2, "{:?}", loc.layers);
    // a copy outside any campaign: base layers, size from the .slb
    let tmp =
        kfx_formats::testutil::scratch_dir(&format!("kfx-editor/locate_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let slb = level_bin::find_file(&dir, 1, "slb").unwrap();
    std::fs::copy(&slb, tmp.join("map00001.slb")).unwrap();
    let l2 = locate(&root, &tmp, 1).unwrap();
    assert!(l2.pack.is_none());
    // base + the map's own overrides folder
    assert_eq!(l2.layers.layers.len(), 2);
    assert_eq!((l2.dims.w, l2.dims.h), (85, 85));
    std::fs::remove_dir_all(&tmp).unwrap();
}

/// Opening a map and saving it untouched keeps its slab and column data.
#[test]
fn untouched_save_keeps_map_data() {
    let Some((_, mut d)) = load_shipped(1) else {
        return;
    };
    let src = d.doc.folder.clone();
    let dims = d.dims();
    let out =
        kfx_formats::testutil::scratch_dir(&format!("kfx-editor/roundtrip_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    d.save_to(&out, 1).unwrap();
    let a = level_bin::LevelBin::load_with_dims(&src, 1, Some(dims)).unwrap();
    let b = level_bin::LevelBin::load_with_dims(&out, 1, Some(dims)).unwrap();
    assert_eq!(
        a.slb.as_ref().unwrap().kinds,
        b.slb.as_ref().unwrap().kinds,
        ".slb"
    );
    let (oa, ob) = (a.own.as_ref().unwrap(), b.own.as_ref().unwrap());
    let diff_own = oa
        .owners
        .iter()
        .zip(&ob.owners)
        .filter(|(x, y)| x != y)
        .count();
    assert_eq!(diff_own, 0, ".own differs in {diff_own} subtiles");
    assert_eq!(
        a.wib.as_ref().unwrap().values,
        b.wib.as_ref().unwrap().values,
        ".wib"
    );
    let col = |bin: &level_bin::LevelBin, i: usize| {
        let c =
            kfx_map::column::loaded_column(bin.clm.as_ref().unwrap(), bin.dat.as_ref().unwrap(), i);
        (c.cubes, c.floor_texture, c.solidmask, c.orient)
    };
    let mut bad = 0;
    for i in 0..dims.stl_grid() {
        if col(&a, i) != col(&b, i) {
            bad += 1;
        }
    }
    assert_eq!(bad, 0, "{bad} subtiles changed column");
    if let (Some(wa), Some(wb)) = (&a.wlb, &b.wlb) {
        assert_eq!(wa.values, wb.values, ".wlb");
    }
    assert!(!d.is_dirty());
    std::fs::remove_dir_all(&out).unwrap();
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(PartialEq, Debug, Clone)]
struct Snapshot {
    dims: MapDims,
    kinds: Vec<u16>,
    owners: Vec<u8>,
    wlb: Vec<u8>,
    slx: Vec<u8>,
    columns: Vec<kfx_formats::level_bin::Column>,
    wibble: Vec<u8>,
    things: Vec<kfx_formats::level_fx::FxThing>,
    aps: Vec<kfx_formats::level_fx::FxActionPoint>,
    lights: Vec<kfx_formats::level_fx::FxLight>,
    texture: u8,
}

fn snapshot(d: &Document) -> Snapshot {
    Snapshot {
        dims: d.doc.dims,
        kinds: d.doc.kinds.clone(),
        owners: d.doc.owners.clone(),
        wlb: d.doc.wlb.clone(),
        slx: d.doc.slx.clone(),
        columns: d.doc.columns.clone(),
        wibble: d.doc.wibble.clone(),
        things: d.doc.things.items.clone(),
        aps: d.doc.action_points.items.clone(),
        lights: d.doc.lights.items.clone(),
        texture: d.doc.texture_id,
    }
}

fn diff_report(a: &Snapshot, b: &Snapshot) -> String {
    format!(
        "dims {} kinds {} owners {} wlb {} slx {} cols {} wib {} things {} aps {} lights {}",
        a.dims == b.dims,
        a.kinds == b.kinds,
        a.owners == b.owners,
        a.wlb == b.wlb,
        a.slx == b.slx,
        a.columns == b.columns,
        a.wibble == b.wibble,
        a.things == b.things,
        a.aps == b.aps,
        a.lights == b.lights
    )
}

/// One random thing operation through the `kfx_map::things` API (as the thing tools do).
fn random_thing_op(d: &mut Document, rng: &mut Rng, n: usize) {
    use kfx_formats::level_fx::StlCoord;
    let dims = d.dims();
    let (sx, sy) = (
        rng.below(dims.stl_w() as u64) as i32,
        rng.below(dims.stl_h() as u64) as i32,
    );
    let owner = rng.below(6) as u8;
    let gd = d.gd.clone();
    match rng.below(9) {
        0 => {
            let t = things::new_thing(
                things::class::CREATURE,
                1 + rng.below(gd.creatures.len() as u64) as i64,
                owner,
                things::centred(sx),
                things::centred(sy),
                d.doc.floor_z(sx, sy),
            );
            d.op(&format!("thing {n}"), |m, g| m.add_thing(g, t));
        }
        1 => {
            let objs = gd.object_names();
            let m = objs[rng.below(objs.len() as u64) as usize].0 as i64;
            let t = things::new_thing(
                things::class::OBJECT,
                m,
                owner,
                StlCoord::new(sx, rng.below(256) as i32),
                things::centred(sy),
                d.doc.floor_z(sx, sy),
            );
            d.op(&format!("thing {n}"), |m, g| m.add_thing(g, t));
        }
        2 if !d.doc.things.items.is_empty() => {
            let i = rng.below(d.doc.things.items.len() as u64) as usize;
            d.op(&format!("thing {n}"), |m, g| ((), m.remove_thing(g, i)));
        }
        3 if !d.doc.things.items.is_empty() => {
            let i = rng.below(d.doc.things.items.len() as u64) as usize;
            d.op(&format!("thing {n}"), |m, g| {
                (
                    (),
                    m.move_thing(g, i, things::centred(sx), things::centred(sy), None),
                )
            });
        }
        4 => {
            let doors = gd.door_names();
            let m = doors
                .iter()
                .find(|(i, _)| *i > 0)
                .map(|x| x.0 as i64)
                .unwrap_or(1);
            let (x, y) = ((sx / 3) as usize, (sy / 3) as usize);
            let locked = rng.below(2) == 0;
            d.op(&format!("door {n}"), |doc, g| {
                doc.place_door(g, x, y, m, owner, locked)
                    .map(|(i, p)| (Some(i), p))
                    .unwrap_or((None, Patch::new()))
            });
        }
        5 => {
            let r = StlCoord::new(rng.below(8) as i32, 0);
            d.op(&format!("ap {n}"), |m, _| {
                m.add_action_point(things::centred(sx), things::centred(sy), r, None)
            });
        }
        6 => {
            d.op(&format!("light {n}"), |m, _| {
                m.add_light(
                    things::centred(sx),
                    things::centred(sy),
                    StlCoord::new(2, 0),
                    StlCoord::new(6, 0),
                    32,
                )
            });
        }
        7 if !d.doc.things.items.is_empty() => {
            let i = rng.below(d.doc.things.items.len() as u64) as usize;
            let mut t = d.doc.things.items[i].clone();
            t.owner = Some(owner as i64);
            t.orientation = Some(rng.below(2048) as i64);
            d.op(&format!("update {n}"), |m, g| ((), m.update_thing(g, i, t)));
        }
        _ => {
            let (x, y) = ((sx / 3) as usize, (sy / 3) as usize);
            d.op(&format!("owner {n}"), |m, g| {
                (
                    (),
                    m.set_slab_owner_with_things(g, x, y, owner)
                        .unwrap_or_default(),
                )
            });
        }
    }
}

/// 260 random strokes of every edit type (slab strokes split like mouse moves, thing / action
/// point / light operations, a resize and a rotation); undo all gives the original bit for
/// bit, redo all gives the edited state.
#[test]
fn undo_redo_exact_over_random_strokes() {
    let Some((_, mut d)) = load_shipped(2) else {
        return;
    };
    let original = snapshot(&d);
    let dims = d.dims();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let palette: Vec<u16> = vec![
        0, 1, 2, 4, 5, 9, 10, 11, 12, 13, 14, 16, 18, 20, 24, 26, 28, 32, 34, 36, 38, 40, 51, 52,
        53,
    ];
    let t0 = std::time::Instant::now();
    for n in 0..260 {
        if n % 4 == 3 {
            random_thing_op(&mut d, &mut rng, n);
            continue;
        }
        if n == 130 {
            let gd = d.gd.clone();
            d.op("resize", |m, _| {
                ((), m.resize(&gd, 2, 0, -3, 1, 2, 5).unwrap())
            });
            continue;
        }
        if n == 200 {
            d.op("rotate", |m, g| {
                (
                    (),
                    m.transform_rect(
                        g,
                        SlabRect {
                            x0: 10,
                            y0: 10,
                            x1: 19,
                            y1: 19,
                        },
                        kfx_map::transform::Transform::Rot90,
                    )
                    .unwrap(),
                )
            });
            continue;
        }
        let dims = d.dims();
        d.begin_stroke(&format!("op {n}"));
        let (cx, cy) = (
            rng.below(dims.w as u64) as i32,
            rng.below(dims.h as u64) as i32,
        );
        let kind = palette[rng.below(palette.len() as u64) as usize];
        let owner = rng.below(6) as u8;
        let op = rng.below(6);
        let cells: Vec<(i32, i32)> = match op {
            0 => crate::tools::brush_cells(1 + rng.below(5) as u32, rng.below(2) == 0)
                .into_iter()
                .map(|(x, y)| (cx + x, cy + y))
                .collect(),
            1 => crate::tools::line(
                (cx, cy),
                (cx + rng.below(9) as i32 - 4, cy + rng.below(9) as i32 - 4),
            ),
            2 => crate::tools::rect_cells(
                (cx, cy),
                (cx + rng.below(6) as i32, cy + rng.below(6) as i32),
                rng.below(2) == 0,
            ),
            3 => crate::tools::circle_cells((cx, cy), rng.below(4) as i32, rng.below(2) == 0),
            _ => vec![(cx, cy), (cx + 1, cy)],
        };
        let edits: Vec<(i32, i32, Edit)> = match op {
            4 => cells
                .iter()
                .map(|&(x, y)| (x, y, Edit::Owner(owner)))
                .collect(),
            5 => cells
                .iter()
                .map(|&(x, y)| (x, y, Edit::Style(rng.below(15) as u8)))
                .collect(),
            _ => cells
                .iter()
                .map(|&(x, y)| (x, y, Edit::Place { kind, owner }))
                .collect(),
        };
        // split the stroke in two batches like mouse moves
        let half = edits.len() / 2;
        d.apply(&edits[..half]);
        d.apply(&edits[half..]);
        d.end_stroke();
        // occasionally undo / redo in between
        if n % 37 == 5 {
            d.undo();
            d.redo();
        }
    }
    let steps = d.history().len();
    println!("260 operations ({steps} steps) in {:?}", t0.elapsed());
    let size: usize = d.history().iter().map(|c| c.size()).sum();
    let changes: usize = d.history().iter().map(|c| c.change_count()).sum();
    println!("journal: {changes} changes, ~{} KiB", size / 1024);
    let edited = snapshot(&d);
    assert_ne!(edited, original);
    assert_ne!(edited.things, original.things);
    assert_ne!(edited.dims, dims);
    let t1 = std::time::Instant::now();
    d.jump_to(0);
    println!("undo all in {:?}", t1.elapsed());
    let back = snapshot(&d);
    assert!(
        back == original,
        "undo all differs: {}",
        diff_report(&back, &original)
    );
    assert!(!d.is_dirty());
    d.jump_to(usize::MAX);
    let again = snapshot(&d);
    assert!(
        again == edited,
        "redo all differs: {}",
        diff_report(&again, &edited)
    );
    assert!(d.is_dirty());
}

/// The batch placement gives the same layout as MapDoc::place_slab one by one, and columns
/// equal to a full regeneration.
#[test]
fn placement_matches_mapdoc() {
    let Some((_, mut d)) = load_shipped(1) else {
        return;
    };
    d.decorator.enabled = false;
    let gd = d.gd.clone();
    d.doc.regenerate_columns(&gd);
    let mut reference = d.doc.clone();
    let cells: Vec<(i32, i32, u16, u8)> = vec![
        (40, 40, 16, 0),
        (41, 40, 16, 0),
        (42, 40, 11, 0),
        (43, 41, 10, 5),
        (39, 40, 2, 5),
        (10, 10, 12, 5),
        (11, 10, 13, 5),
        (45, 45, 4, 1),
    ];
    d.begin_stroke("t");
    let edits: Vec<(i32, i32, Edit)> = cells
        .iter()
        .map(|&(x, y, k, o)| (x, y, Edit::Place { kind: k, owner: o }))
        .collect();
    d.apply(&edits);
    d.end_stroke();
    for &(x, y, k, o) in &cells {
        reference
            .place_slab(&gd, x as usize, y as usize, k, o)
            .unwrap();
    }
    assert_eq!(d.doc.kinds, reference.kinds);
    assert_eq!(d.doc.owners, reference.owners);
    assert_eq!(d.doc.wlb, reference.wlb);
    let full = kfx_map::autotile::MapGen::regenerate_all(&d.doc.view(&gd), &gd);
    let dm = d.dims();
    for y in 0..dm.stl_h() {
        for x in 0..dm.stl_w() {
            let k = dm.stl_index(x, y);
            assert_eq!(d.doc.columns[k], full.columns[k], "subtile ({x},{y})");
        }
    }
}

/// Stroke latency on a 170x170 map: a 3x3 brush dragged over 60 slabs, each pointer move one
/// `Document::apply`, recorded with the snapshot `record` (before) and `record_tracked`
/// (now). Also checks both give the same map and that a custom slab survives the stroke.
#[test]
fn stroke_latency_170() {
    let Some(root) = testutil::install_dir() else {
        return;
    };
    let layers = base_layers(&root);
    let gd = Arc::new(GameData::load(&layers).unwrap());
    let mk = || {
        let mut d = Document::create(
            Path::new("/tmp"),
            1,
            MapDims::new(170, 170),
            kinds::EARTH as u16,
            true,
            0,
            gd.clone(),
            kfx_map::mods::stack_of_layers(&layers),
        );
        d.verify_tracked = false;
        d
    };
    let path = gd.slab_id("PATH").unwrap() as u16;
    let brush = crate::tools::brush_cells(3, false);
    let mut results = vec![];
    let mut maps = vec![];
    for tracked in [false, true] {
        let mut d = mk();
        d.tracked = tracked;
        // a custom slab in the way: it keeps its columns
        let cols: [kfx_formats::level_bin::Column; 9] = std::array::from_fn(|k| {
            d.doc.columns[d.dims().stl_index(3 * 90 + k % 3, 3 * 85 + k / 3)]
        });
        let id = d.doc.edit_sidecar(|s| {
            s.add_custom_slab(kfx_map::sidecar::CustomSlab::new(
                "pillar",
                kinds::ROCK as u16,
                cols,
            ))
        });
        d.apply(&[(90, 85, Edit::Custom { id, owner: 5 })]);
        d.end_stroke();
        let before_custom: Vec<_> = (0..9)
            .map(|k| d.doc.columns[d.dims().stl_index(3 * 90 + k % 3, 3 * 85 + k / 3)])
            .collect();
        d.begin_stroke("latency");
        let mut times = vec![];
        let mut last = (60, 85);
        for x in 61..=120 {
            let cells = crate::tools::stroke_cells(last, (x, 85), &brush);
            last = (x, 85);
            let edits: Vec<(i32, i32, Edit)> = cells
                .into_iter()
                .filter(|&(cx, cy)| (cx, cy) != (90, 85))
                .map(|(cx, cy)| {
                    (
                        cx,
                        cy,
                        Edit::Place {
                            kind: path,
                            owner: 0,
                        },
                    )
                })
                .collect();
            let t = std::time::Instant::now();
            d.apply(&edits);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        d.end_stroke();
        let after_custom: Vec<_> = (0..9)
            .map(|k| d.doc.columns[d.dims().stl_index(3 * 90 + k % 3, 3 * 85 + k / 3)])
            .collect();
        assert_eq!(
            before_custom, after_custom,
            "the custom slab kept its columns"
        );
        times.sort_by(|a, b| a.total_cmp(b));
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        results.push(format!(
            "{}: mean {mean:.2} ms, median {:.2} ms, max {:.2} ms per pointer move",
            if tracked {
                "record_tracked"
            } else {
                "record (snapshot)"
            },
            times[times.len() / 2],
            times[times.len() - 1]
        ));
        maps.push(snapshot(&d));
        // undo restores the start
        d.undo();
    }
    for r in &results {
        println!("stroke latency 170x170 (3x3 brush, 60 moves): {r}");
    }
    assert!(
        maps[0] == maps[1],
        "both recordings give the same map: {}",
        diff_report(&maps[0], &maps[1])
    );
}

#[test]
fn cancel_stroke_restores() {
    let Some((_, mut d)) = load_shipped(1) else {
        return;
    };
    let before = snapshot(&d);
    d.begin_stroke("x");
    d.apply(&[
        (30, 30, Edit::Place { kind: 12, owner: 5 }),
        (31, 30, Edit::Place { kind: 12, owner: 5 }),
    ]);
    assert!(d.cancel_stroke().is_some());
    assert!(snapshot(&d) == before);
    assert_eq!(d.history().len(), 0);
    assert!(!d.is_dirty());
}

#[test]
fn backup_copies_level_files() {
    let tmp =
        kfx_formats::testutil::scratch_dir(&format!("kfx-editor/backup_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("map00003.slb"), [1, 2]).unwrap();
    std::fs::write(src.join("MAP00003.TNG"), [3]).unwrap();
    std::fs::write(src.join("map00004.slb"), [9]).unwrap();
    let dest = backup_level(&src, 3, &tmp.join("backups"))
        .unwrap()
        .unwrap();
    assert!(dest.join("map00003.slb").is_file() && dest.join("MAP00003.TNG").is_file());
    assert!(!dest.join("map00004.slb").exists());
    assert!(backup_level(&src, 7, &tmp.join("backups"))
        .unwrap()
        .is_none());
    std::fs::remove_dir_all(&tmp).unwrap();
}
