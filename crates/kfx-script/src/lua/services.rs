//! Editor services for Lua level scripts: highlight, completions, signature help, hover.
//! (Outline, go-to-definition, map references and the formatter live in [`super::nav`].)
use super::analyze::{walk_block_exprs, walk_stmts, LocalKind};
use super::annot::{parse_ty, parse_ty_prefix, Ty};
use super::ast::*;
use super::diagnostics::kinds_of_type;
use super::doc::LuaDoc;
use super::hooks;
use super::index::{ApiIndex, FuncDef, Member};
use super::infer::{CallSig, Env, MemberKind};
use super::lexer::{TokKind, Token};
use super::stdlib;
use crate::diag::Span;
use crate::provider::ConstantProvider;
use std::collections::BTreeSet;

// ---------------------------------------------------------------------------------------------
// Highlighting

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LuaTokenClass {
    Keyword,
    /// `true`, `false`, `nil`
    Constant,
    Number,
    String,
    Comment,
    /// `@param`, `@class`, ... inside a `---` comment.
    Annotation,
    /// A type expression inside an annotation.
    AnnotationType,
    Operator,
    Punctuation,
    Local,
    Parameter,
    /// A function of the KeeperFX API (index) called or referenced as a global.
    ApiFunction,
    /// A global of the KeeperFX API: classes, `PLAYER0`, `Game`.
    ApiGlobal,
    /// Standard library (`print`, `string`, `math`, ...).
    Std,
    /// A name defined by this file.
    UserGlobal,
    /// Unknown global.
    Unknown,
    Field,
    Method,
    Error,
}

fn tag_span_pieces(doc: &LuaDoc, t: &Token, out: &mut Vec<(Span, LuaTokenClass)>) {
    use LuaTokenClass::*;
    let text = t.text(&doc.src);
    let base = t.span.start;
    let mut emit = |s: usize, e: usize, c: LuaTokenClass| {
        if e > s {
            out.push((Span::new(base + s, base + e), c));
        }
    };
    let dashes = text.len() - text.trim_start_matches('-').len();
    let after = &text[dashes..];
    let ws = after.len() - after.trim_start().len();
    let at = dashes + ws;
    if !text[at..].starts_with('@') {
        emit(0, text.len(), Comment);
        return;
    }
    emit(0, at, Comment);
    let tag_end = at
        + 1
        + text[at + 1..]
            .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(text.len() - at - 1);
    emit(at, tag_end, Annotation);
    let tag = &text[at + 1..tag_end];
    let mut pos = tag_end;
    let skip_ws = |p: usize| p + (text[p..].len() - text[p..].trim_start().len());
    // optional name word
    let name_then_type =
        |pos: &mut usize,
         class: LuaTokenClass,
         emit: &mut dyn FnMut(usize, usize, LuaTokenClass)| {
            let s = skip_ws(*pos);
            emit(*pos, s, Comment);
            let e = s + text[s..]
                .find(char::is_whitespace)
                .unwrap_or(text.len() - s);
            emit(s, e, class);
            *pos = e;
        };
    match tag {
        "param" | "field" => name_then_type(&mut pos, Parameter, &mut emit),
        "class" | "alias" | "enum" => {
            let s = skip_ws(pos);
            emit(pos, s, Comment);
            let e = s + text[s..]
                .find(|c: char| c.is_whitespace() || c == ':')
                .unwrap_or(text.len() - s);
            emit(s, e, AnnotationType);
            pos = e;
        }
        "type" | "return" | "overload" | "vararg" | "generic" | "cast" => {}
        _ => {
            emit(pos, text.len(), Comment);
            return;
        }
    }
    let s = skip_ws(pos);
    emit(pos, s, Comment);
    let rest = &text[s..];
    if tag == "class" && rest.starts_with(':') {
        emit(s, s + 1, Punctuation);
        let s2 = skip_ws(s + 1);
        emit(s + 1, s2, Comment);
        let e = s2
            + text[s2..]
                .find(char::is_whitespace)
                .unwrap_or(text.len() - s2);
        emit(s2, e, AnnotationType);
        emit(e, text.len(), Comment);
        return;
    }
    match parse_ty_prefix(rest) {
        Some((_, tail)) if !rest.is_empty() && tag != "class" => {
            let e = s + (rest.len() - tail.len());
            emit(s, e, AnnotationType);
            emit(e, text.len(), Comment);
        }
        _ => emit(s, text.len(), Comment),
    }
}

/// Classified spans in source order (whitespace is not reported; spans do not overlap).
pub fn highlight(doc: &LuaDoc, index: &ApiIndex) -> Vec<(Span, LuaTokenClass)> {
    use LuaTokenClass::*;
    let env = Env::new(doc, index);
    let mut out: Vec<(Span, LuaTokenClass)> = Vec::with_capacity(doc.tokens.len());
    let sig: Vec<usize> = (0..doc.tokens.len())
        .filter(|&i| !doc.tokens[i].is_trivia() && doc.tokens[i].kind != TokKind::Eof)
        .collect();
    let mut sig_pos = std::collections::HashMap::new();
    for (k, &i) in sig.iter().enumerate() {
        sig_pos.insert(i, k);
    }
    for (i, t) in doc.tokens.iter().enumerate() {
        let text = t.text(&doc.src);
        let in_err = doc
            .lex_errors
            .iter()
            .any(|e| e.span.start < t.span.end && t.span.start < e.span.end);
        let class = match t.kind {
            TokKind::Whitespace | TokKind::Eof => continue,
            TokKind::LineComment => {
                tag_span_pieces(doc, t, &mut out);
                continue;
            }
            TokKind::BlockComment | TokKind::Shebang => Comment,
            TokKind::String | TokKind::LongString => String,
            TokKind::Number => {
                if in_err {
                    Error
                } else {
                    Number
                }
            }
            TokKind::Keyword => match text {
                "true" | "false" | "nil" => Constant,
                _ => Keyword,
            },
            TokKind::Unknown => Error,
            TokKind::Punct => match text {
                "+" | "-" | "*" | "/" | "%" | "^" | "#" | ".." | "==" | "~=" | "<=" | ">="
                | "<" | ">" | "=" | "..." => Operator,
                _ => Punctuation,
            },
            TokKind::Name => {
                let k = sig_pos.get(&i).copied();
                let prev = k.and_then(|k| k.checked_sub(1)).map(|p| sig[p]);
                let next = k.and_then(|k| sig.get(k + 1)).copied();
                let prev_text = prev.map(|p| doc.tokens[p].text(&doc.src));
                let next_text = next.map(|n| doc.tokens[n].text(&doc.src));
                if matches!(prev_text, Some(".") | Some(":"))
                    && prev.map_or(false, |p| doc.tokens[p].kind == TokKind::Punct)
                {
                    if prev_text == Some(":")
                        || matches!(next_text, Some("(") | Some("{"))
                            && next.map_or(false, |n| doc.tokens[n].kind == TokKind::Punct)
                    {
                        Method
                    } else {
                        Field
                    }
                } else if let Some(r) = doc
                    .analysis
                    .ref_at(t.span.start)
                    .filter(|r| r.span == t.span)
                {
                    match r.local {
                        Some(l) => match doc.analysis.locals[l].kind {
                            LocalKind::Param | LocalKind::SelfParam => Parameter,
                            _ => Local,
                        },
                        None => global_class(&env, &r.name),
                    }
                } else if let Some(l) = doc.analysis.locals.iter().find(|l| l.decl == t.span) {
                    match l.kind {
                        LocalKind::Param | LocalKind::SelfParam => Parameter,
                        _ => Local,
                    }
                } else {
                    // e.g. a function name in `function Name()` of a table path, labels
                    global_class(&env, text)
                }
            }
        };
        out.push((
            t.span,
            if in_err
                && matches!(
                    t.kind,
                    TokKind::String | TokKind::LongString | TokKind::BlockComment
                )
            {
                Error
            } else {
                class
            },
        ));
    }
    out
}

fn global_class(env: &Env, name: &str) -> LuaTokenClass {
    use LuaTokenClass::*;
    if env.doc.defs.knows_global(name) && !env.index.knows_global(name) {
        UserGlobal
    } else if env.function(name).is_some() {
        ApiFunction
    } else if env.index.knows_global(name) {
        ApiGlobal
    } else if stdlib::is_std_global(name) {
        Std
    } else if env
        .doc
        .analysis
        .global_assigns
        .iter()
        .any(|g| g.name == name)
    {
        UserGlobal
    } else {
        Unknown
    }
}

// ---------------------------------------------------------------------------------------------
// Shared context helpers

/// A call whose argument list contains an offset.
pub struct CallAt<'a> {
    pub expr: &'a Expr,
    pub args: &'a Args,
    /// Index of the argument the offset is in (the one being typed).
    pub arg_index: usize,
    pub sig: Option<CallSig<'a>>,
    /// Name of the called function as written (`CreateEffect`, `level_up`).
    pub name: String,
}

/// Innermost call (`f(...)` or `o:m(...)`) whose parentheses contain `offset`.
pub fn call_at<'a>(doc: &'a LuaDoc, env: &Env<'a>, offset: usize) -> Option<CallAt<'a>> {
    let mut best: Option<&'a Expr> = None;
    let mut best_open = 0usize;
    walk_block_exprs(&doc.chunk.body, &mut |e| {
        let args = match &e.kind {
            ExprKind::Call { args, .. } | ExprKind::Method { args, .. } => args,
            _ => return,
        };
        let Some(open) = args.open else { return };
        if open.end > offset {
            return;
        }
        let in_region = match args.close {
            Some(c) => offset <= c.start,
            None => true,
        };
        if in_region && (best.is_none() || open.start >= best_open) {
            // an unclosed outer call must not hide a later sibling: require containment of offset
            // in span or unclosed
            if args.close.is_none() && offset > args.span.end + 1 {
                // cursor is beyond everything the parser attached to the call: still accept when
                // only whitespace follows
                let between = &doc.src[args.span.end.min(doc.src.len())..offset.min(doc.src.len())];
                if !between.trim().is_empty() {
                    return;
                }
            }
            best = Some(e);
            best_open = open.start;
        }
    });
    let e = best?;
    let (args, sig, name) = match &e.kind {
        ExprKind::Call { func, args } => {
            let name = match &func.kind {
                ExprKind::Name(n) => n.clone(),
                ExprKind::Field { name, .. } => name.name.clone(),
                _ => String::new(),
            };
            (args, env.resolve_callee(func), name)
        }
        ExprKind::Method {
            obj, name, args, ..
        } => (
            args,
            env.resolve_method_call(obj, &name.name),
            name.name.clone(),
        ),
        _ => return None,
    };
    let arg_index = args.commas.iter().filter(|c| c.end <= offset).count();
    Some(CallAt {
        expr: e,
        args,
        arg_index,
        sig,
        name,
    })
}

/// Expected type of the string literal expression with exactly this span.
fn expected_type_of_string(doc: &LuaDoc, env: &Env, span: Span) -> Option<Ty> {
    // 1. call argument
    let mut found: Option<Ty> = None;
    let mut parents: Vec<&Expr> = Vec::new();
    walk_block_exprs(&doc.chunk.body, &mut |e| match &e.kind {
        ExprKind::Call { .. } | ExprKind::Method { .. } => parents.push(e),
        ExprKind::Binary { op, lhs, rhs, .. } if *op == "==" || *op == "~=" => {
            let other = if lhs.span == span {
                Some(rhs)
            } else if rhs.span == span {
                Some(lhs)
            } else {
                None
            };
            if let Some(o) = other {
                if found.is_none() {
                    found = env.infer(o, o.span.start);
                }
            }
        }
        _ => {}
    });
    if found.is_some() {
        return found;
    }
    for e in parents {
        let (args, sig) = match &e.kind {
            ExprKind::Call { func, args } => (args, env.resolve_callee(func)),
            ExprKind::Method {
                obj, name, args, ..
            } => (args, env.resolve_method_call(obj, &name.name)),
            _ => continue,
        };
        if let Some(i) = args.items.iter().position(|a| a.span == span) {
            let sig = sig?;
            let pi = i as i32 + sig.shift;
            if pi < 0 {
                return None;
            }
            return sig
                .def
                .params
                .get(pi as usize)
                .filter(|p| p.annotated)
                .map(|p| parse_ty(&p.ty))
                .or_else(|| sig.def.vararg.as_ref().map(|t| parse_ty(t)));
        }
    }
    // 3. assignment `x.field = "..."`
    let mut assign_ty = None;
    walk_stmts(&doc.chunk.body, &mut |s| {
        if let StmtKind::Assign { targets, values } = &s.kind {
            if let Some(i) = values.iter().position(|v| v.span == span) {
                if let Some(t) = targets.get(i) {
                    if assign_ty.is_none() {
                        assign_ty = env.infer(t, t.span.start);
                    }
                }
            }
        }
    });
    assign_ty
}

// ---------------------------------------------------------------------------------------------
// Completions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LuaCompletionKind {
    Local,
    Function,
    Global,
    Class,
    Field,
    Method,
    Keyword,
    Snippet,
    /// A string-literal alias value (inside a string argument).
    Constant,
    Hook,
    Annotation,
    Type,
}

#[derive(Clone, Debug)]
pub struct LuaCompletion {
    pub label: String,
    pub kind: LuaCompletionKind,
    /// Text replacing `replace` (snippets may contain `$0` as the final cursor).
    pub insert: String,
    pub detail: String,
    pub doc: String,
    pub replace: Span,
    pub deprecated: bool,
    /// `Some(has_parameters)` for functions and methods the index knows: accepting the item
    /// should write the call parentheses. `None` for everything else.
    pub callable: Option<bool>,
}

fn comp(
    label: &str,
    kind: LuaCompletionKind,
    detail: &str,
    doc: &str,
    replace: Span,
) -> LuaCompletion {
    LuaCompletion {
        label: label.into(),
        kind,
        insert: label.into(),
        detail: detail.into(),
        doc: doc.into(),
        replace,
        deprecated: false,
        callable: None,
    }
}

const KEYWORD_SNIPPETS: &[(&str, &str)] = &[
    ("function", "function $0()\nend"),
    ("local function", "local function $0()\nend"),
    ("if", "if $0 then\nend"),
    ("for i", "for i = 1, $0 do\nend"),
    ("for pairs", "for k, v in pairs($0) do\nend"),
    ("for ipairs", "for i, v in ipairs($0) do\nend"),
    ("while", "while $0 do\nend"),
    ("repeat", "repeat\n\t$0\nuntil "),
];

const PLAIN_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

const ANNOTATION_TAGS: &[(&str, &str)] = &[
    ("class", "Declare a class: @class Name: Parent"),
    ("field", "Declare a class field: @field name type"),
    (
        "param",
        "Document a parameter: @param name type description",
    ),
    (
        "return",
        "Document a return value: @return type description",
    ),
    ("type", "Give the next variable a type: @type Creature"),
    ("alias", "Declare a type alias"),
    ("enum", "Declare an enum"),
    (
        "overload",
        "Alternative signature: @overload fun(a: integer)",
    ),
    ("deprecated", "Mark as deprecated"),
    (
        "diagnostic",
        "Silence diagnostics: @diagnostic disable-next-line: lua-undefined-global",
    ),
    ("meta", "Marks a definition-only file"),
    ("generic", "Declare generic parameters"),
    ("nodiscard", "Return value must be used"),
    ("vararg", "Type of ..."),
    ("see", "Reference"),
    ("async", "Async function"),
    ("cast", "Cast a variable"),
];

/// Completions at byte `offset`. The `provider` supplies config-driven names (creature, room, ...)
/// that extend the alias unions inside string arguments.
pub fn completions(
    doc: &LuaDoc,
    offset: usize,
    index: &ApiIndex,
    provider: &dyn ConstantProvider,
) -> Vec<LuaCompletion> {
    let offset = offset.min(doc.src.len());
    let env = Env::new(doc, index);
    let tok = doc
        .tokens
        .iter()
        .position(|t| t.span.start < offset && offset <= t.span.end && t.kind != TokKind::Eof);

    // inside a comment
    if let Some(ti) = tok {
        let t = doc.tokens[ti];
        if t.kind == TokKind::LineComment && offset > t.span.start {
            return comment_completions(doc, &env, t, offset);
        }
        if t.kind == TokKind::BlockComment && offset < t.span.end {
            return vec![];
        }
        if t.kind == TokKind::String || t.kind == TokKind::LongString {
            let text = t.text(&doc.src);
            let closed = t.kind == TokKind::String
                && text.len() >= 2
                && text.ends_with(text.as_bytes()[0] as char)
                && !doc.lex_errors.iter().any(|e| e.span.start == t.span.start);
            let inside = offset > t.span.start
                && (offset < t.span.end || !closed)
                && t.kind == TokKind::String;
            if inside {
                let content = Span::new(
                    t.span.start + 1,
                    if closed { t.span.end - 1 } else { t.span.end },
                );
                return string_completions(doc, &env, t.span, content, provider);
            }
            if t.kind == TokKind::LongString {
                return vec![];
            }
        }
    }

    // member access: `obj.` / `obj:` with an optional partial name
    let name_tok = tok.filter(|&i| doc.tokens[i].kind == TokKind::Name);
    let prefix_span = name_tok
        .map(|i| doc.tokens[i].span)
        .unwrap_or(Span::new(offset, offset));
    let prefix = &doc.src[prefix_span.start..offset.min(prefix_span.end).max(prefix_span.start)];
    let before = doc.prev_sig_token(prefix_span.start);
    if let Some(b) = before {
        let bt = doc.tokens[b];
        let btext = bt.text(&doc.src);
        if bt.kind == TokKind::Punct && (btext == "." || btext == ":") {
            return member_completions(doc, &env, bt.span, btext == ":", prefix, prefix_span);
        }
        // inside a table constructor key position
        if bt.kind == TokKind::Punct && (btext == "{" || btext == ",") {
            if let Some(c) = table_key_completions(doc, &env, offset, prefix, prefix_span) {
                return c;
            }
        }
    }

    global_completions(
        doc,
        &env,
        offset,
        prefix,
        prefix_span,
        before.map(|b| doc.tokens[b].text(&doc.src)),
    )
}

fn comment_completions(doc: &LuaDoc, env: &Env, t: Token, offset: usize) -> Vec<LuaCompletion> {
    let text = t.text(&doc.src);
    if !text.starts_with("---") {
        return vec![];
    }
    let upto = &doc.src[t.span.start..offset];
    let body = upto.trim_start_matches('-');
    let ws_len = body.len() - body.trim_start().len();
    let body = body.trim_start();
    let body_start = t.span.start + (upto.len() - body.len()) + 0 * ws_len;
    if let Some(rest) = body.strip_prefix('@') {
        if !rest.contains(char::is_whitespace) {
            // completing the tag name
            let sp = Span::new(body_start + 1, offset);
            return ANNOTATION_TAGS
                .iter()
                .filter(|(n, _)| n.starts_with(rest))
                .map(|(n, d)| comp(n, LuaCompletionKind::Annotation, "annotation tag", d, sp))
                .collect();
        }
        // after the tag: complete type names where a type is expected
        let tag = rest.split_whitespace().next().unwrap_or("");
        let words: Vec<&str> = rest.split_whitespace().collect();
        let typing_word = !rest.ends_with(char::is_whitespace);
        let nwords = words.len();
        let type_pos = match tag {
            "type" | "return" | "overload" | "vararg" => nwords == 1 + typing_word as usize,
            "param" | "field" => nwords == 2 + typing_word as usize,
            "alias" => nwords == 2 + typing_word as usize,
            "class" => nwords >= 2,
            _ => false,
        };
        if type_pos {
            let word_start = if typing_word {
                offset - words.last().map_or(0, |w| w.len())
            } else {
                offset
            };
            let cur = &doc.src[word_start..offset];
            // after a `|` complete the last union member
            let (cur_start, cur) = match cur.rfind('|') {
                Some(i) => (word_start + i + 1, &cur[i + 1..]),
                None => (word_start, cur),
            };
            return type_name_completions(env, cur, Span::new(cur_start, offset));
        }
    }
    vec![]
}

fn type_name_completions(env: &Env, prefix: &str, replace: Span) -> Vec<LuaCompletion> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let lp = prefix.to_ascii_lowercase();
    let mut add = |n: &str, kind: LuaCompletionKind, detail: &str, doc: &str| {
        if n.to_ascii_lowercase().starts_with(&lp) && seen.insert(n.to_string()) {
            out.push(comp(n, kind, detail, doc, replace));
        }
    };
    for b in [
        "nil", "any", "boolean", "string", "number", "integer", "function", "table", "userdata",
        "thread",
    ] {
        add(b, LuaCompletionKind::Type, "builtin type", "");
    }
    for src in [&env.doc.defs, env.index] {
        for (n, c) in &src.classes {
            add(n, LuaCompletionKind::Class, "class", &c.doc);
        }
        for (n, a) in &src.aliases {
            add(n, LuaCompletionKind::Type, "alias", &a.doc);
        }
        for n in src.enums.keys() {
            add(n, LuaCompletionKind::Type, "enum", "");
        }
    }
    out
}

fn string_completions(
    doc: &LuaDoc,
    env: &Env,
    str_span: Span,
    content: Span,
    provider: &dyn ConstantProvider,
) -> Vec<LuaCompletion> {
    let Some(ty) = expected_type_of_string(doc, env, str_span) else {
        return vec![];
    };
    let vals = env.type_values(&ty);
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for l in vals.literals.iter().filter(|l| l.is_str) {
        if seen.insert(l.value.to_ascii_uppercase()) {
            out.push(comp(
                &l.value,
                LuaCompletionKind::Constant,
                &ty.to_string(),
                &l.doc,
                content,
            ));
        }
    }
    for k in kinds_of_type(env, &ty) {
        for n in provider.names(k) {
            if seen.insert(n.to_ascii_uppercase()) {
                out.push(comp(
                    n,
                    LuaCompletionKind::Constant,
                    &ty.to_string(),
                    "",
                    content,
                ));
            }
        }
    }
    out
}

fn member_completions(
    doc: &LuaDoc,
    env: &Env,
    dot: Span,
    colon: bool,
    prefix: &str,
    prefix_span: Span,
) -> Vec<LuaCompletion> {
    // find the expression the dot belongs to
    let mut base: Option<&Expr> = None;
    walk_block_exprs(&doc.chunk.body, &mut |e| match &e.kind {
        ExprKind::Field { obj, dot: d, .. } if *d == dot => base = Some(obj),
        ExprKind::Method { obj, colon: c, .. } if *c == dot => base = Some(obj),
        _ => {}
    });
    let Some(base) = base else { return vec![] };
    let Some(ty) = env.infer(base, base.span.start) else {
        return vec![];
    };
    let lp = prefix.to_ascii_lowercase();
    env.members_of(&ty, colon)
        .into_iter()
        .filter(|m| m.name.to_ascii_lowercase().starts_with(&lp))
        .map(|m| LuaCompletion {
            label: m.name.clone(),
            kind: match m.kind {
                MemberKind::Field => LuaCompletionKind::Field,
                MemberKind::Method => LuaCompletionKind::Method,
            },
            insert: m.name,
            detail: m.ty,
            doc: m.doc,
            replace: prefix_span,
            deprecated: m.deprecated,
            callable: m.callable,
        })
        .collect()
}

fn table_key_completions(
    doc: &LuaDoc,
    env: &Env,
    offset: usize,
    prefix: &str,
    prefix_span: Span,
) -> Option<Vec<LuaCompletion>> {
    // innermost table constructor containing `offset` that is a direct call argument
    let mut best: Option<(&Expr, &TableCtor)> = None;
    walk_block_exprs(&doc.chunk.body, &mut |e| {
        if let ExprKind::Table(t) = &e.kind {
            let end = t.close.map_or(usize::MAX, |c| c.start);
            if t.open.end <= offset
                && offset <= end
                && best.map_or(true, |(b, _)| e.span.start >= b.span.start)
            {
                best = Some((e, t));
            }
        }
    });
    let (texpr, tc) = best?;
    // which call argument is it?
    let mut ty: Option<Ty> = None;
    walk_block_exprs(&doc.chunk.body, &mut |e| {
        let (args, sig) = match &e.kind {
            ExprKind::Call { func, args } => (args, env.resolve_callee(func)),
            ExprKind::Method {
                obj, name, args, ..
            } => (args, env.resolve_method_call(obj, &name.name)),
            _ => return,
        };
        if let Some(i) = args.items.iter().position(|a| a.span == texpr.span) {
            if let Some(s) = sig {
                let pi = i as i32 + s.shift;
                if pi >= 0 {
                    ty = s
                        .def
                        .params
                        .get(pi as usize)
                        .filter(|p| p.annotated)
                        .map(|p| parse_ty(&p.ty));
                }
            }
        }
    });
    let ty = ty?;
    let present: BTreeSet<&str> = tc
        .items
        .iter()
        .filter_map(|i| match i {
            TableItem::Named { key, .. } => Some(key.name.as_str()),
            _ => None,
        })
        .collect();
    let lp = prefix.to_ascii_lowercase();
    let mut out = Vec::new();
    for m in ty.members() {
        match &m {
            Ty::Name(c) => {
                for cd in env.class_chain(c) {
                    for (n, f) in &cd.fields {
                        if !present.contains(n.as_str()) && n.to_ascii_lowercase().starts_with(&lp)
                        {
                            out.push(LuaCompletion {
                                label: n.clone(),
                                kind: LuaCompletionKind::Field,
                                insert: format!("{n} = "),
                                detail: f.ty.clone(),
                                doc: f.doc.clone(),
                                replace: prefix_span,
                                deprecated: false,
                                callable: None,
                            });
                        }
                    }
                }
            }
            Ty::Table(fields) => {
                for (n, t) in fields {
                    if !present.contains(n.as_str()) && n.to_ascii_lowercase().starts_with(&lp) {
                        out.push(LuaCompletion {
                            label: n.clone(),
                            kind: LuaCompletionKind::Field,
                            insert: format!("{n} = "),
                            detail: t.to_string(),
                            doc: String::new(),
                            replace: prefix_span,
                            deprecated: false,
                            callable: None,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn global_completions(
    doc: &LuaDoc,
    env: &Env,
    offset: usize,
    prefix: &str,
    prefix_span: Span,
    before: Option<&str>,
) -> Vec<LuaCompletion> {
    use LuaCompletionKind as K;
    let lp = prefix.to_ascii_lowercase();
    let matches = |n: &str| n.to_ascii_lowercase().starts_with(&lp);
    let mut out: Vec<LuaCompletion> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    // after `function`: hook names first
    if before == Some("function") {
        for h in hooks::HOOKS {
            if matches(h.name) {
                let params = h.params.iter().map(|p| p.0).collect::<Vec<_>>().join(", ");
                out.push(LuaCompletion {
                    label: h.name.to_string(),
                    kind: K::Hook,
                    insert: format!("{}({})\n\t$0\nend", h.name, params),
                    detail: format!(
                        "hook({})",
                        h.params
                            .iter()
                            .map(|p| format!("{}: {}", p.0, p.1))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    doc: h.doc.to_string(),
                    replace: prefix_span,
                    deprecated: false,
                    callable: None,
                });
                seen.insert(h.name.to_string());
            }
        }
        return out;
    }
    // locals
    for l in doc.analysis.visible_locals(offset).into_iter().rev() {
        let lv = &doc.analysis.locals[l];
        if lv.decl.start == prefix_span.start
            && lv.decl.end >= prefix_span.start
            && lv.decl.end == prefix_span.end
        {
            continue; // the word being typed is itself a declaration
        }
        if matches(&lv.name) && seen.insert(lv.name.clone()) {
            let ty = env.local_type(l).map(|t| t.to_string()).unwrap_or_default();
            let kind = if lv.kind == LocalKind::LocalFunction {
                K::Function
            } else {
                K::Local
            };
            out.push(comp(&lv.name, kind, &ty, "", prefix_span));
        }
    }
    // file definitions, then library
    for src in [&doc.defs, env.index] {
        for (n, f) in &src.functions {
            if matches(n) && seen.insert(n.clone()) {
                out.push(LuaCompletion {
                    label: n.clone(),
                    kind: if hooks::is_hook_name(n) {
                        K::Hook
                    } else {
                        K::Function
                    },
                    insert: n.clone(),
                    detail: f.signature(),
                    doc: f.doc.clone(),
                    replace: prefix_span,
                    deprecated: f.deprecated.is_some(),
                    callable: if hooks::is_hook_name(n) {
                        None
                    } else {
                        Some(!f.params.is_empty() || f.vararg.is_some())
                    },
                });
            }
        }
        for (n, g) in &src.globals {
            if matches(n) && seen.insert(n.clone()) {
                out.push(comp(n, K::Global, &g.ty, &g.doc, prefix_span));
            }
        }
        for (n, c) in &src.classes {
            if matches(n) && !c.implicit && seen.insert(n.clone()) {
                out.push(comp(n, K::Class, "class", &c.doc, prefix_span));
            }
        }
    }
    for n in stdlib::BASE_GLOBALS {
        if matches(n) && seen.insert(n.to_string()) {
            let sig = stdlib::signature(n).unwrap_or("");
            let is_lib = stdlib::lib_members(n).is_some();
            let mut c = comp(
                n,
                if is_lib { K::Global } else { K::Function },
                sig,
                "",
                prefix_span,
            );
            if !is_lib && stdlib::signature(n).is_some() {
                c.callable = Some(crate::lua::infer::sig_has_params(Some(sig)));
            }
            out.push(c);
        }
    }
    for g in &doc.analysis.global_assigns {
        if matches(&g.name) && seen.insert(g.name.clone()) {
            out.push(comp(&g.name, K::Global, "", "", prefix_span));
        }
    }
    for kw in PLAIN_KEYWORDS {
        if matches(kw) && seen.insert(format!("kw:{kw}")) {
            out.push(comp(kw, K::Keyword, "keyword", "", prefix_span));
        }
    }
    for (label, snip) in KEYWORD_SNIPPETS {
        if matches(label) {
            out.push(LuaCompletion {
                label: label.to_string(),
                kind: K::Snippet,
                insert: snip.to_string(),
                detail: "snippet".into(),
                doc: String::new(),
                replace: prefix_span,
                deprecated: false,
                callable: None,
            });
        }
    }
    // hook skeletons and trigger registrations at statement level
    for h in hooks::HOOKS {
        let label = format!("function {}", h.name);
        if label.to_ascii_lowercase().starts_with(&lp) || matches(h.name) {
            let params = h.params.iter().map(|p| p.0).collect::<Vec<_>>().join(", ");
            out.push(LuaCompletion {
                label,
                kind: K::Snippet,
                insert: format!("function {}({})\n\t$0\nend", h.name, params),
                detail: "hook".into(),
                doc: h.doc.to_string(),
                replace: prefix_span,
                deprecated: false,
                callable: None,
            });
        }
    }
    for (label, snip) in hooks::TRIGGER_SNIPPETS {
        if matches(label) && (env.function(label).is_some() || true) {
            let known = env
                .function(label)
                .map(|f| f.signature())
                .unwrap_or_default();
            out.push(LuaCompletion {
                label: format!("{label} (snippet)"),
                kind: K::Snippet,
                insert: snip.to_string(),
                detail: known,
                doc: "Trigger registration with an anonymous callback".into(),
                replace: prefix_span,
                deprecated: false,
                callable: None,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Signature help

#[derive(Clone, Debug)]
pub struct LuaParamInfo {
    pub name: String,
    pub ty: String,
    pub doc: String,
    pub optional: bool,
    /// Byte range of this parameter inside [`LuaSignatureHelp::label`].
    pub label_range: (usize, usize),
}

#[derive(Clone, Debug)]
pub struct LuaSignatureHelp {
    pub name: String,
    pub label: String,
    pub doc: String,
    pub params: Vec<LuaParamInfo>,
    /// Index into `params` (may be one past the end for a variadic tail).
    pub active: usize,
    pub overloads: Vec<String>,
    pub deprecated: bool,
}

/// Build the help block for a function definition. `skip_self` hides an implicit `self`.
pub fn signature_of_def(def: &FuncDef, display: &str) -> LuaSignatureHelp {
    let mut label = String::new();
    label.push_str(display);
    label.push('(');
    let mut params = Vec::new();
    for (i, p) in def.params.iter().enumerate() {
        if i > 0 {
            label.push_str(", ");
        }
        let start = label.len();
        label.push_str(&p.name);
        if p.optional {
            label.push('?');
        }
        if p.annotated || p.ty != "any" {
            label.push_str(": ");
            label.push_str(&p.ty);
        }
        params.push(LuaParamInfo {
            name: p.name.clone(),
            ty: p.ty.clone(),
            doc: p.doc.clone(),
            optional: p.optional,
            label_range: (start, label.len()),
        });
    }
    if let Some(v) = &def.vararg {
        if !def.params.is_empty() {
            label.push_str(", ");
        }
        let start = label.len();
        label.push_str("...");
        if v != "any" {
            label.push_str(": ");
            label.push_str(v);
        }
        params.push(LuaParamInfo {
            name: "...".into(),
            ty: v.clone(),
            doc: String::new(),
            optional: true,
            label_range: (start, label.len()),
        });
    }
    label.push(')');
    if !def.returns.is_empty() {
        label.push_str(": ");
        label.push_str(
            &def.returns
                .iter()
                .map(|r| r.ty.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    LuaSignatureHelp {
        name: display.to_string(),
        label,
        doc: def.doc.clone(),
        params,
        active: 0,
        overloads: def.overloads.clone(),
        deprecated: def.deprecated.is_some(),
    }
}

/// Signature help for a standard library function from its signature string.
fn signature_of_std(path: &str, sig: &str) -> LuaSignatureHelp {
    let open = sig.find('(').unwrap_or(0);
    let mut depth = 0;
    let mut close = sig.len();
    for (i, c) in sig.char_indices().skip(open) {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth == 0 {
                    close = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let mut params = Vec::new();
    let mut start = open + 1;
    let mut d = 0;
    let inner = &sig[open + 1..close.min(sig.len())];
    let mut pieces: Vec<(usize, usize)> = Vec::new();
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '[' | '{' => d += 1,
            ')' | ']' | '}' => d -= 1,
            ',' if d == 0 => {
                pieces.push((start, open + 1 + i));
                start = open + 1 + i + 1;
            }
            _ => {}
        }
    }
    pieces.push((start, open + 1 + inner.len()));
    for (s, e) in pieces {
        let raw = &sig[s..e];
        let lead = raw.len() - raw.trim_start().len();
        let txt = raw.trim();
        if txt.is_empty() {
            continue;
        }
        let name = txt
            .split(':')
            .next()
            .unwrap_or(txt)
            .trim()
            .trim_end_matches('?')
            .to_string();
        params.push(LuaParamInfo {
            name,
            ty: txt
                .split_once(':')
                .map(|x| x.1.trim().to_string())
                .unwrap_or_default(),
            doc: String::new(),
            optional: txt.contains('?') || txt.starts_with('['),
            label_range: (s + lead, s + lead + txt.len()),
        });
    }
    LuaSignatureHelp {
        name: path.to_string(),
        label: sig.to_string(),
        doc: String::new(),
        params,
        active: 0,
        overloads: vec![],
        deprecated: false,
    }
}

/// Signature help for the call around `offset`.
pub fn signature_help(doc: &LuaDoc, offset: usize, index: &ApiIndex) -> Option<LuaSignatureHelp> {
    let env = Env::new(doc, index);
    let c = call_at(doc, &env, offset)?;
    let mut h = match &c.sig {
        Some(s) => {
            let mut h = signature_of_def(s.def, &s.display);
            if s.shift != 0 {
                // shift = 1: first parameter is the explicit self; shift = -1: argument 0 is self
                h.active = 0;
            }
            let idx = c.arg_index as i32 + s.shift;
            h.active = idx.max(0) as usize;
            if s.shift == 1 && !h.params.is_empty() {
                // colon call on a dot-declared function: `self` was passed implicitly
                h.active = (idx as usize).min(h.params.len());
            }
            h
        }
        None => {
            // standard library: build the path from the callee
            let path = match &c.expr.kind {
                ExprKind::Call { func, .. } => func.path().map(|p| p.join(".")),
                _ => None,
            }?;
            let sig = stdlib::signature(&path)?;
            let mut h = signature_of_std(&path, sig);
            h.active = c.arg_index;
            h
        }
    };
    if !h.params.is_empty() {
        let last_is_vararg = h.params.last().map_or(false, |p| p.name == "...");
        if h.active >= h.params.len() && !last_is_vararg {
            h.active = h.params.len();
        } else if h.active >= h.params.len() {
            h.active = h.params.len() - 1;
        }
    }
    Some(h)
}

// ---------------------------------------------------------------------------------------------
// Hover

#[derive(Clone, Debug)]
pub struct LuaHover {
    pub span: Span,
    pub markdown: String,
}

fn fn_markdown(def: &FuncDef, display: &str) -> String {
    let sig = signature_of_def(def, display);
    let mut md = format!("```lua\n(function) {}\n```\n", sig.label);
    if def.deprecated.is_some() {
        md.push_str("\n**Deprecated**");
        if let Some(m) = &def.deprecated {
            if !m.is_empty() {
                md.push_str(&format!(": {m}"));
            }
        }
        md.push('\n');
    }
    if !def.doc.is_empty() {
        md.push_str(&format!("\n{}\n", def.doc));
    }
    let docs: Vec<String> = def
        .params
        .iter()
        .filter(|p| !p.doc.is_empty())
        .map(|p| format!("- `{}` ({}): {}", p.name, p.ty, p.doc))
        .collect();
    if !docs.is_empty() {
        md.push_str(&format!("\n{}\n", docs.join("\n")));
    }
    for r in def.returns.iter().filter(|r| !r.doc.is_empty()) {
        md.push_str(&format!("\nReturns `{}`: {}\n", r.ty, r.doc));
    }
    if let Some(s) = &def.source {
        md.push_str(&format!("\n*{}:{}*", s.file, s.line));
    }
    md
}

pub fn hover(doc: &LuaDoc, offset: usize, index: &ApiIndex) -> Option<LuaHover> {
    let env = Env::new(doc, index);
    let ti = doc
        .token_at(offset)
        .or_else(|| offset.checked_sub(1).and_then(|o| doc.token_at(o)))?;
    let t = doc.tokens[ti];
    let text = t.text(&doc.src);
    match t.kind {
        TokKind::String => {
            // alias literal docs
            let ty = expected_type_of_string(doc, &env, t.span)?;
            let value = crate::lua::parser::decode_string(text);
            let vals = env.type_values(&ty);
            let lit = vals
                .literals
                .iter()
                .find(|l| l.is_str && l.value.eq_ignore_ascii_case(&value))?;
            let mut md = format!("```lua\n\"{}\": {}\n```", lit.value, ty);
            if !lit.doc.is_empty() {
                md.push_str(&format!("\n\n{}", lit.doc));
            }
            Some(LuaHover {
                span: t.span,
                markdown: md,
            })
        }
        TokKind::Name => {
            // member access?
            let prev = doc.prev_sig_token(t.span.start);
            if let Some(p) = prev {
                let pt = doc.tokens[p];
                let ptext = pt.text(&doc.src);
                if pt.kind == TokKind::Punct && (ptext == "." || ptext == ":") {
                    let mut base: Option<&Expr> = None;
                    walk_block_exprs(&doc.chunk.body, &mut |e| match &e.kind {
                        ExprKind::Field { obj, dot, .. } if *dot == pt.span => base = Some(obj),
                        ExprKind::Method { obj, colon, .. } if *colon == pt.span => {
                            base = Some(obj)
                        }
                        _ => {}
                    });
                    let base = base?;
                    let bty = env.infer(base, base.span.start)?;
                    for m in bty.members() {
                        let class = match &m {
                            Ty::Name(c) => c.clone(),
                            Ty::Generic(g, a) if g == "typeof" => match a.first() {
                                Some(Ty::Name(c)) if c.strip_prefix("lib:").is_none() => c.clone(),
                                Some(Ty::Name(c)) => {
                                    let lib = &c[4..];
                                    let path = format!("{lib}.{text}");
                                    if let Some(sig) = stdlib::signature(&path) {
                                        return Some(LuaHover {
                                            span: t.span,
                                            markdown: format!("```lua\n(function) {sig}\n```"),
                                        });
                                    }
                                    continue;
                                }
                                _ => continue,
                            },
                            _ => continue,
                        };
                        match env.find_member(&class, text) {
                            Some(Member::Method(def, owner)) => {
                                let sep = if def.method { ':' } else { '.' };
                                return Some(LuaHover {
                                    span: t.span,
                                    markdown: fn_markdown(
                                        def,
                                        &format!("{}{}{}", owner.name, sep, text),
                                    ),
                                });
                            }
                            Some(Member::Field(f, owner)) => {
                                let mut md = format!(
                                    "```lua\n(field) {}.{}: {}\n```",
                                    owner.name, f.name, f.ty
                                );
                                if !f.doc.is_empty() {
                                    md.push_str(&format!("\n\n{}", f.doc));
                                }
                                return Some(LuaHover {
                                    span: t.span,
                                    markdown: md,
                                });
                            }
                            None => {}
                        }
                    }
                    return None;
                }
            }
            // declaration of a local
            if let Some(l) = doc.analysis.locals.iter().position(|l| l.decl == t.span) {
                return Some(local_hover(doc, &env, l, t.span));
            }
            if let Some(r) = doc.analysis.ref_at(offset).filter(|r| r.span == t.span) {
                if let Some(l) = r.local {
                    return Some(local_hover(doc, &env, l, t.span));
                }
                return global_hover(&env, &r.name, t.span);
            }
            global_hover(&env, text, t.span)
        }
        _ => None,
    }
}

fn local_hover(doc: &LuaDoc, env: &Env, l: usize, span: Span) -> LuaHover {
    let lv = &doc.analysis.locals[l];
    let ty = env
        .local_type(l)
        .map(|t| t.to_string())
        .unwrap_or_else(|| "any".into());
    let kind = match lv.kind {
        LocalKind::Local | LocalKind::ForVar => "local",
        LocalKind::Param | LocalKind::SelfParam => "param",
        LocalKind::LocalFunction => "local function",
    };
    let mut md = format!("```lua\n({kind}) {}: {}\n```", lv.name, ty);
    if let Some(def) = doc.local_funcs.get(&l) {
        md = fn_markdown(def, &lv.name);
    }
    md.push_str(&format!("\n\n*line {}*", doc.line_of(lv.decl.start)));
    LuaHover { span, markdown: md }
}

fn global_hover(env: &Env, name: &str, span: Span) -> Option<LuaHover> {
    if let Some(def) = env.function(name) {
        let mut md = fn_markdown(def, name);
        if let Some(h) = hooks::hook(name) {
            md.push_str(&format!(
                "\n\nEngine hook (src/lua_triggers.c:{}): {}",
                h.c_line, h.doc
            ));
        }
        return Some(LuaHover { span, markdown: md });
    }
    if let Some(h) = hooks::hook(name) {
        let params = h
            .params
            .iter()
            .map(|p| format!("{}: {}", p.0, p.1))
            .collect::<Vec<_>>()
            .join(", ");
        return Some(LuaHover {
            span,
            markdown: format!(
                "```lua\n(hook) {}({})\n```\n\n{}\n\n*src/lua_triggers.c:{}*",
                h.name, params, h.doc, h.c_line
            ),
        });
    }
    if env.has_class(name) {
        let chain = env.class_chain(name);
        let first = chain.first()?;
        let mut md = format!("```lua\n(class) {}", name);
        if !first.parents.is_empty() {
            md.push_str(&format!(": {}", first.parents.join(", ")));
        }
        md.push_str("\n```");
        if !first.doc.is_empty() {
            md.push_str(&format!("\n\n{}", first.doc));
        }
        let nf: usize = chain.iter().map(|c| c.fields.len()).sum();
        let nm: usize = chain.iter().map(|c| c.methods.len()).sum();
        md.push_str(&format!("\n\n{nf} field(s), {nm} method(s)"));
        return Some(LuaHover { span, markdown: md });
    }
    if let Some(g) = env.global(name) {
        let mut md = format!("```lua\n(global) {}: {}\n```", g.name, g.ty);
        if !g.doc.is_empty() {
            md.push_str(&format!("\n\n{}", g.doc));
        }
        return Some(LuaHover { span, markdown: md });
    }
    if let Some(a) = env.alias(name) {
        let vals = env.alias_values(name)?;
        let list = vals
            .literals
            .iter()
            .take(12)
            .map(|l| {
                if l.doc.is_empty() {
                    format!("- `{}`", l.value)
                } else {
                    format!("- `{}`: {}", l.value, l.doc)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Some(LuaHover {
            span,
            markdown: format!(
                "```lua\n(alias) {}\n```\n\n{}\n\n{}{}",
                a.name,
                a.doc,
                list,
                if vals.literals.len() > 12 {
                    format!("\n- ... {} more", vals.literals.len() - 12)
                } else {
                    String::new()
                }
            ),
        });
    }
    if let Some(sig) = stdlib::signature(name) {
        return Some(LuaHover {
            span,
            markdown: format!("```lua\n(function) {sig}\n```"),
        });
    }
    if stdlib::is_std_global(name) {
        return Some(LuaHover {
            span,
            markdown: format!("```lua\n(global) {name}\n```\nLua standard library"),
        });
    }
    None
}

// ---------------------------------------------------------------------------------------------
// String ids

/// Whether a library parameter holds an id of the game's string tables (`msg_id` of
/// `DisplayInformation`, `DisplayObjective`, `DisplayMessage`, `GetString` ..., `TooltipTextID` of
/// `SetBoxTooltipId`).
pub fn is_string_id_param(name: &str) -> bool {
    matches!(name, "msg_id" | "TooltipTextID")
}

fn int_literal(e: &Expr) -> Option<i64> {
    match &e.kind {
        ExprKind::Number(n) => n.parse::<i64>().ok(),
        _ => None,
    }
}

/// The string-id argument at `offset`: inside the call of a library function whose parameter at
/// that position is a string id. The span is the argument written there, else the digits being
/// typed.
pub fn string_arg_at(
    doc: &LuaDoc,
    index: &ApiIndex,
    offset: usize,
) -> Option<crate::services::StringArg> {
    let env = Env::new(doc, index);
    let c = call_at(doc, &env, offset)?;
    let sig = c.sig?;
    let pi = c.arg_index as i32 + sig.shift;
    if pi < 0 {
        return None;
    }
    let p = sig.def.params.get(pi as usize)?;
    if !is_string_id_param(&p.name) {
        return None;
    }
    match c.args.items.get(c.arg_index) {
        Some(a) if a.span.touches(offset) => Some(crate::services::StringArg {
            command: c.name,
            param: p.name.clone(),
            span: a.span,
            id: int_literal(a),
        }),
        _ => {
            let b = doc.src.as_bytes();
            let mut s = offset.min(b.len());
            while s > 0 && b[s - 1].is_ascii_digit() {
                s -= 1;
            }
            let mut e = offset.min(b.len());
            while e < b.len() && b[e].is_ascii_digit() {
                e += 1;
            }
            Some(crate::services::StringArg {
                command: c.name,
                param: p.name.clone(),
                span: Span::new(s, e),
                id: None,
            })
        }
    }
}

/// Every string-id argument of the file.
pub fn string_args(doc: &LuaDoc, index: &ApiIndex) -> Vec<crate::services::StringArg> {
    let env = Env::new(doc, index);
    let mut out = vec![];
    walk_block_exprs(&doc.chunk.body, &mut |e| {
        let (args, sig, name) = match &e.kind {
            ExprKind::Call { func, args } => {
                let name = match &func.kind {
                    ExprKind::Name(n) => n.clone(),
                    ExprKind::Field { name, .. } => name.name.clone(),
                    _ => String::new(),
                };
                (args, env.resolve_callee(func), name)
            }
            ExprKind::Method {
                obj, name, args, ..
            } => (
                args,
                env.resolve_method_call(obj, &name.name),
                name.name.clone(),
            ),
            _ => return,
        };
        let Some(sig) = sig else { return };
        for (i, a) in args.items.iter().enumerate() {
            let pi = i as i32 + sig.shift;
            if pi < 0 {
                continue;
            }
            if let Some(p) = sig
                .def
                .params
                .get(pi as usize)
                .filter(|p| is_string_id_param(&p.name))
            {
                out.push(crate::services::StringArg {
                    command: name.clone(),
                    param: p.name.clone(),
                    span: a.span,
                    id: int_literal(a),
                });
            }
        }
    });
    out
}
