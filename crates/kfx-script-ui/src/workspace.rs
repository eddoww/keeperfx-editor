//! [`ScriptWorkspace`]: the script editing surface for one level.
//!
//! Tabs for the level's DK script (`mapNNNNN.txt`) and Lua script (`mapNNNNN.lua`), plus read-only
//! tabs for library Lua files opened through go to definition. The workspace does no file I/O: the
//! host passes a [`ScriptFiles`] implementation to the calls that need bytes.
//!
//! * dirty state per tab, save (Ctrl+S) and save all;
//! * a missing file shows a "create" panel (default DK script, Lua script from the template, the
//!   latter optionally with conversion hints taken from the DK script);
//! * a problems panel combining the diagnostics of all tabs;
//! * [`ScriptWorkspace::file_changed_on_disk`]: the host reports a changed file; a clean tab
//!   reloads silently, a modified one asks;
//! * [`ScriptWorkspace::map_references`] unifies the map references of both scripts.
//! * host tabs ([`ScriptWorkspace::set_host_tab`]): further views of a level file that the host
//!   draws itself (the map editor's visual Events editor of the DK script), shown after the file
//!   tabs.
use crate::editor::{ScriptEditor, ScriptEvent, SidePanel};
use crate::lang::{MapReference, PickKind, PickValue};
use crate::lua::LuaLanguage;
use egui::{Color32, FontId, Frame, Margin, RichText, TextFormat, Ui};
use kfx_script::lua::template::{new_script_with_hints, TemplateOptions};
use kfx_script::lua::ApiIndex;
use kfx_script::{ConstantProvider, Diagnostic, Document, Severity, Span};
use std::sync::Arc;

/// File access supplied by the host. Names are logical (`map00007.lua`); the host maps them to
/// paths. Nothing here may block for long.
pub trait ScriptFiles {
    /// Bytes of a level file, `None` when it does not exist.
    fn read(&self, name: &str) -> Option<Vec<u8>>;
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String>;
    /// Names of the script files that exist for the level (informational).
    fn list(&self) -> Vec<String> {
        vec![]
    }
    /// A library Lua file as the Lua API index names it (`bindings/gui.lua` for the stock library,
    /// `label/path.lua` for a [`crate::LibraryRoot`]).
    fn read_library(&self, _name: &str) -> Option<Vec<u8>> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    DkScript,
    LuaScript,
    /// A library file, read-only.
    Library,
}

/// What the host hears from the workspace.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceEvent {
    /// An editor event (text changed, pick from map, jump to map) with the tab it came from.
    Editor {
        tab: String,
        event: ScriptEvent,
    },
    Saved {
        tab: String,
    },
    SaveFailed {
        tab: String,
        error: String,
    },
    /// A file was created in the editor (still unsaved).
    Created {
        tab: String,
    },
    /// A library tab opened.
    Opened {
        tab: String,
    },
    /// A file could not be opened.
    OpenFailed {
        name: String,
    },
}

/// Result of [`ScriptWorkspace::file_changed_on_disk`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalChange {
    /// Same bytes as the tab already has, or an unknown file.
    Unchanged,
    /// The tab was not modified and now shows the new content.
    Reloaded,
    /// The tab has unsaved edits; the workspace asks the user (see
    /// [`ScriptWorkspace::pending_external`]).
    Conflict,
}

#[derive(Clone, Debug)]
pub struct TabInfo {
    pub name: String,
    pub kind: TabKind,
    pub exists: bool,
    pub dirty: bool,
    pub read_only: bool,
    pub errors: usize,
    pub warnings: usize,
}

/// A map reference with the file it is in.
#[derive(Clone, Debug, PartialEq)]
pub struct TabMapReference {
    pub tab: String,
    pub reference: MapReference,
}

/// One row of the combined problems list.
#[derive(Clone, Debug)]
pub struct WorkspaceProblem {
    pub tab: String,
    pub diagnostic: Diagnostic,
}

struct Tab {
    name: String,
    kind: TabKind,
    editor: ScriptEditor,
    /// The file exists on disk (or was created and saved).
    exists: bool,
    /// Created in the editor, never written.
    unsaved_new: bool,
    /// What the file contained when last read or written.
    disk: Option<Vec<u8>>,
    pending_external: Option<Vec<u8>>,
    /// Options of the create panel.
    hints: bool,
    examples: bool,
}

impl Tab {
    fn dirty(&self) -> bool {
        !self.editor.is_read_only() && (self.unsaved_new || self.editor.is_dirty())
    }
}

pub struct ScriptWorkspace {
    map_number: u32,
    tabs: Vec<Tab>,
    active: usize,
    lua: LuaLanguage,
    problems_open: bool,
    problems_filter: [bool; 3],
    font_size: f32,
    light: bool,
    notice: Option<String>,
    events: Vec<WorkspaceEvent>,
    /// Tab to activate and span to select once it is shown.
    reveal: Option<(String, Span)>,
    strings: Option<crate::StringLookup>,
    sounds: Option<crate::SoundLookup>,
    /// The script wizard window: target tab and form.
    wizard: Option<(String, crate::wizard::ScriptWizard)>,
    /// Editor generation the wizard last saw.
    wizard_gen: u64,
    /// Tabs drawn by the host (see [`ScriptWorkspace::set_host_tab`]) and the one shown.
    host_tabs: Vec<HostTab>,
    host_active: Option<usize>,
    /// Where the host draws the active host tab (this frame).
    host_rect: Option<egui::Rect>,
}

/// A tab whose content the host draws: another view of one of the level files (a visual
/// editor of the DK script, ...). It sits after the file tabs; while it is active the file tab
/// it belongs to counts as the active tab (save, wizard, problems) and
/// [`ScriptWorkspace::show`] leaves the central area to the host.
#[derive(Clone, Debug)]
struct HostTab {
    label: String,
    file: String,
    hover: String,
}

pub fn dk_file_name(map_number: u32) -> String {
    format!("map{map_number:05}.txt")
}

pub fn lua_file_name(map_number: u32) -> String {
    kfx_script::lua::template::script_file_name(map_number)
}

/// Text of a new DK script: the setup and win/lose snippets with their default values.
pub fn default_dk_script(map_number: u32) -> String {
    use crate::snippets::LIBRARY;
    let part = |id: &str| {
        LIBRARY
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.plain("dk-script"))
            .unwrap_or_default()
    };
    format!(
        "REM {}: level script\nLEVEL_VERSION(1)\n\n{}\n\n{}\n",
        dk_file_name(map_number),
        part("setup"),
        part("win-lose")
    )
}

impl ScriptWorkspace {
    pub fn new(map_number: u32) -> ScriptWorkspace {
        let lua = LuaLanguage::new();
        let mk = |name: String, kind: TabKind, lang: Box<dyn crate::lang::Language>| {
            let mut editor = ScriptEditor::with_language(lang).with_id(("kfx-ws", name.clone()));
            editor.set_problems_panel(false);
            editor.options.show_problems_button = false;
            editor.set_side_panel(SidePanel::Outline);
            Tab {
                name,
                kind,
                editor,
                exists: false,
                unsaved_new: false,
                disk: None,
                pending_external: None,
                hints: true,
                examples: true,
            }
        };
        let tabs = vec![
            mk(
                dk_file_name(map_number),
                TabKind::DkScript,
                Box::new(crate::dk::DkLanguage),
            ),
            mk(
                lua_file_name(map_number),
                TabKind::LuaScript,
                Box::new(lua.clone()),
            ),
        ];
        ScriptWorkspace {
            map_number,
            tabs,
            active: 0,
            lua,
            problems_open: true,
            problems_filter: [true; 3],
            font_size: 14.0,
            light: false,
            notice: None,
            events: vec![],
            reveal: None,
            strings: None,
            sounds: None,
            wizard: None,
            wizard_gen: u64::MAX,
            host_tabs: vec![],
            host_active: None,
            host_rect: None,
        }
    }

    pub fn map_number(&self) -> u32 {
        self.map_number
    }

    pub fn dk_name(&self) -> String {
        dk_file_name(self.map_number)
    }

    pub fn lua_name(&self) -> String {
        lua_file_name(self.map_number)
    }

    // ---------------------------------------------------------------------------------------
    // Loading and saving

    /// Read the level's DK and Lua scripts from the host. Open library tabs are kept.
    pub fn load(&mut self, files: &dyn ScriptFiles) {
        for i in 0..self.tabs.len() {
            if self.tabs[i].kind == TabKind::Library {
                continue;
            }
            let name = self.tabs[i].name.clone();
            let bytes = files.read(&name);
            self.set_tab_from_disk(i, bytes);
        }
    }

    fn set_tab_from_disk(&mut self, i: usize, bytes: Option<Vec<u8>>) {
        let t = &mut self.tabs[i];
        match bytes {
            Some(b) => {
                t.editor.set_text_bytes(&b);
                t.exists = true;
                t.disk = Some(b);
            }
            None => {
                t.editor.set_text_bytes(b"");
                t.exists = false;
                t.disk = None;
            }
        }
        t.unsaved_new = false;
        t.pending_external = None;
    }

    pub fn is_dirty(&self) -> bool {
        self.tabs.iter().any(|t| t.dirty())
    }

    pub fn dirty_tabs(&self) -> Vec<String> {
        self.tabs
            .iter()
            .filter(|t| t.dirty())
            .map(|t| t.name.clone())
            .collect()
    }

    /// Write one tab through the host. Library tabs and missing files are not written.
    pub fn save(&mut self, name: &str, files: &dyn ScriptFiles) -> Result<(), String> {
        let Some(i) = self.index_of(name) else {
            return Err(format!("no tab {name}"));
        };
        let t = &mut self.tabs[i];
        if t.editor.is_read_only() || !t.exists {
            return Err(format!("{name} cannot be saved"));
        }
        let bytes = t.editor.text_bytes();
        match files.write(name, &bytes) {
            Ok(()) => {
                t.editor.mark_saved();
                t.disk = Some(bytes);
                t.unsaved_new = false;
                t.pending_external = None;
                self.events.push(WorkspaceEvent::Saved {
                    tab: name.to_string(),
                });
                Ok(())
            }
            Err(e) => {
                self.events.push(WorkspaceEvent::SaveFailed {
                    tab: name.to_string(),
                    error: e.clone(),
                });
                Err(e)
            }
        }
    }

    /// Save every modified tab; returns the names that failed.
    pub fn save_all(&mut self, files: &dyn ScriptFiles) -> Vec<String> {
        let mut failed = vec![];
        for name in self.dirty_tabs() {
            if self.save(&name, files).is_err() {
                failed.push(name);
            }
        }
        failed
    }

    /// The host noticed that `name` changed on disk (or was removed). A tab without edits shows
    /// the new content at once; a modified one keeps its text and asks the user.
    pub fn file_changed_on_disk(&mut self, name: &str, files: &dyn ScriptFiles) -> ExternalChange {
        let Some(i) = self.index_of(name) else {
            return ExternalChange::Unchanged;
        };
        let bytes = if self.tabs[i].kind == TabKind::Library {
            files.read_library(name)
        } else {
            files.read(name)
        };
        if bytes == self.tabs[i].disk {
            return ExternalChange::Unchanged;
        }
        if !self.tabs[i].dirty() {
            self.set_tab_from_disk(i, bytes);
            return ExternalChange::Reloaded;
        }
        // keep the user's text; remember what is on disk now
        self.tabs[i].pending_external = Some(bytes.unwrap_or_default());
        ExternalChange::Conflict
    }

    /// Bytes waiting in a conflict raised by [`ScriptWorkspace::file_changed_on_disk`].
    pub fn pending_external(&self, name: &str) -> Option<&[u8]> {
        self.index_of(name)
            .and_then(|i| self.tabs[i].pending_external.as_deref())
    }

    /// Answer a conflict: take the disk content (`true`) or keep the edits (`false`; the tab
    /// then counts the disk content as its base, so saving overwrites it knowingly).
    pub fn resolve_external(&mut self, name: &str, reload: bool) {
        let Some(i) = self.index_of(name) else { return };
        let Some(b) = self.tabs[i].pending_external.take() else {
            return;
        };
        if reload {
            self.set_tab_from_disk(i, Some(b));
        } else {
            self.tabs[i].disk = Some(b);
        }
    }

    // ---------------------------------------------------------------------------------------
    // Tabs

    fn index_of(&self, name: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.name == name)
    }

    pub fn tab_infos(&self) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .map(|t| {
                let (errors, warnings, _) = t.editor.diagnostic_counts();
                TabInfo {
                    name: t.name.clone(),
                    kind: t.kind,
                    exists: t.exists,
                    dirty: t.dirty(),
                    read_only: t.editor.is_read_only(),
                    errors,
                    warnings,
                }
            })
            .collect()
    }

    pub fn active_name(&self) -> &str {
        &self.tabs[self.active].name
    }

    pub fn activate(&mut self, name: &str) -> bool {
        match self.index_of(name) {
            Some(i) => {
                self.active = i;
                self.host_active = None;
                true
            }
            None => false,
        }
    }

    pub fn editor(&self, name: &str) -> Option<&ScriptEditor> {
        self.index_of(name).map(|i| &self.tabs[i].editor)
    }

    pub fn editor_mut(&mut self, name: &str) -> Option<&mut ScriptEditor> {
        self.index_of(name).map(move |i| &mut self.tabs[i].editor)
    }

    pub fn active_editor_mut(&mut self) -> &mut ScriptEditor {
        &mut self.tabs[self.active].editor
    }

    /// Add (or update) a host-drawn tab `label` showing file tab `file` another way (see
    /// [`HostTab`]); `hover` is its tooltip. Returns true when the tab is new.
    pub fn set_host_tab(&mut self, label: &str, file: &str, hover: &str) -> bool {
        let t = HostTab {
            label: label.to_string(),
            file: file.to_string(),
            hover: hover.to_string(),
        };
        match self.host_tabs.iter_mut().find(|h| h.label == label) {
            Some(h) => {
                *h = t;
                false
            }
            None => {
                self.host_tabs.push(t);
                true
            }
        }
    }

    /// Show host tab `label` (its file tab becomes the active one). False when there is no such
    /// tab.
    pub fn activate_host_tab(&mut self, label: &str) -> bool {
        let Some(j) = self.host_tabs.iter().position(|h| h.label == label) else {
            return false;
        };
        if let Some(i) = self.index_of(&self.host_tabs[j].file) {
            self.active = i;
        }
        self.host_active = Some(j);
        true
    }

    /// The host tab shown, if any. The host draws its content after [`ScriptWorkspace::show`]
    /// in [`ScriptWorkspace::host_rect`], unless its file does not exist yet: then the workspace
    /// shows the create panel of that file as usual.
    pub fn host_tab_active(&self) -> Option<&str> {
        self.host_active
            .and_then(|j| self.host_tabs.get(j))
            .map(|h| h.label.as_str())
    }

    /// The area the host draws the active host tab in, as laid out by the last
    /// [`ScriptWorkspace::show`] (`None` when no host tab content is to be drawn).
    pub fn host_rect(&self) -> Option<egui::Rect> {
        self.host_rect
    }

    /// Close a library tab (level tabs stay).
    pub fn close_tab(&mut self, name: &str) {
        if let Some(i) = self.index_of(name) {
            if self.tabs[i].kind == TabKind::Library {
                self.tabs.remove(i);
                if self.active >= self.tabs.len() || self.active > i {
                    self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
                }
            }
        }
    }

    /// Open a library Lua file read-only (or switch to it) and select `span` (byte offsets of the
    /// file) when given.
    pub fn open_library(
        &mut self,
        name: &str,
        span: Option<Span>,
        files: &dyn ScriptFiles,
    ) -> bool {
        if let Some(i) = self.index_of(name) {
            self.active = i;
            self.host_active = None;
            if let Some(s) = span {
                self.reveal = Some((name.to_string(), s));
            }
            return true;
        }
        let Some(bytes) = files.read_library(name) else {
            self.notice = Some(format!("{name} is not available"));
            self.events.push(WorkspaceEvent::OpenFailed {
                name: name.to_string(),
            });
            return false;
        };
        let mut editor = ScriptEditor::with_language(Box::new(self.lua.clone()))
            .with_id(("kfx-ws", name.to_string()));
        editor.set_problems_panel(false);
        editor.options.show_problems_button = false;
        editor.set_side_panel(SidePanel::Outline);
        editor.set_text_bytes(&bytes);
        editor.set_read_only(true);
        self.apply_view(&mut editor);
        self.tabs.push(Tab {
            name: name.to_string(),
            kind: TabKind::Library,
            editor,
            exists: true,
            unsaved_new: false,
            disk: Some(bytes),
            pending_external: None,
            hints: false,
            examples: false,
        });
        self.active = self.tabs.len() - 1;
        self.host_active = None;
        if let Some(s) = span {
            self.reveal = Some((name.to_string(), s));
        }
        self.events.push(WorkspaceEvent::Opened {
            tab: name.to_string(),
        });
        true
    }

    fn apply_view(&self, e: &mut ScriptEditor) {
        e.set_light(self.light);
        e.set_font_size(self.font_size);
        e.set_string_lookup(self.strings.clone());
        e.set_sound_lookup(self.sounds.clone());
    }

    // ---------------------------------------------------------------------------------------
    // Language, display

    /// Use another Lua API index (stock plus the install's, the campaign's and the map's
    /// libraries): see [`LuaLanguage::index_with_roots`].
    pub fn set_lua_index(&mut self, index: Arc<ApiIndex>) {
        self.lua = LuaLanguage::with_index(index);
        for t in &mut self.tabs {
            if t.kind != TabKind::DkScript {
                t.editor.set_language(Box::new(self.lua.clone()));
                t.editor.invalidate();
            }
        }
    }

    /// Tell the workspace that the host's constants or map changed (re-validates every tab).
    pub fn invalidate(&mut self) {
        for t in &mut self.tabs {
            t.editor.invalidate();
        }
    }

    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = size.clamp(8.0, 32.0);
        for t in &mut self.tabs {
            t.editor.set_font_size(self.font_size);
        }
    }

    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    pub fn set_light(&mut self, light: bool) {
        self.light = light;
        for t in &mut self.tabs {
            t.editor.set_light(light);
            t.editor.set_font_size(self.font_size);
        }
    }

    pub fn is_light(&self) -> bool {
        self.light
    }

    /// Give every tab (also tabs opened later) the host's string lookup: string-id arguments show
    /// their text on hover and list ids with a preview in completion; see
    /// [`ScriptEditor::set_string_lookup`].
    pub fn set_string_lookup(&mut self, lookup: Option<crate::StringLookup>) {
        self.strings = lookup.clone();
        for t in &mut self.tabs {
            t.editor.set_string_lookup(lookup.clone());
        }
    }

    /// Give every tab (also tabs opened later) the host's sound lookup: sound arguments show what
    /// they play on hover, with a "Play" button; see [`ScriptEditor::set_sound_lookup`].
    pub fn set_sound_lookup(&mut self, lookup: Option<crate::SoundLookup>) {
        self.sounds = lookup.clone();
        for t in &mut self.tabs {
            t.editor.set_sound_lookup(lookup.clone());
        }
    }

    /// Answer to [`ScriptEvent::RequestNewString`] (arriving as `WorkspaceEvent::Editor`): writes
    /// the new string's id into the tab that asked.
    pub fn insert_string_id(&mut self, tab: &str, id: i64) {
        if let Some(e) = self.editor_mut(tab) {
            e.insert_string_id(id);
        }
    }

    /// Replace several spans of one tab's file in a single undo step, see
    /// [`ScriptEditor::replace_spans`]. False when the tab does not exist or the spans are invalid.
    pub fn replace_spans(&mut self, tab: &str, edits: &[(Span, String)]) -> bool {
        self.editor_mut(tab)
            .map_or(false, |e| e.replace_spans(edits))
    }

    /// Pass a value picked on the map to the active editor (answer to
    /// [`ScriptEvent::RequestPickFromMap`]).
    pub fn insert_pick(&mut self, value: PickValue) {
        self.tabs[self.active].editor.insert_pick(value);
    }

    // ---------------------------------------------------------------------------------------
    // Cross-tab services

    /// Action points, hero gates and coordinates both level scripts refer to.
    pub fn map_references(&mut self) -> Vec<TabMapReference> {
        let mut out = vec![];
        for t in &mut self.tabs {
            if t.kind == TabKind::Library || !t.exists {
                continue;
            }
            for r in t.editor.map_references() {
                out.push(TabMapReference {
                    tab: t.name.clone(),
                    reference: r,
                });
            }
        }
        out
    }

    /// Diagnostics of all tabs (as of their last validation).
    pub fn problems(&self) -> Vec<WorkspaceProblem> {
        let mut out = vec![];
        for t in &self.tabs {
            if t.kind == TabKind::Library || !t.exists {
                continue;
            }
            for d in t.editor.diagnostics() {
                out.push(WorkspaceProblem {
                    tab: t.name.clone(),
                    diagnostic: d,
                });
            }
        }
        out
    }

    /// Validate every tab now, ignoring the debounce (tests, before saving).
    pub fn analyze_now(&mut self, host: &dyn ConstantProvider) {
        for t in &mut self.tabs {
            if t.exists {
                t.editor.analyze_now(host);
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // Creating files

    /// Put the default DK script into the (missing) DK tab. The file is written by the next save.
    pub fn create_dk_script(&mut self) {
        let text = default_dk_script(self.map_number);
        let name = self.dk_name();
        self.fill_new(&name, text.as_bytes());
    }

    /// Put a new Lua script into the (missing) Lua tab, with conversion hints from the current DK
    /// script as comments when `hints` is set.
    pub fn create_lua_script(&mut self, hints: bool, examples: bool) {
        let opts = TemplateOptions {
            map_number: self.map_number,
            title: None,
            examples,
        };
        let dk_name = self.dk_name();
        let dk = self
            .index_of(&dk_name)
            .filter(|&i| self.tabs[i].exists)
            .map(|i| self.tabs[i].editor.text_bytes());
        let text = match (hints, dk) {
            (true, Some(b)) if !b.is_empty() => {
                new_script_with_hints(&opts, &Document::parse(&b), self.lua.index())
            }
            _ => kfx_script::lua::new_script(&opts),
        };
        let name = self.lua_name();
        self.fill_new(&name, text.as_bytes());
    }

    /// Run the DK-to-Lua assistant over the current text of the DK tab: a **starting point, not a
    /// faithful translation** (see [`kfx_script::lua::convert`]). `None` when the DK script does
    /// not exist. Nothing is changed.
    pub fn convert_dk_to_lua(
        &mut self,
        host: &dyn ConstantProvider,
    ) -> Option<kfx_script::lua::convert::Conversion> {
        let dk_name = self.dk_name();
        let i = self.index_of(&dk_name).filter(|&i| self.tabs[i].exists)?;
        let bytes = self.tabs[i].editor.text_bytes();
        let opts = kfx_script::lua::convert::ConvertOptions {
            source_name: Some(dk_name),
        };
        Some(kfx_script::lua::convert::convert(
            &Document::parse(&bytes),
            self.lua.index(),
            host,
            &opts,
        ))
    }

    /// Put the assistant's Lua draft of the DK script into the (missing) Lua tab, unsaved. Returns
    /// the conversion statistics, `None` when there is no DK script.
    pub fn create_lua_from_dk(
        &mut self,
        host: &dyn ConstantProvider,
    ) -> Option<kfx_script::lua::convert::ConvertStats> {
        let c = self.convert_dk_to_lua(host)?;
        let name = self.lua_name();
        self.fill_new(&name, c.lua.as_bytes());
        Some(c.stats)
    }

    fn fill_new(&mut self, name: &str, bytes: &[u8]) {
        let Some(i) = self.index_of(name) else { return };
        let t = &mut self.tabs[i];
        t.editor.set_text_bytes(bytes);
        t.exists = true;
        t.unsaved_new = true;
        t.disk = None;
        self.active = i;
        self.events.push(WorkspaceEvent::Created {
            tab: name.to_string(),
        });
    }

    // ---------------------------------------------------------------------------------------
    // UI

    /// Show the workspace in `ui` (fills the available space). `consts` feeds validation and
    /// completion with the install's and the map's names; `files` is used when a click needs
    /// bytes (opening a library file, saving).
    pub fn show(
        &mut self,
        ui: &mut Ui,
        consts: &dyn ConstantProvider,
        files: &dyn ScriptFiles,
    ) -> Vec<WorkspaceEvent> {
        let now = ui.input(|i| i.time);
        for t in &mut self.tabs {
            // edits queued on a tab that is not shown (insert_pick, insert_snippet ...) apply now
            t.editor.apply_pending();
            if t.exists {
                t.editor.update(now, consts);
            }
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            let name = self.active_name().to_string();
            let _ = self.save(&name, files);
        }
        let theme = self.tabs[self.active].editor.theme().clone();
        let panel = Frame::new()
            .fill(theme.panel)
            .inner_margin(Margin::symmetric(8, 4));
        let mut switch: Option<usize> = None;
        let mut switch_host: Option<usize> = None;
        let mut close: Option<String> = None;
        let mut save_active = false;
        let mut save_all = false;
        let mut open_wizard: Option<crate::wizard::ScriptLang> = None;
        egui::Panel::top("kfx-ws-tabs").frame(panel).show_separator_line(true).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (i, t) in self.tabs.iter().enumerate() {
                    let (errors, warnings, _) = t.editor.diagnostic_counts();
                    let mut job = egui::text::LayoutJob::default();
                    let fmt = |c: Color32| TextFormat { font_id: FontId::proportional(13.0), color: c, ..Default::default() };
                    job.append(&t.name, 0.0, fmt(if t.exists { theme.text } else { theme.text.gamma_multiply(0.5) }));
                    if t.kind == TabKind::Library {
                        job.append("  (read-only)", 0.0, fmt(theme.gutter_fg));
                    }
                    if t.dirty() {
                        job.append("  *", 0.0, fmt(theme.accent));
                    }
                    if errors > 0 {
                        job.append(&format!("  {errors}"), 0.0, fmt(theme.diag_error));
                    } else if warnings > 0 {
                        job.append(&format!("  {warnings}"), 0.0, fmt(theme.diag_warning));
                    }
                    if !t.exists {
                        job.append("  (missing)", 0.0, fmt(theme.gutter_fg));
                    }
                    if ui.selectable_label(i == self.active && self.host_active.is_none(), job).clicked() {
                        switch = Some(i);
                    }
                    if t.kind == TabKind::Library && ui.small_button("x").on_hover_text("Close").clicked() {
                        close = Some(t.name.clone());
                    }
                }
                for (j, h) in self.host_tabs.iter().enumerate() {
                    let job = RichText::new(&h.label).size(13.0).color(if self.host_active == Some(j) { theme.accent } else { theme.text });
                    if ui.selectable_label(self.host_active == Some(j), job).on_hover_text(&h.hover).clicked() {
                        switch_host = Some(j);
                    }
                }
                ui.separator();
                let active_dirty = self.tabs[self.active].dirty();
                if ui.add_enabled(active_dirty, egui::Button::new("Save")).on_hover_text("Save this file (Ctrl+S)").clicked() {
                    save_active = true;
                }
                if ui.add_enabled(self.is_dirty(), egui::Button::new("Save all")).clicked() {
                    save_all = true;
                }
                let lang = if self.tabs[self.active].kind == TabKind::LuaScript { crate::wizard::ScriptLang::Lua } else { crate::wizard::ScriptLang::Dk };
                if ui.button("Wizard…").on_hover_text("Level setup form: start gold, rooms, powers, traps, \
                    creatures, computer players, alliances, win/lose. Writes one block into the script.").clicked() {
                    open_wizard = Some(lang);
                }
                let (e, w, _) = self.problem_counts();
                let mut open = self.problems_open;
                ui.toggle_value(&mut open, format!("Problems {e}/{w}")).on_hover_text("Errors / warnings over all files");
                self.problems_open = open;
            });
        });
        if let Some(i) = switch {
            self.active = i;
            self.host_active = None;
        }
        if let Some(j) = switch_host {
            let label = self.host_tabs[j].label.clone();
            self.activate_host_tab(&label);
        }
        if let Some(lang) = open_wizard {
            self.open_wizard(lang);
            self.wizard_gen = u64::MAX;
        }
        if let Some(n) = close {
            self.close_tab(&n);
        }
        if save_active {
            let n = self.active_name().to_string();
            let _ = self.save(&n, files);
        }
        if save_all {
            self.save_all(files);
        }

        // notice and external-change bars
        let active_name = self.active_name().to_string();
        if let Some(n) = self.notice.clone() {
            egui::Panel::top("kfx-ws-notice")
                .frame(
                    Frame::new()
                        .fill(theme.panel)
                        .inner_margin(Margin::symmetric(8, 3)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(theme.diag_warning, n);
                        if ui.small_button("Dismiss").clicked() {
                            self.notice = None;
                        }
                    });
                });
        }
        if self.tabs[self.active].pending_external.is_some() {
            let mut answer = None;
            egui::Panel::top("kfx-ws-external")
                .frame(
                    Frame::new()
                        .fill(theme.panel)
                        .inner_margin(Margin::symmetric(8, 3)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            theme.diag_warning,
                            format!("{active_name} changed on disk and has unsaved edits."),
                        );
                        if ui.button("Reload from disk").clicked() {
                            answer = Some(true);
                        }
                        if ui.button("Keep my version").clicked() {
                            answer = Some(false);
                        }
                    });
                });
            if let Some(r) = answer {
                self.resolve_external(&active_name, r);
            }
        }

        if self.problems_open {
            let problems = self.problems();
            let mut jump: Option<WorkspaceProblem> = None;
            egui::Panel::bottom("kfx-ws-problems")
                .frame(
                    Frame::new()
                        .fill(theme.panel)
                        .inner_margin(Margin::symmetric(8, 4)),
                )
                .resizable(true)
                .default_size(130.0)
                .min_size(46.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("Problems in all scripts");
                        let names = ["errors", "warnings", "info"];
                        let cols = [theme.diag_error, theme.diag_warning, theme.diag_info];
                        for k in 0..3 {
                            let n = problems
                                .iter()
                                .filter(|p| sev_index(p.diagnostic.severity) == k)
                                .count();
                            let mut on = self.problems_filter[k];
                            if ui
                                .toggle_value(
                                    &mut on,
                                    RichText::new(format!("{n} {}", names[k])).color(cols[k]),
                                )
                                .changed()
                            {
                                self.problems_filter[k] = on;
                            }
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if problems.is_empty() {
                                ui.weak("No problems found.");
                            }
                            for p in &problems {
                                let k = sev_index(p.diagnostic.severity);
                                if !self.problems_filter[k] {
                                    continue;
                                }
                                let col =
                                    [theme.diag_error, theme.diag_warning, theme.diag_info][k];
                                let mut job = egui::text::LayoutJob::default();
                                job.append(
                                    &format!("{:<14}", p.tab),
                                    0.0,
                                    TextFormat {
                                        font_id: FontId::monospace(12.0),
                                        color: theme.gutter_fg_active,
                                        ..Default::default()
                                    },
                                );
                                job.append(
                                    &format!("{:>5}  ", p.diagnostic.line),
                                    0.0,
                                    TextFormat {
                                        font_id: FontId::monospace(12.0),
                                        color: col,
                                        ..Default::default()
                                    },
                                );
                                job.append(
                                    &p.diagnostic.message,
                                    0.0,
                                    TextFormat {
                                        font_id: FontId::proportional(13.0),
                                        color: theme.text,
                                        ..Default::default()
                                    },
                                );
                                job.append(
                                    &format!("   {}", p.diagnostic.code),
                                    0.0,
                                    TextFormat {
                                        font_id: FontId::monospace(11.0),
                                        color: theme.text.gamma_multiply(0.45),
                                        ..Default::default()
                                    },
                                );
                                if ui.selectable_label(false, job).clicked() {
                                    jump = Some(p.clone());
                                }
                            }
                        });
                });
            if let Some(p) = jump {
                if let Some(i) = self.index_of(&p.tab) {
                    self.active = i;
                    self.host_active = None;
                    self.tabs[i].editor.reveal_span(p.diagnostic.span);
                }
            }
        }

        // the active tab
        let mut create_dk = false;
        let mut create_lua: Option<(bool, bool)> = None;
        let active = self.active;
        let mut editor_events: Vec<ScriptEvent> = vec![];
        // a host tab over an existing file: the host draws the central area
        let host_draws = self.host_active.is_some() && self.tabs[active].exists;
        self.host_rect = None;
        egui::CentralPanel::default()
            .frame(Frame::NONE)
            .show(ui, |ui| {
                if host_draws {
                    self.host_rect = Some(ui.max_rect());
                    return;
                }
                let t = &mut self.tabs[active];
                if t.exists {
                    editor_events = t.editor.show(ui, consts);
                } else {
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, theme.background);
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.label(
                            RichText::new(format!("{} does not exist yet", t.name))
                                .heading()
                                .color(theme.text),
                        );
                        ui.add_space(10.0);
                        match t.kind {
                            TabKind::DkScript => {
                                if ui.button("Create default DK script").clicked() {
                                    create_dk = true;
                                }
                                ui.weak("LEVEL_VERSION, a starting setup and win/lose conditions.");
                            }
                            _ => {
                                ui.checkbox(&mut t.examples, "Include commented examples");
                                ui.checkbox(
                                    &mut t.hints,
                                    "Add the commands of the DK script as porting \
                                        hints (comments)",
                                );
                                if ui.button("Create Lua script from template").clicked() {
                                    create_lua = Some((t.hints, t.examples));
                                }
                                ui.weak(
                                    "An OnGameStart hook, with notes on saves and \
                                        trigger callbacks.",
                                );
                            }
                        }
                    });
                }
            });
        if create_dk {
            self.create_dk_script();
        }
        if let Some((h, e)) = create_lua {
            self.create_lua_script(h, e);
        }
        let tab_name = self.tabs[active].name.clone();
        for ev in editor_events {
            match ev {
                ScriptEvent::GotoDefinition { file, span } => {
                    let lua_name = self.lua_name();
                    if file == lua_name {
                        if let Some(i) = self.index_of(&lua_name) {
                            self.active = i;
                            self.tabs[i].editor.reveal_span(span);
                        }
                    } else {
                        self.open_library(&file, Some(span), files);
                    }
                }
                other => self.events.push(WorkspaceEvent::Editor {
                    tab: tab_name.clone(),
                    event: other,
                }),
            }
        }
        if let Some((name, span)) = self.reveal.take() {
            if let Some(i) = self.index_of(&name) {
                self.tabs[i].editor.reveal_span(span);
                ui.ctx().request_repaint();
            }
        }
        // font size and theme chosen in the editor's toolbar apply to every tab
        let (fs, light) = {
            let e = &self.tabs[self.active].editor;
            (e.font_size(), !e.theme().is_dark)
        };
        if (fs - self.font_size).abs() > f32::EPSILON {
            self.set_font_size(fs);
        }
        if light != self.light {
            self.set_light(light);
        }
        self.show_wizard(ui.ctx(), consts);
        std::mem::take(&mut self.events)
    }

    fn problem_counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for t in &self.tabs {
            if t.kind == TabKind::Library || !t.exists {
                continue;
            }
            let (e, w, i) = t.editor.diagnostic_counts();
            c.0 += e;
            c.1 += w;
            c.2 += i;
        }
        c
    }

    // ---------------------------------------------------------------------------------------
    // Script wizard

    /// Open the script wizard window for the level's DK script ([`ScriptLang::Dk`]) or Lua script.
    /// A missing script is created first (default DK script / Lua template, unsaved). The form
    /// starts from what the script already sets. The window is drawn by [`ScriptWorkspace::show`].
    ///
    /// [`ScriptLang::Dk`]: crate::wizard::ScriptLang::Dk
    pub fn open_wizard(&mut self, lang: crate::wizard::ScriptLang) {
        use crate::wizard::{ScriptLang, ScriptWizard};
        let name = match lang {
            ScriptLang::Dk => self.dk_name(),
            ScriptLang::Lua => self.lua_name(),
        };
        let Some(i) = self.index_of(&name) else {
            return;
        };
        if !self.tabs[i].exists {
            match lang {
                ScriptLang::Dk => self.create_dk_script(),
                ScriptLang::Lua => self.create_lua_script(false, false),
            }
        }
        let bytes = self.tabs[i].editor.text_bytes();
        self.wizard = Some((name, ScriptWizard::new(lang, &bytes)));
    }

    pub fn wizard_open(&self) -> bool {
        self.wizard.is_some()
    }

    pub fn close_wizard(&mut self) {
        self.wizard = None;
    }

    /// The open wizard form (tests, hosts that preset values).
    pub fn wizard_mut(&mut self) -> Option<&mut crate::wizard::ScriptWizard> {
        self.wizard.as_mut().map(|w| &mut w.1)
    }

    /// Write the open wizard's block into its script as one undoable edit and show that tab.
    /// Returns the plan that was applied (with its conflicts), `None` when no wizard is open or
    /// the editor refused the edit (read-only).
    pub fn apply_wizard(&mut self) -> Option<crate::wizard::WizardPlan> {
        let (name, w) = self.wizard.as_mut()?;
        let i = self.tabs.iter().position(|t| &t.name == name)?;
        let ed = &mut self.tabs[i].editor;
        let bytes = ed.text_bytes();
        let plan = w.plan_for(&bytes);
        if !plan.changes(&bytes) {
            return Some(plan);
        }
        if !plan.apply_to_editor(ed) {
            return None;
        }
        w.set_script(&ed.text_bytes());
        self.active = i;
        Some(plan)
    }

    /// Draw the wizard window (from `show`).
    fn show_wizard(&mut self, ctx: &egui::Context, consts: &dyn ConstantProvider) {
        let Some((name, w)) = self.wizard.as_mut() else {
            return;
        };
        let Some(i) = self.tabs.iter().position(|t| &t.name == name) else {
            self.wizard = None;
            return;
        };
        // follow edits made in the editor meanwhile
        let ed = &self.tabs[i].editor;
        if ed.generation() != self.wizard_gen {
            self.wizard_gen = ed.generation();
            w.set_script(&ed.text_bytes());
        }
        let mut open = true;
        let mut event = None;
        egui::Window::new(format!("Script wizard: {name}"))
            .id(egui::Id::unique("kfx-ws-wizard"))
            .default_size([880.0, 580.0])
            .min_size([520.0, 360.0])
            .resizable(true)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                event = w.show(ui, consts);
            });
        match event {
            Some(crate::wizard::WizardEvent::Apply) => {
                self.apply_wizard();
            }
            Some(crate::wizard::WizardEvent::Close) => open = false,
            None => {}
        }
        if !open {
            self.wizard = None;
        }
    }

    /// Events raised outside `show` (saves, created files) that the host has not seen yet.
    pub fn take_events(&mut self) -> Vec<WorkspaceEvent> {
        std::mem::take(&mut self.events)
    }

    /// Message shown above the editor (a file that could not be opened).
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// The kind of the picked value the active editor asks for, if any (convenience for hosts).
    pub fn pick_kind_at_caret(&mut self) -> Option<PickKind> {
        self.tabs[self.active].editor.pick_kind_at_caret()
    }
}

fn sev_index(s: Severity) -> usize {
    match s {
        Severity::Error => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    }
}
