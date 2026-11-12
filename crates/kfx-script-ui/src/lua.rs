//! [`Language`] implementation for KeeperFX Lua level scripts (`mapNNNNN.lua`), on top of
//! [`kfx_script::lua`].
//!
//! The text the editor hands over is Latin-1 bytes (one byte, one character). The Lua tooling works
//! on `&str` with UTF-8 byte offsets, so a text that is valid UTF-8 is analysed as it is (offsets
//! are identical) and any other text is decoded as Latin-1 with an offset table; every span that
//! leaves this module is a byte offset of the original text.
use crate::lang::*;
use crate::latin1;
use kfx_script::lua::lexer::{lex, TokKind};
use kfx_script::lua::{
    self, ApiIndex, LuaCompletionKind, LuaDoc, LuaFormatOptions, LuaOutlineItem, LuaOutlineKind,
    LuaTokenClass,
};
use kfx_script::services::{RefKind, ScriptPos, ScriptRef};
use kfx_script::{ConstantProvider, Diagnostic, Span};
use std::cell::OnceCell;
use std::path::PathBuf;
use std::sync::Arc;

/// A directory of Lua files the host wants in the API index (the campaign's `lua` folder, the
/// map folder ...). Definitions found there are reported as `label/relative/path.lua`.
#[derive(Clone, Debug)]
pub struct LibraryRoot {
    pub label: String,
    pub path: PathBuf,
}

/// The Lua language. Cheap to clone; the API index is shared.
#[derive(Clone)]
pub struct LuaLanguage {
    index: Arc<ApiIndex>,
}

impl Default for LuaLanguage {
    fn default() -> Self {
        LuaLanguage::new()
    }
}

impl LuaLanguage {
    /// With the stock KeeperFX library index (`fxdata/lua`, embedded in `kfx-script`).
    pub fn new() -> LuaLanguage {
        LuaLanguage {
            index: Arc::new(ApiIndex::stock()),
        }
    }

    /// With an index the host built (see [`LuaLanguage::index_with_roots`]).
    pub fn with_index(index: Arc<ApiIndex>) -> LuaLanguage {
        LuaLanguage { index }
    }

    /// The stock index plus every `.lua` file below the given roots. Names in the stock index are
    /// relative to the install's `fxdata/lua`; names from a root are prefixed with its label.
    /// Do not add `fxdata/lua` itself (the stock index covers it) and do not add a folder holding
    /// the level's own script (its definitions would shadow the live buffer's).
    pub fn index_with_roots(roots: &[LibraryRoot]) -> ApiIndex {
        let mut ix = ApiIndex::stock();
        for r in roots {
            ix.add_root_labeled(&r.label, &r.path);
        }
        ix
    }

    pub fn index(&self) -> &Arc<ApiIndex> {
        &self.index
    }
}

/// Offsets between the text the editor holds and the string the Lua tooling parses.
struct Mapper {
    /// `(utf8 -> original, original -> utf8)` when the text is not valid UTF-8.
    tables: Option<(Vec<usize>, Vec<usize>)>,
}

impl Mapper {
    fn new(bytes: &[u8]) -> (Mapper, String) {
        match std::str::from_utf8(bytes) {
            Ok(s) => (Mapper { tables: None }, s.to_string()),
            Err(_) => {
                let s = latin1::decode(bytes);
                let mut u2o = Vec::with_capacity(s.len() + 1);
                let mut o2u = Vec::with_capacity(bytes.len() + 1);
                for (i, c) in s.chars().enumerate() {
                    o2u.push(u2o.len());
                    for _ in 0..c.len_utf8() {
                        u2o.push(i);
                    }
                }
                o2u.push(u2o.len());
                u2o.push(bytes.len());
                (
                    Mapper {
                        tables: Some((u2o, o2u)),
                    },
                    s,
                )
            }
        }
    }
    fn out(&self, u: usize) -> usize {
        match &self.tables {
            None => u,
            Some((u2o, _)) => u2o
                .get(u)
                .copied()
                .unwrap_or_else(|| u2o.last().copied().unwrap_or(0)),
        }
    }
    fn into_u8(&self, o: usize) -> usize {
        match &self.tables {
            None => o,
            Some((_, o2u)) => o2u
                .get(o)
                .copied()
                .unwrap_or_else(|| o2u.last().copied().unwrap_or(0)),
        }
    }
    fn span(&self, s: Span) -> Span {
        Span::new(self.out(s.start), self.out(s.end))
    }
}

fn hl(c: LuaTokenClass) -> HlClass {
    use LuaTokenClass as L;
    match c {
        L::Keyword => HlClass::Keyword,
        L::Constant => HlClass::Constant,
        L::Number => HlClass::Number,
        L::String => HlClass::String,
        L::Comment => HlClass::Comment,
        L::Annotation => HlClass::Annotation,
        L::AnnotationType => HlClass::AnnotationType,
        L::Operator => HlClass::Operator,
        L::Punctuation => HlClass::Punctuation,
        L::Local => HlClass::Local,
        L::Parameter => HlClass::Parameter,
        L::ApiFunction => HlClass::ApiFunction,
        L::ApiGlobal => HlClass::ApiGlobal,
        L::Std => HlClass::Std,
        L::UserGlobal => HlClass::UserGlobal,
        L::Unknown => HlClass::Unknown,
        L::Field => HlClass::Field,
        L::Method => HlClass::Method,
        L::Error => HlClass::Error,
    }
}

impl Language for LuaLanguage {
    fn id(&self) -> &'static str {
        "lua"
    }
    fn name(&self) -> &'static str {
        "Lua"
    }
    fn line_comment(&self) -> &'static str {
        "-- "
    }
    fn analyze(&self, text: &[u8]) -> Box<dyn Analysis> {
        let (map, src) = Mapper::new(text);
        let doc = LuaDoc::parse(&src);
        let tokens: Vec<(Span, HlClass)> = lua::highlight(&doc, &self.index)
            .into_iter()
            .map(|(s, c)| (map.span(s), hl(c)))
            .collect();
        let blocks = block_pairs(&doc)
            .into_iter()
            .map(|(a, b)| (map.span(a), map.span(b)))
            .collect();
        Box::new(LuaAnalysis {
            doc,
            index: self.index.clone(),
            map,
            tokens,
            blocks,
            refs: OnceCell::new(),
            poss: OnceCell::new(),
        })
    }
    fn reference(&self) -> Vec<RefEntry> {
        let mut out = vec![];
        for (name, f) in &self.index.functions {
            let file = f.source.as_ref().map(|s| s.file.as_str()).unwrap_or("");
            if file.starts_with("config-api/")
                || file.starts_with("examples/")
                || name.ends_with("_template")
            {
                continue;
            }
            let summary = f.doc.trim().to_string();
            out.push(RefEntry {
                name: name.clone(),
                category: category_of(file),
                signature: f.signature(),
                summary,
                params: f
                    .params
                    .iter()
                    .map(|p| {
                        (
                            p.name.clone(),
                            if p.doc.is_empty() {
                                p.ty.clone()
                            } else {
                                format!("{}: {}", p.ty, p.doc)
                            },
                        )
                    })
                    .collect(),
                examples: vec![],
                link: None,
                snippet_params: f
                    .params
                    .iter()
                    .filter(|p| !p.optional)
                    .map(|p| p.name.clone())
                    .collect(),
                note: f.deprecated.as_ref().map(|d| {
                    if d.is_empty() {
                        "Deprecated".to_string()
                    } else {
                        format!("Deprecated: {d}")
                    }
                }),
            });
        }
        out.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
        out
    }
    fn trigger_chars(&self) -> &'static [char] {
        &['.', ':', '"', '\'']
    }
    fn auto_pairs(&self) -> &'static [(char, char)] {
        &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')]
    }
    fn call_parens(&self) -> CallParens {
        CallParens::Pair
    }
    fn enter_accepts_completion(&self) -> bool {
        false
    }
    fn space_keeps_completion(&self) -> bool {
        false
    }
    fn dedent_word(&self, word: &str) -> bool {
        matches!(word, "end" | "else" | "elseif" | "until")
    }
}

/// `bindings/gui.lua` -> `GUI`, `triggers/Events.lua` -> `Triggers`.
fn category_of(file: &str) -> String {
    let stem = file
        .rsplit('/')
        .next()
        .unwrap_or(file)
        .trim_end_matches(".lua");
    let dir = file.split('/').next().unwrap_or("");
    let nice = |s: &str| {
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    };
    match dir {
        "bindings" => match stem {
            "gui" => "GUI and messages".to_string(),
            "setup" => "Level setup".to_string(),
            "parties" => "Parties and creatures".to_string(),
            "things" => "Things".to_string(),
            "map" => "Map".to_string(),
            "players" => "Players".to_string(),
            other => nice(other),
        },
        "triggers" => "Triggers".to_string(),
        "" => "Other".to_string(),
        d => nice(d),
    }
}

/// Opener/closer keyword pairs (`for`..`end`, `function`..`end`, `repeat`..`until`).
fn block_pairs(doc: &LuaDoc) -> Vec<(Span, Span)> {
    #[derive(PartialEq)]
    enum K {
        Plain,
        Loop,
        LoopDo,
        Repeat,
    }
    let mut stack: Vec<(Span, K)> = vec![];
    let mut out = vec![];
    for t in &doc.tokens {
        if t.kind != TokKind::Keyword {
            continue;
        }
        match t.text(&doc.src) {
            "function" | "if" => stack.push((t.span, K::Plain)),
            "for" | "while" => stack.push((t.span, K::Loop)),
            "do" => match stack.last_mut() {
                Some((_, k)) if *k == K::Loop => *k = K::LoopDo,
                _ => stack.push((t.span, K::Plain)),
            },
            "repeat" => stack.push((t.span, K::Repeat)),
            "end" => {
                if let Some(pos) = stack.iter().rposition(|(_, k)| *k != K::Repeat) {
                    let (o, _) = stack.remove(pos);
                    out.push((o, t.span));
                }
            }
            "until" => {
                if let Some(pos) = stack.iter().rposition(|(_, k)| *k == K::Repeat) {
                    let (o, _) = stack.remove(pos);
                    out.push((o, t.span));
                }
            }
            _ => {}
        }
    }
    out
}

/// How the lines after `line` are indented relative to it: +1 after a block opener. The line
/// itself is assumed to be indented correctly already (closers at its start were dedented).
pub fn lua_block_delta(line: &str) -> i32 {
    let lexed = lex(line);
    let mut net = 0i32;
    let mut lead = 0i32;
    let mut at_start = true;
    for t in &lexed.tokens {
        if t.is_trivia() || t.kind == TokKind::Eof {
            continue;
        }
        let text = t.text(line);
        // (delta, closes a block)
        let (d, closer) = match (t.kind, text) {
            // `for ... do` and `while ... do` open with their `do`
            (TokKind::Keyword, "function" | "repeat" | "then" | "do") => (1, false),
            (TokKind::Keyword, "end" | "until" | "elseif") => (-1, true),
            // closes the then-branch and opens the else-branch
            (TokKind::Keyword, "else") => (0, true),
            (TokKind::Punct, "(" | "{" | "[") => (1, false),
            (TokKind::Punct, ")" | "}" | "]") => (-1, true),
            _ => (0, false),
        };
        if closer && at_start {
            lead += 1;
        } else {
            at_start = false;
        }
        net += d;
    }
    (net + lead).clamp(0, 1)
}

pub struct LuaAnalysis {
    pub doc: LuaDoc,
    index: Arc<ApiIndex>,
    map: Mapper,
    tokens: Vec<(Span, HlClass)>,
    blocks: Vec<(Span, Span)>,
    refs: OnceCell<Vec<ScriptRef>>,
    poss: OnceCell<Vec<ScriptPos>>,
}

impl LuaAnalysis {
    fn refs(&self) -> &[ScriptRef] {
        self.refs
            .get_or_init(|| lua::references(&self.doc, &self.index))
    }

    fn poss(&self) -> &[ScriptPos] {
        self.poss
            .get_or_init(|| lua::positions(&self.doc, &self.index))
    }

    pub fn document(&self) -> &LuaDoc {
        &self.doc
    }

    fn call_param_at(&self, offset: usize) -> Option<(String, String, Option<Span>)> {
        let env = kfx_script::lua::infer::Env::new(&self.doc, &self.index);
        let c = kfx_script::lua::services::call_at(&self.doc, &env, offset)?;
        let sig = c.sig?;
        let pi = c.arg_index as i32 + sig.shift;
        if pi < 0 {
            return None;
        }
        let p = sig.def.params.get(pi as usize)?;
        let span = c.args.items.get(c.arg_index).map(|a| a.span);
        Some((p.name.clone(), p.ty.clone(), span))
    }
}

fn norm(s: &str) -> String {
    s.to_ascii_lowercase().replace('_', "")
}

fn ref_pick(kind: RefKind) -> PickKind {
    match kind {
        RefKind::ActionPoint => PickKind::ActionPoint,
        RefKind::HeroGate => PickKind::HeroGate,
        RefKind::SubtileX | RefKind::SubtileY => PickKind::Subtile,
        RefKind::SlabX | RefKind::SlabY => PickKind::Slab,
    }
}

impl Analysis for LuaAnalysis {
    fn tokens(&self) -> &[(Span, HlClass)] {
        &self.tokens
    }

    fn validate(&self, host: &dyn ConstantProvider) -> Validation {
        let diagnostics = lua::diagnostics(&self.doc, &self.index, host)
            .iter()
            .map(|d| {
                let mut x = Diagnostic::from(d);
                x.span = self.map.span(d.span);
                x
            })
            .collect();
        Validation {
            diagnostics,
            meters: vec![],
        }
    }

    fn completions(&self, offset: usize, host: &dyn ConstantProvider) -> Vec<Completion> {
        let off = self.map.into_u8(offset);
        lua::completions(&self.doc, off, &self.index, host)
            .into_iter()
            // string values replace the whole literal content: keep those that match what is typed
            .filter(|c| {
                c.kind != LuaCompletionKind::Constant || {
                    let typed = self
                        .doc
                        .src
                        .get(c.replace.start..off.max(c.replace.start))
                        .unwrap_or("");
                    c.label
                        .to_ascii_lowercase()
                        .starts_with(&typed.to_ascii_lowercase())
                }
            })
            .map(|c| {
                let kind = match c.kind {
                    LuaCompletionKind::Function | LuaCompletionKind::Method => {
                        CompletionKind::Command
                    }
                    LuaCompletionKind::Local
                    | LuaCompletionKind::Global
                    | LuaCompletionKind::Class => CompletionKind::Variable,
                    LuaCompletionKind::Field => CompletionKind::Field,
                    LuaCompletionKind::Keyword | LuaCompletionKind::Annotation => {
                        CompletionKind::Keyword
                    }
                    LuaCompletionKind::Snippet => CompletionKind::Snippet,
                    LuaCompletionKind::Constant => CompletionKind::Constant,
                    LuaCompletionKind::Hook => CompletionKind::Hook,
                    LuaCompletionKind::Type => CompletionKind::Type,
                };
                // the flag comes from the declaration in the index; functions without declaration
                // data (local functions) are assumed to take arguments
                let callable = match c.kind {
                    LuaCompletionKind::Function | LuaCompletionKind::Method
                        if c.insert == c.label =>
                    {
                        Some(c.callable.unwrap_or(true))
                    }
                    _ => None,
                };
                Completion {
                    label: c.label,
                    kind,
                    insert: c.insert,
                    detail: c.detail,
                    doc: c.doc,
                    replace: self.map.span(c.replace),
                    deprecated: c.deprecated,
                    callable,
                    action: None,
                }
            })
            .collect()
    }

    fn signature_help(&self, offset: usize) -> Option<SignatureHelp> {
        let s = lua::signature_help(&self.doc, self.map.into_u8(offset), &self.index)?;
        let params = s
            .params
            .iter()
            .map(|p| SigParam {
                name: p.name.clone(),
                detail: if p.ty.is_empty() || p.ty == "any" {
                    None
                } else {
                    Some(p.ty.clone())
                },
                doc: p.doc.clone(),
                optional: p.optional,
                label_range: p.label_range,
            })
            .collect();
        Some(SignatureHelp {
            command: s.name,
            label: s.label,
            doc: s.doc,
            params,
            active: s.active,
        })
    }

    fn string_arg_span(&self, offset: usize) -> Option<Span> {
        lua::string_arg_at(&self.doc, &self.index, self.map.into_u8(offset))
            .map(|a| self.map.span(a.span))
    }

    fn hover_with(
        &self,
        offset: usize,
        strings: Option<&crate::StringLookup>,
    ) -> Option<HoverInfo> {
        if let Some(lookup) = strings {
            if let Some(arg) = lua::string_arg_at(&self.doc, &self.index, self.map.into_u8(offset))
            {
                if let Some(mut h) = crate::strings::hover(&arg, lookup) {
                    h.span = self.map.span(h.span);
                    return Some(h);
                }
            }
        }
        self.hover(offset)
    }

    fn completions_with(
        &self,
        offset: usize,
        host: &dyn ConstantProvider,
        strings: Option<&crate::StringLookup>,
    ) -> Vec<Completion> {
        if let Some(lookup) = strings {
            let off = self.map.into_u8(offset);
            if let Some(arg) = lua::string_arg_at(&self.doc, &self.index, off) {
                let typed = self
                    .doc
                    .src
                    .get(
                        arg.span.start
                            ..off
                                .max(arg.span.start)
                                .min(arg.span.end.max(arg.span.start)),
                    )
                    .unwrap_or("")
                    .to_string();
                let span = self.map.span(arg.span);
                return crate::strings::completions(&arg, &typed, lookup)
                    .into_iter()
                    .map(|c| Completion { replace: span, ..c })
                    .collect();
            }
        }
        self.completions(offset, host)
    }

    fn hover(&self, offset: usize) -> Option<HoverInfo> {
        let h = lua::hover(&self.doc, self.map.into_u8(offset), &self.index)?;
        Some(hover_from_markdown(self.map.span(h.span), &h.markdown))
    }

    fn outline(&self) -> Vec<OutlineItem> {
        fn conv(map: &Mapper, it: &LuaOutlineItem) -> OutlineItem {
            let (tag, tone) = match it.kind {
                LuaOutlineKind::Function => ("fn", Tone::Function),
                LuaOutlineKind::LocalFunction => ("local", Tone::Muted),
                LuaOutlineKind::Method => ("meth", Tone::Function),
                LuaOutlineKind::Hook => ("hook", Tone::Keyword),
                LuaOutlineKind::Trigger => ("trig", Tone::Constant),
                LuaOutlineKind::Class => ("class", Tone::Good),
            };
            OutlineItem {
                tag,
                tone,
                label: it.label.clone(),
                span: map.span(it.span),
                line: it.line,
                children: it.children.iter().map(|c| conv(map, c)).collect(),
            }
        }
        lua::outline(&self.doc)
            .iter()
            .map(|i| conv(&self.map, i))
            .collect()
    }

    fn map_references(&self) -> Vec<MapReference> {
        let mut out = vec![];
        for r in self.refs() {
            let target = match r.kind {
                RefKind::ActionPoint => MapTarget::ActionPoint(r.value),
                RefKind::HeroGate => MapTarget::HeroGate(r.value),
                _ => continue,
            };
            out.push(MapReference {
                span: self.map.span(r.span),
                target,
                command: r.command.clone(),
                line: self.doc.line_of(r.span.start),
            });
        }
        for p in self.poss() {
            let target = if p.slab {
                MapTarget::Slab { x: p.x, y: p.y }
            } else {
                MapTarget::Subtile { x: p.x, y: p.y }
            };
            out.push(MapReference {
                span: self.map.span(p.x_span.join(p.y_span)),
                target,
                command: p.command.clone(),
                line: self.doc.line_of(p.x_span.start),
            });
        }
        out.sort_by_key(|r| r.span.start);
        out
    }

    fn block_match(&self, offset: usize) -> Option<(Span, Span)> {
        self.blocks
            .iter()
            .filter(|(a, b)| a.touches(offset) || b.touches(offset))
            .min_by_key(|(a, b)| b.end - a.start)
            .copied()
    }

    fn pick_target(&self, offset: usize) -> Option<PickKind> {
        let off = self.map.into_u8(offset);
        if let Some(r) = self.refs().iter().find(|r| r.span.touches(off)) {
            return Some(ref_pick(r.kind));
        }
        let (name, ty, _) = self.call_param_at(off)?;
        let n = norm(&name);
        if n.contains("herogate") {
            Some(PickKind::HeroGate)
        } else if ty.contains("location") {
            Some(PickKind::Location)
        } else if matches!(n.as_str(), "actionpoint" | "ap") || ty.contains("actionpoint") {
            Some(PickKind::ActionPoint)
        } else if n.contains("stl") || n.contains("subtile") {
            Some(PickKind::Subtile)
        } else if n.contains("slb") || n.contains("slab") {
            Some(PickKind::Slab)
        } else {
            None
        }
    }

    fn pick_replacements(&self, offset: usize, value: PickValue) -> Vec<(Span, String)> {
        let off = self.map.into_u8(offset);
        let hit = self.refs().iter().find(|r| r.span.touches(off));
        let kind = self.pick_target(offset);
        let single = |span: Option<Span>, text: String| -> Vec<(Span, String)> {
            vec![(self.map.span(span.unwrap_or(Span::new(off, off))), text)]
        };
        let slot = || self.call_param_at(off).and_then(|x| x.2);
        match (kind, value) {
            (Some(PickKind::ActionPoint | PickKind::Location), PickValue::ActionPoint(n)) => {
                single(hit.map(|r| r.span).or_else(slot), n.to_string())
            }
            (Some(PickKind::HeroGate), PickValue::HeroGate(n)) => {
                single(hit.map(|r| r.span).or_else(slot), n.abs().to_string())
            }
            (Some(PickKind::Location), PickValue::HeroGate(n)) => {
                single(hit.map(|r| r.span).or_else(slot), (-n.abs()).to_string())
            }
            (
                Some(k @ (PickKind::Subtile | PickKind::Slab)),
                PickValue::Subtile { x, y } | PickValue::Slab { x, y },
            ) => {
                let slab = k == PickKind::Slab;
                if let Some(p) = self
                    .poss()
                    .iter()
                    .find(|p| p.slab == slab && (p.x_span.touches(off) || p.y_span.touches(off)))
                {
                    vec![
                        (self.map.span(p.x_span), x.to_string()),
                        (self.map.span(p.y_span), y.to_string()),
                    ]
                } else {
                    single(hit.map(|r| r.span).or_else(slot), format!("{x}, {y}"))
                }
            }
            _ => vec![],
        }
    }

    fn arg_nav(&self, offset: usize) -> Option<ArgNav> {
        let off = self.map.into_u8(offset);
        let env = kfx_script::lua::infer::Env::new(&self.doc, &self.index);
        let c = kfx_script::lua::services::call_at(&self.doc, &env, off)?;
        let after = c.args.close.map(|x| x.end).unwrap_or(c.args.span.end);
        Some(ArgNav {
            args: c.args.items.iter().map(|a| self.map.span(a.span)).collect(),
            after: self.map.out(after),
        })
    }

    fn block_delta(&self, line: &str) -> i32 {
        lua_block_delta(line)
    }

    fn definition(&self, offset: usize) -> Option<Definition> {
        let loc = lua::definition(&self.doc, self.map.into_u8(offset), &self.index)?;
        Some(match loc.file {
            None => Definition {
                file: None,
                span: self.map.span(loc.span),
            },
            Some(f) => Definition {
                file: Some(f),
                span: loc.span,
            },
        })
    }

    fn status_items(&self) -> Vec<StatusItem> {
        let mut out = vec![];
        let outline = lua::outline(&self.doc);
        let mut triggers = 0;
        for it in &outline {
            match it.kind {
                LuaOutlineKind::Hook => out.push(StatusItem {
                    text: it.label.clone(),
                    tone: Tone::Good,
                    tooltip: format!("Hook function, line {}", it.line),
                    at: Some(self.map.span(it.span)),
                }),
                LuaOutlineKind::Trigger => triggers += 1,
                _ => {}
            }
        }
        if out.is_empty() {
            out.push(StatusItem {
                text: "no hook functions".into(),
                tone: Tone::Muted,
                tooltip: "The game calls global functions such as OnGameStart by name".into(),
                at: None,
            });
        }
        if triggers > 0 {
            out.push(StatusItem {
                text: format!("{triggers} trigger{}", if triggers == 1 { "" } else { "s" }),
                tone: Tone::Constant,
                tooltip: "Trigger registrations (RegisterTimerEvent ...)".into(),
                at: None,
            });
        }
        out
    }

    fn format(&self, indent: &str) -> Option<Vec<u8>> {
        let out = lua::format(
            &self.doc,
            &LuaFormatOptions {
                indent: indent.to_string(),
            },
        );
        Some(if self.map.tables.is_none() {
            out.into_bytes()
        } else {
            latin1::encode(&out)
        })
    }
}

/// Flatten the markdown of the Lua hover into a card: the fenced line is the title, bullet lines
/// ``- `name` (type): doc`` become parameters.
pub fn hover_from_markdown(span: Span, md: &str) -> HoverInfo {
    let mut title = None;
    let mut summary: Vec<String> = vec![];
    let mut params = vec![];
    let mut in_fence = false;
    for l in md.lines() {
        if l.starts_with("```") {
            in_fence = !in_fence;
        } else if in_fence {
            title.get_or_insert_with(String::new).push_str(l);
        } else if let Some(rest) = l.strip_prefix("- `") {
            if let Some((name, tail)) = rest.split_once('`') {
                params.push((name.to_string(), tail.trim().to_string()));
                continue;
            }
            summary.push(l.replace(['`', '*'], ""));
        } else {
            let t = l.replace(['`', '*'], "");
            if !(t.is_empty() && summary.last().map_or(true, |s| s.is_empty())) {
                summary.push(t);
            }
        }
    }
    while summary.last().map_or(false, |s| s.is_empty()) {
        summary.pop();
    }
    HoverInfo {
        span,
        title,
        summary: summary.join("\n"),
        params,
        examples: vec![],
        link: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textops::{apply, newline, Transaction};

    #[test]
    fn block_delta_of_lines() {
        for (line, d) in [
            ("function OnGameStart()", 1),
            ("\tif x > 1 then", 1),
            ("for i = 1, 10 do", 1),
            ("while x do", 1),
            ("do", 1),
            ("repeat", 1),
            ("local f = function(a)", 1),
            ("RegisterTimerEvent(function()", 1),
            ("local t = {", 1),
            ("else", 1),
            ("elseif y then", 1),
            ("end", 0),
            ("\tend", 0),
            ("end)", 0),
            ("until x", 0),
            ("}", 0),
            ("if x then return end", 0),
            ("x = foo(1, 2)", 0),
            ("-- function foo() then", 0),
            ("local s = \"do end if\"", 0),
            ("", 0),
            ("local x = 1 -- if then", 0),
        ] {
            assert_eq!(lua_block_delta(line), d, "{line:?}");
        }
    }

    fn enter(text: &str, _caret: usize) -> String {
        let caret = text.len();
        let line_start = text[..caret].rfind('\n').map_or(0, |i| i + 1);
        let line_end = text[caret..].find('\n').map_or(text.len(), |i| caret + i);
        let delta = lua_block_delta(&text[line_start..line_end]);
        let tx: Transaction = newline(text.as_bytes(), (caret, caret), "\t", delta);
        let mut b = text.as_bytes().to_vec();
        apply(&mut b, &tx);
        String::from_utf8(b).unwrap()
    }

    #[test]
    fn enter_indents_after_openers_and_keeps_level_after_closers() {
        assert_eq!(enter("function F()", 12), "function F()\n\t");
        assert_eq!(
            enter("function F()\n\tif x then", 25),
            "function F()\n\tif x then\n\t\t"
        );
        assert_eq!(
            enter("function F()\n\tfoo()", 19),
            "function F()\n\tfoo()\n\t"
        );
        // `end` was dedented while typing: Enter keeps its level
        assert_eq!(
            enter("function F()\n\tif x then\n\tend", 30),
            "function F()\n\tif x then\n\tend\n\t"
        );
        assert_eq!(enter("if x then\nelse", 15), "if x then\nelse\n\t");
        assert_eq!(enter("t = {", 5), "t = {\n\t");
    }

    #[test]
    fn mapper_round_trips_offsets() {
        let bytes = b"a\xe9b\xffc";
        let (m, s) = Mapper::new(bytes);
        assert!(m.tables.is_some());
        assert_eq!(s.chars().count(), 5);
        // offsets of b and c in the decoded string map back to byte offsets 2 and 4
        let ub = s.find('b').unwrap();
        let uc = s.find('c').unwrap();
        assert_eq!((m.out(ub), m.out(uc)), (2, 4));
        assert_eq!((m.into_u8(2), m.into_u8(4)), (ub, uc));
        assert_eq!(m.out(s.len()), 5);
        let (m, _) = Mapper::new(b"plain \xc3\xa9");
        assert!(m.tables.is_none());
    }
}
