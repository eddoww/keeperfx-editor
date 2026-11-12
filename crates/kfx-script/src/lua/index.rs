//! The annotation index: functions, classes, aliases, enums and globals collected from library
//! roots (`fxdata/lua`, a campaign `lua/` directory, the map folder) and from the current file.
//!
//! The index is plain serialisable data; the stock install's index ships as `data/lua_api.json`
//! (regenerate with the `lua_index_gen` test, see its docs) so the editor works before an install
//! is configured.
use super::annot::{parse_ty, DocBlock, Tag, Ty};
use super::ast::*;
use super::doc::LuaDoc;
use super::infer::Env;
use crate::diag::Span;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub line: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ParamDef {
    pub name: String,
    pub ty: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    /// False when the parameter has no `@param` annotation.
    #[serde(default, skip_serializing_if = "is_false")]
    pub annotated: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ReturnDef {
    pub ty: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<ParamDef>,
    /// Type of `...` when the function is variadic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vararg: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub returns: Vec<ReturnDef>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overloads: Vec<String>,
    /// Has at least one `@param` or `@return`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub annotated: bool,
    /// Declared with `:` (implicit `self` as first argument).
    #[serde(default, skip_serializing_if = "is_false")]
    pub method: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub nodiscard: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

impl FuncDef {
    /// One-line signature: `name(a: integer, b?: string): boolean`.
    pub fn signature(&self) -> String {
        let mut s = String::new();
        s.push_str(&self.name);
        s.push('(');
        for (i, p) in self.params.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&p.name);
            if p.optional {
                s.push('?');
            }
            if p.annotated || p.ty != "any" {
                s.push_str(": ");
                s.push_str(&p.ty);
            }
        }
        if let Some(v) = &self.vararg {
            if !self.params.is_empty() {
                s.push_str(", ");
            }
            s.push_str("...");
            if v != "any" {
                s.push_str(": ");
                s.push_str(v);
            }
        }
        s.push(')');
        if !self.returns.is_empty() {
            s.push_str(": ");
            s.push_str(
                &self
                    .returns
                    .iter()
                    .map(|r| r.ty.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        s
    }
    pub fn required_params(&self) -> usize {
        self.params
            .iter()
            .rposition(|p| !p.optional)
            .map_or(0, |i| i + 1)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub ty: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ClassDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, FieldDef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub methods: BTreeMap<String, FuncDef>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    /// True when no `---@class` declares it: it only exists because functions are defined on a
    /// global table of that name.
    #[serde(default, skip_serializing_if = "is_false")]
    pub implicit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AliasLit {
    /// Literal text without quotes for strings.
    pub value: String,
    pub is_str: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AliasDef {
    pub name: String,
    /// The full union as written (normalised).
    pub ty: String,
    /// Direct literal members (string and number literals) with their `---|` comments.
    pub literals: Vec<AliasLit>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub members: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GlobalDef {
    pub name: String,
    pub ty: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiIndex {
    #[serde(default)]
    pub functions: BTreeMap<String, FuncDef>,
    #[serde(default)]
    pub classes: BTreeMap<String, ClassDef>,
    #[serde(default)]
    pub aliases: BTreeMap<String, AliasDef>,
    #[serde(default)]
    pub enums: BTreeMap<String, EnumDef>,
    #[serde(default)]
    pub globals: BTreeMap<String, GlobalDef>,
    /// Files that contributed, relative to their root.
    #[serde(default)]
    pub files: Vec<String>,
}

/// Alias members after following alias references.
#[derive(Clone, Debug, Default)]
pub struct AliasValues {
    pub literals: Vec<AliasLit>,
    /// Non-literal member types that are not aliases (`string`, `Player`, ...).
    pub other: Vec<String>,
    /// True when an arbitrary string is accepted (`string`, `any`, unknown names).
    pub open: bool,
}

pub enum Member<'a> {
    Field(&'a FieldDef, &'a ClassDef),
    Method(&'a FuncDef, &'a ClassDef),
}

const BUILTIN_TYPES: &[&str] = &[
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
    "true",
    "false",
];

impl ApiIndex {
    pub fn function(&self, name: &str) -> Option<&FuncDef> {
        self.functions.get(name)
    }
    pub fn class(&self, name: &str) -> Option<&ClassDef> {
        self.classes.get(name)
    }
    pub fn alias(&self, name: &str) -> Option<&AliasDef> {
        self.aliases.get(name)
    }

    /// Class and its ancestors, nearest first.
    pub fn class_chain(&self, name: &str) -> Vec<&ClassDef> {
        let mut out: Vec<&ClassDef> = Vec::new();
        let mut queue = vec![name.to_string()];
        let mut seen = BTreeSet::new();
        while let Some(n) = queue.first().cloned() {
            queue.remove(0);
            if !seen.insert(n.clone()) {
                continue;
            }
            if let Some(c) = self.classes.get(&n) {
                out.push(c);
                queue.extend(c.parents.iter().cloned());
            }
        }
        out
    }

    pub fn find_member(&self, class: &str, member: &str) -> Option<Member<'_>> {
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

    /// Follow alias references and collect literal members.
    pub fn alias_values(&self, name: &str) -> Option<AliasValues> {
        self.alias(name)?;
        let mut out = AliasValues::default();
        let mut seen = BTreeSet::new();
        self.collect_alias(name, &mut out, &mut seen);
        Some(out)
    }

    fn collect_alias(&self, name: &str, out: &mut AliasValues, seen: &mut BTreeSet<String>) {
        if !seen.insert(name.to_string()) {
            return;
        }
        let Some(a) = self.alias(name) else { return };
        out.literals.extend(a.literals.iter().cloned());
        self.collect_ty(&parse_ty(&a.ty), out, seen);
    }

    /// Resolve an arbitrary type expression into literal members, other types and openness.
    pub fn type_values(&self, ty: &Ty) -> AliasValues {
        let mut out = AliasValues::default();
        let mut seen = BTreeSet::new();
        self.collect_ty(ty, &mut out, &mut seen);
        out
    }

    fn collect_ty(&self, ty: &Ty, out: &mut AliasValues, seen: &mut BTreeSet<String>) {
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
                    } else if self.aliases.contains_key(&n) {
                        self.collect_alias(&n, out, seen);
                    } else if BUILTIN_TYPES.contains(&n.as_str())
                        || self.classes.contains_key(&n)
                        || self.enums.contains_key(&n)
                    {
                        out.other.push(n);
                    } else {
                        // unknown type name: cannot say anything about the accepted strings
                        out.open = true;
                        out.other.push(n);
                    }
                }
                Ty::Generic(n, _) => out.other.push(n),
                Ty::Array(_) | Ty::Fun(_) | Ty::Table(_) | Ty::Bool(_) | Ty::Vararg(_) => {
                    out.other.push(m.to_string())
                }
                Ty::Union(_) | Ty::Optional(_) => {}
            }
        }
    }

    /// Merge another index into this one (later definitions of the same name win for
    /// functions/aliases/globals; classes are merged member by member).
    pub fn merge(&mut self, o: ApiIndex) {
        for (k, v) in o.functions {
            match self.functions.get(&k) {
                // an unannotated redefinition must not erase a documented stub
                Some(old) if !v.annotated && old.annotated => {}
                _ => {
                    self.functions.insert(k, v);
                }
            }
        }
        for (k, v) in o.classes {
            match self.classes.get_mut(&k) {
                None => {
                    self.classes.insert(k, v);
                }
                Some(c) => {
                    for p in v.parents {
                        if !c.parents.contains(&p) {
                            c.parents.push(p);
                        }
                    }
                    c.fields.extend(v.fields);
                    for (mk, mv) in v.methods {
                        match c.methods.get(&mk) {
                            Some(old) if !mv.annotated && old.annotated => {}
                            _ => {
                                c.methods.insert(mk, mv);
                            }
                        }
                    }
                    if c.doc.is_empty() {
                        c.doc = v.doc;
                    }
                    if c.implicit && !v.implicit {
                        c.implicit = false;
                        c.source = v.source;
                    }
                }
            }
        }
        for (k, v) in o.aliases {
            self.aliases.insert(k, v);
        }
        for (k, v) in o.enums {
            self.enums.insert(k, v);
        }
        for (k, v) in o.globals {
            match self.globals.get(&k) {
                // keep an annotated type over a later untyped assignment
                Some(old) if v.ty == "any" && old.ty != "any" => {}
                _ => {
                    self.globals.insert(k, v);
                }
            }
        }
        for f in o.files {
            if !self.files.contains(&f) {
                self.files.push(f);
            }
        }
    }

    /// Parse and add one source file.
    pub fn add_source(&mut self, file: &str, src: &str) {
        let doc = LuaDoc::parse_named(file, src);
        let mut d = doc.defs;
        d.files.push(file.to_string());
        self.merge(d);
    }

    /// Add every `.lua` file below `root` (names relative to `root`, `/` separated, sorted).
    pub fn add_root(&mut self, root: &Path) -> usize {
        let mut files = Vec::new();
        collect_lua(root, &mut files);
        files.sort_by_key(|p| p.to_string_lossy().to_ascii_lowercase());
        let mut n = 0;
        for f in files {
            let Ok(bytes) = std::fs::read(&f) else {
                continue;
            };
            let rel = f
                .strip_prefix(root)
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            self.add_source(&rel, &String::from_utf8_lossy(&bytes));
            n += 1;
        }
        n
    }

    /// Like [`ApiIndex::add_root`] but file names are prefixed with `label/` so equally named files
    /// of different roots (`fxdata/lua/init.lua`, the campaign's `lua/init.lua`) stay
    /// distinguishable.
    pub fn add_root_labeled(&mut self, label: &str, root: &Path) -> usize {
        let mut files = Vec::new();
        collect_lua(root, &mut files);
        files.sort_by_key(|p| p.to_string_lossy().to_ascii_lowercase());
        let mut n = 0;
        for f in files {
            let Ok(bytes) = std::fs::read(&f) else {
                continue;
            };
            let rel = f
                .strip_prefix(root)
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            self.add_source(&format!("{label}/{rel}"), &String::from_utf8_lossy(&bytes));
            n += 1;
        }
        n
    }

    pub fn build(roots: &[&Path]) -> ApiIndex {
        let mut ix = ApiIndex::default();
        for r in roots {
            ix.add_root(r);
        }
        ix
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Result<ApiIndex, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// The index of the stock KeeperFX library, embedded from `data/lua_api.json`.
    pub fn stock() -> ApiIndex {
        const JSON: &str = include_str!("../../../../data/lua_api.json");
        ApiIndex::from_json(JSON).unwrap_or_default()
    }

    /// True when `name` is defined anywhere in the index as a global.
    pub fn knows_global(&self, name: &str) -> bool {
        self.functions.contains_key(name)
            || self.globals.contains_key(name)
            || self.classes.contains_key(name)
            || self.enums.contains_key(name)
    }
}

fn collect_lua(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_lua(&p, out);
        } else if p
            .extension()
            .map_or(false, |x| x.eq_ignore_ascii_case("lua"))
        {
            out.push(p);
        }
    }
}

// ---- extraction ----------------------------------------------------------------------------

fn source_of(file: &str, doc: &LuaDoc, span: Span) -> Source {
    Source {
        file: file.to_string(),
        start: span.start,
        end: span.end,
        line: doc.line_of(span.start),
    }
}

/// Build a function definition from its syntax and attached annotation block.
pub fn make_func(
    file: &str,
    fdoc: &LuaDoc,
    name: &str,
    f: &FuncBody,
    block: Option<&DocBlock>,
    method: bool,
    name_span: Span,
) -> FuncDef {
    let mut params: Vec<ParamDef> = Vec::new();
    let mut returns = Vec::new();
    let mut overloads = Vec::new();
    let mut deprecated = None;
    let mut vararg = if f.vararg.is_some() {
        Some("any".to_string())
    } else {
        None
    };
    let mut tag_params: Vec<(&str, &str, bool, &str)> = Vec::new();
    let mut annotated = false;
    let mut nodiscard = false;
    let mut doc = String::new();
    if let Some(b) = block {
        doc = b.description.clone();
        for (t, _) in &b.tags {
            match t {
                Tag::Param {
                    name,
                    ty,
                    optional,
                    doc,
                } => {
                    annotated = true;
                    if name == "..." {
                        vararg = Some(ty.clone());
                    } else if name != "self" {
                        tag_params.push((name, ty, *optional, doc));
                    }
                }
                Tag::Return { ty, doc } => {
                    annotated = true;
                    returns.push(ReturnDef {
                        ty: ty.clone(),
                        doc: doc.clone(),
                    });
                }
                Tag::Overload(o) => overloads.push(o.clone()),
                Tag::Deprecated(m) => deprecated = Some(m.clone()),
                Tag::Vararg(t) => vararg = Some(t.clone()),
                Tag::Nodiscard => nodiscard = true,
                _ => {}
            }
        }
    }
    for p in &f.params {
        match tag_params.iter().find(|t| t.0 == p.name) {
            Some((_, ty, opt, d)) => {
                let optional = *opt || parse_ty(ty).accepts_nil();
                params.push(ParamDef {
                    name: p.name.clone(),
                    ty: ty.to_string(),
                    optional,
                    doc: d.to_string(),
                    annotated: true,
                });
            }
            None => params.push(ParamDef {
                name: p.name.clone(),
                ty: "any".into(),
                optional: true,
                doc: String::new(),
                annotated: false,
            }),
        }
    }
    for (n, ty, opt, d) in &tag_params {
        if !f.params.iter().any(|p| p.name == *n) {
            let optional = *opt || parse_ty(ty).accepts_nil();
            params.push(ParamDef {
                name: n.to_string(),
                ty: ty.to_string(),
                optional,
                doc: d.to_string(),
                annotated: true,
            });
        }
    }
    FuncDef {
        name: name.to_string(),
        params,
        vararg,
        returns,
        doc,
        deprecated,
        overloads,
        annotated,
        method,
        nodiscard,
        source: Some(source_of(file, fdoc, name_span)),
    }
}

fn flatten_type_values(types: &[String]) -> String {
    types.join("|")
}

fn alias_from_tag(
    file: &str,
    fdoc: &LuaDoc,
    name: &str,
    ty: &Option<String>,
    variants: &[(String, String)],
    doc: &str,
    span: Span,
) -> AliasDef {
    let mut literals = Vec::new();
    let mut union_parts: Vec<String> = Vec::new();
    if let Some(t) = ty {
        let parsed = parse_ty(t);
        for m in parsed.members() {
            match &m {
                Ty::Str(s) => literals.push(AliasLit {
                    value: s.clone(),
                    is_str: true,
                    doc: String::new(),
                }),
                Ty::Num(n) => literals.push(AliasLit {
                    value: n.clone(),
                    is_str: false,
                    doc: String::new(),
                }),
                _ => {}
            }
        }
        union_parts.push(parsed.to_string());
    }
    for (vty, vdoc) in variants {
        let parsed = parse_ty(vty);
        for m in parsed.members() {
            match &m {
                Ty::Str(s) => literals.push(AliasLit {
                    value: s.clone(),
                    is_str: true,
                    doc: vdoc.clone(),
                }),
                Ty::Num(n) => literals.push(AliasLit {
                    value: n.clone(),
                    is_str: false,
                    doc: vdoc.clone(),
                }),
                _ => {}
            }
        }
        union_parts.push(parsed.to_string());
    }
    AliasDef {
        name: name.to_string(),
        ty: flatten_type_values(&union_parts),
        literals,
        doc: doc.to_string(),
        source: Some(source_of(file, fdoc, span)),
    }
}

/// Extract the definitions a file makes. Used for library roots and for the current buffer.
pub fn extract_defs(doc: &LuaDoc) -> ApiIndex {
    let file = doc.file.as_str();
    let mut ix = ApiIndex::default();
    let mut declared_classes: BTreeSet<String> = BTreeSet::new();

    // 1. standalone declarations in every annotation block
    for b in &doc.blocks {
        let mut current_class: Option<String> = None;
        for (t, span) in &b.tags {
            match t {
                Tag::Class { name, parents, .. } => {
                    declared_classes.insert(name.clone());
                    ix.classes.insert(
                        name.clone(),
                        ClassDef {
                            name: name.clone(),
                            parents: parents.clone(),
                            doc: b.description.clone(),
                            source: Some(source_of(file, doc, *span)),
                            ..Default::default()
                        },
                    );
                    current_class = Some(name.clone());
                }
                Tag::Field {
                    name,
                    ty,
                    optional,
                    doc: fdoc,
                    visibility,
                } => {
                    if visibility.as_deref() == Some("private") {
                        continue;
                    }
                    if let Some(c) = current_class.as_ref().and_then(|c| ix.classes.get_mut(c)) {
                        let ty = if *optional && !ty.ends_with('?') {
                            format!("{ty}?")
                        } else {
                            ty.clone()
                        };
                        c.fields.insert(
                            name.clone(),
                            FieldDef {
                                name: name.clone(),
                                ty,
                                optional: *optional,
                                doc: fdoc.clone(),
                                source: Some(source_of(file, doc, *span)),
                            },
                        );
                    }
                }
                Tag::Alias { name, ty, variants } => {
                    ix.aliases.insert(
                        name.clone(),
                        alias_from_tag(file, doc, name, ty, variants, &b.description, *span),
                    );
                }
                Tag::Enum { name } => {
                    ix.enums.insert(
                        name.clone(),
                        EnumDef {
                            name: name.clone(),
                            members: vec![],
                            source: Some(source_of(file, doc, *span)),
                        },
                    );
                }
                _ => {}
            }
        }
    }

    // 2. statements (file level, looking through top-level `do`/`if`)
    let mut stmts = Vec::new();
    super::analyze::top_level_stmts(&doc.chunk.body, &mut stmts);
    let env_empty = ApiIndex::default();
    let env = Env::new(doc, &env_empty);
    for s in stmts {
        let block = doc.block_for(s.span.start);
        match &s.kind {
            StmtKind::Function { target, func } => {
                let first = &target.parts[0];
                if first.name.is_empty() {
                    continue;
                }
                let name = target.display();
                if target.parts.len() == 1 && target.method.is_none() {
                    let fd = make_func(file, doc, &name, func, block, false, target.span);
                    ix.functions.insert(
                        first.name.clone(),
                        FuncDef {
                            name: first.name.clone(),
                            ..fd
                        },
                    );
                } else if target.parts.len() == 1
                    || (target.parts.len() == 2 && target.method.is_none())
                {
                    let (member, method) = match &target.method {
                        Some(m) => (m.name.clone(), true),
                        None => (target.parts[1].name.clone(), false),
                    };
                    if member.is_empty() {
                        continue;
                    }
                    let is_local_table = doc
                        .analysis
                        .resolve(&first.name, target.span.start)
                        .is_some();
                    if is_local_table && !declared_classes.contains(&first.name) {
                        continue;
                    }
                    let mut fd = make_func(file, doc, &member, func, block, method, target.span);
                    if member == "new" && fd.returns.is_empty() {
                        fd.returns.push(ReturnDef {
                            ty: first.name.clone(),
                            doc: String::new(),
                        });
                    }
                    let c = ix
                        .classes
                        .entry(first.name.clone())
                        .or_insert_with(|| ClassDef {
                            name: first.name.clone(),
                            implicit: true,
                            ..Default::default()
                        });
                    c.methods.insert(member, fd);
                }
            }
            StmtKind::Assign { targets, values } if targets.len() == 1 => {
                let value = values.first();
                match &targets[0].kind {
                    ExprKind::Name(n) => {
                        let is_local = doc.analysis.resolve(n, targets[0].span.start).is_some();
                        if is_local {
                            continue;
                        }
                        if let Some(Expr {
                            kind: ExprKind::Function(f),
                            ..
                        }) = value
                        {
                            let fd = make_func(file, doc, n, f, block, false, targets[0].span);
                            ix.functions.insert(n.clone(), fd);
                            continue;
                        }
                        let declared = doc.declared_type(s.span.start, s.span.end, 0);
                        let class_here = block.map_or(false, |b| {
                            b.tags.iter().any(|(t, _)| matches!(t, Tag::Class { .. }))
                        });
                        let ty = match declared {
                            Some(t) => t,
                            None if class_here => n.clone(),
                            None => value
                                .and_then(|v| env.infer(v, s.span.start))
                                .map(|t| t.to_string())
                                .unwrap_or_else(|| "any".into()),
                        };
                        let gd = GlobalDef {
                            name: n.clone(),
                            ty,
                            doc: block.map(|b| b.description.clone()).unwrap_or_default(),
                            source: Some(source_of(file, doc, targets[0].span)),
                        };
                        ix.globals.insert(n.clone(), gd);
                    }
                    ExprKind::Field { obj, name, .. } => {
                        // `Class.member = function ... end`
                        if let (
                            ExprKind::Name(c),
                            Some(Expr {
                                kind: ExprKind::Function(f),
                                ..
                            }),
                        ) = (&obj.kind, value)
                        {
                            if doc.analysis.resolve(c, targets[0].span.start).is_some()
                                && !declared_classes.contains(c)
                            {
                                continue;
                            }
                            let fd = make_func(file, doc, &name.name, f, block, false, name.span);
                            let cl = ix.classes.entry(c.clone()).or_insert_with(|| ClassDef {
                                name: c.clone(),
                                implicit: true,
                                ..Default::default()
                            });
                            cl.methods.insert(name.name.clone(), fd);
                        }
                    }
                    _ => {}
                }
            }
            StmtKind::Local { names, values } => {
                // `---@enum Name` + `local Name = { A = 1 }`
                if let Some(b) = block {
                    for (t, _) in &b.tags {
                        if let Tag::Enum { name } = t {
                            if let Some(Expr {
                                kind: ExprKind::Table(tc),
                                ..
                            }) = values.first()
                            {
                                let members = tc
                                    .items
                                    .iter()
                                    .filter_map(|i| match i {
                                        TableItem::Named { key, .. } => Some(key.name.clone()),
                                        _ => None,
                                    })
                                    .collect();
                                if let Some(e) = ix.enums.get_mut(name) {
                                    e.members = members;
                                }
                            }
                            let _ = names;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    ix
}
