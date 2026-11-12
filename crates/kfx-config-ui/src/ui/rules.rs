//! Rules editor: `rules.cfg` named-field blocks through the generic form, research order and
//! sacrifice recipes as list editors (`parse_rules_research_blocks` /
//! `parse_rules_sacrifices_blocks`, src/config_rules.c:432-705; both blocks replace the whole list
//! when present in a later layer, so they are written complete, and dropped again when they
//! equal the inherited list). Rows that differ from the inherited list are marked in the target
//! layer's colour; entries the game would reject or ignore are flagged.
use super::common::{badge, layer_color, picker, weak};
use super::form::{cfg_section_form, raw_view, FormState};
use super::Cx;
use egui::{Color32, RichText, Ui};
use kfx_formats::cfg::split_tokens;

const FILE: &str = "rules.cfg";

pub struct RulesState {
    pub section: String,
    pub form: FormState,
}

impl Default for RulesState {
    fn default() -> Self {
        RulesState {
            section: "game".into(),
            form: FormState::new(FILE),
        }
    }
}

/// `research_desc` (src/config_rules.c:240).
const RESEARCH_TYPES: [&str; 3] = ["MAGIC", "ROOM", "CREATURE"];
/// `rules_sacrifices_commands` the game handles (src/config_rules.c:247; CUSTOMREWARD /
/// CUSTOMPUNISH have no handler).
const SAC_COMMANDS: [&str; 6] = [
    "MKCREATURE",
    "MKGOODHERO",
    "NEGSPELLALL",
    "POSSPELLALL",
    "NEGUNIQFUNC",
    "POSUNIQFUNC",
];
/// `sacrifice_unique_desc` (src/config_rules.c:259).
/// `MAX_SACRIFICE_VICTIMS` (src/config_rules.h:27).
const MAX_VICTIMS: usize = 6;
/// `MAX_SACRIFICE_RECIPES` (src/config_rules.h:28); slot 0 is never used
/// (`get_unused_sacrifice_recipe_slot`, src/config_rules.c:318-327).
const MAX_RECIPES: usize = 99;
const UNIQUE: [&str; 8] = [
    "ALL_CREATRS_ANGRY",
    "ALL_CREATRS_HAPPY",
    "COMPLETE_RESEARCH",
    "COMPLETE_MANUFACTR",
    "KILL_ALL_CHICKENS",
    "CHEAPER_IMPS",
    "COSTLIER_IMPS",
    "ALL_CREATRS_VER_ANGRY",
];

pub fn show(ui: &mut Ui, cx: &mut Cx, st: &mut RulesState) {
    let sections: Vec<String> =
        cx.s.cfg(FILE)
            .map(|c| c.section_names())
            .unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        for s in &sections {
            let ov = cx.s.cfg(FILE).is_some_and(|c| c.target_has_section(s));
            let mut t = RichText::new(s);
            if ov {
                t = t.color(layer_color(cx.s.target_kind()));
            }
            if ui
                .selectable_label(st.section.eq_ignore_ascii_case(s), t)
                .clicked()
            {
                st.section = s.clone();
                st.form.raw = false;
            }
        }
        ui.separator();
        if ui.selectable_label(st.form.raw, "Raw / diff").clicked() {
            st.form.raw = !st.form.raw;
        }
    });
    ui.separator();
    if st.form.raw {
        raw_view(ui, cx, &mut st.form);
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("rules")
        .auto_shrink([false, false])
        .show(ui, |ui| match st.section.to_ascii_lowercase().as_str() {
            "research" => research(ui, cx),
            "sacrifices" => sacrifices(ui, cx),
            s => cfg_section_form(ui, cx, FILE, s, ""),
        });
}

/// Header of a list block; returns the list as of the target layer and the inherited one.
fn block_header(
    ui: &mut Ui,
    cx: &mut Cx,
    sec: &str,
) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let (layer, lines) =
        cx.s.cfg(FILE)
            .and_then(|c| c.block_at_target(sec))
            .unwrap_or((0, vec![]));
    let inherited =
        cx.s.cfg(FILE)
            .and_then(|c| c.inherited_block(sec))
            .map(|b| b.1)
            .unwrap_or_default();
    let kind = cx.s.layers.layers[layer].kind.clone();
    ui.horizontal(|ui| {
        ui.heading(sec);
        ui.label("from");
        badge(ui, &kind.badge(), layer_color(&kind));
        let has = cx.s.cfg(FILE).is_some_and(|c| c.target_has_section(sec));
        if has
            && ui
                .small_button("Revert list")
                .on_hover_text("Remove the list from the target layer (the inherited list applies)")
                .clicked()
            && cx.s.cfg(FILE).is_some_and(|c| c.revert_section(sec))
        {
            cx.s.touched(FILE);
        }
        if let Some(l) = cx.s.cfg(FILE).and_then(|c| c.shadowed_by(sec, "")) {
            let k = cx.s.layers.layers[l].kind.clone();
            ui.label(
                RichText::new(format!("{} replaces this list in the game", k.badge()))
                    .small()
                    .color(Color32::from_rgb(230, 90, 80)),
            );
        }
    });
    ui.label(
        RichText::new(
            "A later layer's block replaces the whole list; the list is written complete.",
        )
        .small()
        .color(Color32::from_rgb(220, 180, 80)),
    );
    (lines, inherited)
}

/// Same entry (key and tokens, case-insensitive)?
fn same_line(a: Option<&(String, String)>, key: &str, toks: &[String]) -> bool {
    a.is_some_and(|(k, v)| {
        let t = split_tokens(v);
        k.eq_ignore_ascii_case(key)
            && t.len() == toks.len()
            && t.iter().zip(toks).all(|(x, y)| x.eq_ignore_ascii_case(y))
    })
}

/// Row number label, in the target layer's colour when the row differs from the inherited list.
fn row_label(ui: &mut Ui, cx: &Cx, i: usize, changed: bool) {
    let mut t = RichText::new(format!("{}", i + 1));
    if changed {
        t = t.strong().color(layer_color(cx.s.target_kind()));
    }
    ui.label(t).on_hover_text(if changed {
        "differs from the inherited list"
    } else {
        "as inherited"
    });
}

fn warn(ui: &mut Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(text.into())
            .small()
            .color(Color32::from_rgb(230, 150, 60)),
    );
}

fn known(opts: &[(i64, String)], v: &str) -> bool {
    opts.iter().any(|(_, n)| n.eq_ignore_ascii_case(v))
}

fn commit(cx: &mut Cx, sec: &str, lines: Vec<(String, String)>) {
    if cx.s.cfg(FILE).is_some_and(|c| c.set_block(sec, &lines)) {
        cx.s.touched(FILE);
    }
}

fn research(ui: &mut Ui, cx: &mut Cx) {
    let (lines, inherited) = block_header(ui, cx, "research");
    let inherited: Vec<(String, String)> = inherited
        .into_iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("Research"))
        .collect();
    let mut items: Vec<[String; 3]> = lines
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("Research"))
        .map(|(_, v)| {
            let t = split_tokens(v);
            [
                t.first().cloned().unwrap_or_default(),
                t.get(1).cloned().unwrap_or_default(),
                t.get(2).cloned().unwrap_or_default(),
            ]
        })
        .collect();
    let mut changed = false;
    let mut action: Option<(usize, i32)> = None;
    // parse_rules_research_blocks, src/config_rules.c:432
    ui.label(weak(format!(
        "{} items, researched in this order",
        items.len()
    )));
    egui::Grid::new("research")
        .striped(true)
        .num_columns(6)
        .show(ui, |ui| {
            ui.label(weak("#"));
            ui.label(weak("type"));
            ui.label(weak("item"));
            ui.label(weak("points"));
            ui.label("");
            ui.label("");
            ui.end_row();
            for (i, it) in items.iter_mut().enumerate() {
                row_label(ui, cx, i, !same_line(inherited.get(i), "Research", &it[..]));
                let mut ty = it[0].clone();
                egui::ComboBox::from_id_salt(("rtype", i))
                    .selected_text(ty.clone())
                    .width(90.0)
                    .show_ui(ui, |ui| {
                        for t in RESEARCH_TYPES {
                            ui.selectable_value(&mut ty, t.to_string(), t);
                        }
                    });
                if ty != it[0] {
                    it[0] = ty;
                    changed = true;
                }
                let kind = match it[0].to_ascii_uppercase().as_str() {
                    "MAGIC" => "power",
                    "ROOM" => "room",
                    _ => "creature",
                };
                let opts = cx.s.names(kind);
                changed |= picker(ui, ("ritem", i), &mut it[1], &opts, 170.0, |_, _, _| None);
                let mut n: i64 = it[2].parse().unwrap_or(0);
                if ui
                    .add(
                        egui::DragValue::new(&mut n)
                            .range(0..=i32::MAX as i64)
                            .speed(10.0),
                    )
                    .on_hover_text("Research points")
                    .changed()
                {
                    it[2] = n.to_string();
                    changed = true;
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Up").clicked() {
                        action = Some((i, -1));
                    }
                    if ui.small_button("Down").clicked() {
                        action = Some((i, 1));
                    }
                    if ui
                        .small_button("+")
                        .on_hover_text("Insert a copy below")
                        .clicked()
                    {
                        action = Some((i, 2));
                    }
                    if ui.small_button("x").on_hover_text("Remove").clicked() {
                        action = Some((i, 0));
                    }
                });
                ui.horizontal(|ui| {
                    if !RESEARCH_TYPES
                        .iter()
                        .any(|t| t.eq_ignore_ascii_case(&it[0]))
                    {
                        warn(ui, "unknown type: the game skips this line");
                    } else if !known(&opts, &it[1]) {
                        warn(ui, format!("no {kind} named {}", it[1]));
                    }
                    if it[2].parse::<i64>().is_err() {
                        warn(ui, "points missing: the game skips this line");
                    }
                });
                ui.end_row();
            }
        });
    if let Some((i, d)) = action {
        match d {
            0 => {
                items.remove(i);
            }
            2 => {
                let c = items[i].clone();
                items.insert(i + 1, c);
            }
            -1 if i > 0 => items.swap(i, i - 1),
            1 if i + 1 < items.len() => items.swap(i, i + 1),
            _ => {}
        }
        changed = true;
    }
    if ui.button("Add research item").clicked() {
        items.push(["MAGIC".into(), "POWER_HAND".into(), "1000".into()]);
        changed = true;
    }
    if changed {
        commit(
            cx,
            "research",
            items
                .into_iter()
                .map(|t| ("Research".to_string(), t.join(" ")))
                .collect(),
        );
    }
}

fn sacrifices(ui: &mut Ui, cx: &mut Cx) {
    let (lines, inherited) = block_header(ui, cx, "sacrifices");
    let mut items: Vec<(String, Vec<String>)> = lines
        .iter()
        .map(|(k, v)| (k.to_ascii_uppercase(), split_tokens(v)))
        .collect();
    let mut changed = false;
    let mut action: Option<(usize, i32)> = None;
    let creatures = cx.s.names("creature");
    let spells = cx.s.names("spell");
    // parse_rules_sacrifices_blocks, src/config_rules.c:527
    ui.label(weak(format!(
        "{} recipes: drop the victims into a temple to get the \
            reward or punishment",
        items.len()
    )));
    if items.len() > MAX_RECIPES {
        warn(
            ui,
            format!("more than {MAX_RECIPES} recipes: the game ignores the rest"),
        );
    }
    egui::Grid::new("sacrifices")
        .striped(true)
        .num_columns(5)
        .show(ui, |ui| {
            ui.label(weak("#"));
            ui.label(weak("effect"));
            ui.label(weak("reward / punishment"));
            ui.label(weak("victims"));
            ui.label("");
            ui.end_row();
            for (i, (cmd, toks)) in items.iter_mut().enumerate() {
                row_label(ui, cx, i, !same_line(inherited.get(i), cmd, toks));
                let mut c = cmd.clone();
                egui::ComboBox::from_id_salt(("scmd", i))
                    .selected_text(c.clone())
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for t in SAC_COMMANDS {
                            ui.selectable_value(&mut c, t.to_string(), t);
                        }
                    });
                if c != *cmd {
                    *cmd = c;
                    changed = true;
                }
                if toks.is_empty() {
                    toks.push(String::new());
                }
                let opts: Vec<(i64, String)> = match cmd.as_str() {
                    "MKCREATURE" | "MKGOODHERO" => creatures.clone(),
                    "NEGSPELLALL" | "POSSPELLALL" => spells.clone(),
                    _ => UNIQUE
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (i as i64 + 1, n.to_string()))
                        .collect(),
                };
                ui.vertical(|ui| {
                    changed |= picker(ui, ("sreward", i), &mut toks[0], &opts, 200.0, |_, _, _| {
                        None
                    });
                    if !SAC_COMMANDS.contains(&cmd.as_str()) {
                        warn(ui, "effect without handler: ignored");
                    } else if !known(&opts, &toks[0]) {
                        warn(ui, format!("unknown {}", toks[0]));
                    }
                });
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.set_max_width(560.0);
                        let mut rm = None;
                        for (j, v) in toks.iter_mut().enumerate().skip(1) {
                            changed |=
                                picker(ui, ("svictim", i, j), v, &creatures, 110.0, |_, _, _| None);
                            if ui
                                .small_button("-")
                                .on_hover_text("Remove this victim")
                                .clicked()
                            {
                                rm = Some(j);
                            }
                        }
                        if let Some(j) = rm {
                            toks.remove(j);
                            changed = true;
                        }
                        if toks.len() <= MAX_VICTIMS && ui.small_button("+ victim").clicked() {
                            toks.push("FLY".into());
                            changed = true;
                        }
                    });
                    let victims = &toks[1..];
                    if victims.is_empty() {
                        warn(ui, "no victims: the game ignores this recipe");
                    } else if victims.len() > MAX_VICTIMS {
                        warn(
                            ui,
                            format!(
                                // add_sacrifice_victim, src/config_rules.c:366
                                "more than {MAX_VICTIMS} victims: the game drops the rest"
                            ),
                        );
                    }
                    if let Some(v) = victims.iter().find(|v| !known(&creatures, v)) {
                        warn(ui, format!("unknown creature {v}"));
                    }
                });
                ui.horizontal(|ui| {
                    if ui.small_button("Up").clicked() {
                        action = Some((i, -1));
                    }
                    if ui.small_button("Down").clicked() {
                        action = Some((i, 1));
                    }
                    if ui.small_button("x").on_hover_text("Remove").clicked() {
                        action = Some((i, 0));
                    }
                });
                ui.end_row();
            }
        });
    if let Some((i, d)) = action {
        match d {
            0 => {
                items.remove(i);
            }
            -1 if i > 0 => items.swap(i, i - 1),
            1 if i + 1 < items.len() => items.swap(i, i + 1),
            _ => {}
        }
        changed = true;
    }
    if ui.button("Add recipe").clicked() {
        items.push(("MKCREATURE".into(), vec!["DRAGON".into(), "FLY".into()]));
        changed = true;
    }
    if changed {
        commit(
            cx,
            "sacrifices",
            items.into_iter().map(|(c, t)| (c, t.join(" "))).collect(),
        );
    }
}
