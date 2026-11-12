//! Diagnostics for Lua level scripts.
//!
//! Codes: `lua-syntax` (Error), `lua-undefined-global`, `lua-global-assign`, `lua-arg-count`,
//! `lua-bad-literal`, `lua-arg-type` (literal or literal-initialised local of the wrong type),
//! `lua-sandbox-removed`, `lua-mp-unsafe`, `lua-trigger-local-fn`, `lua-trigger-capture`,
//! `lua-hook-state` (Warning), `lua-anon-callback-layout`, `lua-deprecated`, `lua-unknown-hook`
//! (Info).
//!
//! Escape hatches: `---@diagnostic disable-next-line: <code>`, `disable-line`, `disable`/`enable`
//! (codes with or without the `lua-` prefix; no code list means all), `-- luacheck: ignore` for a
//! line and `-- luacheck: globals a b` for names, plus `_G.name = ...`/`rawset(_G, ...)`
//! definitions and `setmetatable(_G, ...)` (dynamic globals: undefined-global is skipped).
use super::analyze::{walk_block_exprs, walk_stmts, LocalKind};
use super::annot::{parse_ty, Ty};
use super::ast::*;
use super::doc::LuaDoc;
use super::hooks;
use super::index::{ApiIndex, FuncDef};
use super::infer::{find_expr, Env};
use super::lexer::TokKind;
use super::stdlib;
use crate::diag::{Diagnostic, Severity, Span};
use crate::provider::{ConstKind, ConstantProvider};
use crate::util::levenshtein;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LuaDiagnostic {
    pub severity: Severity,
    pub span: Span,
    /// 1-based line of `span.start`.
    pub line: usize,
    pub message: String,
    pub code: &'static str,
    /// Replacement text for the span (for bad string literals: the corrected name, unquoted).
    pub suggestion: Option<String>,
}

impl From<&LuaDiagnostic> for Diagnostic {
    fn from(d: &LuaDiagnostic) -> Diagnostic {
        Diagnostic {
            severity: d.severity,
            span: d.span,
            line: d.line,
            message: d.message.clone(),
            code: d.code,
        }
    }
}

// ---- suppression comments ---------------------------------------------------------------------

struct Supp {
    from: usize,
    to: usize,
    codes: Vec<String>,
}

struct Directives {
    supps: Vec<Supp>,
    luacheck_globals: BTreeSet<String>,
}

fn aliases(code: &str) -> Vec<&str> {
    let short = code.strip_prefix("lua-").unwrap_or(code);
    let mut v = vec![code, short];
    match code {
        "lua-arg-count" => v.extend(["redundant-parameter", "missing-parameter"]),
        "lua-bad-literal" | "lua-arg-type" => v.push("param-type-mismatch"),
        "lua-global-assign" => v.extend(["lowercase-global", "global-assign"]),
        "lua-deprecated" => v.push("deprecated"),
        _ => {}
    }
    v
}

fn directives(doc: &LuaDoc) -> Directives {
    let mut d = Directives {
        supps: vec![],
        luacheck_globals: BTreeSet::new(),
    };
    let mut open: Vec<(usize, Vec<String>)> = Vec::new();
    for t in &doc.tokens {
        if t.kind != TokKind::LineComment {
            continue;
        }
        let text = t.text(&doc.src).trim_start_matches('-').trim();
        let line = doc.line_of(t.span.start);
        if let Some(rest) = text.strip_prefix("@diagnostic") {
            let rest = rest.trim();
            let (action, codes) = match rest.find(':') {
                Some(i) => (
                    rest[..i].trim(),
                    rest[i + 1..]
                        .split(',')
                        .map(|c| c.trim().to_string())
                        .filter(|c| !c.is_empty())
                        .collect::<Vec<_>>(),
                ),
                None => (rest, vec![]),
            };
            match action {
                "disable-next-line" => d.supps.push(Supp {
                    from: line + 1,
                    to: line + 1,
                    codes,
                }),
                "disable-line" => d.supps.push(Supp {
                    from: line,
                    to: line,
                    codes,
                }),
                "disable" => open.push((line, codes)),
                "enable" => {
                    // close the matching open `disable` entries (all of them when no code list)
                    let mut keep = Vec::new();
                    for (from, c) in open.drain(..) {
                        if codes.is_empty() || c.iter().any(|x| codes.contains(x)) || c.is_empty() {
                            d.supps.push(Supp {
                                from,
                                to: line,
                                codes: c,
                            });
                        } else {
                            keep.push((from, c));
                        }
                    }
                    open = keep;
                }
                _ => {}
            }
        } else if let Some(rest) = text.strip_prefix("luacheck:") {
            let rest = rest.trim();
            if let Some(g) = rest.strip_prefix("globals") {
                for n in g.split_whitespace() {
                    d.luacheck_globals.insert(n.to_string());
                }
            } else if let Some(g) = rest.strip_prefix("ignore") {
                let codes: Vec<String> = g.split_whitespace().map(|s| s.to_string()).collect();
                let _ = codes;
                d.supps.push(Supp {
                    from: line,
                    to: line,
                    codes: vec![],
                });
            }
        }
    }
    for (from, codes) in open {
        d.supps.push(Supp {
            from,
            to: usize::MAX,
            codes,
        });
    }
    d
}

impl Directives {
    fn suppressed(&self, code: &str, line: usize) -> bool {
        let names = aliases(code);
        self.supps.iter().any(|s| {
            s.from <= line
                && line <= s.to
                && (s.codes.is_empty() || s.codes.iter().any(|c| names.contains(&c.as_str())))
        })
    }
}

// ---- helpers ------------------------------------------------------------------------------

fn alias_kind(name: &str) -> Option<ConstKind> {
    Some(match name {
        "creature_type" => ConstKind::Creature,
        "room_type" => ConstKind::Room,
        "trap_type" => ConstKind::Trap,
        "door_type" => ConstKind::Door,
        "power_kind" => ConstKind::Power,
        "spell_type" => ConstKind::Spell,
        "object_type" => ConstKind::Object,
        "effect_type" => ConstKind::Effect,
        "effect_generator_type" => ConstKind::EffectGen,
        _ => return None,
    })
}

/// Config-driven kinds the type mentions (directly or through other aliases).
pub fn kinds_of_type(env: &Env, ty: &Ty) -> Vec<ConstKind> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    fn go(env: &Env, ty: &Ty, out: &mut Vec<ConstKind>, seen: &mut BTreeSet<String>) {
        for m in ty.members() {
            if let Ty::Name(n) = m {
                if !seen.insert(n.clone()) {
                    continue;
                }
                if let Some(k) = alias_kind(&n) {
                    if !out.contains(&k) {
                        out.push(k);
                    }
                }
                if let Some(a) = env.alias(&n) {
                    go(env, &parse_ty(&a.ty), out, seen);
                }
            }
        }
    }
    go(env, ty, &mut out, &mut seen);
    out
}

/// Best suggestion for `word` among `cands` (case-insensitive).
pub fn suggest<'a>(word: &str, cands: impl Iterator<Item = &'a str>) -> Option<String> {
    let w = word.to_ascii_uppercase();
    let mut best: Option<(usize, &str)> = None;
    for c in cands {
        let cu = c.to_ascii_uppercase();
        let mut d = levenshtein(&w, &cu);
        if cu.starts_with(&w) || (w.len() >= 3 && cu.contains(&w)) {
            d = d.min(1);
        }
        if best.map_or(true, |b| d < b.0) {
            best = Some((d, c));
        }
    }
    let limit = (w.len() / 3).max(2);
    best.filter(|b| b.0 <= limit && b.0 < w.len())
        .map(|b| b.1.to_string())
}

fn is_multi(e: &Expr) -> bool {
    matches!(
        e.kind,
        ExprKind::Call { .. } | ExprKind::Method { .. } | ExprKind::Vararg
    )
}

fn root_of(e: &Expr) -> Option<(&str, Span)> {
    match &e.kind {
        ExprKind::Name(n) => Some((n.as_str(), e.span)),
        ExprKind::Field { obj, .. }
        | ExprKind::Index { obj, .. }
        | ExprKind::Method { obj, .. } => root_of(obj),
        ExprKind::Paren(i) => root_of(i),
        _ => None,
    }
}

struct Ctx<'a> {
    doc: &'a LuaDoc,
    env: Env<'a>,
    provider: &'a dyn ConstantProvider,
    out: Vec<LuaDiagnostic>,
}

impl<'a> Ctx<'a> {
    fn push(
        &mut self,
        severity: Severity,
        span: Span,
        code: &'static str,
        message: String,
        suggestion: Option<String>,
    ) {
        let line = self.doc.line_of(span.start);
        self.out.push(LuaDiagnostic {
            severity,
            span,
            line,
            message,
            code,
            suggestion,
        });
    }
}

// ---- entry point --------------------------------------------------------------------------

pub fn diagnostics(
    doc: &LuaDoc,
    index: &ApiIndex,
    provider: &dyn ConstantProvider,
) -> Vec<LuaDiagnostic> {
    let env = Env::new(doc, index);
    let mut c = Ctx {
        doc,
        env,
        provider,
        out: Vec::new(),
    };
    let dir = directives(doc);

    // 1. syntax
    let mut syntax: Vec<LuaDiagnostic> = Vec::new();
    for e in &doc.lex_errors {
        syntax.push(LuaDiagnostic {
            severity: Severity::Error,
            span: e.span,
            line: doc.line_of(e.span.start),
            message: e.message.clone(),
            code: "lua-syntax",
            suggestion: None,
        });
    }
    for e in &doc.syntax_errors {
        syntax.push(LuaDiagnostic {
            severity: Severity::Error,
            span: e.span,
            line: doc.line_of(e.span.start),
            message: e.message.clone(),
            code: "lua-syntax",
            suggestion: None,
        });
    }

    globals(&mut c, &dir);
    calls(&mut c);
    sandbox(&mut c);
    hook_checks(&mut c);

    let mut out: Vec<LuaDiagnostic> = c
        .out
        .into_iter()
        .filter(|d| !dir.suppressed(d.code, d.line))
        .collect();
    out.extend(syntax);
    out.sort_by_key(|d| (d.span.start, d.span.end));
    out.dedup();
    out
}

// ---- globals ------------------------------------------------------------------------------

fn globals(c: &mut Ctx, dir: &Directives) {
    /// `_G` itself.
    fn is_g(k: &ExprKind) -> bool {
        matches!(k, ExprKind::Name(g) if g == "_G")
    }
    let doc = c.doc;
    // names the file defines by assignment anywhere, and `_G` tricks
    let mut defined: BTreeSet<String> = doc
        .analysis
        .global_assigns
        .iter()
        .map(|g| g.name.clone())
        .collect();
    defined.extend(dir.luacheck_globals.iter().cloned());
    let mut dynamic = false;
    walk_block_exprs(&doc.chunk.body, &mut |e| match &e.kind {
        ExprKind::Call { func, args } => {
            if let ExprKind::Name(f) = &func.kind {
                let first_is_g = matches!(
                    args.items.first().map(|a| &a.kind),
                    Some(ExprKind::Name(g)) if g == "_G"
                );
                if f == "setmetatable" && first_is_g {
                    dynamic = true;
                }
                if f == "rawset" && first_is_g {
                    if let Some(Expr {
                        kind: ExprKind::Str(s),
                        ..
                    }) = args.items.get(1)
                    {
                        defined.insert(s.clone());
                    } else {
                        dynamic = true;
                    }
                }
            }
        }
        _ => {}
    });
    walk_stmts(&doc.chunk.body, &mut |s| {
        if let StmtKind::Assign { targets, .. } = &s.kind {
            for t in targets {
                match &t.kind {
                    ExprKind::Field { obj, name, .. } if is_g(&obj.kind) => {
                        defined.insert(name.name.clone());
                    }
                    ExprKind::Index { obj, key } if is_g(&obj.kind) => {
                        if let ExprKind::Str(k) = &key.kind {
                            defined.insert(k.clone());
                        } else {
                            dynamic = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    });

    // known names for suggestions
    let mut known_names: Vec<&str> = Vec::new();
    known_names.extend(c.env.index.functions.keys().map(|s| s.as_str()));
    known_names.extend(c.env.index.globals.keys().map(|s| s.as_str()));
    known_names.extend(doc.defs.functions.keys().map(|s| s.as_str()));
    known_names.extend(doc.defs.globals.keys().map(|s| s.as_str()));
    known_names.extend(stdlib::BASE_GLOBALS.iter().copied());

    let mut reported: Vec<LuaDiagnostic> = Vec::new();
    let mut seen_assign: BTreeSet<String> = BTreeSet::new();
    if !dynamic {
        for r in &doc.analysis.refs {
            if r.local.is_some() || r.write {
                continue;
            }
            let n = r.name.as_str();
            if n == "_ENV"
                || c.env.knows_global(n)
                || defined.contains(n)
                || stdlib::UNOPENED_LIBS.contains(&n)
            {
                continue;
            }
            let sug = suggest(n, known_names.iter().copied());
            let msg = match &sug {
                Some(s) => format!("undefined global '{n}' (did you mean '{s}'?)"),
                None => format!("undefined global '{n}'"),
            };
            let line = doc.line_of(r.span.start);
            reported.push(LuaDiagnostic {
                severity: Severity::Warning,
                span: r.span,
                line,
                message: msg,
                code: "lua-undefined-global",
                suggestion: sug,
            });
        }
    }
    c.out.extend(reported);

    // assignment to an undeclared global from inside a function
    for g in &doc.analysis.global_assigns {
        if g.top_level || g.is_function {
            continue;
        }
        let top_defined = doc
            .analysis
            .global_assigns
            .iter()
            .any(|o| o.top_level && o.name == g.name);
        if top_defined
            || c.env.knows_global(&g.name)
            || dir.luacheck_globals.contains(&g.name)
            || g.name == "_ENV"
        {
            continue;
        }
        if !seen_assign.insert(g.name.clone()) {
            // report each name once at its first assignment
            continue;
        }
        c.push(
            Severity::Warning,
            g.span,
            "lua-global-assign",
            format!(
                "assignment to undeclared global '{}' (use 'local {}', or define it at file level)",
                g.name, g.name
            ),
            None,
        );
    }
}

// ---- calls ----------------------------------------------------------------------------------

fn arities(def: &FuncDef) -> Vec<(usize, Option<usize>)> {
    // (min, max) per accepted signature; max None = unbounded
    let mut v = vec![(
        def.required_params(),
        if def.vararg.is_some() {
            None
        } else {
            Some(def.params.len())
        },
    )];
    for o in &def.overloads {
        if let Ty::Fun(f) = parse_ty(o) {
            let min = f
                .params
                .iter()
                .rposition(|(_, t, opt)| !*opt && !t.accepts_nil())
                .map_or(0, |i| i + 1);
            let var = f
                .params
                .iter()
                .any(|(n, t, _)| n == "..." || matches!(t, Ty::Vararg(_)));
            v.push((min, if var { None } else { Some(f.params.len()) }));
        }
    }
    v
}

fn calls(c: &mut Ctx) {
    let doc = c.doc;
    let mut callbacks: Vec<(&FuncBody, Span)> = Vec::new();
    let mut anon_lines: BTreeMap<usize, usize> = BTreeMap::new();
    let mut work: Vec<&Expr> = Vec::new();
    walk_block_exprs(&doc.chunk.body, &mut |e| {
        if matches!(e.kind, ExprKind::Call { .. } | ExprKind::Method { .. }) {
            work.push(e);
        }
    });
    for e in work {
        let (sig, args, name_span, simple_name) = match &e.kind {
            ExprKind::Call { func, args } => {
                let sig = c.env.resolve_callee(func);
                let (ns, sn) = match &func.kind {
                    ExprKind::Name(n) => (func.span, n.clone()),
                    ExprKind::Field { name, .. } => (name.span, name.name.clone()),
                    _ => (func.span, String::new()),
                };
                (sig, args, ns, sn)
            }
            ExprKind::Method {
                obj, name, args, ..
            } => (
                c.env.resolve_method_call(obj, &name.name),
                args,
                name.span,
                name.name.clone(),
            ),
            _ => continue,
        };

        if let Some(sig) = &sig {
            check_args(c, sig.def, sig.shift, &sig.display, args, name_span);
            if let Some(msg) = &sig.def.deprecated {
                let m = if msg.is_empty() {
                    format!("'{}' is deprecated", sig.display)
                } else {
                    format!("'{}' is deprecated: {}", sig.display, msg)
                };
                c.push(Severity::Info, name_span, "lua-deprecated", m, None);
            }
        }

        if hooks::is_trigger_registration(&simple_name) {
            for a in &args.items {
                match &a.kind {
                    ExprKind::Function(f) => {
                        callbacks.push((f, a.span));
                        *anon_lines.entry(doc.line_of(a.span.start)).or_insert(0) += 1;
                    }
                    ExprKind::Name(n) => {
                        if let Some(l) = doc.analysis.ref_at(a.span.start).and_then(|r| r.local) {
                            let lv = &doc.analysis.locals[l];
                            if lv.kind == LocalKind::LocalFunction
                                || lv.init_kind == super::analyze::InitKind::Function
                            {
                                c.push(
                                    Severity::Warning,
                                    a.span,
                                    "lua-trigger-local-fn",
                                    format!(
                                        "trigger callback '{n}' is a local function; saves \
                                        restore it as an anonymous function. Give it a global name \
                                        so saves stay portable"
                                    ),
                                    None,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    for (f, span) in callbacks {
        // closures capturing tables/functions are rejected by CreateTrigger/TriggerAddCondition
        if let Some(info) = doc.analysis.func_by_span(f.span) {
            for l in doc.analysis.captures(info) {
                let lv = &doc.analysis.locals[l];
                let bad = lv.kind == LocalKind::LocalFunction
                    || matches!(
                        lv.init_kind,
                        super::analyze::InitKind::Table | super::analyze::InitKind::Function
                    );
                if bad {
                    let what = if lv.init_kind == super::analyze::InitKind::Table {
                        "table"
                    } else {
                        "function"
                    };
                    c.push(
                        Severity::Warning,
                        lv_use_span(doc, f, &lv.name).unwrap_or(span),
                        "lua-trigger-capture",
                        format!(
                            "trigger callback captures the local {what} '{}'; saves can only \
                            store numbers, strings and booleans (keep it in the Game table or make \
                            it global)",
                            lv.name
                        ),
                        None,
                    );
                }
            }
        }
        // layout: starts on its own line, ends on its own line
        let start_line = doc.line_of(span.start);
        let first_on_line = {
            let ls = doc.line_start(start_line);
            doc.src[ls..span.start].trim().is_empty()
        };
        let end_ok = {
            let le = doc.src[span.end..]
                .find('\n')
                .map_or(doc.src.len(), |i| span.end + i);
            let rest = doc.src[span.end..le].trim();
            let rest = rest.split("--").next().unwrap_or("").trim();
            rest.chars()
                .all(|ch| matches!(ch, ')' | ',' | ';' | '}' | ' ' | '\t' | '\r'))
        };
        let multi = anon_lines.get(&start_line).copied().unwrap_or(0) > 1;
        if !first_on_line || !end_ok || multi {
            c.push(
                Severity::Info,
                Span::new(span.start, (span.start + 8).min(span.end)),
                "lua-anon-callback-layout",
                "anonymous trigger callback should start and end on its own lines (one-liners and \
                    two callbacks on a line cannot be rebuilt from a save; the game falls back to \
                    non-portable bytecode)"
                    .into(),
                None,
            );
        }
    }
}

fn lv_use_span(doc: &LuaDoc, f: &FuncBody, name: &str) -> Option<Span> {
    doc.analysis
        .refs
        .iter()
        .find(|r| r.name == name && f.span.contains(r.span.start))
        .map(|r| r.span)
}

fn check_args(c: &mut Ctx, def: &FuncDef, shift: i32, display: &str, args: &Args, name_span: Span) {
    if args.style == ArgStyle::Paren && args.open.is_some() && args.close.is_none() {
        return; // half-typed call
    }
    let n = args.items.len() as i32;
    let provided = (n + shift).max(0) as usize;
    let last_multi = args.items.last().map_or(false, is_multi);
    let ar = arities(def);
    let too_many = ar
        .iter()
        .all(|(_, max)| max.map_or(false, |m| provided > m));
    let too_few = !last_multi && def.annotated && ar.iter().all(|(min, _)| provided < *min);
    if too_many {
        let max = ar.iter().filter_map(|a| a.1).max().unwrap_or(0);
        let start = args
            .items
            .get((max as i32 - shift).max(0) as usize)
            .map_or(args.span, |a| a.span);
        c.push(
            Severity::Warning,
            start,
            "lua-arg-count",
            format!("'{display}' takes at most {max} argument(s) but {provided} given"),
            None,
        );
    } else if too_few {
        let min = ar.iter().map(|a| a.0).min().unwrap_or(0);
        let span = args.close.unwrap_or(name_span);
        let missing = def
            .params
            .get(provided)
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        c.push(
            Severity::Warning,
            span,
            "lua-arg-count",
            format!(
                "'{display}' needs at least {min} \
            argument(s) but {provided} given (missing '{missing}')"
            ),
            None,
        );
    }

    for (i, a) in args.items.iter().enumerate() {
        let pi = i as i32 + shift;
        if pi < 0 {
            continue;
        }
        let Some(p) = def.params.get(pi as usize) else {
            continue;
        };
        if !p.annotated {
            continue;
        }
        let ty = parse_ty(&p.ty);
        // value-level check of string literals against alias unions
        if let ExprKind::Str(value) = &a.kind {
            let vals = c.env.type_values(&ty);
            let lits: Vec<&str> = vals
                .literals
                .iter()
                .filter(|l| l.is_str)
                .map(|l| l.value.as_str())
                .collect();
            if !vals.open && !lits.is_empty() {
                let mut valid: Vec<String> = lits.iter().map(|s| s.to_string()).collect();
                for k in kinds_of_type(&c.env, &ty) {
                    valid.extend(c.provider.names(k).iter().cloned());
                }
                let upper = value.to_ascii_uppercase();
                if upper == "RANDOM" || valid.iter().any(|v| v.to_ascii_uppercase() == upper) {
                    continue;
                }
                let sug = suggest(value, valid.iter().map(|s| s.as_str()));
                let msg = match &sug {
                    Some(s) => format!(
                        "'{value}' is not a valid value for '{}' of {display} (did \
                        you mean '{s}'?)",
                        p.name
                    ),
                    None => format!(
                        "'{value}' is not a valid value for '{}' of {display} \
                        (expected {})",
                        p.name, p.ty
                    ),
                };
                c.push(Severity::Warning, a.span, "lua-bad-literal", msg, sug);
                continue;
            }
        }
        // type-level check of literals and obviously typed locals
        let Some(kind) = value_kind(c, a) else {
            continue;
        };
        if c.env.accepts(&ty, p.optional, &kind) == Accept::No {
            let msg = format!(
                "argument '{}' of {display} should be {}, got {}",
                p.name,
                p.ty.trim_end_matches('?'),
                kind.describe()
            );
            c.push(Severity::Warning, a.span, "lua-arg-type", msg, None);
        }
    }
}

/// Primitive kind of an argument expression that is certain enough to check.
#[derive(Clone, Debug, PartialEq)]
pub enum ValueKind {
    Str(Option<String>),
    Num,
    Bool,
    Nil,
    Table,
    Function,
}

impl ValueKind {
    fn describe(&self) -> String {
        match self {
            ValueKind::Str(Some(s)) => format!("string \"{s}\""),
            ValueKind::Str(None) => "string".into(),
            ValueKind::Num => "number".into(),
            ValueKind::Bool => "boolean".into(),
            ValueKind::Nil => "nil".into(),
            ValueKind::Table => "table".into(),
            ValueKind::Function => "function".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accept {
    Yes,
    No,
}

fn value_kind(c: &Ctx, e: &Expr) -> Option<ValueKind> {
    Some(match &e.kind {
        ExprKind::Str(s) => ValueKind::Str(Some(s.clone())),
        ExprKind::Number(_) => ValueKind::Num,
        ExprKind::True | ExprKind::False => ValueKind::Bool,
        ExprKind::Nil => ValueKind::Nil,
        ExprKind::Table(_) => ValueKind::Table,
        ExprKind::Function(_) => ValueKind::Function,
        ExprKind::Paren(i) => return value_kind(c, i),
        ExprKind::Name(_) => {
            // a never reassigned local initialised from a literal
            let id = c.doc.analysis.ref_at(e.span.start).and_then(|r| r.local)?;
            let l = &c.doc.analysis.locals[id];
            if l.kind != LocalKind::Local
                || l.reassigned
                || l.init_kind != super::analyze::InitKind::Scalar
                || c.doc
                    .declared_type(l.stmt_start, l.scope.start, l.index)
                    .is_some()
            {
                return None;
            }
            let init = find_expr(&c.doc.chunk.body, l.init?)?;
            match &init.kind {
                ExprKind::Str(s) => ValueKind::Str(Some(s.clone())),
                ExprKind::Number(_) => ValueKind::Num,
                ExprKind::True | ExprKind::False => ValueKind::Bool,
                _ => return None,
            }
        }
        _ => return None,
    })
}

impl<'a> Env<'a> {
    /// Whether a value of `kind` can be passed where `ty` is declared. Anything not clearly
    /// incompatible (unions with `any`, unknown names, generics) is accepted.
    pub fn accepts(&self, ty: &Ty, optional: bool, kind: &ValueKind) -> Accept {
        if optional && *kind == ValueKind::Nil {
            return Accept::Yes;
        }
        let mut seen = BTreeSet::new();
        if self.accepts_ty(ty, kind, &mut seen, 0) {
            Accept::Yes
        } else {
            Accept::No
        }
    }

    fn accepts_ty(
        &self,
        ty: &Ty,
        kind: &ValueKind,
        seen: &mut BTreeSet<String>,
        depth: u32,
    ) -> bool {
        if depth > 8 {
            return true;
        }
        let numeric_str = |s: &str| s.trim().parse::<f64>().is_ok();
        for m in ty.members() {
            let ok = match &m {
                Ty::Name(n) => match n.as_str() {
                    "any" | "unknown" | "void" | "self" => true,
                    "nil" => *kind == ValueKind::Nil,
                    // Lua converts numbers to strings
                    "string" => matches!(kind, ValueKind::Str(_) | ValueKind::Num),
                    "number" | "integer" => match kind {
                        ValueKind::Num => true,
                        ValueKind::Str(Some(s)) => numeric_str(s),
                        _ => false,
                    },
                    "boolean" => *kind == ValueKind::Bool,
                    "function" => *kind == ValueKind::Function,
                    "table" | "userdata" | "lightuserdata" | "thread" => *kind == ValueKind::Table,
                    other => {
                        if let Some(a) = self.alias(other) {
                            if !seen.insert(other.to_string()) {
                                false
                            } else {
                                let lit_ok = a.literals.iter().any(|l| match kind {
                                    ValueKind::Str(_) => l.is_str,
                                    ValueKind::Num => !l.is_str,
                                    _ => false,
                                });
                                lit_ok || self.accepts_ty(&parse_ty(&a.ty), kind, seen, depth + 1)
                            }
                        } else if self.has_class(other) || self.has_enum(other) {
                            *kind == ValueKind::Table
                        } else {
                            true // unknown name: cannot say
                        }
                    }
                },
                Ty::Str(_) => matches!(kind, ValueKind::Str(_)),
                Ty::Num(_) => {
                    *kind == ValueKind::Num
                        || matches!(kind, ValueKind::Str(Some(s)) if numeric_str(s))
                }
                Ty::Bool(_) => *kind == ValueKind::Bool,
                Ty::Array(_) | Ty::Table(_) => *kind == ValueKind::Table,
                Ty::Fun(_) => *kind == ValueKind::Function,
                Ty::Generic(n, _) => n != "table" || *kind == ValueKind::Table,
                Ty::Vararg(_) | Ty::Union(_) | Ty::Optional(_) => true,
            };
            if ok {
                return true;
            }
        }
        false
    }
}

// ---- sandbox ---------------------------------------------------------------------------------

fn sandbox(c: &mut Ctx) {
    let doc = c.doc;
    let mut hits: Vec<(Severity, Span, &'static str, String)> = Vec::new();
    walk_block_exprs(&doc.chunk.body, &mut |e| match &e.kind {
        ExprKind::Field { obj, name, .. } => {
            if let ExprKind::Name(lib) = &obj.kind {
                if doc
                    .analysis
                    .ref_at(obj.span.start)
                    .map_or(true, |r| r.local.is_some())
                {
                    return;
                }
                if let Some(msg) = stdlib::sandbox_removed(lib, &name.name) {
                    hits.push((
                        Severity::Warning,
                        e.span,
                        "lua-sandbox-removed",
                        msg.to_string(),
                    ));
                } else if let Some(msg) = stdlib::mp_unsafe(&format!("{lib}.{}", name.name)) {
                    hits.push((Severity::Warning, e.span, "lua-mp-unsafe", msg.to_string()));
                }
            }
        }
        ExprKind::Name(n) => {
            if doc
                .analysis
                .ref_at(e.span.start)
                .map_or(true, |r| r.local.is_some())
            {
                return;
            }
            if stdlib::UNOPENED_LIBS.contains(&n.as_str()) {
                hits.push((
                    Severity::Warning,
                    e.span,
                    "lua-sandbox-removed",
                    // src/lua_base.c
                    format!("the '{n}' library is not opened by KeeperFX"),
                ));
            } else if let Some(msg) = stdlib::mp_unsafe(n) {
                hits.push((Severity::Warning, e.span, "lua-mp-unsafe", msg.to_string()));
            }
        }
        _ => {}
    });
    for (s, sp, code, m) in hits {
        c.push(s, sp, code, m, None);
    }
}

// ---- hooks and saved state ---------------------------------------------------------------------

fn hook_checks(c: &mut Ctx) {
    let doc = c.doc;
    // names used as trigger callbacks (by identifier or string)
    let mut cb_names: BTreeSet<String> = BTreeSet::new();
    let mut anon_bodies: Vec<&FuncBody> = Vec::new();
    walk_block_exprs(&doc.chunk.body, &mut |e| {
        let (name, args): (String, &Args) = match &e.kind {
            ExprKind::Call { func, args } => match &func.kind {
                ExprKind::Name(n) => (n.clone(), args),
                ExprKind::Field { name, .. } => (name.name.clone(), args),
                _ => return,
            },
            ExprKind::Method { name, args, .. } => (name.name.clone(), args),
            _ => return,
        };
        if !hooks::is_trigger_registration(&name) {
            return;
        }
        for a in &args.items {
            match &a.kind {
                ExprKind::Name(n) => {
                    cb_names.insert(n.clone());
                }
                ExprKind::Str(s) => {
                    cb_names.insert(s.clone());
                }
                ExprKind::Function(f) => anon_bodies.push(f),
                _ => {}
            }
        }
    });
    let mut bodies: Vec<(&FuncBody, String)> = anon_bodies
        .into_iter()
        .map(|f| (f, "a trigger callback".to_string()))
        .collect();
    let mut unknown_hooks: Vec<(Span, String)> = Vec::new();
    walk_stmts(&doc.chunk.body, &mut |s| {
        if let StmtKind::Function { target, func } = &s.kind {
            if target.parts.len() == 1 && target.method.is_none() {
                let n = &target.parts[0].name;
                if hooks::is_hook_name(n) {
                    bodies.push((func, format!("the hook {n}")));
                } else if cb_names.contains(n) {
                    bodies.push((func, format!("the trigger callback {n}")));
                }
                let looks_hook = n.len() > 2
                    && n.starts_with("On")
                    && n.chars().nth(2).map_or(false, |ch| ch.is_ascii_uppercase());
                let at_top = doc
                    .analysis
                    .global_assigns
                    .iter()
                    .any(|g| g.span == target.parts[0].span && g.top_level);
                if looks_hook
                    && at_top
                    && !hooks::is_hook_name(n)
                    && !c.env.index.functions.contains_key(n.as_str())
                    && !c.env.index.classes.contains_key(n.as_str())
                {
                    unknown_hooks.push((target.parts[0].span, n.clone()));
                }
            }
        }
    });
    for (span, n) in unknown_hooks {
        let sug = suggest(&n, hooks::HOOKS.iter().map(|h| h.name));
        let m = match &sug {
            Some(s) => {
                format!("'{n}' is not a KeeperFX hook the engine calls (did you mean '{s}'?)")
            }
            None => format!("'{n}' is not a KeeperFX hook the engine calls"),
        };
        c.push(Severity::Info, span, "lua-unknown-hook", m, sug);
    }

    for (body, what) in bodies {
        let mut reported: BTreeSet<String> = BTreeSet::new();
        let mut found: Vec<(Span, String)> = Vec::new();
        let check_root = |root: &str,
                          rspan: Span,
                          found: &mut Vec<(Span, String)>,
                          reported: &mut BTreeSet<String>,
                          c: &Ctx| {
            if root == "Game" || root == "self" || root == "_ENV" {
                return;
            }
            let Some(r) = c.doc.analysis.ref_at(rspan.start) else {
                return;
            };
            match r.local {
                Some(l) => {
                    let lv = &c.doc.analysis.locals[l];
                    let declared_inside = body.body.span.start <= lv.decl.start
                        && lv.decl.end <= body.body.span.end
                        || body.params_span.start <= lv.decl.start
                            && lv.decl.end <= body.params_span.end;
                    if lv.fn_id == 0
                        && !declared_inside
                        && lv.kind == LocalKind::Local
                        && reported.insert(root.to_string())
                    {
                        found.push((
                            rspan,
                            format!(
                                "'{root}' is file-level state mutated in \
                            {what}; it is not saved with the game (keep it in the Game table)"
                            ),
                        ));
                    }
                }
                None => {
                    if c.env.index.knows_global(root) || stdlib::is_std_global(root) {
                        return;
                    }
                    if reported.insert(root.to_string()) {
                        found.push((
                            rspan,
                            format!(
                                "global '{root}' is mutated in {what}; it is \
                            not saved with the game (keep it in the Game table)"
                            ),
                        ));
                    }
                }
            }
        };
        walk_stmts(&body.body, &mut |s| {
            if let StmtKind::Assign { targets, .. } = &s.kind {
                for t in targets {
                    if let Some((root, rs)) = root_of(t) {
                        check_root(root, rs, &mut found, &mut reported, c);
                    }
                }
            }
            if let StmtKind::Call(Expr {
                kind: ExprKind::Call { func, args },
                ..
            }) = &s.kind
            {
                if let Some(p) = func.path() {
                    if matches!(
                        p.join(".").as_str(),
                        "table.insert" | "table.remove" | "table.sort"
                    ) {
                        if let Some((root, rs)) = args.items.first().and_then(root_of) {
                            check_root(root, rs, &mut found, &mut reported, c);
                        }
                    }
                }
            }
        });
        for (span, msg) in found {
            c.push(Severity::Warning, span, "lua-hook-state", msg, None);
        }
    }
}
