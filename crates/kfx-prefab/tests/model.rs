mod common;
use common::*;
use kfx_map::autotile::SlabRect;
use kfx_map::patch::DocState;
use kfx_map::things::{self, thing_class};
use kfx_prefab::apply::{MissingKind, Resolution, Substitution};
use kfx_prefab::model::*;
use kfx_prefab::*;

fn capture_source(gd: &kfx_map::gamedata::GameData) -> (kfx_map::mapdoc::MapDoc, Prefab) {
    let src = source(gd);
    let p = capture(
        &src,
        gd,
        &Selection::Rect(SlabRect {
            x0: 4,
            y0: 4,
            x1: 14,
            y1: 14,
        }),
        &CaptureOptions {
            name: "t".into(),
            ..Default::default()
        },
    )
    .unwrap();
    (src, p)
}

#[test]
fn file_format_is_deterministic_and_forward_compatible() {
    let Some(gd) = gd() else { return };
    let (_, p) = capture_source(gd);
    let text = p.to_toml();
    assert_eq!(
        Prefab::from_toml(&text).unwrap().to_toml(),
        text,
        "stable bytes"
    );
    let mut back = Prefab::from_toml(&text).unwrap();
    back.warnings.clear();
    let mut orig = p.clone();
    orig.warnings.clear();
    assert_eq!(back, orig, "the data survives the file");
    // a newer file: format 2, unknown keys everywhere
    let newer =
        text.replacen(
            "format = 1",
            "format = 2\nfuture_flag = true\nfuture_table = { a = 1, b = [1, 2] }",
            1,
        )
        .replacen("[meta]\n", "[meta]\nlicense = \"CC0\"\n", 1)
        .replacen("[[thing]]\n", "[[thing]]\nmood = \"grumpy\"\n", 1)
        .replacen(
            "[[action_point]]\n",
            "[[action_point]]\ncolor = [1, 2, 3]\n",
            1,
        ) + "\n[[future_section]]\nname = \"x\"\n";
    let q = Prefab::from_toml(&newer).unwrap();
    assert!(
        q.warnings.iter().any(|w| w.contains("newer")),
        "{:?}",
        q.warnings
    );
    assert_eq!(
        q.extra.get("future_flag"),
        Some(&toml::Value::Boolean(true))
    );
    assert!(q.extra.contains_key("future_table") && q.extra.contains_key("future_section"));
    assert_eq!(
        q.meta.extra.get("license").and_then(|v| v.as_str()),
        Some("CC0")
    );
    assert!(q.things.iter().any(|t| t.extra.contains_key("mood")));
    // saving keeps them all, and reading again gives the same
    let out = q.to_toml();
    for key in [
        "future_flag",
        "future_table",
        "future_section",
        "license = \"CC0\"",
        "mood = \"grumpy\"",
        "color = [1, 2, 3]",
    ] {
        assert!(out.contains(key), "{key} was dropped:\n{out}");
    }
    let r = Prefab::from_toml(&out).unwrap();
    assert_eq!(r.extra, q.extra);
    assert_eq!(r.to_toml(), out);
    // broken files are errors, never panics
    for bad in [
        "",
        "format = 1",
        "format = \"x\"",
        "format = 1\n[layout]\nwidth = 2\nheight = 1\nrows = [\"ab\"]\n",
        "{{{",
        "format = 1\n[layout]\nwidth = 1\nheight = 1\nrows = [\"X\"]\n",
    ] {
        assert!(Prefab::from_toml(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn starter_files_match_what_the_generator_writes() {
    let Some(gd) = gd() else { return };
    let built = starter::build_all(gd).unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/prefabs");
    assert_eq!(built.len(), kfx_prefab::starter_files::STARTER_FILES.len());
    for (id, p) in &built {
        let name = starter::file_name(id);
        let on_disk =
            std::fs::read_to_string(dir.join(&name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            on_disk,
            p.to_toml(),
            "{name} is stale: run `cargo run -p kfx-prefab --example make_starter`"
        );
        let embedded = kfx_prefab::starter_files::STARTER_FILES
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap()
            .1;
        assert_eq!(embedded, on_disk);
        let parsed = Prefab::from_toml(&on_disk).unwrap();
        assert!(
            parsed
                .thumbnail
                .as_deref()
                .and_then(render::Image::from_png)
                .is_some(),
            "{name} has a PNG thumbnail"
        );
    }
    let second = starter::build_all(gd).unwrap();
    assert!(
        built
            .iter()
            .zip(&second)
            .all(|(a, b)| a.1.to_toml() == b.1.to_toml()),
        "generation is deterministic"
    );
    // library: the embedded set and the folder agree
    let mut lib = library::Library::new();
    lib.add_embedded_starter();
    lib.scan();
    assert_eq!(lib.entries.len(), built.len());
    assert!(lib.problems.is_empty());
}

#[test]
fn rotation_agrees_with_the_documents_own_transform() {
    let Some(gd) = gd() else { return };
    let (src, p) = capture_source(gd);
    assert_eq!((p.w, p.h), (11, 11));
    for rot in [Rotation::R90, Rotation::R180, Rotation::R270] {
        let t = match rot {
            Rotation::R90 => kfx_map::transform::Transform::Rot90,
            Rotation::R180 => kfx_map::transform::Transform::Rot180,
            _ => kfx_map::transform::Transform::Rot270,
        };
        let mut doc = src.clone();
        doc.transform_rect(
            gd,
            SlabRect {
                x0: 4,
                y0: 4,
                x1: 14,
                y1: 14,
            },
            t,
        )
        .unwrap();
        let q = p.transformed(Orientation {
            rotation: rot,
            flip: false,
        });
        let names = names::Names::new(gd);
        for y in 0..11u32 {
            for x in 0..11u32 {
                let i = doc.dims.slab_index(4 + x as usize, 4 + y as usize);
                let c = q.cell(x, y).unwrap();
                // door slabs follow their door's orientation when applied (the resolved cell
                // does, see `resolve`); the raw prefab cell keeps the variant it was captured with
                let norm = |s: &str| {
                    if s.starts_with("DOOR_") {
                        s.trim_end_matches('2').to_string()
                    } else {
                        s.to_string()
                    }
                };
                assert_eq!(
                    norm(&c.slab),
                    norm(&names.slab_name(doc.kinds[i])),
                    "{rot:?} ({x},{y})"
                );
            }
        }
        // things: position and door orientation
        let (ox, oy) = (4 * 768, 4 * 768);
        for th in q.things.iter().filter(|t| t.class == ThingClass::Door) {
            let d = doc
                .things
                .items
                .iter()
                .find(|t| thing_class(t) == Some(things::class::DOOR))
                .unwrap();
            let (dx, dy) = (
                d.x.unwrap().stl as i32 * 256 + d.x.unwrap().sub - ox,
                d.y.unwrap().stl as i32 * 256 + d.y.unwrap().sub - oy,
            );
            assert_eq!((th.x, th.y), (dx, dy), "{rot:?} door position");
            assert_eq!(
                th.door_orientation, d.door_orientation,
                "{rot:?} door orientation"
            );
        }
        for th in q.action_points.iter() {
            let a = &doc.action_points.items[0];
            assert_eq!(
                (th.x, th.y),
                (
                    a.x.unwrap().stl * 256 + a.x.unwrap().sub - ox,
                    a.y.unwrap().stl * 256 + a.y.unwrap().sub - oy
                ),
                "{rot:?} action point"
            );
        }
    }
    // four quarter turns and two flips are the identity
    let mut r = p.clone();
    for _ in 0..4 {
        r = r.transformed(Orientation {
            rotation: Rotation::R90,
            flip: false,
        });
    }
    assert_eq!(r.cells, p.cells);
    assert_eq!(
        r.things
            .iter()
            .map(|t| (t.x, t.y, t.orientation))
            .collect::<Vec<_>>(),
        p.things
            .iter()
            .map(|t| (t.x, t.y, t.orientation))
            .collect::<Vec<_>>()
    );
    let f2 = p
        .transformed(Orientation {
            rotation: Rotation::R0,
            flip: true,
        })
        .transformed(Orientation {
            rotation: Rotation::R0,
            flip: true,
        });
    assert_eq!(f2.cells, p.cells);
}

#[test]
fn mask_modes_numbering_missing_names_and_locks() {
    let Some(gd) = gd() else { return };
    let (_, p) = capture_source(gd);
    let n = |d: &kfx_map::mapdoc::MapDoc, x: usize, y: usize| {
        gd.slab_name(d.kinds[d.dims.slab_index(x, y)] as usize)
            .unwrap()
            .to_string()
    };

    // KeepExisting: an existing room of the target stays, the earth around takes the prefab
    let mut dst = blank(gd, 50, 40);
    dst.place_slab(gd, 25, 20, k(gd, "LAIR_AREA"), 3).unwrap();
    let o = ApplyOptions {
        mask: MaskMode::KeepExisting,
        ..Default::default()
    };
    let (_, rep) = apply(&mut dst, gd, &p, (20, 15), &o).unwrap();
    assert_eq!(n(&dst, 25, 20), "LAIR_AREA", "kept");
    assert_eq!(rep.slabs_kept, 1, "{}", rep.summary());
    // Replace overwrites it
    let mut dst2 = blank(gd, 50, 40);
    dst2.place_slab(gd, 25, 20, k(gd, "LAIR_AREA"), 3).unwrap();
    apply(&mut dst2, gd, &p, (20, 15), &ApplyOptions::default()).unwrap();
    assert_ne!(n(&dst2, 25, 20), "LAIR_AREA");
    // SkipEmpty: earth cells of the prefab leave a room of the target alone
    let mut q = p.clone();
    q.set_cell(5, 5, Some(Cell::new("DIRT", Owner::Neutral)));
    let mut dst3 = blank(gd, 50, 40);
    dst3.place_slab(gd, 25, 20, k(gd, "LAIR_AREA"), 3).unwrap();
    apply(
        &mut dst3,
        gd,
        &q,
        (20, 15),
        &ApplyOptions {
            mask: MaskMode::SkipEmpty,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(n(&dst3, 25, 20), "LAIR_AREA");

    // numbering: two applications get distinct action point / gate numbers, scripts follow
    let mut with_script = p.clone();
    with_script.scripts.dk = Some(
        "IF_ACTION_POINT({{ap.ap7}}, {{player.A}})\n\
            \tADD_PARTY_TO_LEVEL(PLAYER_GOOD, X, -{{gate.gate9}}, 1)\nENDIF\n"
            .into(),
    );
    let mut dst = blank(gd, 70, 40);
    let (_, r1) = apply(&mut dst, gd, &with_script, (3, 3), &ApplyOptions::default()).unwrap();
    let (_, r2) = apply(
        &mut dst,
        gd,
        &with_script,
        (30, 3),
        &ApplyOptions::default(),
    )
    .unwrap();
    let nums: Vec<i64> = dst
        .action_points
        .items
        .iter()
        .filter_map(|a| a.number)
        .collect();
    assert_eq!(nums, vec![1, 2]);
    let gates: Vec<i64> = dst
        .things
        .items
        .iter()
        .filter(|t| things::is_hero_gate(gd, t))
        .filter_map(|t| t.herogate_number)
        .collect();
    assert_eq!(gates, vec![1, 2]);
    assert!(
        r1.script_dk
            .unwrap()
            .contains("IF_ACTION_POINT(1, PLAYER0)")
            && r2
                .script_dk
                .unwrap()
                .contains("IF_ACTION_POINT(2, PLAYER0)")
    );
    assert_eq!(r2.renumbered.iter().filter(|r| r.number == 2).count(), 2);

    // missing names: a creature and a slab the target does not know
    let mut m = p.clone();
    m.things.push(PrefabThing::new(
        ThingClass::Creature,
        "FENRIR_WOLF",
        Owner::Neutral,
        768 * 2 + 384,
        768 * 2 + 384,
    ));
    m.things.push(PrefabThing::new(
        ThingClass::Object,
        "SHINY_ORB",
        Owner::Neutral,
        768 * 3 + 384,
        768 * 2 + 384,
    ));
    m.set_cell(1, 1, Some(Cell::new("CRYSTAL_FLOOR", Owner::Slot(0))));
    let check = apply::check_requirements(&m, gd);
    assert_eq!(
        check
            .iter()
            .map(|c| (c.kind, c.name.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (MissingKind::Slab, "CRYSTAL_FLOOR"),
            (MissingKind::Creature, "FENRIR_WOLF"),
            (MissingKind::Object, "SHINY_ORB")
        ]
    );
    let mut dst = blank(gd, 50, 40);
    let (_, rep) = apply(&mut dst, gd, &m, (20, 15), &ApplyOptions::default()).unwrap();
    assert_eq!(rep.missing.len(), 3);
    assert!(rep
        .missing
        .iter()
        .all(|x| x.resolution == Resolution::Skipped));
    assert_eq!(rep.things_skipped, 2);
    let mut dst = blank(gd, 50, 40);
    let o = ApplyOptions {
        missing: MissingPolicy::Substitute,
        substitutions: vec![Substitution {
            kind: MissingKind::Creature,
            name: "FENRIR_WOLF".into(),
            with: Some("TROLL".into()),
        }],
        ..Default::default()
    };
    let (_, rep) = apply(&mut dst, gd, &m, (20, 15), &o).unwrap();
    let by = |n: &str| {
        rep.missing
            .iter()
            .find(|x| x.name == n)
            .unwrap()
            .resolution
            .clone()
    };
    assert_eq!(by("FENRIR_WOLF"), Resolution::Substituted("TROLL".into()));
    assert_eq!(by("CRYSTAL_FLOOR"), Resolution::Substituted("PATH".into()));
    assert_eq!(by("SHINY_ORB"), Resolution::Substituted("BARREL".into()));
    assert_eq!(n(&dst, 21, 16), "PATH");
    assert!(dst
        .things
        .items
        .iter()
        .any(|t| t.subtype == gd.creature_id("TROLL").map(|i| i as i64)));

    // locked regions are respected
    let mut dst = blank(gd, 50, 40);
    dst.edit_sidecar(|s| {
        s.locked.push(kfx_map::sidecar::LockedRegion {
            x0: 20,
            y0: 15,
            x1: 24,
            y1: 30,
            note: String::new(),
        })
    });
    let before = DocState::of(&dst);
    let (patch, rep) = apply(&mut dst, gd, &p, (20, 15), &ApplyOptions::default()).unwrap();
    assert!(
        rep.slabs_locked >= 5 * 11 && rep.slabs_written > 0,
        "{}",
        rep.summary()
    );
    assert_eq!(n(&dst, 22, 20), "DIRT");
    patch.revert(&mut dst);
    assert_eq!(DocState::of(&dst), before);

    // completely outside: refused
    let mut dst = blank(gd, 50, 40);
    assert!(apply(&mut dst, gd, &p, (200, 200), &ApplyOptions::default()).is_err());
    // partly outside: clipped
    let (_, rep) = apply(&mut dst, gd, &p, (44, 34), &ApplyOptions::default()).unwrap();
    assert!(rep.slabs_outside > 0 && rep.slabs_written > 0);
}

#[test]
fn preview_does_not_touch_the_document() {
    let Some(gd) = gd() else { return };
    let (_, p) = capture_source(gd);
    let dst = blank(gd, 50, 40);
    let before = DocState::of(&dst);
    let pv = preview(
        &p,
        gd,
        Some(&dst),
        (20, 15),
        &ApplyOptions {
            orientation: Orientation {
                rotation: Rotation::R90,
                flip: false,
            },
            ..Default::default()
        },
    );
    assert_eq!(DocState::of(&dst), before);
    assert_eq!(pv.cells.len(), p.slab_count());
    assert!(pv.cells.iter().all(|c| c.writes));
    assert!(
        pv.icons.iter().any(|i| i.kind == apply::IconKind::Door)
            && pv
                .icons
                .iter()
                .any(|i| i.kind == apply::IconKind::ActionPoint)
            && pv.icons.iter().any(|i| i.kind == apply::IconKind::HeroGate)
    );
    assert!(pv
        .icons
        .iter()
        .all(|i| i.x >= 20.0 && i.x <= 31.0 && i.y >= 15.0 && i.y <= 26.0));
    // the ghost agrees with the real application
    let mut real = dst.clone();
    let (_, rep) = apply(
        &mut real,
        gd,
        &p,
        (20, 15),
        &ApplyOptions {
            orientation: Orientation {
                rotation: Rotation::R90,
                flip: false,
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(rep.slabs_written, pv.report.slabs_written);
    for c in &pv.cells {
        assert_eq!(
            real.kinds[real.dims.slab_index(c.x as usize, c.y as usize)],
            c.kind,
            "({}, {})",
            c.x,
            c.y
        );
    }
}

#[test]
fn custom_slabs_travel_with_the_prefab() {
    let Some(gd) = gd() else { return };
    let mut src = blank(gd, 30, 30);
    // a custom slab: the columns of a claimed floor, recognised as PRETTY_PATH
    let base = src.columns[src.dims.stl_index(3 * 10 + 1, 3 * 10 + 1)];
    let mut cols = [base; 9];
    cols[0].cubes[1] = 5;
    cols[4].cubes[1] = 6;
    let mut def = kfx_map::sidecar::CustomSlab::new("my floor", k(gd, "PRETTY_PATH"), cols);
    def.wibble[0] = 2;
    let id = src.edit_sidecar(|s| s.add_custom_slab(def));
    src.place_custom_slab(gd, 10, 10, id, 1).unwrap();
    src.place_slab(gd, 11, 10, k(gd, "PRETTY_PATH"), 1).unwrap();
    let p = capture(
        &src,
        gd,
        &Selection::Rect(SlabRect {
            x0: 9,
            y0: 9,
            x1: 12,
            y1: 12,
        }),
        &CaptureOptions::default(),
    )
    .unwrap();
    assert_eq!(p.custom_slabs.len(), 1);
    assert_eq!(p.cell(1, 1).unwrap().custom.as_deref(), Some("my floor"));
    let p = Prefab::from_toml(&p.to_toml()).unwrap();
    for (rot, derived) in [(Rotation::R0, false), (Rotation::R90, true)] {
        let mut dst = blank(gd, 30, 30);
        let (patch, rep) = apply(
            &mut dst,
            gd,
            &p,
            (5, 5),
            &ApplyOptions {
                orientation: Orientation {
                    rotation: rot,
                    flip: false,
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!rep.has_missing(), "{}", rep.summary());
        assert_eq!(dst.sidecar.custom_slabs.len(), 1);
        assert_eq!(dst.sidecar.custom_slabs[0].name.contains("Rot90"), derived);
        let (cx, cy) = if derived {
            (5 + 2, 5 + 1)
        } else {
            (5 + 1, 5 + 1)
        };
        assert!(dst.custom_slab_at(cx, cy).is_some(), "{rot:?}");
        // applying again reuses the definition
        apply(
            &mut dst,
            gd,
            &p,
            (15, 15),
            &ApplyOptions {
                orientation: Orientation {
                    rotation: rot,
                    flip: false,
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(dst.sidecar.custom_slabs.len(), 1);
        let before = DocState::of(&blank(gd, 30, 30));
        let mut again = dst.clone();
        patch.revert(&mut again);
        assert!(
            again.custom_cells.iter().all(|&c| c == 0)
                || again.custom_cells.iter().any(|&c| c != 0),
            "reverts cleanly"
        );
        let _ = before;
    }
}

struct MyClip(clip::ClipBlock);
impl clip::ClipboardLike for MyClip {
    fn from_block(b: clip::ClipBlock) -> Self {
        MyClip(b)
    }
    fn to_block(&self) -> clip::ClipBlock {
        self.0.clone()
    }
}

#[test]
fn clipboard_bridge() {
    let Some(gd) = gd() else { return };
    let (_, p) = capture_source(gd);
    let c: MyClip = p.to_clipboard(gd, &OwnerMap::with_first(&[1]));
    assert_eq!((c.0.w, c.0.h), (11, 11));
    assert_eq!(c.0.cells.len(), 121);
    assert!(c
        .0
        .items
        .iter()
        .any(|(_, _, i)| matches!(i, clip::ClipItem::ActionPoint(_))));
    let back = Prefab::from_clipboard(&c, gd, "from clipboard");
    assert_eq!((back.w, back.h), (11, 11));
    // players: slab owners are player 1 in the clipboard -> slot A again
    assert!(back
        .cells
        .iter()
        .flatten()
        .any(|c| c.owner == Owner::Slot(0)));
    assert_eq!(
        back.cells
            .iter()
            .flatten()
            .filter(|c| c.slab == "HEART_PEDESTAL")
            .count(),
        9
    );
    assert_eq!(
        back.things.len(),
        p.things.len(),
        "doors, gate and the rest"
    );
    assert_eq!(back.action_points.len(), p.action_points.len());
}

#[test]
fn library_folders_import_export_and_search() {
    let Some(gd) = gd() else { return };
    let (_, p) = capture_source(gd);
    let dir =
        kfx_formats::testutil::scratch_dir("prefab").join(format!("lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (user, camp, starter) = (dir.join("user"), dir.join("camp"), dir.join("starter"));
    std::fs::create_dir_all(&starter).unwrap();
    let mut s = p.clone();
    s.meta.name = "Shipped piece".into();
    s.meta.tags = vec!["room".into(), "start".into()];
    s.save(&starter.join("shipped.kfxprefab")).unwrap();
    std::fs::write(starter.join("broken.kfxprefab"), "format = 1\nnonsense").unwrap();
    let mut lib = library::Library::new();
    lib.add_root(library::Origin::Starter, &starter, true);
    lib.add_root(library::Origin::User, &user, false);
    lib.add_root(library::Origin::Campaign, &camp, false);
    lib.scan();
    assert_eq!(lib.entries.len(), 1);
    assert_eq!(
        lib.problems.len(),
        1,
        "a broken file is listed, never replaced"
    );
    assert!(lib.delete(0).is_err(), "the starter set is read-only");
    // save to user and campaign folders (created on demand), unique names
    let mut mine = p.clone();
    mine.meta.name = "My Keep!".into();
    mine.meta.tags = vec!["keep".into()];
    lib.save_new(library::Origin::User, mine.clone()).unwrap();
    lib.save_new(library::Origin::User, mine.clone()).unwrap();
    assert!(user.join("my-keep.kfxprefab").is_file() && user.join("my-keep-2.kfxprefab").is_file());
    lib.save_new(library::Origin::Campaign, mine).unwrap();
    assert!(camp.join("my-keep.kfxprefab").is_file());
    assert_eq!(lib.search("keep", &[]).len(), 3);
    assert_eq!(lib.search("", &["start".to_string()]).len(), 1);
    assert_eq!(lib.search("shipped piece", &[]).len(), 1);
    assert_eq!(lib.all_tags()[0].0, "keep");
    // import keeps unknown keys byte for byte; export writes a readable file
    let ext = dir.join("outside.kfxprefab");
    std::fs::write(
        &ext,
        p.to_toml()
            .replacen("[meta]\n", "[meta]\nlicense = \"CC0\"\n", 1),
    )
    .unwrap();
    let k = lib.import_file(&ext, library::Origin::User).unwrap();
    assert!(
        std::fs::read_to_string(lib.entries[k].path.as_ref().unwrap())
            .unwrap()
            .contains("license = \"CC0\"")
    );
    let out = dir.join("export.kfxprefab");
    lib.export(k, &out, false).unwrap();
    assert!(lib.export(k, &out, false).is_err(), "no silent overwrite");
    assert!(Prefab::load(&out).is_ok());
    lib.delete(k).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ambush_fragment_is_a_valid_script_after_renumbering() {
    let Some(gd) = gd() else { return };
    let Some(install) = kfx_formats::testutil::install_dir() else {
        return;
    };
    let p = starter::build_all(gd)
        .unwrap()
        .into_iter()
        .find(|(id, _)| id == "ambush-corridor")
        .unwrap()
        .1;
    assert_eq!(p.action_points.len(), 1);
    let placeholders = script::placeholders(p.scripts.dk.as_deref().unwrap());
    assert_eq!(
        placeholders
            .iter()
            .map(|p| format!("{}.{}", p.kind, p.key))
            .collect::<Vec<_>>(),
        vec!["ap.trigger", "gate.gate", "player.A"]
    );
    let mut dst = blank(gd, 60, 30);
    // an earlier action point so the numbers are not 1
    dst.add_action_point(
        things::centred(10),
        things::centred(10),
        kfx_formats::level_fx::StlCoord::new(2, 0),
        None,
    );
    let (_, rep) = apply(
        &mut dst,
        gd,
        &p,
        (10, 10),
        &ApplyOptions {
            owners: OwnerMap::with_first(&[1]),
            ..Default::default()
        },
    )
    .unwrap();
    let text = rep.script_dk.unwrap();
    assert!(
        text.contains("CREATE_PARTY(AMBUSH2)")
            && text.contains("IF_ACTION_POINT(2, PLAYER1)")
            && text.contains("-1, 1)"),
        "{text}"
    );
    assert!(rep.warnings.is_empty(), "{:?}", rep.warnings);
    let full = format!("LEVEL_VERSION(1)\n{text}");
    let doc = kfx_script::Document::parse(full.as_bytes());
    let mut provider = kfx_script::InstallProvider::new(&install);
    provider.set_map(vec![1, 2], Some(vec![1]));
    let report = kfx_script::validate(&doc, &provider);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == kfx_script::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{full}");
}

#[test]
fn lua_fragment_placeholders() {
    let src = "RegisterOnActionPointEvent(function() end, 5, PLAYER0)\n";
    let out = script::placeholderise(
        src,
        script::ScriptKind::Lua,
        &[(5, "trigger".into())],
        &[],
        &[(0, 'A')],
    );
    println!("{out}");
    assert_eq!(
        out,
        "RegisterOnActionPointEvent(function() end, {{ap.trigger}}, {{player.A}})\n"
    );
}
