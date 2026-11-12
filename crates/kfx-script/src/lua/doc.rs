//! [`LuaDoc`]: one parsed Lua file (tokens, tree, annotations, scopes, local definitions).
use super::analyze::{analyze, walk_stmts, Analysis};
use super::annot::{extract_blocks, DocBlock, Tag};
use super::ast::*;
use super::index::{extract_defs, make_func, ApiIndex, FuncDef};
use super::lexer::{lex, LexError, TokKind, Token};
use super::parser::parse;
use crate::diag::Span;
use std::collections::HashMap;

pub struct LuaDoc {
    pub file: String,
    pub src: String,
    pub tokens: Vec<Token>,
    pub lex_errors: Vec<LexError>,
    pub chunk: Chunk,
    pub syntax_errors: Vec<SyntaxError>,
    pub blocks: Vec<DocBlock>,
    /// statement start offset -> index into `blocks`
    pub attach: HashMap<usize, usize>,
    pub analysis: Analysis,
    /// Definitions made by this file (functions, classes, aliases, globals).
    pub defs: ApiIndex,
    /// Signatures of the `local function`s of this file, keyed by local id.
    pub local_funcs: HashMap<usize, FuncDef>,
    line_starts: Vec<usize>,
}

impl LuaDoc {
    pub fn parse(src: &str) -> LuaDoc {
        LuaDoc::parse_named("<buffer>", src)
    }

    pub fn parse_named(file: &str, src: &str) -> LuaDoc {
        let lexed = lex(src);
        let parsed = parse(src, &lexed.tokens);
        let blocks = extract_blocks(src, &lexed.tokens);
        let analysis = analyze(&parsed.chunk, src.len());
        let mut line_starts = vec![0usize];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        let mut doc = LuaDoc {
            file: file.to_string(),
            src: src.to_string(),
            tokens: lexed.tokens,
            lex_errors: lexed.errors,
            chunk: parsed.chunk,
            syntax_errors: parsed.errors,
            blocks,
            attach: HashMap::new(),
            analysis,
            defs: ApiIndex::default(),
            local_funcs: HashMap::new(),
            line_starts,
        };
        doc.compute_attach();
        doc.compute_local_funcs();
        doc.defs = extract_defs(&doc);
        doc
    }

    fn compute_attach(&mut self) {
        let mut attach = HashMap::new();
        for (bi, b) in self.blocks.iter().enumerate() {
            let mut i = self.tokens.partition_point(|t| t.span.start < b.span.end);
            let mut newlines = 0;
            while let Some(t) = self.tokens.get(i) {
                match t.kind {
                    TokKind::Whitespace => {
                        newlines += t.text(&self.src).bytes().filter(|&c| c == b'\n').count();
                        i += 1;
                    }
                    TokKind::Eof => break,
                    TokKind::LineComment | TokKind::BlockComment => break,
                    _ => {
                        if newlines <= 1 {
                            attach.entry(t.span.start).or_insert(bi);
                        }
                        break;
                    }
                }
            }
        }
        self.attach = attach;
    }

    fn compute_local_funcs(&mut self) {
        let mut found: Vec<(Span, FuncDef)> = Vec::new();
        {
            let file = self.file.clone();
            let this: &LuaDoc = self;
            walk_stmts(&this.chunk.body, &mut |s| {
                if let StmtKind::LocalFunction { name, func } = &s.kind {
                    let block = this.block_for(s.span.start);
                    let fd = make_func(&file, this, &name.name, func, block, false, name.span);
                    found.push((name.span, fd));
                }
            });
        }
        let by_decl: HashMap<(usize, usize), usize> = self
            .analysis
            .locals
            .iter()
            .enumerate()
            .map(|(i, l)| ((l.decl.start, l.decl.end), i))
            .collect();
        for (span, fd) in found {
            if let Some(&id) = by_decl.get(&(span.start, span.end)) {
                self.local_funcs.insert(id, fd);
            }
        }
    }

    /// The annotation block attached to the statement starting at `stmt_start`.
    pub fn block_for(&self, stmt_start: usize) -> Option<&DocBlock> {
        self.attach.get(&stmt_start).map(|&i| &self.blocks[i])
    }

    /// `---@type T` trailing comment on the same line right after a statement.
    pub fn trailing_type(&self, stmt_end: usize) -> Option<String> {
        let i = self.tokens.partition_point(|t| t.span.start < stmt_end);
        for t in self.tokens[i..].iter() {
            match t.kind {
                TokKind::Whitespace if !t.text(&self.src).contains('\n') => continue,
                TokKind::LineComment => {
                    let txt = t.text(&self.src).trim_start_matches('-').trim();
                    let rest = txt.strip_prefix("@type")?;
                    return Some(super::annot::parse_ty(rest).to_string());
                }
                _ => return None,
            }
        }
        None
    }

    /// Type annotation (`---@type`) of the declaration statement starting at `stmt_start`.
    pub fn declared_type(
        &self,
        stmt_start: usize,
        stmt_end: usize,
        index: usize,
    ) -> Option<String> {
        if let Some(b) = self.block_for(stmt_start) {
            for (t, _) in &b.tags {
                if let Tag::Type(v) = t {
                    if let Some(x) = v.get(index) {
                        return Some(x.clone());
                    }
                }
            }
        }
        if index == 0 {
            return self.trailing_type(stmt_end);
        }
        None
    }

    /// 1-based line of a byte offset.
    pub fn line_of(&self, off: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= off).max(1)
    }
    /// Byte offset of the start of a 1-based line.
    pub fn line_start(&self, line: usize) -> usize {
        self.line_starts
            .get(line.saturating_sub(1))
            .copied()
            .unwrap_or(self.src.len())
    }
    /// (line, column in bytes), both 1-based/0-based respectively: line 1-based, column 0-based.
    pub fn line_col(&self, off: usize) -> (usize, usize) {
        let l = self.line_of(off);
        (l, off - self.line_start(l))
    }
    pub fn text(&self, span: Span) -> &str {
        self.src.get(span.start..span.end).unwrap_or("")
    }
    /// Index of the token containing `off` (a token touching `off` from the left when between).
    pub fn token_at(&self, off: usize) -> Option<usize> {
        let i = self.tokens.partition_point(|t| t.span.end <= off);
        if i < self.tokens.len()
            && self.tokens[i].span.start <= off
            && self.tokens[i].kind != TokKind::Eof
        {
            Some(i)
        } else {
            None
        }
    }
    /// Index of the last significant (non-trivia) token that ends at or before `off`.
    pub fn prev_sig_token(&self, off: usize) -> Option<usize> {
        let i = self.tokens.partition_point(|t| t.span.end <= off);
        (0..i)
            .rev()
            .find(|&k| !self.tokens[k].is_trivia() && self.tokens[k].kind != TokKind::Eof)
    }
    pub fn token_text(&self, i: usize) -> &str {
        self.tokens[i].text(&self.src)
    }
}
