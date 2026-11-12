//! TOML constructs and map saves checked against the real game (`KFX_GAME_TEST=1`, offscreen, one
//! game at a time; see `support/game.rs`). Skipped otherwise. The rig is unix only.
//!
//! * Three TOML constructs the game reads (an empty parent table header, a quoted unknown key,
//!   three header levels) and a thing of a class the FX loader rejects: hand-written, then loaded
//!   and saved by the editor; the editor's file must load in the game.
//! * The editor's emitted TOML for 2-, 3- and 4-level tables (and arrays of tables), in an `.lgtfx`
//!   and in a per-level TOML config override, loads in the game.
//! * An FX map saved in each `SaveColumns` mode and in the legacy format loads with no new log
//!   problems.
#![cfg(unix)]
mod support;

use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::levelinfo::Lof;
use kfx_formats::{rnc, testutil, toml_emit};
use kfx_map::gamedata::{ConfigLayers, GameData};
use kfx_map::mapdoc::{MapDoc, SaveColumns};
use std::path::{Path, PathBuf};
use support::game::{GameRun, Rig};

const SECS: u64 = 25;

fn enabled() -> Option<PathBuf> {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: game tests skipped");
        return None;
    }
    testutil::install_dir()
}

fn dims_of(dir: &Path, n: u32) -> MapDims {
    if let Some(l) = level_bin::find_file(dir, n, "lof").and_then(|p| Lof::load(&p).ok()) {
        let (w, h) = l.info().mapsize_or_default();
        return MapDims::new(w as u16, h as u16);
    }
    MapDims::from_slb_len(
        rnc::load_file(&level_bin::find_file(dir, n, "slb").unwrap())
            .unwrap()
            .len(),
    )
    .unwrap()
}

fn script_of(dir: &Path, n: u32) -> Option<String> {
    level_bin::find_file(dir, n, "txt")
        .map(|p| String::from_utf8_lossy(&std::fs::read(p).unwrap()).to_string())
}

fn show(tag: &str, r: &GameRun) {
    println!("[{tag}] {}", r.summary());
    for l in r.problems().iter().take(25) {
        println!("[{tag}]   {}", l.raw);
    }
}

/// Lines saying the game could not load a TOML file.
fn toml_failures(r: &GameRun) -> Vec<String> {
    r.lines
        .iter()
        .filter(|l| {
            l.raw.contains("Unable to load")
                || l.raw.contains("No [common]")
                || l.raw.contains("Failed to load section")
        })
        .map(|l| l.raw.clone())
        .collect()
}

fn copy_map(src: &Path, n: u32, dst: &Path) {
    let _ = std::fs::remove_dir_all(dst);
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let f = e.file_name().to_string_lossy().to_ascii_lowercase();
        if f.starts_with(&format!("map{n:05}.")) && e.path().is_file() {
            std::fs::copy(e.path(), dst.join(&f)).unwrap();
        }
    }
}

/// Hand-written TOML constructs the game reads, with the editor's own save checked in the game.
#[test]
fn hand_written_toml_constructs_load_after_an_editor_save() {
    let Some(install) = enabled() else { return };
    let rig = Rig::new(&install, "safety-toml");
    let dir = install.join("multiplayer/classic");
    let n = 55;
    let dims = dims_of(&dir, n);
    let gd = GameData::load(&ConfigLayers::for_level(&install, None, None, None, None)).unwrap();
    let script = script_of(&dir, n);
    let read = |ext: &str| {
        String::from_utf8(std::fs::read(level_bin::find_file(&dir, n, ext).unwrap()).unwrap())
            .unwrap()
    };
    rig.install_level(1, &dir, n, dims.w, dims.h, script.as_deref());
    let base = rig.run(1, SECS);
    show("original", &base);
    assert!(base.started && !base.crashed);
    let work = rig.dir.join("safety-work");
    let mut failures = vec![];
    let variants: Vec<(&str, &str, String)> = vec![
        (
            "lgtfx-empty-parent",
            "lgtfx",
            format!("{}\n\n[foo]\n\n[foo.bar]\ny = 2\n", read("lgtfx")),
        ),
        (
            "tngfx-quoted-key",
            "tngfx",
            read("tngfx").replacen("[thing0]\n", "[thing0]\n\"Foo Bar\" = 1\n", 1),
        ),
        (
            "lgtfx-three-levels",
            "lgtfx",
            format!(
                "{}\n\n[deep]\nx = 1\n\n[deep.a]\nz = 1\n\n[deep.a.b]\nw = 1\n",
                read("lgtfx")
            ),
        ),
    ];
    for (name, ext, text) in variants {
        let src = work.join(name);
        copy_map(&dir, n, &src);
        std::fs::write(src.join(format!("map{n:05}.{ext}")), &text).unwrap();
        rig.install_level(1, &src, n, dims.w, dims.h, script.as_deref());
        let hand = rig.run(1, SECS);
        show(&format!("{name} hand-written"), &hand);
        println!(
            "[{name}] the game on the hand-written file: TOML failures {:?}",
            toml_failures(&hand)
        );
        let doc = MapDoc::load(&src, n, dims, &gd)
            .unwrap_or_else(|e| panic!("{name}: the editor cannot load it: {e}"));
        assert!(doc.unreadable.is_empty(), "{name}: {:?}", doc.unreadable);
        let out = work.join(format!("{name}-saved"));
        doc.save(&gd, &out, SaveColumns::Current).unwrap();
        rig.install_level(1, &out, n, dims.w, dims.h, script.as_deref());
        let saved = rig.run(1, SECS);
        show(&format!("{name} editor-saved"), &saved);
        if !saved.started || saved.crashed {
            failures.push(format!("{name}: the editor's save did not start"));
        }
        for l in toml_failures(&saved) {
            failures.push(format!("{name}: the game rejects the editor's file: {l}"));
        }
        for l in saved.new_problems_vs(&base) {
            failures.push(format!("{name}: new problem vs the original map: {l}"));
        }
    }
    // a Shot thing: the game refuses the whole level; the editor refuses to save it, and after
    // the explicit removal the level loads
    let t = read("tngfx");
    let cnt: usize = t
        .lines()
        .find_map(|l| l.strip_prefix("ThingsCount = "))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let shot =
        format!(
        "{}\n\n[thing{cnt}]\nThingType = \"Shot\"\nSubtype = 1\nOwnership = 0\nSubtileX = [40, \
            128]\nSubtileY = [40, 128]\nSubtileZ = [1, 0]\n",
        t.replacen(&format!("ThingsCount = {cnt}"), &format!("ThingsCount = {}", cnt + 1), 1)
    );
    let src = work.join("tngfx-shot");
    copy_map(&dir, n, &src);
    std::fs::write(src.join(format!("map{n:05}.tngfx")), &shot).unwrap();
    rig.install_level(1, &src, n, dims.w, dims.h, script.as_deref());
    let hand = rig.run(1, SECS);
    show("tngfx-shot hand-written", &hand);
    println!(
        "[tngfx-shot] the game on the hand-written file: started={}",
        hand.started
    );
    let mut doc = MapDoc::load(&src, n, dims, &gd).unwrap();
    let out = work.join("tngfx-shot-saved");
    assert!(
        doc.save(&gd, &out, SaveColumns::Current).is_err(),
        "the editor must refuse to write the Shot"
    );
    assert_eq!(doc.drop_game_rejected_things(&gd), 1);
    doc.save(&gd, &out, SaveColumns::Current).unwrap();
    rig.install_level(1, &out, n, dims.w, dims.h, script.as_deref());
    let saved = rig.run(1, SECS);
    show("tngfx-shot editor-saved", &saved);
    if !saved.started {
        failures.push("tngfx-shot: the editor's save (Shot removed) did not start".into());
    }
    let _ = std::fs::remove_dir_all(&work);
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Our emitted TOML for each table shape, as preserved FX tables and as a per-level config
/// override, loads in the game.
#[test]
fn emitted_table_forms_load_in_the_game() {
    let Some(install) = enabled() else { return };
    let rig = Rig::new(&install, "safety-forms");
    let dir = install.join("multiplayer/classic");
    let n = 55;
    let dims = dims_of(&dir, n);
    let gd = GameData::load(&ConfigLayers::for_level(&install, None, None, None, None)).unwrap();
    let script = script_of(&dir, n);
    rig.install_level(1, &dir, n, dims.w, dims.h, script.as_deref());
    let base = rig.run(1, SECS);
    show("original", &base);
    let cases: Vec<(&str, &str)> = vec![
        ("two levels, empty parent", "[kfxt]\n[kfxt.b]\ny = 2\n"),
        ("two levels", "[kfxt]\nx = 1\n[kfxt.b]\ny = 2\n"),
        (
            "three levels",
            "[kfxt]\nx = 1\n[kfxt.b]\ny = 2\n[kfxt.b.c]\nz = 3\n[kfxt.b.d]\nw = \
            4\n[kfxt.b.e]\nv = 5\n",
        ),
        (
            "four levels",
            "[kfxt]\n[kfxt.b.c.d]\nz = 3\n[kfxt.b.c.e]\nempty = {}\n",
        ),
        (
            "arrays of tables",
            "[[kfxa]]\nx = 1\n[[kfxa]]\nx = 2\n[kfxt]\n[[kfxt.list]]\ny = \
            1\n[[kfxt.list]]\ny = 2\n",
        ),
    ];
    let work = rig.dir.join("forms-work");
    let mut failures = vec![];
    for (name, src_toml) in cases {
        let tables: toml::Table = src_toml.parse().unwrap();
        // in the .lgtfx, through the FX writer
        let mut doc = MapDoc::load(&dir, n, dims, &gd).unwrap();
        doc.lights.other = tables.clone().into_iter().collect();
        let out = work.join("fx");
        let _ = std::fs::remove_dir_all(&out);
        doc.save(&gd, &out, SaveColumns::Current).unwrap();
        let lgtfx = std::fs::read_to_string(out.join(format!("map{n:05}.lgtfx"))).unwrap();
        rig.install_level(1, &out, n, dims.w, dims.h, script.as_deref());
        // and as a per-level slabset override (the loader ignores unknown tables)
        let cfg = toml_emit::document(&tables);
        rig.write_level_file(1, "slabset.toml", &cfg);
        let run = rig.run(1, SECS);
        show(name, &run);
        if !run.started || run.crashed {
            failures.push(format!("{name}: did not start"));
        }
        for l in toml_failures(&run) {
            failures.push(format!(
                "{name}: {l}\n--- lgtfx tail:\n{}\n--- slabset override:\n{cfg}",
                lgtfx
                    .lines()
                    .rev()
                    .take(12)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        for l in run.new_problems_vs(&base) {
            failures.push(format!("{name}: new problem: {l}"));
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    assert!(failures.is_empty(), "{failures:#?}");
}

/// An FX map saved in each column mode, and a legacy map saved in the legacy format, load in the
/// game with no new log problems.
#[test]
fn saves_in_every_mode_load_in_the_game() {
    let Some(install) = enabled() else { return };
    let rig = Rig::new(&install, "safety-modes");
    let mut failures = vec![];
    for (rel, n) in [("multiplayer/classic", 55u32), ("campgns/keeporig", 6)] {
        let dir = install.join(rel);
        let dims = dims_of(&dir, n);
        let script = script_of(&dir, n);
        let gd = GameData::load(&ConfigLayers::for_level(
            &install,
            None,
            Some(&rig.pack_dir()),
            None,
            Some(1),
        ))
        .unwrap();
        rig.install_level(1, &dir, n, dims.w, dims.h, script.as_deref());
        let base = rig.run(1, SECS);
        show(&format!("{rel}/{n} original"), &base);
        assert!(base.started && !base.crashed);
        let mut doc = MapDoc::load(&dir, n, dims, &gd).unwrap();
        doc.set_slab(&gd, 3, 3, 10).unwrap();
        let mut saves: Vec<(String, PathBuf)> = vec![];
        for which in [SaveColumns::Current, SaveColumns::Regenerate] {
            let out = rig.dir.join(format!("modes-{n}-{which:?}"));
            let _ = std::fs::remove_dir_all(&out);
            doc.save(&gd, &out, which).unwrap();
            saves.push((format!("{rel}/{n} {which:?}"), out));
        }
        if dims == MapDims::new(85, 85) {
            let out = rig.dir.join(format!("modes-{n}-legacy"));
            let _ = std::fs::remove_dir_all(&out);
            let rep = doc.save_legacy(&gd, &out, SaveColumns::Current).unwrap();
            println!("[{rel}/{n} legacy] warnings: {:?}", rep.warnings);
            assert!(level_bin::find_file(&out, n, "lgtfx").is_none());
            saves.push((format!("{rel}/{n} legacy"), out));
        }
        for (tag, out) in saves {
            rig.install_level(1, &out, n, dims.w, dims.h, script.as_deref());
            let run = rig.run(1, SECS);
            show(&tag, &run);
            if !run.started || run.crashed || run.exited_early {
                failures.push(format!("{tag}: did not run"));
            }
            for l in run.new_problems_vs(&base) {
                // a legacy level: the game tries `.lgtfx` first (src/lvl_filesdk1.c:1440) and logs
                // its absence, as it does for the shipped legacy maps
                if tag.ends_with("legacy") && l.contains(".lgtfx\" doesn't exist") {
                    continue;
                }
                failures.push(format!("{tag}: new problem: {l}"));
            }
            let _ = std::fs::remove_dir_all(&out);
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
