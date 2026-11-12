//! Syntax tree for LuaJIT chunks. Every node carries a byte [`Span`]; erroneous constructs are
//! represented (`StmtKind::Error`, `ExprKind::Error`) so editor services still see the rest.
use crate::diag::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chunk {
    pub body: Block,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    Local {
        names: Vec<Ident>,
        values: Vec<Expr>,
    },
    LocalFunction {
        name: Ident,
        func: Box<FuncBody>,
    },
    /// `function a.b:c() end`
    Function {
        target: FuncName,
        func: Box<FuncBody>,
    },
    Assign {
        targets: Vec<Expr>,
        values: Vec<Expr>,
    },
    Call(Expr),
    Do(Block),
    While {
        cond: Expr,
        body: Block,
    },
    Repeat {
        body: Block,
        cond: Expr,
    },
    If {
        arms: Vec<(Expr, Block)>,
        else_block: Option<Block>,
    },
    NumericFor {
        var: Ident,
        start: Expr,
        limit: Expr,
        step: Option<Expr>,
        body: Block,
    },
    GenericFor {
        vars: Vec<Ident>,
        iters: Vec<Expr>,
        body: Block,
    },
    Return(Vec<Expr>),
    Break,
    Goto(Ident),
    Label(Ident),
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FuncName {
    /// `a`, `b` in `a.b:c`.
    pub parts: Vec<Ident>,
    pub method: Option<Ident>,
    pub span: Span,
}

impl FuncName {
    /// Dotted display name (`Creature:level_up`, `a.b`).
    pub fn display(&self) -> String {
        let mut s = self
            .parts
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(".");
        if let Some(m) = &self.method {
            s.push(':');
            s.push_str(&m.name);
        }
        s
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FuncBody {
    pub params: Vec<Ident>,
    pub vararg: Option<Span>,
    pub body: Block,
    /// From the `function` keyword to `end` (or to the last parsed token when unclosed).
    pub span: Span,
    /// `(` ... `)` including the parentheses when present.
    pub params_span: Span,
    /// Span of the closing `end`, `None` when missing.
    pub end: Option<Span>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum ArgStyle {
    Paren,
    /// `f"str"` / `f[[str]]`
    Str,
    /// `f{...}`
    Table,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    pub items: Vec<Expr>,
    /// Whole argument region including parentheses.
    pub span: Span,
    pub open: Option<Span>,
    /// Closing parenthesis, `None` when missing (half-typed call).
    pub close: Option<Span>,
    /// Commas between arguments (offsets), one per separator seen.
    pub commas: Vec<Span>,
    pub style: ArgStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableCtor {
    pub items: Vec<TableItem>,
    pub open: Span,
    pub close: Option<Span>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TableItem {
    Pos(Expr),
    Named { key: Ident, value: Expr },
    Keyed { key: Expr, value: Expr },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Nil,
    True,
    False,
    Number(String),
    /// Decoded value (best effort) of a short or long string literal.
    Str(String),
    Vararg,
    Function(Box<FuncBody>),
    Name(String),
    Field {
        obj: Box<Expr>,
        dot: Span,
        name: Ident,
    },
    Index {
        obj: Box<Expr>,
        key: Box<Expr>,
    },
    Call {
        func: Box<Expr>,
        args: Args,
    },
    Method {
        obj: Box<Expr>,
        colon: Span,
        name: Ident,
        args: Args,
    },
    Table(TableCtor),
    Binary {
        op: &'static str,
        op_span: Span,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: &'static str,
        operand: Box<Expr>,
    },
    Paren(Box<Expr>),
    Error,
}

impl Expr {
    /// `a.b.c` / `a` as path of names when the expression is a pure name/field chain.
    pub fn path(&self) -> Option<Vec<&str>> {
        match &self.kind {
            ExprKind::Name(n) => Some(vec![n.as_str()]),
            ExprKind::Field { obj, name, .. } => {
                let mut p = obj.path()?;
                p.push(name.name.as_str());
                Some(p)
            }
            _ => None,
        }
    }
}

/// A syntax error found by the parser or lexer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub span: Span,
    pub message: String,
}
