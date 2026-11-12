//! Toolbar, find bar, status bar, problems list, outline and command reference panels.
use crate::editor::*;
use crate::lang::*;
use crate::ui::hover_card;
use egui::{vec2, Color32, FontId, Sense, Stroke, TextFormat, Ui};
use kfx_script::{Severity, Span};

impl ScriptEditor {
    pub(crate) fn toolbar(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let ro = self.is_read_only();
            if ui
                .add_enabled(!ro, egui::Button::new("Format"))
                .on_hover_text("Re-indent the script (Alt+Shift+F)")
                .clicked()
            {
                self.format_document();
            }
            let marker = self.lang.line_comment().trim().to_string();
            if ui
                .add_enabled(!ro, egui::Button::new(marker))
                .on_hover_text("Toggle comment (Ctrl+/)")
                .clicked()
            {
                self.toggle_comment();
                self.want_focus = true;
            }
            if ui.button("Find").on_hover_text("Find (Ctrl+F)").clicked() {
                self.open_find(false);
            }
            if ui
                .button("Replace")
                .on_hover_text("Find and replace (Ctrl+H)")
                .clicked()
            {
                self.open_find(true);
            }
            if ui
                .button("Go to")
                .on_hover_text("Go to line (Ctrl+G)")
                .clicked()
            {
                self.goto.open = true;
                self.goto.focus = true;
            }
            ui.separator();
            ui.selectable_value(&mut self.side, SidePanel::Outline, "Outline");
            ui.selectable_value(&mut self.side, SidePanel::Commands, "Commands");
            ui.selectable_value(&mut self.side, SidePanel::Snippets, "Snippets");
            let (e, w, i) = self.diagnostic_counts();
            let prob = format!("Problems {e}/{w}/{i}");
            if self.options.show_problems_button {
                let mut open = self.problems_open;
                ui.toggle_value(&mut open, prob)
                    .on_hover_text("Errors / warnings / infos");
                self.problems_open = open;
            }
            ui.separator();
            if ui
                .small_button("A-")
                .on_hover_text("Smaller font")
                .clicked()
            {
                self.set_font_size(self.font_size() - 1.0);
            }
            ui.label(egui::RichText::new(format!("{:.0}", self.font_size())).small());
            if ui.small_button("A+").on_hover_text("Larger font").clicked() {
                self.set_font_size(self.font_size() + 1.0);
            }
            let light = !self.theme.is_dark;
            if ui
                .small_button(if light { "Dark" } else { "Light" })
                .on_hover_text("Switch between the dark and the light theme")
                .clicked()
            {
                self.set_light(!light);
            }
            // map integration
            let off = self.caret.1;
            let pick = self
                .cache
                .as_ref()
                .and_then(|c| c.analysis.pick_target(off));
            let here = self.cache.as_ref().and_then(|c| {
                c.map_refs()
                    .iter()
                    .find(|r| r.span.touches(off))
                    .map(|r| r.target)
            });
            if pick.is_some() || here.is_some() {
                ui.separator();
            }
            if let Some(kind) = pick {
                if ui
                    .button(egui::RichText::new("Pick from map").color(theme.accent))
                    .on_hover_text("Choose this argument's value on the map")
                    .clicked()
                {
                    self.events.push(ScriptEvent::RequestPickFromMap { kind });
                }
            }
            if let Some(t) = here {
                if ui
                    .button("Show on map")
                    .on_hover_text("Ctrl+click on a value does the same")
                    .clicked()
                {
                    self.events.push(ScriptEvent::JumpToMapLocation(t));
                }
            }
        });
    }

    pub(crate) fn find_bar(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let fid = self.id.with("find");
        let rid = self.id.with("replace");
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label("Find");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.find.query)
                    .id(fid)
                    .desired_width(200.0)
                    .hint_text("text"),
            );
            if self.find.focus {
                r.request_focus();
                self.find.focus = false;
            }
            if r.changed() {
                self.find.matches_key = None;
                self.find.current = None;
                if !self.find.query.is_empty() {
                    self.find_from_caret();
                }
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let back = ui.input(|i| i.modifiers.shift);
                self.find_step(!back);
                r.request_focus();
            }
            let mut cs = self.find.opts.case_sensitive;
            if ui
                .toggle_value(&mut cs, "Aa")
                .on_hover_text("Match case")
                .changed()
            {
                self.find.opts.case_sensitive = cs;
                self.find.matches_key = None;
            }
            if ui.button("Prev").clicked() {
                self.find_step(false);
            }
            if ui.button("Next").clicked() {
                self.find_step(true);
            }
            let ms = self.find_matches();
            let idx = self
                .find
                .current
                .and_then(|c| ms.iter().position(|m| *m == c));
            let txt = match (idx, ms.len()) {
                (_, 0) if self.find.query.is_empty() => String::new(),
                (_, 0) => "no results".to_string(),
                (Some(i), n) => format!("{} of {}", i + 1, n),
                (None, n) => format!("{n} results"),
            };
            ui.label(egui::RichText::new(txt).small().color(
                if ms.is_empty() && !self.find.query.is_empty() {
                    theme.diag_error
                } else {
                    theme.text.gamma_multiply(0.7)
                },
            ));
            if ui.button("x").on_hover_text("Close (Esc)").clicked() {
                self.find.open = false;
                self.want_focus = true;
            }
        });
        if self.find.replace {
            ui.horizontal(|ui| {
                ui.label("With");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.find.replacement)
                        .id(rid)
                        .desired_width(200.0)
                        .hint_text("replacement"),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.replace_current();
                    r.request_focus();
                }
                if ui.button("Replace").clicked() {
                    self.replace_current();
                }
                if ui.button("Replace all").clicked() {
                    self.replace_all();
                }
            });
        }
    }

    pub(crate) fn goto_bar(&mut self, ui: &mut Ui) {
        let gid = self.id.with("goto");
        ui.horizontal(|ui| {
            ui.label("Go to line");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.goto.text)
                    .id(gid)
                    .desired_width(80.0),
            );
            if self.goto.focus {
                r.request_focus();
                self.goto.focus = false;
            }
            let lines = self
                .cache
                .as_ref()
                .map(|c| c.lines.line_count())
                .unwrap_or(1);
            ui.weak(format!("1 - {lines}"));
            let go = ui.button("Go").clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go {
                if let Ok(n) = self.goto.text.trim().parse::<usize>() {
                    self.goto_line(n);
                    self.goto.open = false;
                }
            }
            if ui.button("x").clicked() {
                self.goto.open = false;
                self.want_focus = true;
            }
        });
    }

    pub(crate) fn status_bar(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let (l, c) = self.caret_line_col();
        let sel = self.caret.0.abs_diff(self.caret.1);
        let eol = match self.eol {
            crate::latin1::Eol::Lf => "LF",
            crate::latin1::Eol::CrLf => "CRLF",
            crate::latin1::Eol::Raw => "mixed EOL",
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.label(egui::RichText::new(format!("Ln {l}, Col {c}")).small());
            if sel > 0 {
                ui.label(egui::RichText::new(format!("{sel} selected")).small());
            }
            ui.label(egui::RichText::new(eol).small().weak());
            ui.label(egui::RichText::new(self.lang.name()).small().weak());
            if self.is_read_only() {
                ui.label(egui::RichText::new("read-only").small().color(theme.accent));
            } else if self.is_dirty() {
                ui.label(egui::RichText::new("modified").small().color(theme.accent));
            }
            ui.separator();
            let meters = self.validation.meters.clone();
            if meters.is_empty() {
                let items = if self.is_read_only() {
                    vec![]
                } else {
                    self.status_items()
                };
                if items.is_empty() && self.validation_pending() {
                    ui.weak("Checking…");
                }
                let mut jump = None;
                for it in &items {
                    let r = ui.add(
                        egui::Label::new(
                            egui::RichText::new(&it.text)
                                .small()
                                .monospace()
                                .color(theme.tone_color(it.tone)),
                        )
                        .sense(Sense::click()),
                    );
                    if r.on_hover_text(&it.tooltip).clicked() {
                        jump = it.at;
                    }
                }
                if let Some(sp) = jump {
                    self.caret = (sp.start, sp.start);
                    self.want_sel = Some(self.caret);
                    self.want_scroll = true;
                    self.want_focus = true;
                }
            }
            for m in &meters {
                meter_widget(ui, m, &theme);
            }
        });
    }

    pub(crate) fn problems_panel(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        ui.horizontal(|ui| {
            ui.strong("Problems");
            let names = ["errors", "warnings", "info"];
            let cols = [theme.diag_error, theme.diag_warning, theme.diag_info];
            let counts = self.diagnostic_counts();
            let cs = [counts.0, counts.1, counts.2];
            for i in 0..3 {
                let mut on = self.problems_filter[i];
                if ui
                    .toggle_value(
                        &mut on,
                        egui::RichText::new(format!("{} {}", cs[i], names[i])).color(cols[i]),
                    )
                    .changed()
                {
                    self.problems_filter[i] = on;
                }
            }
        });
        ui.separator();
        let diags = self.validation.diagnostics.clone();
        let mut jump: Option<kfx_script::Diagnostic> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(
                ui,
                ui.text_style_height(&egui::TextStyle::Body) + 6.0,
                diags.len(),
                |ui, range| {
                    for d in &diags[range] {
                        let idx = match d.severity {
                            Severity::Error => 0,
                            Severity::Warning => 1,
                            Severity::Info => 2,
                        };
                        if !self.problems_filter[idx] {
                            continue;
                        }
                        let (lbl, col) = match d.severity {
                            Severity::Error => ("error", theme.diag_error),
                            Severity::Warning => ("warning", theme.diag_warning),
                            Severity::Info => ("info", theme.diag_info),
                        };
                        let mut job = egui::text::LayoutJob::default();
                        job.append(
                            &format!("{lbl:<8}"),
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(12.0),
                                color: col,
                                ..Default::default()
                            },
                        );
                        job.append(
                            &format!("{:>5}  ", d.line),
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(12.0),
                                color: theme.gutter_fg_active,
                                ..Default::default()
                            },
                        );
                        job.append(
                            &d.message,
                            0.0,
                            TextFormat {
                                font_id: FontId::proportional(13.0),
                                color: theme.text,
                                ..Default::default()
                            },
                        );
                        job.append(
                            &format!("   {}", d.code),
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(11.0),
                                color: theme.text.gamma_multiply(0.45),
                                ..Default::default()
                            },
                        );
                        if ui.selectable_label(false, job).clicked() {
                            jump = Some(d.clone());
                        }
                    }
                    if diags.is_empty() {
                        ui.weak("No problems found.");
                    }
                },
            );
        if let Some(d) = jump {
            self.select_line_of_diag(&d);
        }
    }

    pub(crate) fn outline_panel(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        ui.strong("Outline");
        ui.separator();
        self.ensure_cache();
        let items: Vec<OutlineItem> = self
            .cache
            .as_ref()
            .map(|c| c.outline().to_vec())
            .unwrap_or_default();
        let caret = self.caret.1;
        let mut jump: Option<Span> = None;
        egui::ScrollArea::vertical()
            .id_salt("outline")
            .max_height((ui.available_height() * 0.62).max(80.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                fn walk(
                    ui: &mut Ui,
                    items: &[OutlineItem],
                    depth: usize,
                    caret: usize,
                    theme: &crate::theme::Theme,
                    jump: &mut Option<Span>,
                ) {
                    for it in items {
                        let col = theme.tone_color(it.tone);
                        let inside = it.span.start <= caret && caret <= it.span.end;
                        let mut job = egui::text::LayoutJob::default();
                        job.append(&"  ".repeat(depth), 0.0, TextFormat::default());
                        job.append(
                            &format!("{:<6}", it.tag),
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(11.0),
                                color: col,
                                ..Default::default()
                            },
                        );
                        let label: String = it.label.chars().take(60).collect();
                        job.append(
                            &label,
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(12.0),
                                color: theme.text,
                                ..Default::default()
                            },
                        );
                        job.append(
                            &format!("  :{}", it.line),
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(11.0),
                                color: theme.gutter_fg,
                                ..Default::default()
                            },
                        );
                        if ui
                            .selectable_label(
                                inside
                                    && it
                                        .children
                                        .iter()
                                        .all(|c| !(c.span.start <= caret && caret <= c.span.end)),
                                job,
                            )
                            .clicked()
                        {
                            *jump = Some(Span::new(it.span.start, it.span.start));
                        }
                        walk(ui, &it.children, depth + 1, caret, theme, jump);
                    }
                }
                if items.is_empty() {
                    ui.weak("Nothing to show yet.");
                }
                walk(ui, &items, 0, caret, &theme, &mut jump);
            });
        ui.add_space(6.0);
        ui.strong("Map references");
        ui.separator();
        // spans of references are in buffer coordinates here (internal use)
        let refs: Vec<MapReference> = self
            .cache
            .as_ref()
            .map(|c| c.map_refs().to_vec())
            .unwrap_or_default();
        let mut jump_map: Option<MapTarget> = None;
        egui::ScrollArea::vertical()
            .id_salt("maprefs")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if refs.is_empty() {
                    ui.weak(
                        "The script does not reference action points, hero gates or coordinates.",
                    );
                }
                for r in refs.iter().take(400) {
                    let desc = match r.target {
                        MapTarget::ActionPoint(n) => format!("action point {n}"),
                        MapTarget::HeroGate(n) => format!("hero gate {n}"),
                        MapTarget::Subtile { x, y } => format!("subtile {x}, {y}"),
                        MapTarget::Slab { x, y } => format!("slab {x}, {y}"),
                    };
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("Map")
                            .on_hover_text("Show on the map")
                            .clicked()
                        {
                            jump_map = Some(r.target);
                        }
                        let mut job = egui::text::LayoutJob::default();
                        job.append(
                            &desc,
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(12.0),
                                color: theme.number,
                                ..Default::default()
                            },
                        );
                        job.append(
                            &format!("  {} :{}", r.command, r.line),
                            0.0,
                            TextFormat {
                                font_id: FontId::monospace(11.0),
                                color: theme.gutter_fg,
                                ..Default::default()
                            },
                        );
                        if ui.selectable_label(false, job).clicked() {
                            jump = Some(r.span);
                        }
                    });
                }
            });
        if let Some(t) = jump_map {
            self.events.push(ScriptEvent::JumpToMapLocation(t));
        }
        if let Some(s) = jump {
            self.caret = (s.start, s.end);
            self.want_sel = Some(self.caret);
            self.want_scroll = true;
            self.want_focus = true;
        }
    }

    pub(crate) fn commands_panel(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        ui.strong(if self.lang.id() == "lua" {
            "Function reference"
        } else {
            "Command reference"
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.ref_search)
                .hint_text("search the reference")
                .desired_width(f32::INFINITY),
        );
        ui.separator();
        let q = self.ref_search.trim().to_ascii_lowercase();
        let selected = self.ref_selected.clone();
        let detail_h = if selected.is_some() {
            (ui.available_height() * 0.5).max(120.0)
        } else {
            0.0
        };
        let list_h = (ui.available_height() - detail_h - 8.0).max(80.0);
        let mut pick: Option<String> = None;
        let mut insert: Option<String> = None;
        let categories: Vec<&str> = {
            let mut v: Vec<&str> = crate::docs::CATEGORY_ORDER
                .iter()
                .copied()
                .filter(|c| self.reference.iter().any(|e| e.category == *c))
                .collect();
            let mut rest: Vec<&str> = self
                .reference
                .iter()
                .map(|e| e.category.as_str())
                .filter(|c| !crate::docs::CATEGORY_ORDER.contains(c))
                .collect();
            rest.sort();
            rest.dedup();
            v.extend(rest);
            v
        };
        egui::ScrollArea::vertical()
            .id_salt("cmdlist")
            .max_height(list_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for cat in &categories {
                    let entries: Vec<&RefEntry> = self
                        .reference
                        .iter()
                        .filter(|e| e.category == *cat)
                        .filter(|e| {
                            q.is_empty()
                                || e.name.to_ascii_lowercase().contains(&q)
                                || e.summary.to_ascii_lowercase().contains(&q)
                        })
                        .collect();
                    if entries.is_empty() {
                        continue;
                    }
                    egui::CollapsingHeader::new(
                        egui::RichText::new(format!("{cat} ({})", entries.len()))
                            .color(theme.accent),
                    )
                    .id_salt(("cat", self.lang.id(), *cat))
                    .open(if q.is_empty() { None } else { Some(true) })
                    .default_open(*cat == "Flow and conditions" || *cat == "Level setup")
                    .show(ui, |ui| {
                        for e in entries {
                            let mut job = egui::text::LayoutJob::default();
                            job.append(
                                &e.name,
                                0.0,
                                TextFormat {
                                    font_id: FontId::monospace(12.5),
                                    color: if e.note.is_some() {
                                        theme.text.gamma_multiply(0.6)
                                    } else {
                                        theme.command
                                    },
                                    ..Default::default()
                                },
                            );
                            let r = ui.selectable_label(selected.as_deref() == Some(&e.name), job);
                            if r.clicked() {
                                pick = Some(e.name.clone());
                            }
                            if r.double_clicked() {
                                insert = Some(e.name.clone());
                            }
                        }
                    });
                }
            });
        if let Some(name) = &selected {
            ui.separator();
            if let Some(e) = self.reference.iter().find(|e| &e.name == name).cloned() {
                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new("Insert at caret").color(theme.accent))
                        .clicked()
                    {
                        insert = Some(e.name.clone());
                    }
                    if let Some(u) = &e.link {
                        if ui.button("Wiki").clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(u));
                        }
                    }
                    if let Some(n) = &e.note {
                        ui.weak(n);
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("cmddetail")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let info = HoverInfo {
                            title: Some(e.signature.clone()),
                            summary: e.summary.clone(),
                            params: e
                                .params
                                .iter()
                                .map(|(a, b)| (a.clone(), crate::docs::short(b)))
                                .collect(),
                            examples: e.examples.clone(),
                            ..Default::default()
                        };
                        hover_card(ui, &info, &theme);
                    });
            }
        }
        if let Some(p) = pick {
            self.ref_selected = Some(p);
        }
        if let Some(n) = insert {
            self.ref_selected = Some(n.clone());
            self.insert_command(&n);
        }
    }
}

fn meter_widget(ui: &mut Ui, m: &Meter, theme: &crate::theme::Theme) {
    let col = match m.level() {
        MeterLevel::Ok => theme.meter_ok,
        MeterLevel::Near => theme.meter_near,
        MeterLevel::Over => theme.meter_over,
    };
    let text = if m.limit > 0 {
        format!("{} {}/{}", m.label, m.used, m.limit)
    } else {
        format!("{} {}", m.label, m.used)
    };
    let font = FontId::proportional(11.5);
    let w = ui.fonts_mut(|f| {
        f.layout_no_wrap(text.clone(), font.clone(), Color32::WHITE)
            .size()
            .x
    }) + 14.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(90.0), 17.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, theme.background);
    let mut fill = rect;
    fill.set_width(rect.width() * m.fraction().max(if m.used > 0 { 0.02 } else { 0.0 }));
    p.rect_filled(fill, 3.0, col.gamma_multiply(0.45));
    let stroke = if m.level() == MeterLevel::Ok {
        Stroke::new(1.0, theme.gutter_fg.gamma_multiply(0.4))
    } else {
        Stroke::new(1.5, col)
    };
    p.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font,
        theme.text,
    );
    resp.on_hover_text(match m.level() {
        MeterLevel::Over => format!("{}: over the game limit of {}", m.label, m.limit),
        MeterLevel::Near => format!("{}: near the game limit of {}", m.label, m.limit),
        MeterLevel::Ok => format!("{}: {} used", m.label, m.used),
    });
}

impl ScriptEditor {
    pub(crate) fn snippets_panel(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let lang_id = self.lang.id();
        ui.strong("Snippets");
        ui.add(
            egui::TextEdit::singleline(&mut self.snippet_search)
                .hint_text("search snippets")
                .desired_width(f32::INFINITY),
        );
        ui.separator();
        let q = self.snippet_search.trim().to_ascii_lowercase();
        let all = crate::snippets::for_language(lang_id);
        let mut categories: Vec<&str> = all.iter().map(|s| s.category).collect();
        categories.dedup();
        let selected = self.snippet_selected.clone();
        let detail_h = if selected.is_some() {
            (ui.available_height() * 0.55).max(140.0)
        } else {
            0.0
        };
        let list_h = (ui.available_height() - detail_h - 8.0).max(80.0);
        let mut pick: Option<&'static str> = None;
        let mut insert: Option<&'static crate::snippets::Snippet> = None;
        egui::ScrollArea::vertical()
            .id_salt("snippet-list")
            .max_height(list_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut seen: Vec<&str> = vec![];
                for cat in categories {
                    if seen.contains(&cat) {
                        continue;
                    }
                    seen.push(cat);
                    let entries: Vec<&&crate::snippets::Snippet> = all
                        .iter()
                        .filter(|s| s.category == cat)
                        .filter(|s| {
                            q.is_empty()
                                || s.title.to_ascii_lowercase().contains(&q)
                                || s.description.to_ascii_lowercase().contains(&q)
                        })
                        .collect();
                    if entries.is_empty() {
                        continue;
                    }
                    ui.label(egui::RichText::new(cat).strong().color(theme.accent));
                    for e in entries {
                        let r = ui.selectable_label(
                            selected.as_deref() == Some(e.id),
                            egui::RichText::new(e.title).color(theme.text),
                        );
                        if r.clicked() {
                            pick = Some(e.id);
                        }
                        if r.double_clicked() {
                            insert = Some(e);
                        }
                    }
                }
            });
        if let Some(id) = &selected {
            ui.separator();
            if let Some(snip) = all.iter().find(|s| s.id == id.as_str()).copied() {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.is_read_only(),
                            egui::Button::new(
                                egui::RichText::new("Insert at caret").color(theme.accent),
                            ),
                        )
                        .clicked()
                    {
                        insert = Some(snip);
                    }
                    ui.weak("Tab walks the placeholders");
                });
                ui.label(snip.description);
                if lang_id == "lua"
                    && snip.lua_place() == Some(crate::snippets::LuaPlace::GameStart)
                {
                    ui.weak("Goes into function OnGameStart (created when missing).");
                }
                let text = snip.plain(lang_id).unwrap_or_default();
                egui::ScrollArea::both()
                    .id_salt("snippet-preview")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(text)
                                    .monospace()
                                    .size(self.theme.font_size - 1.0)
                                    .color(theme.text.gamma_multiply(0.85)),
                            )
                            .extend(),
                        );
                    });
            }
        }
        if let Some(p) = pick {
            self.snippet_selected = Some(p.to_string());
        }
        if let Some(s) = insert {
            self.snippet_selected = Some(s.id.to_string());
            self.insert_library_snippet(s);
        }
    }
}
