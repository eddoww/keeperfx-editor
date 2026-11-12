//! Navigation and structure services: outline, go to definition, map references and the
//! indentation-only formatter.
use super::analyze::{walk_block_exprs, FuncKind};
use super::annot::{parse_ty, Tag, Ty};
use super::ast::*;
use super::doc::LuaDoc;
use super::hooks;
use super::index::{ApiIndex, Member};
use super::infer::Env;
use super::lexer::TokKind;
use crate::diag::Span;
use crate::services::{RefKind, ScriptPos, ScriptRef};

// ---------------------------------------------------------------------------------------------
// Outline

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LuaOutlineKind {
    Function,
    LocalFunction,
    Method,
    /// A global function the engine calls (`OnGameStart`, `OnGameTick`, ...).
    Hook,
    /// A trigger registration call (`RegisterTimerEvent(...)`, `CreateTrigger(...)`).
    Trigger,
    Class,
}

#[derive(Clone, Debug)]
pub struct LuaOutlineItem {
    pub kind: LuaOutlineKind,
    pub label: String,
    pub span: Span,
    pub line: usize,
    pub children: Vec<LuaOutlineItem>,
}

fn collect_outline(doc: &LuaDoc, block: &Block, out: &mut Vec<LuaOutlineItem>) {
    for s in &block.stmts {
        match &s.kind {
            StmtKind::Function { target, func } => {
                let name = target.display();
                let kind = if target.parts.len() == 1 && target.method.is_none() {
                    if hooks::is_hook_name(&name) {
                        LuaOutlineKind::Hook
                    } else {
                        LuaOutlineKind::Function
                    }
                } else {
                    LuaOutlineKind::Method
                };
                let mut children = Vec::new();
                collect_outline(doc, &func.body, &mut children);
                let params = func
                    .params
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push(LuaOutlineItem {
                    kind,
                    label: format!("{name}({params})"),
                    span: s.span,
                    line: doc.line_of(s.span.start),
                    children,
                });
            }
            StmtKind::LocalFunction { name, func } => {
                let mut children = Vec::new();
                collect_outline(doc, &func.body, &mut children);
                let params = func
                    .params
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push(LuaOutlineItem {
                    kind: LuaOutlineKind::LocalFunction,
                    label: format!("{}({params})", name.name),
                    span: s.span,
                    line: doc.line_of(s.span.start),
                    children,
                });
            }
            StmtKind::Call(e) => {
                let name = match &e.kind {
                    ExprKind::Call { func, .. } => match &func.kind {
                        ExprKind::Name(n) => Some(n.as_str()),
                        ExprKind::Field { name, .. } => Some(name.name.as_str()),
                        _ => None,
                    },
                    ExprKind::Method { name, .. } => Some(name.name.as_str()),
                    _ => None,
                };
                if let Some(n) = name.filter(|n| hooks::is_trigger_registration(n)) {
                    out.push(LuaOutlineItem {
                        kind: LuaOutlineKind::Trigger,
                        label: n.to_string(),
                        span: s.span,
                        line: doc.line_of(s.span.start),
                        children: vec![],
                    });
                }
            }
            StmtKind::Local { values, .. } | StmtKind::Assign { values, .. } => {
                // `local t = RegisterTimerEvent(...)`: triggers assigned to variables
                for v in values {
                    if let ExprKind::Call { func, .. } = &v.kind {
                        let n = match &func.kind {
                            ExprKind::Name(n) => Some(n.as_str()),
                            _ => None,
                        };
                        if let Some(n) = n.filter(|n| hooks::is_trigger_registration(n)) {
                            out.push(LuaOutlineItem {
                                kind: LuaOutlineKind::Trigger,
                                label: n.to_string(),
                                span: s.span,
                                line: doc.line_of(s.span.start),
                                children: vec![],
                            });
                        }
                    }
                }
            }
            StmtKind::Do(b)
            | StmtKind::While { body: b, .. }
            | StmtKind::Repeat { body: b, .. }
            | StmtKind::NumericFor { body: b, .. }
            | StmtKind::GenericFor { body: b, .. } => collect_outline(doc, b, out),
            StmtKind::If { arms, else_block } => {
                for (_, b) in arms {
                    collect_outline(doc, b, out);
                }
                if let Some(b) = else_block {
                    collect_outline(doc, b, out);
                }
            }
            _ => {}
        }
    }
}

/// Functions, hooks, trigger registrations and `---@class` declarations in source order.
pub fn outline(doc: &LuaDoc) -> Vec<LuaOutlineItem> {
    let mut out = Vec::new();
    collect_outline(doc, &doc.chunk.body, &mut out);
    for b in &doc.blocks {
        for (t, span) in &b.tags {
            if let Tag::Class { name, .. } = t {
                out.push(LuaOutlineItem {
                    kind: LuaOutlineKind::Class,
                    label: name.clone(),
                    span: *span,
                    line: doc.line_of(span.start),
                    children: vec![],
                });
            }
        }
    }
    out.sort_by_key(|i| i.span.start);
    out
}

// ---------------------------------------------------------------------------------------------
// Go to definition

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LuaLocation {
    /// `None` = this file; otherwise the path relative to its library root.
    pub file: Option<String>,
    pub span: Span,
}

fn loc(doc: &LuaDoc, s: &super::index::Source) -> LuaLocation {
    if s.file == doc.file {
        LuaLocation {
            file: None,
            span: Span::new(s.start, s.end),
        }
    } else {
        LuaLocation {
            file: Some(s.file.clone()),
            span: Span::new(s.start, s.end),
        }
    }
}

/// Definition of the identifier at `offset`: locals, this file's globals, then the library.
pub fn definition(doc: &LuaDoc, offset: usize, index: &ApiIndex) -> Option<LuaLocation> {
    let env = Env::new(doc, index);
    let ti = doc
        .token_at(offset)
        .or_else(|| offset.checked_sub(1).and_then(|o| doc.token_at(o)))?;
    let t = doc.tokens[ti];
    if t.kind != TokKind::Name {
        return None;
    }
    let name = t.text(&doc.src);
    // member?
    if let Some(p) = doc.prev_sig_token(t.span.start) {
        let pt = doc.tokens[p];
        let ptext = pt.text(&doc.src);
        if pt.kind == TokKind::Punct && (ptext == "." || ptext == ":") {
            let mut base: Option<&Expr> = None;
            walk_block_exprs(&doc.chunk.body, &mut |e| match &e.kind {
                ExprKind::Field { obj, dot, .. } if *dot == pt.span => base = Some(obj),
                ExprKind::Method { obj, colon, .. } if *colon == pt.span => base = Some(obj),
                _ => {}
            });
            let bty = env.infer(base?, 0)?;
            for m in bty.members() {
                let class = match &m {
                    Ty::Name(c) => c.clone(),
                    Ty::Generic(g, a) if g == "typeof" => match a.first() {
                        Some(Ty::Name(c)) => c.clone(),
                        _ => continue,
                    },
                    _ => continue,
                };
                match env.find_member(&class, name) {
                    Some(Member::Method(d, _)) => return d.source.as_ref().map(|s| loc(doc, s)),
                    Some(Member::Field(f, _)) => return f.source.as_ref().map(|s| loc(doc, s)),
                    None => {}
                }
            }
            return None;
        }
    }
    if let Some(r) = doc.analysis.ref_at(offset).filter(|r| r.span == t.span) {
        if let Some(l) = r.local {
            return Some(LuaLocation {
                file: None,
                span: doc.analysis.locals[l].decl,
            });
        }
    }
    // global in this file first (the whole file's defs), then the library
    if let Some(f) = doc.defs.functions.get(name) {
        return f.source.as_ref().map(|s| loc(doc, s));
    }
    if let Some(g) = doc.defs.globals.get(name) {
        return g.source.as_ref().map(|s| loc(doc, s));
    }
    if let Some(c) = doc.defs.classes.get(name) {
        return c.source.as_ref().map(|s| loc(doc, s));
    }
    if let Some(f) = index.functions.get(name) {
        return f.source.as_ref().map(|s| loc(doc, s));
    }
    if let Some(g) = index.globals.get(name) {
        return g.source.as_ref().map(|s| loc(doc, s));
    }
    if let Some(c) = index.classes.get(name) {
        return c.source.as_ref().map(|s| loc(doc, s));
    }
    if let Some(a) = index
        .aliases
        .get(name)
        .or_else(|| doc.defs.aliases.get(name))
    {
        return a.source.as_ref().map(|s| loc(doc, s));
    }
    // a global assigned in this file
    doc.analysis
        .global_assigns
        .iter()
        .find(|g| g.name == name)
        .map(|g| LuaLocation {
            file: None,
            span: g.span,
        })
}

// ---------------------------------------------------------------------------------------------
// Map references

fn int_of(e: &Expr) -> Option<i32> {
    match &e.kind {
        ExprKind::Number(n) => {
            let n = n.trim_end_matches(|c: char| matches!(c, 'L' | 'l' | 'U' | 'u'));
            if let Some(h) = n.strip_prefix("0x").or_else(|| n.strip_prefix("0X")) {
                i64::from_str_radix(h, 16)
                    .ok()
                    .and_then(|v| i32::try_from(v).ok())
            } else if n.bytes().all(|b| b.is_ascii_digit()) {
                n.parse().ok()
            } else {
                None
            }
        }
        ExprKind::Unary { op: "-", operand } => int_of(operand).map(|v| -v),
        _ => None,
    }
}

fn norm(s: &str) -> String {
    s.to_ascii_lowercase().replace('_', "")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

/// `(is slab, axis)` for coordinate-like parameter names (`stl_x`, `next_stl_y`, `slab_x`,
/// `slb_y`).
fn coord_param(name: &str) -> Option<(bool, Axis)> {
    let n = name.to_ascii_lowercase();
    let axis = if n.ends_with("_x") || n == "x" {
        Axis::X
    } else if n.ends_with("_y") || n == "y" {
        Axis::Y
    } else {
        return None;
    };
    if n.contains("stl") || n.contains("subtile") {
        Some((false, axis))
    } else if n.contains("slb") || n.contains("slab") {
        Some((true, axis))
    } else {
        None
    }
}

struct RawRefs {
    refs: Vec<ScriptRef>,
    pos: Vec<ScriptPos>,
}

fn scan_map_refs(doc: &LuaDoc, index: &ApiIndex) -> RawRefs {
    let env = Env::new(doc, index);
    let mut out = RawRefs {
        refs: vec![],
        pos: vec![],
    };
    let mut calls: Vec<&Expr> = Vec::new();
    walk_block_exprs(&doc.chunk.body, &mut |e| {
        if matches!(e.kind, ExprKind::Call { .. } | ExprKind::Method { .. }) {
            calls.push(e);
        }
    });
    for e in calls {
        let (sig, args, cname) = match &e.kind {
            ExprKind::Call { func, args } => {
                let n = func.path().map(|p| p.join(".")).unwrap_or_default();
                (env.resolve_callee(func), args, n)
            }
            ExprKind::Method {
                obj, name, args, ..
            } => (
                env.resolve_method_call(obj, &name.name),
                args,
                name.name.clone(),
            ),
            _ => continue,
        };
        let Some(sig) = sig else { continue };
        let short = cname.rsplit('.').next().unwrap_or(&cname).to_string();
        let mut pending_x: Option<(bool, i32, Span, String)> = None;
        for (i, a) in args.items.iter().enumerate() {
            let pi = i as i32 + sig.shift;
            if pi < 0 {
                continue;
            }
            let Some(p) = sig.def.params.get(pi as usize) else {
                continue;
            };
            let pn = norm(&p.name);
            let ty = parse_ty(&p.ty);
            let alias_names: Vec<String> = ty
                .members()
                .into_iter()
                .filter_map(|m| if let Ty::Name(n) = m { Some(n) } else { None })
                .collect();
            let Some(v) = int_of(a) else {
                pending_x = None;
                continue;
            };
            if pn.contains("herogate") || alias_names.iter().any(|n| norm(n) == "herogate") {
                out.refs.push(ScriptRef {
                    span: a.span,
                    kind: RefKind::HeroGate,
                    value: v,
                    command: short.clone(),
                });
            } else if matches!(
                pn.as_str(),
                "actionpoint" | "ap" | "actionpointnumber" | "apnumber"
            ) || alias_names
                .iter()
                .any(|n| n == "actionpoint" || n == "location")
            {
                out.refs.push(ScriptRef {
                    span: a.span,
                    kind: RefKind::ActionPoint,
                    value: v,
                    command: short.clone(),
                });
            } else if let Some((slab, axis)) = coord_param(&p.name) {
                let kind = match (slab, axis) {
                    (false, Axis::X) => RefKind::SubtileX,
                    (false, Axis::Y) => RefKind::SubtileY,
                    (true, Axis::X) => RefKind::SlabX,
                    (true, Axis::Y) => RefKind::SlabY,
                };
                out.refs.push(ScriptRef {
                    span: a.span,
                    kind,
                    value: v,
                    command: short.clone(),
                });
                match axis {
                    Axis::X => pending_x = Some((slab, v, a.span, p.name.clone())),
                    Axis::Y => {
                        if let Some((s2, x, xs, xn)) = pending_x.take() {
                            // the y parameter must mirror the x parameter's name
                            if s2 == slab && xn[..xn.len() - 1] == p.name[..p.name.len() - 1] {
                                out.pos.push(ScriptPos {
                                    x,
                                    y: v,
                                    x_span: xs,
                                    y_span: a.span,
                                    slab,
                                    command: cname.clone(),
                                });
                            }
                        }
                    }
                }
                continue;
            }
            pending_x = None;
        }
    }
    out.refs.sort_by_key(|r| r.span.start);
    out.pos.sort_by_key(|p| p.x_span.start);
    out
}

/// Action points, hero gates and coordinates used as literal arguments of annotated API calls.
///
/// A parameter counts as an action point when its name is `actionPoint`/`action_point` or its type
/// is the `actionpoint` or `location` alias (a number given for a `location` is an action point,
/// `bindings/effects.lua`), a hero gate when its name or type mentions `herogate`, a coordinate
/// when it is named `stl_x`/`slab_y`/... (`stl`, `subtile` = subtiles, `slb`, `slab` = slabs).
pub fn references(doc: &LuaDoc, index: &ApiIndex) -> Vec<ScriptRef> {
    scan_map_refs(doc, index).refs
}

/// Coordinate pairs written as literals (for markers and "pick from map").
pub fn positions(doc: &LuaDoc, index: &ApiIndex) -> Vec<ScriptPos> {
    scan_map_refs(doc, index).pos
}

// ---------------------------------------------------------------------------------------------
// Formatter

#[derive(Clone, Debug)]
pub struct LuaFormatOptions {
    /// Text repeated once per nesting level.
    pub indent: String,
}

impl Default for LuaFormatOptions {
    fn default() -> Self {
        LuaFormatOptions {
            indent: "    ".into(),
        }
    }
}

struct Entry {
    line: usize,
    contributes: bool,
    /// `if` entries stay open across `else`/`elseif`.
    is_if: bool,
}

/// Re-indent code. Only the leading whitespace of lines changes; the contents of multi-line
/// strings and block comments, blank lines and trailing whitespace are untouched.
///
/// Blocks (`function`, `if`, `do`, loops, `repeat`) and brackets add one level each, except that
/// several openers on the same line add a single level (so `foo(function()` indents its body once).
pub fn format(doc: &LuaDoc, opts: &LuaFormatOptions) -> String {
    let src = &doc.src;
    let sig: Vec<usize> = (0..doc.tokens.len())
        .filter(|&i| !doc.tokens[i].is_trivia() && doc.tokens[i].kind != TokKind::Eof)
        .collect();
    // first significant-or-comment token of each line
    let nlines = doc.line_of(src.len()) + 1;
    let mut indent_of_line: Vec<Option<usize>> = vec![None; nlines + 1];
    let mut closer_first: Vec<bool> = vec![false; nlines + 1];
    let mut stack: Vec<Entry> = Vec::new();
    let contributing = |st: &Vec<Entry>| st.iter().filter(|e| e.contributes).count();
    let mut pending_do_owner = 0usize; // `while`/`for` waiting for their `do`
    let mut k = 0;
    let line_of_tok = |i: usize| doc.line_of(doc.tokens[i].span.start);
    while k < sig.len() {
        let i = sig[k];
        let line = line_of_tok(i);
        // start of a new line of code: tokens on this line
        let first_on_line = k == 0
            || line_of_tok(sig[k - 1]) != line
            || doc.tokens[sig[k - 1]].span.end <= doc.line_start(line);
        let first_on_line = first_on_line
            && src[doc.line_start(line)..doc.tokens[i].span.start]
                .trim()
                .is_empty()
            && !prev_token_spans_into_line(doc, i, line);
        if first_on_line {
            // leading closers on this line
            let mut dedent = 0usize;
            let mut sim: Vec<bool> = stack.iter().map(|e| e.contributes).collect();
            let mut j = k;
            let mut else_line = false;
            while j < sig.len() && line_of_tok(sig[j]) == line {
                let t = doc.tokens[sig[j]];
                let text = t.text(src);
                let closer = t.kind == TokKind::Keyword && matches!(text, "end" | "until")
                    || t.kind == TokKind::Punct && matches!(text, ")" | "}" | "]");
                if closer {
                    if let Some(c) = sim.pop() {
                        if c {
                            dedent += 1;
                        }
                    }
                } else if j == k && t.kind == TokKind::Keyword && matches!(text, "else" | "elseif")
                {
                    if stack.last().map_or(false, |e| e.is_if && e.contributes) {
                        dedent += 1;
                    }
                    else_line = true;
                    break;
                } else {
                    break;
                }
                j += 1;
            }
            closer_first[line] = dedent > 0 || else_line;
            let depth = contributing(&stack).saturating_sub(dedent);
            indent_of_line[line] = Some(depth);
        }
        // update the stack with this token
        let t = doc.tokens[i];
        let text = t.text(src);
        let push = |stack: &mut Vec<Entry>, is_if: bool| {
            let has_contrib_on_line = stack.iter().any(|e| e.line == line && e.contributes);
            stack.push(Entry {
                line,
                contributes: !has_contrib_on_line,
                is_if,
            });
        };
        if t.kind == TokKind::Keyword {
            match text {
                "function" | "repeat" => push(&mut stack, false),
                "if" => push(&mut stack, true),
                "while" | "for" => {
                    pending_do_owner += 1;
                    push(&mut stack, false);
                }
                "do" => {
                    if pending_do_owner > 0 {
                        pending_do_owner -= 1;
                    } else {
                        push(&mut stack, false);
                    }
                }
                "end" | "until" => {
                    stack.pop();
                }
                _ => {}
            }
        } else if t.kind == TokKind::Punct {
            match text {
                "(" | "{" | "[" => push(&mut stack, false),
                ")" | "}" | "]" => {
                    stack.pop();
                }
                _ => {}
            }
        }
        k += 1;
    }

    // comment-only lines take the indentation of the code that follows (one level deeper when
    // that code closes a block)
    for t in &doc.tokens {
        if matches!(t.kind, TokKind::LineComment | TokKind::BlockComment) {
            let line = doc.line_of(t.span.start);
            if src[doc.line_start(line)..t.span.start].trim().is_empty()
                && indent_of_line[line].is_none()
            {
                let next = (line + 1..indent_of_line.len()).find(|&l| indent_of_line[l].is_some());
                if let Some(n) = next {
                    let d = indent_of_line[n].unwrap_or(0);
                    indent_of_line[line] = Some(if closer_first[n] { d + 1 } else { d });
                }
            }
        }
    }

    // rebuild: only lines whose first token starts the line, outside multi-line tokens
    let mut out = String::with_capacity(src.len());
    let mut inside_multi: Vec<Span> = Vec::new();
    for t in &doc.tokens {
        if matches!(t.kind, TokKind::LongString | TokKind::BlockComment)
            && src[t.span.start..t.span.end].contains('\n')
        {
            inside_multi.push(t.span);
        }
    }
    let mut ls = 0;
    let mut lineno = 1;
    while ls <= src.len() {
        let le = src[ls..].find('\n').map_or(src.len(), |i| ls + i);
        let line_text = &src[ls..le];
        let in_multi = inside_multi.iter().any(|s| s.start < ls && ls < s.end);
        let trimmed = line_text.trim_start_matches([' ', '\t']);
        match indent_of_line.get(lineno).copied().flatten() {
            Some(d) if !in_multi && !trimmed.trim().is_empty() => {
                out.push_str(&opts.indent.repeat(d));
                out.push_str(trimmed);
            }
            _ => out.push_str(line_text),
        }
        if le < src.len() {
            out.push('\n');
        }
        ls = le + 1;
        lineno += 1;
        if le >= src.len() {
            break;
        }
    }
    out
}

fn prev_token_spans_into_line(doc: &LuaDoc, i: usize, line: usize) -> bool {
    // true when a multi-line token (long string / block comment) ends on this line before token `i`
    let ls = doc.line_start(line);
    doc.tokens[..i].iter().rev().take(3).any(|t| {
        matches!(t.kind, TokKind::LongString | TokKind::BlockComment)
            && t.span.start < ls
            && t.span.end > ls
    })
}

/// Names and kinds of the functions found in a file.
pub fn function_kinds(doc: &LuaDoc) -> Vec<(String, FuncKind)> {
    doc.analysis
        .funcs
        .iter()
        .filter_map(|f| f.name.clone().map(|n| (n, f.kind)))
        .collect()
}
