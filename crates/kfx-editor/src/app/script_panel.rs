//! The level's scripts in the editor: the script workspace (docked, own window, or maximised
//! over the map with F4), map markers for every script reference, jumps both ways, pick from
//! map, renumbering prompts and the link checks of the Validate panel.
use super::*;
use crate::scripts::{self, LevelFiles, MapConsts, NumberChange};
use crate::settings::ScriptDock;
use kfx_prefab::i18n::{trd, trf};
use kfx_script::Span;
use kfx_script_ui::workspace::{ScriptWorkspace, TabMapReference, WorkspaceEvent};
use kfx_script_ui::{MapTarget, PickKind, ScriptEvent};

/// The open level's scripts.
pub struct ScriptState {
    pub ws: ScriptWorkspace,
    pub files: LevelFiles,
    pub consts: MapConsts,
    /// Map references of both scripts as of this frame.
    pub refs: Vec<TabMapReference>,
    /// The string lookup was given to the workspace.
    pub lookup_set: bool,
}

/// A script marker on the map: one per referenced place, with every reference to it.
#[derive(Clone, Debug)]
pub struct Marker {
    pub pos: Vec3,
    pub target: MapTarget,
    /// (tab, span, line, line text)
    pub refs: Vec<(String, Span, usize, String)>,
    /// Screen rectangle of the last drawing (viewport-local egui points).
    pub rect: Option<egui::Rect>,
}

/// The question asked after an action point / hero gate changed number or was deleted.
#[derive(Clone, Debug)]
pub struct NumberPrompt {
    pub changes: Vec<(NumberChange, Vec<TabMapReference>)>,
}

impl EditorApp {
    /// (Re)attach the scripts of the open document: kept when the folder and level are the same
    /// (a reload), else loaded from disk.
    pub(super) fn attach_scripts(&mut self) {
        let (Some(doc), Some(install)) = (&self.doc, self.install().map(Path::to_path_buf)) else {
            self.scripts = None;
            return;
        };
        let mut files = LevelFiles::new(&install, &doc.doc.folder, doc.doc.level, &doc.layers);
        files.backups = self.session_backups.clone();
        let mut consts = MapConsts::new(&doc.gd, &doc.layers);
        consts.set_map(&doc.doc, &doc.gd);
        self.numbers = scripts::map_numbers(&doc.doc, &doc.gd);
        let same = self
            .scripts
            .as_ref()
            .is_some_and(|s| s.files.folder == files.folder && s.files.level == files.level);
        if same {
            let s = self.scripts.as_mut().unwrap();
            s.files = files;
            s.consts = consts;
            s.ws.invalidate();
            return;
        }
        let mut ws = ScriptWorkspace::new(doc.doc.level);
        ws.set_lua_index(std::sync::Arc::new(scripts::lua_index(
            &doc.doc.folder,
            &doc.layers,
        )));
        ws.set_font_size(self.settings.ui.script_font_size);
        ws.set_light(self.settings.ui.script_light);
        ws.load(&files);
        self.scripts = Some(ScriptState {
            ws,
            files,
            consts,
            refs: vec![],
            lookup_set: false,
        });
        self.pick = None;
        self.refresh_strings();
    }

    /// Scripts have unsaved edits.
    pub fn scripts_dirty(&self) -> bool {
        self.scripts.as_ref().is_some_and(|s| s.ws.is_dirty())
    }

    /// Map or scripts have unsaved changes.
    pub fn anything_dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.is_dirty())
            || self.scripts_dirty()
            || self.strings_dirty()
    }

    /// Save the scripts with unsaved edits (after the map). Returns an error listing failures.
    pub(super) fn save_scripts(&mut self) -> Result<usize, String> {
        let Some(s) = self.scripts.as_mut() else {
            return Ok(0);
        };
        let dirty = s.ws.dirty_tabs();
        let failed = s.ws.save_all(&s.files);
        if failed.is_empty() {
            Ok(dirty.len())
        } else {
            Err(trf(
                "kfx_editor.could_not_save_failed",
                "could not save {failed}",
                &[("failed", &failed.join(", "))],
            ))
        }
    }

    /// After a Save As to another folder / level: write the scripts there under the new names
    /// and attach the workspace to the new level.
    pub(super) fn move_scripts(&mut self, folder: &Path, level: u32) -> Result<(), String> {
        let Some(old) = self.scripts.take() else {
            return Ok(());
        };
        if old.files.folder == folder && old.files.level == level {
            self.scripts = Some(old);
            return self.save_scripts().map(|_| ());
        }
        let mut new_files = old.files.clone();
        new_files.folder = folder.to_path_buf();
        new_files.level = level;
        for t in old.ws.tab_infos() {
            if !t.exists || t.read_only {
                continue;
            }
            let Some(ed) = old.ws.editor(&t.name) else {
                continue;
            };
            let name = if t.name == old.ws.lua_name() {
                kfx_script_ui::workspace::lua_file_name(level)
            } else {
                kfx_script_ui::workspace::dk_file_name(level)
            };
            kfx_script_ui::workspace::ScriptFiles::write(&new_files, &name, &ed.text_bytes())?;
        }
        self.scripts = None;
        self.attach_scripts();
        Ok(())
    }

    /// Per-frame upkeep: map numbers into the constants (re-checks the scripts when they
    /// change), the current references, the renumbering prompt.
    pub(super) fn update_scripts(&mut self) {
        let (Some(doc), Some(s)) = (&self.doc, self.scripts.as_mut()) else {
            return;
        };
        if s.consts.set_map(&doc.doc, &doc.gd) {
            s.ws.invalidate();
        }
        let refs = s.ws.map_references();
        if refs != s.refs {
            s.refs = refs;
            self.issues_rev = u64::MAX;
        }
    }

    /// Called after every map change: action points / hero gates renumbered or deleted.
    pub(super) fn check_numbers(&mut self) {
        let (Some(doc), Some(s)) = (&self.doc, self.scripts.as_mut()) else {
            return;
        };
        let now = scripts::map_numbers(&doc.doc, &doc.gd);
        if now == self.numbers {
            return;
        }
        let refs = s.ws.map_references();
        let changes: Vec<(NumberChange, Vec<TabMapReference>)> =
            scripts::number_changes(&self.numbers, &now)
                .into_iter()
                .map(|c| (c, scripts::affected(&refs, &c)))
                .filter(|(_, r)| !r.is_empty())
                .collect();
        self.numbers = now;
        if !changes.is_empty() {
            self.number_prompt = Some(NumberPrompt { changes });
        }
    }

    /// Answer the renumbering prompt: edit the scripts (`true`) or leave them.
    pub fn answer_number_prompt(&mut self, update: bool) -> usize {
        let Some(p) = self.number_prompt.take() else {
            return 0;
        };
        if !update {
            return 0;
        }
        let Some(s) = self.scripts.as_mut() else {
            return 0;
        };
        let mut n = 0;
        for (c, _) in &p.changes {
            let refs = s.ws.map_references();
            n += scripts::apply_change(&mut s.ws, &refs, c);
        }
        self.status = trf(
            "kfx_editor.updated_n_script_references_undo_in_the_script_e",
            "Updated {n} script references (undo in the script editor)",
            &[("n", &n)],
        );
        n
    }

    fn number_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.number_prompt else { return };
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("number-prompt")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(
                RichText::new(trd("kfx_editor.script_references", "Script references")).color(GOLD),
            );
            for (c, refs) in &p.changes {
                ui.label(RichText::new(c.describe()).strong());
                for r in refs.iter().take(8) {
                    ui.label(
                        RichText::new(trf(
                            "kfx_editor.tab_line_line_command",
                            "  {tab} line {line}: {command}",
                            &[
                                ("tab", &r.tab),
                                ("line", &r.reference.line),
                                ("command", &r.reference.command),
                            ],
                        ))
                        .small()
                        .color(TEXT_DIM),
                    );
                }
                if refs.len() > 8 {
                    ui.label(
                        RichText::new(trf(
                            "kfx_editor.refs_more",
                            "  … {refs} more",
                            &[("refs", &(refs.len() - 8))],
                        ))
                        .small()
                        .color(TEXT_DIM),
                    );
                }
            }
            let renames = p
                .changes
                .iter()
                .any(|(c, _)| matches!(c, NumberChange::Renamed { .. }));
            let n: usize = p.changes.iter().map(|(_, r)| r.len()).sum();
            ui.separator();
            ui.horizontal(|ui| {
                let label = if renames {
                    trf(
                        "kfx_editor.update_n_references",
                        "Update {n} references",
                        &[("n", &n)],
                    )
                } else {
                    trf(
                        "kfx_editor.comment_out_n_references",
                        "Comment out {n} references",
                        &[("n", &n)],
                    )
                };
                if ui.button(label).clicked() {
                    answer = Some(true);
                }
                if ui
                    .button(trd("kfx_editor.leave_the_scripts", "Leave the scripts"))
                    .clicked()
                {
                    answer = Some(false);
                }
            });
            ui.label(
                RichText::new(trd(
                    "kfx_editor.each_script_is_changed_in_one_step_ctrl_z_in_the",
                    "Each script is changed in one step: Ctrl+Z in the script editor reverts it.",
                ))
                .small()
                .color(TEXT_DIM),
            );
        });
        if let Some(a) = answer {
            self.answer_number_prompt(a);
        }
    }

    // ------------------------------------------------------------------------- markers

    /// Markers of every resolvable reference (grouped by place).
    pub(super) fn build_markers(&mut self) {
        self.markers.clear();
        let (Some(doc), Some(s)) = (&self.doc, self.scripts.as_ref()) else {
            return;
        };
        if !self.settings.layout.layers.script_markers {
            return;
        }
        let mut texts: HashMap<String, Vec<u8>> = HashMap::new();
        for r in &s.refs {
            let Some(pos) = scripts::target_pos(&doc.doc, &doc.gd, r.reference.target) else {
                continue;
            };
            let bytes = texts.entry(r.tab.clone()).or_insert_with(|| {
                s.ws.editor(&r.tab)
                    .map(|e| e.text_bytes())
                    .unwrap_or_default()
            });
            let line = scripts::line_text(bytes, r.reference.line);
            let entry = (r.tab.clone(), r.reference.span, r.reference.line, line);
            match self
                .markers
                .iter_mut()
                .find(|m| m.target == r.reference.target || (m.pos - pos).length() < 0.01)
            {
                Some(m) => m.refs.push(entry),
                None => self.markers.push(Marker {
                    pos,
                    target: r.reference.target,
                    refs: vec![entry],
                    rect: None,
                }),
            }
        }
    }

    /// Draw the markers and the flash; returns the marker under the pointer.
    pub(super) fn draw_markers(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        vp: Vec2,
        ppp: f32,
        pointer: Option<egui::Pos2>,
    ) -> Option<usize> {
        let to_ui = |p: Vec2| rect.min + EVec2::new(p.x / ppp, p.y / ppp);
        let mut hovered = None;
        let col = Color32::from_rgb(200, 120, 255);
        // zoomed out: markers close on the screen are clustered into one bubble with a count
        if self.settings.graphics.declutter && self.camera.zoom < 0.5 {
            let mut clusters: Vec<(egui::Pos2, Vec<usize>, usize)> = vec![];
            for (i, m) in self.markers.iter_mut().enumerate() {
                m.rect = None;
                let Some(p) = self.camera.project(m.pos, vp) else {
                    continue;
                };
                let at = to_ui(p);
                if !rect.contains(at) {
                    continue;
                }
                match clusters.iter_mut().find(|c| (c.0 - at).length() < 46.0) {
                    Some(c) => {
                        c.1.push(i);
                        c.2 += m.refs.len();
                    }
                    None => clusters.push((at, vec![i], m.refs.len())),
                }
            }
            for (at, members, n) in clusters {
                let text = if members.len() == 1 {
                    trf("kfx_editor.n", "\u{1F4DC} {n}", &[("n", &n)])
                } else {
                    trf(
                        "kfx_editor.n_members_places",
                        "\u{1F4DC} {n} ({members} places)",
                        &[("n", &n), ("members", &members.len())],
                    )
                };
                let galley =
                    painter.layout_no_wrap(text, egui::FontId::proportional(10.0), Color32::WHITE);
                let r = egui::Rect::from_center_size(
                    at + EVec2::new(0.0, -12.0),
                    galley.size() + EVec2::new(8.0, 3.0),
                );
                let hot = pointer.is_some_and(|p| r.expand(2.0).contains(p));
                painter.rect(
                    r,
                    6.0,
                    if hot {
                        Color32::from_rgb(110, 50, 150)
                    } else {
                        Color32::from_rgba_unmultiplied(50, 20, 70, 200)
                    },
                    egui::Stroke::new(1.0, col),
                    egui::StrokeKind::Inside,
                );
                painter.galley(r.min + EVec2::new(4.0, 1.5), galley, Color32::WHITE);
                painter.circle_filled(at, 2.5, col);
                for &i in &members {
                    self.markers[i].rect = Some(r);
                }
                if hot {
                    hovered = members.first().copied();
                }
            }
            return hovered.or(self.draw_flash(painter, rect, vp, ppp));
        }
        for (i, m) in self.markers.iter_mut().enumerate() {
            m.rect = None;
            let Some(p) = self.camera.project(m.pos + Vec3::Y * 0.5, vp) else {
                continue;
            };
            // to the right of the place, clear of the action point / gate badges above it
            let at = to_ui(p) + EVec2::new(26.0, -14.0);
            if !rect.contains(at) {
                continue;
            }
            let lines: Vec<String> = m
                .refs
                .iter()
                .take(3)
                .map(|r| format!("{}{}", if r.0.ends_with(".lua") { "L" } else { "" }, r.2))
                .collect();
            let more = if m.refs.len() > 3 { "+" } else { "" };
            let text = format!("\u{1F4DC} {}{more}", lines.join(","));
            let galley =
                painter.layout_no_wrap(text, egui::FontId::proportional(10.5), Color32::WHITE);
            let r = egui::Rect::from_min_size(
                at - EVec2::new(0.0, (galley.size().y + 3.0) / 2.0),
                galley.size() + EVec2::new(8.0, 3.0),
            );
            let hot = pointer.is_some_and(|p| r.expand(2.0).contains(p));
            painter.rect(
                r,
                4.0,
                if hot {
                    Color32::from_rgb(110, 50, 150)
                } else {
                    Color32::from_rgba_unmultiplied(50, 20, 70, 220)
                },
                egui::Stroke::new(1.0, col),
                egui::StrokeKind::Inside,
            );
            painter.galley(r.min + EVec2::new(4.0, 1.5), galley, Color32::WHITE);
            painter.line_segment(
                [
                    egui::pos2(r.min.x, r.center().y),
                    to_ui(self.camera.project(m.pos, vp).unwrap_or(p)),
                ],
                egui::Stroke::new(1.0, col.gamma_multiply(0.7)),
            );
            m.rect = Some(r);
            if hot {
                hovered = Some(i);
            }
        }
        hovered.or(self.draw_flash(painter, rect, vp, ppp))
    }

    /// The flash after a jump (never "hovered").
    fn draw_flash(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        vp: Vec2,
        ppp: f32,
    ) -> Option<usize> {
        let to_ui = |p: Vec2| rect.min + EVec2::new(p.x / ppp, p.y / ppp);
        if let Some((pos, t0)) = self.flash {
            let age = if self.fixed_frame.is_some() {
                0.4
            } else {
                t0.elapsed().as_secs_f32()
            };
            if age < 2.0 {
                if let Some(p) = self.camera.project(pos, vp) {
                    let c = to_ui(p);
                    let k = (age * 3.0).fract();
                    let a = ((1.0 - age / 2.0) * 255.0) as u8;
                    for r in [10.0 + 30.0 * k, 22.0 + 30.0 * k] {
                        painter.circle_stroke(
                            c,
                            r,
                            egui::Stroke::new(
                                3.0,
                                Color32::from_rgba_unmultiplied(255, 230, 90, a),
                            ),
                        );
                    }
                    painter.circle_filled(c, 4.0, Color32::from_rgba_unmultiplied(255, 230, 90, a));
                }
                self.ctx.request_repaint();
            } else {
                self.flash = None;
            }
        }
        None
    }

    /// Tooltip of a hovered marker, drawn next to `at`.
    pub(super) fn marker_tooltip(&self, painter: &egui::Painter, i: usize, at: egui::Pos2) {
        let Some(m) = self.markers.get(i) else { return };
        let mut text: Vec<String> = m
            .refs
            .iter()
            .take(12)
            .map(|(tab, _, line, t)| {
                trf(
                    "kfx_editor.tab_line_line_t",
                    "{tab} line {line}: {t}",
                    &[("tab", &tab), ("line", &line), ("t", &t)],
                )
            })
            .collect();
        if m.refs.len() > 12 {
            text.push(trf(
                "kfx_editor.refs_more_545a",
                "… {refs} more",
                &[("refs", &(m.refs.len() - 12))],
            ));
        }
        let label = trf(
            "kfx_editor.target_text_click_show_in_the_script_editor",
            "{target}\n{text}\nClick: show in the script editor",
            &[
                ("target", &scripts::target_label(m.target)),
                ("text", &text.join("\n")),
            ],
        );
        let galley = painter.layout(label, egui::FontId::proportional(12.5), theme::TEXT, 520.0);
        let clip = painter.clip_rect();
        let mut r = egui::Rect::from_min_size(
            at + EVec2::new(14.0, 14.0),
            galley.size() + EVec2::new(14.0, 10.0),
        );
        if r.max.x > clip.max.x {
            r = r.translate(EVec2::new(clip.max.x - r.max.x - 4.0, 0.0));
        }
        if r.max.y > clip.max.y {
            r = r.translate(EVec2::new(0.0, at.y - r.min.y - r.height() - 8.0));
        }
        painter.rect(
            r,
            4.0,
            Color32::from_rgba_unmultiplied(24, 16, 30, 240),
            egui::Stroke::new(1.0, Color32::from_rgb(200, 120, 255)),
            egui::StrokeKind::Inside,
        );
        painter.galley(r.min + EVec2::new(7.0, 5.0), galley, theme::TEXT);
    }

    /// Show a script line: open the panel if needed, activate the tab, select the span.
    pub fn reveal_script(&mut self, tab: &str, span: Span) {
        if self.settings.script_dock == ScriptDock::Hidden && !self.script_max {
            self.settings.script_dock = ScriptDock::Docked;
        }
        if let Some(s) = self.scripts.as_mut() {
            s.ws.activate(tab);
            if let Some(e) = s.ws.editor_mut(tab) {
                e.reveal_span(span);
            }
        }
    }

    /// Show a script line by number.
    pub fn reveal_script_line(&mut self, tab: &str, line: usize) {
        if self.settings.script_dock == ScriptDock::Hidden && !self.script_max {
            self.settings.script_dock = ScriptDock::Docked;
        }
        if let Some(s) = self.scripts.as_mut() {
            s.ws.activate(tab);
            if let Some(e) = s.ws.editor_mut(tab) {
                e.goto_line(line);
            }
        }
    }

    /// Click on marker `i`.
    pub fn click_marker(&mut self, i: usize) {
        let Some((tab, span, line, _)) = self.markers.get(i).and_then(|m| m.refs.first().cloned())
        else {
            return;
        };
        self.reveal_script(&tab, span);
        self.status = trf(
            "kfx_editor.tab_line_line",
            "{tab} line {line}",
            &[("tab", &tab), ("line", &line)],
        );
    }

    /// Centre the camera on a script target and flash it.
    pub fn jump_to_target(&mut self, t: MapTarget) {
        let pos = self
            .doc
            .as_ref()
            .and_then(|d| scripts::target_pos(&d.doc, &d.gd, t));
        match pos {
            Some(p) => {
                self.camera.target = Vec3::new(p.x, self.camera.target.y, p.z);
                self.flash = Some((p, Instant::now()));
                self.status = trf(
                    "kfx_editor.t_on_the_map",
                    "{t} on the map",
                    &[("t", &scripts::target_label(t))],
                );
            }
            None => {
                self.status = trf(
                    "kfx_editor.t_is_not_on_the_map",
                    "{t} is not on the map",
                    &[("t", &scripts::target_label(t))],
                )
            }
        }
    }

    // ------------------------------------------------------------------------- pick mode

    pub fn start_pick(&mut self, kind: PickKind) {
        self.pick = Some(kind);
        let what = match kind {
            PickKind::ActionPoint => trd("kfx_editor.an_action_point", "an action point"),
            PickKind::HeroGate => trd("kfx_editor.a_hero_gate", "a hero gate"),
            PickKind::Location => trd(
                "kfx_editor.an_action_point_or_a_hero_gate",
                "an action point or a hero gate",
            ),
            PickKind::Subtile => trd("kfx_editor.a_subtile", "a subtile"),
            PickKind::Slab => trd("kfx_editor.a_slab", "a slab"),
        };
        self.status = trf(
            "kfx_editor.pick_what_on_the_map_esc_cancels",
            "Pick {what} on the map (Esc cancels)",
            &[("what", &what)],
        );
    }

    /// A click in pick mode at viewport pixel `px`.
    pub(super) fn pick_click(&mut self, px: Vec2, vp: Vec2) {
        let Some(kind) = self.pick else { return };
        let Some(doc) = &self.doc else { return };
        let hits = self.items_at_pixel(px);
        let stl = self
            .pick_at(px, vp)
            .map(|h| (h.hit.stl_x as i32, h.hit.stl_y as i32));
        let ap = hits
            .iter()
            .find_map(|it| {
                if let ItemRef::ActionPoint(i) = it {
                    doc.doc.action_points.items.get(*i).and_then(|a| a.number)
                } else {
                    None
                }
            })
            .map(|n| n as i32);
        let gate = hits
            .iter()
            .find_map(|it| {
                if let ItemRef::Thing(i) = it {
                    doc.doc
                        .things
                        .items
                        .get(*i)
                        .filter(|t| kfx_map::things::is_hero_gate(&doc.gd, t))
                        .and_then(|t| t.herogate_number)
                } else {
                    None
                }
            })
            .map(|n| n as i32);
        self.pick_with(kind, stl, ap, gate);
    }

    /// Finish pick mode with what was clicked (also the headless `pick` action).
    pub fn pick_with(
        &mut self,
        kind: PickKind,
        stl: Option<(i32, i32)>,
        ap: Option<i32>,
        gate: Option<i32>,
    ) {
        let v = stl
            .and_then(|s| scripts::pick_value(kind, s, ap, gate))
            .or_else(|| {
                scripts::pick_value(kind, (0, 0), ap, gate)
                    .filter(|_| ap.is_some() || gate.is_some())
            });
        match v {
            Some(v) => {
                if self.logic_take_pick(v) {
                    // the Events editor asked
                } else if let Some(s) = self.scripts.as_mut() {
                    s.ws.insert_pick(v);
                }
                self.pick = None;
                self.status = trf(
                    "kfx_editor.picked_v",
                    "Picked {v}",
                    &[("v", &format!("{:?}", v))],
                );
            }
            None => self.status = trd(
                "kfx_editor.nothing_of_that_kind_there_click_an_action_point",
                "Nothing of that kind there: click an action point / hero gate icon (Esc cancels)",
            ),
        }
    }

    /// Headless pick at a subtile: the action point / hero gate on it, if any.
    pub fn pick_subtile(&mut self, stl: (i32, i32)) {
        let Some(kind) = self.pick else { return };
        let Some(doc) = &self.doc else { return };
        let ap = doc
            .doc
            .action_points
            .items
            .iter()
            .find(|a| a.x.zip(a.y).is_some_and(|(x, y)| (x.stl, y.stl) == stl))
            .and_then(|a| a.number)
            .map(|n| n as i32);
        let gate = doc
            .doc
            .things
            .items
            .iter()
            .find(|t| {
                kfx_map::things::is_hero_gate(&doc.gd, t)
                    && t.x.zip(t.y).is_some_and(|(x, y)| (x.stl, y.stl) == stl)
            })
            .and_then(|t| t.herogate_number)
            .map(|n| n as i32);
        self.pick_with(kind, Some(stl), ap, gate);
    }

    // ------------------------------------------------------------------------- UI

    fn handle_ws_events(&mut self, events: Vec<WorkspaceEvent>) {
        for e in events {
            match e {
                WorkspaceEvent::Editor {
                    event: ScriptEvent::JumpToMapLocation(t),
                    ..
                } => {
                    self.script_max = false;
                    self.jump_to_target(t);
                }
                WorkspaceEvent::Editor {
                    event: ScriptEvent::RequestPickFromMap { kind },
                    ..
                } => {
                    self.script_max = false;
                    self.start_pick(kind);
                }
                WorkspaceEvent::Editor {
                    tab,
                    event: ScriptEvent::RequestNewString { .. },
                } => self.new_string_for_script(&tab),
                WorkspaceEvent::Editor {
                    event: ScriptEvent::PlaySound(a),
                    ..
                } => self.play_script_sound(&a),
                WorkspaceEvent::Saved { tab } => {
                    self.status = trf("kfx_editor.saved_tab", "Saved {tab}", &[("tab", &tab)])
                }
                WorkspaceEvent::SaveFailed { tab, error } => {
                    self.status = trf(
                        "kfx_editor.could_not_save_tab_error",
                        "Could not save {tab}: {error}",
                        &[("tab", &tab), ("error", &error)],
                    )
                }
                WorkspaceEvent::Created { tab } => {
                    self.status = trf(
                        "kfx_editor.tab_created_not_saved_yet",
                        "{tab} created (not saved yet)",
                        &[("tab", &tab)],
                    )
                }
                WorkspaceEvent::OpenFailed { name } => {
                    self.status = trf(
                        "kfx_editor.name_is_not_available",
                        "{name} is not available",
                        &[("name", &name)],
                    )
                }
                _ => {}
            }
        }
    }

    /// The workspace in `ui` (fills it).
    pub(super) fn script_ui(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.scripts.as_mut() else {
            ui.label(
                RichText::new(trd(
                    "kfx_editor.open_a_map_to_edit_its_scripts",
                    "Open a map to edit its scripts",
                ))
                .color(TEXT_DIM),
            );
            return;
        };
        let events = s.ws.show(ui, &s.consts, &s.files);
        let (fs, light) = (s.ws.font_size(), s.ws.is_light());
        if (fs - self.settings.ui.script_font_size).abs() > f32::EPSILON
            || light != self.settings.ui.script_light
        {
            self.settings.ui.script_font_size = fs;
            self.settings.ui.script_light = light;
        }
        self.handle_ws_events(events);
        self.logic_tab_ui(ui);
    }

    /// Header of the docked / windowed workspace: mode buttons.
    fn script_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(trd("kfx_editor.scripts", "Scripts"))
                    .color(GOLD)
                    .strong(),
            );
            if let Some(s) = &self.scripts {
                ui.label(
                    RichText::new(trf(
                        "kfx_editor.refs_references_on_the_map",
                        "{refs} references on the map",
                        &[("refs", &s.refs.len())],
                    ))
                    .small()
                    .color(TEXT_DIM),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("✖")
                    .on_hover_text(trd(
                        "kfx_editor.hide_the_script_panel_f3",
                        "Hide the script panel (F3)",
                    ))
                    .clicked()
                {
                    self.settings.script_dock = ScriptDock::Hidden;
                    self.script_max = false;
                }
                if ui
                    .small_button(if self.script_max { "🗗" } else { "🗖" })
                    .on_hover_text(trd(
                        "kfx_editor.maximise_over_the_map_f4",
                        "Maximise over the map (F4)",
                    ))
                    .clicked()
                {
                    self.script_max = !self.script_max;
                }
                let pop = self.settings.script_dock == ScriptDock::Window;
                if ui
                    .small_button(if pop {
                        trd("kfx_editor.dock", "⬅ Dock")
                    } else {
                        trd("kfx_editor.window_e36e", "🗗 Window")
                    })
                    .on_hover_text(trd(
                        "kfx_editor.own_window_docked_beside_the_map",
                        "Own window / docked beside the map",
                    ))
                    .clicked()
                {
                    self.settings.script_dock = if pop {
                        ScriptDock::Docked
                    } else {
                        ScriptDock::Window
                    };
                    self.script_max = false;
                }
            });
        });
    }

    /// The docked panel (call before the central panel).
    pub(super) fn script_dock_ui(&mut self, ui: &mut egui::Ui) {
        if self.script_max || self.settings.script_dock != ScriptDock::Docked {
            return;
        }
        let max = super::side_panel_max(ui);
        let r = egui::Panel::right("scripts")
            .resizable(true)
            .default_size(self.settings.script_panel_w.min(max))
            .size_range(280f32.min(max)..=max)
            .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::same(0)))
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(6, 3))
                    .show(ui, |ui| {
                        self.script_header(ui);
                        self.script_strings_bar(ui);
                    });
                self.script_ui(ui);
            });
        self.settings.script_panel_w = r.response.rect.width();
        self.script_rect = Some(r.response.rect);
    }

    /// The maximised workspace in the central area.
    pub(super) fn script_max_ui(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(6, 3))
            .show(ui, |ui| {
                self.script_header(ui);
                self.script_strings_bar(ui);
            });
        self.script_ui(ui);
    }

    /// The workspace in its own window (an OS window where the backend has several viewports).
    pub(super) fn script_window_ui(&mut self, ctx: &egui::Context) {
        if self.script_max || self.settings.script_dock != ScriptDock::Window {
            return;
        }
        let title = self
            .scripts
            .as_ref()
            .map(|s| {
                trf(
                    "kfx_editor.scripts_map_level",
                    "Scripts - map{level}",
                    &[("level", &format!("{:05}", s.files.level))],
                )
            })
            .unwrap_or_else(|| trd("kfx_editor.scripts", "Scripts").into());
        let close = ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("kfx-scripts"),
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([1100.0, 800.0])
                .with_min_inner_size([480.0, 320.0]),
            |ui, class| {
                if class == egui::ViewportClass::EmbeddedWindow {
                    ui.set_min_size(EVec2::new(900.0, 640.0));
                }
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(6, 3))
                    .show(ui, |ui| self.script_header(ui));
                self.script_ui(ui);
                ui.ctx().input(|i| i.viewport().close_requested())
            },
        );
        if close {
            self.settings.script_dock = ScriptDock::Docked;
        }
    }

    pub(super) fn script_dialogs(&mut self, ctx: &egui::Context) {
        self.number_prompt_ui(ctx);
    }

    /// Toggle the panel (F3).
    pub fn toggle_script_panel(&mut self) {
        self.settings.script_dock = match self.settings.script_dock {
            ScriptDock::Hidden => ScriptDock::Docked,
            _ => ScriptDock::Hidden,
        };
        self.script_max = false;
    }

    /// Screen rectangles of the drawn markers with their first line (headless).
    pub fn marker_rects(&self) -> Vec<(egui::Rect, String)> {
        self.markers
            .iter()
            .filter_map(|m| {
                m.rect.map(|r| {
                    (
                        r,
                        format!(
                            "{} {:?}",
                            scripts::target_label(m.target),
                            m.refs.iter().map(|x| x.2).collect::<Vec<_>>()
                        ),
                    )
                })
            })
            .collect()
    }

    /// Script workspace for the headless actions.
    pub fn script_ws(&mut self) -> Option<&mut ScriptWorkspace> {
        self.scripts.as_mut().map(|s| &mut s.ws)
    }

    pub fn script_consts(&self) -> Option<&MapConsts> {
        self.scripts.as_ref().map(|s| &s.consts)
    }

    /// Script link issues for `revalidate`.
    pub(super) fn script_link_issues(
        &mut self,
    ) -> Vec<(kfx_map::validate::Issue, Option<(String, Span)>)> {
        let (Some(doc), Some(s)) = (&self.doc, self.scripts.as_mut()) else {
            return vec![];
        };
        let refs = s.ws.map_references();
        let exist = s.ws.tab_infos().iter().any(|t| t.exists && !t.read_only);
        scripts::link_issues(&doc.doc, &doc.gd, &refs, exist)
    }
}
