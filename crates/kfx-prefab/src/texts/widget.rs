//! The translation grid widget (egui, host independent): rows are string ids, columns languages,
//! the pinned source language first; cells are colour coded (missing, outdated, copied,
//! inherited), editable in place with the code-page check; a toolbar for CSV / `.po` exchange,
//! "copy source to missing" and saving; a statistics line per language.
use super::project::*;
use egui::{Color32, RichText, Ui};
use kfx_formats::strings::Lang;

/// Show only some rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowFilter {
    All,
    /// Rows with a missing or outdated cell in a shown language.
    NeedsWork,
}

/// What a host doing its own file handling ([`TranslationGrid::host_io`]) is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridRequest {
    /// Write the changed files (the host backs them up first, then calls
    /// [`TextProject::save`]).
    Save,
    /// Ask for a file name and write [`TextProject::to_csv`] of every column.
    ExportCsv,
    /// Ask for a file and [`TextProject::import_csv`] it.
    ImportCsv,
    /// Ask for a file name and write [`TextProject::to_po`] of this language.
    ExportPo(Lang),
    /// Ask for a file and [`TextProject::import_po`] it into this language.
    ImportPo(Lang),
}

pub struct TranslationGrid {
    pub project: TextProject,
    pub filter: RowFilter,
    pub search: String,
    /// Edit buffers: (row, language) -> text being typed.
    edits: std::collections::HashMap<(usize, Lang), String>,
    pub message: Option<(bool, String)>,
    /// Path field for import / export.
    pub path_text: String,
    pub add_lang: Lang,
    pub row_limit: usize,
    /// The host saves and picks files itself: Save and the exchange buttons only queue
    /// [`GridRequest`]s ([`TranslationGrid::take_requests`]); no path field is shown.
    pub host_io: bool,
    /// Target language of the `.po` buttons.
    pub po_lang: Lang,
    requests: Vec<GridRequest>,
}

fn state_color(s: CellState) -> Option<Color32> {
    match s {
        CellState::Missing => Some(Color32::from_rgb(120, 40, 40)),
        CellState::Outdated => Some(Color32::from_rgb(130, 100, 30)),
        CellState::Copied => Some(Color32::from_rgb(60, 70, 120)),
        CellState::Inherited => Some(Color32::from_rgb(60, 60, 60)),
        _ => None,
    }
}

fn state_label(s: CellState) -> &'static str {
    match s {
        CellState::Source => "source",
        CellState::Ok => "",
        CellState::Copied => "copy",
        CellState::Outdated => "outdated",
        CellState::Missing => "missing",
        CellState::Inherited => "inherited",
    }
}

impl TranslationGrid {
    pub fn new(project: TextProject) -> TranslationGrid {
        TranslationGrid {
            project,
            filter: RowFilter::All,
            search: String::new(),
            edits: Default::default(),
            message: None,
            path_text: String::new(),
            add_lang: Lang::French,
            row_limit: 400,
            host_io: false,
            po_lang: Lang::French,
            requests: vec![],
        }
    }

    /// Requests queued since the last call (host file handling).
    pub fn take_requests(&mut self) -> Vec<GridRequest> {
        std::mem::take(&mut self.requests)
    }

    /// Forget the cell texts being typed (after the host changed the project).
    pub fn clear_edits(&mut self) {
        self.edits.clear();
    }

    /// Whether a cell has typed text that is not in the project yet.
    pub fn is_editing(&self) -> bool {
        !self.edits.is_empty()
    }

    pub fn show(&mut self, ui: &mut Ui) {
        self.toolbar(ui);
        self.stats_line(ui);
        ui.separator();
        self.grid(ui);
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Source");
            let cur = self.project.source();
            egui::ComboBox::from_id_salt("tr-source")
                .selected_text(cur.code())
                .show_ui(ui, |ui| {
                    for l in self.project.langs.clone() {
                        if ui.selectable_label(l == cur, l.code()).clicked() {
                            self.project.set_source(l);
                        }
                    }
                });
            ui.label("Add language");
            egui::ComboBox::from_id_salt("tr-add")
                .selected_text(self.add_lang.code())
                .show_ui(ui, |ui| {
                    for l in Lang::ALL {
                        ui.selectable_value(&mut self.add_lang, l, l.code());
                    }
                });
            if ui.button("Add").clicked() {
                self.project.add_language(self.add_lang);
            }
            ui.separator();
            ui.selectable_value(&mut self.filter, RowFilter::All, "All rows");
            ui.selectable_value(&mut self.filter, RowFilter::NeedsWork, "Missing / outdated");
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("search text or ID")
                    .desired_width(140.0),
            );
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Copy source to missing")
                .on_hover_text(
                    "Fill every missing cell with the source text (shown as 'copy' until edited)",
                )
                .clicked()
            {
                let r = self.project.copy_source_to_missing(None);
                self.message = Some((
                    false,
                    format!(
                        "{} cells filled, {} cannot be encoded, {} have no file to go to",
                        r.copied,
                        r.unencodable.len(),
                        r.no_target.len()
                    ),
                ));
                self.edits.clear();
            }
            if ui
                .add_enabled(self.project.is_dirty(), egui::Button::new("Save"))
                .clicked()
            {
                if self.host_io {
                    self.requests.push(GridRequest::Save);
                } else {
                    self.message = Some(match self.project.save() {
                        Ok(f) => (false, format!("Saved {} file(s).", f.len())),
                        Err(e) => (true, e.to_string()),
                    });
                }
            }
            ui.separator();
            if self.host_io {
                if ui
                    .button("Export CSV…")
                    .on_hover_text("Write every column to a CSV file")
                    .clicked()
                {
                    self.requests.push(GridRequest::ExportCsv);
                }
                if ui
                    .button("Import CSV…")
                    .on_hover_text("Read translations from a CSV file")
                    .clicked()
                {
                    self.requests.push(GridRequest::ImportCsv);
                }
                ui.separator();
                ui.label(".po language");
                let targets: Vec<Lang> = self
                    .project
                    .langs
                    .iter()
                    .copied()
                    .filter(|l| *l != self.project.source())
                    .collect();
                if !targets.contains(&self.po_lang) {
                    if let Some(l) = targets.first() {
                        self.po_lang = *l;
                    }
                }
                egui::ComboBox::from_id_salt("tr-po-lang")
                    .selected_text(self.po_lang.code())
                    .show_ui(ui, |ui| {
                        for l in targets.iter().copied() {
                            ui.selectable_value(&mut self.po_lang, l, l.code());
                        }
                    });
                let can = targets.contains(&self.po_lang);
                if ui
                    .add_enabled(can, egui::Button::new("Export .po…"))
                    .on_hover_text("Write a gettext file of this language for a translator")
                    .clicked()
                {
                    self.requests.push(GridRequest::ExportPo(self.po_lang));
                }
                if ui
                    .add_enabled(can, egui::Button::new("Import .po…"))
                    .on_hover_text("Read a translated gettext file into this language")
                    .clicked()
                {
                    self.requests.push(GridRequest::ImportPo(self.po_lang));
                }
                return;
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.path_text)
                    .hint_text("file path (.csv / .po)")
                    .desired_width(220.0),
            );
            if ui.button("Export CSV").clicked() {
                let langs = self.project.langs.clone();
                let text = self.project.to_csv(&langs);
                self.message = Some(match std::fs::write(self.path_text.trim(), text) {
                    Ok(()) => (false, "Exported.".into()),
                    Err(e) => (true, e.to_string()),
                });
            }
            if ui.button("Import CSV").clicked() {
                self.message = Some(
                    match std::fs::read_to_string(self.path_text.trim())
                        .map_err(|e| e.to_string())
                        .and_then(|t| self.project.import_csv(&t))
                    {
                        Ok(r) => (
                            !r.errors.is_empty(),
                            format!(
                                "{} updated, {} unchanged, {} unknown, {} errors",
                                r.updated.len(),
                                r.unchanged,
                                r.unknown.len(),
                                r.errors.len()
                            ),
                        ),
                        Err(e) => (true, e),
                    },
                );
                self.edits.clear();
            }
        });
        if let Some((err, m)) = &self.message {
            ui.colored_label(
                if *err {
                    Color32::from_rgb(235, 120, 90)
                } else {
                    Color32::from_rgb(90, 200, 110)
                },
                m,
            );
        }
    }

    fn stats_line(&self, ui: &mut Ui) {
        egui::Grid::new("tr-stats").striped(true).show(ui, |ui| {
            for h in [
                "Language",
                "Translated",
                "Copied",
                "Outdated",
                "Missing",
                "Inherited",
                "Not encodable",
            ] {
                ui.label(RichText::new(h).strong().small());
            }
            ui.end_row();
            for &l in &self.project.langs {
                let s = self.project.stats(l);
                ui.label(RichText::new(l.code()).small());
                for n in [
                    s.translated,
                    s.copied,
                    s.outdated,
                    s.missing,
                    s.inherited,
                    s.unencodable,
                ] {
                    ui.label(RichText::new(format!("{n}/{}", s.total)).small());
                }
                ui.end_row();
            }
        });
    }

    fn grid(&mut self, ui: &mut Ui) {
        let langs = self.project.langs.clone();
        let q = self.search.to_lowercase();
        let mut rows: Vec<usize> = (0..self.project.rows.len())
            .filter(|&ri| {
                let r = &self.project.rows[ri];
                let hit = q.is_empty()
                    || r.id.to_string() == q
                    || langs
                        .iter()
                        .any(|&l| self.project.cell(ri, l).text.to_lowercase().contains(&q));
                let work = self.filter == RowFilter::All
                    || langs.iter().any(|&l| {
                        matches!(
                            self.project.cell(ri, l).state,
                            CellState::Missing | CellState::Outdated
                        )
                    });
                hit && work
            })
            .collect();
        let total = rows.len();
        rows.truncate(self.row_limit);
        ui.label(
            RichText::new(format!(
                "{total} row(s){}",
                if total > rows.len() {
                    format!(", first {} shown", rows.len())
                } else {
                    String::new()
                }
            ))
            .weak()
            .small(),
        );
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("tr-grid")
                    .striped(false)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("ID").strong());
                        for &l in &langs {
                            let label = if l == self.project.source() {
                                format!("{} (source)", l.code())
                            } else {
                                l.code().to_string()
                            };
                            ui.label(RichText::new(label).strong());
                        }
                        ui.end_row();
                        for ri in rows {
                            let r = self.project.rows[ri].clone();
                            let hint = r
                                .refs
                                .iter()
                                .map(|x| format!("{}:{} {}", x.file, x.line, x.command))
                                .collect::<Vec<_>>()
                                .join("\n");
                            ui.label(format!(
                                "{}{}",
                                if r.scope == Scope::Campaign { "C " } else { "" },
                                r.id
                            ))
                            .on_hover_text(hint);
                            for &l in &langs {
                                self.cell_ui(ui, ri, l);
                            }
                            ui.end_row();
                        }
                    });
            });
    }

    fn cell_ui(&mut self, ui: &mut Ui, ri: usize, l: Lang) {
        let info = self.project.cell(ri, l);
        let key = (ri, l);
        let mut buf = self
            .edits
            .get(&key)
            .cloned()
            .unwrap_or_else(|| info.text.clone());
        let before = buf.clone();
        let editable = self.project.write_target(ri, l).is_ok();
        let bad = TextProject::check_encodable(l, &buf).err();
        let frame_color = if bad.is_some() {
            Some(Color32::from_rgb(170, 40, 40))
        } else {
            state_color(info.state)
        };
        let mut resp = None;
        egui::Frame::new()
            .fill(frame_color.unwrap_or(Color32::TRANSPARENT))
            .inner_margin(2.0)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(260.0);
                    let te = egui::TextEdit::multiline(&mut buf)
                        .desired_rows(2)
                        .desired_width(260.0)
                        .interactive(editable);
                    resp = Some(ui.add(te));
                    let label = state_label(info.state);
                    if !label.is_empty() || bad.is_some() {
                        let msg = match &bad {
                            Some(b) => format!(
                                "cannot encode: {}",
                                b.iter().map(|(_, c)| *c).collect::<String>()
                            ),
                            None => label.to_string(),
                        };
                        ui.label(RichText::new(msg).small().weak());
                    }
                });
            });
        if buf != before {
            self.edits.insert(key, buf.clone());
        }
        if let Some(r) = resp {
            if r.lost_focus() {
                if let Some(t) = self.edits.remove(&key) {
                    if t != info.text {
                        match self.project.set_text(ri, l, &t) {
                            Ok(()) => {}
                            Err(e) => {
                                self.message = Some((
                                    true,
                                    format!("id {} {}: {e}", self.project.rows[ri].id, l.code()),
                                ));
                                self.edits.insert(key, t);
                            }
                        }
                    }
                }
            }
            if !editable {
                r.on_hover_text(self.project.write_target(ri, l).err().unwrap_or_default());
            }
        }
    }
}
