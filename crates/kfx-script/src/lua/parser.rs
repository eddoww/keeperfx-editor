//! Tolerant recursive-descent parser for LuaJIT (Lua 5.1 + `goto`).
//!
//! Never panics and never stops at the first error: unexpected tokens are reported and skipped,
//! missing `end`/`)`/`}` are reported at the construct that was left open, and the rest of the file
//! is still parsed. Nesting is limited to 200 levels like LuaJIT (`LJ_MAX_XLEVEL`).
use super::ast::*;
use super::lexer::{TokKind, Token};
use crate::diag::Span;

pub struct Parsed {
    pub chunk: Chunk,
    pub errors: Vec<SyntaxError>,
}

const MAX_DEPTH: usize = 200;

struct Parser<'a> {
    src: &'a str,
    toks: &'a [Token],
    /// Indices into `toks` of the non-trivia tokens, ending with the `Eof` token.
    sig: Vec<usize>,
    p: usize,
    errors: Vec<SyntaxError>,
    depth: usize,
    last_end: usize,
    last_err_start: Option<usize>,
}

pub fn parse(src: &str, tokens: &[Token]) -> Parsed {
    let mut sig: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.is_trivia())
        .map(|(i, _)| i)
        .collect();
    if sig.last().map_or(true, |&i| tokens[i].kind != TokKind::Eof) {
        // Defensive: a hand-built token list without Eof. Parse an empty chunk.
        sig.clear();
        return Parsed {
            chunk: Chunk::default(),
            errors: vec![],
        };
    }
    let mut ps = Parser {
        src,
        toks: tokens,
        sig,
        p: 0,
        errors: vec![],
        depth: 0,
        last_end: 0,
        last_err_start: None,
    };
    let mut body = ps.block(true);
    body.span = Span::new(0, src.len());
    Parsed {
        chunk: Chunk { body },
        errors: ps.errors,
    }
}

fn binary_prio(op: &str) -> Option<(u8, u8)> {
    Some(match op {
        "or" => (1, 1),
        "and" => (2, 2),
        "<" | ">" | "<=" | ">=" | "~=" | "==" => (3, 3),
        ".." => (5, 4),
        "+" | "-" => (6, 6),
        "*" | "/" | "%" => (7, 7),
        "^" => (10, 9),
        _ => return None,
    })
}
const UNARY_PRIO: u8 = 8;

fn is_block_end(t: &str) -> bool {
    matches!(t, "end" | "else" | "elseif" | "until")
}

/// Keywords that can never continue an expression or argument list: used to resynchronise.
fn is_stopper(kind: TokKind, t: &str) -> bool {
    kind == TokKind::Eof
        || (kind == TokKind::Keyword
            && matches!(
                t,
                "end"
                    | "else"
                    | "elseif"
                    | "until"
                    | "then"
                    | "do"
                    | "local"
                    | "return"
                    | "if"
                    | "while"
                    | "for"
                    | "repeat"
                    | "break"
                    | "goto"
                    | "in"
            ))
}

impl<'a> Parser<'a> {
    fn tok(&self) -> Token {
        self.toks[self.sig[self.p]]
    }
    fn text(&self) -> &'a str {
        let t = self.tok();
        &self.src[t.span.start..t.span.end]
    }
    fn nth(&self, n: usize) -> (TokKind, &'a str) {
        let i = (self.p + n).min(self.sig.len() - 1);
        let t = self.toks[self.sig[i]];
        (t.kind, &self.src[t.span.start..t.span.end])
    }
    fn at_eof(&self) -> bool {
        self.tok().kind == TokKind::Eof
    }
    fn at_kw(&self, k: &str) -> bool {
        let t = self.tok();
        t.kind == TokKind::Keyword && self.text() == k
    }
    fn at_punct(&self, k: &str) -> bool {
        let t = self.tok();
        t.kind == TokKind::Punct && self.text() == k
    }
    fn bump(&mut self) -> Token {
        let t = self.tok();
        if t.kind != TokKind::Eof {
            self.p += 1;
            self.last_end = t.span.end;
        }
        t
    }
    fn eat_kw(&mut self, k: &str) -> Option<Span> {
        if self.at_kw(k) {
            Some(self.bump().span)
        } else {
            None
        }
    }
    fn eat_punct(&mut self, k: &str) -> Option<Span> {
        if self.at_punct(k) {
            Some(self.bump().span)
        } else {
            None
        }
    }
    /// Span used to report "expected X": the current token, or the last token at end of file.
    fn here(&self) -> Span {
        let t = self.tok();
        if t.kind == TokKind::Eof {
            let e = self.last_end;
            let mut s = e.saturating_sub(1);
            while s > 0 && !self.src.is_char_boundary(s) {
                s -= 1;
            }
            Span::new(s, e.max(s))
        } else {
            t.span
        }
    }
    fn near(&self) -> String {
        if self.at_eof() {
            "<eof>".into()
        } else {
            format!("'{}'", self.text().chars().take(24).collect::<String>())
        }
    }
    fn err(&mut self, span: Span, msg: impl Into<String>) {
        if self.last_err_start == Some(span.start) {
            return;
        }
        self.last_err_start = Some(span.start);
        self.errors.push(SyntaxError {
            span,
            message: msg.into(),
        });
    }
    fn expect_kw(&mut self, k: &str, opener: &str, opener_span: Span) -> Option<Span> {
        if let Some(s) = self.eat_kw(k) {
            return Some(s);
        }
        let near = self.near();
        self.err(
            opener_span,
            format!("'{k}' expected to close '{opener}' (found {near})"),
        );
        None
    }
    fn expect_punct(&mut self, k: &str, what: &str) -> Option<Span> {
        if let Some(s) = self.eat_punct(k) {
            return Some(s);
        }
        let near = self.near();
        let h = self.here();
        self.err(h, format!("'{k}' expected {what} near {near}"));
        None
    }
    fn name(&mut self, what: &str) -> Ident {
        let t = self.tok();
        if t.kind == TokKind::Name {
            self.bump();
            Ident {
                name: self.src[t.span.start..t.span.end].to_string(),
                span: t.span,
            }
        } else {
            let near = self.near();
            let h = self.here();
            self.err(h, format!("<name> expected {what} near {near}"));
            Ident {
                name: String::new(),
                span: Span::new(self.last_end, self.last_end),
            }
        }
    }

    // ---- blocks and statements -------------------------------------------------------------

    fn block(&mut self, top: bool) -> Block {
        let start = self.tok().span.start;
        let mut stmts = Vec::new();
        if self.depth >= MAX_DEPTH {
            let h = self.here();
            self.err(h, "chunk has too many syntax levels");
            // consume the rest of this level conservatively
            while !self.at_eof()
                && !(is_block_end(self.text()) && self.tok().kind == TokKind::Keyword)
            {
                self.bump();
            }
            return Block {
                stmts,
                span: Span::new(start, self.last_end.max(start)),
            };
        }
        self.depth += 1;
        loop {
            while self.eat_punct(";").is_some() {}
            let t = self.tok();
            if t.kind == TokKind::Eof {
                break;
            }
            if t.kind == TokKind::Keyword && is_block_end(self.text()) {
                if top {
                    let msg = format!("unexpected '{}'", self.text());
                    self.err(t.span, msg);
                    self.bump();
                    continue;
                }
                break;
            }
            let before = self.p;
            let stmt_is_return = self.at_kw("return");
            let s = self.stmt();
            stmts.push(s);
            if self.p == before {
                self.bump();
            }
            if stmt_is_return {
                while self.eat_punct(";").is_some() {}
                let t = self.tok();
                if t.kind != TokKind::Eof
                    && !(t.kind == TokKind::Keyword && is_block_end(self.text()))
                {
                    self.err(t.span, "'return' must be the last statement of a block");
                }
            }
        }
        self.depth -= 1;
        let end = if self.at_eof() {
            self.src.len()
        } else {
            self.last_end.max(start)
        };
        Block {
            stmts,
            span: Span::new(start.min(end), end),
        }
    }

    fn stmt(&mut self) -> Stmt {
        let t = self.tok();
        let start = t.span.start;
        let kind = if t.kind == TokKind::Keyword {
            match self.text() {
                "local" => self.local_stmt(),
                "function" => self.function_stmt(),
                "if" => self.if_stmt(),
                "while" => {
                    let kw = self.bump().span;
                    let cond = self.expr();
                    let c = self.eat_kw("do");
                    if c.is_none() {
                        let near = self.near();
                        let h = self.here();
                        self.err(h, format!("'do' expected near {near}"));
                    }
                    let body = self.block(false);
                    self.expect_kw("end", "while", kw);
                    StmtKind::While { cond, body }
                }
                "do" => {
                    let kw = self.bump().span;
                    let body = self.block(false);
                    self.expect_kw("end", "do", kw);
                    StmtKind::Do(body)
                }
                "for" => self.for_stmt(),
                "repeat" => {
                    let kw = self.bump().span;
                    let body = self.block(false);
                    self.expect_kw("until", "repeat", kw);
                    let cond = self.expr();
                    StmtKind::Repeat { body, cond }
                }
                "return" => {
                    self.bump();
                    let mut vals = Vec::new();
                    let t = self.tok();
                    let stop = t.kind == TokKind::Eof
                        || (t.kind == TokKind::Keyword && is_block_end(self.text()))
                        || self.at_punct(";");
                    if !stop {
                        vals = self.expr_list();
                    }
                    StmtKind::Return(vals)
                }
                "break" => {
                    self.bump();
                    StmtKind::Break
                }
                "goto" => {
                    self.bump();
                    let n = self.name("after 'goto'");
                    StmtKind::Goto(n)
                }
                _ => self.expr_stmt(),
            }
        } else if self.at_punct("::") {
            self.bump();
            let n = self.name("in label");
            self.expect_punct("::", "to close the label");
            StmtKind::Label(n)
        } else {
            self.expr_stmt()
        };
        let end = self.last_end.max(start);
        Stmt {
            kind,
            span: Span::new(start, end),
        }
    }

    fn local_stmt(&mut self) -> StmtKind {
        self.bump(); // local
        if self.at_kw("function") {
            let kw = self.bump().span;
            let name = self.name("after 'local function'");
            let func = self.func_body(kw);
            return StmtKind::LocalFunction {
                name,
                func: Box::new(func),
            };
        }
        let mut names = Vec::new();
        loop {
            let n = self.name("in local declaration");
            let empty = n.name.is_empty();
            names.push(n);
            if empty {
                break;
            }
            if self.eat_punct(",").is_none() {
                break;
            }
        }
        let values = if self.eat_punct("=").is_some() {
            self.expr_list()
        } else {
            Vec::new()
        };
        StmtKind::Local { names, values }
    }

    fn function_stmt(&mut self) -> StmtKind {
        let kw = self.bump().span;
        let first = self.name("after 'function'");
        let start = first.span.start;
        let mut parts = vec![first];
        let mut method = None;
        while self.at_punct(".") {
            self.bump();
            parts.push(self.name("after '.'"));
        }
        if self.at_punct(":") {
            self.bump();
            method = Some(self.name("after ':'"));
        }
        let end = self.last_end.max(start);
        let target = FuncName {
            parts,
            method,
            span: Span::new(start, end),
        };
        let func = self.func_body(kw);
        StmtKind::Function {
            target,
            func: Box::new(func),
        }
    }

    fn if_stmt(&mut self) -> StmtKind {
        let kw = self.bump().span;
        let mut arms = Vec::new();
        let mut else_block = None;
        let cond = self.expr();
        self.expect_then();
        let body = self.block(false);
        arms.push((cond, body));
        loop {
            if self.at_kw("elseif") {
                self.bump();
                let cond = self.expr();
                self.expect_then();
                let body = self.block(false);
                arms.push((cond, body));
            } else if self.at_kw("else") {
                self.bump();
                else_block = Some(self.block(false));
                break;
            } else {
                break;
            }
        }
        self.expect_kw("end", "if", kw);
        StmtKind::If { arms, else_block }
    }

    fn expect_then(&mut self) {
        if self.eat_kw("then").is_none() {
            let near = self.near();
            let h = self.here();
            self.err(h, format!("'then' expected near {near}"));
        }
    }

    fn for_stmt(&mut self) -> StmtKind {
        let kw = self.bump().span;
        let first = self.name("after 'for'");
        if self.at_punct("=") {
            self.bump();
            let start = self.expr();
            self.expect_punct(",", "in numeric 'for'");
            let limit = self.expr();
            let step = if self.eat_punct(",").is_some() {
                Some(self.expr())
            } else {
                None
            };
            if self.eat_kw("do").is_none() {
                let near = self.near();
                let h = self.here();
                self.err(h, format!("'do' expected near {near}"));
            }
            let body = self.block(false);
            self.expect_kw("end", "for", kw);
            StmtKind::NumericFor {
                var: first,
                start,
                limit,
                step,
                body,
            }
        } else {
            let mut vars = vec![first];
            while self.eat_punct(",").is_some() {
                vars.push(self.name("in 'for'"));
            }
            if self.eat_kw("in").is_none() {
                let near = self.near();
                let h = self.here();
                self.err(h, format!("'=' or 'in' expected near {near}"));
            }
            let iters = self.expr_list();
            if self.eat_kw("do").is_none() {
                let near = self.near();
                let h = self.here();
                self.err(h, format!("'do' expected near {near}"));
            }
            let body = self.block(false);
            self.expect_kw("end", "for", kw);
            StmtKind::GenericFor { vars, iters, body }
        }
    }

    fn expr_stmt(&mut self) -> StmtKind {
        let t = self.tok();
        if !self.can_start_expr() {
            let near = self.near();
            self.err(t.span, format!("unexpected symbol near {near}"));
            self.bump();
            return StmtKind::Error;
        }
        let e = self.suffixed_expr();
        if self.at_punct("=") || self.at_punct(",") {
            let mut targets = vec![e];
            while self.eat_punct(",").is_some() {
                targets.push(self.suffixed_expr());
            }
            for t in &targets {
                if !matches!(
                    t.kind,
                    ExprKind::Name(_)
                        | ExprKind::Field { .. }
                        | ExprKind::Index { .. }
                        | ExprKind::Error
                ) {
                    self.err(t.span, "syntax error: cannot assign to this expression");
                }
            }
            let values = if self.eat_punct("=").is_some() {
                self.expr_list()
            } else {
                let near = self.near();
                let h = self.here();
                self.err(h, format!("'=' expected near {near}"));
                Vec::new()
            };
            StmtKind::Assign { targets, values }
        } else if matches!(e.kind, ExprKind::Call { .. } | ExprKind::Method { .. }) {
            StmtKind::Call(e)
        } else {
            if !matches!(e.kind, ExprKind::Error) {
                let near = self.near();
                self.err(
                    e.span,
                    format!("syntax error: expected '=' or a function call (found {near})"),
                );
            }
            StmtKind::Call(e)
        }
    }

    // ---- functions -------------------------------------------------------------------------

    fn func_body(&mut self, kw: Span) -> FuncBody {
        let mut params = Vec::new();
        let mut vararg = None;
        let pstart = self.tok().span.start;
        let mut params_end = pstart;
        if self.at_punct("(") {
            self.bump();
            loop {
                let t = self.tok();
                if self.at_punct(")") {
                    break;
                }
                if t.kind == TokKind::Name {
                    params.push(self.name(""));
                } else if self.at_punct("...") {
                    vararg = Some(self.bump().span);
                } else {
                    let near = self.near();
                    self.err(t.span, format!("<name> or '...' expected near {near}"));
                    if is_stopper(t.kind, self.text()) {
                        break;
                    }
                    self.bump();
                    continue;
                }
                if self.eat_punct(",").is_none() {
                    break;
                }
            }
            if self.eat_punct(")").is_none() {
                let near = self.near();
                let h = self.here();
                self.err(h, format!("')' expected near {near}"));
            }
            params_end = self.last_end;
        } else {
            let near = self.near();
            let h = self.here();
            self.err(h, format!("'(' expected near {near}"));
        }
        let body = self.block(false);
        let end = self.expect_kw("end", "function", kw);
        let span_end = match end {
            Some(e) => e.end,
            None => {
                if self.at_eof() {
                    self.src.len()
                } else {
                    self.last_end
                }
            }
        };
        FuncBody {
            params,
            vararg,
            body,
            span: Span::new(kw.start, span_end),
            params_span: Span::new(pstart, params_end.max(pstart)),
            end,
        }
    }

    // ---- expressions -----------------------------------------------------------------------

    fn can_start_expr(&self) -> bool {
        let t = self.tok();
        match t.kind {
            TokKind::Name | TokKind::Number | TokKind::String | TokKind::LongString => true,
            TokKind::Keyword => {
                matches!(self.text(), "nil" | "true" | "false" | "function" | "not")
            }
            TokKind::Punct => matches!(self.text(), "(" | "{" | "-" | "#" | "..."),
            _ => false,
        }
    }

    fn expr_list(&mut self) -> Vec<Expr> {
        let mut v = vec![self.expr()];
        while self.eat_punct(",").is_some() {
            v.push(self.expr());
        }
        v
    }

    fn expr(&mut self) -> Expr {
        self.subexpr(0)
    }

    fn error_expr(&mut self, what: &str) -> Expr {
        let near = self.near();
        let h = self.here();
        self.err(h, format!("{what} expected near {near}"));
        let p = self.last_end;
        Expr {
            kind: ExprKind::Error,
            span: Span::new(p, p),
        }
    }

    fn subexpr(&mut self, limit: u8) -> Expr {
        if self.depth >= MAX_DEPTH {
            let h = self.here();
            self.err(h, "chunk has too many syntax levels");
            if !self.at_eof() && !is_stopper(self.tok().kind, self.text()) {
                self.bump();
            }
            let p = self.last_end;
            return Expr {
                kind: ExprKind::Error,
                span: Span::new(p, p),
            };
        }
        self.depth += 1;
        let t = self.tok();
        let mut lhs = if (t.kind == TokKind::Keyword && self.text() == "not")
            || (t.kind == TokKind::Punct && matches!(self.text(), "-" | "#"))
        {
            let op: &'static str = match self.text() {
                "not" => "not",
                "-" => "-",
                _ => "#",
            };
            self.bump();
            let operand = self.subexpr(UNARY_PRIO);
            Expr {
                span: Span::new(t.span.start, self.last_end.max(t.span.start)),
                kind: ExprKind::Unary {
                    op,
                    operand: Box::new(operand),
                },
            }
        } else {
            self.simple_expr()
        };
        let mut chain = 0usize;
        loop {
            let t = self.tok();
            let is_op = matches!(t.kind, TokKind::Punct | TokKind::Keyword);
            if !is_op {
                break;
            }
            let txt = self.text();
            let Some((lp, rp)) = binary_prio(txt) else {
                break;
            };
            if lp <= limit {
                break;
            }
            chain += 1;
            if chain > 4000 {
                self.err(t.span, "expression too long");
                break;
            }
            let op: &'static str = match txt {
                "or" => "or",
                "and" => "and",
                "<" => "<",
                ">" => ">",
                "<=" => "<=",
                ">=" => ">=",
                "~=" => "~=",
                "==" => "==",
                ".." => "..",
                "+" => "+",
                "-" => "-",
                "*" => "*",
                "/" => "/",
                "%" => "%",
                _ => "^",
            };
            self.bump();
            let rhs = self.subexpr(rp);
            let span = Span::new(lhs.span.start, self.last_end.max(lhs.span.start));
            lhs = Expr {
                span,
                kind: ExprKind::Binary {
                    op,
                    op_span: t.span,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            };
        }
        self.depth -= 1;
        lhs
    }

    fn simple_expr(&mut self) -> Expr {
        let t = self.tok();
        let kind = match t.kind {
            TokKind::Number => {
                self.bump();
                ExprKind::Number(self.src[t.span.start..t.span.end].to_string())
            }
            TokKind::String | TokKind::LongString => {
                self.bump();
                ExprKind::Str(decode_string(&self.src[t.span.start..t.span.end]))
            }
            TokKind::Keyword => match self.text() {
                "nil" => {
                    self.bump();
                    ExprKind::Nil
                }
                "true" => {
                    self.bump();
                    ExprKind::True
                }
                "false" => {
                    self.bump();
                    ExprKind::False
                }
                "function" => {
                    let kw = self.bump().span;
                    let f = self.func_body(kw);
                    return Expr {
                        span: f.span,
                        kind: ExprKind::Function(Box::new(f)),
                    };
                }
                _ => return self.error_expr("expression"),
            },
            TokKind::Punct => match self.text() {
                "..." => {
                    self.bump();
                    ExprKind::Vararg
                }
                "{" => {
                    let c = self.table();
                    return Expr {
                        span: Span::new(c.open.start, self.last_end),
                        kind: ExprKind::Table(c),
                    };
                }
                "(" => return self.suffixed_expr(),
                _ => return self.error_expr("expression"),
            },
            TokKind::Name => return self.suffixed_expr(),
            _ => return self.error_expr("expression"),
        };
        Expr { kind, span: t.span }
    }

    fn primary_expr(&mut self) -> Expr {
        let t = self.tok();
        if t.kind == TokKind::Name {
            self.bump();
            return Expr {
                kind: ExprKind::Name(self.src[t.span.start..t.span.end].to_string()),
                span: t.span,
            };
        }
        if self.at_punct("(") {
            self.bump();
            let inner = self.expr();
            if self.eat_punct(")").is_none() {
                let near = self.near();
                let h = self.here();
                self.err(h, format!("')' expected to close '(' near {near}"));
            }
            return Expr {
                span: Span::new(t.span.start, self.last_end.max(t.span.start)),
                kind: ExprKind::Paren(Box::new(inner)),
            };
        }
        self.error_expr("<name> or '('")
    }

    fn suffixed_expr(&mut self) -> Expr {
        let mut e = self.primary_expr();
        if matches!(e.kind, ExprKind::Error) {
            return e;
        }
        loop {
            let t = self.tok();
            match t.kind {
                TokKind::Punct => match self.text() {
                    "." => {
                        self.bump();
                        let name = self.name("after '.'");
                        let span = Span::new(e.span.start, self.last_end.max(e.span.start));
                        e = Expr {
                            span,
                            kind: ExprKind::Field {
                                obj: Box::new(e),
                                dot: t.span,
                                name,
                            },
                        };
                    }
                    "[" => {
                        self.bump();
                        let key = self.expr();
                        if self.eat_punct("]").is_none() {
                            let near = self.near();
                            let h = self.here();
                            self.err(h, format!("']' expected near {near}"));
                        }
                        let span = Span::new(e.span.start, self.last_end.max(e.span.start));
                        e = Expr {
                            span,
                            kind: ExprKind::Index {
                                obj: Box::new(e),
                                key: Box::new(key),
                            },
                        };
                    }
                    ":" => {
                        self.bump();
                        let name = self.name("after ':'");
                        let args = self.call_args(true);
                        let span = Span::new(e.span.start, self.last_end.max(e.span.start));
                        e = Expr {
                            span,
                            kind: ExprKind::Method {
                                obj: Box::new(e),
                                colon: t.span,
                                name,
                                args,
                            },
                        };
                    }
                    "(" | "{" => {
                        let args = self.call_args(false);
                        let span = Span::new(e.span.start, self.last_end.max(e.span.start));
                        e = Expr {
                            span,
                            kind: ExprKind::Call {
                                func: Box::new(e),
                                args,
                            },
                        };
                    }
                    _ => break,
                },
                TokKind::String | TokKind::LongString => {
                    let args = self.call_args(false);
                    let span = Span::new(e.span.start, self.last_end.max(e.span.start));
                    e = Expr {
                        span,
                        kind: ExprKind::Call {
                            func: Box::new(e),
                            args,
                        },
                    };
                }
                _ => break,
            }
        }
        e
    }

    fn call_args(&mut self, required: bool) -> Args {
        let t = self.tok();
        match t.kind {
            TokKind::String | TokKind::LongString => {
                self.bump();
                let e = Expr {
                    kind: ExprKind::Str(decode_string(&self.src[t.span.start..t.span.end])),
                    span: t.span,
                };
                Args {
                    items: vec![e],
                    span: t.span,
                    open: None,
                    close: None,
                    commas: vec![],
                    style: ArgStyle::Str,
                }
            }
            TokKind::Punct if self.text() == "{" => {
                let c = self.table();
                let span = Span::new(c.open.start, self.last_end);
                let e = Expr {
                    kind: ExprKind::Table(c),
                    span,
                };
                Args {
                    items: vec![e],
                    span,
                    open: None,
                    close: None,
                    commas: vec![],
                    style: ArgStyle::Table,
                }
            }
            TokKind::Punct if self.text() == "(" => {
                let open = self.bump().span;
                let mut items = Vec::new();
                let mut commas = Vec::new();
                let mut close = None;
                loop {
                    let t = self.tok();
                    if self.at_punct(")") {
                        close = Some(self.bump().span);
                        break;
                    }
                    if t.kind == TokKind::Eof
                        || is_stopper(t.kind, self.text()) && !self.can_start_expr()
                    {
                        let h = self.here();
                        self.err(open, "')' expected to close this call");
                        let _ = h;
                        break;
                    }
                    if self.can_start_expr() {
                        let e = self.expr();
                        items.push(e);
                    } else if self.at_punct(",") {
                        // empty argument slot: `f(a,,b)` / `f(,)`
                        let h = self.here();
                        self.err(h, "expression expected before ','");
                    } else {
                        let near = self.near();
                        self.err(t.span, format!("unexpected symbol near {near}"));
                        self.bump();
                        continue;
                    }
                    if let Some(c) = self.eat_punct(",") {
                        commas.push(c);
                        continue;
                    }
                    if self.at_punct(")") {
                        continue;
                    }
                    // Not a comma and not ')': the call is unterminated or garbage follows.
                    let near = self.near();
                    let h = self.here();
                    self.err(h, format!("')' expected near {near}"));
                    break;
                }
                let end = self.last_end.max(open.end);
                Args {
                    items,
                    span: Span::new(open.start, end),
                    open: Some(open),
                    close,
                    commas,
                    style: ArgStyle::Paren,
                }
            }
            _ => {
                if required {
                    let near = self.near();
                    let h = self.here();
                    self.err(h, format!("function arguments expected near {near}"));
                }
                let p = self.last_end;
                Args {
                    items: vec![],
                    span: Span::new(p, p),
                    open: None,
                    close: None,
                    commas: vec![],
                    style: ArgStyle::Paren,
                }
            }
        }
    }

    fn table(&mut self) -> TableCtor {
        let open = self.bump().span; // {
        let mut items = Vec::new();
        let mut close = None;
        loop {
            let t = self.tok();
            if self.at_punct("}") {
                close = Some(self.bump().span);
                break;
            }
            if t.kind == TokKind::Eof || (is_stopper(t.kind, self.text()) && !self.can_start_expr())
            {
                self.err(open, "'}' expected to close this table");
                break;
            }
            let before = self.p;
            if t.kind == TokKind::Name && self.nth(1) == (TokKind::Punct, "=") {
                let key = self.name("");
                self.bump(); // =
                let value = self.expr();
                items.push(TableItem::Named { key, value });
            } else if self.at_punct("[") {
                self.bump();
                let key = self.expr();
                if self.eat_punct("]").is_none() {
                    let near = self.near();
                    let h = self.here();
                    self.err(h, format!("']' expected near {near}"));
                }
                if self.eat_punct("=").is_none() {
                    let near = self.near();
                    let h = self.here();
                    self.err(h, format!("'=' expected near {near}"));
                }
                let value = self.expr();
                items.push(TableItem::Keyed { key, value });
            } else if self.can_start_expr() {
                let e = self.expr();
                items.push(TableItem::Pos(e));
            } else {
                let near = self.near();
                self.err(t.span, format!("unexpected symbol near {near}"));
                self.bump();
                continue;
            }
            if self.eat_punct(",").is_some() || self.eat_punct(";").is_some() {
                continue;
            }
            if self.at_punct("}") {
                continue;
            }
            let near = self.near();
            let h = self.here();
            self.err(h, format!("'}}' expected near {near}"));
            if self.p == before {
                self.bump();
            }
            // Resynchronise: stop the table at stoppers, otherwise keep reading items.
            let t = self.tok();
            if t.kind == TokKind::Eof || is_stopper(t.kind, self.text()) {
                break;
            }
        }
        TableCtor { items, open, close }
    }
}

/// Best-effort decoding of a Lua string literal (short or long) to its value.
pub fn decode_string(raw: &str) -> String {
    let b = raw.as_bytes();
    if b.first() == Some(&b'[') {
        let level = b.iter().skip(1).take_while(|&&c| c == b'=').count();
        let open = level + 2;
        let close = level + 2;
        if raw.len() >= open + close && raw.ends_with(&format!("]{}]", "=".repeat(level))) {
            let mut body = &raw[open..raw.len() - close];
            if let Some(r) = body
                .strip_prefix("\r\n")
                .or_else(|| body.strip_prefix('\n'))
            {
                body = r;
            }
            return body.to_string();
        }
        return raw.get(open.min(raw.len())..).unwrap_or("").to_string();
    }
    if b.len() < 2 {
        return String::new();
    }
    let q = b[0];
    let inner_end = if b.len() >= 2 && *b.last().unwrap() == q {
        b.len() - 1
    } else {
        b.len()
    };
    let inner = &raw[1..inner_end];
    let ib = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(ib.len());
    let mut i = 0;
    while i < ib.len() {
        let c = ib[i];
        if c != b'\\' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        let Some(&e) = ib.get(i) else { break };
        i += 1;
        match e {
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(11),
            b'\n' => out.push(b'\n'),
            b'z' => {
                while i < ib.len() && ib[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
            b'x' => {
                let h = inner
                    .get(i..i + 2)
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(v) = h {
                    out.push(v);
                    i += 2;
                }
            }
            b'u' if ib.get(i) == Some(&b'{') => {
                if let Some(end) = inner[i..].find('}') {
                    if let Some(ch) = u32::from_str_radix(&inner[i + 1..i + end], 16)
                        .ok()
                        .and_then(char::from_u32)
                    {
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                    i += end + 1;
                }
            }
            d if d.is_ascii_digit() => {
                let mut v = (d - b'0') as u32;
                let mut n = 1;
                while n < 3 && i < ib.len() && ib[i].is_ascii_digit() {
                    v = v * 10 + (ib[i] - b'0') as u32;
                    i += 1;
                    n += 1;
                }
                out.push(v.min(255) as u8);
            }
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::lexer::lex;

    fn p(s: &str) -> Parsed {
        let l = lex(s);
        parse(s, &l.tokens)
    }

    #[test]
    fn simple_ok() {
        let r = p(
            "local a, b = 1, 2\nfunction f(x, ...) return x + a * 2 ^ 3 end\nif a then b() \
            elseif c then d.e:f(1) else g{1,2} end\nfor i=1,10 do end\nfor k,v in pairs(t) do goto \
            cont ::cont:: end\nrepeat local z = 1 until z",
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.chunk.body.stmts.len(), 6);
    }

    #[test]
    fn recovers_and_continues() {
        let r = p("local x = \nfoo(1,\nfunction bar() end\nlocal y = 2\n");
        assert!(!r.errors.is_empty());
        assert!(r.chunk.body.stmts.len() >= 2);
        let r = p("x = = 3\ny = 4\n)\nz = 5");
        assert!(r.errors.len() >= 2);
        assert!(
            r.chunk
                .body
                .stmts
                .iter()
                .filter(|s| matches!(s.kind, StmtKind::Assign { .. }))
                .count()
                >= 3
        );
    }

    #[test]
    fn half_typed_member_access() {
        let r = p("local c = GetCreature()\nc.\nc:\n");
        assert!(!r.errors.is_empty());
    }

    #[test]
    fn missing_end_reports_opener() {
        let src = "function f()\n  if x then\n";
        let r = p(src);
        assert!(
            r.errors
                .iter()
                .any(|e| e.span.start == 0 || e.span.start == 15),
            "{:?}",
            r.errors
        );
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let s = "x = ".to_string() + &"(".repeat(5000) + "1" + &")".repeat(5000);
        let r = p(&s);
        assert!(!r.errors.is_empty());
        let s = "x = ".to_string() + &"{".repeat(5000);
        let _ = p(&s);
    }

    #[test]
    fn string_decode() {
        assert_eq!(decode_string("'a\\n\\65\\x42'"), "a\nAB");
        assert_eq!(decode_string("[==[\nxy]==]"), "xy");
        assert_eq!(decode_string("\"unterminated"), "unterminated");
    }
}
