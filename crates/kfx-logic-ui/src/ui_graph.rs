//! Node graph view: events as nodes (trigger header, action chain), flags / timers as wires from
//! the events that set them to the events that test them, nested events and party spawns as
//! thin links. Pan (drag the background), zoom (wheel), drag nodes, auto-layout, double-click
//! opens the card. Rewiring: drag from a node's right port onto another event to chain them
//! with a new flag (the source sets it, the target gains `flag == 1`).
use crate::analysis::{action_text, condition_text, notes_of, var_usage, Access};
use crate::editor::{LogicEditor, LogicHost, Op, View};
use crate::model::*;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui};
use std::collections::{HashMap, HashSet};

const W: f32 = 240.0;
const COL_GAP: f32 = 90.0;
const ROW_GAP: f32 = 18.0;

#[derive(Clone)]
struct GNode {
    id: NodeId,
    title: String,
    conds: Vec<String>,
    actions: Vec<String>,
    more: usize,
    color: Color32,
    party: bool,
    enabled: bool,
}

impl GNode {
    fn height(&self) -> f32 {
        24.0 + self.conds.len() as f32 * 15.0
            + self.actions.len() as f32 * 14.0
            + if self.more > 0 { 14.0 } else { 0.0 }
            + 8.0
    }
}

#[derive(Clone)]
struct Wire {
    from: NodeId,
    to: NodeId,
    label: String,
    kind: u8, // 0 variable, 1 nested, 2 spawn
}

fn build(doc: &LogicDoc) -> (Vec<GNode>, Vec<Wire>) {
    let mut nodes = vec![];
    for p in doc.parties() {
        nodes.push(GNode {
            id: p.id,
            title: format!("PARTY {}", p.name),
            conds: vec![],
            actions: p
                .members
                .iter()
                .take(6)
                .map(|m| format!("{} L{} {}g", m.creature, m.level, m.gold))
                .collect(),
            more: p.members.len().saturating_sub(6),
            color: Color32::from_rgb(200, 170, 90),
            party: true,
            enabled: true,
        });
    }
    for (e, _) in doc.events() {
        let conds: Vec<String> = if e.conditions.is_empty() {
            vec!["Level start".into()]
        } else {
            e.conditions
                .iter()
                .map(|c| condition_text(doc, c))
                .collect()
        };
        let acts: Vec<String> = e
            .body
            .iter()
            .filter_map(|n| match n {
                Node::Action(a) => Some(action_text(doc, a)),
                Node::Event(_) => None,
                Node::Party(p) => Some(format!("party {}", p.name)),
            })
            .collect();
        let notes = notes_of(&e.pre);
        let title = notes
            .lines()
            .next()
            .map(|s| s.chars().take(40).collect::<String>())
            .unwrap_or_default();
        let color = if !e.enabled {
            Color32::GRAY
        } else if e.conditions.is_empty() {
            Color32::from_rgb(110, 140, 180)
        } else if crate::analysis::time_gate(doc, e).is_some() {
            Color32::from_rgb(220, 150, 60)
        } else if e.conditions[0].cmd.name == "IF_ACTION_POINT" {
            Color32::from_rgb(90, 190, 110)
        } else {
            Color32::from_rgb(200, 90, 90)
        };
        nodes.push(GNode {
            id: e.id,
            title,
            conds,
            more: acts.len().saturating_sub(6),
            actions: acts.into_iter().take(6).collect(),
            color,
            party: false,
            enabled: e.enabled,
        });
    }
    let mut wires = vec![];
    // variables: writer -> reader
    let uses = var_usage(doc);
    let mut seen = HashSet::new();
    for w in uses.iter().filter(|u| u.access == Access::Write) {
        for r in uses.iter().filter(|u| {
            u.access == Access::Read
                && u.var == w.var
                && (u.player == w.player || w.player == "ALL_PLAYERS" || u.player == "ALL_PLAYERS")
        }) {
            if r.event == w.event || !seen.insert((w.event, r.event, w.var.clone())) {
                continue;
            }
            let label = doc
                .alias_of(&w.player, &w.var)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| w.var.clone());
            wires.push(Wire {
                from: w.event,
                to: r.event,
                label,
                kind: 0,
            });
        }
    }
    // nesting
    for (e, _) in doc.events() {
        for n in &e.body {
            if let Node::Event(c) = n {
                wires.push(Wire {
                    from: e.id,
                    to: c.id,
                    label: String::new(),
                    kind: 1,
                });
            }
        }
    }
    // party spawns
    for p in doc.parties() {
        for (aid, _, _) in crate::waves::party_routes(doc, &p.name) {
            if let Some((Some(ev), _)) = doc.locate(aid) {
                if seen.insert((p.id, ev, String::from("party"))) {
                    wires.push(Wire {
                        from: p.id,
                        to: ev,
                        label: String::new(),
                        kind: 2,
                    });
                }
            }
        }
    }
    (nodes, wires)
}

/// Layered layout: columns by longest dependency path; unconnected events in a grid after them.
fn layout(nodes: &[GNode], wires: &[Wire]) -> HashMap<NodeId, Pos2> {
    let idx: HashMap<NodeId, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let connected: HashSet<NodeId> = wires.iter().flat_map(|w| [w.from, w.to]).collect();
    let mut layer: HashMap<NodeId, usize> = HashMap::new();
    for n in nodes.iter().filter(|n| connected.contains(&n.id)) {
        layer.insert(n.id, if n.party { 0 } else { 1 });
    }
    // relax longest paths; cycles are cut by the iteration bound
    for _ in 0..nodes.len().min(40) {
        let mut changed = false;
        for w in wires {
            let (Some(&a), Some(&b)) = (layer.get(&w.from), layer.get(&w.to)) else {
                continue;
            };
            if b < a + 1 && a + 1 < 30 {
                layer.insert(w.to, a + 1);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut pos = HashMap::new();
    let max_layer = layer.values().copied().max().unwrap_or(0);
    let mut bottom: f32 = 0.0;
    for l in 0..=max_layer {
        let mut y = 0.0;
        for n in nodes.iter().filter(|n| layer.get(&n.id) == Some(&l)) {
            pos.insert(n.id, pos2(l as f32 * (W + COL_GAP), y));
            y += n.height() + ROW_GAP;
        }
        bottom = bottom.max(y);
    }
    // the rest in a grid below
    let cols = 5usize;
    let mut col_y = vec![bottom + 60.0; cols];
    let mut k = 0usize;
    for n in nodes.iter().filter(|n| !connected.contains(&n.id)) {
        let c = k % cols;
        pos.insert(n.id, pos2(c as f32 * (W + 30.0), col_y[c]));
        col_y[c] += n.height() + ROW_GAP;
        k += 1;
    }
    let _ = idx;
    pos
}

pub(crate) fn show(ed: &mut LogicEditor, ui: &mut Ui, _host: &dyn LogicHost, ops: &mut Vec<Op>) {
    let (nodes, wires) = build(&ed.doc);
    let gen = ed.generation();
    let structural: u64 = nodes.iter().map(|n| n.id).sum::<u64>() ^ (wires.len() as u64) << 32;
    if ed.graph.laid_out_for != structural || ed.graph.pos.len() != nodes.len() {
        let fresh = layout(&nodes, &wires);
        for (k, v) in fresh {
            ed.graph.pos.entry(k).or_insert(v);
        }
        ed.graph.pos.retain(|k, _| nodes.iter().any(|n| n.id == *k));
        ed.graph.laid_out_for = structural;
    }
    let _ = gen;
    ui.horizontal(|ui| {
        if ui.button("Auto-layout").clicked() {
            ed.graph.pos = layout(&nodes, &wires);
        }
        if ui.button("Fit").clicked() {
            ed.graph.fit = true;
        }
        ui.add(egui::Slider::new(&mut ed.graph.zoom, 0.2..=2.0).text("zoom"));
        ui.label(
            egui::RichText::new(
                "Drag background: pan | wheel: zoom | double-click: open \
                    card | drag the right port onto an event: chain with a flag",
            )
            .small()
            .weak(),
        );
    });
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let area = resp.rect;
    painter.rect_filled(area, 0.0, ui.visuals().extreme_bg_color);
    if ed.graph.fit {
        ed.graph.fit = false;
        // fit
        let mut bb = Rect::NOTHING;
        for n in &nodes {
            if let Some(p) = ed.graph.pos.get(&n.id) {
                bb = bb.union(Rect::from_min_size(*p, vec2(W, n.height())));
            }
        }
        if bb.is_positive() {
            let z = (area.width() / (bb.width() + 40.0))
                .min(area.height() / (bb.height() + 40.0))
                .clamp(0.2, 1.5);
            ed.graph.zoom = z;
            ed.graph.pan = vec2(20.0, 20.0) / z - bb.min.to_vec2();
        } else {
            ed.graph.zoom = 1.0;
        }
    }
    if resp.dragged() && ed.graph.drag_wire.is_none() {
        ed.graph.pan += resp.drag_delta() / ed.graph.zoom;
    }
    if resp.hovered() {
        let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let old = ed.graph.zoom;
            let z = (old * (1.0 + scroll * 0.0015)).clamp(0.2, 2.0);
            if let Some(p) = ui.ctx().pointer_hover_pos() {
                // keep the world point under the pointer fixed
                let w = (p - area.min) / old - ed.graph.pan;
                ed.graph.pan = (p - area.min) / z - w;
            }
            ed.graph.zoom = z;
        }
    }
    let z = ed.graph.zoom;
    let pan = ed.graph.pan;
    let to_screen = |p: Pos2| area.min + (p.to_vec2() + pan) * z;
    let painter = painter.with_clip_rect(area);
    let rect_of = |id: NodeId, pos: &HashMap<NodeId, Pos2>| -> Option<Rect> {
        let n = nodes.iter().find(|n| n.id == id)?;
        let p = pos.get(&id)?;
        Some(Rect::from_min_max(
            to_screen(*p),
            to_screen(*p + vec2(W, n.height())),
        ))
    };
    // wires
    for w in &wires {
        let (Some(a), Some(b)) = (rect_of(w.from, &ed.graph.pos), rect_of(w.to, &ed.graph.pos))
        else {
            continue;
        };
        let p0 = pos2(a.right(), a.top() + 12.0 * z);
        let p3 = pos2(b.left(), b.top() + 12.0 * z);
        let dx = ((p3.x - p0.x).abs() * 0.5).max(40.0 * z);
        let (color, width) = match w.kind {
            0 => (Color32::from_rgb(240, 200, 80), 2.0),
            1 => (Color32::from_gray(150), 1.0),
            _ => (Color32::from_rgb(200, 170, 90).gamma_multiply(0.6), 1.0),
        };
        let pts = [p0, p0 + vec2(dx, 0.0), p3 - vec2(dx, 0.0), p3];
        painter.add(Shape::CubicBezier(
            egui::epaint::CubicBezierShape::from_points_stroke(
                pts,
                false,
                Color32::TRANSPARENT,
                Stroke::new(width * z.max(0.5), color),
            ),
        ));
        painter.add(Shape::convex_polygon(
            vec![
                p3,
                p3 - vec2(8.0 * z, 4.0 * z),
                p3 - vec2(8.0 * z, -4.0 * z),
            ],
            color,
            Stroke::NONE,
        ));
        if !w.label.is_empty() && z > 0.35 {
            let mid = pos2((p0.x + p3.x) / 2.0, (p0.y + p3.y) / 2.0);
            painter.text(
                mid,
                Align2::CENTER_BOTTOM,
                &w.label,
                FontId::proportional(11.0 * z),
                color,
            );
        }
    }
    // nodes
    let fs = 12.0 * z;
    let mut drag_target: Option<NodeId> = None;
    for n in &nodes {
        let Some(r) = rect_of(n.id, &ed.graph.pos) else {
            continue;
        };
        if !r.intersects(area) {
            continue;
        }
        let selected = ed.selected.and_then(|s| ed.enclosing_event(s)) == Some(n.id)
            || ed.selected == Some(n.id);
        let fill = ui.visuals().widgets.noninteractive.bg_fill;
        painter.rect_filled(r, 6.0 * z, fill);
        let header = Rect::from_min_size(r.min, vec2(r.width(), 20.0 * z));
        painter.rect_filled(
            header,
            egui::CornerRadius {
                nw: (6.0 * z) as u8,
                ne: (6.0 * z) as u8,
                sw: 0,
                se: 0,
            },
            n.color.gamma_multiply(if n.enabled { 0.85 } else { 0.4 }),
        );
        painter.rect_stroke(
            r,
            6.0 * z,
            Stroke::new(
                if selected { 2.5 } else { 1.0 },
                if selected {
                    ui.visuals().selection.stroke.color
                } else {
                    n.color
                },
            ),
            egui::StrokeKind::Inside,
        );
        let text_color = ui.visuals().text_color();
        let head = if n.title.is_empty() {
            if n.party {
                n.title.clone()
            } else {
                format!("#{}", n.id)
            }
        } else {
            n.title.clone()
        };
        painter.text(
            header.left_center() + vec2(6.0 * z, 0.0),
            Align2::LEFT_CENTER,
            clip(&head, 34),
            FontId::proportional(fs),
            Color32::BLACK,
        );
        let mut y = header.bottom() + 3.0 * z;
        for c in &n.conds {
            painter.text(
                pos2(r.left() + 6.0 * z, y),
                Align2::LEFT_TOP,
                clip(c, 38),
                FontId::proportional(fs * 0.95),
                n.color,
            );
            y += 15.0 * z;
        }
        for a in &n.actions {
            painter.text(
                pos2(r.left() + 12.0 * z, y),
                Align2::LEFT_TOP,
                format!("> {}", clip(a, 36)),
                FontId::proportional(fs * 0.9),
                text_color,
            );
            y += 14.0 * z;
        }
        if n.more > 0 {
            painter.text(
                pos2(r.left() + 12.0 * z, y),
                Align2::LEFT_TOP,
                format!("… {} more", n.more),
                FontId::proportional(fs * 0.9),
                text_color.gamma_multiply(0.6),
            );
        }
        // interaction: body
        let id = egui::Id::unique(("graph-node", n.id));
        let body = ui.interact(r, id, Sense::click_and_drag());
        if body.dragged() && ed.graph.drag_wire.is_none() {
            if let Some(p) = ed.graph.pos.get_mut(&n.id) {
                *p += body.drag_delta() / z;
            }
        }
        if body.clicked() {
            ops.push(Op::Select(n.id));
        }
        if body.double_clicked() {
            if n.party {
                ops.push(Op::ShowParty(n.id));
            } else {
                ed.view = View::Cards;
                ed.select(n.id);
            }
        }
        if body.hovered() {
            drag_target = Some(n.id);
        }
        // output port
        if !n.party {
            let port = Rect::from_center_size(
                pos2(r.right(), r.top() + 10.0 * z),
                vec2(12.0 * z, 12.0 * z),
            );
            painter.circle_filled(port.center(), 5.0 * z, Color32::from_rgb(240, 200, 80));
            let pr = ui
                .interact(port, egui::Id::unique(("graph-port", n.id)), Sense::drag())
                .on_hover_text("Drag onto another event to chain them with a flag");
            if pr.drag_started() {
                ed.graph.drag_wire = Some(n.id);
            }
        }
    }
    if let Some(from) = ed.graph.drag_wire {
        if let (Some(a), Some(p)) = (
            rect_of(from, &ed.graph.pos),
            ui.ctx().pointer_interact_pos(),
        ) {
            painter.line_segment(
                [pos2(a.right(), a.top() + 10.0 * z), p],
                Stroke::new(2.0, Color32::from_rgb(240, 200, 80)),
            );
        }
        if ui.ctx().input(|i| i.pointer.any_released()) {
            if let Some(to) =
                drag_target.filter(|t| *t != from && !nodes.iter().any(|n| n.id == *t && n.party))
            {
                ops.push(Op::Connect { from, to });
            }
            ed.graph.drag_wire = None;
        }
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    } else {
        s.to_string()
    }
}
