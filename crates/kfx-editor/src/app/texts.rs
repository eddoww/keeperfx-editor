//! The translation grid (`kfx_prefab::texts`) of the open level or its whole campaign, opened
//! from the Strings window: one row per string id the level uses, one column per language,
//! CSV and gettext `.po` exchange through file dialogs, and saving through the editor's string
//! table path (each file backed up once per session before it is overwritten, then written
//! crash-safely by `kfx_prefab`).
//!
//! The Strings window and the grid read the same `mapNNNNN.<lang>.dat` / campaign files: the
//! grid does not open while the Strings window has unsaved edits, the Strings window does not
//! set texts while the grid has unsaved edits, and each reloads after the other saved.
use super::*;
use kfx_formats::strings::Lang;
use kfx_prefab::i18n::{trd, trf, trn};
use kfx_prefab::texts::widget::{GridRequest, TranslationGrid};
use kfx_prefab::texts::{ProjectSpec, TextProject};

pub struct TextsState {
    pub grid: TranslationGrid,
    /// The whole campaign (else the open level only).
    pub campaign: bool,
    pub open: bool,
}

impl EditorApp {
    fn texts_spec(&self, campaign: bool) -> Result<ProjectSpec, String> {
        let d = self.doc.as_ref().ok_or("open a map first")?;
        let install = self.install().ok_or("no install")?.to_path_buf();
        let level = d.doc.level;
        let editor_lang = Lang::from_code(&self.settings.language).unwrap_or(Lang::English);
        let mut spec = match &d.pack_cfg {
            Some((cfg, _)) => {
                let mut s = ProjectSpec::for_campaign(
                    &install,
                    cfg,
                    if campaign { None } else { Some(vec![level]) },
                )
                .map_err(|e| e.to_string())?;
                if !campaign {
                    s.levels_dir = d.doc.folder.clone();
                }
                s
            }
            None => {
                if campaign {
                    return Err(trd(
                        "kfx_editor.this_level_belongs_to_no_campaign_fd8b",
                        "this level belongs to no campaign",
                    ));
                }
                ProjectSpec {
                    install,
                    campaign_cfg: None,
                    campaign_strings: vec![],
                    levels_dir: d.doc.folder.clone(),
                    levels: vec![level],
                    langs: vec![Lang::English],
                    source: Lang::English,
                }
            }
        };
        if !spec.langs.contains(&editor_lang) {
            spec.langs.push(editor_lang);
        }
        Ok(spec)
    }

    /// Open the translation grid of the open level (`campaign`: of its whole campaign).
    pub fn open_texts(&mut self, campaign: bool) -> Result<(), String> {
        if self
            .strings
            .as_ref()
            .is_some_and(|s| s.dirty_level || s.dirty_campaign)
        {
            return Err(trd(
                "kfx_editor.save_the_strings_of_the_strings_window_first_the",
                "Save the strings of the Strings window first: the translation grid \
                writes the same files.",
            ));
        }
        if self
            .texts
            .as_ref()
            .is_some_and(|t| t.grid.project.is_dirty())
        {
            if let Some(t) = self.texts.as_mut() {
                t.open = true;
            }
            return Err(trd(
                "kfx_editor.the_translation_grid_has_unsaved_changes_save_or",
                "The translation grid has unsaved changes: save or discard them first.",
            ));
        }
        let spec = self.texts_spec(campaign)?;
        let project = TextProject::open(spec).map_err(|e| e.to_string())?;
        let mut grid = TranslationGrid::new(project);
        grid.host_io = true;
        self.texts = Some(TextsState {
            grid,
            campaign,
            open: true,
        });
        Ok(())
    }

    pub fn texts_dirty(&self) -> bool {
        self.texts
            .as_ref()
            .is_some_and(|t| t.grid.project.is_dirty())
    }

    pub fn texts_grid(&mut self) -> Option<&mut TranslationGrid> {
        self.texts.as_mut().map(|t| &mut t.grid)
    }

    /// Write the grid's changed files (backed up first), then reload the Strings window.
    pub fn save_texts(&mut self) -> Result<Vec<PathBuf>, String> {
        let backups = self.session_backups.clone();
        let Some(t) = self.texts.as_mut() else {
            return Ok(vec![]);
        };
        if !t.grid.project.is_dirty() {
            return Ok(vec![]);
        }
        for f in t.grid.project.dirty_files() {
            backups.ensure(&f)?;
        }
        let written = t.grid.project.save().map_err(|e| e.to_string())?;
        t.grid.message = Some((
            false,
            trn(
                "kfx_editor.saved_written_file_s",
                written.len() as i64,
                "Saved {n} file.",
                "Saved {n} files.",
                &[],
            ),
        ));
        self.refresh_fingerprint_for(&written);
        if !self
            .strings
            .as_ref()
            .is_some_and(|s| s.dirty_level || s.dirty_campaign)
        {
            self.strings = None;
            self.refresh_strings();
        }
        Ok(written)
    }

    /// After the Strings window saved: the grid reads the files again (when it has no edits).
    pub(super) fn texts_reload(&mut self) {
        let Some(t) = self.texts.as_ref() else { return };
        if t.grid.project.is_dirty() {
            return;
        }
        let (campaign, open) = (t.campaign, t.open);
        if let Ok(spec) = self.texts_spec(campaign) {
            if let Ok(p) = TextProject::open(spec) {
                if let Some(t) = self.texts.as_mut() {
                    let langs = t.grid.project.langs.clone();
                    t.grid.project = p;
                    for l in langs {
                        t.grid.project.add_language(l);
                    }
                    t.grid.clear_edits();
                    t.open = open;
                }
            }
        }
    }

    fn texts_request(&mut self, r: GridRequest) {
        let res: Result<String, String> = (|| match r {
            GridRequest::Save => self.save_texts().map(|w| {
                trn(
                    "kfx_editor.saved_written_file_s",
                    w.len() as i64,
                    "Saved {n} file.",
                    "Saved {n} files.",
                    &[],
                )
            }),
            GridRequest::ExportCsv => {
                let Some(p) = rfd::FileDialog::new()
                    .add_filter(trd("kfx_editor.csv", "CSV"), &["csv"])
                    .set_file_name("texts.csv")
                    .save_file()
                else {
                    return Ok(String::new());
                };
                self.texts_export_csv(&p).map(|_| {
                    trf(
                        "kfx_editor.exported_p",
                        "Exported {p}.",
                        &[("p", &p.display())],
                    )
                })
            }
            GridRequest::ImportCsv => {
                let Some(p) = rfd::FileDialog::new()
                    .add_filter(trd("kfx_editor.csv", "CSV"), &["csv"])
                    .pick_file()
                else {
                    return Ok(String::new());
                };
                self.texts_import(&p, None)
            }
            GridRequest::ExportPo(l) => {
                let name = format!("texts.{}.po", kfx_prefab::texts::po::iso_code(l));
                let Some(p) = rfd::FileDialog::new()
                    .add_filter(trd("kfx_editor.gettext", "gettext"), &["po"])
                    .set_file_name(name)
                    .save_file()
                else {
                    return Ok(String::new());
                };
                self.texts_export_po(l, &p).map(|_| {
                    trf(
                        "kfx_editor.exported_p",
                        "Exported {p}.",
                        &[("p", &p.display())],
                    )
                })
            }
            GridRequest::ImportPo(l) => {
                let Some(p) = rfd::FileDialog::new()
                    .add_filter(trd("kfx_editor.gettext", "gettext"), &["po"])
                    .pick_file()
                else {
                    return Ok(String::new());
                };
                self.texts_import(&p, Some(l))
            }
        })();
        if let Some(t) = self.texts.as_mut() {
            match res {
                Ok(m) if m.is_empty() => {}
                Ok(m) => t.grid.message = Some((false, m)),
                Err(e) => t.grid.message = Some((true, e)),
            }
        }
    }

    pub fn texts_export_csv(&mut self, path: &Path) -> Result<(), String> {
        let t = self
            .texts
            .as_ref()
            .ok_or("the translation grid is not open")?;
        let text = t.grid.project.to_csv(&t.grid.project.langs);
        kfx_formats::atomic::atomic_write(path, text.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn texts_export_po(&mut self, lang: Lang, path: &Path) -> Result<(), String> {
        let t = self
            .texts
            .as_ref()
            .ok_or("the translation grid is not open")?;
        let text = t.grid.project.to_po(lang);
        kfx_formats::atomic::atomic_write(path, text.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Import a CSV file (`lang` None) or a `.po` file of `lang`. Returns the report line.
    pub fn texts_import(&mut self, path: &Path, lang: Option<Lang>) -> Result<String, String> {
        let t = self
            .texts
            .as_mut()
            .ok_or("the translation grid is not open")?;
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let r = match lang {
            Some(l) => {
                t.grid.project.add_language(l);
                t.grid.project.import_po(l, &text)?
            }
            None => t.grid.project.import_csv(&text)?,
        };
        t.grid.clear_edits();
        Ok(trf(
            "kfx_editor.updated_updated_unchanged_unchanged_unknown_unkn",
            "{updated} updated, {unchanged} unchanged, {unknown} unknown, {errors} errors",
            &[
                ("updated", &r.updated.len()),
                ("unchanged", &r.unchanged),
                ("unknown", &r.unknown.len()),
                ("errors", &r.errors.len()),
            ],
        ))
    }

    /// Per frame: the Translations window.
    pub(super) fn texts_window(&mut self, ctx: &egui::Context) {
        let Some(t) = self.texts.as_mut() else { return };
        if !t.open {
            return;
        }
        let has_campaign = self.doc.as_ref().is_some_and(|d| d.pack_cfg.is_some());
        let mut open = true;
        let mut scope: Option<bool> = None;
        let mut discard = false;
        let title = if t.grid.project.is_dirty() {
            trd("kfx_editor.translations_a7c7", "Translations *")
        } else {
            trd("kfx_editor.translations_2fcd", "Translations")
        };
        egui::Window::new(title)
            .id(egui::Id::unique("translations-window"))
            .open(&mut open)
            .default_pos([560.0, 60.0])
            .default_size([900.0, 600.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(!t.campaign, trd("kfx_editor.this_level", "This level"))
                        .clicked()
                        && t.campaign
                    {
                        scope = Some(false);
                    }
                    if ui
                        .add_enabled(
                            has_campaign,
                            egui::Button::selectable(
                                t.campaign,
                                trd("kfx_editor.whole_campaign", "Whole campaign"),
                            ),
                        )
                        .on_disabled_hover_text(trd(
                            "kfx_editor.this_level_belongs_to_no_campaign",
                            "This level belongs to no campaign",
                        ))
                        .clicked()
                        && !t.campaign
                    {
                        scope = Some(true);
                    }
                    ui.label(
                        RichText::new(trf(
                            "kfx_editor.rows_string_ids_ignored_ignored_gui_or_translati",
                            "{rows} string IDs, {ignored} ignored (GUI or translation table)",
                            &[
                                ("rows", &t.grid.project.rows.len()),
                                ("ignored", &t.grid.project.ignored.len()),
                            ],
                        ))
                        .small()
                        .color(TEXT_DIM),
                    );
                    if t.grid.project.is_dirty()
                        && ui
                            .small_button(trd("kfx_editor.discard_changes", "Discard changes"))
                            .clicked()
                    {
                        discard = true;
                    }
                });
                ui.separator();
                t.grid.show(ui);
            });
        let reqs = t.grid.take_requests();
        if !open {
            t.open = false;
        }
        for r in reqs {
            self.texts_request(r);
        }
        if discard {
            if let Some(t) = self.texts.as_mut() {
                let c = t.campaign;
                self.texts = None;
                if let Err(e) = self.open_texts(c) {
                    self.status = e;
                }
            }
        }
        if let Some(c) = scope {
            if self.texts_dirty() {
                self.status = trd(
                    "kfx_editor.save_the_translation_grid_before_changing_its_sc",
                    "Save the translation grid before changing its scope.",
                );
            } else if let Err(e) = self.open_texts(c) {
                self.status = e;
            }
        }
    }
}

/// `--actions` entries of the translation grid.
pub fn run_actions(
    app: &mut EditorApp,
    obj: &serde_json::Map<String, serde_json::Value>,
    rel: &dyn Fn(&str) -> PathBuf,
) -> Result<(), String> {
    let get = |k: &str| obj.get(k);
    let lang = |v: Option<&serde_json::Value>| -> Result<Lang, String> {
        let c = v.and_then(|x| x.as_str()).ok_or("needs \"lang\"")?;
        Lang::from_code(c).ok_or_else(|| format!("unknown language {c}"))
    };
    if let Some(v) = get("texts").and_then(|x| x.as_str()) {
        app.open_texts(v == "campaign")?;
    }
    if let Some(v) = get("texts_add_lang") {
        let l = lang(Some(v))?;
        app.texts_grid()
            .ok_or("open the grid first")?
            .project
            .add_language(l);
    }
    if let Some(v) = get("texts_set").and_then(|x| x.as_object()) {
        let id = v
            .get("id")
            .and_then(|x| x.as_i64())
            .ok_or("texts_set needs id")?;
        let l = lang(v.get("lang"))?;
        let text = v.get("text").and_then(|x| x.as_str()).unwrap_or_default();
        let g = app.texts_grid().ok_or("open the grid first")?;
        g.project.add_language(l);
        let row = g
            .project
            .rows
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| format!("string id {id} is not used by the level"))?;
        g.project
            .set_text(row, l, text)
            .map_err(|e| e.to_string())?;
    }
    if get("texts_save").is_some() {
        let w = app.save_texts()?;
        eprintln!("texts saved: {w:?}");
    }
    if let Some(v) = get("texts_export_csv").and_then(|x| x.as_str()) {
        app.texts_export_csv(&rel(v))?;
    }
    if let Some(v) = get("texts_import_csv").and_then(|x| x.as_str()) {
        eprintln!("texts import: {}", app.texts_import(&rel(v), None)?);
    }
    if let Some(v) = get("texts_export_po").and_then(|x| x.as_object()) {
        let p = v
            .get("path")
            .and_then(|x| x.as_str())
            .ok_or("texts_export_po needs path")?;
        app.texts_export_po(lang(v.get("lang"))?, &rel(p))?;
    }
    if let Some(v) = get("texts_import_po").and_then(|x| x.as_object()) {
        let p = v
            .get("path")
            .and_then(|x| x.as_str())
            .ok_or("texts_import_po needs path")?;
        eprintln!(
            "texts import: {}",
            app.texts_import(&rel(p), Some(lang(v.get("lang"))?))?
        );
    }
    if get("texts_print").is_some() {
        let g = app.texts_grid().ok_or("open the grid first")?;
        let langs = g.project.langs.clone();
        for (i, r) in g.project.rows.iter().enumerate() {
            let cells: Vec<String> = langs
                .iter()
                .map(|&l| {
                    let c = g.project.cell(i, l);
                    format!("{}={:?} ({:?})", l.code(), c.text, c.state)
                })
                .collect();
            eprintln!("  {:?} {}: {}", r.scope, r.id, cells.join(" | "));
        }
    }
    Ok(())
}
