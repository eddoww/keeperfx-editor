//! Runs the real game (offscreen, `KFX_GAME_TEST=1`, one game at a time) on a shipped level
//! with `map00001.rules.cfg`, `map00001.terrain.cfg` and `map00001.slabset.toml` written by
//! this crate, and checks: the level starts, no new log errors / warnings versus the same
//! level without overrides, and the slabset override is observable: a slab changed to PATH by
//! a level Lua script gets the overridden column height.
//!
//! The harness is `kfx-map`'s (`crates/kfx-map/tests/support/game.rs`, used read-only).
//! The rig is unix only.
#![cfg(unix)]
#[path = "../../kfx-map/tests/support/game.rs"]
mod game;

use game::{GameRun, Rig};
use kfx_config_ui::{ConfigSession, LayerStack, StdFs};
use kfx_formats::level_bin::{self, MapDims};
use kfx_formats::{rnc, testutil};
use std::path::PathBuf;
use std::sync::Arc;

const SECS: u64 = 25;
const PATH: usize = 10;
/// Probed slab.
const PX: i32 = 20;
const PY: i32 = 20;

fn enabled() -> Option<PathBuf> {
    if std::env::var("KFX_GAME_TEST").ok().as_deref() != Some("1") {
        println!("KFX_GAME_TEST != 1: game test skipped");
        return None;
    }
    testutil::install_dir()
}

/// At turn 2 the probed slab becomes PATH (placed by the game with its slabset), at turn 6
/// its 9 subtile floor heights are printed.
fn probe_lua() -> String {
    format!(
        r#"-- kfx-config-ui game test
local function change()
  local s = GetSlab({PX}, {PY})
  print("KFXPROBE BEFORE " .. tostring(s.kind))
  s.kind = "PATH"
end
local function report()
  local row = {{}}
  for dy = 0, 2 do
    for dx = 0, 2 do
      row[#row + 1] = tostring(math.floor(GetFloorHeight(3 * {PX} + dx, 3 * {PY} + dy) / 256))
    end
  end
  print("KFXPROBE HGT " .. table.concat(row, " ") .. " KIND " .. tostring(GetSlab({PX}, {PY}).kind))
end
RegisterTimerEvent(change, 2, false)
RegisterTimerEvent(report, 6, false)
"#
    )
}

fn probe(run: &GameRun) -> Option<Vec<u32>> {
    let l = run.lines.iter().find(|l| l.raw.contains("KFXPROBE HGT"))?;
    let rest = &l.raw[l.raw.find("KFXPROBE HGT")? + 13..];
    let rest = rest.split(" KIND").next()?;
    Some(
        rest.split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect(),
    )
}

fn report(tag: &str, run: &GameRun) {
    println!("[{tag}] {}", run.summary());
    for l in run.lines.iter().filter(|l| l.raw.contains("KFXPROBE")) {
        println!("[{tag}]   {}", l.raw);
    }
    for l in run.problems() {
        println!("[{tag}]   {}", l.raw);
    }
}

#[test]
fn level_with_written_overrides_loads() {
    let Some(install) = enabled() else { return };
    let rig = Rig::new(&install, "config-ui");
    let camp = testutil::resolve_ci(&install, "campgns/keeporig").unwrap();
    let slb = rnc::load_file(&level_bin::find_file(&camp, 1, "slb").unwrap()).unwrap();
    let dims = MapDims::from_slb_len(slb.len()).unwrap();
    rig.install_level(1, &camp, 1, dims.w, dims.h, None);
    rig.write_level_file(1, "lua", &probe_lua());

    let base = rig.run(1, SECS);
    report("baseline", &base);
    assert!(base.started, "baseline did not start");
    let base_h = probe(&base).expect("baseline probe line");

    // write the overrides with the editor's session (map layer of the personal pack)
    let pack = rig.pack_dir();
    let layers = LayerStack::for_level(&StdFs, &install, None, None, Some(&pack), Some(1));
    let mut s = ConfigSession::new(Arc::new(StdFs), layers);
    // a column of height 2 (two solid cubes from the bottom)
    let cs = s.columnset();
    let col = (1..cs.count()).find(|&i| {
        let c = cs.col(i);
        c.cubes[0] != 0 && c.cubes[1] != 0 && c.cubes[2..].iter().all(|&x| x == 0)
    });
    let col = col.expect("a two-cube column in columnset.toml") as i64;
    for style in 0..28 {
        let mut st = s.slabset().style(PATH, style);
        st.columns = [col; 9];
        s.slabset().set_style(PATH, style, st);
    }
    let gph = s.cfg_value("rules.cfg", "game", "GoldPerHoard").unwrap();
    let new_gph = (gph.trim().parse::<i64>().unwrap() + 500).to_string();
    s.set_cfg("rules.cfg", "game", "GoldPerHoard", &new_gph);
    s.set_cfg("rules.cfg", "game", "PayDayGap", "9000");
    let w = s.cfg_value("terrain.cfg", "slab13", "Wibble").unwrap();
    s.set_cfg(
        "terrain.cfg",
        "slab13",
        "Wibble",
        if w.trim() == "0" { "1" } else { "0" },
    );
    let saved = s.save().unwrap();
    for p in &saved {
        println!(
            "wrote {}:\n{}",
            p.display(),
            std::fs::read_to_string(p)
                .unwrap()
                .lines()
                .take(12)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    assert_eq!(saved.len(), 3);

    let run = rig.run(1, SECS);
    report("overrides", &run);
    assert!(run.started, "level with overrides did not start");
    assert!(!run.crashed, "game crashed");
    let new = run.new_problems_vs(&base);
    for l in &new {
        println!("NEW vs baseline: {l}");
    }
    let h = probe(&run).expect("probe line");
    println!(
        "PATH placed at ({PX},{PY}): baseline heights {base_h:?}, with slabset override {h:?}"
    );
    // a shipped PATH floor is one cube high; the override uses a two-cube column
    assert!(
        base_h.len() == 9 && base_h.iter().all(|&x| x == 1),
        "baseline PATH should be 1 cube high"
    );
    assert!(
        h.len() == 9 && h.iter().all(|&x| x == 2),
        "overridden PATH columns should be 2 cubes high"
    );
    assert!(new.is_empty(), "new log problems: {new:?}");
}
