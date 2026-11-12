//! Offscreen renders of the logic editor views, to look at the widgets without the editor
//! (egui_kittest + wgpu; nothing is shown on the desktop).
//!
//!   KFX_INSTALL=<install> cargo run -p kfx-logic-ui --example logic_shot -- [OUT_DIR] [SCRIPT]
//!
//! Writes PNGs to `OUT_DIR` (default `logic-ui/shots` of the test scratch folder,
//! [`kfx_formats::testutil::scratch_dir`]: `$KFX_TEST_TMP`, else `target/tmp`). `SCRIPT` is a DK
//! level script, by default `campgns/keeporig/map00018.txt` of the KeeperFX install named by
//! `KFX_INSTALL` (needed in any case for the script commands and constants).
use egui::{vec2, Color32, ColorImage, TextureHandle, TextureOptions};
use egui_kittest::Harness;
use kfx_logic_ui::{LogicEditor, LogicEvent, LogicHost, View};
use kfx_script::{ConstantProvider, InstallProvider};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

struct Host {
    provider: InstallProvider,
    portraits: HashMap<String, TextureHandle>,
}

impl LogicHost for Host {
    fn provider(&self) -> &dyn ConstantProvider {
        &self.provider
    }
    fn creature_portrait(&self, c: &str) -> Option<egui::TextureId> {
        self.portraits.get(&c.to_ascii_uppercase()).map(|t| t.id())
    }
    fn creature_strength(&self, c: &str, level: u32) -> Option<f32> {
        // stand-in stats for the review renders (a real host reads creature.cfg)
        let base = match c.to_ascii_uppercase().as_str() {
            "KNIGHT" | "AVATAR" => 9.0,
            "GIANT" | "SAMURAI" => 7.0,
            "BARBARIAN" | "WIZARD" | "MONK" => 5.0,
            "FAIRY" | "WITCH" | "ARCHER" => 4.0,
            "THIEF" | "DWARFA" | "TUNNELLER" => 3.0,
            _ => return None,
        };
        Some(base * (1.0 + 0.25 * (level as f32 - 1.0)))
    }
}

struct State {
    ed: LogicEditor,
    host: Host,
    events: Vec<LogicEvent>,
}

/// The KeeperFX install named by `KFX_INSTALL`; exits with a message when it is not set.
fn install() -> PathBuf {
    kfx_formats::testutil::install_dir().unwrap_or_else(|| {
        eprintln!("set KFX_INSTALL to the folder of a KeeperFX install");
        std::process::exit(2)
    })
}

/// A flat coloured tile with a darker frame: placeholder portraits supplied by the host.
fn portrait(ctx: &egui::Context, name: &str) -> TextureHandle {
    let h = name
        .bytes()
        .fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let c = Color32::from_rgb(
        (h & 0xff) as u8 / 2 + 90,
        ((h >> 8) & 0xff) as u8 / 2 + 60,
        ((h >> 16) & 0xff) as u8 / 2 + 60,
    );
    let mut img = ColorImage::filled([24, 24], c);
    for i in 0..24 {
        for (x, y) in [(i, 0), (i, 23), (0, i), (23, i)] {
            img[(x, y)] = Color32::from_gray(30);
        }
    }
    ctx.load_texture(format!("portrait-{name}"), img, TextureOptions::NEAREST)
}

fn harness(state: State, w: f32, h: f32) -> Harness<'static, State> {
    Harness::builder()
        .with_context_setup(kfx_fonts::install)
        .with_size(vec2(w, h))
        .with_pixels_per_point(1.0)
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(
            |ui, s: &mut State| {
                if s.host.portraits.is_empty() {
                    for c in [
                        "KNIGHT",
                        "AVATAR",
                        "GIANT",
                        "SAMURAI",
                        "BARBARIAN",
                        "WIZARD",
                        "MONK",
                        "FAIRY",
                        "WITCH",
                        "ARCHER",
                        "THIEF",
                        "DWARFA",
                        "TUNNELLER",
                    ] {
                        let t = portrait(ui.ctx(), c);
                        s.host.portraits.insert(c.to_string(), t);
                    }
                }
                let ev = s.ed.show(ui, &s.host);
                s.events.extend(ev);
            },
            state,
        )
}

fn save(h: &mut Harness<'static, State>, out: &Path, name: &str) {
    h.run_steps(5);
    let img = h.render().expect("render");
    let p = out.join(name);
    img.save(&p).expect("save png");
    println!("wrote {} ({}x{})", p.display(), img.width(), img.height());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| kfx_formats::testutil::scratch_dir("logic-ui/shots"));
    std::fs::create_dir_all(&out).unwrap();
    let inst = install();
    let script_path = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| inst.join("campgns/keeporig/map00018.txt"));
    let script = kfx_formats::rnc::load_file(&script_path).expect("script");
    let mut provider = InstallProvider::new(&inst);
    provider.set_map((1..=12).collect(), Some((1..=9).collect()));
    let mk = |provider: &InstallProvider| {
        let mut ed = LogicEditor::new();
        ed.load(&script);
        State {
            ed,
            host: Host {
                provider: provider.clone(),
                portraits: HashMap::new(),
            },
            events: vec![],
        }
    };

    // 1. event cards, the first conditional events expanded
    let mut h = harness(mk(&provider), 1400.0, 900.0);
    save(&mut h, &out, "01-cards.png");
    // select a tunneller-wave event and scroll to it
    let target = h
        .state()
        .ed
        .doc()
        .events()
        .into_iter()
        .find(|(e, _)| e.conditions.iter().any(|c| c.cmd.arg(1) == "TIMER3"))
        .map(|(e, _)| e.id);
    if let Some(t) = target {
        h.state_mut().ed.select(t);
        save(&mut h, &out, "02-cards-selected.png");
    }
    println!(
        "events: {:?}",
        h.state()
            .events
            .iter()
            .filter(|e| matches!(e, LogicEvent::Highlight { .. }))
            .collect::<Vec<_>>()
    );

    // 2. graph
    let mut s = mk(&provider);
    s.ed.set_view(View::Graph);
    let mut h = harness(s, 1400.0, 900.0);
    save(&mut h, &out, "03-graph.png");

    // 3. timeline
    let mut s = mk(&provider);
    s.ed.set_view(View::Timeline);
    let mut h = harness(s, 1400.0, 600.0);
    save(&mut h, &out, "04-timeline.png");

    // 4. parties
    let mut s = mk(&provider);
    s.ed.set_view(View::Parties);
    let mut h = harness(s, 1400.0, 900.0);
    save(&mut h, &out, "05-parties.png");

    // 5. variables
    let mut s = mk(&provider);
    s.ed.set_view(View::Variables);
    let mut h = harness(s, 1400.0, 600.0);
    save(&mut h, &out, "06-variables.png");

    // 6. Lua export
    let mut s = mk(&provider);
    s.ed.set_view(View::Lua);
    let mut h = harness(s, 1400.0, 900.0);
    save(&mut h, &out, "07-lua.png");

    // 8. wave templates + timeline: escalating waves of TROOPA from gate 1, an ambush, and a
    // flag chain made with the graph's connect
    let mut s = mk(&provider);
    s.ed.edit(|d| {
        let w = kfx_logic_ui::waves::EscalatingWaves {
            party: "TROOPA".into(),
            first_turn: 2000,
            interval: 1500,
            count: 4,
            ..Default::default()
        };
        for e in w.build(d) {
            d.nodes.push(kfx_logic_ui::Node::Event(e));
        }
        let t = kfx_logic_ui::waves::EscalatingWaves {
            party: "CASTERS".into(),
            spawn: kfx_logic_ui::waves::Spawn::Tunneller {
                location: "-2".into(),
                head_for: "DUNGEON_HEART".into(),
                target: "0".into(),
                level: 4,
                gold: 300,
            },
            first_turn: 4000,
            interval: 3000,
            count: 2,
            ..Default::default()
        };
        for e in t.build(d) {
            d.nodes.push(kfx_logic_ui::Node::Event(e));
        }
    });
    s.ed.set_view(View::Timeline);
    let mut h = harness(s, 1400.0, 700.0);
    h.run_steps(2);
    save(&mut h, &out, "09-timeline-waves.png");
    let evs: Vec<u64> = h
        .state()
        .ed
        .doc()
        .events()
        .into_iter()
        .filter(|(e, _)| !e.conditions.is_empty())
        .map(|(e, _)| e.id)
        .collect();
    let (a, b) = (evs[evs.len() - 2], evs[evs.len() - 1]);
    h.state_mut().ed.connect(a, b);
    h.state_mut().ed.set_view(View::Graph);
    save(&mut h, &out, "10-graph-after-edits.png");
    h.state_mut().ed.set_view(View::Cards);
    h.state_mut().ed.select(b);
    save(&mut h, &out, "11-cards-after-edits.png");
    std::fs::write(out.join("edited-script.txt"), h.state().ed.script_bytes()).unwrap();

    // 7. a view-only script
    let mut ed = LogicEditor::new();
    ed.load(b"LEVEL_VERSION(1)\nIF(PLAYER0,FLAG1 == 1)\n\tWIN_GAME\nREM the ENDIF is missing\n");
    let mut h = harness(
        State {
            ed,
            host: Host {
                provider: provider.clone(),
                portraits: HashMap::new(),
            },
            events: vec![],
        },
        1000.0,
        300.0,
    );
    save(&mut h, &out, "08-view-only.png");
}
