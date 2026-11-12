//! Name resolution and simple type inference over a file plus the annotation index.
//!
//! [`Env`] looks every definition up in the current file first, then in the library index.
//! Inference is deliberately shallow: literals, annotated globals/params/returns, `---@type`,
//! `ipairs`/`pairs` over typed arrays and field/method chains. Anything unclear is `None`.
use super::analyze::{walk_block_exprs, walk_stmts, LocalKind};
use super::annot::{parse_ty, Ty};
use super::ast::*;
use super::doc::LuaDoc;
use super::index::{
    AliasDef, AliasLit, AliasValues, ApiIndex, ClassDef, FieldDef, FuncDef, GlobalDef, Member,
};
use super::stdlib;
use crate::diag::Span;
use std::collections::BTreeSet;

pub struct Env<'a> {
    pub doc: &'a LuaDoc,
    pub index: &'a ApiIndex,
}

/// Where a call's arguments land in the parameter list.
pub struct CallSig<'a> {
    pub def: &'a FuncDef,
    /// Parameter index of argument `i` is `i + shift` (negative means the implicit/explicit self).
    pub shift: i32,
    /// Display name, e.g. `Creature:level_up`.
    pub display: String,
}

pub enum MemberKind {
    Field,
    Method,
}

pub struct MemberInfo {
    pub name: String,
    pub kind: MemberKind,
    pub ty: String,
    pub doc: String,
    pub owner: String,
    pub deprecated: bool,
    /// For methods and function fields: `Some(has_parameters)` taken from the declaration.
    pub callable: Option<bool>,
}

const MAX_DEPTH: u32 = 8;

impl<'a> Env<'a> {
    pub fn new(doc: &'a LuaDoc, index: &'a ApiIndex) -> Env<'a> {
        Env { doc, index }
    }

    // ---- definition lookup ----------------------------------------------------------------

    pub fn function(&self, name: &str) -> Option<&'a FuncDef> {
        self.doc
            .defs
            .functions
            .get(name)
            .or_else(|| self.index.functions.get(name))
    }
    pub fn global(&self, name: &str) -> Option<&'a GlobalDef> {
        self.doc
            .defs
            .globals
            .get(name)
            .or_else(|| self.index.globals.get(name))
    }
    pub fn alias(&self, name: &str) -> Option<&'a AliasDef> {
        self.doc
            .defs
            .aliases
            .get(name)
            .or_else(|| self.index.aliases.get(name))
    }
    pub fn has_class(&self, name: &str) -> bool {
        self.doc.defs.classes.contains_key(name) || self.index.classes.contains_key(name)
    }
    pub fn has_enum(&self, name: &str) -> bool {
        self.doc.defs.enums.contains_key(name) || self.index.enums.contains_key(name)
    }

    /// Class and ancestors, nearest first; a class defined in both places yields two entries.
    pub fn class_chain(&self, name: &str) -> Vec<&'a ClassDef> {
        let mut out = Vec::new();
        let mut queue = vec![name.to_string()];
        let mut seen = BTreeSet::new();
        while !queue.is_empty() {
            let n = queue.remove(0);
            if !seen.insert(n.clone()) {
                continue;
            }
            for src in [&self.doc.defs, self.index] {
                if let Some(c) = src.classes.get(&n) {
                    out.push(c);
                    queue.extend(c.parents.iter().cloned());
                }
            }
        }
        out
    }

    pub fn find_member(&self, class: &str, member: &str) -> Option<Member<'a>> {
        for c in self.class_chain(class) {
            if let Some(f) = c.fields.get(member) {
                return Some(Member::Field(f, c));
            }
            if let Some(m) = c.methods.get(member) {
                return Some(Member::Method(m, c));
            }
        }
        None
    }

    /// True when a name is defined as a global in the file or the library.
    pub fn knows_global(&self, name: &str) -> bool {
        self.doc.defs.knows_global(name)
            || self.index.knows_global(name)
            || stdlib::is_std_global(name)
    }

    pub fn alias_values(&self, name: &str) -> Option<AliasValues> {
        self.alias(name)?;
        let mut out = AliasValues::default();
        let mut seen = BTreeSet::new();
        self.collect_alias(name, &mut out, &mut seen);
        Some(out)
    }

    pub fn type_values(&self, ty: &Ty) -> AliasValues {
        let mut out = AliasValues::default();
        let mut seen = BTreeSet::new();
        self.collect_ty(ty, &mut out, &mut seen);
        out
    }

    fn collect_alias(&self, name: &str, out: &mut AliasValues, seen: &mut BTreeSet<String>) {
        if !seen.insert(name.to_string()) {
            return;
        }
        let Some(a) = self.alias(name) else { return };
        out.literals.extend(a.literals.iter().cloned());
        self.collect_ty(&parse_ty(&a.ty), out, seen);
    }

    fn collect_ty(&self, ty: &Ty, out: &mut AliasValues, seen: &mut BTreeSet<String>) {
        const BUILTIN: &[&str] = &[
            "nil",
            "any",
            "boolean",
            "string",
            "number",
            "integer",
            "function",
            "table",
            "thread",
            "userdata",
            "lightuserdata",
            "unknown",
            "void",
            "self",
            "fun",
        ];
        for m in ty.members() {
            match m {
                Ty::Str(s) => {
                    if !out.literals.iter().any(|l| l.is_str && l.value == s) {
                        out.literals.push(AliasLit {
                            value: s,
                            is_str: true,
                            doc: String::new(),
                        });
                    }
                }
                Ty::Num(n) => out.literals.push(AliasLit {
                    value: n,
                    is_str: false,
                    doc: String::new(),
                }),
                Ty::Name(n) => {
                    if n == "string" || n == "any" || n == "unknown" {
                        out.open = true;
                        out.other.push(n);
                    } else if self.alias(&n).is_some() {
                        self.collect_alias(&n, out, seen);
                    } else if BUILTIN.contains(&n.as_str())
                        || self.has_class(&n)
                        || self.has_enum(&n)
                    {
                        out.other.push(n);
                    } else {
                        out.open = true;
                        out.other.push(n);
                    }
                }
                Ty::Generic(n, _) => out.other.push(n),
                Ty::Union(_) | Ty::Optional(_) => {}
                other => out.other.push(other.to_string()),
            }
        }
    }

    // ---- inference ------------------------------------------------------------------------

    pub fn infer(&self, e: &Expr, _at: usize) -> Option<Ty> {
        self.infer_d(e, 0)
    }

    fn name_type(&self, n: &str, at: usize, depth: u32) -> Option<Ty> {
        if let Some(l) = self.doc.analysis.resolve(n, at) {
            return self.local_type_d(l, depth + 1);
        }
        if self.has_class(n) {
            return Some(Ty::Generic("typeof".into(), vec![Ty::Name(n.to_string())]));
        }
        if let Some(g) = self.global(n) {
            if g.ty != "any" {
                return Some(parse_ty(&g.ty));
            }
        }
        if self.function(n).is_some() {
            return Some(Ty::Name("function".into()));
        }
        if stdlib::lib_members(n).is_some() {
            return Some(Ty::Generic(
                "typeof".into(),
                vec![Ty::Name(format!("lib:{n}"))],
            ));
        }
        None
    }

    fn infer_d(&self, e: &Expr, depth: u32) -> Option<Ty> {
        if depth > MAX_DEPTH {
            return None;
        }
        match &e.kind {
            ExprKind::Str(_) => Some(Ty::Name("string".into())),
            ExprKind::Number(n) => Some(Ty::Name(
                if n.contains(['.', 'e', 'E']) && !n.starts_with("0x") {
                    "number"
                } else {
                    "integer"
                }
                .into(),
            )),
            ExprKind::True | ExprKind::False => Some(Ty::Name("boolean".into())),
            ExprKind::Nil => Some(Ty::Name("nil".into())),
            ExprKind::Table(_) => Some(Ty::Name("table".into())),
            ExprKind::Function(_) => Some(Ty::Name("function".into())),
            ExprKind::Paren(i) => self.infer_d(i, depth + 1),
            ExprKind::Name(n) => self.name_type(n, e.span.start, depth),
            ExprKind::Field { obj, name, .. } => {
                let base = self.infer_d(obj, depth + 1)?;
                self.member_type(&base, &name.name, depth)
            }
            ExprKind::Index { obj, .. } => {
                let base = self.infer_d(obj, depth + 1)?;
                let mut out = Vec::new();
                for m in base.members() {
                    if let Ty::Array(t) = m {
                        out.push(*t);
                    } else if let Ty::Generic(n, a) = &m {
                        if n == "table" && a.len() == 2 {
                            out.push(a[1].clone());
                        }
                    }
                }
                union_of(out)
            }
            ExprKind::Call { func, args } => {
                if let ExprKind::Name(n) = &func.kind {
                    if n == "setmetatable" {
                        return args.items.first().and_then(|a| self.infer_d(a, depth + 1));
                    }
                }
                let sig = self.resolve_callee_d(func, false, depth + 1);
                match sig {
                    Some(s) => s.def.returns.first().map(|r| parse_ty(&r.ty)),
                    None => func
                        .path()
                        .and_then(|p| stdlib::return_type(&p.join(".")))
                        .map(parse_ty),
                }
            }
            ExprKind::Method { obj, name, .. } => {
                let base = self.infer_d(obj, depth + 1)?;
                let sig = self.method_sig(&base, &name.name, true)?;
                sig.def.returns.first().map(|r| parse_ty(&r.ty))
            }
            ExprKind::Binary { op, lhs, rhs, .. } => match *op {
                "or" => self
                    .infer_d(lhs, depth + 1)
                    .filter(|t| !matches!(t, Ty::Name(n) if n == "nil"))
                    .or_else(|| self.infer_d(rhs, depth + 1)),
                "and" => self.infer_d(rhs, depth + 1),
                "==" | "~=" | "<" | ">" | "<=" | ">=" => Some(Ty::Name("boolean".into())),
                ".." => Some(Ty::Name("string".into())),
                _ => Some(Ty::Name("number".into())),
            },
            ExprKind::Unary { op, .. } => Some(Ty::Name(
                if *op == "not" { "boolean" } else { "number" }.into(),
            )),
            ExprKind::Vararg | ExprKind::Error => None,
        }
    }

    /// Type of `base.name`.
    pub fn member_type(&self, base: &Ty, name: &str, _depth: u32) -> Option<Ty> {
        let mut out = Vec::new();
        for m in base.members() {
            match &m {
                Ty::Name(c) => {
                    if let Some(Member::Field(f, _)) = self.find_member(c, name) {
                        out.push(parse_ty(&f.ty));
                    } else if let Some(Member::Method(..)) = self.find_member(c, name) {
                        out.push(Ty::Name("function".into()));
                    }
                }
                Ty::Generic(t, a) if t == "typeof" => {
                    if let Some(Ty::Name(c)) = a.first() {
                        if let Some(lib) = c.strip_prefix("lib:") {
                            if stdlib::lib_members(lib).map_or(false, |m| m.contains(&name)) {
                                out.push(Ty::Name("function".into()));
                            }
                        } else {
                            match self.find_member(c, name) {
                                Some(Member::Field(f, _)) => out.push(parse_ty(&f.ty)),
                                Some(Member::Method(..)) => out.push(Ty::Name("function".into())),
                                None => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        union_of(out)
    }

    /// Signature of calling `name` on a value of type `base` (`colon` = `base:name(...)`).
    pub fn method_sig(&self, base: &Ty, name: &str, colon: bool) -> Option<CallSig<'a>> {
        for m in base.members() {
            let (class, is_static) = match &m {
                Ty::Name(c) => (c.clone(), false),
                Ty::Generic(t, a) if t == "typeof" => match a.first() {
                    Some(Ty::Name(c)) if !c.starts_with("lib:") => (c.clone(), true),
                    _ => continue,
                },
                _ => continue,
            };
            let _ = is_static;
            if let Some(Member::Method(def, owner)) = self.find_member(&class, name) {
                let shift = match (colon, def.method) {
                    (true, true) | (false, false) => 0,
                    (true, false) => 1,
                    (false, true) => -1,
                };
                let sep = if def.method { ':' } else { '.' };
                return Some(CallSig {
                    def,
                    shift,
                    display: format!("{}{}{}", owner.name, sep, name),
                });
            }
        }
        None
    }

    /// Resolve the callee of a call expression to an indexed signature.
    pub fn resolve_callee(&self, callee: &Expr) -> Option<CallSig<'a>> {
        self.resolve_callee_d(callee, false, 0)
    }

    pub fn resolve_callee_d(&self, callee: &Expr, colon: bool, depth: u32) -> Option<CallSig<'a>> {
        if depth > MAX_DEPTH {
            return None;
        }
        match &callee.kind {
            ExprKind::Name(n) => {
                if let Some(l) = self.doc.analysis.resolve(n, callee.span.start) {
                    return self.doc.local_funcs.get(&l).map(|def| CallSig {
                        def,
                        shift: 0,
                        display: n.clone(),
                    });
                }
                self.function(n).map(|def| CallSig {
                    def,
                    shift: 0,
                    display: n.clone(),
                })
            }
            ExprKind::Field { obj, name, .. } => {
                let base = self.infer_d(obj, depth + 1)?;
                self.method_sig(&base, &name.name, colon)
            }
            ExprKind::Paren(i) => self.resolve_callee_d(i, colon, depth + 1),
            _ => None,
        }
    }

    pub fn resolve_method_call(&self, obj: &Expr, name: &str) -> Option<CallSig<'a>> {
        let base = self.infer_d(obj, 0)?;
        self.method_sig(&base, name, true)
    }

    // ---- locals ---------------------------------------------------------------------------

    pub fn local_type(&self, id: usize) -> Option<Ty> {
        self.local_type_d(id, 0)
    }

    fn local_type_d(&self, id: usize, depth: u32) -> Option<Ty> {
        if depth > MAX_DEPTH {
            return None;
        }
        let a = &self.doc.analysis;
        let l = a.locals.get(id)?;
        match l.kind {
            LocalKind::SelfParam => {
                let f = a
                    .funcs
                    .iter()
                    .filter(|f| {
                        f.method && f.span.start <= l.scope.start && l.scope.start <= f.span.end
                    })
                    .min_by_key(|f| f.span.len())?;
                f.class.as_ref().map(|c| Ty::Name(c.clone()))
            }
            LocalKind::Param => {
                let f = a
                    .funcs
                    .iter()
                    .find(|f| f.params.iter().any(|p| p.span == l.decl))?;
                // explicit annotation
                if let Some(sig) = f.stmt_start.and_then(|s| self.doc.block_for(s)) {
                    for (t, _) in &sig.tags {
                        if let super::annot::Tag::Param { name, ty, .. } = t {
                            if *name == l.name {
                                return Some(parse_ty(ty));
                            }
                        }
                    }
                }
                // a hook documented in the library: take the parameter type by position
                let fname = f.name.as_deref()?;
                if f.method {
                    return None;
                }
                let pos = f.params.iter().position(|p| p.span == l.decl)?;
                // the library's documentation of the hook (not this file's own unannotated
                // definition)
                if let Some(p) = self
                    .index
                    .functions
                    .get(fname)
                    .and_then(|d| d.params.get(pos))
                    .filter(|p| p.annotated)
                {
                    return Some(parse_ty(&p.ty));
                }
                // the engine's own parameter list (src/lua_triggers.c)
                super::hooks::hook(fname)
                    .and_then(|h| h.params.get(pos))
                    .map(|(_, t)| parse_ty(t))
            }
            LocalKind::LocalFunction => Some(Ty::Name("function".into())),
            LocalKind::Local => {
                if let Some(t) = self.doc.declared_type(l.stmt_start, l.scope.start, l.index) {
                    return Some(parse_ty(&t));
                }
                let init = l.init?;
                let e = find_expr(&self.doc.chunk.body, init)?;
                self.infer_d(e, depth + 1)
            }
            LocalKind::ForVar => {
                let span = l.loop_span?;
                let mut found: Option<&Stmt> = None;
                walk_stmts(&self.doc.chunk.body, &mut |s| {
                    if s.span == span {
                        found = Some(s);
                    }
                });
                let stmt = found?;
                let iters = match &stmt.kind {
                    StmtKind::GenericFor { iters, .. } => iters,
                    StmtKind::NumericFor { .. } => return Some(Ty::Name("number".into())),
                    _ => return None,
                };
                let Expr {
                    kind: ExprKind::Call { func, args },
                    ..
                } = iters.first()?
                else {
                    return None;
                };
                let fname = match &func.kind {
                    ExprKind::Name(n) => n.as_str(),
                    _ => return None,
                };
                if fname != "ipairs" && fname != "pairs" {
                    return None;
                }
                let base = self.infer_d(args.items.first()?, depth + 1)?;
                let mut elems = Vec::new();
                for m in base.members() {
                    if let Ty::Array(t) = m {
                        elems.push(*t);
                    }
                }
                match l.index {
                    0 if fname == "ipairs" => Some(Ty::Name("integer".into())),
                    1 => union_of(elems),
                    _ => None,
                }
            }
        }
    }

    // ---- members for completion / hover -----------------------------------------------------

    /// All members visible on a value of type `ty` (fields and methods, nearest class first).
    pub fn members_of(&self, ty: &Ty, colon: bool) -> Vec<MemberInfo> {
        let mut out: Vec<MemberInfo> = Vec::new();
        let mut seen = BTreeSet::new();
        for m in ty.members() {
            match &m {
                Ty::Name(c) if c == "string" => {
                    for n in stdlib::lib_members("string").unwrap_or(&[]) {
                        if seen.insert(n.to_string()) {
                            out.push(MemberInfo {
                                name: n.to_string(),
                                kind: MemberKind::Method,
                                ty: stdlib::signature(&format!("string.{n}"))
                                    .unwrap_or("function")
                                    .to_string(),
                                doc: String::new(),
                                owner: "string".into(),
                                deprecated: false,
                                callable: Some(sig_has_params(stdlib::signature(&format!(
                                    "string.{n}"
                                )))),
                            });
                        }
                    }
                }
                Ty::Name(c) => self.push_class_members(c, colon, false, &mut out, &mut seen),
                Ty::Generic(t, a) if t == "typeof" => {
                    if let Some(Ty::Name(c)) = a.first() {
                        if let Some(lib) = c.strip_prefix("lib:") {
                            for n in stdlib::lib_members(lib).unwrap_or(&[]) {
                                if seen.insert(n.to_string()) {
                                    let sig = stdlib::signature(&format!("{lib}.{n}"));
                                    out.push(MemberInfo {
                                        name: n.to_string(),
                                        kind: if sig.is_some()
                                            || !matches!(
                                                *n,
                                                "pi" | "huge"
                                                    | "path"
                                                    | "cpath"
                                                    | "loaded"
                                                    | "preload"
                                                    | "config"
                                            ) {
                                            MemberKind::Method
                                        } else {
                                            MemberKind::Field
                                        },
                                        ty: sig.unwrap_or("").to_string(),
                                        doc: String::new(),
                                        owner: lib.to_string(),
                                        deprecated: false,
                                        callable: sig.map(|_| sig_has_params(sig)),
                                    });
                                }
                            }
                        } else {
                            self.push_class_members(c, colon, true, &mut out, &mut seen);
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn push_class_members(
        &self,
        class: &str,
        colon: bool,
        is_static: bool,
        out: &mut Vec<MemberInfo>,
        seen: &mut BTreeSet<String>,
    ) {
        for c in self.class_chain(class) {
            for (n, f) in &c.fields {
                if !is_static || true {
                    self.push_field(n, f, c, colon, out, seen);
                }
            }
            for (n, m) in &c.methods {
                // metamethods are not useful completions
                if n.starts_with("__") {
                    continue;
                }
                if colon && !m.method && !is_static {
                    // `obj:fn()` passes obj as the first argument: allowed but unusual; still list
                    // it
                }
                if seen.insert(n.clone()) {
                    out.push(MemberInfo {
                        name: n.clone(),
                        kind: MemberKind::Method,
                        ty: m.signature(),
                        doc: m.doc.clone(),
                        owner: c.name.clone(),
                        deprecated: m.deprecated.is_some(),
                        callable: Some(!m.params.is_empty() || m.vararg.is_some()),
                    });
                }
            }
        }
    }

    fn push_field(
        &self,
        n: &str,
        f: &FieldDef,
        c: &ClassDef,
        colon: bool,
        out: &mut Vec<MemberInfo>,
        seen: &mut BTreeSet<String>,
    ) {
        if colon {
            // fields are not callable with `:` but may hold functions
            if !f.ty.starts_with("fun(") && f.ty != "function" {
                return;
            }
        }
        if seen.insert(n.to_string()) {
            out.push(MemberInfo {
                name: n.to_string(),
                kind: MemberKind::Field,
                ty: f.ty.clone(),
                doc: f.doc.clone(),
                owner: c.name.clone(),
                deprecated: false,
                callable: if f.ty.starts_with("fun(") {
                    Some(!f.ty.starts_with("fun()"))
                } else if f.ty == "function" {
                    Some(true)
                } else {
                    None
                },
            });
        }
    }
}

/// Whether a signature text like `print(...)` has anything between the parentheses.
pub fn sig_has_params(sig: Option<&str>) -> bool {
    sig.and_then(|s| s.find('(').map(|i| !s[i + 1..].starts_with(')')))
        .unwrap_or(true)
}

fn union_of(mut v: Vec<Ty>) -> Option<Ty> {
    v.dedup();
    match v.len() {
        0 => None,
        1 => v.pop(),
        _ => Some(Ty::Union(v)),
    }
}

/// Find the expression with exactly this span.
pub fn find_expr(b: &Block, span: Span) -> Option<&Expr> {
    let mut found = None;
    walk_block_exprs(b, &mut |e| {
        if found.is_none() && e.span == span {
            found = Some(e);
        }
    });
    found
}
