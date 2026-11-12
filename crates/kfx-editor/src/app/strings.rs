//! Strings of the level (`kfx_formats::strings`): the resolver the game uses for a string id
//! (map strings, campaign strings, translations, GUI strings), a Strings window to search them
//! and edit / add the level's own strings (`mapNNNNN.<lang>.dat`) or the campaign's, and the
//! text of the message a DK script line shows (`DISPLAY_OBJECTIVE` / `DISPLAY_INFORMATION`).
use super::*;
use kfx_formats::strings::{Lang, StringResolver, StringSource, StringTable, STRINGS_MAX};
use kfx_prefab::i18n::{trd, trf};

/// Which file an edited string goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringTarget {
    /// `mapNNNNN.<lang>.dat` next to the map (the game reads it first).
    Level,
    /// The campaign's `[strings]` file for the language.
    Campaign,
}

pub struct StringsState {
    pub resolver: StringResolver,
    pub lang: Lang,
    /// The level's table and where it is (or would be) written.
    pub level: (PathBuf, StringTable),
    /// The campaign's table, when the level belongs to a campaign with strings.
    pub campaign: Option<(PathBuf, StringTable)>,
    pub dirty_level: bool,
    pub dirty_campaign: bool,
    pub search: String,
    pub selected: Option<usize>,
    pub edit: String,
    pub target: StringTarget,
    /// Show the GUI strings too (ids 4000+).
    pub show_gui: bool,
    pub message: Option<String>,
}

/// Commands whose first argument is a string id (`command_add_value` users of
/// `script_message_*`, lvl_script_commands.c).
pub const MESSAGE_COMMANDS: &[&str] = &[
    "DISPLAY_OBJECTIVE",
    "DISPLAY_INFORMATION",
    "DISPLAY_OBJECTIVE_WITH_POS",
    "DISPLAY_INFORMATION_WITH_POS",
    "DISPLAY_MESSAGE",
];

/// The string ids a DK script line passes to a message command: (command, id, byte range of
/// the id in the line).
pub fn message_ids(line: &str) -> Vec<(String, i64, std::ops::Range<usize>)> {
    let mut out = vec![];
    let up = line.to_ascii_uppercase();
    let code: &str = if up.trim_start().starts_with("REM") {
        ""
    } else {
        &up
    };
    for c in MESSAGE_COMMANDS {
        let mut from = 0;
        while let Some(p) = code[from..].find(c) {
            let start = from + p;
            from = start + c.len();
            let rest = &line[from..];
            let Some(open) = rest.find('(') else { continue };
            if !rest[..open].trim().is_empty() {
                continue;
            }
            let arg_start = from + open + 1;
            let arg: String = line[arg_start..]
                .chars()
                .take_while(|ch| *ch != ',' && *ch != ')')
                .collect();
            let lead = arg.len() - arg.trim_start().len();
            if let Ok(n) = arg.trim().parse::<i64>() {
                let s = arg_start + lead;
                out.push((c.to_string(), n, s..s + arg.trim().len()));
            }
        }
    }
    out
}

impl EditorApp {
    /// The resolver files of the open level for the preference language.
    fn load_strings(&self) -> Result<StringsState, String> {
        let d = self.doc.as_ref().ok_or("no map")?;
        let install = self.install().ok_or("no install")?.to_path_buf();
        let lang = Lang::from_code(&self.settings.language).unwrap_or(Lang::English);
        let mod_dirs: Vec<PathBuf> = {
            let mut v: Vec<PathBuf> = vec![];
            for (o, l) in &d.stack.config {
                if o.is_mod() && l.prefix.is_empty() {
                    if let Some(root) = l.dir.parent() {
                        if !v.iter().any(|x| x == root) {
                            v.push(root.to_path_buf());
                        }
                    }
                }
            }
            v
        };
        let campaign = d
            .pack_cfg
            .as_ref()
            .and_then(|(p, _)| kfx_formats::campaign::Campaign::load(p).ok());
        let entries = campaign.as_ref().map(|c| c.strings()).unwrap_or_default();
        let src = kfx_formats::strings::StringSources::resolve(
            &install,
            &mod_dirs,
            &entries,
            Some(&d.doc.folder),
            Some(d.doc.level),
            lang,
        )
        .map_err(|e| e.to_string())?;
        let resolver = StringResolver::load(&src).map_err(|e| e.to_string())?;
        let level_path = src
            .level
            .as_ref()
            .map(|(p, _)| p.clone())
            .unwrap_or_else(|| d.doc.folder.join(lang.map_strings_file(d.doc.level)));
        let level_table = src
            .level
            .as_ref()
            .and_then(|(p, l)| StringTable::load(p, *l).ok())
            .unwrap_or_else(|| StringTable::new(lang));
        // the campaign's file is editable unless it is a base game file (the original campaign
        // uses fxdata/gtext_<lang>.dat itself)
        let fx = kfx_formats::testutil::resolve_ci(&install, "fxdata")
            .and_then(|f| f.canonicalize().ok());
        let in_base = |p: &Path| {
            fx.as_ref()
                .is_some_and(|f| p.canonicalize().is_ok_and(|c| c.starts_with(f)))
        };
        let camp = src
            .campaign
            .first()
            .filter(|(p, _)| !in_base(p))
            .and_then(|(p, l)| StringTable::load(p, *l).ok().map(|t| (p.clone(), t)));
        Ok(StringsState {
            resolver,
            lang,
            level: (level_path, level_table),
            campaign: camp,
            dirty_level: false,
            dirty_campaign: false,
            search: String::new(),
            selected: None,
            edit: String::new(),
            target: StringTarget::Level,
            show_gui: false,
            message: None,
        })
    }

    /// (Re)load the strings of the open level (keeps unsaved edits of the same level).
    pub fn refresh_strings(&mut self) {
        if self
            .strings
            .as_ref()
            .is_some_and(|s| s.dirty_level || s.dirty_campaign)
        {
            return;
        }
        self.strings = self.load_strings().ok();
        self.publish_strings();
    }

    /// Give the script editors the current string texts (hover / completion of string ids,
    /// `kfx_script_ui::StringLookup`).
    pub(super) fn publish_strings(&mut self) {
        let r = self.strings.as_ref().map(|s| s.resolver.clone());
        if let Ok(mut g) = self.string_shared.write() {
            *g = r;
        }
        let shared = self.string_shared.clone();
        if let Some(sc) = self.scripts.as_mut() {
            if !sc.lookup_set {
                let lookup: kfx_script_ui::StringLookup = Arc::new(move |id| {
                    shared
                        .read()
                        .ok()
                        .and_then(|g| g.as_ref().map(|r| r.get(id)))
                        .filter(|t| !t.is_empty())
                });
                sc.ws.set_string_lookup(Some(lookup));
                sc.lookup_set = true;
            }
        }
    }

    /// `ScriptEvent::RequestNewString` of a script editor: a new level string, its id written
    /// where the editor asked, and the Strings window open on it.
    pub(super) fn new_string_for_script(&mut self, tab: &str) {
        match self.new_level_string("New message") {
            Ok(id) => {
                if let Some(ws) = self.script_ws() {
                    ws.insert_string_id(tab, id as i64);
                }
                self.show_strings = true;
                if let Some(s) = self.strings.as_mut() {
                    s.selected = Some(id);
                    s.edit = "New message".into();
                    s.target = StringTarget::Level;
                    s.message = Some(trf(
                        "kfx_editor.string_id_created_for_tab_type_its_text_and_set",
                        "string {id} created for {tab}: type its text and Set, then save",
                        &[("id", &id), ("tab", &tab)],
                    ));
                }
            }
            Err(e) => self.status = e,
        }
    }

    /// Text of string `id` as the game shows it for the open level.
    pub fn string_text(&mut self, id: i64) -> Option<String> {
        if self.strings.is_none() {
            self.refresh_strings();
        }
        self.strings.as_ref().map(|s| s.resolver.get(id))
    }

    /// Set string `id` in the level's (or the campaign's) table; not written until
    /// [`EditorApp::save_strings`].
    pub fn set_string(
        &mut self,
        id: usize,
        text: &str,
        target: StringTarget,
    ) -> Result<(), String> {
        if self.strings.is_none() {
            self.refresh_strings();
        }
        if self.texts_dirty() {
            return Err(trd(
                "kfx_editor.the_translation_grid_has_unsaved_changes_to_the",
                "The translation grid has unsaved changes to the same files: save it first.",
            ));
        }
        let s = self.strings.as_mut().ok_or("no strings")?;
        if id >= STRINGS_MAX {
            return Err(trf(
                "kfx_editor.only_ids_below_strings_max_belong_to_a_level_or",
                "only IDs below {STRINGS_MAX} belong to a level or campaign",
                &[("STRINGS_MAX", &STRINGS_MAX)],
            ));
        }
        match target {
            StringTarget::Level => {
                s.level.1.set(id, text).map_err(|e| e.to_string())?;
                s.resolver
                    .level
                    .slots
                    .resize(s.resolver.level.slots.len().max(id + 1), String::new());
                s.resolver.level.slots[id] = text.to_string();
                s.dirty_level = true;
                self.publish_strings();
                return Ok(());
            }
            StringTarget::Campaign => {
                let (_, t) = s
                    .campaign
                    .as_mut()
                    .ok_or("the level has no campaign string file")?;
                t.set(id, text).map_err(|e| e.to_string())?;
                s.resolver
                    .campaign
                    .slots
                    .resize(s.resolver.campaign.slots.len().max(id + 1), String::new());
                s.resolver.campaign.slots[id] = text.to_string();
                s.dirty_campaign = true;
            }
        }
        self.publish_strings();
        Ok(())
    }

    /// A new level string (the first id free in the level, campaign and GUI fallbacks are not
    /// considered free); returns its id.
    pub fn new_level_string(&mut self, text: &str) -> Result<usize, String> {
        if self.strings.is_none() {
            self.refresh_strings();
        }
        let s = self.strings.as_ref().ok_or("no strings")?;
        // an id that shows nothing yet (no level, campaign or GUI fallback text), so the new
        // text hides no message the level could already use
        let id = (1..STRINGS_MAX)
            .find(|&i| s.resolver.get(i as i64).trim().is_empty())
            .ok_or("no free string id")?;
        self.set_string(id, text, StringTarget::Level)?;
        Ok(id)
    }

    /// Write the edited tables. Each file is backed up before its first overwrite in the
    /// session (a failed backup leaves it unwritten), and written crash-safely.
    pub fn save_strings(&mut self) -> Result<Vec<PathBuf>, String> {
        // the translation grid's changes (same backups and crash-safe writes)
        let grid = self.save_texts()?;
        let backups = self.session_backups.clone();
        let Some(s) = self.strings.as_mut() else {
            return Ok(grid);
        };
        let mut out = vec![];
        if s.dirty_level {
            backups.ensure(&s.level.0)?;
            s.level
                .1
                .save(&s.level.0)
                .map_err(|e| format!("{}: {e}", s.level.0.display()))?;
            s.dirty_level = false;
            out.push(s.level.0.clone());
        }
        if s.dirty_campaign {
            if let Some((p, t)) = &s.campaign {
                backups.ensure(p)?;
                t.save(p).map_err(|e| format!("{}: {e}", p.display()))?;
                out.push(p.clone());
            }
            s.dirty_campaign = false;
        }
        self.refresh_fingerprint_for(&out);
        if !out.is_empty() {
            self.texts_reload();
        }
        out.extend(grid);
        Ok(out)
    }

    pub fn strings_dirty(&self) -> bool {
        self.strings
            .as_ref()
            .is_some_and(|s| s.dirty_level || s.dirty_campaign)
            || self.texts_dirty()
    }

    /// Create a new level string and insert its id at the script caret.
    pub fn insert_new_string(&mut self, text: &str) -> Result<usize, String> {
        let id = self.new_level_string(text)?;
        let ws = self.script_ws().ok_or("no scripts")?;
        let e = ws.active_editor_mut();
        e.insert_at_caret(&id.to_string());
        e.apply_pending();
        self.status = trf(
            "kfx_editor.string_id_created_save_it_from_the_strings_windo",
            "String {id} created (save it from the Strings window) and inserted",
            &[("id", &id)],
        );
        Ok(id)
    }

    /// The message ids of the script line at the caret with their text.
    pub fn caret_messages(&mut self) -> Vec<(String, i64, String)> {
        let Some(s) = self.scripts.as_mut() else {
            return vec![];
        };
        if !s.ws.active_name().to_ascii_lowercase().ends_with(".txt") {
            return vec![];
        }
        let e = s.ws.active_editor_mut();
        let bytes = e.text_bytes();
        let off = e.caret_file_offset().min(bytes.len());
        let start = bytes[..off]
            .iter()
            .rposition(|&c| c == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let end = bytes[off..]
            .iter()
            .position(|&c| c == b'\n')
            .map(|i| off + i)
            .unwrap_or(bytes.len());
        let line = kfx_formats::cfg::latin1_decode(&bytes[start..end]);
        let ids = message_ids(&line);
        ids.into_iter()
            .map(|(c, id, _)| (c, id, self.string_text(id).unwrap_or_default()))
            .collect()
    }

    /// Under the script header: the text of the messages on the caret line.
    pub(super) fn script_strings_bar(&mut self, ui: &mut egui::Ui) {
        let msgs = self.caret_messages();
        if msgs.is_empty() {
            return;
        }
        let mut open = false;
        ui.horizontal_wrapped(|ui| {
            for (c, id, text) in &msgs {
                ui.label(RichText::new(format!("{c} {id}:")).small().color(GOLD));
                ui.label(
                    RichText::new(trf(
                        "kfx_editor.text_4b95",
                        "\u{201C}{text}\u{201D}",
                        &[("text", &text)],
                    ))
                    .small(),
                );
            }
            if ui
                .small_button(trd("kfx_editor.strings_11f7", "Strings…"))
                .on_hover_text(trd(
                    "kfx_editor.edit_the_level_s_strings",
                    "Edit the level's strings",
                ))
                .clicked()
            {
                open = true;
            }
        });
        if open {
            self.show_strings = true;
            if let (Some(s), Some((_, id, _))) = (self.strings.as_mut(), msgs.first()) {
                s.selected = Some(*id as usize);
                s.edit = s.resolver.get(*id);
            }
        }
    }

    pub(super) fn strings_window(&mut self, ctx: &egui::Context) {
        if !self.show_strings {
            return;
        }
        if self.strings.is_none() {
            self.refresh_strings();
        }
        let mut open = true;
        let mut act: Option<&'static str> = None;
        let has_scripts = self.scripts.is_some();
        egui::Window::new(trd("kfx_editor.strings", "Strings"))
            .open(&mut open)
            .default_size([760.0, 560.0])
            .max_width(900.0)
            .show(ctx, |ui| {
                let Some(s) = self.strings.as_mut() else {
                    ui.label(
                        RichText::new(trd(
                            "kfx_editor.open_a_map_to_see_its_strings",
                            "Open a map to see its strings",
                        ))
                        .color(TEXT_DIM),
                    );
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(trf(
                        "kfx_editor.language_lang",
                        "Language {lang}",
                        &[("lang", &s.lang.code())],
                    ));
                    ui.label(
                        RichText::new(trf(
                            "kfx_editor.level_file_level",
                            "level file: {level}",
                            &[("level", &s.level.0.display())],
                        ))
                        .small()
                        .color(TEXT_DIM),
                    );
                });
                match &s.campaign {
                    Some((p, _)) => {
                        ui.label(
                            RichText::new(trf(
                                "kfx_editor.campaign_file_p",
                                "campaign file: {p}",
                                &[("p", &p.display())],
                            ))
                            .small()
                            .color(TEXT_DIM),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new(trd(
                                "kfx_editor.the_campaign_strings_are_base_game_files_not_edi",
                                "the campaign strings are base game files (not edited here): \
                                new texts go to the level file",
                            ))
                            .small()
                            .color(TEXT_DIM),
                        );
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    ui.add(
                        egui::TextEdit::singleline(&mut s.search)
                            .hint_text(trd("kfx_editor.text_or_id", "text or ID"))
                            .desired_width(260.0),
                    );
                    ui.checkbox(
                        &mut s.show_gui,
                        trd("kfx_editor.gui_strings_too", "GUI strings too"),
                    );
                    if ui
                        .add_enabled(
                            s.dirty_level || s.dirty_campaign,
                            egui::Button::new(trd("kfx_editor.save_strings", "💾 Save strings")),
                        )
                        .clicked()
                    {
                        act = Some("save");
                    }
                    if ui
                        .button(trd(
                            "kfx_editor.new_level_string_248d",
                            "➕ New level string",
                        ))
                        .clicked()
                    {
                        act = Some("new");
                    }
                    if ui
                        .button(trd("kfx_editor.translations", "Translations…"))
                        .on_hover_text(trd(
                            "kfx_editor.every_text_of_this_level_in_every_language_side",
                            "Every text of this level in every language side by side, with \
                            CSV and .po exchange",
                        ))
                        .clicked()
                    {
                        act = Some("texts");
                    }
                });
                if let Some(m) = &s.message {
                    ui.label(RichText::new(m).small().color(GOLD));
                }
                ui.separator();
                let q = s.search.trim().to_lowercase();
                let r = &s.resolver;
                let mut rows: Vec<(usize, String, StringSource)> = vec![];
                for id in 0..STRINGS_MAX {
                    let src = r.source_of(id as i64);
                    if src == StringSource::Gui && !s.show_gui {
                        continue;
                    }
                    let t = r.get(id as i64);
                    if t.is_empty() {
                        continue;
                    }
                    if !q.is_empty() && !t.to_lowercase().contains(&q) && id.to_string() != q {
                        continue;
                    }
                    rows.push((id, t, src));
                }
                if s.show_gui {
                    for i in 0..r.gui.slots.len() {
                        let id = kfx_formats::strings::GUI_STRINGS_START + i;
                        let t = r.gui.get(i).to_string();
                        if t.is_empty()
                            || (!q.is_empty()
                                && !t.to_lowercase().contains(&q)
                                && id.to_string() != q)
                        {
                            continue;
                        }
                        rows.push((id, t, StringSource::Gui));
                    }
                }
                let h = (ui.available_height() - 110.0).max(120.0);
                let mut pick = None;
                egui::ScrollArea::vertical()
                    .max_height(h)
                    .auto_shrink([false, false])
                    .show_rows(ui, 18.0, rows.len(), |ui, range| {
                        for (id, t, src) in &rows[range] {
                            ui.horizontal(|ui| {
                                let sel = s.selected == Some(*id);
                                if ui
                                    .add(egui::Button::selectable(
                                        sel,
                                        RichText::new(format!("{id:>5}")).monospace(),
                                    ))
                                    .clicked()
                                {
                                    pick = Some(*id);
                                }
                                ui.label(
                                    RichText::new(match src {
                                        StringSource::Level => trd("kfx_editor.map_2d91", "map"),
                                        StringSource::Campaign => {
                                            trd("kfx_editor.campaign_5b53", "campaign")
                                        }
                                        StringSource::Translation => {
                                            trd("kfx_editor.translation", "translation")
                                        }
                                        StringSource::Gui => trd("kfx_editor.gui", "GUI"),
                                        StringSource::Invalid => "?".into(),
                                    })
                                    .small()
                                    .color(TEXT_DIM),
                                );
                                ui.add(egui::Label::new(t.replace('\n', " ")).truncate())
                                    .on_hover_text(t);
                            });
                        }
                    });
                if let Some(id) = pick {
                    s.selected = Some(id);
                    s.edit = s.resolver.get(id as i64);
                    s.target = if s.resolver.source_of(id as i64) == StringSource::Campaign
                        && s.campaign.is_some()
                    {
                        StringTarget::Campaign
                    } else {
                        StringTarget::Level
                    };
                }
                ui.separator();
                if let Some(id) = s.selected {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(trf(
                                "kfx_editor.string_id",
                                "String {id}",
                                &[("id", &id)],
                            ))
                            .strong(),
                        );
                        ui.selectable_value(
                            &mut s.target,
                            StringTarget::Level,
                            trd("kfx_editor.this_map", "this map"),
                        )
                        .on_hover_text(trd(
                            "kfx_editor.mapnnnnn_lang_dat_read_before_the_campaign_strin",
                            "mapNNNNN.<lang>.dat: read before the campaign strings",
                        ));
                        ui.add_enabled_ui(s.campaign.is_some(), |ui| {
                            ui.selectable_value(
                                &mut s.target,
                                StringTarget::Campaign,
                                trd("kfx_editor.campaign_file", "campaign file"),
                            );
                        });
                    });
                    ui.add(
                        egui::TextEdit::multiline(&mut s.edit)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                id < STRINGS_MAX,
                                egui::Button::new(trd("kfx_editor.set", "Set")),
                            )
                            .clicked()
                        {
                            act = Some("set");
                        }
                        if ui
                            .add_enabled(
                                has_scripts,
                                egui::Button::new(trd(
                                    "kfx_editor.insert_id_in_the_script",
                                    "Insert ID in the script",
                                )),
                            )
                            .clicked()
                        {
                            act = Some("insert");
                        }
                    });
                }
            });
        self.show_strings = open;
        match act {
            Some("save") => match self.save_strings() {
                Ok(v) => {
                    self.status = trf(
                        "kfx_editor.strings_saved",
                        "Strings saved: {_}",
                        &[(
                            "_",
                            &(v.iter()
                                .map(|p| p.display().to_string())
                                .collect::<Vec<_>>()
                                .join(", ")),
                        )],
                    )
                }
                Err(e) => self.status = e,
            },
            Some("new") => match self.new_level_string("New message") {
                Ok(id) => {
                    if let Some(s) = self.strings.as_mut() {
                        s.selected = Some(id);
                        s.edit = "New message".into();
                        s.target = StringTarget::Level;
                        s.search.clear();
                        s.message = Some(trf(
                            "kfx_editor.string_id_added_to_the_level_not_saved_yet",
                            "string {id} added to the level (not saved yet)",
                            &[("id", &id)],
                        ));
                    }
                }
                Err(e) => self.status = e,
            },
            Some("set") => {
                if let Some((id, text, target)) = self
                    .strings
                    .as_ref()
                    .and_then(|s| Some((s.selected?, s.edit.clone(), s.target)))
                {
                    if let Err(e) = self.set_string(id, &text, target) {
                        self.status = e;
                    }
                }
            }
            Some("texts") => {
                if let Err(e) = self.open_texts(false) {
                    self.status = e.clone();
                    if let Some(s) = self.strings.as_mut() {
                        s.message = Some(e);
                    }
                }
            }
            Some("insert") => {
                if let Some(id) = self.strings.as_ref().and_then(|s| s.selected) {
                    if let Some(ws) = self.script_ws() {
                        let e = ws.active_editor_mut();
                        e.insert_at_caret(&id.to_string());
                        e.apply_pending();
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_ids_of_lines() {
        let v = message_ids("    DISPLAY_OBJECTIVE(12, PLAYER0)");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].1, 12);
        assert_eq!(&"    DISPLAY_OBJECTIVE(12, PLAYER0)"[v[0].2.clone()], "12");
        assert_eq!(
            message_ids("DISPLAY_INFORMATION_WITH_POS( 7 ,10,20)")[0].1,
            7
        );
        assert!(message_ids("REM DISPLAY_OBJECTIVE(3,ALL_PLAYERS)").is_empty());
        assert!(message_ids("SET_FLAG(PLAYER0,FLAG0,1)").is_empty());
    }
}
