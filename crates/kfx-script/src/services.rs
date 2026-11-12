//! Editor services: completions, signature help, hover, highlighting, outline, map references
//! and the formatter. All are pure functions over a parsed [`Document`].
use crate::consts::{Env, Extras};
use crate::diag::Span;
use crate::doc::{ArgTok, Command, Document, Line, LineKind, TokKind};
use crate::overlay::{ArgKind, Overlay};
use crate::provider::ConstantProvider;
use crate::tables::Tables;
use crate::validate::is_if_command;

// ---------------------------------------------------------------------------------------------
// Context helpers

/// Where the cursor is inside a line.
#[derive(Clone, Debug)]
pub enum Context<'a> {
    /// Nothing typed yet or typing the command name; `word` is the span of the prefix.
    Command {
        word: Span,
    },
    /// Inside the parentheses of `cmd`, at argument `slot`; `word` is the prefix being typed.
    Arg {
        cmd: &'a Command,
        slot: usize,
        word: Span,
    },
    Comment,
    Other,
}

fn word_span(doc: &Document, offset: usize) -> Span {
    let mut s = offset.min(doc.src.len());
    let e = s;
    let is_w = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'[' | b']' | b'-');
    while s > 0 && is_w(doc.src[s - 1]) {
        s -= 1;
    }
    let mut e2 = e;
    while e2 < doc.src.len() && is_w(doc.src[e2]) {
        e2 += 1;
    }
    Span::new(s, e2)
}

/// Slot (argument index as the game counts it) at `offset`, from the raw tokens of the line.
pub fn slot_at(doc: &Document, line: &Line, open: Span, offset: usize) -> usize {
    let mut depth = 0i32;
    let mut slot = 0usize;
    let mut prev_sig: Option<TokKind> = None;
    for t in &doc.tokens[line.toks.0..line.toks.1] {
        if t.span.start < open.start {
            continue;
        }
        if t.span.start >= offset && !(t.span.start == offset && false) {
            break;
        }
        match t.kind {
            TokKind::LParen => depth += 1,
            TokKind::RParen => depth -= 1,
            TokKind::Comma if depth == 1 => slot += 1,
            TokKind::Operator | TokKind::Tilde if depth == 1 => {
                if !matches!(
                    prev_sig,
                    Some(TokKind::Comma) | Some(TokKind::LParen) | None
                ) {
                    // operator occupies its own slot, the next value another one
                    slot += if t.span.end < offset { 2 } else { 1 };
                }
            }
            _ => {}
        }
        if !t.kind.is_trivia() {
            prev_sig = Some(t.kind);
        }
    }
    slot
}

pub fn context_at(doc: &Document, offset: usize) -> Context<'_> {
    let Some(li) = doc.line_at(offset.min(doc.src.len())) else {
        return Context::Command {
            word: Span::new(offset, offset),
        };
    };
    let line = &doc.lines[li];
    // inside comment?
    if let Some(b) = line.block {
        if offset > b.start && offset < b.end {
            return Context::Comment;
        }
    }
    if let Some(r) = line.rem {
        if offset > r.start {
            return Context::Comment;
        }
    }
    let word = word_span(doc, offset);
    match &line.cmd {
        None => Context::Command { word },
        Some(cmd) => {
            if offset <= cmd.name_span.end {
                return Context::Command {
                    word: cmd.name_span.join(word),
                };
            }
            let open = cmd.open.or_else(|| {
                doc.tokens[line.toks.0..line.toks.1]
                    .iter()
                    .find(|t| t.kind == TokKind::LParen)
                    .map(|t| t.span)
            });
            match open {
                Some(o) if offset >= o.end => {
                    if let Some(c) = cmd.close {
                        if offset > c.start && cmd.syntax_ok {
                            return Context::Other;
                        }
                    }
                    let mut slot = slot_at(doc, line, o, offset);
                    // `IF(P, VAR |`: after a finished argument and a blank, the operator slot comes
                    // next
                    if word.is_empty() || word.end == offset && word.start == offset {
                        let prev = doc.tokens[line.toks.0..line.toks.1]
                            .iter()
                            .rev()
                            .find(|t| t.span.end <= offset && !t.kind.is_trivia());
                        let blank_before =
                            offset > 0 && matches!(doc.src[offset - 1], b' ' | b'\t');
                        if let (Some(p), Some(d)) = (prev, cmd.desc) {
                            if blank_before
                                && matches!(p.kind, TokKind::Ident | TokKind::Number | TokKind::Str)
                                && p.span.start > o.end
                            {
                                let mut src = 0;
                                let mut dst = 0;
                                while src < crate::tables::ARGS_COUNT && dst <= slot {
                                    if d.letter(src) != b'!' {
                                        dst += 1;
                                    }
                                    src += 1;
                                }
                                if d.letter(src) == b'O' {
                                    slot += 1;
                                }
                            }
                        }
                    }
                    Context::Arg { cmd, slot, word }
                }
                _ => Context::Other,
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Completions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Command,
    Constant,
    Snippet,
}

#[derive(Clone, Debug)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    /// Text to insert in place of `replace` (snippets may contain `$0` as the final cursor).
    pub insert: String,
    pub detail: String,
    /// Longer description (property docs from the config schema); empty when there is none.
    pub doc: String,
    pub replace: Span,
}

fn snippets(replace: Span) -> Vec<Completion> {
    let s = |label: &str, insert: &str, detail: &str| Completion {
        label: label.into(),
        kind: CompletionKind::Snippet,
        insert: insert.into(),
        detail: detail.into(),
        doc: String::new(),
        replace,
    };
    vec![
        s(
            "IF...ENDIF",
            "IF(PLAYER0, MONEY >= 1000)\n\t$0\nENDIF",
            "Conditional block",
        ),
        s(
            "IF flag block",
            "IF(PLAYER0, FLAG0 == 1)\n\t$0\nENDIF",
            "Conditional block on a flag",
        ),
        s(
            "IF action point block",
            "IF_ACTION_POINT(1, PLAYER0)\n\t$0\nENDIF",
            "Block run when an action point is triggered",
        ),
        s(
            "win condition",
            "IF(PLAYER0, ALL_DUNGEONS_DESTROYED == 1)\n\tWIN_GAME\nENDIF",
            "Win when all enemy dungeons are destroyed",
        ),
        s(
            "lose condition",
            "IF(PLAYER0, DUNGEON_DESTROYED == 1)\n\tLOSE_GAME\nENDIF",
            "Lose when the dungeon heart is destroyed",
        ),
        s(
            "party",
            "CREATE_PARTY(NAME)\n\tADD_TO_PARTY(NAME, KNIGHT, 1, 0, ATTACK_ENEMIES, 0)\n$0",
            "Party definition",
        ),
        s(
            "level version header",
            "LEVEL_VERSION(1)\n$0",
            "Selects the KeeperFX command set",
        ),
    ]
}

/// Completions at byte `offset`.
pub fn completions(
    doc: &Document,
    offset: usize,
    provider: &dyn ConstantProvider,
) -> Vec<Completion> {
    let extras = Extras::default();
    let env = Env {
        provider,
        extras: &extras,
        version: doc.version,
    };
    let tables = Tables::get();
    let ov = Overlay::get();
    match context_at(doc, offset) {
        Context::Comment | Context::Other => vec![],
        Context::Command { word } => {
            let prefix = doc
                .text_lossy(Span::new(word.start, offset.min(word.end)))
                .to_ascii_uppercase();
            let dk1_only = doc.version <= 0;
            let mut v: Vec<Completion> = tables
                .commands
                .iter()
                .filter(|c| c.name != "REM" && c.name.starts_with(&prefix))
                .map(|c| {
                    let mut detail = ov.cmd(&c.name).map(|i| i.doc.clone()).unwrap_or_default();
                    if dk1_only && !tables.dk1.iter().any(|d| d.name == c.name) {
                        detail = format!("[needs LEVEL_VERSION(1)] {detail}");
                    }
                    Completion {
                        label: c.name.clone(),
                        kind: CompletionKind::Command,
                        insert: c.name.clone(),
                        detail,
                        doc: String::new(),
                        replace: word,
                    }
                })
                .collect();
            if prefix.is_empty()
                || "IF".starts_with(&prefix)
                || "PARTY".starts_with(&prefix)
                || "WIN".starts_with(&prefix)
                || "LOSE".starts_with(&prefix)
                || "LEVEL".starts_with(&prefix)
            {
                v.extend(snippets(word).into_iter().filter(|s| {
                    prefix.is_empty()
                        || s.label.to_ascii_uppercase().starts_with(&prefix)
                        || s.insert.starts_with(&prefix)
                }));
            }
            v
        }
        Context::Arg { cmd, slot, word } => {
            let prefix = doc
                .text_lossy(Span::new(word.start, offset.min(word.end)))
                .to_ascii_uppercase();
            let Some(desc) = cmd.desc else { return vec![] };
            let (kind, ext) = arg_kind_at(cmd, slot);
            let _ = desc;
            let cands = arg_candidates(cmd, slot, kind, ext, &env);
            cands
                .into_iter()
                .filter(|(c, _)| c.to_ascii_uppercase().starts_with(&prefix))
                .map(|(c, doc)| Completion {
                    label: c.clone(),
                    kind: CompletionKind::Constant,
                    insert: c,
                    detail: if doc.is_empty() {
                        kind.name().to_string()
                    } else {
                        crate::util::first_sentence(&doc)
                    },
                    doc,
                    replace: word,
                })
                .collect()
        }
    }
}

/// Names valid for argument `slot` of `cmd` with a description where the config schema has one:
/// the `SET_GAME_RULE` rules and their values, the properties of `SET_*_CONFIGURATION` and their
/// values, the computer processes/checks/events, and the constants of [`Env::candidates`].
pub fn arg_candidates(
    cmd: &Command,
    slot: usize,
    kind: ArgKind,
    ext: bool,
    env: &Env,
) -> Vec<(String, String)> {
    use crate::provider::ConstKind;
    use crate::schema::{ConfigTarget, Prop, Schema, ValueKind};
    let schema = Schema::get();
    let members = |p: &Prop, i: usize| -> Vec<String> {
        match p.args.get(i).map(|a| &a.kind) {
            Some(ValueKind::Enum { members, .. }) | Some(ValueKind::Flags { members, .. }) => {
                members.clone()
            }
            _ => vec![],
        }
    };
    let plain = |v: Vec<String>| -> Vec<(String, String)> {
        v.into_iter().map(|n| (n, String::new())).collect()
    };
    let prop_of = |target: ConfigTarget| {
        cmd.args
            .iter()
            .find(|a| a.slot == 1)
            .and_then(|a| schema.prop(target, &a.text))
    };
    match (kind, cmd.name.as_str()) {
        (ArgKind::RuleName, _) => schema
            .rules()
            .iter()
            .map(|r| (r.name.clone(), r.doc.clone()))
            .collect(),
        (ArgKind::RuleValue, _) => {
            let rule = cmd
                .args
                .iter()
                .find(|a| a.slot == 0)
                .and_then(|a| schema.rule(&a.text));
            plain(rule.map(|r| members(r, 0)).unwrap_or_default())
        }
        (ArgKind::ConfigProperty, c) => match ConfigTarget::of_command(c) {
            Some(t) => schema
                .props(t)
                .iter()
                .map(|p| (p.name.clone(), p.doc.clone()))
                .collect(),
            None => vec![],
        },
        (ArgKind::ConfigValue, c) => match ConfigTarget::of_command(c).and_then(|t| prop_of(t)) {
            Some(p) if slot >= 2 => plain(members(p, slot - 2)),
            _ => vec![],
        },
        (ArgKind::ComputerName, c) => {
            let k = match c {
                "SET_COMPUTER_PROCESS" => ConstKind::ComputerProcess,
                "SET_COMPUTER_CHECKS" => ConstKind::ComputerCheck,
                _ => ConstKind::ComputerEvent,
            };
            plain(env.const_names(k))
        }
        _ => plain(env.candidates(kind, ext)),
    }
}

/// Kind and extended flag of argument `slot` of `cmd` (overlay first, letters as fallback).
pub fn arg_kind_at(cmd: &Command, slot: usize) -> (ArgKind, bool) {
    let ov = Overlay::get();
    let desc = cmd.desc;
    let mut ext = false;
    let mut letter = 0u8;
    if let Some(d) = desc {
        // walk letters like the parser to find the type letter and extended flag of this slot
        let mut src = 0usize;
        let mut dst = 0usize;
        while src < crate::tables::ARGS_COUNT {
            let c = d.letter(src);
            if c == b'!' {
                src += 1;
                continue;
            }
            if dst == slot {
                letter = c;
                ext = d.letter(src + 1) == b'!';
                break;
            }
            if d.letter(src + 1) == b'+' {
                // repeat: same letter for all remaining slots
                letter = c;
                break;
            }
            dst += 1;
            src += 1;
        }
    }
    let lk = ArgKind::from_letter(letter);
    let k = match ov.arg(&cmd.name, slot) {
        Some(o)
            if matches!(o.kind, ArgKind::Text | ArgKind::Number)
                && !matches!(lk, ArgKind::Text | ArgKind::Number) =>
        {
            lk
        }
        Some(o) => o.kind,
        None => lk,
    };
    (k, ext)
}

// ---------------------------------------------------------------------------------------------
// Signature help

#[derive(Clone, Debug)]
pub struct ParamInfo {
    pub name: String,
    pub kind: ArgKind,
    pub optional: bool,
    pub guessed: bool,
    /// Byte range of this parameter inside `SignatureHelp::label`.
    pub label_range: (usize, usize),
}

#[derive(Clone, Debug)]
pub struct SignatureHelp {
    pub command: String,
    pub label: String,
    pub doc: String,
    pub params: Vec<ParamInfo>,
    /// Index into `params`.
    pub active: usize,
}

pub fn signature_of(name: &str) -> Option<SignatureHelp> {
    let tables = Tables::get();
    let desc = tables
        .find_full(name)
        .or_else(|| tables.dk1.iter().find(|c| c.name == name))?;
    let ov = Overlay::get();
    let info = ov.cmd(name);
    let letters = desc.arg_letters();
    let required = desc.required();
    let mut label = format!("{name}(");
    let mut params = vec![];
    for (i, &l) in letters.iter().enumerate() {
        let (pname, kind, guessed) = match info.and_then(|x| x.args.get(i)) {
            Some(a) => (a.name.clone(), a.kind, a.guessed),
            None => (format!("arg{}", i + 1), ArgKind::from_letter(l), true),
        };
        let optional = i >= required;
        if i > 0 {
            label.push_str(", ");
        }
        let start = label.len();
        let shown = if optional {
            format!("[{pname}]")
        } else {
            pname.clone()
        };
        label.push_str(&shown);
        params.push(ParamInfo {
            name: pname,
            kind,
            optional,
            guessed,
            label_range: (start, label.len()),
        });
    }
    if desc.repeats() {
        label.push_str(", ...");
    }
    label.push(')');
    Some(SignatureHelp {
        command: name.to_string(),
        label,
        doc: info.map(|i| i.doc.clone()).unwrap_or_default(),
        params,
        active: 0,
    })
}

pub fn signature_help(doc: &Document, offset: usize) -> Option<SignatureHelp> {
    match context_at(doc, offset) {
        Context::Arg { cmd, slot, .. } => {
            let mut sig = signature_of(&cmd.name)?;
            sig.active = slot.min(sig.params.len().saturating_sub(1));
            Some(sig)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Hover

#[derive(Clone, Debug)]
pub struct Hover {
    pub span: Span,
    pub markdown: String,
}

pub fn hover(doc: &Document, offset: usize) -> Option<Hover> {
    let li = doc.line_at(offset)?;
    let line = &doc.lines[li];
    let cmd = line.cmd.as_ref()?;
    if cmd.name_span.touches(offset) {
        let sig = signature_of(&cmd.name);
        let md = match sig {
            Some(s) => {
                let mut m = format!("```\n{}\n```\n{}", s.label, s.doc);
                for p in &s.params {
                    m.push_str(&format!(
                        "\n- `{}`{}: {}{}",
                        p.name,
                        if p.optional { " (optional)" } else { "" },
                        p.kind.name(),
                        if p.guessed { " (name guessed)" } else { "" }
                    ));
                }
                m
            }
            None => format!("Unknown command `{}`", cmd.name),
        };
        return Some(Hover {
            span: cmd.name_span,
            markdown: md,
        });
    }
    for a in &cmd.args {
        if a.span.touches(offset) {
            if let Some(f) = &a.func {
                for fa in &f.args {
                    if fa.span.touches(offset) {
                        return Some(Hover {
                            span: fa.span,
                            markdown: format!("`{}`: argument of {}", fa.text, f.name),
                        });
                    }
                }
                let what = match f.name.as_str() {
                    "RANDOM" => "RANDOM(min, max): random number (level version 0 only)",
                    "DRAWFROM" => {
                        "DRAWFROM(a, b~c, ...): draws one value from the list/ranges \
                        (level version 1+)"
                    }
                    _ => {
                        "IMPORT(player, CAMPAIGN_FLAGn): value of a campaign flag carried over \
                        from earlier levels"
                    }
                };
                return Some(Hover {
                    span: a.span,
                    markdown: what.into(),
                });
            }
            let (kind, _) = arg_kind_at(cmd, a.slot);
            let ov = Overlay::get();
            let name = ov
                .arg(&cmd.name, a.slot)
                .map(|o| o.name.clone())
                .unwrap_or_else(|| format!("arg{}", a.slot + 1));
            let mut md = format!("`{}`: {} `{}`", a.text, name, kind.name());
            {
                use crate::schema::{ConfigTarget, Schema};
                let prop = match kind {
                    ArgKind::RuleName => Schema::get().rule(&a.text),
                    ArgKind::ConfigProperty => ConfigTarget::of_command(&cmd.name)
                        .and_then(|t| Schema::get().prop(t, &a.text)),
                    _ => None,
                };
                if let Some(p) = prop.filter(|p| !p.doc.is_empty()) {
                    md.push_str(&format!("\n{}", p.doc));
                }
            }
            if matches!(kind, ArgKind::Location) {
                let t = a.text.as_str();
                if let Ok(n) = t.parse::<i64>() {
                    md.push_str(&if n > 0 {
                        format!(" - action point {n}")
                    } else {
                        format!(" - hero gate {}", -n)
                    });
                } else if t == "LAST_EVENT" {
                    md.push_str(" - location of the last event shown to the player");
                }
            }
            return Some(Hover {
                span: a.span,
                markdown: md,
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Highlighting

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenClass {
    Command,
    /// Flow control and structural commands (IF family, ENDIF, WIN_GAME, LOSE_GAME, ...).
    Keyword,
    /// `RANDOM`, `DRAWFROM`, `IMPORT`.
    Function,
    Number,
    String,
    Constant,
    Operator,
    Punctuation,
    Comment,
    Error,
}

fn is_keyword(name: &str) -> bool {
    is_if_command(name)
        || matches!(
            name,
            "ENDIF"
                | "WIN_GAME"
                | "LOSE_GAME"
                | "NEXT_COMMAND_REUSABLE"
                | "LEVEL_VERSION"
                | "CREATE_PARTY"
        )
}

/// Classified spans in source order; whitespace is not reported.
pub fn highlight(doc: &Document) -> Vec<(Span, TokenClass)> {
    use std::collections::HashMap;
    let mut roles: HashMap<usize, TokenClass> = HashMap::new();
    for line in &doc.lines {
        if let Some(cmd) = &line.cmd {
            let cls = if cmd.desc.is_none() {
                TokenClass::Error
            } else if is_keyword(&cmd.name) {
                TokenClass::Keyword
            } else {
                TokenClass::Command
            };
            roles.insert(cmd.name_span.start, cls);
            let mut stack: Vec<&crate::doc::Arg> = cmd.args.iter().collect();
            while let Some(a) = stack.pop() {
                let c = match a.tok {
                    ArgTok::Operator | ArgTok::Tilde => TokenClass::Operator,
                    ArgTok::Number => TokenClass::Number,
                    ArgTok::Str => TokenClass::String,
                    ArgTok::Ident => {
                        if a.func.is_some() {
                            TokenClass::Function
                        } else if a.text.bytes().all(|b| b.is_ascii_digit()) {
                            TokenClass::Number
                        } else {
                            TokenClass::Constant
                        }
                    }
                };
                roles.insert(a.span.start, c);
                if let Some(f) = &a.func {
                    stack.extend(f.args.iter());
                }
            }
        }
    }
    let mut out = Vec::new();
    for t in &doc.tokens {
        let c = match t.kind {
            TokKind::Whitespace | TokKind::Newline | TokKind::NonAscii => continue,
            TokKind::LineComment | TokKind::BlockComment => TokenClass::Comment,
            TokKind::Invalid | TokKind::UnterminatedStr => TokenClass::Error,
            TokKind::LParen | TokKind::RParen | TokKind::Comma => TokenClass::Punctuation,
            TokKind::Operator | TokKind::Tilde => TokenClass::Operator,
            TokKind::Number => TokenClass::Number,
            TokKind::Str => TokenClass::String,
            TokKind::Ident => roles.get(&t.span.start).copied().unwrap_or_else(|| {
                if doc.text(t.span).iter().all(|b| b.is_ascii_digit()) {
                    TokenClass::Number
                } else {
                    TokenClass::Constant
                }
            }),
        };
        let c = match (t.kind, roles.get(&t.span.start)) {
            (TokKind::Number | TokKind::Str, Some(&r)) if matches!(r, TokenClass::Error) => r,
            _ => c,
        };
        out.push((t.span, c));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Outline

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineKind {
    LevelVersion,
    IfBlock,
    Party,
    Win,
    Lose,
    NewType,
}

#[derive(Clone, Debug)]
pub struct OutlineItem {
    pub kind: OutlineKind,
    pub label: String,
    pub span: Span,
    pub line: usize,
    pub children: Vec<OutlineItem>,
}

/// Tree of IF blocks with the parties, win/lose conditions and new types they contain.
pub fn outline(doc: &Document) -> Vec<OutlineItem> {
    let mut roots: Vec<OutlineItem> = vec![];
    let mut stack: Vec<OutlineItem> = vec![];
    fn push(roots: &mut Vec<OutlineItem>, stack: &mut Vec<OutlineItem>, item: OutlineItem) {
        match stack.last_mut() {
            Some(top) => top.children.push(item),
            None => roots.push(item),
        }
    }
    for line in &doc.lines {
        let Some(cmd) = &line.cmd else { continue };
        if cmd.desc.is_none() || !cmd.syntax_ok {
            continue;
        }
        let line_no = doc.line_of(cmd.name_span.start);
        let text = doc.text_lossy(cmd.span).trim().to_string();
        let mk = |kind, label: String| OutlineItem {
            kind,
            label,
            span: cmd.span,
            line: line_no,
            children: vec![],
        };
        match cmd.name.as_str() {
            n if is_if_command(n) => stack.push(mk(OutlineKind::IfBlock, text)),
            "ENDIF" => {
                if let Some(mut top) = stack.pop() {
                    top.span = top.span.join(cmd.span);
                    push(&mut roots, &mut stack, top);
                }
            }
            "CREATE_PARTY" => push(
                &mut roots,
                &mut stack,
                mk(OutlineKind::Party, format!("party {}", cmd.text(0))),
            ),
            "WIN_GAME" => push(
                &mut roots,
                &mut stack,
                mk(OutlineKind::Win, "WIN_GAME".into()),
            ),
            "LOSE_GAME" => push(
                &mut roots,
                &mut stack,
                mk(OutlineKind::Lose, "LOSE_GAME".into()),
            ),
            "LEVEL_VERSION" => push(&mut roots, &mut stack, mk(OutlineKind::LevelVersion, text)),
            "NEW_TRAP_TYPE" | "NEW_OBJECT_TYPE" | "NEW_ROOM_TYPE" | "NEW_CREATURE_TYPE"
            | "COPY_CREATURE_TYPE" => push(&mut roots, &mut stack, mk(OutlineKind::NewType, text)),
            _ => {}
        }
    }
    while let Some(top) = stack.pop() {
        push(&mut roots, &mut stack, top);
    }
    roots
}

// ---------------------------------------------------------------------------------------------
// Map references

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    ActionPoint,
    HeroGate,
    SubtileX,
    SubtileY,
    SlabX,
    SlabY,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptRef {
    pub span: Span,
    pub kind: RefKind,
    pub value: i32,
    /// The command (DK script) or function (Lua) the value is an argument of.
    pub command: String,
}

/// A coordinate pair written in a command (for drawing markers and "pick from map").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptPos {
    pub x: i32,
    pub y: i32,
    pub x_span: Span,
    pub y_span: Span,
    /// True for slab coordinates, false for subtiles.
    pub slab: bool,
    pub command: String,
}

fn parse_int(s: &str) -> Option<i32> {
    let (v, whole) = crate::doc::strtol0(s);
    whole.then_some(v as i32)
}

/// Action points, hero gates and coordinates referenced by the script.
pub fn references(doc: &Document) -> Vec<ScriptRef> {
    let mut out = vec![];
    for line in &doc.lines {
        let Some(cmd) = &line.cmd else { continue };
        if cmd.desc.is_none() {
            continue;
        }
        let mut prev_head_ap = false;
        for a in &cmd.args {
            if a.func.is_some() {
                continue;
            }
            let (kind, _) = arg_kind_at(cmd, a.slot);
            let mut push = |k: RefKind, v: i32| {
                out.push(ScriptRef {
                    span: a.text_span,
                    kind: k,
                    value: v,
                    command: cmd.name.clone(),
                })
            };
            match kind {
                ArgKind::ApNumber => {
                    if let Some(v) = parse_int(&a.text) {
                        push(RefKind::ActionPoint, v);
                    }
                }
                ArgKind::HeroGate => {
                    if let Some(v) = parse_int(&a.text) {
                        push(RefKind::HeroGate, v.abs());
                    }
                }
                ArgKind::Location => {
                    if let Some(v) = parse_int(&a.text) {
                        if v > 0 {
                            push(RefKind::ActionPoint, v);
                        } else if v < 0 {
                            push(RefKind::HeroGate, -v);
                        }
                    }
                }
                ArgKind::Criterion => {
                    if let Some(rest) = a.text.strip_prefix("AT_ACTION_POINT[") {
                        if let Some(v) = rest.strip_suffix(']').and_then(parse_int) {
                            push(RefKind::ActionPoint, v);
                        }
                    }
                }
                ArgKind::SubtileX | ArgKind::SubtileY | ArgKind::SlabX | ArgKind::SlabY => {
                    if let Some(v) = parse_int(&a.text) {
                        let k = match kind {
                            ArgKind::SubtileX => RefKind::SubtileX,
                            ArgKind::SubtileY => RefKind::SubtileY,
                            ArgKind::SlabX => RefKind::SlabX,
                            _ => RefKind::SlabY,
                        };
                        push(k, v);
                    }
                }
                ArgKind::HeadFor => prev_head_ap = a.text == "ACTION_POINT",
                ArgKind::Number
                    if prev_head_ap
                        && matches!(
                            cmd.name.as_str(),
                            "ADD_TUNNELLER_TO_LEVEL" | "ADD_TUNNELLER_PARTY_TO_LEVEL"
                        ) =>
                {
                    if let Some(v) = parse_int(&a.text) {
                        push(RefKind::ActionPoint, v);
                    }
                    prev_head_ap = false;
                }
                _ => {}
            }
        }
    }
    out
}

/// Coordinate pairs (x then y arguments of the same command).
pub fn positions(doc: &Document) -> Vec<ScriptPos> {
    let mut out = vec![];
    for line in &doc.lines {
        let Some(cmd) = &line.cmd else { continue };
        if cmd.desc.is_none() {
            continue;
        }
        let mut pending: Option<(i32, Span, bool)> = None;
        for a in &cmd.args {
            let (kind, _) = arg_kind_at(cmd, a.slot);
            match kind {
                ArgKind::SubtileX | ArgKind::SlabX => {
                    pending = parse_int(&a.text).map(|v| (v, a.text_span, kind == ArgKind::SlabX))
                }
                ArgKind::SubtileY | ArgKind::SlabY => {
                    if let (Some((x, xs, slab)), Some(y)) = (pending.take(), parse_int(&a.text)) {
                        out.push(ScriptPos {
                            x,
                            y,
                            x_span: xs,
                            y_span: a.text_span,
                            slab,
                            command: cmd.name.clone(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Formatter

#[derive(Clone, Debug)]
pub struct FormatOptions {
    /// Text repeated once per nesting level.
    pub indent: String,
    /// Also re-indent lines that only hold a `REM` comment.
    pub indent_rem: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            indent: "\t".into(),
            indent_rem: false,
        }
    }
}

/// Re-indent IF blocks. Only the leading whitespace of command lines (and optionally `REM` lines)
/// changes; every other byte of the file is preserved.
pub fn format(doc: &Document, opts: &FormatOptions) -> Vec<u8> {
    let mut out = Vec::with_capacity(doc.src.len() + 64);
    let mut depth: usize = 0;
    for line in &doc.lines {
        let toks = &doc.tokens[line.toks.0..line.toks.1];
        let lead_end = toks
            .iter()
            .take_while(|t| t.kind == TokKind::Whitespace)
            .last()
            .map(|t| t.span.end);
        let first_sig = toks.iter().find(|t| t.kind != TokKind::Whitespace);
        let line_start = line.span.start;
        let mut lines_depth: Option<usize> = None;
        match (&line.cmd, line.kind) {
            (Some(cmd), LineKind::Command) if line.block.is_none() => {
                let d = if cmd.desc.is_some() && cmd.name == "ENDIF" {
                    depth.saturating_sub(1)
                } else {
                    depth
                };
                lines_depth = Some(d);
                if cmd.desc.is_some() {
                    if is_if_command(&cmd.name) && cmd.syntax_ok {
                        depth += 1;
                    } else if cmd.name == "ENDIF" {
                        depth = depth.saturating_sub(1);
                    }
                }
            }
            (None, LineKind::Comment) if opts.indent_rem && line.block.is_none() => {
                lines_depth = Some(depth)
            }
            _ => {}
        }
        match (lines_depth, first_sig) {
            (Some(d), Some(f)) if f.span.start >= line_start && f.kind != TokKind::Newline => {
                let ws_end = lead_end
                    .filter(|&e| e == f.span.start)
                    .unwrap_or(f.span.start);
                let _ = ws_end;
                for _ in 0..d {
                    out.extend_from_slice(opts.indent.as_bytes());
                }
                out.extend_from_slice(&doc.src[f.span.start..line.span.end]);
            }
            _ => out.extend_from_slice(&doc.src[line.span.start..line.span.end]),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// String ids

/// An argument that holds a string id of the game's string tables (message ids of
/// `DISPLAY_OBJECTIVE`, `DISPLAY_INFORMATION`, `DISPLAY_MESSAGE`, `SET_BOX_TOOLTIP_ID` ...; the
/// `QUICK_*` commands take a quick-message slot and the text itself, so they hold no string id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringArg {
    /// Command (DK script) or function (Lua) name.
    pub command: String,
    /// Parameter name as documented.
    pub param: String,
    /// Where the id is written (or would be: the word being typed).
    pub span: Span,
    /// The id when it is written as a plain integer.
    pub id: Option<i64>,
}

fn string_arg_of(cmd: &Command, arg_slot: usize, span: Span, text: &str) -> StringArg {
    let param = Overlay::get()
        .arg(&cmd.name, arg_slot)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "message".into());
    let (v, whole) = crate::doc::strtol0(text);
    StringArg {
        command: cmd.name.clone(),
        param,
        span,
        id: whole.then_some(v),
    }
}

/// The string-id argument the caret is in (or at the end of), if any.
pub fn string_arg_at(doc: &Document, offset: usize) -> Option<StringArg> {
    let Context::Arg { cmd, slot, word } = context_at(doc, offset) else {
        return None;
    };
    if arg_kind_at(cmd, slot).0 != ArgKind::MessageId {
        return None;
    }
    match cmd
        .args
        .iter()
        .find(|a| a.slot == slot && a.func.is_none() && a.span.touches(offset))
    {
        Some(a) => Some(string_arg_of(cmd, slot, a.text_span, &a.text)),
        None => Some(StringArg {
            command: cmd.name.clone(),
            param: Overlay::get()
                .arg(&cmd.name, slot)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "message".into()),
            span: word,
            id: None,
        }),
    }
}

/// Every string-id argument of the script, in order.
pub fn string_args(doc: &Document) -> Vec<StringArg> {
    let mut out = vec![];
    for (_, cmd) in doc.commands() {
        if cmd.desc.is_none() {
            continue;
        }
        for a in &cmd.args {
            if a.func.is_none() && arg_kind_at(cmd, a.slot).0 == ArgKind::MessageId {
                out.push(string_arg_of(cmd, a.slot, a.text_span, &a.text));
            }
        }
    }
    out
}
