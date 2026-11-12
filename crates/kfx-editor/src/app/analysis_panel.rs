//! The Analysis panel (View > Analysis): `kfx_analysis` layers drawn as heat maps over the 3D
//! and 2D views with a legend, dig routes and first-contact points on the map, the fairness,
//! economy, route, symmetry and hero tables, click-to-locate, and the findings that go to the
//! Validate panel. The analysis runs on a background thread (see [`crate::analysis_view`]),
//! debounced after edits.
use super::*;
use crate::analysis_view::{self as av, AnalysisTab, DocKey, LayerKind, Worker};
use kfx_analysis::layer::{Layer, LayerData};
use kfx_analysis::nav::INF;
use kfx_analysis::reach::Endpoint;
use kfx_analysis::sites::owner_name;
use kfx_analysis::Analysis;
use kfx_map::transform::SymmetryMode;
#[cfg(test)]
use kfx_map::validate::Severity;
use kfx_prefab::i18n::{trd, trf, trn};

/// Debounce after the last edit before the map is analysed again.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// What the cached heat map was built for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct OverlayKey {
    result: DocKey,
    layer: LayerKind,
    player: u8,
    sym: Option<SymmetryMode>,
    opacity: u8,
    /// Document revision (slab heights).
    revision: u64,
}

/// Runtime state of the Analysis panel.
#[derive(Default)]
pub struct AnalysisState {
    worker: Worker,
    /// The newest analysis of the open map, with the key it was computed for and the map
    /// (folder, level) it belongs to.
    pub result: Option<Arc<Analysis>>,
    pub key: Option<DocKey>,
    map: Option<(PathBuf, u32)>,
    /// Wall time of the last analysis (ms) and how many ran.
    pub ms: f64,
    pub runs: u32,
    seen: Option<DocKey>,
    changed: Option<Instant>,
    findings: Vec<Issue>,
    layer: Option<(OverlayKey, Option<Arc<Layer>>)>,
    tris: Option<(OverlayKey, Arc<Vec<LineVertex>>)>,
    /// Highlighted route (index into `Analysis::routes`).
    pub route: Option<usize>,
    /// Symmetry mode of the Symmetry tab and layer (`None` = the best matching one).
    pub sym: Option<SymmetryMode>,
    /// The symmetry breaker clicked last.
    pub breaker: Option<usize>,
}

fn rgba(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// An owner's colour, lightened when too dark to read on the dark panels.
fn owner_col(o: u8) -> Color32 {
    let c = kfx_analysis::layer::owner_color(o);
    let lum = 0.3 * c[0] as f32 + 0.59 * c[1] as f32 + 0.11 * c[2] as f32;
    if lum < 110.0 {
        let m = |v: u8| (v as f32 + (255.0 - v as f32) * 0.45) as u8;
        Color32::from_rgb(m(c[0]), m(c[1]), m(c[2]))
    } else {
        rgba(c)
    }
}

impl EditorApp {
    // ------------------------------------------------------------------- worker / tick

    /// Per frame: take a finished analysis, start a new one after edits (debounced).
    pub(super) fn analysis_tick(&mut self) {
        let want = self.settings.layout.analysis.open || self.settings.layout.analysis.validate;
        let Some(d) = &self.doc else {
            self.analysis.result = None;
            self.analysis.key = None;
            self.analysis.findings.clear();
            return;
        };
        let current = DocKey::of(&d.doc, d.revision);
        let in_stroke = d.in_stroke();
        // another map was opened: drop the old map's result at once and analyse the new one
        let ident = (d.doc.folder.clone(), d.doc.level);
        if self.analysis.map.as_ref().is_some_and(|m| *m != ident)
            || self.analysis.result.as_ref().is_some_and(|a| {
                (a.terrain.w, a.terrain.h) != (d.doc.dims.w as i32, d.doc.dims.h as i32)
            })
        {
            self.analysis.result = None;
            self.analysis.key = None;
            self.analysis.map = None;
            self.analysis.findings.clear();
            self.analysis.layer = None;
            self.analysis.tris = None;
            self.analysis.route = None;
            self.analysis.breaker = None;
        }
        if let Some(done) = self.analysis.worker.poll() {
            if done.key == current {
                self.accept_analysis(done);
            }
        }
        if !want
            || self.analysis.key == Some(current)
            || self.analysis.worker.pending == Some(current)
            || in_stroke
        {
            if self.analysis.worker.pending.is_some() {
                self.ctx.request_repaint_after(Duration::from_millis(50));
            }
            return;
        }
        if self.analysis.seen != Some(current) {
            self.analysis.seen = Some(current);
            self.analysis.changed = Some(Instant::now());
        }
        let first = self.analysis.key.is_none();
        let waited = self
            .analysis
            .changed
            .is_some_and(|t| t.elapsed() >= DEBOUNCE);
        if first || waited || self.fixed_frame.is_some() {
            self.request_analysis();
        } else {
            self.ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn request_analysis(&mut self) {
        let Some(d) = &self.doc else { return };
        let key = DocKey::of(&d.doc, d.revision);
        self.analysis.worker.request(&d.doc, d.gd.clone(), key);
        self.ctx.request_repaint_after(Duration::from_millis(50));
    }

    fn accept_analysis(&mut self, done: av::Done) {
        let map = self
            .doc
            .as_ref()
            .map(|d| (d.doc.folder.clone(), d.doc.level));
        self.analysis.findings = av::findings(&done.analysis);
        self.analysis.result = Some(done.analysis);
        self.analysis.key = Some(done.key);
        self.analysis.map = map;
        self.analysis.ms = done.ms;
        self.analysis.runs += 1;
        self.analysis.layer = None;
        self.analysis.tris = None;
        if self.analysis.route.is_some_and(|r| {
            self.analysis
                .result
                .as_ref()
                .is_none_or(|a| r >= a.routes.len())
        }) {
            self.analysis.route = None;
        }
        self.merge_analysis_issues();
        self.ctx.request_repaint();
    }

    /// Analyse the open map now and wait for the result (headless runs and tests).
    pub fn analysis_wait(&mut self) -> Result<(), String> {
        let d = self.doc.as_ref().ok_or("no map open")?;
        let key = DocKey::of(&d.doc, d.revision);
        if self.analysis.key == Some(key) {
            return Ok(());
        }
        self.request_analysis();
        let done = self
            .analysis
            .worker
            .wait(key, Duration::from_secs(120))
            .ok_or("the analysis did not finish")?;
        self.accept_analysis(done);
        Ok(())
    }

    /// The analysis findings for the Validate panel (empty when switched off or when they
    /// belong to another map).
    pub(super) fn analysis_findings(&self) -> Vec<Issue> {
        let same = self.doc.as_ref().is_some_and(|d| {
            self.analysis
                .map
                .as_ref()
                .is_some_and(|(f, l)| *f == d.doc.folder && *l == d.doc.level)
        });
        if !self.settings.layout.analysis.validate || !same {
            return vec![];
        }
        self.analysis.findings.clone()
    }

    /// Replace the analysis findings in the Validate list.
    fn merge_analysis_issues(&mut self) {
        let issues = std::mem::take(&mut self.issues);
        let links = std::mem::take(&mut self.issue_links);
        let mut v: Vec<(Issue, Option<(String, kfx_script::Span)>)> = issues
            .into_iter()
            .enumerate()
            .filter(|(_, i)| !i.code.starts_with("analysis-"))
            .map(|(k, i)| (i, links.get(k).cloned().flatten()))
            .collect();
        v.extend(self.analysis_findings().into_iter().map(|i| (i, None)));
        v.sort_by(|a, b| b.0.severity.cmp(&a.0.severity));
        let (i, l): (Vec<_>, Vec<_>) = v.into_iter().unzip();
        self.issues = i;
        self.issue_links = l;
    }

    // ------------------------------------------------------------------- layer / overlay

    fn overlay_key(&self) -> Option<OverlayKey> {
        let s = &self.settings.layout.analysis;
        if !s.open || s.layer == LayerKind::Off {
            return None;
        }
        Some(OverlayKey {
            result: self.analysis.key?,
            layer: s.layer,
            player: s.player,
            sym: self.analysis.sym,
            opacity: s.opacity,
            revision: self.doc.as_ref()?.revision,
        })
    }

    /// The shown layer (cached).
    pub fn analysis_layer(&mut self) -> Option<Arc<Layer>> {
        let k = self.overlay_key()?;
        let same = |a: &OverlayKey| {
            a.result == k.result && a.layer == k.layer && a.player == k.player && a.sym == k.sym
        };
        if let Some((ck, l)) = &self.analysis.layer {
            if same(ck) {
                return l.clone();
            }
        }
        let a = self.analysis.result.clone()?;
        let l = av::build_layer(&a, k.layer, k.player, k.sym).map(Arc::new);
        self.analysis.layer = Some((k, l.clone()));
        l
    }

    /// The heat map as translucent slab-top quads (one per run of equal colour and height).
    pub(super) fn analysis_tris(&mut self, tris: &mut Vec<LineVertex>) {
        let Some(k) = self.overlay_key() else { return };
        if let Some((ck, v)) = &self.analysis.tris {
            if *ck == k {
                tris.extend_from_slice(v);
                return;
            }
        }
        let Some(layer) = self.analysis_layer() else {
            self.analysis.tris = Some((k, Arc::new(vec![])));
            return;
        };
        let t = &self.terrain;
        let (w, h) = ((t.stl_w / 3) as i32, (t.stl_h / 3) as i32);
        let mut out = vec![];
        let lv = |p: Vec3, c: [u8; 4]| LineVertex {
            pos: p.to_array(),
            color: c,
        };
        for y in 0..h.min(layer.h) {
            let mut x = 0;
            while x < w.min(layer.w) {
                let i = (y * layer.w + x) as usize;
                let Some(mut c) = kfx_analysis::layer::layer_color(&layer, i) else {
                    x += 1;
                    continue;
                };
                c[3] = k.opacity;
                let top = t.slab_top(x as usize, y as usize);
                let mut x1 = x + 1;
                while x1 < w.min(layer.w)
                    && kfx_analysis::layer::layer_color(&layer, (y * layer.w + x1) as usize)
                        .is_some_and(|d| d[..3] == c[..3])
                    && t.slab_top(x1 as usize, y as usize) == top
                {
                    x1 += 1;
                }
                let hh = top as f32 + 0.07;
                let (x0, z0, xe, ze) = (
                    x as f32 * 3.0,
                    y as f32 * 3.0,
                    x1 as f32 * 3.0,
                    (y + 1) as f32 * 3.0,
                );
                let q = [
                    Vec3::new(x0, hh, z0),
                    Vec3::new(x0, hh, ze),
                    Vec3::new(xe, hh, ze),
                    Vec3::new(xe, hh, z0),
                ];
                for n in [0, 1, 2, 0, 2, 3] {
                    out.push(lv(q[n], c));
                }
                x = x1;
            }
        }
        tris.extend_from_slice(&out);
        self.analysis.tris = Some((k, Arc::new(out)));
    }

    /// Number of heat map quads currently drawn (tests).
    pub fn analysis_overlay_quads(&self) -> usize {
        match (&self.analysis.tris, self.overlay_key()) {
            (Some((ck, v)), Some(k)) if *ck == k => v.len() / 6,
            _ => 0,
        }
    }

    /// Routes, contact points, the legend over a view pane.
    pub(super) fn draw_analysis(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        vp: Vec2,
        ppp: f32,
    ) {
        if !self.settings.layout.analysis.open || self.doc.is_none() {
            return;
        }
        let Some(a) = self.analysis.result.clone() else {
            return;
        };
        let to_ui = |p: Vec2| rect.min + EVec2::new(p.x / ppp, p.y / ppp);
        let t = &self.terrain;
        let (sw, sh) = (t.stl_w / 3, t.stl_h / 3);
        let at = |x: i32, y: i32, lift: f32| {
            let top = if (x as usize) < sw && (y as usize) < sh && x >= 0 && y >= 0 {
                t.slab_top(x as usize, y as usize) as f32
            } else {
                0.0
            };
            Vec3::new(x as f32 * 3.0 + 1.5, top + lift, y as f32 * 3.0 + 1.5)
        };
        if self.settings.layout.analysis.routes {
            for (k, r) in a.routes.iter().enumerate() {
                let sel = self.analysis.route == Some(k);
                if self.analysis.route.is_some() && !sel {
                    continue;
                }
                let pts: Vec<egui::Pos2> = r
                    .slabs
                    .iter()
                    .filter_map(|&(x, y)| self.camera.project(at(x, y, 0.4), vp))
                    .map(to_ui)
                    .collect();
                if pts.len() < 2 {
                    continue;
                }
                let col = match r.from {
                    Endpoint::Gate { .. } | Endpoint::HeroGroup { .. } => {
                        Color32::from_rgb(250, 250, 250)
                    }
                    Endpoint::Heart { owner } if sel => owner_col(owner),
                    _ => Color32::from_rgb(255, 70, 210),
                };
                let w = if sel { 4.0 } else { 2.5 };
                painter.add(egui::Shape::line(
                    pts.clone(),
                    egui::Stroke::new(w + 2.0, Color32::from_black_alpha(170)),
                ));
                painter.add(egui::Shape::line(pts.clone(), egui::Stroke::new(w, col)));
                if sel {
                    if let (Some(s), Some(e)) = (pts.first(), pts.last()) {
                        painter.circle_filled(*s, 5.0, col);
                        painter.circle_stroke(*e, 6.0, egui::Stroke::new(2.0, col));
                    }
                }
            }
        }
        if self.settings.layout.analysis.contacts {
            for c in &a.contacts {
                if let Some(p) = self.camera.project(at(c.slab.0, c.slab.1, 0.4), vp) {
                    let p = to_ui(p);
                    painter.circle_filled(p, 6.0, Color32::from_black_alpha(160));
                    painter.circle_stroke(
                        p,
                        6.0,
                        egui::Stroke::new(2.0, Color32::from_rgb(255, 0, 255)),
                    );
                    painter.line_segment(
                        [p - EVec2::new(4.0, 4.0), p + EVec2::new(4.0, 4.0)],
                        egui::Stroke::new(1.5, Color32::WHITE),
                    );
                    painter.line_segment(
                        [p - EVec2::new(4.0, -4.0), p + EVec2::new(4.0, -4.0)],
                        egui::Stroke::new(1.5, Color32::WHITE),
                    );
                }
            }
        }
        if self.settings.layout.analysis.legend {
            if let Some(l) = self.analysis_layer() {
                self.draw_legend(painter, rect, &l, &a);
            }
        }
    }

    fn draw_legend(&self, painter: &egui::Painter, rect: egui::Rect, l: &Layer, a: &Analysis) {
        let font = egui::FontId::proportional(12.0);
        let title = painter.layout_no_wrap(l.name.clone(), egui::FontId::proportional(13.0), GOLD);
        let rows: Vec<(Color32, String)> = match &l.data {
            LayerData::Categorical { legend, values } => legend
                .iter()
                .filter(|e| values.contains(&e.value))
                .map(|e| (rgba(e.color), e.label.clone()))
                .collect(),
            LayerData::Scalar { .. } => vec![],
        };
        let row_h = 16.0;
        let body_h = match &l.data {
            LayerData::Scalar { .. } => 34.0,
            LayerData::Categorical { .. } => rows.len().max(1) as f32 * row_h,
        };
        let width = rows
            .iter()
            .map(|r| {
                painter
                    .layout_no_wrap(r.1.clone(), font.clone(), Color32::WHITE)
                    .size()
                    .x
                    + 30.0
            })
            .fold(title.size().x, f32::max)
            .max(180.0)
            + 16.0;
        let height = title.size().y + body_h + 16.0;
        // bottom-left, above the grid data line
        let r = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + 10.0, rect.max.y - 44.0 - height),
            EVec2::new(width, height),
        );
        painter.rect(
            r,
            4.0,
            Color32::from_black_alpha(200),
            egui::Stroke::new(1.0, GOLD_DIM),
            egui::StrokeKind::Inside,
        );
        let mut y = r.min.y + 6.0;
        painter.galley(egui::pos2(r.min.x + 8.0, y), title, GOLD);
        y += 18.0;
        match &l.data {
            LayerData::Scalar { min, max, unit, .. } => {
                let bar = egui::Rect::from_min_size(
                    egui::pos2(r.min.x + 8.0, y + 2.0),
                    EVec2::new(width - 16.0, 10.0),
                );
                let n = 32;
                for k in 0..n {
                    let c = kfx_analysis::layer::ramp(k as f32 / (n - 1) as f32);
                    let x0 = bar.min.x + bar.width() * k as f32 / n as f32;
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(x0, bar.min.y),
                            egui::pos2(x0 + bar.width() / n as f32 + 0.5, bar.max.y),
                        ),
                        0.0,
                        rgba(c),
                    );
                }
                painter.text(
                    egui::pos2(bar.min.x, bar.max.y + 2.0),
                    egui::Align2::LEFT_TOP,
                    format!("{min:.0} {unit}"),
                    font.clone(),
                    theme::TEXT,
                );
                painter.text(
                    egui::pos2(bar.max.x, bar.max.y + 2.0),
                    egui::Align2::RIGHT_TOP,
                    format!("{max:.0} {unit}"),
                    font.clone(),
                    theme::TEXT,
                );
                painter.text(
                    egui::pos2(bar.center().x, bar.max.y + 2.0),
                    egui::Align2::CENTER_TOP,
                    trd("kfx_editor.unreached_clear", "unreached: clear"),
                    font,
                    TEXT_DIM,
                );
            }
            LayerData::Categorical { .. } => {
                if rows.is_empty() {
                    painter.text(
                        egui::pos2(r.min.x + 8.0, y),
                        egui::Align2::LEFT_TOP,
                        trd("kfx_editor.nothing_to_show", "nothing to show"),
                        font.clone(),
                        TEXT_DIM,
                    );
                }
                for (c, label) in rows {
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(r.min.x + 8.0, y + 2.0),
                            EVec2::new(16.0, 11.0),
                        ),
                        2.0,
                        c,
                    );
                    painter.text(
                        egui::pos2(r.min.x + 30.0, y),
                        egui::Align2::LEFT_TOP,
                        label,
                        font.clone(),
                        theme::TEXT,
                    );
                    y += row_h;
                }
            }
        }
        let _ = a;
    }

    // ------------------------------------------------------------------- locate

    /// Centre both views on slab (x, y), select it and flash it.
    pub fn analysis_locate(&mut self, x: i32, y: i32) {
        let top = self.terrain.slab_top(x.max(0) as usize, y.max(0) as usize) as f32;
        let p = Vec3::new(x as f32 * 3.0 + 1.5, top, y as f32 * 3.0 + 1.5);
        self.camera.target = Vec3::new(p.x, self.camera.target.y, p.z);
        self.camera2.target = Vec3::new(p.x, self.camera2.target.y, p.z);
        self.selection = Some(((x, y), (x, y)));
        self.flash = Some((p, Instant::now()));
    }

    /// The symmetry breakers of the shown mode.
    pub fn analysis_breakers(&self) -> Vec<av::Breaker> {
        let Some(a) = &self.analysis.result else {
            return vec![];
        };
        let Some(m) = self.analysis.sym.or_else(|| av::best_symmetry(a)) else {
            return vec![];
        };
        av::breakers(a, m)
    }

    /// Click symmetry breaker `n` (0-based): locate it, select its thing.
    pub fn click_breaker(&mut self, n: usize) -> Result<av::Breaker, String> {
        let list = self.analysis_breakers();
        let b = list
            .get(n)
            .cloned()
            .ok_or_else(|| format!("no symmetry breaker {n} ({} listed)", list.len()))?;
        self.analysis.breaker = Some(n);
        self.analysis_locate(b.slab.0, b.slab.1);
        if let Some(t) = b.thing {
            self.selected = BTreeSet::from([ItemRef::Thing(t)]);
        }
        self.status = b.text.clone();
        Ok(b)
    }

    /// Highlight route `n` (`None`: all routes) and centre on its start.
    pub fn show_route(&mut self, n: Option<usize>) -> Result<(), String> {
        self.settings.layout.analysis.routes = true;
        let Some(n) = n else {
            self.analysis.route = None;
            return Ok(());
        };
        let a = self.analysis.result.clone().ok_or("no analysis yet")?;
        let r = a
            .routes
            .get(n)
            .ok_or_else(|| format!("no route {n} ({} routes)", a.routes.len()))?;
        self.analysis.route = Some(n);
        let mid = r.slabs.get(r.slabs.len() / 2).copied().unwrap_or((0, 0));
        self.camera.target = Vec3::new(
            mid.0 as f32 * 3.0 + 1.5,
            self.camera.target.y,
            mid.1 as f32 * 3.0 + 1.5,
        );
        self.camera2.target = Vec3::new(
            mid.0 as f32 * 3.0 + 1.5,
            self.camera2.target.y,
            mid.1 as f32 * 3.0 + 1.5,
        );
        self.status = av::route_name(&a, r);
        Ok(())
    }

    pub fn set_analysis_open(&mut self, open: bool) {
        self.settings.layout.analysis.open = open;
        if open {
            self.ctx.request_repaint();
        }
    }

    // ------------------------------------------------------------------- shortcuts / menu

    /// The panel's shortcut (called with the global shortcuts).
    pub(super) fn analysis_shortcuts(&mut self, ctx: &egui::Context) {
        let k = self.keymap.clone();
        if k.pressed(ctx, "view.analysis") {
            let o = !self.settings.layout.analysis.open;
            self.set_analysis_open(o);
        }
        // (a map shortcut: not while a text field has the keys)
        if !ctx.egui_wants_keyboard_input()
            && self.doc.is_some()
            && k.pressed(ctx, "map.generate_selection")
        {
            self.open_gen_patch();
        }
    }

    // ------------------------------------------------------------------- window

    pub(super) fn analysis_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings.layout.analysis.open;
        if !open {
            return;
        }
        egui::Window::new(trd("kfx_editor.analysis", "Analysis"))
            .id(egui::Id::unique("analysis-window"))
            .open(&mut open)
            .default_size([460.0, 560.0])
            .default_pos(ctx.content_rect().right_top() + EVec2::new(-500.0, 90.0))
            .resizable(true)
            .show(ctx, |ui| self.analysis_ui(ui));
        self.settings.layout.analysis.open = open;
    }

    fn analysis_ui(&mut self, ui: &mut egui::Ui) {
        if self.doc.is_none() {
            ui.label(
                RichText::new(trd(
                    "kfx_editor.open_a_map_to_analyse_it",
                    "Open a map to analyse it.",
                ))
                .color(TEXT_DIM),
            );
            return;
        }
        let a = self.analysis.result.clone();
        ui.horizontal(|ui| {
            let busy = self.analysis.worker.pending.is_some();
            match &a {
                Some(a) => {
                    let stale = self.doc.as_ref().is_some_and(|d| {
                        self.analysis.key.is_some_and(|k| k.revision != d.revision)
                    });
                    let txt = trf(
                        "kfx_editor.analysed_in_ms_ms_total_us_ms_in_the_analysis_st",
                        "Analysed in {ms} ms ({total_us} ms in the analysis){stale}",
                        &[
                            ("ms", &format!("{:.0}", self.analysis.ms)),
                            (
                                "total_us",
                                &format!("{:.0}", a.timings.total_us as f64 / 1000.0),
                            ),
                            (
                                "stale",
                                &(if stale {
                                    trd(
                                        "kfx_editor.updating_after_the_edit",
                                        ", updating after the edit",
                                    )
                                } else {
                                    String::new()
                                }),
                            ),
                        ],
                    );
                    ui.label(RichText::new(txt).small().color(TEXT_DIM));
                }
                None => {
                    ui.label(
                        RichText::new(trd("kfx_editor.analysing_the_map", "Analysing the map…"))
                            .color(TEXT_DIM),
                    );
                }
            }
            if busy {
                ui.spinner();
            }
            if ui
                .small_button(trd("kfx_editor.recompute", "Recompute"))
                .on_hover_text(trd(
                    "kfx_editor.analyse_the_map_again_now",
                    "Analyse the map again now.",
                ))
                .clicked()
            {
                self.analysis.key = None;
                self.request_analysis();
            }
        });
        ui.horizontal_wrapped(|ui| {
            for t in AnalysisTab::ALL {
                if ui
                    .add(egui::Button::selectable(
                        self.settings.layout.analysis.tab == t,
                        t.title(),
                    ))
                    .clicked()
                {
                    self.settings.layout.analysis.tab = t;
                }
            }
        });
        ui.separator();
        let Some(a) = a else { return };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match self.settings.layout.analysis.tab {
                AnalysisTab::Layers => self.analysis_layers_ui(ui, &a),
                AnalysisTab::Fairness => fairness_ui(ui, &a),
                AnalysisTab::Economy => self.economy_ui(ui, &a),
                AnalysisTab::Routes => self.routes_ui(ui, &a),
                AnalysisTab::Symmetry => self.analysis_symmetry_ui(ui, &a),
                AnalysisTab::Heroes => self.heroes_ui(ui, &a),
            });
    }

    fn analysis_layers_ui(&mut self, ui: &mut egui::Ui, a: &Arc<Analysis>) {
        let s = &mut self.settings.layout.analysis;
        ui.label(RichText::new(trd("kfx_editor.heat_map", "Heat map")).color(GOLD));
        for k in LayerKind::ALL {
            ui.radio_value(&mut s.layer, k, k.label())
                .on_hover_text(k.tooltip());
        }
        if s.layer.per_keeper() {
            ui.horizontal(|ui| {
                ui.label(trd("kfx_editor.keeper", "Keeper"));
                let cur = av::keeper_or_first(a, s.player);
                for f in &a.fields {
                    let sel = cur == Some(f.player);
                    let col = owner_col(f.player);
                    // the chosen keeper: dark text on its colour (its colour on the selection
                    // highlight is hard to read)
                    let b = if sel {
                        egui::Button::new(
                            RichText::new(owner_name(f.player))
                                .color(Color32::BLACK)
                                .strong(),
                        )
                        .fill(col)
                    } else {
                        egui::Button::new(RichText::new(owner_name(f.player)).color(col).strong())
                            .frame(false)
                    };
                    if ui.add(b).clicked() {
                        s.player = f.player;
                    }
                }
                if a.fields.is_empty() {
                    ui.label(
                        RichText::new(trd(
                            "kfx_editor.this_map_has_no_keeper_with_a_heart",
                            "this map has no keeper with a heart",
                        ))
                        .color(TEXT_DIM),
                    );
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label(trd("kfx_editor.opacity", "Opacity"));
            ui.add(egui::Slider::new(&mut s.opacity, 20..=240));
        });
        ui.checkbox(
            &mut s.routes,
            trd("kfx_editor.dig_routes_on_the_map", "Dig routes on the map"),
        )
        .on_hover_text(trd(
            "kfx_editor.shortest_dig_routes_heart_to_heart_and_from_each",
            "Shortest dig routes heart to heart, and from each hero gate to the nearest heart.",
        ));
        ui.checkbox(
            &mut s.contacts,
            trd("kfx_editor.first_contact_points", "First-contact points"),
        )
        .on_hover_text(trd(
            "kfx_editor.where_two_keepers_digging_towards_each_other_mee",
            "Where two keepers digging towards each other meet.",
        ));
        ui.checkbox(
            &mut s.legend,
            trd("kfx_editor.legend_in_the_view", "Legend in the view"),
        );
        ui.checkbox(
            &mut s.validate,
            trd(
                "kfx_editor.report_problems_in_the_validate_panel",
                "Report problems in the Validate panel",
            ),
        )
        .on_hover_text(trd(
            "kfx_editor.unreachable_portals_keepers_without_gold_isolate",
            "Unreachable portals, keepers without gold, isolated hero gates and \
            closed pockets with things.",
        ));
        if s.layer != LayerKind::Off && self.analysis_layer().is_none() {
            ui.colored_label(
                Color32::from_rgb(240, 180, 80),
                trd(
                    "kfx_editor.this_layer_has_nothing_to_show_on_this_map",
                    "This layer has nothing to show on this map.",
                ),
            );
        }
        ui.separator();
        ui.label(
            RichText::new(trf(
                "kfx_editor.fields_keeper_s_with_a_heart_gates_hero_gate_s_p",
                "{fields} keeper(s) with a heart, {gates} hero gate(s), {portals} \
                portal(s), {pockets} closed pocket(s)",
                &[
                    ("fields", &a.fields.len()),
                    ("gates", &a.sites.gates.len()),
                    ("portals", &a.sites.portals.len()),
                    ("pockets", &a.pockets.len()),
                ],
            ))
            .color(TEXT_DIM),
        );
        let mut go = None;
        for (k, p) in a.pockets.iter().enumerate().take(50) {
            let c = kfx_analysis::sites::group_centre(&p.cells);
            let txt = trn(
                "kfx_editor.pocket_k_cells_slabs_around_c_c2_things_thing_s",
                p.things.len() as i64,
                "Pocket {k}: {cells} slabs around ({c},{c2}), {n} thing",
                "Pocket {k}: {cells} slabs around ({c},{c2}), {n} things",
                &[
                    ("k", &(k + 1)),
                    ("cells", &p.cells.len()),
                    ("c", &c.0),
                    ("c2", &c.1),
                ],
            );
            if ui
                .add(egui::Button::selectable(false, txt))
                .on_hover_text(trd("kfx_editor.click_to_locate", "Click to locate"))
                .clicked()
            {
                go = Some(
                    p.cells
                        .iter()
                        .copied()
                        .min_by_key(|q| (q.0 - c.0).pow(2) + (q.1 - c.1).pow(2))
                        .unwrap_or(c),
                );
            }
        }
        if let Some((x, y)) = go {
            self.analysis_locate(x, y);
        }
        let t = a.timings;
        ui.label(
            RichText::new(trf(
                "kfx_editor.timings_classify_classify_us_ms_fields_and_route",
                "Timings: classify {classify_us} ms, fields and routes {fields_us} \
                ms, economy {economy_us} ms, heroes and fairness {heroes_us} ms, symmetry \
                {symmetry_us} ms.",
                &[
                    (
                        "classify_us",
                        &format!("{:.1}", t.classify_us as f64 / 1000.0),
                    ),
                    ("fields_us", &format!("{:.1}", t.fields_us as f64 / 1000.0)),
                    (
                        "economy_us",
                        &format!("{:.1}", t.economy_us as f64 / 1000.0),
                    ),
                    ("heroes_us", &format!("{:.1}", t.heroes_us as f64 / 1000.0)),
                    (
                        "symmetry_us",
                        &format!("{:.1}", t.symmetry_us as f64 / 1000.0),
                    ),
                ],
            ))
            .small()
            .color(TEXT_DIM),
        );
    }

    fn economy_ui(&mut self, ui: &mut egui::Ui, a: &Arc<Analysis>) {
        let e = &a.economy;
        ui.label(trf(
            "kfx_editor.gold_slabs_total_gold_slabs_total_gold_value_gol",
            "Gold slabs: {total_gold_slabs} ({total_gold_value} gold), \
            {unreachable_gold_slabs} that no keeper reaches. Gem slabs: {total_gems}.",
            &[
                ("total_gold_slabs", &e.total_gold_slabs),
                ("total_gold_value", &e.total_gold_value),
                ("unreachable_gold_slabs", &e.unreachable_gold_slabs),
                ("total_gems", &e.total_gems),
            ],
        ));
        if e.players.is_empty() {
            ui.label(
                RichText::new(trd(
                    "kfx_editor.no_keeper_with_a_heart_nothing_to_share_out",
                    "No keeper with a heart: nothing to share out.",
                ))
                .color(TEXT_DIM),
            );
        } else {
            egui::Grid::new("analysis-economy")
                .striped(true)
                .num_columns(9)
                .show(ui, |ui| {
                    for h in [
                        trd("kfx_editor.keeper", "Keeper"),
                        trd("kfx_editor.gold_slabs", "Gold slabs"),
                        trd("kfx_editor.gold", "Gold"),
                        trd("kfx_editor.contested", "Contested"),
                        trd("kfx_editor.reachable", "Reachable"),
                        trd("kfx_editor.gems_9c3d", "Gems"),
                        trd("kfx_editor.loose_gold", "Loose gold"),
                        trd("kfx_editor.claimable", "Claimable"),
                        trd("kfx_editor.portal", "Portal"),
                    ] {
                        ui.label(RichText::new(h).color(GOLD));
                    }
                    ui.end_row();
                    for p in &e.players {
                        ui.label(RichText::new(owner_name(p.player)).color(owner_col(p.player)));
                        ui.label(p.gold_slabs.to_string());
                        ui.label(p.gold_value.to_string());
                        ui.label(p.contested_gold_slabs.to_string());
                        ui.label(p.reachable_gold_slabs.to_string());
                        ui.label(if p.gems > 0 {
                            trf(
                                "kfx_editor.gems_gem_gold_per_hit_hit",
                                "{gems} ({gem_gold_per_hit}/hit)",
                                &[("gems", &p.gems), ("gem_gold_per_hit", &p.gem_gold_per_hit)],
                            )
                        } else {
                            "0".into()
                        });
                        ui.label(p.loose_gold.to_string());
                        ui.label(p.claimable.to_string());
                        ui.label(
                            p.portals
                                .first()
                                .map(|d| av::seconds(a, d.turns))
                                .unwrap_or_else(|| trd("kfx_editor.none_557b", "none").into()),
                        );
                        ui.end_row();
                    }
                });
        }
        if !e.rooms.is_empty() {
            ui.separator();
            ui.label(
                RichText::new(trd("kfx_editor.pre_built_rooms", "Pre-built rooms")).color(GOLD),
            );
            let mut go = None;
            for r in &e.rooms {
                let cap = if r.capacity > 0 {
                    trf(
                        "kfx_editor.capacity_capacity",
                        ", capacity {capacity}",
                        &[("capacity", &r.capacity)],
                    )
                } else {
                    String::new()
                };
                let txt = trf(
                    "kfx_editor.name_owner_slabs_slabs_at_centre_centre2_cap",
                    "{name} ({owner}): {slabs} slabs at ({centre},{centre2}){cap}",
                    &[
                        ("name", &r.name),
                        ("owner", &owner_name(r.owner)),
                        ("slabs", &r.slabs),
                        ("centre", &r.centre.0),
                        ("centre2", &r.centre.1),
                        ("cap", &cap),
                    ],
                );
                if ui
                    .add(egui::Button::selectable(false, txt))
                    .on_hover_text(trd("kfx_editor.click_to_locate", "Click to locate"))
                    .clicked()
                {
                    go = Some(r.centre);
                }
            }
            if let Some((x, y)) = go {
                self.analysis_locate(x, y);
            }
        }
    }

    fn routes_ui(&mut self, ui: &mut egui::Ui, a: &Arc<Analysis>) {
        ui.checkbox(
            &mut self.settings.layout.analysis.routes,
            trd(
                "kfx_editor.draw_routes_on_the_map",
                "Draw routes on the map",
            ),
        );
        if a.routes.is_empty() {
            ui.label(
                RichText::new(trd(
                    "kfx_editor.no_routes_the_map_needs_two_keepers_or_a_keeper",
                    "No routes: the map needs two keepers, or a keeper and a hero gate.",
                ))
                .color(TEXT_DIM),
            );
        }
        let mut pick: Option<Option<usize>> = None;
        if ui
            .add(egui::Button::selectable(
                self.analysis.route.is_none(),
                trd("kfx_editor.all_routes", "All routes"),
            ))
            .clicked()
        {
            pick = Some(None);
        }
        for (k, r) in a.routes.iter().enumerate() {
            let txt = format!("{}. {}", k + 1, av::route_name(a, r));
            let r = ui
                .add(egui::Button::selectable(
                    self.analysis.route == Some(k),
                    txt,
                ))
                .on_hover_text(trf(
                    "kfx_editor.slabs_slabs_digs_passes_earth_passes2_gold_passe",
                    "{slabs} slabs; digs {passes} earth, {passes2} gold; passes {doors} door(s)",
                    &[
                        ("slabs", &r.slabs.len()),
                        (
                            "passes",
                            &r.passes.count(kfx_analysis::terrain::Class::Earth),
                        ),
                        (
                            "passes2",
                            &r.passes.count(kfx_analysis::terrain::Class::Gold),
                        ),
                        ("doors", &r.passes.doors.len()),
                    ],
                ));
            if r.clicked() {
                pick = Some(Some(k));
            }
        }
        if let Some(p) = pick {
            let _ = self.show_route(p);
        }
        if !a.contacts.is_empty() {
            ui.separator();
            ui.label(RichText::new(trd("kfx_editor.first_contact", "First contact")).color(GOLD));
            let mut go = None;
            for c in &a.contacts {
                let txt = trf(
                    "kfx_editor.a_and_b_meet_at_slab_slab2_after_turns",
                    "{a} and {b} meet at ({slab},{slab2}) after {turns}",
                    &[
                        ("a", &owner_name(c.a)),
                        ("b", &owner_name(c.b)),
                        ("slab", &c.slab.0),
                        ("slab2", &c.slab.1),
                        ("turns", &av::seconds(a, c.turns)),
                    ],
                );
                if ui
                    .add(egui::Button::selectable(false, txt))
                    .on_hover_text(trd("kfx_editor.click_to_locate", "Click to locate"))
                    .clicked()
                {
                    go = Some(c.slab);
                }
            }
            if let Some((x, y)) = go {
                self.analysis_locate(x, y);
            }
        }
    }

    fn analysis_symmetry_ui(&mut self, ui: &mut egui::Ui, a: &Arc<Analysis>) {
        if a.symmetry.is_empty() {
            ui.label(
                RichText::new(trd("kfx_editor.no_symmetry_check", "No symmetry check."))
                    .color(TEXT_DIM),
            );
            return;
        }
        let best = av::best_symmetry(a);
        let shown = self.analysis.sym.or(best);
        for c in &a.symmetry {
            let mark = if c.exact() {
                trd("kfx_editor.exact", "exact")
            } else if c.terrain_exact() {
                trd(
                    "kfx_editor.terrain_exact_things_differ",
                    "terrain exact, things differ",
                )
            } else {
                String::new()
            };
            // (two decimals: one differing slab of a large map is 99.98 %, not 100.0 %)
            let txt = trf(
                "kfx_editor.mode_match_fraction_of_slabs_match_mark_mark2",
                "{mode}: {match_fraction}% of slabs match{mark}{mark2}",
                &[
                    ("mode", &av::mode_name(c.mode)),
                    (
                        "match_fraction",
                        &format!("{:.2}", c.match_fraction * 100.0),
                    ),
                    ("mark", &(if mark.is_empty() { "" } else { ", " })),
                    ("mark2", &mark),
                ],
            );
            if ui
                .add(egui::Button::selectable(shown == Some(c.mode), txt))
                .clicked()
            {
                self.analysis.sym = Some(c.mode);
                self.analysis.breaker = None;
            }
        }
        if !a.symmetric_modes().is_empty() {
            ui.label(
                RichText::new(trd(
                    "kfx_editor.the_map_is_exactly_symmetric_for_the_modes_marke",
                    "The map is exactly symmetric for the modes marked exact.",
                ))
                .color(TEXT_DIM),
            );
        }
        ui.separator();
        let list = self.analysis_breakers();
        ui.label(
            RichText::new(trf(
                "kfx_editor.what_breaks_it_list_click_to_locate",
                "What breaks it ({list}): click to locate",
                &[("list", &list.len())],
            ))
            .color(GOLD),
        );
        let mut go = None;
        for (n, b) in list.iter().enumerate().take(300) {
            let txt = format!("{}. {}", n + 1, b.text);
            if ui
                .add(egui::Button::selectable(
                    self.analysis.breaker == Some(n),
                    txt,
                ))
                .clicked()
            {
                go = Some(n);
            }
        }
        if list.len() > 300 {
            ui.label(
                RichText::new(trf(
                    "kfx_editor.and_list_more",
                    "… and {list} more",
                    &[("list", &(list.len() - 300))],
                ))
                .color(TEXT_DIM),
            );
        }
        if let Some(n) = go {
            let _ = self.click_breaker(n);
        }
    }

    fn heroes_ui(&mut self, ui: &mut egui::Ui, a: &Arc<Analysis>) {
        if a.hero_pressure.is_empty() {
            ui.label(
                RichText::new(trd(
                    "kfx_editor.no_hero_gates_and_no_pre_placed_heroes",
                    "No hero gates and no pre-placed heroes.",
                ))
                .color(TEXT_DIM),
            );
            return;
        }
        let mut go = None;
        for p in &a.hero_pressure {
            let src = av::endpoint_name(&p.source);
            let target = match p.target {
                Some(t) => trf(
                    "kfx_editor.reaches_t_first_after_turns",
                    "reaches {t} first after {turns}",
                    &[("t", &owner_name(t)), ("turns", &av::seconds(a, p.turns))],
                ),
                None => trd("kfx_editor.reaches_no_keeper", "reaches no keeper").into(),
            };
            let how = if p.walk.is_some() {
                trd("kfx_editor.walking", "walking")
            } else if p.tunnel.is_some() {
                trd("kfx_editor.with_a_tunneller_909f", "with a tunneller")
            } else {
                String::new()
            };
            let all: Vec<String> = p
                .per_keeper
                .iter()
                .map(|(o, t)| format!("{} {}", owner_name(*o), av::seconds(a, *t)))
                .collect();
            let txt = format!(
                "{src}: {target}{}{}",
                if how.is_empty() {
                    String::new()
                } else {
                    format!(" ({how})")
                },
                if all.len() > 1 {
                    format!("; {}", all.join(", "))
                } else {
                    String::new()
                }
            );
            let col = if p.target.is_none() {
                Color32::from_rgb(240, 180, 80)
            } else {
                theme::TEXT
            };
            if ui
                .add(egui::Button::selectable(
                    false,
                    RichText::new(txt).color(col),
                ))
                .on_hover_text(trd("kfx_editor.click_to_locate", "Click to locate"))
                .clicked()
            {
                go = match p.source {
                    Endpoint::Gate { number } => a
                        .sites
                        .gates
                        .iter()
                        .find(|g| g.number == number)
                        .map(|g| g.slab),
                    Endpoint::HeroGroup { index } => a
                        .hero_groups
                        .get(index)
                        .and_then(|g| g.cells.first().copied()),
                    Endpoint::Slab(x, y) => Some((x, y)),
                    Endpoint::Heart { .. } => None,
                };
            }
        }
        if !a.hero_groups.is_empty() {
            ui.separator();
            ui.label(
                RichText::new(trd(
                    "kfx_editor.pre_placed_hero_groups",
                    "Pre-placed hero groups",
                ))
                .color(GOLD),
            );
            for (k, g) in a.hero_groups.iter().enumerate() {
                ui.label(trn(
                    "kfx_editor.group_k_members_hero_es_levels_levels_strength_s",
                    g.members.len() as i64,
                    "Group {k}: {n} hero, levels {levels}, strength {score}{tunneller}",
                    "Group {k}: {n} heroes, levels {levels}, strength {score}{tunneller}",
                    &[
                        ("k", &(k + 1)),
                        ("levels", &g.levels),
                        ("score", &g.score),
                        (
                            "tunneller",
                            &(if g.tunneller {
                                trd("kfx_editor.with_a_tunneller", ", with a tunneller")
                            } else {
                                String::new()
                            }),
                        ),
                    ],
                ));
            }
        }
        if let Some((x, y)) = go {
            self.analysis_locate(x, y);
        }
    }

    // ------------------------------------------------------------------- headless

    /// `--actions` of the Analysis panel and the generator (see docs/manual/cli.md).
    pub fn run_analysis_actions(
        &mut self,
        obj: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        let get = |k: &str| obj.get(k);
        if let Some(v) = get("analysis") {
            let open = v.as_bool().unwrap_or(true);
            self.set_analysis_open(open);
            if open {
                self.analysis_wait()?;
            }
        }
        if let Some(v) = get("analysis_tab").and_then(|v| v.as_str()) {
            self.settings.layout.analysis.tab =
                AnalysisTab::parse(v).ok_or_else(|| format!("unknown analysis tab {v:?}"))?;
        }
        if let Some(v) = get("analysis_layer").and_then(|v| v.as_str()) {
            self.settings.layout.analysis.layer = LayerKind::parse(v).ok_or_else(|| {
                format!(
                    "unknown analysis layer {v:?} (off, reach, dig, walk, territory, \
                    heroes, gold, class, symmetry)"
                )
            })?;
            self.settings.layout.analysis.open = true;
        }
        if let Some(v) = get("analysis_player").and_then(|v| v.as_u64()) {
            self.settings.layout.analysis.player = v as u8;
        }
        if let Some(v) = get("analysis_opacity").and_then(|v| v.as_u64()) {
            self.settings.layout.analysis.opacity = v.min(255) as u8;
        }
        if let Some(v) = get("analysis_symmetry").and_then(|v| v.as_str()) {
            self.analysis.sym = if v == "auto" {
                None
            } else {
                Some(
                    crate::newmap::parse_symmetry(v)
                        .ok_or_else(|| format!("unknown symmetry {v:?}"))?,
                )
            };
        }
        if get("analysis_wait").is_some() {
            self.analysis_wait()?;
        }
        if let Some(v) = get("analysis_route") {
            self.analysis_wait()?;
            let n = v.as_u64().map(|n| n as usize);
            self.show_route(n)?;
        }
        if let Some(v) = get("analysis_breaker").and_then(|v| v.as_u64()) {
            self.analysis_wait()?;
            let b = self.click_breaker(v as usize)?;
            eprintln!("symmetry breaker {v}: {} at {:?}", b.text, b.slab);
        }
        if get("analysis_print").is_some() {
            self.analysis_wait()?;
            let a = self.analysis.result.clone().ok_or("no analysis")?;
            eprintln!(
                "analysis: {:.1} ms wall, {:.1} ms analysis; {} keepers, {} gates, {} \
                routes, {} pockets, fairness {:?}, symmetric {:?}, layer quads {}",
                self.analysis.ms,
                a.timings.total_us as f64 / 1000.0,
                a.fields.len(),
                a.sites.gates.len(),
                a.routes.len(),
                a.pockets.len(),
                a.fairness.as_ref().map(|f| f.score),
                a.symmetric_modes(),
                self.analysis_overlay_quads()
            );
            for (k, r) in a.routes.iter().enumerate() {
                eprintln!("  route {k}: {}", av::route_name(&a, r));
            }
            for i in &self.analysis.findings {
                eprintln!("  finding [{:?}] {}: {}", i.severity, i.code, i.message);
            }
        }
        if let Some(n) = get("analysis_bench").and_then(|v| v.as_u64()) {
            // the full path of one recompute: snapshot (clone) + worker + accept, n times
            let mut t = vec![];
            for _ in 0..n.max(1) {
                let t0 = Instant::now();
                self.analysis.key = None;
                self.analysis_wait()?;
                t.push(t0.elapsed().as_secs_f64() * 1000.0);
            }
            t.sort_by(|a, b| a.total_cmp(b));
            let dims = self
                .doc
                .as_ref()
                .map(|d| d.dims())
                .map(|d| (d.w, d.h))
                .unwrap_or((0, 0));
            eprintln!(
                "bench analysis {}x{}: n {} median {:.1} ms, max {:.1} ms (worker {:.1} ms)",
                dims.0,
                dims.1,
                t.len(),
                t[t.len() / 2],
                t[t.len() - 1],
                self.analysis.ms
            );
        }
        self.run_generator_actions(obj)
    }

    /// Analysis findings currently in the Validate list (tests).
    #[cfg(test)]
    pub fn analysis_issue_codes(&self) -> Vec<(&'static str, Severity)> {
        self.issues
            .iter()
            .filter(|i| i.code.starts_with("analysis-"))
            .map(|i| (i.code, i.severity))
            .collect()
    }
}

fn fairness_ui(ui: &mut egui::Ui, a: &Arc<Analysis>) {
    let Some(f) = &a.fairness else {
        ui.label(
            RichText::new(trd(
                "kfx_editor.fairness_compares_keepers_this_map_has_fewer_tha",
                "Fairness compares keepers: this map has fewer than two keepers with a heart.",
            ))
            .color(TEXT_DIM),
        );
        return;
    };
    let col = if f.score < 0.1 {
        Color32::from_rgb(120, 220, 120)
    } else if f.score < 0.25 {
        Color32::from_rgb(240, 200, 90)
    } else {
        Color32::from_rgb(240, 110, 90)
    };
    ui.label(
        RichText::new(trf(
            "kfx_editor.imbalance_score_0_is_perfectly_even",
            "Imbalance {score} (0 is perfectly even)",
            &[("score", &format!("{:.2}", f.score))],
        ))
        .color(col)
        .strong(),
    );
    let secs = |t: u32| {
        if t == INF {
            "never".to_string()
        } else {
            format!("{:.0}", a.rules.seconds(t))
        }
    };
    egui::Grid::new("analysis-fairness")
        .striped(true)
        .num_columns(10)
        .show(ui, |ui| {
            for h in [
                trd("kfx_editor.keeper", "Keeper"),
                trd("kfx_editor.gold", "Gold"),
                trd("kfx_editor.area", "Area"),
                trd("kfx_editor.gems_9c3d", "Gems"),
                trd("kfx_editor.portals", "Portals"),
                trd("kfx_editor.to_portal_seconds", "To portal (s)"),
                trd("kfx_editor.to_enemy_seconds", "To enemy (s)"),
                trd("kfx_editor.to_centre_seconds", "To centre (s)"),
                trd("kfx_editor.creatures_f8b1", "Creatures"),
                trd("kfx_editor.hero_threat", "Hero threat"),
            ] {
                ui.label(RichText::new(h).color(GOLD));
            }
            ui.end_row();
            for r in &f.rows {
                ui.label(RichText::new(owner_name(r.player)).color(owner_col(r.player)));
                ui.label(r.gold.to_string());
                ui.label(r.area.to_string());
                ui.label(r.gems.to_string());
                ui.label(r.portals.to_string());
                ui.label(secs(r.nearest_portal));
                ui.label(secs(r.nearest_enemy));
                ui.label(secs(r.centre));
                ui.label(trf(
                    "kfx_editor.creatures_lv_creature_levels",
                    "{creatures} (lv {creature_levels})",
                    &[
                        ("creatures", &r.creatures),
                        ("creature_levels", &r.creature_levels),
                    ],
                ));
                ui.label(format!("{} ({})", r.hero_sources, r.hero_score));
                ui.end_row();
            }
        });
    ui.separator();
    ui.label(
        RichText::new(trd("kfx_editor.largest_differences", "Largest differences")).color(GOLD),
    );
    let top = f.top(6);
    if top.is_empty() {
        ui.label(
            RichText::new(trd(
                "kfx_editor.none_every_keeper_has_the_same",
                "None: every keeper has the same.",
            ))
            .color(TEXT_DIM),
        );
    }
    for t in top {
        ui.label(t);
    }
}
