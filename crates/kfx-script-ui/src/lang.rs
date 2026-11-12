//! The language plug-in boundary of the editor.
//!
//! A [`Language`] describes a scripting language statically (comment marker, indentation rules,
//! command reference) and turns a text into an [`Analysis`]: tokens, completions, hover,
//! signature help, outline and map references for that exact text. The DK level script is
//! implemented in [`crate::dk`]; a Lua language only has to implement these two traits.
//!
//! Every span is a byte range of the text handed to [`Language::analyze`]
//! (the editor passes its LF-normalised buffer; one byte is one character).
pub use crate::strings::StringLookup;
use kfx_script::{ConstantProvider, Diagnostic, Span};

/// One usage meter (script limit of the game).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meter {
    pub label: String,
    pub used: usize,
    /// Game limit; 0 means "no limit".
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeterLevel {
    Ok,
    /// 80% of the limit or more.
    Near,
    /// Over the limit.
    Over,
}

impl Meter {
    pub fn level(&self) -> MeterLevel {
        if self.limit == 0 {
            MeterLevel::Ok
        } else if self.used > self.limit {
            MeterLevel::Over
        } else if self.used * 5 >= self.limit * 4 {
            MeterLevel::Near
        } else {
            MeterLevel::Ok
        }
    }
    pub fn fraction(&self) -> f32 {
        if self.limit == 0 {
            0.0
        } else {
            (self.used as f32 / self.limit as f32).min(1.0)
        }
    }
}

/// Highlight class of a token (the theme maps it to a colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HlClass {
    Command,
    Keyword,
    Function,
    Number,
    String,
    Constant,
    Operator,
    Punctuation,
    Comment,
    Error,
    /// `@param`, `@class` ... inside a Lua `---` comment.
    Annotation,
    /// A type expression inside an annotation.
    AnnotationType,
    Local,
    Parameter,
    /// A function of the KeeperFX library.
    ApiFunction,
    /// A global of the KeeperFX library (`PLAYER0`, `Game`, classes).
    ApiGlobal,
    /// Lua standard library.
    Std,
    /// A name defined by the file itself.
    UserGlobal,
    /// Unknown global.
    Unknown,
    Field,
    Method,
}

impl HlClass {
    /// Drawn in italics.
    pub fn italic(self) -> bool {
        matches!(
            self,
            HlClass::Comment | HlClass::Annotation | HlClass::AnnotationType
        )
    }
}

impl From<kfx_script::TokenClass> for HlClass {
    fn from(c: kfx_script::TokenClass) -> HlClass {
        use kfx_script::TokenClass as T;
        match c {
            T::Command => HlClass::Command,
            T::Keyword => HlClass::Keyword,
            T::Function => HlClass::Function,
            T::Number => HlClass::Number,
            T::String => HlClass::String,
            T::Constant => HlClass::Constant,
            T::Operator => HlClass::Operator,
            T::Punctuation => HlClass::Punctuation,
            T::Comment => HlClass::Comment,
            T::Error => HlClass::Error,
        }
    }
}

/// What a completion item is (drives the tag in the popup and the acceptance rules).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    /// A function or command; see [`Completion::callable`].
    Command,
    Constant,
    Snippet,
    Variable,
    Field,
    Keyword,
    Type,
    Hook,
}

impl CompletionKind {
    /// Short tag shown in the popup.
    pub fn tag(self) -> &'static str {
        match self {
            CompletionKind::Command => "cmd",
            CompletionKind::Constant => "const",
            CompletionKind::Snippet => "snip",
            CompletionKind::Variable => "var",
            CompletionKind::Field => "field",
            CompletionKind::Keyword => "kw",
            CompletionKind::Type => "type",
            CompletionKind::Hook => "hook",
        }
    }
}

/// One completion item.
#[derive(Clone, Debug)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    /// Text replacing `replace`. Snippets may contain `$0` (final caret) and `${n:default}`
    /// placeholders that Tab walks through.
    pub insert: String,
    pub detail: String,
    pub doc: String,
    pub replace: Span,
    pub deprecated: bool,
    /// `Some(has_arguments)` when accepting the item should also write the call parentheses
    /// (see [`Language::call_parens`]).
    pub callable: Option<bool>,
    /// What accepting the item does besides (or instead of) inserting text.
    pub action: Option<CompletionAction>,
}

/// Special behaviour of a completion item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionAction {
    /// Do not insert anything: raise [`crate::ScriptEvent::RequestNewString`] so the host creates
    /// a string and answers with [`crate::ScriptEditor::insert_string_id`].
    NewString,
}

impl Completion {
    pub fn new(
        label: impl Into<String>,
        kind: CompletionKind,
        insert: impl Into<String>,
        replace: Span,
    ) -> Completion {
        Completion {
            label: label.into(),
            kind,
            insert: insert.into(),
            detail: String::new(),
            doc: String::new(),
            replace,
            deprecated: false,
            callable: None,
            action: None,
        }
    }
}

impl From<kfx_script::Completion> for Completion {
    fn from(c: kfx_script::Completion) -> Completion {
        let kind = match c.kind {
            kfx_script::CompletionKind::Command => CompletionKind::Command,
            kfx_script::CompletionKind::Constant => CompletionKind::Constant,
            kfx_script::CompletionKind::Snippet => CompletionKind::Snippet,
        };
        Completion {
            label: c.label,
            kind,
            insert: c.insert,
            detail: c.detail,
            doc: c.doc,
            replace: c.replace,
            deprecated: false,
            callable: None,
            action: None,
        }
    }
}

/// How accepting a callable completion writes the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallParens {
    /// `NAME(` (the DK script closes the parenthesis itself).
    Open,
    /// `name()` with the caret between the parentheses (after them for a function without
    /// parameters).
    Pair,
}

/// One parameter of a signature.
#[derive(Clone, Debug)]
pub struct SigParam {
    pub name: String,
    /// Type or kind text, when it says more than the name.
    pub detail: Option<String>,
    pub doc: String,
    pub optional: bool,
    /// Byte range of the parameter inside [`SignatureHelp::label`].
    pub label_range: (usize, usize),
}

/// Signature of the call around the caret.
#[derive(Clone, Debug)]
pub struct SignatureHelp {
    /// Command or function name.
    pub command: String,
    pub label: String,
    pub doc: String,
    pub params: Vec<SigParam>,
    /// Index into `params` (may be one past the end for a variadic tail).
    pub active: usize,
}

/// One entry of the outline panel.
#[derive(Clone, Debug)]
pub struct OutlineItem {
    /// Short tag shown before the label (`IF`, `fn`, `hook`).
    pub tag: &'static str,
    pub tone: Tone,
    pub label: String,
    pub span: Span,
    pub line: usize,
    pub children: Vec<OutlineItem>,
}

/// Colour class of tags and status items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Keyword,
    Function,
    Constant,
    Good,
    Bad,
    Muted,
}

/// A place a definition lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    /// `None` = the analysed text; otherwise a library file name as the language's index knows it
    /// (the host resolves it, see `ScriptHost::read_library`).
    pub file: Option<String>,
    pub span: Span,
}

/// An item of the status line shown instead of the usage meters (Lua).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusItem {
    pub text: String,
    pub tone: Tone,
    pub tooltip: String,
    /// Where the item points in the text (clicking jumps there).
    pub at: Option<Span>,
}

/// Result of the (debounced) validation pass.
#[derive(Clone, Debug, Default)]
pub struct Validation {
    pub diagnostics: Vec<Diagnostic>,
    pub meters: Vec<Meter>,
}

/// Hover card content.
#[derive(Clone, Debug, Default)]
pub struct HoverInfo {
    pub span: Span,
    /// Signature or heading, shown in monospace.
    pub title: Option<String>,
    pub summary: String,
    pub params: Vec<(String, String)>,
    pub examples: Vec<String>,
    /// Button label and URL.
    pub link: Option<(String, String)>,
}

/// What the argument under the caret stands for (what "pick from map" would insert).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PickKind {
    ActionPoint,
    HeroGate,
    /// An action point (positive) or a hero gate (negative number).
    Location,
    Subtile,
    Slab,
}

/// A value picked on the map, to be written into the script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickValue {
    ActionPoint(i32),
    HeroGate(i32),
    Subtile { x: i32, y: i32 },
    Slab { x: i32, y: i32 },
}

/// A place on the map a script refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapTarget {
    ActionPoint(i32),
    HeroGate(i32),
    Subtile { x: i32, y: i32 },
    Slab { x: i32, y: i32 },
}

/// An action point, hero gate or coordinate pair used by the script, with where it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapReference {
    /// Where the value (or both coordinates) are written. File byte offsets when returned by
    /// `ScriptEditor::map_references`.
    pub span: Span,
    pub target: MapTarget,
    pub command: String,
    /// 1-based line.
    pub line: usize,
}

/// Argument spans of the command around an offset, for Tab navigation between placeholders.
#[derive(Clone, Debug)]
pub struct ArgNav {
    pub args: Vec<Span>,
    /// Where the caret goes after the last argument.
    pub after: usize,
}

/// One entry of the command reference panel.
#[derive(Clone, Debug)]
pub struct RefEntry {
    pub name: String,
    pub category: String,
    pub signature: String,
    pub summary: String,
    pub params: Vec<(String, String)>,
    pub examples: Vec<String>,
    pub link: Option<String>,
    /// Parameter names for the insertion snippet (required ones only).
    pub snippet_params: Vec<String>,
    /// Shown dimmed ("needs a newer level version", undocumented ...).
    pub note: Option<String>,
}

/// Per-text analysis. Built by [`Language::analyze`] whenever the text changes.
pub trait Analysis {
    /// Classified tokens in source order (gaps are plain text).
    fn tokens(&self) -> &[(Span, HlClass)];
    /// Full validation. Heavier than parsing, so the editor debounces it.
    fn validate(&self, host: &dyn ConstantProvider) -> Validation;
    fn completions(&self, offset: usize, host: &dyn ConstantProvider) -> Vec<Completion>;
    fn signature_help(&self, offset: usize) -> Option<SignatureHelp>;
    fn hover(&self, offset: usize) -> Option<HoverInfo>;
    /// [`Analysis::hover`] with the host's string lookup: string-id arguments show their text.
    fn hover_with(&self, offset: usize, _strings: Option<&StringLookup>) -> Option<HoverInfo> {
        self.hover(offset)
    }
    /// [`Analysis::completions`] with the host's string lookup: string-id arguments list the ids
    /// with a preview of their text.
    fn completions_with(
        &self,
        offset: usize,
        host: &dyn ConstantProvider,
        _strings: Option<&StringLookup>,
    ) -> Vec<Completion> {
        self.completions(offset, host)
    }
    fn outline(&self) -> Vec<OutlineItem>;
    /// Span of the string-id argument the offset is in (or would be typed at), if any.
    fn string_arg_span(&self, _offset: usize) -> Option<Span> {
        None
    }
    /// The sound argument under the offset (`PLAY_MESSAGE`, `SET_MUSIC`), see [`crate::sounds`].
    fn sound_arg_at(&self, _offset: usize) -> Option<crate::sounds::SoundArg> {
        None
    }
    fn map_references(&self) -> Vec<MapReference> {
        vec![]
    }
    /// Spans of a matching opener/closer pair when the caret touches one of them.
    fn block_match(&self, _offset: usize) -> Option<(Span, Span)> {
        None
    }
    fn pick_target(&self, _offset: usize) -> Option<PickKind> {
        None
    }
    /// Replacements that write `value` into the argument(s) at `offset`; empty = no such argument.
    fn pick_replacements(&self, _offset: usize, _value: PickValue) -> Vec<(Span, String)> {
        vec![]
    }
    fn arg_nav(&self, _offset: usize) -> Option<ArgNav> {
        None
    }
    /// Indentation change a line causes for the lines after it: +1 opens a block, -1 closes one.
    fn block_delta(&self, _line: &str) -> i32 {
        0
    }
    /// Definition of the name at `offset` (F12, Ctrl+click).
    fn definition(&self, _offset: usize) -> Option<Definition> {
        None
    }
    /// Items of the status line that replace the usage meters when `Validation::meters` is empty.
    fn status_items(&self) -> Vec<StatusItem> {
        vec![]
    }
    /// Re-indented text, if the language has a formatter.
    fn format(&self, _indent: &str) -> Option<Vec<u8>> {
        None
    }
}

pub trait Language {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    /// Comment marker including the trailing space (`"REM "`, `"-- "`).
    fn line_comment(&self) -> &'static str;
    fn analyze(&self, text: &[u8]) -> Box<dyn Analysis>;
    /// Entries of the searchable command reference panel.
    fn reference(&self) -> Vec<RefEntry>;
    /// Characters that make up a word for completion triggering.
    fn is_word_byte(&self, b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    /// Characters that open the completion popup by themselves.
    fn trigger_chars(&self) -> &'static [char] {
        &['(', ',']
    }
    /// Whether Enter accepts the highlighted completion straight away (otherwise only after the
    /// user moved the selection with the arrow keys; Tab always accepts).
    fn enter_accepts_completion(&self) -> bool {
        true
    }
    /// Whether typing a space keeps the completion popup open (DK arguments continue after
    /// `, `).
    fn space_keeps_completion(&self) -> bool {
        true
    }
    /// Bracket and quote pairs the editor closes automatically (empty: no auto-closing).
    fn auto_pairs(&self) -> &'static [(char, char)] {
        &[]
    }
    fn call_parens(&self) -> CallParens {
        CallParens::Open
    }
    /// Whether a line of code that starts with `word` closes a block (used to outdent while
    /// typing).
    fn dedent_word(&self, _word: &str) -> bool {
        false
    }
}
