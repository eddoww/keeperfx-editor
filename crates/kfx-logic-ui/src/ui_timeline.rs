//! Timeline view: events gated by `GAME_TURN` or by timers on a time axis (game turns, 20 per
//! second at normal speed). Timer-gated events hang off the event that starts their timer.
//! Drag a marker sideways to retime it (rewrites the compared value).
use crate::analysis::{
    condition_text, earliest_turns, notes_of, time_gate, var_usage, Access, TimeGate,
};
use crate::editor::{LogicEditor, LogicHost, Op, View};
use crate::model::*;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Ui};

const LABEL_W: f32 = 300.0;
const ROW_H: f32 = 26.0;
pub const TURNS_PER_SECOND: f64 = 20.0;

struct Row {
    id: NodeId,
    label: String,
    turn: Option<i64>,
    gate: Option<TimeGate>,
    /// Earliest turn of the event starting the timer, when known.
    anchor: Option<i64>,
    relative: bool,
    /// "t0" row of an event with no known time that starts timers.
    header: bool,
    color: Color32,
}

fn fmt_time(turn: i64) -> String {
    let s = (turn as f64 / TURNS_PER_SECOND) as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub(crate) fn show(ed: &mut LogicEditor, ui: &mut Ui, _host: &dyn LogicHost, ops: &mut Vec<Op>) {
    let doc = &ed.doc;
    let known = earliest_turns(doc);
    // timer setters
    let setters: Vec<NodeId> = var_usage(doc)
        .into_iter()
        .filter(|u| u.access == Access::Write && u.command == "SET_TIMER")
        .map(|u| u.event)
        .collect();
    let mut rows: Vec<Row> = vec![];
    for (e, _) in doc.events() {
        let gate = time_gate(doc, e);
        if gate.is_none() && !setters.contains(&e.id) {
            continue;
        }
        let notes = notes_of(&e.pre);
        let summary = if e.conditions.is_empty() {
            "Level start".to_string()
        } else {
            e.conditions
                .iter()
                .map(|c| condition_text(doc, c))
                .collect::<Vec<_>>()
                .join(" AND ")
        };
        let label = match notes.lines().next() {
            Some(t) if !t.is_empty() => format!("{t} | {summary}"),
            _ => summary,
        };
        let anchor = match &gate {
            Some(TimeGate::Timer { started_by, .. }) => started_by
                .iter()
                .filter_map(|s| known.get(s))
                .min()
                .copied(),
            _ => None,
        };
        let relative = matches!(gate, Some(TimeGate::Timer { .. })) && anchor.is_none();
        let color = match gate {
            Some(TimeGate::GameTurn { .. }) => Color32::from_rgb(220, 150, 60),
            Some(TimeGate::Timer { .. }) => Color32::from_rgb(90, 170, 220),
            None => Color32::from_rgb(140, 140, 160),
        };
        // a GAME_TURN gate combined with other conditions: shown at its turn (the earliest it can
        // fire)
        let turn = known.get(&e.id).copied().or(match &gate {
            Some(TimeGate::GameTurn { turn, .. }) => Some(*turn),
            Some(TimeGate::Timer { turns, .. }) => anchor.map(|a| a + turns),
            None => None,
        });
        rows.push(Row {
            id: e.id,
            label,
            turn,
            gate,
            anchor,
            relative,
            header: false,
            color,
        });
    }
    // absolute rows by time, then each untimed timer starter followed by the events it times
    let mut ordered: Vec<Row> = vec![];
    let mut abs: Vec<&Row> = rows
        .iter()
        .filter(|r| !r.relative && r.turn.is_some())
        .collect();
    abs.sort_by_key(|r| r.turn);
    for r in abs {
        ordered.push(Row {
            label: r.label.clone(),
            gate: r.gate.clone(),
            ..*r
        });
    }
    let mut placed: Vec<NodeId> = vec![];
    for r in rows
        .iter()
        .filter(|r| r.turn.is_none() && setters.contains(&r.id))
    {
        let deps: Vec<&Row> = rows
            .iter()
            .filter(|d| d.relative && !placed.contains(&d.id) && matches!(&d.gate, Some(TimeGate::Timer { started_by, .. }) if started_by.contains(&r.id)))
            .collect();
        if deps.is_empty() {
            continue;
        }
        ordered.push(Row {
            label: r.label.clone(),
            gate: None,
            header: true,
            relative: true,
            ..*r
        });
        for d in deps {
            placed.push(d.id);
            ordered.push(Row {
                label: format!("   {}", d.label),
                gate: d.gate.clone(),
                ..*d
            });
        }
    }
    for d in rows
        .iter()
        .filter(|d| d.relative && !placed.contains(&d.id))
    {
        ordered.push(Row {
            label: format!("{} (timer never started)", d.label),
            gate: d.gate.clone(),
            ..*d
        });
    }
    let rows = ordered;
    let max_turn = rows
        .iter()
        .filter_map(|r| {
            r.turn.or(match &r.gate {
                Some(TimeGate::Timer { turns, .. }) => Some(*turns),
                _ => None,
            })
        })
        .max()
        .unwrap_or(1000)
        .max(1000);
    ui.horizontal(|ui| {
        ui.label("Scale");
        ui.add(
            egui::Slider::new(&mut ed.timeline.px_per_turn, 0.002..=0.5)
                .logarithmic(true)
                .text("px / turn"),
        );
        if ui.button("Fit").clicked() {
            ed.timeline.px_per_turn = ((ui.available_width() + 200.0 - LABEL_W).max(200.0)
                / (max_turn as f32 * 1.1))
                .clamp(0.002, 0.5);
        }
        ui.label(
            RichText::new(format!(
                "{} timed events; drag a marker to retime; double-click opens the card",
                rows.len()
            ))
            .small()
            .weak(),
        );
    });
    if rows.is_empty() {
        ui.label("No event of this script is gated by GAME_TURN or a timer.");
        return;
    }
    let ppt = ed.timeline.px_per_turn;
    let width = LABEL_W + max_turn as f32 * 1.1 * ppt + 80.0;
    let height = 30.0 + rows.len() as f32 * ROW_H + 30.0;
    egui::ScrollArea::both()
        .id_salt("logic-timeline")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(
                vec2(width.max(ui.available_width()), height),
                Sense::hover(),
            );
            let o = resp.rect.min;
            let x_of = |t: i64| o.x + LABEL_W + t as f32 * ppt;
            // axis
            let step = nice_step((90.0 / ppt) as i64);
            let mut t = 0;
            let vis = ui.visuals().clone();
            while t <= (max_turn as f64 * 1.1) as i64 {
                let x = x_of(t);
                painter.line_segment(
                    [pos2(x, o.y + 22.0), pos2(x, o.y + height)],
                    Stroke::new(
                        1.0,
                        vis.widgets
                            .noninteractive
                            .bg_stroke
                            .color
                            .gamma_multiply(0.5),
                    ),
                );
                painter.text(
                    pos2(x, o.y + 2.0),
                    Align2::CENTER_TOP,
                    format!("{t}"),
                    FontId::proportional(10.0),
                    vis.weak_text_color(),
                );
                painter.text(
                    pos2(x, o.y + 12.0),
                    Align2::CENTER_TOP,
                    fmt_time(t),
                    FontId::proportional(9.0),
                    vis.weak_text_color().gamma_multiply(0.7),
                );
                t += step;
            }
            let mut y = o.y + 30.0;
            let mut relative_header = false;
            for r in &rows {
                if r.relative && !relative_header {
                    relative_header = true;
                    painter.text(
                        pos2(o.x + 4.0, y + 4.0),
                        Align2::LEFT_TOP,
                        "Timer chains started by events with no known time (x = \
                            delay after the start):",
                        FontId::proportional(11.0),
                        vis.warn_fg_color,
                    );
                    y += ROW_H;
                }
                let row_rect = Rect::from_min_size(pos2(o.x, y), vec2(width, ROW_H));
                if r.header {
                    let c = pos2(x_of(0), y + ROW_H / 2.0);
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            c + vec2(0.0, -7.0),
                            c + vec2(7.0, 0.0),
                            c + vec2(0.0, 7.0),
                            c + vec2(-7.0, 0.0),
                        ],
                        r.color,
                        Stroke::NONE,
                    ));
                    painter.text(
                        pos2(o.x + 4.0, y + ROW_H / 2.0),
                        Align2::LEFT_CENTER,
                        clip(&format!("t0 = {}", r.label), 44),
                        FontId::proportional(12.0),
                        vis.strong_text_color(),
                    );
                    painter.text(
                        c + vec2(10.0, 0.0),
                        Align2::LEFT_CENTER,
                        "fires (time not known), starts its timers",
                        FontId::proportional(10.0),
                        vis.weak_text_color(),
                    );
                    let lr = ui.interact(
                        Rect::from_min_size(pos2(o.x, y), vec2(LABEL_W, ROW_H)),
                        egui::Id::unique(("tl-row", r.id)),
                        Sense::click(),
                    );
                    if lr.clicked() {
                        ops.push(Op::Select(r.id));
                    }
                    if lr.double_clicked() {
                        ui.data_mut(|d| d.insert_temp(egui::Id::unique("tl-open"), r.id));
                    }
                    y += ROW_H;
                    continue;
                }
                if ed.selected == Some(r.id) {
                    painter.rect_filled(row_rect, 0.0, vis.selection.bg_fill.gamma_multiply(0.3));
                }
                painter.text(
                    pos2(o.x + 4.0, y + ROW_H / 2.0),
                    Align2::LEFT_CENTER,
                    clip(&r.label, 44),
                    FontId::proportional(12.0),
                    vis.text_color(),
                );
                let (pos_turn, base) = match (&r.gate, r.turn) {
                    (Some(TimeGate::Timer { turns, .. }), _) if r.relative => {
                        (Some(*turns), Some(0))
                    }
                    (Some(TimeGate::Timer { .. }), Some(t)) => (Some(t), r.anchor),
                    (_, Some(t)) => (Some(t), None),
                    _ => (None, None),
                };
                if let Some(pt) = pos_turn {
                    if let Some(b) = base {
                        // timer bracket from its start
                        let yb = y + ROW_H / 2.0;
                        painter.line_segment(
                            [pos2(x_of(b), yb), pos2(x_of(pt), yb)],
                            Stroke::new(2.0, r.color.gamma_multiply(0.6)),
                        );
                        painter.circle_filled(pos2(x_of(b), yb), 3.0, r.color);
                    }
                    let id = egui::Id::unique(("tl-marker", r.id));
                    let mut mx = x_of(pt);
                    let drag_dx: f32 = ui.data(|d| d.get_temp(id).unwrap_or(0.0));
                    mx += drag_dx;
                    let mrect =
                        Rect::from_center_size(pos2(mx, y + ROW_H / 2.0), vec2(14.0, ROW_H - 8.0));
                    let m = ui
                        .interact(mrect, id, Sense::click_and_drag())
                        .on_hover_text(match &r.gate {
                            Some(TimeGate::GameTurn { turn, .. }) => {
                                format!("GAME_TURN {turn} ({})", fmt_time(*turn))
                            }
                            Some(TimeGate::Timer {
                                timer,
                                turns,
                                player,
                                ..
                            }) => format!("{player} {timer} + {turns} turns"),
                            None => "starts a timer".into(),
                        });
                    let fill = if m.dragged() || m.hovered() {
                        r.color
                    } else {
                        r.color.gamma_multiply(0.85)
                    };
                    if r.relative {
                        painter.rect_stroke(
                            mrect,
                            3.0,
                            Stroke::new(2.0, fill),
                            egui::StrokeKind::Inside,
                        );
                    } else {
                        painter.rect_filled(mrect, 3.0, fill);
                    }
                    let label = match &r.gate {
                        Some(TimeGate::GameTurn { turn, .. }) => {
                            format!("{}", turn + ((drag_dx / ppt) as i64))
                        }
                        Some(TimeGate::Timer { timer, turns, .. }) => {
                            format!("{timer} +{}", turns + ((drag_dx / ppt) as i64))
                        }
                        None => "timer start".into(),
                    };
                    painter.text(
                        pos2(mrect.right() + 4.0, y + ROW_H / 2.0),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(11.0),
                        vis.text_color(),
                    );
                    if m.dragged() && r.gate.is_some() {
                        ui.data_mut(|d| d.insert_temp(id, drag_dx + m.drag_delta().x));
                    }
                    if m.drag_stopped() {
                        ui.data_mut(|d| d.remove::<f32>(id));
                        let delta = (drag_dx / ppt / 10.0).round() as i64 * 10;
                        if delta != 0 {
                            if let Some(g) = &r.gate {
                                let (cond, cur) = match g {
                                    TimeGate::GameTurn { cond, turn } => (*cond, *turn),
                                    TimeGate::Timer { cond, turns, .. } => (*cond, *turns),
                                };
                                // the stored value: `>` compares one less
                                let strict = doc
                                    .events()
                                    .into_iter()
                                    .flat_map(|(e, _)| e.conditions.iter())
                                    .find(|c| c.id == cond)
                                    .map(|c| c.cmd.arg(2) == ">")
                                    .unwrap_or(false);
                                let v = (cur + delta).max(0) - strict as i64;
                                ops.push(Op::Retime { cond, value: v });
                            }
                        }
                    }
                    if m.clicked() {
                        ops.push(Op::Select(r.id));
                    }
                    if m.double_clicked() {
                        ops.push(Op::Select(r.id));
                        ui.data_mut(|d| d.insert_temp(egui::Id::unique("tl-open"), r.id));
                    }
                }
                let lr = ui.interact(
                    Rect::from_min_size(pos2(o.x, y), vec2(LABEL_W, ROW_H)),
                    egui::Id::unique(("tl-row", r.id)),
                    Sense::click(),
                );
                if lr.clicked() {
                    ops.push(Op::Select(r.id));
                }
                if lr.double_clicked() {
                    ui.data_mut(|d| d.insert_temp(egui::Id::unique("tl-open"), r.id));
                }
                y += ROW_H;
            }
        });
    if let Some(id) = ui.data_mut(|d| d.remove_temp::<NodeId>(egui::Id::unique("tl-open"))) {
        ed.view = View::Cards;
        ed.select(id);
    }
}

fn nice_step(raw: i64) -> i64 {
    let raw = raw.max(1);
    let mut s = 1;
    loop {
        for m in [1, 2, 5] {
            if s * m >= raw {
                return s * m;
            }
        }
        s *= 10;
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    } else {
        s.to_string()
    }
}
