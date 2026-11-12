//! Creature editor: per-creature model file (`creatrs/<name>.cfg`, layered as
//! `src/config_crtrmodel.c:2905-2950`): stats through the generic form, animated previews of every
//! `[sprites]` animation, experience growth table, `[experience]` powers, and "new creature
//! from copy".
use super::common::{image_box, image_fit, weak};
use super::form::cfg_section_form;
use super::Cx;
use crate::creature::{growth_table, new_creature_from_copy, ExpRates};
use egui::{Color32, RichText, Ui};
use kfx_map::gamedata::parse_number;

#[derive(Default)]
pub struct CreatureState {
    pub name: String,
    pub page: Page,
    pub anim: Option<String>,
    pub quarter: u8,
    pub new_name: String,
    pub message: Option<String>,
    pub filter: String,
    /// Slot of `[sounds]` that "Use" in the sound browser writes.
    pub sound_slot: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Stats,
    Sprites,
    Growth,
    Powers,
    Sounds,
}

const STAT_SECTIONS: [&str; 6] = [
    "attributes",
    "attraction",
    "annoyance",
    "senses",
    "appearance",
    "jobs",
];

fn key_of(name: &str) -> String {
    format!("creatrs/{}.cfg", name.to_ascii_lowercase())
}

pub fn show(ui: &mut Ui, cx: &mut Cx, st: &mut CreatureState) {
    let names = cx.s.creature_names();
    if st.name.is_empty() {
        st.name = names
            .iter()
            .find(|n| n.eq_ignore_ascii_case("DRAGON"))
            .cloned()
            .or_else(|| names.first().cloned())
            .unwrap_or_default();
    }
    let ctx = ui.ctx().clone();
    egui::Panel::left("creature-list")
        .resizable(true)
        .default_size(200.0)
        .show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text("filter"));
            egui::ScrollArea::vertical()
                .id_salt("crlist")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for n in &names {
                        if !st.filter.is_empty()
                            && !n
                                .to_ascii_lowercase()
                                .contains(&st.filter.to_ascii_lowercase())
                        {
                            continue;
                        }
                        let key = key_of(n);
                        let qs =
                            cx.s.cfg(&key)
                                .and_then(|c| c.at_target("sprites", "QuerySymbol"))
                                .map(|p| p.value)
                                .unwrap_or_default();
                        let ov = cx.s.cfg(&key).is_some_and(|c| !c.overrides().is_empty());
                        let assets = &*cx.assets;
                        let tex = cx.tex.get(&ctx, &format!("portrait:{qs}"), || {
                            assets.as_ref()?.portrait(&qs)
                        });
                        ui.horizontal(|ui| {
                            image_box(ui, tex.as_ref(), egui::vec2(28.0, 28.0));
                            let mut t = RichText::new(n);
                            if ov {
                                t = t.color(super::common::layer_color(cx.s.target_kind()));
                            }
                            if ui.selectable_label(st.name == *n, t).clicked() {
                                st.name = n.clone();
                                st.anim = None;
                            }
                        });
                    }
                });
        });
    egui::CentralPanel::default().show(ui, |ui| {
        let key = key_of(&st.name);
        ui.horizontal(|ui| {
            ui.heading(&st.name);
            for (p, l) in [
                (Page::Stats, "Stats"),
                (Page::Sprites, "Sprites"),
                (Page::Growth, "Growth"),
                (Page::Powers, "Powers"),
                (Page::Sounds, "Sounds"),
            ] {
                if ui.selectable_label(st.page == p, l).clicked() {
                    st.page = p;
                }
            }
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut st.new_name)
                    .hint_text("NEW_NAME")
                    .desired_width(110.0),
            );
            if ui
                .button("New creature from copy")
                .on_hover_text(
                    "Copies the effective file of this creature to \
                        creatrs/<new>.cfg at the target layer and adds it to creature.cfg",
                )
                .clicked()
            {
                st.message = Some(match new_creature_from_copy(cx.s, &st.name, &st.new_name) {
                    Ok(k) => {
                        let n = st.new_name.to_ascii_uppercase();
                        st.name = n;
                        format!("created {k} (unsaved)")
                    }
                    Err(e) => e,
                });
            }
            if let Some(m) = &st.message {
                ui.label(weak(m.clone()));
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt(("crpage", &st.name))
            .auto_shrink([false, false])
            .show(ui, |ui| match st.page {
                Page::Stats => {
                    for s in STAT_SECTIONS {
                        egui::CollapsingHeader::new(RichText::new(s).strong())
                            .default_open(s == "attributes")
                            .show(ui, |ui| {
                                cfg_section_form(ui, cx, &key, s, "");
                            });
                    }
                }
                Page::Sprites => sprites_page(ui, cx, st, &key),
                Page::Growth => growth_page(ui, cx, &key),
                Page::Powers => powers_page(ui, cx, &key),
                Page::Sounds => sounds_page(ui, cx, st, &key),
            });
    });
}

/// The `[sounds]` form, and the host's sound browser (when there is one) to listen to the
/// creature's sounds and assign another sound to a slot.
fn sounds_page(ui: &mut Ui, cx: &mut Cx, st: &mut CreatureState, key: &str) {
    cfg_section_form(ui, cx, key, "sounds", "");
    let Some(host) = cx.sound.as_deref_mut() else {
        return;
    };
    ui.separator();
    ui.label(RichText::new("Listen").strong());
    ui.label(weak(
        "The browser plays what the game resolves for this level \
            (saved files); \"Use\" assigns the selected sound to the slot below.",
    ));
    let slots: Vec<String> = cx.s.cfg(key).map(|c| c.keys("sounds")).unwrap_or_default();
    if st.sound_slot.is_empty() {
        st.sound_slot = slots.first().cloned().unwrap_or_default();
    }
    ui.horizontal(|ui| {
        ui.label("Assign to");
        egui::ComboBox::from_id_salt("creature-sound-slot")
            .selected_text(st.sound_slot.clone())
            .show_ui(ui, |ui| {
                for s in &slots {
                    ui.selectable_value(&mut st.sound_slot, s.clone(), s);
                }
            });
    });
    let mut pick = None;
    ui.push_id(("creature-sounds", &st.name), |ui| {
        ui.allocate_ui(egui::vec2(ui.available_width(), 380.0), |ui| {
            pick = host.browser(ui, super::SoundPlace::Creature { name: &st.name });
        });
    });
    if let Some(p) = pick {
        if !st.sound_slot.is_empty() {
            cx.s.set_cfg(key, "sounds", &st.sound_slot, &p.cfg_value());
            st.message = Some(format!("{} = {} (unsaved)", st.sound_slot, p.cfg_value()));
        }
    }
}

fn sprites_page(ui: &mut Ui, cx: &mut Cx, st: &mut CreatureState, key: &str) {
    let ctx = ui.ctx().clone();
    let keys: Vec<(String, String)> = match cx.s.cfg(key) {
        Some(c) => c
            .keys("sprites")
            .into_iter()
            .filter_map(|k| c.at_target("sprites", &k).map(|p| (k, p.value)))
            .collect(),
        None => vec![],
    };
    ui.horizontal(|ui| {
        ui.label("View");
        for (q, l) in ["front", "front-right", "right", "back-right", "back"]
            .iter()
            .enumerate()
        {
            if ui.selectable_label(st.quarter == q as u8, *l).clicked() {
                st.quarter = q as u8;
            }
        }
    });
    if let Some(a) = st.anim.clone() {
        if let Some((_, v)) = keys.iter().find(|(k, _)| *k == a) {
            let frames = cx.tex.anim(&ctx, cx.assets, v, st.quarter);
            ui.horizontal(|ui| {
                if !frames.is_empty() {
                    let i = (cx.time * 10.0) as usize % frames.len();
                    image_fit(
                        ui,
                        &frames[i],
                        egui::vec2(220.0, 220.0),
                        egui::Sense::hover(),
                    );
                    ui.label(format!(
                        "{a} = {v}\n{} frames, frame {}",
                        frames.len(),
                        i + 1
                    ));
                } else {
                    ui.label(weak(format!("{a} = {v}: no sprite data")));
                }
            });
            ui.separator();
        }
    }
    ui.horizontal_wrapped(|ui| {
        for (k, v) in &keys {
            let is_icon = k.to_ascii_lowercase().ends_with("symbol");
            ui.allocate_ui_with_layout(
                egui::vec2(96.0, 130.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(96.0);
                    if is_icon {
                        let assets = &*cx.assets;
                        let vv = v.clone();
                        let gui1 = k.eq_ignore_ascii_case("QuerySymbol")
                            || k.eq_ignore_ascii_case("HandSymbol");
                        let tex = cx.tex.get(
                            &ctx,
                            &format!("icon:{}:{v}", if gui1 { 1 } else { 2 }),
                            || {
                                let a = assets.as_ref()?;
                                if gui1 {
                                    a.portrait(&vv)
                                } else {
                                    a.panel(&vv)
                                }
                            },
                        );
                        image_box(ui, tex.as_ref(), egui::vec2(80.0, 80.0));
                    } else {
                        let frames = cx.tex.anim(&ctx, cx.assets, v, st.quarter);
                        let tex = (!frames.is_empty())
                            .then(|| frames[(cx.time * 10.0) as usize % frames.len()].clone());
                        if image_box(ui, tex.as_ref(), egui::vec2(80.0, 80.0)).clicked() {
                            st.anim = Some(k.clone());
                        }
                    }
                    ui.label(RichText::new(k).small());
                    let mut buf = v.clone();
                    if ui
                        .add(egui::TextEdit::singleline(&mut buf).desired_width(80.0))
                        .changed()
                    {
                        cx.s.set_cfg(key, "sprites", k, &buf);
                        cx.tex.forget_prefix(&format!("anim:{buf}"));
                    }
                },
            );
        }
    });
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
}

fn growth_page(ui: &mut Ui, cx: &mut Cx, key: &str) {
    let rates = ExpRates::load(cx.s);
    let vals: Vec<(String, i64)> = match cx.s.cfg(key) {
        Some(c) => ["attributes", "attraction", "jobs"]
            .iter()
            .flat_map(|sec| {
                c.keys(sec)
                    .into_iter()
                    .filter_map(|k| {
                        c.at_target(sec, &k)
                            .and_then(|p| parse_number(p.value.split_whitespace().next()?))
                            .map(|n| (k, n))
                    })
                    .collect::<Vec<_>>()
            })
            .collect(),
        None => vec![],
    };
    let base = |k: &str| {
        vals.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| *v)
    };
    let table = growth_table(&base, &rates);
    // compute_creature_max_*, src/thing_stats.c
    ui.label(weak(
        "Values per experience level, as the game computes them from [attributes] \
            and creature.cfg [experience].",
    ));
    egui::Grid::new("growth")
        .striped(true)
        .num_columns(11)
        .show(ui, |ui| {
            ui.label(RichText::new("stat").strong());
            for l in 1..=10 {
                ui.label(RichText::new(format!("L{l}")).strong());
            }
            ui.end_row();
            for (name, row) in table {
                ui.label(name);
                for v in row {
                    ui.label(RichText::new(v.to_string()).monospace());
                }
                ui.end_row();
            }
        });
    ui.label(weak(format!(
        "rates (%/level): health {} strength {} dexterity {} defense \
            {} armour {} loyalty {} pay {} job value {}",
        rates.health,
        rates.strength,
        rates.dexterity,
        rates.defense,
        rates.armour,
        rates.loyalty,
        rates.pay,
        rates.job_value
    )));
    if let Some(c) = cx.s.cfg(key) {
        if let Some(p) = c.at_target("experience", "LevelsTrainValues") {
            ui.label(format!("training points to reach L2..L10: {}", p.value));
        }
    }
}

fn powers_page(ui: &mut Ui, cx: &mut Cx, key: &str) {
    let (powers, levels) = match cx.s.cfg(key) {
        Some(c) => (
            c.at_target("experience", "Powers")
                .map(|p| p.value)
                .unwrap_or_default(),
            c.at_target("experience", "PowersLevelRequired")
                .map(|p| p.value)
                .unwrap_or_default(),
        ),
        None => return,
    };
    let mut p: Vec<String> = kfx_formats::cfg::split_tokens(&powers);
    let mut l: Vec<String> = kfx_formats::cfg::split_tokens(&levels);
    let n = p.len().max(l.len()).max(10);
    p.resize(n, "NULL".into());
    l.resize(n, "0".into());
    let opts = {
        let mut v = cx.s.names("instance");
        v.insert(0, (0, "NULL".into()));
        v
    };
    let mut changed = false;
    ui.label(weak(
        "Instances (spells / abilities) the creature learns and the level each needs.",
    ));
    egui::Grid::new("powers").striped(true).show(ui, |ui| {
        for i in 0..n {
            ui.label(format!("slot {}", i + 1));
            changed |=
                super::common::picker(ui, ("pow", i), &mut p[i], &opts, 200.0, |_, _, _| None);
            let mut lv: i64 = l[i].parse().unwrap_or(0);
            if ui
                .add(egui::DragValue::new(&mut lv).range(0..=10))
                .changed()
            {
                l[i] = lv.to_string();
                changed = true;
            }
            if p[i] != "NULL" && lv == 0 {
                ui.label(
                    RichText::new("Level 0: never learned")
                        .small()
                        .color(Color32::from_rgb(220, 180, 80)),
                );
            }
            ui.end_row();
        }
    });
    if changed {
        cx.s.set_cfg(key, "experience", "Powers", &p.join(" "));
        cx.s.set_cfg(key, "experience", "PowersLevelRequired", &l.join(" "));
    }
    ui.separator();
    cfg_section_form(ui, cx, key, "experience", "");
}
