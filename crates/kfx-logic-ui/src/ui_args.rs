//! Typed argument controls (kinds from `data/script_arg_overlay.json`).
use crate::editor::{LogicHost, Op, PickKind, PickRequest};
use crate::model::*;
use crate::syntax::{arg_kind, arg_name, arity, num, quote, required};
use egui::{Id, RichText, Ui};
use kfx_script::consts::{Env, Extras};
use kfx_script::ArgKind;

pub(crate) struct ArgCtx<'a> {
    pub host: &'a dyn LogicHost,
    pub version: i64,
    pub aliases: &'a [VarAlias],
    pub parties: &'a [String],
    pub ops: &'a mut Vec<Op>,
}

impl ArgCtx<'_> {
    fn candidates(&self, kind: ArgKind, ext: bool) -> Vec<String> {
        let extras = Extras::default();
        let env = Env {
            provider: self.host.provider(),
            extras: &extras,
            version: self.version,
        };
        let mut v = env.candidates(kind, ext);
        match kind {
            ArgKind::Party => v = self.parties.to_vec(),
            ArgKind::Variable => {
                for p in [
                    "KEEPERS_DESTROYED[PLAYER0]",
                    "BATTLES_WON[PLAYER0]",
                    "BATTLES_LOST[PLAYER0]",
                    "BOX1_ACTIVATED",
                    "TRAP1_ACTIVATED",
                ] {
                    v.push(p.to_string());
                }
            }
            ArgKind::ApNumber => {
                v = self
                    .host
                    .provider()
                    .action_points()
                    .iter()
                    .map(|a| a.to_string())
                    .collect()
            }
            ArgKind::HeroGate => {
                v = self
                    .host
                    .provider()
                    .hero_gates()
                    .iter()
                    .map(|a| a.to_string())
                    .collect()
            }
            _ => {}
        }
        v
    }
}

/// A combo with a filter box; free text can be used too. Returns true when `value` changed.
pub(crate) fn pick_combo(
    ui: &mut Ui,
    id: Id,
    value: &mut String,
    cands: &[String],
    width: f32,
    describe: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let mut changed = false;
    let shown = match describe(value) {
        Some(d) => egui::RichText::new(format!("{value}  ({d})")),
        None if value.is_empty() => egui::RichText::new("choose…").weak(),
        None => egui::RichText::new(value.clone()),
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(width)
        .height(320.0)
        .show_ui(ui, |ui| {
            let fid = id.with("filter");
            let mut f: String = ui.data_mut(|d| d.get_temp::<String>(fid).unwrap_or_default());
            let r = ui.add(
                egui::TextEdit::singleline(&mut f)
                    .hint_text("filter / type a value")
                    .desired_width(width.max(160.0)),
            );
            if !r.has_focus() && f.is_empty() {
                r.request_focus();
            }
            ui.data_mut(|d| d.insert_temp(fid, f.clone()));
            let fu = f.to_ascii_uppercase();
            if !f.is_empty()
                && !cands.iter().any(|c| c.eq_ignore_ascii_case(&f))
                && ui.button(format!("Use \"{f}\"")).clicked()
            {
                *value = f.clone();
                changed = true;
            }
            for c in cands
                .iter()
                .filter(|c| fu.is_empty() || c.to_ascii_uppercase().contains(&fu))
                .take(300)
            {
                let label = match describe(c) {
                    Some(d) => format!("{c}  ({d})"),
                    None => c.clone(),
                };
                if ui
                    .selectable_label(c.eq_ignore_ascii_case(value), label)
                    .clicked()
                {
                    *value = c.clone();
                    changed = true;
                }
            }
        });
    changed
}

fn small_label(ui: &mut Ui, s: &str) {
    ui.label(RichText::new(s).small().weak());
}

fn number(ui: &mut Ui, value: &mut String, speed: f64, range: Option<(i64, i64)>) -> bool {
    match num(value) {
        Some(mut n) if !value.contains('x') && !value.contains('X') => {
            let mut dv = egui::DragValue::new(&mut n).speed(speed);
            if let Some((a, b)) = range {
                dv = dv.range(a..=b);
            }
            if ui.add(dv).changed() {
                *value = n.to_string();
                return true;
            }
            false
        }
        _ => ui
            .add(egui::TextEdit::singleline(value).desired_width(110.0))
            .changed(),
    }
}

/// Controls for one argument. Returns true when changed.
#[allow(clippy::too_many_arguments)]
fn arg(
    ui: &mut Ui,
    cx: &mut ArgCtx,
    node: NodeId,
    cmd_name: &str,
    slot: usize,
    value: &mut String,
    kind: ArgKind,
    ext: bool,
) -> bool {
    let id = Id::unique(("logic-arg", node, slot));
    let aliases = cx.aliases.to_vec();
    match kind {
        ArgKind::Number
        | ArgKind::Count
        | ArgKind::Amount
        | ArgKind::IfValue
        | ArgKind::Turns
        | ArgKind::BoxNumber => {
            let speed = if matches!(kind, ArgKind::Amount | ArgKind::Turns | ArgKind::IfValue) {
                10.0
            } else {
                0.2
            };
            number(ui, value, speed, None)
        }
        ArgKind::Level => number(ui, value, 0.1, Some((1, 10))),
        ArgKind::ApNumber | ArgKind::HeroGate => {
            let c = number(ui, value, 0.1, None);
            if ui
                .small_button("Map")
                .on_hover_text("Pick on the map")
                .clicked()
            {
                let k = if kind == ArgKind::ApNumber {
                    PickKind::ActionPoint
                } else {
                    PickKind::HeroGate
                };
                cx.ops.push(Op::Pick(PickRequest { node, slot }, k));
            }
            let _ = ext;
            c
        }
        ArgKind::SubtileX | ArgKind::SlabX => {
            let c = number(ui, value, 0.2, Some((0, 1000)));
            if ui
                .small_button("Map")
                .on_hover_text("Pick the position on the map")
                .clicked()
            {
                let k = if kind == ArgKind::SlabX {
                    PickKind::Slab
                } else {
                    PickKind::Subtile
                };
                cx.ops.push(Op::Pick(PickRequest { node, slot }, k));
            }
            c
        }
        ArgKind::SubtileY | ArgKind::SlabY => number(ui, value, 0.2, Some((0, 1000))),
        ArgKind::MessageId => {
            let c = number(ui, value, 0.2, Some((0, 2000)));
            if ui
                .small_button("Str")
                .on_hover_text("Pick a string")
                .clicked()
            {
                cx.ops.push(Op::PickString(PickRequest { node, slot }));
            }
            if let Some(t) = num(value).and_then(|n| cx.host.string_text(n)) {
                let short: String = t.chars().take(60).collect();
                ui.label(
                    RichText::new(format!(
                        "\"{short}{}\"",
                        if t.chars().count() > 60 { "…" } else { "" }
                    ))
                    .italics()
                    .weak(),
                )
                .on_hover_text(t);
            }
            c
        }
        ArgKind::Text | ArgKind::NewName | ArgKind::Lua
            if value.starts_with('"') || value.is_empty() =>
        {
            let mut inner = value.trim_matches('"').to_string();
            let r = ui.add(egui::TextEdit::singleline(&mut inner).desired_width(220.0));
            if r.changed() {
                *value = quote(&inner);
                return true;
            }
            false
        }
        ArgKind::Location => {
            let mut cands = cx.candidates(kind, ext);
            cands.retain(|c| num(c).is_some() || c.starts_with("PLAYER"));
            let c = pick_combo(ui, id, value, &cands, 90.0, &|v| {
                crate::analysis::parse_location(v).map(|l| l.label())
            });
            if ui
                .small_button("Map")
                .on_hover_text("Pick an action point, hero gate or heart on the map")
                .clicked()
            {
                cx.ops
                    .push(Op::Pick(PickRequest { node, slot }, PickKind::Location));
            }
            c
        }
        _ => {
            let cands = cx.candidates(kind, ext);
            let describe = |v: &str| -> Option<String> {
                if kind == ArgKind::Variable
                    || kind == ArgKind::FlagVar
                    || kind == ArgKind::Timer
                    || kind == ArgKind::CampaignFlag
                {
                    aliases
                        .iter()
                        .find(|a| a.var().eq_ignore_ascii_case(v))
                        .map(|a| a.name.clone())
                } else {
                    None
                }
            };
            if cands.is_empty() {
                ui.add(egui::TextEdit::singleline(value).desired_width(120.0))
                    .changed()
            } else {
                let w = if kind == ArgKind::Operator {
                    36.0
                } else {
                    120.0
                };
                let _ = cmd_name;
                pick_combo(ui, id, value, &cands, w, &describe)
            }
        }
    }
}

/// Controls for every argument of a command. Returns true when something changed.
pub(crate) fn cmd_args(
    ui: &mut Ui,
    cx: &mut ArgCtx,
    node: NodeId,
    cmd: &mut Cmd,
    labels: bool,
) -> bool {
    let mut changed = false;
    let n_decl = arity(&cmd.name, cx.version).max(cmd.args.len());
    let req = required(&cmd.name, cx.version);
    while cmd.args.len() < req {
        cmd.args.push(
            crate::editor::default_cmd(&cmd.name, cx.version)
                .args
                .get(cmd.args.len())
                .cloned()
                .unwrap_or_else(|| "0".into()),
        );
        changed = true;
    }
    let shown = cmd.args.len();
    for slot in 0..shown {
        let (kind, ext, _) = arg_kind(&cmd.name, slot, cx.version);
        if labels && kind != ArgKind::Operator {
            small_label(ui, &arg_name(&cmd.name, slot));
        }
        let mut v = cmd.args[slot].clone();
        if arg(ui, cx, node, &cmd.name, slot, &mut v, kind, ext) {
            cmd.args[slot] = v;
            changed = true;
        }
    }
    // optional arguments
    if shown < n_decl {
        let next = arg_name(&cmd.name, shown);
        if ui
            .small_button(format!("+{next}"))
            .on_hover_text("Add the optional argument")
            .clicked()
        {
            let (k, _, _) = arg_kind(&cmd.name, shown, cx.version);
            cmd.args.push(match k {
                ArgKind::Text => "\"\"".into(),
                ArgKind::Player => "PLAYER0".into(),
                _ => "0".into(),
            });
            changed = true;
        }
    }
    if shown > req
        && ui
            .small_button("-")
            .on_hover_text("Remove the last optional argument")
            .clicked()
    {
        cmd.args.pop();
        changed = true;
    }
    changed
}
