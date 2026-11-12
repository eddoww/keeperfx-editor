//! Timing of the editor pipeline (load, analysis, highlighting, edits) on the biggest level
//! scripts of a KeeperFX install, and on generated Lua scripts; prints the medians.
//!
//!   KFX_INSTALL=<install> cargo run --release -p kfx-script-ui --example bench
use kfx_script::InstallProvider;
use kfx_script_ui::{DkLanguage, Language, LuaLanguage, ScriptEditor, Theme};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn walk(dir: &Path, out: &mut Vec<(u64, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            let n = p
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            if n.starts_with("map") && n.ends_with(".txt") {
                out.push((e.metadata().map(|m| m.len()).unwrap_or(0), p));
            }
        }
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn time<R>(n: usize, mut f: impl FnMut() -> R) -> f64 {
    let mut v = vec![];
    for _ in 0..n {
        let t = Instant::now();
        std::hint::black_box(f());
        v.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    median(v)
}

fn report(name: &str, lang: &dyn Language, bytes: &[u8], host: &InstallProvider) {
    let lines = bytes.iter().filter(|&&b| b == b'\n').count();
    let parse = time(30, || lang.analyze(bytes));
    let analysis = lang.analyze(bytes);
    let validate = time(30, || analysis.validate(host));
    let text: String = bytes.iter().map(|&b| b as char).collect();
    let theme = Theme::default();
    let font = egui::FontId::monospace(14.0);
    let job = time(30, || {
        kfx_script_ui::build_job(&text, analysis.tokens(), &theme, &font, 18.0)
    });

    // whole frames through egui (layout of the galley included), headless
    let ctx = egui::Context::default();
    let mut ed = if lang.id() == "lua" {
        ScriptEditor::with_language(Box::new(LuaLanguage::new()))
    } else {
        ScriptEditor::new()
    };
    ed.set_text_bytes(bytes);
    let mut t = 0.0;
    let mut frame = |ed: &mut ScriptEditor| {
        t += 0.016;
        let input = egui::RawInput {
            time: Some(t),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            ..Default::default()
        };
        let t0 = Instant::now();
        let _ = ctx.run_ui(input, |ui| {
            ed.show(ui, host);
        });
        t0.elapsed().as_secs_f64() * 1000.0
    };
    let first = frame(&mut ed);
    frame(&mut ed);
    let steady = median((0..30).map(|_| frame(&mut ed)).collect());
    // a frame right after an edit: re-parse + re-highlight + re-layout (validation is debounced)
    let mut after_edit = vec![];
    for i in 0..15 {
        ed.set_caret(10);
        ed.insert_at_caret(if i % 2 == 0 { "X" } else { "" });
        after_edit.push(frame(&mut ed));
    }
    let after_edit = median(after_edit);
    println!(
        "{name}: {} bytes, {lines} lines | parse+highlight \
            {parse:.2} ms | validate {validate:.2} ms | highlight layout job {job:.2} ms | \
            frame: first {first:.1} ms, steady (cached) {steady:.2} ms, after an edit \
            {after_edit:.1} ms",
        bytes.len()
    );
}

fn main() {
    let Some(inst) = std::env::var_os("KFX_INSTALL").map(PathBuf::from) else {
        eprintln!("set KFX_INSTALL to the folder of a KeeperFX install");
        std::process::exit(2)
    };
    let mut files = vec![];
    for sub in ["campgns", "levels", "multiplayer"] {
        walk(&inst.join(sub), &mut files);
    }
    files.sort();
    files.reverse();
    println!(
        "{} scripts found; biggest: {}",
        files.len(),
        files[0].1.display()
    );
    let mut host = InstallProvider::new(&inst);
    host.set_map((1..=30).collect(), Some((1..=30).collect()));
    let biggest = std::fs::read(&files[0].1).unwrap();
    report("biggest shipped script", &DkLanguage, &biggest, &host);
    // a synthetic 20x script (thousands of lines) to see how it scales
    let mut big = vec![];
    for (_, p) in files.iter().take(12) {
        big.extend(std::fs::read(p).unwrap());
        big.push(b'\n');
    }
    report("12 biggest scripts concatenated", &DkLanguage, &big, &host);

    // Lua: a realistic level script repeated to 1000 lines
    let lua = lua_source(1000);
    report(
        "Lua, 1000 lines",
        &LuaLanguage::new(),
        lua.as_bytes(),
        &host,
    );
    let lua = lua_source(4000);
    report(
        "Lua, 4000 lines",
        &LuaLanguage::new(),
        lua.as_bytes(),
        &host,
    );
}

/// A level script of about `lines` lines: hooks, helper functions, trigger registrations, spawns.
fn lua_source(lines: usize) -> String {
    let mut s = String::from(
        "---@class Wave\n---@field size integer\n\nfunction \
            OnGameStart()\n\tStartMoney(PLAYER0, 3000)\n\tMaxCreatures(PLAYER0, 20)\nend\n\n",
    );
    let mut i = 0;
    while s.lines().count() < lines {
        i += 1;
        s.push_str(&format!(
            "---Spawn wave {i}\n---@param count integer\nfunction \
                SpawnWave{i}(eventData, triggerData)\n\tlocal n = {n}\n\tfor k = 1, n do\n\
                \t\tAddCreatureToLevel(PLAYER_GOOD, \"KNIGHT\", {ap}, 3, 0)\n\t\tif k % 2 == 0 \
                then\n\t\t\tAddCreatureToLevel(PLAYER_GOOD, \"ARCHER\", {ap}, 2, 0)\n\t\tend\n\
                \tend\n\tQuickObjective(\"Wave {i} is coming\", {ap})\n\tGame.waves = \
                (Game.waves or 0) + 1\nend\n\nfunction Setup{i}()\n\
                \tRegisterTimerEvent(SpawnWave{i}, {t}, false)\n\
                \tRegisterOnActionPointEvent(function() RevealMapLocation(PLAYER0, {ap}, 9) \
                end, {ap}, PLAYER0)\n\tlocal creatures = {{ \"IMP\", \"FLY\", \"BUG\" }}\n\tfor \
                _, c in ipairs(creatures) do CreatureAvailable(PLAYER0, c, true, 1) end\nend\n\n",
            n = 3 + i % 5,
            ap = 1 + i % 12,
            t = 1000 + i * 100
        ));
    }
    s
}
