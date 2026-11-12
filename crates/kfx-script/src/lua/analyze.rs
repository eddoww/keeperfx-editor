//! Scope analysis over the syntax tree: locals, parameters, upvalue use, global writes and
//! function definitions. Also a few generic tree walkers used by the editor services.
use super::ast::*;
use crate::diag::Span;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    Local,
    Param,
    SelfParam,
    ForVar,
    LocalFunction,
}

/// What the initialiser of a local syntactically is (used for the "captures a table" lint).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitKind {
    None,
    Table,
    Function,
    Scalar,
    Other,
}

#[derive(Clone, Debug)]
pub struct LocalVar {
    pub name: String,
    pub kind: LocalKind,
    pub decl: Span,
    /// Region where the name is visible (declaration end to end of the enclosing block).
    pub scope: Span,
    /// Id of the function the local belongs to (0 = main chunk).
    pub fn_id: usize,
    /// Span of the initialiser expression, when the local is initialised from one.
    pub init: Option<Span>,
    pub init_kind: InitKind,
    /// Start of the declaring statement (for attached annotations).
    pub stmt_start: usize,
    /// Index of the name in a multi-name declaration.
    pub index: usize,
    /// For `ForVar`: index into the statement list order is unnecessary; the loop statement span.
    pub loop_span: Option<Span>,
    pub reassigned: bool,
}

#[derive(Clone, Debug)]
pub struct NameRef {
    pub name: String,
    pub span: Span,
    pub local: Option<usize>,
    pub write: bool,
    pub fn_id: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FuncKind {
    Main,
    Anonymous,
    /// `function name()` (global when no local of that name is in scope).
    Global,
    Local,
    /// `function a.b()` / `function a:b()`
    Member,
}

#[derive(Clone, Debug)]
pub struct FuncInfo {
    pub id: usize,
    pub parent: Option<usize>,
    pub kind: FuncKind,
    pub span: Span,
    /// `Creature:level_up`, `OnGameStart`, ...
    pub name: Option<String>,
    pub name_span: Option<Span>,
    pub params: Vec<Ident>,
    pub vararg: bool,
    /// Declared with `:` (implicit `self`).
    pub method: bool,
    /// `[Class]` for members: the first path component.
    pub class: Option<String>,
    /// Statement start for annotation lookup.
    pub stmt_start: Option<usize>,
    pub local_id: Option<usize>,
    pub body: Span,
}

#[derive(Clone, Debug)]
pub struct GlobalAssign {
    pub name: String,
    pub span: Span,
    pub fn_id: usize,
    /// True for plain assignments at the top level of the file (or its top-level `if`/`do`).
    pub top_level: bool,
    /// Created by `function name() end` rather than `name = value`.
    pub is_function: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Analysis {
    pub locals: Vec<LocalVar>,
    pub refs: Vec<NameRef>,
    pub funcs: Vec<FuncInfo>,
    pub global_assigns: Vec<GlobalAssign>,
    /// Local ids by name (declaration order), for fast resolution.
    pub by_name: HashMap<String, Vec<usize>>,
}

struct Walker {
    a: Analysis,
    scopes: Vec<HashMap<String, usize>>,
    fn_stack: Vec<usize>,
    /// nesting depth of statement blocks inside the current function (0 = body of the function)
    block_depth: Vec<usize>,
    cond_depth: Vec<usize>,
}

pub fn analyze(chunk: &Chunk, src_len: usize) -> Analysis {
    let mut w = Walker {
        a: Analysis::default(),
        scopes: vec![HashMap::new()],
        fn_stack: vec![0],
        block_depth: vec![0],
        cond_depth: vec![0],
    };
    w.a.funcs.push(FuncInfo {
        id: 0,
        parent: None,
        kind: FuncKind::Main,
        span: Span::new(0, src_len),
        name: None,
        name_span: None,
        params: vec![],
        vararg: true,
        method: false,
        class: None,
        stmt_start: None,
        local_id: None,
        body: Span::new(0, src_len),
    });
    w.block_inline(&chunk.body);
    w.a.refs.sort_by_key(|r| r.span.start);
    for (i, l) in w.a.locals.iter().enumerate() {
        if !l.name.is_empty() {
            w.a.by_name.entry(l.name.clone()).or_default().push(i);
        }
    }
    w.a
}

fn classify(e: &Expr) -> InitKind {
    match &e.kind {
        ExprKind::Table(_) => InitKind::Table,
        ExprKind::Function(_) => InitKind::Function,
        ExprKind::Number(_)
        | ExprKind::Str(_)
        | ExprKind::True
        | ExprKind::False
        | ExprKind::Nil => InitKind::Scalar,
        ExprKind::Paren(i) => classify(i),
        _ => InitKind::Other,
    }
}

impl Walker {
    fn cur_fn(&self) -> usize {
        *self.fn_stack.last().unwrap()
    }
    fn lookup(&self, name: &str) -> Option<usize> {
        for s in self.scopes.iter().rev() {
            if let Some(&id) = s.get(name) {
                return Some(id);
            }
        }
        None
    }
    #[allow(clippy::too_many_arguments)]
    fn declare(
        &mut self,
        name: &Ident,
        kind: LocalKind,
        scope: Span,
        init: Option<&Expr>,
        stmt_start: usize,
        index: usize,
        loop_span: Option<Span>,
    ) -> usize {
        let id = self.a.locals.len();
        self.a.locals.push(LocalVar {
            name: name.name.clone(),
            kind,
            decl: name.span,
            scope,
            fn_id: self.cur_fn(),
            init: init.map(|e| e.span),
            init_kind: init.map_or(InitKind::None, classify),
            stmt_start,
            index,
            loop_span,
            reassigned: false,
        });
        if !name.name.is_empty() {
            self.scopes
                .last_mut()
                .unwrap()
                .insert(name.name.clone(), id);
        }
        id
    }
    fn name_ref(&mut self, name: &str, span: Span, write: bool) {
        let local = self.lookup(name);
        if write {
            if let Some(l) = local {
                self.a.locals[l].reassigned = true;
            }
        }
        let fn_id = self.cur_fn();
        self.a.refs.push(NameRef {
            name: name.to_string(),
            span,
            local,
            write,
            fn_id,
        });
    }

    fn block(&mut self, b: &Block) {
        self.scopes.push(HashMap::new());
        *self.block_depth.last_mut().unwrap() += 1;
        self.block_inline(b);
        *self.block_depth.last_mut().unwrap() -= 1;
        self.scopes.pop();
    }
    fn block_inline(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s, b.span.end);
        }
    }

    fn stmt(&mut self, s: &Stmt, block_end: usize) {
        match &s.kind {
            StmtKind::Local { names, values } => {
                for v in values {
                    self.expr(v);
                }
                let scope = Span::new(s.span.end, block_end.max(s.span.end));
                for (i, n) in names.iter().enumerate() {
                    let init = values.get(i);
                    let id = self.declare(n, LocalKind::Local, scope, init, s.span.start, i, None);
                    let _ = id;
                }
            }
            StmtKind::LocalFunction { name, func } => {
                let scope = Span::new(name.span.end, block_end.max(name.span.end));
                let id = self.declare(
                    name,
                    LocalKind::LocalFunction,
                    scope,
                    None,
                    s.span.start,
                    0,
                    None,
                );
                self.a.locals[id].init_kind = InitKind::Function;
                self.func(
                    func,
                    FuncKind::Local,
                    Some(name.name.clone()),
                    Some(name.span),
                    false,
                    None,
                    Some(s.span.start),
                    Some(id),
                );
            }
            StmtKind::Function { target, func } => {
                let first = &target.parts[0];
                let simple = target.parts.len() == 1 && target.method.is_none();
                let mut local_id = None;
                if !first.name.is_empty() {
                    local_id = self.lookup(&first.name);
                    self.name_ref(&first.name, first.span, simple);
                    if simple && local_id.is_none() {
                        let top = self.cur_fn() == 0 && self.cond_depth[0] == 0;
                        self.a.global_assigns.push(GlobalAssign {
                            name: first.name.clone(),
                            span: first.span,
                            fn_id: self.cur_fn(),
                            top_level: top,
                            is_function: true,
                        });
                    }
                }
                let kind = if simple {
                    if local_id.is_some() {
                        FuncKind::Local
                    } else {
                        FuncKind::Global
                    }
                } else {
                    FuncKind::Member
                };
                let class = if !simple {
                    Some(first.name.clone())
                } else {
                    None
                };
                self.func(
                    func,
                    kind,
                    Some(target.display()),
                    Some(target.span),
                    target.method.is_some(),
                    class,
                    Some(s.span.start),
                    local_id,
                );
            }
            StmtKind::Assign { targets, values } => {
                for v in values {
                    self.expr(v);
                }
                for t in targets {
                    match &t.kind {
                        ExprKind::Name(n) => {
                            let local = self.lookup(n);
                            self.name_ref(n, t.span, true);
                            if local.is_none() {
                                let top = self.cur_fn() == 0 && self.cond_depth[0] == 0;
                                self.a.global_assigns.push(GlobalAssign {
                                    name: n.clone(),
                                    span: t.span,
                                    fn_id: self.cur_fn(),
                                    top_level: top,
                                    is_function: false,
                                });
                            }
                        }
                        _ => self.expr(t),
                    }
                }
            }
            StmtKind::Call(e) => self.expr(e),
            StmtKind::Do(b) => self.block(b),
            StmtKind::While { cond, body } => {
                self.expr(cond);
                self.cond_depth[self.fn_stack.len() - 1] += 1;
                self.block(body);
                self.cond_depth[self.fn_stack.len() - 1] -= 1;
            }
            StmtKind::Repeat { body, cond } => {
                self.scopes.push(HashMap::new());
                self.cond_depth[self.fn_stack.len() - 1] += 1;
                self.block_inline(body);
                self.cond_depth[self.fn_stack.len() - 1] -= 1;
                self.expr(cond);
                self.scopes.pop();
            }
            StmtKind::If { arms, else_block } => {
                for (c, b) in arms {
                    self.expr(c);
                    self.block(b);
                }
                if let Some(b) = else_block {
                    self.block(b);
                }
            }
            StmtKind::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => {
                self.expr(start);
                self.expr(limit);
                if let Some(e) = step {
                    self.expr(e);
                }
                self.scopes.push(HashMap::new());
                self.declare(
                    var,
                    LocalKind::ForVar,
                    body.span,
                    None,
                    s.span.start,
                    0,
                    Some(s.span),
                );
                self.cond_depth[self.fn_stack.len() - 1] += 1;
                self.block(body);
                self.cond_depth[self.fn_stack.len() - 1] -= 1;
                self.scopes.pop();
            }
            StmtKind::GenericFor { vars, iters, body } => {
                for e in iters {
                    self.expr(e);
                }
                self.scopes.push(HashMap::new());
                for (i, v) in vars.iter().enumerate() {
                    self.declare(
                        v,
                        LocalKind::ForVar,
                        body.span,
                        None,
                        s.span.start,
                        i,
                        Some(s.span),
                    );
                }
                self.cond_depth[self.fn_stack.len() - 1] += 1;
                self.block(body);
                self.cond_depth[self.fn_stack.len() - 1] -= 1;
                self.scopes.pop();
            }
            StmtKind::Return(v) => {
                for e in v {
                    self.expr(e);
                }
            }
            StmtKind::Break | StmtKind::Goto(_) | StmtKind::Label(_) | StmtKind::Error => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn func(
        &mut self,
        f: &FuncBody,
        kind: FuncKind,
        name: Option<String>,
        name_span: Option<Span>,
        method: bool,
        class: Option<String>,
        stmt_start: Option<usize>,
        local_id: Option<usize>,
    ) {
        let id = self.a.funcs.len();
        let parent = Some(self.cur_fn());
        self.a.funcs.push(FuncInfo {
            id,
            parent,
            kind,
            span: f.span,
            name,
            name_span,
            params: f.params.clone(),
            vararg: f.vararg.is_some(),
            method,
            class,
            stmt_start,
            local_id,
            body: f.body.span,
        });
        self.fn_stack.push(id);
        self.cond_depth.push(0);
        self.block_depth.push(0);
        self.scopes.push(HashMap::new());
        let scope = Span::new(
            f.params_span.end.min(f.body.span.start),
            f.body.span.end.max(f.params_span.end),
        );
        if method {
            let selfid = Ident {
                name: "self".into(),
                span: Span::new(f.params_span.start, f.params_span.start),
            };
            self.declare(
                &selfid,
                LocalKind::SelfParam,
                scope,
                None,
                stmt_start.unwrap_or(f.span.start),
                0,
                None,
            );
        }
        for (i, p) in f.params.iter().enumerate() {
            self.declare(
                p,
                LocalKind::Param,
                scope,
                None,
                stmt_start.unwrap_or(f.span.start),
                i,
                None,
            );
        }
        self.block_inline(&f.body);
        self.scopes.pop();
        self.block_depth.pop();
        self.cond_depth.pop();
        self.fn_stack.pop();
    }

    fn args(&mut self, a: &Args) {
        for e in &a.items {
            self.expr(e);
        }
    }

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Name(n) => self.name_ref(n, e.span, false),
            ExprKind::Field { obj, .. } => self.expr(obj),
            ExprKind::Index { obj, key } => {
                self.expr(obj);
                self.expr(key);
            }
            ExprKind::Call { func, args } => {
                self.expr(func);
                self.args(args);
            }
            ExprKind::Method { obj, args, .. } => {
                self.expr(obj);
                self.args(args);
            }
            ExprKind::Function(f) => {
                self.func(f, FuncKind::Anonymous, None, None, false, None, None, None)
            }
            ExprKind::Table(t) => {
                for it in &t.items {
                    match it {
                        TableItem::Pos(v) => self.expr(v),
                        TableItem::Named { value, .. } => self.expr(value),
                        TableItem::Keyed { key, value } => {
                            self.expr(key);
                            self.expr(value);
                        }
                    }
                }
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Paren(i) => self.expr(i),
            ExprKind::Nil
            | ExprKind::True
            | ExprKind::False
            | ExprKind::Number(_)
            | ExprKind::Str(_)
            | ExprKind::Vararg
            | ExprKind::Error => {}
        }
    }
}

impl Analysis {
    /// The reference whose span contains `off` (touching the end counts).
    pub fn ref_at(&self, off: usize) -> Option<&NameRef> {
        let i = self.refs.partition_point(|r| r.span.start <= off);
        self.refs[..i]
            .iter()
            .rev()
            .take(4)
            .find(|r| r.span.touches(off))
    }
    /// Locals visible at `off`, innermost declaration last.
    pub fn visible_locals(&self, off: usize) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.locals.len())
            .filter(|&i| {
                let l = &self.locals[i];
                !l.name.is_empty()
                    && l.scope.start <= off
                    && off <= l.scope.end
                    && !(l.kind == LocalKind::SelfParam && l.scope.start > off)
            })
            .collect();
        v.sort_by_key(|&i| self.locals[i].decl.start);
        v
    }
    /// Resolve `name` at `off` to the innermost visible local.
    pub fn resolve(&self, name: &str, off: usize) -> Option<usize> {
        self.by_name
            .get(name)?
            .iter()
            .copied()
            .filter(|&i| {
                let l = &self.locals[i];
                l.scope.start <= off && off <= l.scope.end
            })
            .max_by_key(|&i| self.locals[i].decl.start)
    }
    /// Id of the innermost function containing `off`.
    pub fn function_at(&self, off: usize) -> usize {
        let mut best = 0;
        let mut best_len = usize::MAX;
        for f in &self.funcs {
            if f.span.start <= off && off <= f.span.end && f.span.len() <= best_len && f.id != 0 {
                best = f.id;
                best_len = f.span.len();
            }
        }
        best
    }
    /// Locals declared outside `func` that are used inside it (its upvalues).
    pub fn captures(&self, func: &FuncInfo) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        for r in &self.refs {
            if let Some(l) = r.local {
                let d = self.locals[l].decl;
                if func.span.contains(r.span.start)
                    && !(func.span.start <= d.start && d.end <= func.span.end)
                    && !out.contains(&l)
                {
                    out.push(l);
                }
            }
        }
        out
    }
    pub fn func_by_span(&self, span: Span) -> Option<&FuncInfo> {
        self.funcs.iter().find(|f| f.id != 0 && f.span == span)
    }
}

// ---- generic walkers -----------------------------------------------------------------------

/// Visit every expression of a block (including nested function bodies), parents before children.
pub fn walk_block_exprs<'a>(b: &'a Block, f: &mut dyn FnMut(&'a Expr)) {
    for s in &b.stmts {
        walk_stmt_exprs(s, f);
    }
}

pub fn walk_stmt_exprs<'a>(s: &'a Stmt, f: &mut dyn FnMut(&'a Expr)) {
    match &s.kind {
        StmtKind::Local { values, .. } => values.iter().for_each(|e| walk_expr(e, f)),
        StmtKind::LocalFunction { func, .. } | StmtKind::Function { func, .. } => {
            walk_block_exprs(&func.body, f)
        }
        StmtKind::Assign { targets, values } => {
            targets.iter().for_each(|e| walk_expr(e, f));
            values.iter().for_each(|e| walk_expr(e, f));
        }
        StmtKind::Call(e) => walk_expr(e, f),
        StmtKind::Do(b) => walk_block_exprs(b, f),
        StmtKind::While { cond, body } => {
            walk_expr(cond, f);
            walk_block_exprs(body, f);
        }
        StmtKind::Repeat { body, cond } => {
            walk_block_exprs(body, f);
            walk_expr(cond, f);
        }
        StmtKind::If { arms, else_block } => {
            for (c, b) in arms {
                walk_expr(c, f);
                walk_block_exprs(b, f);
            }
            if let Some(b) = else_block {
                walk_block_exprs(b, f);
            }
        }
        StmtKind::NumericFor {
            start,
            limit,
            step,
            body,
            ..
        } => {
            walk_expr(start, f);
            walk_expr(limit, f);
            if let Some(e) = step {
                walk_expr(e, f);
            }
            walk_block_exprs(body, f);
        }
        StmtKind::GenericFor { iters, body, .. } => {
            iters.iter().for_each(|e| walk_expr(e, f));
            walk_block_exprs(body, f);
        }
        StmtKind::Return(v) => v.iter().for_each(|e| walk_expr(e, f)),
        StmtKind::Break | StmtKind::Goto(_) | StmtKind::Label(_) | StmtKind::Error => {}
    }
}

pub fn walk_expr<'a>(e: &'a Expr, f: &mut dyn FnMut(&'a Expr)) {
    f(e);
    match &e.kind {
        ExprKind::Field { obj, .. } => walk_expr(obj, f),
        ExprKind::Index { obj, key } => {
            walk_expr(obj, f);
            walk_expr(key, f);
        }
        ExprKind::Call { func, args } => {
            walk_expr(func, f);
            args.items.iter().for_each(|a| walk_expr(a, f));
        }
        ExprKind::Method { obj, args, .. } => {
            walk_expr(obj, f);
            args.items.iter().for_each(|a| walk_expr(a, f));
        }
        ExprKind::Function(func) => walk_block_exprs(&func.body, f),
        ExprKind::Table(t) => {
            for it in &t.items {
                match it {
                    TableItem::Pos(v) => walk_expr(v, f),
                    TableItem::Named { value, .. } => walk_expr(value, f),
                    TableItem::Keyed { key, value } => {
                        walk_expr(key, f);
                        walk_expr(value, f);
                    }
                }
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, f);
            walk_expr(rhs, f);
        }
        ExprKind::Unary { operand, .. } => walk_expr(operand, f),
        ExprKind::Paren(i) => walk_expr(i, f),
        _ => {}
    }
}

/// Visit every statement (recursively, including nested function bodies), parents first.
pub fn walk_stmts<'a>(b: &'a Block, f: &mut dyn FnMut(&'a Stmt)) {
    for s in &b.stmts {
        f(s);
        match &s.kind {
            StmtKind::LocalFunction { func, .. } | StmtKind::Function { func, .. } => {
                walk_stmts(&func.body, f)
            }
            StmtKind::Do(b) => walk_stmts(b, f),
            StmtKind::While { body, .. }
            | StmtKind::Repeat { body, .. }
            | StmtKind::NumericFor { body, .. }
            | StmtKind::GenericFor { body, .. } => walk_stmts(body, f),
            StmtKind::If { arms, else_block } => {
                for (_, b) in arms {
                    walk_stmts(b, f);
                }
                if let Some(b) = else_block {
                    walk_stmts(b, f);
                }
            }
            _ => {}
        }
    }
}

/// Statements at file level, looking through top-level `do` and `if` blocks (not loops/functions).
pub fn top_level_stmts<'a>(b: &'a Block, out: &mut Vec<&'a Stmt>) {
    for s in &b.stmts {
        match &s.kind {
            StmtKind::Do(inner) => top_level_stmts(inner, out),
            StmtKind::If { arms, else_block } => {
                out.push(s);
                for (_, blk) in arms {
                    top_level_stmts(blk, out);
                }
                if let Some(blk) = else_block {
                    top_level_stmts(blk, out);
                }
            }
            _ => out.push(s),
        }
    }
}
