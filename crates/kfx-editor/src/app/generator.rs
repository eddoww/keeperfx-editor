//! The constrained map generator in the editor: the New map dialog's generator options, its
//! preview generated off the UI thread (top-down render of `kfx_analysis` with the constraints
//! report), and Map > Generate into selection (`generate_patch`, one undo step).
use super::*;
use crate::newmap::{self, NewMapSpec};
use kfx_analysis::generate::{self, GenParams, GenReport, Generated, SymmetryChoice, ALL_STYLES};
use kfx_prefab::i18n::{trd, trf, trn};
use std::sync::mpsc::{channel, Receiver};

/// Debounce of the New map preview after the options change.
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(250);

/// A finished preview: the map, its rendered overview, or why generation failed.
pub struct GenResult {
    /// The parameters it was generated with (folder / level ignored).
    pub params: GenParams,
    pub gd: Arc<GameData>,
    pub result: Result<Arc<Generated>, String>,
    pub image: Option<egui::ColorImage>,
}

/// The New map dialog's generator preview.
#[derive(Default)]
pub struct GenPreview {
    rx: Option<Receiver<(u64, GenResult)>>,
    serial: u64,
    /// Parameters of the newest request, when it was asked.
    asked: Option<(GenParams, Instant)>,
    pub pending: bool,
    pub done: Option<GenResult>,
    pub texture: Option<egui::TextureHandle>,
}

/// Parameters compared for "same map" (folder and level do not change the terrain).
fn norm(p: &GenParams) -> GenParams {
    GenParams {
        folder: PathBuf::from("."),
        level: 1,
        ..p.clone()
    }
}

impl GenPreview {
    /// Keep the preview in step with `p` (debounced; generated on a thread).
    pub fn tick(&mut self, ctx: &egui::Context, gd: &Arc<GameData>, p: &GenParams, now: bool) {
        let p = norm(p);
        if let Some(rx) = &self.rx {
            while let Ok((serial, r)) = rx.try_recv() {
                if serial == self.serial {
                    self.pending = false;
                    if let Some(img) = &r.image {
                        match &mut self.texture {
                            Some(t) => t.set(img.clone(), egui::TextureOptions::NEAREST),
                            None => {
                                self.texture = Some(ctx.load_texture(
                                    "newmap-generated",
                                    img.clone(),
                                    egui::TextureOptions::NEAREST,
                                ))
                            }
                        }
                    }
                    self.done = Some(r);
                }
            }
        }
        let have = self
            .done
            .as_ref()
            .is_some_and(|d| d.params == p && Arc::ptr_eq(&d.gd, gd));
        let asked = self.asked.as_ref().is_some_and(|(q, _)| *q == p);
        if have || (asked && self.pending) {
            if self.pending {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            return;
        }
        if !asked {
            self.asked = Some((p.clone(), Instant::now()));
        }
        let ready = now
            || self
                .asked
                .as_ref()
                .is_some_and(|(_, t)| t.elapsed() >= PREVIEW_DEBOUNCE);
        if !ready {
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        self.serial += 1;
        let serial = self.serial;
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.pending = true;
        let gd = gd.clone();
        let ctx2 = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("kfx-newmap-gen".into())
            .spawn(move || {
                let r = run(&gd, &p);
                let _ = tx.send((serial, r));
                ctx2.request_repaint();
            });
        if spawned.is_err() {
            self.pending = false;
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    /// Wait for the pending preview (headless runs and tests).
    pub fn wait(&mut self, ctx: &egui::Context, gd: &Arc<GameData>, p: &GenParams) {
        self.tick(ctx, gd, p, true);
        let end = Instant::now() + Duration::from_secs(120);
        while self.pending && Instant::now() < end {
            if let Some(rx) = &self.rx {
                if let Ok((serial, r)) = rx.recv_timeout(Duration::from_millis(100)) {
                    if serial == self.serial {
                        self.pending = false;
                        if let Some(img) = &r.image {
                            match &mut self.texture {
                                Some(t) => t.set(img.clone(), egui::TextureOptions::NEAREST),
                                None => {
                                    self.texture = Some(ctx.load_texture(
                                        "newmap-generated",
                                        img.clone(),
                                        egui::TextureOptions::NEAREST,
                                    ))
                                }
                            }
                        }
                        self.done = Some(r);
                    }
                }
            }
        }
    }

    /// The finished map for `p` with `gd`, if the preview has it.
    pub fn take_for(&self, gd: &Arc<GameData>, p: &GenParams) -> Option<Arc<Generated>> {
        let d = self.done.as_ref()?;
        (d.params == norm(p) && Arc::ptr_eq(&d.gd, gd))
            .then(|| d.result.as_ref().ok().cloned())
            .flatten()
    }
}

/// Generate, analyse and render one map.
fn run(gd: &Arc<GameData>, p: &GenParams) -> GenResult {
    match generate::generate(gd, p) {
        Ok(g) => {
            let a = kfx_analysis::Analysis::of_doc(&g.doc, gd);
            GenResult {
                params: p.clone(),
                gd: gd.clone(),
                image: Some(newmap::generated_preview(&a)),
                result: Ok(Arc::new(g)),
            }
        }
        Err(e) => GenResult {
            params: p.clone(),
            gd: gd.clone(),
            result: Err(e.to_string()),
            image: None,
        },
    }
}

fn sym_label(s: SymmetryChoice) -> String {
    match s {
        SymmetryChoice::Auto => trd("kfx_editor.automatic", "Automatic").into(),
        SymmetryChoice::None => trd("kfx_editor.none_c95b", "None").into(),
        SymmetryChoice::Radial => {
            trd("kfx_editor.radial_approximate", "Radial (approximate)").into()
        }
        SymmetryChoice::Mode(m) => crate::analysis_view::mode_name(m).into(),
    }
}

/// A new seed from the old one (the dialog's Regenerate).
pub fn next_seed(s: u64) -> u64 {
    (s.wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
        >> 24)
        % 1_000_000
        + 1
}

/// The generator rows of the New map dialog's grid (style, symmetry, amounts, extras).
pub(super) fn gen_options_ui(ui: &mut egui::Ui, s: &mut NewMapSpec) {
    let g = &mut s.gen;
    ui.label(trd("kfx_editor.style", "Style"));
    egui::ComboBox::from_id_salt("nm-style")
        .selected_text(newmap::style_label(g.style))
        .show_ui(ui, |ui| {
            for st in ALL_STYLES {
                ui.selectable_value(&mut g.style, st, newmap::style_label(st))
                    .on_hover_text(newmap::style_tooltip(st));
            }
        });
    ui.end_row();
    ui.label(trd("kfx_editor.symmetry", "Symmetry"));
    egui::ComboBox::from_id_salt("nm-gsym")
        .selected_text(sym_label(g.symmetry))
        .show_ui(ui, |ui| {
            use kfx_map::transform::SymmetryMode as M;
            for c in [
                SymmetryChoice::Auto,
                SymmetryChoice::None,
                SymmetryChoice::Radial,
                SymmetryChoice::Mode(M::NorthSouth),
                SymmetryChoice::Mode(M::EastWest),
                SymmetryChoice::Mode(M::FourWay),
                SymmetryChoice::Mode(M::Rot2),
                SymmetryChoice::Mode(M::Rot4),
            ] {
                ui.selectable_value(&mut g.symmetry, c, sym_label(c));
            }
        })
        .response
        .on_hover_text(trd(
            "kfx_editor.automatic_half_turn_for_2_keepers_quarter_turn_o",
            "Automatic: half turn for 2 keepers, quarter turn or 4-way for 4, radial for \
            other counts.",
        ));
    ui.end_row();
    ui.label(trd("kfx_editor.gold", "Gold"));
    ui.add(egui::Slider::new(&mut g.richness, 0..=100).suffix(" %"));
    ui.end_row();
    ui.label(trd("kfx_editor.water_lava", "Water / lava"));
    ui.horizontal(|ui| {
        ui.add(egui::Slider::new(&mut g.water, 0..=100).suffix(" %"));
        ui.add(egui::Slider::new(&mut g.lava, 0..=100).suffix(" %"));
    });
    ui.end_row();
    ui.label(trd("kfx_editor.extras", "Extras"));
    ui.horizontal(|ui| {
        ui.label(trd("kfx_editor.gems", "gems"));
        ui.add(egui::DragValue::new(&mut g.gems).range(0..=12));
        ui.label(trd("kfx_editor.chambers", "chambers"));
        ui.add(egui::DragValue::new(&mut g.neutral_rooms).range(0..=12));
        ui.label(trd("kfx_editor.creatures", "creatures"));
        ui.add(egui::DragValue::new(&mut g.neutral_creatures).range(0..=40));
        ui.label(trd("kfx_editor.boxes", "boxes"));
        ui.add(egui::DragValue::new(&mut g.special_boxes).range(0..=20));
    });
    ui.end_row();
    ui.label(trd("kfx_editor.heroes", "Heroes"));
    ui.horizontal(|ui| {
        ui.checkbox(&mut g.heroes, trd("kfx_editor.hero_gates", "Hero gates"));
        ui.add_enabled_ui(g.heroes, |ui| {
            ui.add(egui::DragValue::new(&mut s.hero_gates).range(0..=8))
                .on_hover_text(trd(
                    "kfx_editor.hero_gates_wanted_0_one_set_around_the_map_centr",
                    "Hero gates wanted (0: one set around the map centre).",
                ));
        });
    });
    ui.end_row();
    let g = &mut s.gen;
    ui.label(trd("kfx_editor.constraints", "Constraints"));
    ui.horizontal(|ui| {
        ui.label(trd("kfx_editor.min_gold", "min gold"));
        ui.add(
            egui::DragValue::new(&mut g.min_gold)
                .range(0..=100_000)
                .speed(256),
        )
        .on_hover_text(trd(
            "kfx_editor.gold_each_keeper_must_reach_before_anyone_else_0",
            "Gold each keeper must reach before anyone else (0: from the gold amount).",
        ));
        ui.label(trd("kfx_editor.max_imbalance", "max imbalance"));
        ui.add(
            egui::DragValue::new(&mut g.max_imbalance)
                .range(0.0..=1.0)
                .speed(0.01)
                .max_decimals(2),
        )
        .on_hover_text(trd(
            "kfx_editor.largest_fairness_imbalance_accepted_0_is_perfect",
            "Largest fairness imbalance accepted (0 is perfectly even).",
        ));
        ui.label(trd("kfx_editor.attempts", "attempts"));
        ui.add(egui::DragValue::new(&mut g.max_attempts).range(1..=64));
    });
    ui.end_row();
}

/// The preview column of the New map dialog in generate mode: the rendered map and the
/// constraints report.
pub(super) fn gen_preview_ui(ui: &mut egui::Ui, pv: &GenPreview) {
    match (&pv.texture, &pv.done) {
        (Some(t), Some(d)) if d.result.is_ok() => {
            let sz = t.size_vec2();
            let scale = (340.0 / sz.x).min(340.0 / sz.y);
            let (rect, _) = ui.allocate_exact_size(sz * scale, egui::Sense::hover());
            ui.painter().image(
                t.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            if pv.pending {
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_black_alpha(110));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    trd("kfx_editor.generating", "Generating…"),
                    egui::FontId::proportional(16.0),
                    theme::TEXT,
                );
            }
        }
        _ => {
            let (rect, _) = ui.allocate_exact_size(EVec2::splat(340.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_rgb(20, 18, 16));
            let msg = if pv.pending || pv.done.is_none() {
                trd("kfx_editor.generating", "Generating…")
            } else {
                trd("kfx_editor.no_map_see_below", "No map: see below.")
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                msg,
                egui::FontId::proportional(16.0),
                TEXT_DIM,
            );
        }
    }
    // a fixed-size report: the modal is centred, so a growing report would move its buttons
    let (rect, _) = ui.allocate_exact_size(EVec2::new(340.0, 210.0), egui::Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let ui = &mut child;
    let Some(d) = &pv.done else { return };
    match &d.result {
        Ok(g) => report_ui(
            ui,
            &g.report,
            g.plan.bases.len(),
            g.doc.dims.w,
            g.doc.dims.h,
        ),
        Err(e) => {
            ui.colored_label(
                Color32::LIGHT_RED,
                trd(
                    "kfx_editor.no_map_met_every_constraint_try_another_seed_few",
                    "No map met every constraint. Try another seed, fewer keepers, more \
                    gold or more attempts.",
                ),
            );
            egui::ScrollArea::vertical()
                .id_salt("nm-fail")
                .auto_shrink([false, false])
                .max_height(150.0)
                .show(ui, |ui| {
                    ui.label(RichText::new(e).small().color(TEXT_DIM));
                });
        }
    }
}

/// The constraints report of a generated map.
fn report_ui(ui: &mut egui::Ui, r: &GenReport, keepers: usize, w: u16, h: u16) {
    let sym = match (r.symmetry, r.radial) {
        (Some(m), _) => crate::analysis_view::mode_name(m).to_string(),
        (None, true) => trd("kfx_editor.radial", "radial").into(),
        _ => trd("kfx_editor.none_557b", "none").into(),
    };
    ui.label(
        RichText::new(trf(
            "kfx_editor.w_x_h_keepers_keeper_s_symmetry_sym_attempts_att",
            "{w}x{h}, {keepers} keeper(s), symmetry {sym}, {attempts} attempt(s), {time_us} ms",
            &[
                ("w", &w),
                ("h", &h),
                ("keepers", &keepers),
                ("sym", &sym),
                ("attempts", &r.attempts),
                ("time_us", &format!("{:.0}", r.time_us as f64 / 1000.0)),
            ],
        ))
        .small()
        .color(TEXT_DIM),
    );
    if let Some(i) = r.imbalance {
        ui.label(
            RichText::new(trf(
                "kfx_editor.imbalance_i_0_is_perfectly_even",
                "Imbalance {i} (0 is perfectly even)",
                &[("i", &format!("{:.2}", i))],
            ))
            .small(),
        );
    }
    egui::ScrollArea::vertical()
        .id_salt("nm-checks")
        .auto_shrink([false, false])
        .max_height(160.0)
        .show(ui, |ui| {
            for c in &r.checks {
                let (mark, col) = if c.ok {
                    ("✔", Color32::from_rgb(120, 220, 120))
                } else {
                    ("✖", Color32::from_rgb(240, 110, 90))
                };
                ui.label(
                    RichText::new(format!("{mark} {}: {}", c.name, c.detail))
                        .small()
                        .color(col),
                );
            }
            for x in &r.repairs {
                ui.label(
                    RichText::new(trf("kfx_editor.repaired_x", "repaired: {x}", &[("x", &x)]))
                        .small()
                        .color(TEXT_DIM),
                );
            }
            for x in &r.warnings {
                ui.label(
                    RichText::new(trf("kfx_editor.note_x", "note: {x}", &[("x", &x)]))
                        .small()
                        .color(Color32::from_rgb(240, 180, 80)),
                );
            }
        });
}

/// Map > Generate into selection.
pub struct PatchDialog {
    pub params: GenParams,
    pub result: Option<Result<String, String>>,
}

impl EditorApp {
    /// The generated map for a spec: the dialog's preview when it matches, else generated now.
    pub(super) fn generated_for(
        &mut self,
        spec: &NewMapSpec,
        folder: &Path,
        level: u32,
        gd: &Arc<GameData>,
    ) -> Result<Arc<Generated>, String> {
        let p = newmap::gen_params(spec, folder, level, self.settings.decoration);
        if let Some(g) = self
            .new_dialog
            .as_ref()
            .and_then(|d| d.gen.take_for(gd, &p))
        {
            return Ok(g);
        }
        generate::generate(gd, &p)
            .map(Arc::new)
            .map_err(|e| e.to_string())
    }

    pub fn open_gen_patch(&mut self) {
        let params = GenParams {
            players: 0,
            heroes: false,
            neutral_rooms: 1,
            neutral_creatures: 2,
            special_boxes: 1,
            decorate: self.settings.decoration,
            ..Default::default()
        };
        self.gen_patch = Some(PatchDialog {
            params,
            result: None,
        });
    }

    /// Fill the slab selection with generated terrain, as one undo step.
    pub fn generate_into_selection(&mut self, p: &GenParams) -> Result<GenReport, String> {
        let ((ax, ay), (bx, by)) = self.selection.ok_or(trd(
            "kfx_editor.select_a_rectangle_of_slabs_first_select_tool",
            "Select a rectangle of slabs first (Select tool).",
        ))?;
        let rect = SlabRect {
            x0: ax.min(bx),
            y0: ay.min(by),
            x1: ax.max(bx),
            y1: ay.max(by),
        };
        let d = self.doc.as_mut().ok_or("no map open")?;
        let (r, rf) = d.op(
            &trd(
                "kfx_editor.generate_into_selection",
                "Generate into selection",
            ),
            |m, gd| match generate::generate_patch(m, gd, rect, p) {
                Ok((patch, rep)) => (Ok(rep), patch),
                Err(e) => (Err(e.to_string()), kfx_map::patch::Patch::default()),
            },
        );
        let rep = r?;
        self.after_change(rf);
        self.status = trn(
            "kfx_editor.generated_x0_x_y0_slabs_in_attempts_attempt_s_ti",
            rep.attempts as i64,
            "Generated {x0}x{y0} slabs in {n} attempt, {time_us} ms.",
            "Generated {x0}x{y0} slabs in {n} attempts, {time_us} ms.",
            &[
                ("x0", &(rect.x1 - rect.x0 + 1)),
                ("y0", &(rect.y1 - rect.y0 + 1)),
                ("time_us", &format!("{:.0}", rep.time_us as f64 / 1000.0)),
            ],
        );
        Ok(rep)
    }

    pub(super) fn gen_patch_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.gen_patch.take() else {
            return;
        };
        let mut open = true;
        let mut go = false;
        let sel = self
            .selection
            .map(|((ax, ay), (bx, by))| ((bx - ax).abs() + 1, (by - ay).abs() + 1));
        egui::Window::new(trd(
            "kfx_editor.generate_into_selection",
            "Generate into selection",
        ))
        .id(egui::Id::unique("gen-patch"))
        .default_pos(ctx.content_rect().center_top() + EVec2::new(-220.0, 140.0))
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            let p = &mut dlg.params;
            match sel {
                Some((w, h)) => ui.label(trf(
                    "kfx_editor.fills_the_selected_w_x_h_slabs_with_generated_te",
                    "Fills the selected {w}x{h} slabs with generated terrain (one \
                    undo step). Things inside are removed.",
                    &[("w", &w), ("h", &h)],
                )),
                None => ui.colored_label(
                    Color32::from_rgb(240, 180, 80),
                    trd(
                        "kfx_editor.select_a_rectangle_of_slabs_first_select_tool",
                        "Select a rectangle of slabs first (Select tool).",
                    ),
                ),
            };
            egui::Grid::new("gen-patch-grid")
                .num_columns(2)
                .spacing([10.0, 5.0])
                .show(ui, |ui| {
                    ui.label(trd("kfx_editor.style", "Style"));
                    egui::ComboBox::from_id_salt("gp-style")
                        .selected_text(newmap::style_label(p.style))
                        .show_ui(ui, |ui| {
                            for st in ALL_STYLES {
                                ui.selectable_value(&mut p.style, st, newmap::style_label(st))
                                    .on_hover_text(newmap::style_tooltip(st));
                            }
                        });
                    ui.end_row();
                    ui.label(trd("kfx_editor.seed", "Seed"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut p.seed));
                        if ui
                            .small_button("🎲")
                            .on_hover_text(trd("kfx_editor.new_seed", "New seed"))
                            .clicked()
                        {
                            p.seed = next_seed(p.seed);
                        }
                    });
                    ui.end_row();
                    ui.label(trd("kfx_editor.keepers", "Keepers"));
                    ui.add(egui::Slider::new(&mut p.players, 0..=4))
                        .on_hover_text(trd(
                            "kfx_editor.keepers_with_a_heart_inside_the_rectangle_0_terr",
                            "Keepers with a heart inside the rectangle (0: terrain only).",
                        ));
                    ui.end_row();
                    ui.label(trd("kfx_editor.gold", "Gold"));
                    ui.add(egui::Slider::new(&mut p.richness, 0..=100).suffix(" %"));
                    ui.end_row();
                    ui.label(trd("kfx_editor.water_lava", "Water / lava"));
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut p.water, 0..=100).suffix(" %"));
                        ui.add(egui::Slider::new(&mut p.lava, 0..=100).suffix(" %"));
                    });
                    ui.end_row();
                    ui.label(trd("kfx_editor.extras", "Extras"));
                    ui.horizontal(|ui| {
                        ui.label(trd("kfx_editor.chambers", "chambers"));
                        ui.add(egui::DragValue::new(&mut p.neutral_rooms).range(0..=8));
                        ui.label(trd("kfx_editor.creatures", "creatures"));
                        ui.add(egui::DragValue::new(&mut p.neutral_creatures).range(0..=20));
                        ui.label(trd("kfx_editor.boxes", "boxes"));
                        ui.add(egui::DragValue::new(&mut p.special_boxes).range(0..=10));
                        ui.checkbox(
                            &mut p.heroes,
                            trd("kfx_editor.hero_gates_d2ad", "hero gates"),
                        );
                    });
                    ui.end_row();
                });
            if let Some(r) = &dlg.result {
                match r {
                    Ok(t) => ui.label(RichText::new(t).color(Color32::from_rgb(120, 220, 120))),
                    Err(e) => ui.colored_label(Color32::LIGHT_RED, e),
                };
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        sel.is_some(),
                        egui::Button::new(trd("kfx_editor.generate", "Generate")),
                    )
                    .clicked()
                {
                    go = true;
                }
            });
        });
        if go {
            let p = dlg.params.clone();
            dlg.result = Some(
                self.generate_into_selection(&p)
                    .map(|_| self.status.clone()),
            );
        }
        if open {
            self.gen_patch = Some(dlg);
        }
    }

    /// `--actions` of the generator: `generate_selection`, `new_dialog_folder`,
    /// `new_dialog_seed`, `new_dialog_regenerate`, `new_dialog_wait`, `new_dialog_create`.
    pub(super) fn run_generator_actions(
        &mut self,
        obj: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        let get = |k: &str| obj.get(k);
        if let Some(v) = get("generate_selection").and_then(|v| v.as_object()) {
            let mut p = GenParams {
                players: 0,
                heroes: false,
                decorate: self.settings.decoration,
                ..Default::default()
            };
            let mut spec = NewMapSpec::default();
            spec.gen.heroes = false;
            for (k, val) in v {
                match k.as_str() {
                    "seed" => p.seed = val.as_u64().unwrap_or(1),
                    "players" => p.players = val.as_u64().unwrap_or(0) as u8,
                    _ => {
                        if !newmap::apply_gen_key(&mut spec, k, val)? {
                            return Err(trf(
                                "kfx_editor.unknown_generate_selection_field_k",
                                "unknown generate_selection field {k}",
                                &[("k", &k)],
                            ));
                        }
                    }
                }
            }
            let g = &spec.gen;
            (p.style, p.richness, p.water, p.lava, p.gems, p.heroes) =
                (g.style, g.richness, g.water, g.lava, g.gems, g.heroes);
            (
                p.neutral_rooms,
                p.neutral_creatures,
                p.special_boxes,
                p.max_attempts,
            ) = (
                g.neutral_rooms,
                g.neutral_creatures,
                g.special_boxes,
                g.max_attempts,
            );
            let rep = self.generate_into_selection(&p)?;
            eprintln!(
                "generate_selection: {} attempt(s), {:.0} ms; {}",
                rep.attempts,
                rep.time_us as f64 / 1000.0,
                self.status
            );
        }
        if let Some(v) = get("new_dialog_folder").and_then(|v| v.as_str()) {
            let d = self.new_dialog.as_mut().ok_or("no New map dialog")?;
            d.folder = v.to_string();
        }
        if let Some(v) = get("new_dialog_seed").and_then(|v| v.as_u64()) {
            let d = self.new_dialog.as_mut().ok_or("no New map dialog")?;
            d.spec.seed = v;
        }
        if get("new_dialog_regenerate").is_some() {
            let d = self.new_dialog.as_mut().ok_or("no New map dialog")?;
            d.spec.seed = next_seed(d.spec.seed);
        }
        if get("new_dialog_wait").is_some() {
            self.new_dialog_wait()?;
            if let Some(d) = self.new_dialog.as_ref().and_then(|d| d.gen.done.as_ref()) {
                match &d.result {
                    Ok(g) => eprintln!(
                        "new map preview: {} attempt(s), imbalance {:?}, {} check(s) ok, {:.0} ms",
                        g.report.attempts,
                        g.report.imbalance,
                        g.report.checks.iter().filter(|c| c.ok).count(),
                        g.report.time_us as f64 / 1000.0
                    ),
                    Err(e) => eprintln!("new map preview failed: {e}"),
                }
            }
        }
        if get("new_dialog_create").is_some() {
            self.new_dialog_create()?;
        }
        Ok(())
    }

    /// Generate the New map dialog's preview now and wait for it.
    pub fn new_dialog_wait(&mut self) -> Result<(), String> {
        let gd = self.base_gd().ok_or("no game data")?;
        let ctx = self.ctx.clone();
        let dec = self.settings.decoration;
        let d = self.new_dialog.as_mut().ok_or("no New map dialog")?;
        if !d.spec.generate {
            return Ok(());
        }
        let p = newmap::gen_params(&d.spec, Path::new(d.folder.trim()), d.level, dec);
        d.gen.wait(&ctx, &gd, &p);
        Ok(())
    }

    /// The New map dialog's Create button.
    pub fn new_dialog_create(&mut self) -> Result<(), String> {
        let d = self.new_dialog.as_ref().ok_or("no New map dialog")?;
        let (folder, level, spec) = (PathBuf::from(d.folder.trim()), d.level, d.spec.clone());
        self.new_map_spec(&folder, level, &spec)?;
        self.new_dialog = None;
        Ok(())
    }

    /// Set the New map dialog's folder and level (tests).
    #[cfg(test)]
    pub fn new_dialog_target(&mut self, folder: &Path, level: u32) {
        if let Some(d) = self.new_dialog.as_mut() {
            d.folder = folder.display().to_string();
            d.level = level;
        }
    }

    #[cfg(test)]
    pub fn close_gen_patch(&mut self) {
        self.gen_patch = None;
    }

    #[cfg(test)]
    pub fn gen_patch_open(&self) -> bool {
        self.gen_patch.is_some()
    }

    /// The generator's preview state of the New map dialog (tests).
    #[cfg(test)]
    pub fn new_dialog_generated(&self) -> Option<(u64, bool, Option<Result<usize, String>>)> {
        let d = self.new_dialog.as_ref()?;
        let r = d.gen.done.as_ref().map(|x| {
            x.result
                .as_ref()
                .map(|g| g.plan.bases.len())
                .map_err(|e| e.clone())
        });
        Some((d.spec.seed, d.gen.pending, r))
    }
}
