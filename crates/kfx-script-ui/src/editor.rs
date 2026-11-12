//! [`ScriptEditor`]: state, host API and non-visual logic. The egui side is in [`crate::ui`].
use crate::dk::DkLanguage;
use crate::lang::*;
use crate::latin1::{self, Eol, LatinBuffer};
use crate::textops::{self, Edit, FindOptions, LineIndex, Transaction};
use crate::theme::Theme;
use egui::{Id, Pos2, Rect};
use kfx_script::{ConstantProvider, Diagnostic, Severity, Span};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Things the host wants to hear about.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptEvent {
    /// The text changed (any edit, undo, redo, format ...). Not sent for `set_text_bytes`.
    Changed,
    /// The user asked to pick a value on the map; answer with [`ScriptEditor::insert_pick`].
    RequestPickFromMap { kind: PickKind },
    /// The user asked to see a script location on the map.
    JumpToMapLocation(MapTarget),
    /// Go to definition found the name in another file (a library file or another Lua file):
    /// open `file` (as the language's index names it) and select `span` (byte offsets of that
    /// file). Definitions in the same file are followed inside the editor, without an event.
    GotoDefinition { file: String, span: Span },
    /// The user asked for a new string ("New string..." in the completion of a string-id argument,
    /// or [`ScriptEditor::request_new_string`]). `at` (file bytes) is where the id goes. Create the
    /// string, then answer with [`ScriptEditor::insert_string_id`].
    RequestNewString { at: Span },
    /// The user pressed "Play" on the hover card of a sound argument (see [`crate::sounds`]).
    PlaySound(crate::sounds::SoundArg),
}

#[derive(Clone, Debug)]
pub struct EditorOptions {
    /// Text of one indentation level.
    pub indent: String,
    /// Seconds of quiet before validation runs after an edit.
    pub validate_debounce: f64,
    pub show_line_numbers: bool,
    /// Auto-close brackets and quotes (when the language defines pairs).
    pub auto_close: bool,
    /// The toolbar's Problems toggle (a host that shows its own problems list hides it).
    pub show_problems_button: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        EditorOptions {
            indent: "\t".into(),
            validate_debounce: 0.3,
            show_line_numbers: true,
            auto_close: true,
            show_problems_button: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidePanel {
    None,
    Outline,
    Commands,
    /// The shared snippet library.
    Snippets,
}

pub(crate) struct Cache {
    pub gen: u64,
    pub bytes: Vec<u8>,
    pub analysis: Box<dyn Analysis>,
    pub lines: LineIndex,
    refs: std::cell::OnceCell<Vec<MapReference>>,
    outline: std::cell::OnceCell<Vec<OutlineItem>>,
}

impl Cache {
    pub fn new(gen: u64, bytes: Vec<u8>, analysis: Box<dyn Analysis>, lines: LineIndex) -> Cache {
        Cache {
            gen,
            bytes,
            analysis,
            lines,
            refs: Default::default(),
            outline: Default::default(),
        }
    }
    /// Map references in buffer coordinates (computed once per text).
    pub fn map_refs(&self) -> &[MapReference] {
        self.refs.get_or_init(|| self.analysis.map_references())
    }
    pub fn outline(&self) -> &[OutlineItem] {
        self.outline.get_or_init(|| self.analysis.outline())
    }
}

/// Placeholders of an inserted snippet that Tab walks through. Ranges follow the edits made while
/// the session is active.
pub(crate) struct SnippetSession {
    pub stops: Vec<(usize, usize)>,
    pub end: Option<usize>,
    pub prev: Vec<u8>,
    pub gen: u64,
}

pub(crate) struct CompletionState {
    pub items: Vec<Completion>,
    pub selected: usize,
    pub scroll_to_selected: bool,
    /// The user moved the selection with the arrow keys.
    pub navigated: bool,
}

#[derive(Default)]
pub(crate) struct FindState {
    pub open: bool,
    pub replace: bool,
    pub query: String,
    pub replacement: String,
    pub opts: FindOptions,
    pub focus: bool,
    /// Matches for (generation, query, case).
    pub matches: Vec<Span>,
    pub matches_key: Option<(u64, String, bool)>,
    pub current: Option<Span>,
}

#[derive(Default)]
pub(crate) struct GotoState {
    pub open: bool,
    pub text: String,
    pub focus: bool,
}

pub(crate) struct GalleyCache {
    pub gen: u64,
    pub ppp: f32,
    pub font_size: f32,
    pub theme_rev: u64,
    pub galley: Arc<egui::Galley>,
}

pub(crate) struct HoverCard {
    pub span: Span,
    pub diags: Vec<Diagnostic>,
    pub info: Option<HoverInfo>,
    pub pos: Pos2,
    pub rect: Option<Rect>,
    /// The sound argument the card describes (draws a "Play" button).
    pub sound: Option<crate::sounds::SoundArg>,
}

/// Result of the sizing of the text area, needed by overlays.
pub(crate) struct TextGeometry {
    pub galley: Arc<egui::Galley>,
    pub pos: Pos2,
}

pub struct ScriptEditor {
    pub(crate) lang: Box<dyn Language>,
    pub(crate) theme: Theme,
    pub(crate) theme_rev: u64,
    pub options: EditorOptions,
    pub(crate) buf: LatinBuffer,
    pub(crate) generation: Arc<AtomicU64>,
    pub(crate) eol: Eol,
    original: Vec<u8>,
    original_gen: u64,
    saved_hash: u64,
    pub(crate) cache: Option<Cache>,
    pub(crate) validation: Validation,
    pub(crate) validation_gen: Option<u64>,
    pub(crate) validate_due: Option<f64>,
    /// (anchor, head) in buffer coordinates.
    pub(crate) caret: (usize, usize),
    pub(crate) prev_caret: (usize, usize),
    pub(crate) want_sel: Option<(usize, usize)>,
    pub(crate) want_scroll: bool,
    pub(crate) want_focus: bool,
    pub(crate) pending: Vec<Transaction>,
    pub(crate) events: Vec<ScriptEvent>,
    pub(crate) completion: Option<CompletionState>,
    pub(crate) snippet: Option<SnippetSession>,
    pub(crate) new_snippet: Option<(Vec<(usize, usize)>, Option<usize>)>,
    pub(crate) typed_override: Option<char>,
    pub(crate) snippet_search: String,
    pub(crate) snippet_selected: Option<String>,
    read_only: bool,
    pub(crate) completion_armed: bool,
    pub(crate) find: FindState,
    pub(crate) goto: GotoState,
    pub(crate) side: SidePanel,
    pub(crate) problems_open: bool,
    pub(crate) problems_filter: [bool; 3],
    pub(crate) reference: Vec<RefEntry>,
    pub(crate) ref_search: String,
    pub(crate) ref_selected: Option<String>,
    pub(crate) galley_cache: Option<GalleyCache>,
    pub(crate) hover: Option<HoverCard>,
    pub(crate) hover_since: Option<(Span, f64)>,
    pub(crate) text_width: f32,
    pub(crate) geometry: Option<TextGeometry>,
    /// Frames left in which a caret under the line-number gutter scrolls the text back.
    pub(crate) caret_settle: u8,
    pub(crate) id: Id,
    pub(crate) reported_gen: u64,
    pub(crate) clear_undo: bool,
    /// One of our text fields had keyboard focus at the end of the previous frame.
    pub(crate) was_ours_focused: bool,
    pub(crate) suppress_at_caret: bool,
    pub(crate) strings: Option<crate::StringLookup>,
    pub(crate) sounds: Option<crate::SoundLookup>,
    /// (span in buffer coordinates, generation) the answer of a string request goes to.
    pub(crate) new_string_target: Option<(Span, u64)>,
    /// Layer and screen rect of the signature help drawn in the last frame.
    pub(crate) sig_drawn: Option<(egui::LayerId, Rect)>,
}

fn hash_bytes(b: &[u8]) -> u64 {
    // FNV-1a
    let mut h: u64 = 0xcbf29ce484222325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl Default for ScriptEditor {
    fn default() -> Self {
        ScriptEditor::new()
    }
}

impl ScriptEditor {
    /// An empty editor for DK level scripts.
    pub fn new() -> ScriptEditor {
        ScriptEditor::with_language(Box::new(DkLanguage))
    }

    pub fn with_language(lang: Box<dyn Language>) -> ScriptEditor {
        let generation = Arc::new(AtomicU64::new(0));
        let reference = lang.reference();
        let mut e = ScriptEditor {
            lang,
            theme: Theme::default(),
            theme_rev: 0,
            options: EditorOptions::default(),
            buf: LatinBuffer::new(generation.clone()),
            generation,
            eol: Eol::Lf,
            original: vec![],
            original_gen: 0,
            saved_hash: hash_bytes(&[]),
            cache: None,
            validation: Validation::default(),
            validation_gen: None,
            validate_due: None,
            caret: (0, 0),
            prev_caret: (0, 0),
            want_sel: None,
            want_scroll: false,
            want_focus: false,
            pending: vec![],
            events: vec![],
            completion: None,
            snippet: None,
            new_snippet: None,
            typed_override: None,
            snippet_search: String::new(),
            snippet_selected: None,
            read_only: false,
            completion_armed: false,
            find: FindState::default(),
            goto: GotoState::default(),
            side: SidePanel::Outline,
            problems_open: true,
            problems_filter: [true, true, true],
            reference,
            ref_search: String::new(),
            ref_selected: None,
            galley_cache: None,
            hover: None,
            hover_since: None,
            text_width: 0.0,
            geometry: None,
            caret_settle: 0,
            id: Id::unique("kfx-script-editor"),
            reported_gen: 0,
            clear_undo: false,
            was_ours_focused: false,
            suppress_at_caret: true,
            strings: None,
            sounds: None,
            new_string_target: None,
            sig_drawn: None,
        };
        e.original_gen = e.gen();
        e.reported_gen = e.gen();
        e
    }

    /// Use a different widget id (needed when a host shows several editors).
    pub fn with_id(mut self, id: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id = Id::unique(id);
        self
    }

    pub fn language(&self) -> &dyn Language {
        &*self.lang
    }

    /// Switch language (re-analyses the text).
    pub fn set_language(&mut self, lang: Box<dyn Language>) {
        self.reference = lang.reference();
        self.lang = lang;
        self.cache = None;
        self.validation_gen = None;
        self.galley_cache = None;
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.theme_rev += 1;
    }

    pub(crate) fn gen(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    // -----------------------------------------------------------------------------------------
    // Text in and out

    /// Load a script. Undo history, caret and the dirty flag are reset; no `Changed` event.
    pub fn set_text_bytes(&mut self, bytes: &[u8]) {
        self.eol = latin1::detect_eol(bytes);
        self.buf.set_strip_cr(self.eol != Eol::Raw);
        self.buf.set(latin1::to_buffer_text(bytes, self.eol));
        self.original = bytes.to_vec();
        self.original_gen = self.gen();
        self.saved_hash = hash_bytes(bytes);
        self.caret = (0, 0);
        self.prev_caret = (0, 0);
        self.want_sel = Some((0, 0));
        self.pending.clear();
        self.completion = None;
        self.snippet = None;
        self.new_snippet = None;
        self.find.matches_key = None;
        self.find.current = None;
        self.hover = None;
        self.cache = None;
        self.validation_gen = None;
        self.validate_due = None;
        self.validation = Validation::default();
        self.clear_undo = true;
        self.reported_gen = self.gen();
    }

    /// The script as file bytes: byte-identical to what was loaded while untouched; CR LF
    /// endings are restored when the file used them throughout.
    pub fn text_bytes(&self) -> Vec<u8> {
        if self.gen() == self.original_gen {
            return self.original.clone();
        }
        latin1::from_buffer_text(self.buf.text(), self.eol)
    }

    /// The text differs from the last loaded/saved bytes.
    pub fn is_dirty(&self) -> bool {
        if self.gen() == self.original_gen {
            return self.saved_hash != hash_bytes(&self.original);
        }
        hash_bytes(&self.text_bytes()) != self.saved_hash
    }

    /// Call after the host wrote the script to disk.
    pub fn mark_saved(&mut self) {
        let b = self.text_bytes();
        self.saved_hash = hash_bytes(&b);
    }

    /// Increases with every change of the text (typing, undo, set_text_bytes ...). The host can
    /// compare it to re-run its own checks.
    pub fn generation(&self) -> u64 {
        self.gen()
    }

    pub fn line_ending(&self) -> Eol {
        self.eol
    }

    // -----------------------------------------------------------------------------------------
    // Analysis

    pub(crate) fn ensure_cache(&mut self) {
        let gen = self.gen();
        if self.cache.as_ref().map(|c| c.gen) == Some(gen) {
            return;
        }
        let bytes = latin1::encode(self.buf.text());
        let analysis = self.lang.analyze(&bytes);
        let lines = LineIndex::new(&bytes);
        self.cache = Some(Cache::new(gen, bytes, analysis, lines));
    }

    /// Update the analysis for the current text and run the validation when due. Returns true
    /// when a validation ran. `now` is a monotonic clock in seconds.
    pub fn update(&mut self, now: f64, host: &dyn ConstantProvider) -> bool {
        self.ensure_cache();
        let gen = self.gen();
        if self.validation_gen == Some(gen) {
            return false;
        }
        let due = *self
            .validate_due
            .get_or_insert(if self.validation_gen.is_none() {
                now
            } else {
                now + self.options.validate_debounce
            });
        if now >= due {
            self.run_validation(host);
            true
        } else {
            false
        }
    }

    pub(crate) fn run_validation(&mut self, host: &dyn ConstantProvider) {
        self.ensure_cache();
        let c = self.cache.as_ref().unwrap();
        self.validation = c.analysis.validate(host);
        self.validation_gen = Some(c.gen);
        self.validate_due = None;
    }

    /// Parse and validate now, ignoring the debounce (tests, "save" paths).
    pub fn analyze_now(&mut self, host: &dyn ConstantProvider) {
        self.run_validation(host);
    }

    /// Re-run the validation at the next frame (the host's constants or map changed).
    pub fn invalidate(&mut self) {
        self.validation_gen = None;
        self.validate_due = None;
    }

    pub(crate) fn validation_pending(&self) -> bool {
        self.validation_gen != Some(self.gen())
    }

    // ------- coordinate mapping (public spans are file byte offsets) -------

    pub(crate) fn buf_to_file(&self, off: usize) -> usize {
        match (&self.cache, self.eol) {
            (Some(c), Eol::CrLf) => off + c.lines.line_of(off),
            _ => off,
        }
    }

    pub(crate) fn file_to_buf(&self, f: usize) -> usize {
        match (&self.cache, self.eol) {
            (Some(c), Eol::CrLf) => {
                let n = c.lines.line_count();
                // file line starts are buffer starts + k
                let (mut lo, mut hi) = (0usize, n);
                while lo + 1 < hi {
                    let mid = (lo + hi) / 2;
                    if c.lines.line_start(mid) + mid <= f {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let st = c.lines.line_start(lo);
                st + (f - (st + lo)).min(c.lines.line_end(lo) - st)
            }
            _ => f,
        }
    }

    pub(crate) fn span_to_file(&self, s: Span) -> Span {
        Span::new(self.buf_to_file(s.start), self.buf_to_file(s.end))
    }

    pub(crate) fn span_to_buf(&self, s: Span) -> Span {
        Span::new(self.file_to_buf(s.start), self.file_to_buf(s.end))
    }

    /// Diagnostics of the last validation, spans in file bytes. Empty until the first
    /// validation ran (see [`ScriptEditor::update`]).
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.validation
            .diagnostics
            .iter()
            .map(|d| Diagnostic {
                span: self.span_to_file(d.span),
                ..d.clone()
            })
            .collect()
    }

    /// While the editor has focus, value complaints about the token under the caret (unknown
    /// name, unfinished argument ...) are not drawn: the user is still typing it. Default on.
    /// [`ScriptEditor::diagnostics`] and the problems list always report everything.
    pub fn set_suppress_at_caret(&mut self, on: bool) {
        self.suppress_at_caret = on;
    }

    pub fn suppresses_at_caret(&self) -> bool {
        self.suppress_at_caret
    }

    /// Diagnostics as drawn right now (file bytes): [`ScriptEditor::diagnostics`] minus the value
    /// complaints about the token at the caret (`kfx_script::suppress_at_caret`) when suppression
    /// is on, whether or not the editor has focus.
    pub fn visible_diagnostics(&self) -> Vec<Diagnostic> {
        let caret = self.caret.1;
        self.validation
            .diagnostics
            .iter()
            .filter(|d| !(self.suppress_at_caret && kfx_script::hidden_at_caret(d, caret)))
            .map(|d| Diagnostic {
                span: self.span_to_file(d.span),
                ..d.clone()
            })
            .collect()
    }

    /// Whether a squiggle for `d` is drawn (the focused caret is not in its token).
    pub(crate) fn is_shown(&self, d: &Diagnostic) -> bool {
        !(self.suppress_at_caret
            && self.was_ours_focused
            && kfx_script::hidden_at_caret(d, self.caret.1))
    }

    /// Let hover and completion know the text of string ids (message arguments of
    /// `DISPLAY_OBJECTIVE`, `DISPLAY_INFORMATION`, `DISPLAY_MESSAGE`, `SET_BOX_TOOLTIP_ID` and the
    /// Lua `msg_id` parameters). `None` turns it off. See [`crate::strings`].
    pub fn set_string_lookup(&mut self, lookup: Option<crate::StringLookup>) {
        self.strings = lookup;
    }

    pub fn string_lookup(&self) -> Option<&crate::StringLookup> {
        self.strings.as_ref()
    }

    /// Let hover describe sound arguments (`PLAY_MESSAGE`, `SET_MUSIC`) and offer a "Play" button
    /// ([`ScriptEvent::PlaySound`]). `None` turns it off. See [`crate::sounds`].
    pub fn set_sound_lookup(&mut self, lookup: Option<crate::SoundLookup>) {
        self.sounds = lookup;
    }

    /// Ask the host for a new string to be written at the caret (or over the string-id argument
    /// the caret is in): raises [`ScriptEvent::RequestNewString`]; answer with
    /// [`ScriptEditor::insert_string_id`].
    pub fn request_new_string(&mut self) {
        self.ensure_cache();
        let span = self
            .string_arg_span_at_caret()
            .unwrap_or(Span::new(self.caret.1, self.caret.1));
        self.new_string_target = Some((span, self.gen()));
        let at = self.span_to_file(span);
        self.events.push(ScriptEvent::RequestNewString { at });
    }

    /// The span of the string-id argument at the caret (buffer coordinates), if it is in one.
    fn string_arg_span_at_caret(&mut self) -> Option<Span> {
        self.ensure_cache();
        self.cache
            .as_ref()
            .unwrap()
            .analysis
            .string_arg_span(self.caret.1)
    }

    /// Write the id of a new string where [`ScriptEvent::RequestNewString`] asked for it (or at
    /// the caret when the text changed in the meantime), as one edit.
    pub fn insert_string_id(&mut self, id: i64) {
        let text = id.to_string();
        match self.new_string_target.take() {
            Some((span, gen)) if gen == self.gen() => {
                let end = span.start + text.chars().count();
                self.queue(Transaction::single(
                    Edit::new(span.start, span.end, text),
                    Some((end, end)),
                ));
                self.want_focus = true;
            }
            _ => self.insert_at_caret(&text),
        }
    }

    /// Replace several spans (file bytes) in one step, so one undo reverts them all. Spans must
    /// not overlap; they are applied together on the next frame (or
    /// [`ScriptEditor::apply_pending`]). The caret keeps its place in the text. Returns false (and
    /// changes nothing) when spans overlap, lie outside the text, or the editor is read-only.
    pub fn replace_spans(&mut self, edits: &[(Span, String)]) -> bool {
        if self.read_only || edits.is_empty() {
            return !self.read_only;
        }
        self.ensure_cache();
        let len = self.cache.as_ref().unwrap().bytes.len();
        let mut list: Vec<(Span, String)> = edits
            .iter()
            .map(|(s, t)| (self.span_to_buf(*s), t.clone()))
            .collect();
        list.sort_by_key(|(s, _)| (s.start, s.end));
        for (i, (s, _)) in list.iter().enumerate() {
            if s.end < s.start || s.end > len || (i > 0 && list[i - 1].0.end > s.start) {
                return false;
            }
        }
        // where the caret ends up: shifted by the edits before it, at the end of an edit it was in
        let caret = self.caret.1;
        let mut shift: isize = 0;
        let mut new_caret = caret;
        for (s, t) in &list {
            let n = t.chars().count() as isize;
            if s.end <= caret {
                shift += n - (s.end - s.start) as isize;
                new_caret = (caret as isize + shift) as usize;
            } else if s.start < caret {
                new_caret = (s.start as isize + shift + n) as usize;
                shift += n - (s.end - s.start) as isize;
            }
        }
        let edits: Vec<Edit> = list
            .into_iter()
            .map(|(s, t)| Edit::new(s.start, s.end, t))
            .collect();
        self.queue(Transaction {
            edits,
            select: Some((new_caret, new_caret)),
        });
        true
    }

    /// Make the current text an undo point of the text field, so that Ctrl+Z comes back to it.
    /// For a host that changes the text while the editor is not shown (another view of the same
    /// script): the text field's undo only records while it has the keyboard focus.
    pub fn checkpoint_undo(&mut self, ctx: &egui::Context) {
        self.apply_pending();
        let id = self.text_id();
        let mut st = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        let cursor = st.cursor.char_range().unwrap_or_default();
        let mut undoer = st.undoer();
        undoer.add_undo(&(cursor, self.buf.text().to_string()));
        st.set_undoer(undoer);
        st.store(ctx, id);
    }

    /// Usage meters of the last validation.
    pub fn meters(&self) -> &[Meter] {
        &self.validation.meters
    }

    /// (errors, warnings, infos).
    pub fn diagnostic_counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for d in &self.validation.diagnostics {
            match d.severity {
                Severity::Error => c.0 += 1,
                Severity::Warning => c.1 += 1,
                Severity::Info => c.2 += 1,
            }
        }
        c
    }

    /// Action points, hero gates and coordinate pairs the script refers to (spans in file bytes).
    pub fn map_references(&mut self) -> Vec<MapReference> {
        self.ensure_cache();
        let refs = self.cache.as_ref().unwrap().map_refs().to_vec();
        refs.into_iter()
            .map(|r| MapReference {
                span: self.span_to_file(r.span),
                ..r
            })
            .collect()
    }

    pub fn outline(&mut self) -> Vec<OutlineItem> {
        self.ensure_cache();
        self.cache.as_ref().unwrap().outline().to_vec()
    }

    /// Completions at the caret (what the popup would show), for hosts and tests.
    pub fn completions_at_caret(&mut self, host: &dyn ConstantProvider) -> Vec<Completion> {
        self.ensure_cache();
        let off = self.caret.1;
        self.cache
            .as_ref()
            .unwrap()
            .analysis
            .completions_with(off, host, self.strings.as_ref())
    }

    pub fn signature_help_at_caret(&mut self) -> Option<SignatureHelp> {
        self.ensure_cache();
        let off = self.caret.1;
        self.cache.as_ref().unwrap().analysis.signature_help(off)
    }

    // -----------------------------------------------------------------------------------------
    // Caret and selection

    /// Caret (head of the selection) as a file byte offset.
    pub fn caret_file_offset(&mut self) -> usize {
        self.ensure_cache();
        self.buf_to_file(self.caret.1)
    }

    /// Selection as a file byte span.
    pub fn selection_span(&mut self) -> Span {
        self.ensure_cache();
        let (a, b) = self.caret;
        self.span_to_file(Span::new(a.min(b), a.max(b)))
    }

    /// Set the caret/selection from a file byte span (anchor at `start`, head at `end`).
    pub fn select_span(&mut self, span: Span) {
        self.ensure_cache();
        let s = self.span_to_buf(span);
        self.caret = (s.start, s.end);
        self.want_sel = Some(self.caret);
    }

    /// Jump to and select a span of the file bytes (e.g. one of [`ScriptEditor::map_references`]).
    pub fn reveal_span(&mut self, span: Span) {
        self.select_span(span);
        self.want_scroll = true;
        self.want_focus = true;
    }

    /// Jump to a 1-based line.
    pub fn goto_line(&mut self, line: usize) {
        self.ensure_cache();
        let c = self.cache.as_ref().unwrap();
        let l = line.saturating_sub(1).min(c.lines.line_count() - 1);
        let p = c.lines.line_start(l);
        self.caret = (p, p);
        self.want_sel = Some(self.caret);
        self.want_scroll = true;
        self.want_focus = true;
    }

    /// (line, column), both 1-based, of the caret.
    /// Screen position of the laid out text after the last frame (moves when the text scrolls).
    pub fn galley_screen_pos(&self) -> Option<Pos2> {
        self.geometry.as_ref().map(|g| g.pos)
    }

    pub fn caret_line_col(&mut self) -> (usize, usize) {
        self.ensure_cache();
        let (l, c) = self.cache.as_ref().unwrap().lines.line_col(self.caret.1);
        (l + 1, c + 1)
    }

    /// 1-based `(line, column)` of a file byte offset.
    pub fn file_offset_line_col(&mut self, off: usize) -> (usize, usize) {
        self.ensure_cache();
        let b = self.file_to_buf(off);
        let (l, c) = self.cache.as_ref().unwrap().lines.line_col(b);
        (l + 1, c + 1)
    }

    pub fn set_caret(&mut self, buffer_offset: usize) {
        self.caret = (buffer_offset, buffer_offset);
        self.want_sel = Some(self.caret);
    }

    // -----------------------------------------------------------------------------------------
    // Edits

    pub(crate) fn queue(&mut self, tx: Transaction) {
        self.pending.push(tx);
    }

    /// Insert text at the caret (replacing the selection). Applied on the next frame (or by
    /// [`ScriptEditor::apply_pending`]).
    pub fn insert_at_caret(&mut self, text: &str) {
        let (lo, hi) = (
            self.caret.0.min(self.caret.1),
            self.caret.0.max(self.caret.1),
        );
        let end = lo + text.chars().count();
        self.queue(Transaction::single(
            Edit::new(lo, hi, text),
            Some((end, end)),
        ));
        self.want_focus = true;
    }

    /// Write a value picked on the map into the argument under the caret; when the caret is not
    /// in a matching argument the value is inserted as text at the caret.
    pub fn insert_pick(&mut self, value: PickValue) {
        self.ensure_cache();
        let off = self.caret.1;
        let reps = self
            .cache
            .as_ref()
            .unwrap()
            .analysis
            .pick_replacements(off, value);
        if reps.is_empty() {
            let t = match value {
                PickValue::ActionPoint(n) => n.to_string(),
                PickValue::HeroGate(n) => (-n.abs()).to_string(),
                PickValue::Subtile { x, y } | PickValue::Slab { x, y } => format!("{x}, {y}"),
            };
            self.insert_at_caret(&t);
            return;
        }
        let end = reps
            .iter()
            .map(|(s, t)| s.start + t.chars().count())
            .max()
            .unwrap_or(off);
        let mut edits: Vec<Edit> = reps
            .into_iter()
            .map(|(s, t)| Edit::new(s.start, s.end, t))
            .collect();
        edits.sort_by_key(|e| e.start);
        // positions after earlier replacements shift by their length change
        let mut shift: isize = 0;
        let mut caret = end;
        for e in &edits {
            let new_end = (e.start as isize + shift) as usize + e.text.chars().count();
            caret = new_end;
            shift += e.text.chars().count() as isize - (e.end - e.start) as isize;
        }
        self.queue(Transaction {
            edits,
            select: Some((caret, caret)),
        });
        self.want_focus = true;
    }

    /// Insert the snippet of a reference entry at the caret (`NAME(${1:a}, ${2:b})`, Tab walks the
    /// arguments).
    pub fn insert_command(&mut self, name: &str) {
        let params = self
            .reference
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.snippet_params.clone())
            .unwrap_or_default();
        let mut t = name.to_string();
        if !params.is_empty() || self.lang.call_parens() == CallParens::Pair {
            t.push('(');
            for (i, p) in params.iter().enumerate() {
                if i > 0 {
                    t.push_str(", ");
                }
                let ph: String = p
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect();
                t.push_str(&format!("${{{}:{}}}", i + 1, ph));
            }
            t.push(')');
        }
        self.insert_snippet(&t);
    }

    /// Insert a snippet template at the caret. `${n:default}` marks placeholders (Tab and
    /// Shift+Tab move between them), `$0` the final caret. On a blank line the snippet replaces
    /// the blank, otherwise it goes on a new line below; tabs in the template become the
    /// editor's indentation unit. Ignored in a read-only editor.
    pub fn insert_snippet(&mut self, template: &str) {
        if self.read_only {
            return;
        }
        self.ensure_cache();
        let template = template.replace('\t', &self.options.indent);
        let acc = textops::insert_snippet_template(
            &self.cache.as_ref().unwrap().bytes,
            self.caret.1,
            &template,
        );
        self.queue(acc.tx);
        self.new_snippet = Some((acc.stops, acc.end));
        self.want_focus = true;
    }

    /// True while Tab walks through the placeholders of a snippet.
    pub fn snippet_active(&self) -> bool {
        self.snippet.is_some() || self.new_snippet.is_some()
    }

    /// Keep the placeholder ranges in step with edits made since the last call.
    pub(crate) fn sync_snippet(&mut self) {
        let gen = self.gen();
        let Some(sn) = &mut self.snippet else { return };
        if sn.gen == gen {
            return;
        }
        let bytes = latin1::encode(self.buf.text());
        let mut end = [sn.end.unwrap_or(0)];
        textops::shift_ranges(&sn.prev, &bytes, &mut sn.stops, &mut end);
        if sn.end.is_some() {
            sn.end = Some(end[0]);
        }
        sn.prev = bytes;
        sn.gen = gen;
    }

    /// Leave the snippet session when the caret left all placeholders.
    pub(crate) fn check_snippet_caret(&mut self) {
        self.sync_snippet();
        let (lo, hi) = (
            self.caret.0.min(self.caret.1),
            self.caret.0.max(self.caret.1),
        );
        if let Some(sn) = &self.snippet {
            let inside = sn.stops.iter().any(|&(a, b)| a <= lo && hi <= b)
                || sn.end.map_or(false, |e| lo == e && hi == e);
            if !inside {
                self.snippet = None;
            }
        }
    }

    /// Tab / Shift+Tab inside a snippet session. False when no session is active.
    pub(crate) fn snippet_step(&mut self, forward: bool) -> bool {
        self.check_snippet_caret();
        let (lo, hi) = (
            self.caret.0.min(self.caret.1),
            self.caret.0.max(self.caret.1),
        );
        let Some(sn) = &self.snippet else {
            return false;
        };
        let cur = sn.stops.iter().position(|&(a, b)| a <= lo && hi <= b);
        let target = match (cur, forward) {
            (Some(i), true) => sn.stops.get(i + 1).copied(),
            (Some(0), false) => return true,
            (Some(i), false) => sn.stops.get(i - 1).copied(),
            (None, true) => None,
            (None, false) => sn.stops.last().copied(),
        };
        let sel = match target {
            Some(t) => t,
            None if forward => {
                let e = sn
                    .end
                    .unwrap_or_else(|| sn.stops.last().map_or(hi, |s| s.1));
                self.snippet = None;
                (e, e)
            }
            None => return true,
        };
        self.caret = sel;
        self.want_sel = Some(sel);
        self.want_scroll = true;
        true
    }

    /// Apply queued edits to the buffer now (the UI does this at the start of each frame).
    pub fn apply_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        self.sync_snippet();
        let txs = std::mem::take(&mut self.pending);
        if self.read_only {
            self.new_snippet = None;
            return;
        }
        for tx in txs {
            let mut edits = tx.edits;
            edits.sort_by_key(|e| std::cmp::Reverse(e.start));
            for e in &edits {
                self.buf.replace(e.start, e.end, &e.text);
            }
            if let Some(sel) = tx.select {
                self.caret = sel;
                self.want_sel = Some(sel);
            }
        }
        self.completion_armed = false;
        match self.new_snippet.take() {
            Some((stops, end)) if !stops.is_empty() => {
                self.snippet = Some(SnippetSession {
                    stops,
                    end,
                    prev: latin1::encode(self.buf.text()),
                    gen: self.gen(),
                });
            }
            Some(_) => self.snippet = None,
            None => self.sync_snippet(),
        }
    }

    /// Re-indent the whole script with the language formatter.
    pub fn format_document(&mut self) {
        self.ensure_cache();
        let c = self.cache.as_ref().unwrap();
        let Some(out) = c.analysis.format(&self.options.indent) else {
            return;
        };
        if out == c.bytes {
            return;
        }
        let end = c.bytes.len();
        let text = latin1::decode(&out);
        let caret = self.caret.1.min(out.len());
        self.queue(Transaction::single(
            Edit::new(0, end, text),
            Some((caret, caret)),
        ));
    }

    pub fn toggle_comment(&mut self) {
        self.ensure_cache();
        let tx = textops::toggle_comment(
            &self.cache.as_ref().unwrap().bytes,
            self.caret,
            self.lang.line_comment(),
        );
        self.queue(tx);
    }

    pub(crate) fn newline_tx(&mut self) -> Transaction {
        self.ensure_cache();
        let c = self.cache.as_ref().unwrap();
        let (lo, _) = (self.caret.0.min(self.caret.1), 0);
        let l = c.lines.line_of(lo);
        let line_text = latin1::decode(&c.bytes[c.lines.line_start(l)..c.lines.line_end(l)]);
        let delta = c.analysis.block_delta(&line_text);
        textops::newline(&c.bytes, self.caret, &self.options.indent, delta)
    }

    // -----------------------------------------------------------------------------------------
    // Find

    pub fn open_find(&mut self, replace: bool) {
        self.find.open = true;
        self.find.replace = replace;
        self.find.focus = true;
        // seed with a single-line selection
        self.ensure_cache();
        let (lo, hi) = (
            self.caret.0.min(self.caret.1),
            self.caret.0.max(self.caret.1),
        );
        if hi > lo && hi - lo < 100 {
            let s = latin1::decode(&self.cache.as_ref().unwrap().bytes[lo..hi]);
            if !s.contains('\n') {
                self.find.query = s;
            }
        }
    }

    pub fn find_matches(&mut self) -> Vec<Span> {
        self.ensure_cache();
        let key = (
            self.gen(),
            self.find.query.clone(),
            self.find.opts.case_sensitive,
        );
        if self.find.matches_key.as_ref() != Some(&key) {
            let q = latin1::encode(&self.find.query);
            self.find.matches =
                textops::find_all(&self.cache.as_ref().unwrap().bytes, &q, self.find.opts);
            self.find.matches_key = Some(key);
        }
        self.find.matches.clone()
    }

    /// Select the next (or previous) match relative to the caret.
    pub fn find_step(&mut self, forward: bool) -> Option<Span> {
        self.ensure_cache();
        let q = latin1::encode(&self.find.query);
        let bytes = &self.cache.as_ref().unwrap().bytes;
        let (lo, hi) = (
            self.caret.0.min(self.caret.1),
            self.caret.0.max(self.caret.1),
        );
        let m = if forward {
            textops::find_next(bytes, &q, hi, self.find.opts)
        } else {
            textops::find_prev(bytes, &q, lo, self.find.opts)
        }?;
        self.caret = (m.start, m.end);
        self.want_sel = Some(self.caret);
        self.want_scroll = true;
        self.find.current = Some(m);
        Some(m)
    }

    /// Select the first match at or after the caret without moving past a current match
    /// (incremental search while typing).
    pub fn find_from_caret(&mut self) -> Option<Span> {
        self.ensure_cache();
        let q = latin1::encode(&self.find.query);
        let from = self.caret.0.min(self.caret.1);
        let m = textops::find_next(
            &self.cache.as_ref().unwrap().bytes,
            &q,
            from,
            self.find.opts,
        )?;
        self.caret = (m.start, m.end);
        self.want_sel = Some(self.caret);
        self.want_scroll = true;
        self.find.current = Some(m);
        Some(m)
    }

    pub fn replace_current(&mut self) {
        self.ensure_cache();
        let q = latin1::encode(&self.find.query);
        let with = self.find.replacement.clone();
        if let Some(tx) = textops::replace_current(
            &self.cache.as_ref().unwrap().bytes,
            &q,
            &with,
            self.caret,
            self.find.opts,
        ) {
            self.queue(tx);
            self.apply_pending();
            self.find_step(true);
        } else {
            self.find_step(true);
        }
    }

    /// Replace every match; returns how many.
    pub fn replace_all(&mut self) -> usize {
        self.ensure_cache();
        let q = latin1::encode(&self.find.query);
        let with = self.find.replacement.clone();
        match textops::replace_all(
            &self.cache.as_ref().unwrap().bytes,
            &q,
            &with,
            self.find.opts,
        ) {
            Some((tx, n)) => {
                self.queue(tx);
                self.apply_pending();
                self.find.current = None;
                n
            }
            None => 0,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Completion

    /// Accept item `i` of the open popup.
    pub fn accept_completion(&mut self, i: usize) {
        let Some(st) = self.completion.take() else {
            return;
        };
        let Some(mut c) = st.items.get(i).cloned() else {
            return;
        };
        self.ensure_cache();
        if c.action == Some(CompletionAction::NewString) {
            self.new_string_target = Some((c.replace, self.gen()));
            let at = self.span_to_file(c.replace);
            self.events.push(ScriptEvent::RequestNewString { at });
            self.want_focus = true;
            return;
        }
        if c.kind == CompletionKind::Snippet {
            c.insert = c.insert.replace('\t', &self.options.indent);
        }
        let acc = textops::accept_completion(
            &self.cache.as_ref().unwrap().bytes,
            &c,
            self.lang.call_parens(),
        );
        // A command that opened a parenthesis immediately offers its first argument.
        let opens = c.callable == Some(true) && self.lang.call_parens() == CallParens::Open;
        self.queue(acc.tx);
        self.new_snippet = Some((acc.stops, acc.end));
        self.apply_pending();
        self.completion_armed = opens;
        self.want_focus = true;
    }

    /// Recompute the popup items for the caret (opens the popup when there are some).
    pub fn refresh_completions(&mut self, host: &dyn ConstantProvider) {
        self.ensure_cache();
        let off = self.caret.1;
        let mut items = self.cache.as_ref().unwrap().analysis.completions_with(
            off,
            host,
            self.strings.as_ref(),
        );
        items.truncate(300);
        // an exact single match needs no popup
        if items.len() == 1
            && self
                .word_before_caret()
                .eq_ignore_ascii_case(&items[0].insert)
        {
            items.clear();
        }
        if items.is_empty() {
            self.completion = None;
            return;
        }
        let prev = self
            .completion
            .as_ref()
            .and_then(|c| c.items.get(c.selected))
            .map(|c| c.label.clone());
        let selected = prev
            .and_then(|l| items.iter().position(|c| c.label == l))
            .unwrap_or(0);
        let navigated = self.completion.as_ref().map_or(false, |c| c.navigated);
        self.completion = Some(CompletionState {
            items,
            selected,
            scroll_to_selected: true,
            navigated,
        });
    }

    pub(crate) fn word_before_caret(&mut self) -> String {
        self.ensure_cache();
        let c = self.cache.as_ref().unwrap();
        let end = self.caret.1.min(c.bytes.len());
        let mut s = end;
        while s > 0 && self.lang.is_word_byte(c.bytes[s - 1]) {
            s -= 1;
        }
        latin1::decode(&c.bytes[s..end])
    }

    pub(crate) fn all_diags_at(&self, off: usize) -> Vec<Diagnostic> {
        self.validation
            .diagnostics
            .iter()
            .filter(|d| {
                if d.span.is_empty() {
                    d.span.start == off
                } else {
                    d.span.contains(off)
                }
            })
            .filter(|d| self.is_shown(d))
            .cloned()
            .collect()
    }

    pub(crate) fn select_line_of_diag(&mut self, d: &Diagnostic) {
        self.caret = (d.span.start, d.span.end.max(d.span.start));
        self.want_sel = Some(self.caret);
        self.want_scroll = true;
        self.want_focus = true;
    }
}

/// Hooks used by the integration tests (kept public so they can live in `tests/`).
impl ScriptEditor {
    #[doc(hidden)]
    pub fn refresh_completions_for_test(&mut self, host: &dyn ConstantProvider) {
        self.refresh_completions(host);
    }
    #[doc(hidden)]
    pub fn completion_labels(&self) -> Vec<String> {
        self.completion
            .as_ref()
            .map(|c| c.items.iter().map(|i| i.label.clone()).collect())
            .unwrap_or_default()
    }
    #[doc(hidden)]
    pub fn pick_kind_at_caret(&mut self) -> Option<PickKind> {
        self.ensure_cache();
        self.cache
            .as_ref()
            .unwrap()
            .analysis
            .pick_target(self.caret.1)
    }
    #[doc(hidden)]
    pub fn find_query_for_test(&mut self, q: &str) {
        self.find.query = q.to_string();
        self.find.matches_key = None;
    }
    #[doc(hidden)]
    pub fn replace_all_with_for_test(&mut self, with: &str) -> usize {
        self.find.replacement = with.to_string();
        self.replace_all()
    }
}

/// Hooks used by the screenshot example and the tests.
impl ScriptEditor {
    #[doc(hidden)]
    pub fn completion_clear_for_test(&mut self) {
        self.completion = None;
    }
    #[doc(hidden)]
    pub fn set_reference_search_for_test(&mut self, q: &str) {
        self.ref_search = q.to_string();
    }
    #[doc(hidden)]
    pub fn select_reference_for_test(&mut self, name: &str) {
        self.ref_selected = Some(name.to_string());
    }
    #[doc(hidden)]
    pub fn set_replacement_for_test(&mut self, r: &str) {
        self.find.replacement = r.to_string();
    }
    /// First buffer offset of `needle` (test helper).
    #[doc(hidden)]
    pub fn test_offset_of(&self, needle: &[u8]) -> usize {
        let b = crate::latin1::encode(self.buf.text());
        b.windows(needle.len())
            .position(|w| w == needle)
            .unwrap_or(0)
    }
    /// Screen position of a buffer offset as of the last frame (test helper).
    #[doc(hidden)]
    pub fn test_char_pos(&self, off: usize) -> Option<egui::Pos2> {
        let g = self.geometry.as_ref()?;
        let r = g
            .galley
            .pos_from_cursor(egui::text::CCursor::new(off))
            .translate(g.pos.to_vec2());
        Some(egui::pos2(r.center().x + 3.0, r.center().y))
    }
}

/// Panel layout (hosts can drive it from their own menus).
impl ScriptEditor {
    pub fn set_side_panel(&mut self, p: SidePanel) {
        self.side = p;
    }
    pub fn side_panel(&self) -> SidePanel {
        self.side
    }
    pub fn set_problems_panel(&mut self, open: bool) {
        self.problems_open = open;
    }
    pub fn problems_panel_open(&self) -> bool {
        self.problems_open
    }
}

/// Navigation, read-only mode and display options.
impl ScriptEditor {
    /// Definition of the name at the caret. A definition in this text selects it; one in another
    /// file is reported as [`ScriptEvent::GotoDefinition`]. Returns whether a definition was found.
    pub fn goto_definition(&mut self) -> bool {
        let off = self.caret.1;
        self.follow_definition(off)
    }

    pub(crate) fn follow_definition(&mut self, buf_off: usize) -> bool {
        self.ensure_cache();
        let Some(def) = self.cache.as_ref().unwrap().analysis.definition(buf_off) else {
            return false;
        };
        match def.file {
            None => {
                self.caret = (def.span.start, def.span.end);
                self.want_sel = Some(self.caret);
                self.want_scroll = true;
                self.want_focus = true;
            }
            Some(file) => self.events.push(ScriptEvent::GotoDefinition {
                file,
                span: def.span,
            }),
        }
        true
    }

    /// Show the text but refuse edits (library files). Selection, find and navigation still work.
    pub fn set_read_only(&mut self, ro: bool) {
        self.read_only = ro;
        self.buf.set_read_only(ro);
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn font_size(&self) -> f32 {
        self.theme.font_size
    }

    /// Editor font size in points (8 to 32).
    pub fn set_font_size(&mut self, size: f32) {
        self.theme.font_size = size.clamp(8.0, 32.0);
        self.theme_rev += 1;
    }

    /// Switch between the built-in dark and light themes, keeping the font size.
    pub fn set_light(&mut self, light: bool) {
        let size = self.theme.font_size;
        self.theme = if light { Theme::light() } else { Theme::dark() }.with_font_size(size);
        self.theme_rev += 1;
    }

    /// Items of the status line of the last analysis (Lua: the hook functions of the file).
    pub fn status_items(&mut self) -> Vec<StatusItem> {
        self.ensure_cache();
        self.cache.as_ref().unwrap().analysis.status_items()
    }

    /// Select a snippet in the snippet panel (screenshots).
    #[doc(hidden)]
    pub fn select_snippet_for_test(&mut self, id: &str) {
        self.snippet_selected = Some(id.to_string());
    }

    /// Layer and screen rect of the signature help drawn in the last frame (tests).
    #[doc(hidden)]
    pub fn signature_popup_for_test(&self) -> Option<(egui::LayerId, Rect)> {
        self.sig_drawn
    }

    /// Events queued by non-UI calls such as [`ScriptEditor::goto_definition`] (tests).
    #[doc(hidden)]
    pub fn take_events_for_test(&mut self) -> Vec<ScriptEvent> {
        std::mem::take(&mut self.events)
    }

    /// Where the ranges of the active snippet placeholders are (buffer coordinates; tests).
    #[doc(hidden)]
    pub fn snippet_stops_for_test(&mut self) -> Vec<(usize, usize)> {
        self.sync_snippet();
        self.snippet
            .as_ref()
            .map(|s| s.stops.clone())
            .unwrap_or_default()
    }
}

/// Snippet library.
impl ScriptEditor {
    /// Insert a library snippet in the form of this editor's language. A Lua snippet that belongs
    /// into `OnGameStart` goes into the body of that function when the file has one (at the caret
    /// when the caret is inside it, else after the last statement); a file without the function
    /// gets a new one. Returns false when the snippet has no form for this language.
    pub fn insert_library_snippet(&mut self, snip: &crate::snippets::Snippet) -> bool {
        use crate::snippets::{lua_insertion, LuaPlace};
        let id = self.lang.id();
        if id != "lua" {
            return match snip.template(id) {
                Some(t) => {
                    self.insert_snippet(t);
                    true
                }
                None => false,
            };
        }
        let Some((_, place)) = snip.lua else {
            return false;
        };
        self.ensure_cache();
        let hook = if place == LuaPlace::GameStart {
            let c = self.cache.as_ref().unwrap();
            c.outline()
                .iter()
                .find(|i| i.tag == "hook" && i.label.starts_with("OnGameStart("))
                .map(|i| i.span)
        } else {
            None
        };
        if let Some(sp) = hook {
            let c = self.cache.as_ref().unwrap();
            let caret = self.caret.1;
            let (first, last) = (
                c.lines.line_of(sp.start),
                c.lines.line_of(sp.end.saturating_sub(1)),
            );
            if !(sp.start < caret && caret < sp.end) && last > first {
                let at = c.lines.line_end(last - 1);
                self.caret = (at, at);
            }
        }
        match lua_insertion(snip, hook.is_some()) {
            Some(t) => {
                self.insert_snippet(&t);
                true
            }
            None => false,
        }
    }
}
