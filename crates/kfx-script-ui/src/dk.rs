//! [`Language`] implementation for DK / KeeperFX level scripts (`mapNNNNN.txt`).
use crate::docs;
use crate::lang::*;
use kfx_script::services::{arg_kind_at, context_at, Context};
use kfx_script::validate::{
    CONDITIONS_COUNT, PARTY_TRIGGERS_COUNT, SCRIPT_VALUES_COUNT, TUNNELLER_TRIGGERS_COUNT,
    WIN_CONDITIONS_COUNT,
};
use kfx_script::{
    completions, format, highlight, hover, outline, positions, references, signature_help,
    signature_of, validate, ArgKind, ConstantProvider, Document, FormatOptions, OutlineKind,
    RefKind, Span, Tables,
};

pub struct DkLanguage;

impl Language for DkLanguage {
    fn id(&self) -> &'static str {
        "dk-script"
    }
    fn name(&self) -> &'static str {
        "DK script"
    }
    fn line_comment(&self) -> &'static str {
        "REM "
    }
    fn analyze(&self, text: &[u8]) -> Box<dyn Analysis> {
        let doc = Document::parse(text);
        let tokens = highlight(&doc)
            .into_iter()
            .map(|(s, c)| (s, HlClass::from(c)))
            .collect();
        let blocks = block_pairs(&doc);
        Box::new(DkAnalysis {
            doc,
            tokens,
            blocks,
        })
    }
    fn reference(&self) -> Vec<RefEntry> {
        let tables = Tables::get();
        let mut out = vec![];
        let mut seen = std::collections::HashSet::new();
        for c in &tables.commands {
            if !seen.insert(c.name.clone()) {
                continue;
            }
            out.push(entry(&c.name, false));
        }
        // commands only the level-version-0 table knows
        for c in &tables.dk1 {
            if seen.insert(c.name.clone()) {
                out.push(entry(&c.name, true));
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

fn entry(name: &str, dk1_only: bool) -> RefEntry {
    let d = docs::lookup(name);
    let sig = signature_of(name);
    let snippet_params = sig
        .as_ref()
        .map(|s| {
            s.params
                .iter()
                .filter(|p| !p.optional)
                .map(|p| p.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut note = None;
    if dk1_only {
        note = Some("Only with LEVEL_VERSION(0)".to_string());
    } else if d.is_none() {
        note = Some("No wiki page".to_string());
    }
    RefEntry {
        name: name.to_string(),
        category: docs::category(name).to_string(),
        signature: d
            .map(|d| d.signature.clone())
            .or_else(|| sig.as_ref().map(|s| s.label.clone()))
            .unwrap_or_else(|| name.to_string()),
        summary: d
            .map(|d| d.summary.clone())
            .or_else(|| {
                sig.as_ref()
                    .map(|s| s.doc.clone())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_default(),
        params: d
            .map(|d| {
                d.params
                    .iter()
                    .map(|p| (p.name.clone(), p.text.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        examples: d.map(|d| d.examples.clone()).unwrap_or_default(),
        link: d.map(|d| d.wiki_url(name)),
        snippet_params,
        note,
    }
}

fn block_pairs(doc: &Document) -> Vec<(Span, Span)> {
    let mut stack: Vec<Span> = vec![];
    let mut out = vec![];
    for (_, c) in doc.commands() {
        if c.desc.is_none() || !c.syntax_ok {
            continue;
        }
        if kfx_script::validate::is_if_command(&c.name) {
            stack.push(c.name_span);
        } else if c.name == "ENDIF" {
            if let Some(o) = stack.pop() {
                out.push((o, c.name_span));
            }
        }
    }
    out
}

pub struct DkAnalysis {
    pub doc: Document,
    tokens: Vec<(Span, HlClass)>,
    blocks: Vec<(Span, Span)>,
}

impl DkAnalysis {
    pub fn document(&self) -> &Document {
        &self.doc
    }
}

fn first_word(line: &str) -> String {
    line.trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_ascii_uppercase()
}

impl Analysis for DkAnalysis {
    fn tokens(&self) -> &[(Span, HlClass)] {
        &self.tokens
    }

    fn validate(&self, host: &dyn ConstantProvider) -> Validation {
        let r = validate(&self.doc, host);
        let s = &r.stats;
        let m = |label: &str, used: usize, limit: usize| Meter {
            label: label.to_string(),
            used,
            limit,
        };
        Validation {
            diagnostics: r.diagnostics,
            meters: vec![
                m("conditions", s.conditions, CONDITIONS_COUNT),
                m("script values", s.values, SCRIPT_VALUES_COUNT),
                m(
                    "parties",
                    s.parties,
                    kfx_script::validate::CREATURE_PARTYS_COUNT,
                ),
                m("party triggers", s.party_triggers, PARTY_TRIGGERS_COUNT),
                m(
                    "tunneller triggers",
                    s.tunneller_triggers,
                    TUNNELLER_TRIGGERS_COUNT,
                ),
                m("win conditions", s.win_conditions, WIN_CONDITIONS_COUNT),
                m("lose conditions", s.lose_conditions, WIN_CONDITIONS_COUNT),
            ],
        }
    }

    fn completions(&self, offset: usize, host: &dyn ConstantProvider) -> Vec<Completion> {
        completions(&self.doc, offset, host)
            .into_iter()
            .map(|c| {
                let mut c: Completion = c.into();
                if c.kind == CompletionKind::Command
                    && signature_of(&c.insert)
                        .map(|s| !s.params.is_empty())
                        .unwrap_or(false)
                {
                    c.callable = Some(true);
                }
                c
            })
            .collect()
    }

    fn completions_with(
        &self,
        offset: usize,
        host: &dyn ConstantProvider,
        strings: Option<&StringLookup>,
    ) -> Vec<Completion> {
        if let Some(lookup) = strings {
            if let Some(arg) = kfx_script::string_arg_at(&self.doc, offset) {
                let typed =
                    crate::strings::typed_prefix(&self.doc.src, arg.span, offset).to_string();
                return crate::strings::completions(&arg, &typed, lookup);
            }
        }
        self.completions(offset, host)
    }

    fn hover_with(&self, offset: usize, strings: Option<&StringLookup>) -> Option<HoverInfo> {
        if let Some(lookup) = strings {
            if let Some(arg) = kfx_script::string_arg_at(&self.doc, offset) {
                if let Some(h) = crate::strings::hover(&arg, lookup) {
                    return Some(h);
                }
            }
        }
        self.hover(offset)
    }

    fn string_arg_span(&self, offset: usize) -> Option<Span> {
        kfx_script::string_arg_at(&self.doc, offset).map(|a| a.span)
    }

    fn signature_help(&self, offset: usize) -> Option<SignatureHelp> {
        let s = signature_help(&self.doc, offset)?;
        let params = s
            .params
            .iter()
            .map(|p| {
                let detail = match (p.kind.name() == p.name, p.optional) {
                    (true, false) => None,
                    (true, true) => Some("optional".to_string()),
                    (false, o) => Some(format!(
                        "{}{}",
                        p.kind.name(),
                        if o { ", optional" } else { "" }
                    )),
                };
                let doc = docs::lookup(&s.command)
                    .and_then(|d| d.param(&p.name))
                    .map(|p| docs::short(&p.text))
                    .unwrap_or_default();
                SigParam {
                    name: p.name.clone(),
                    detail,
                    doc,
                    optional: p.optional,
                    label_range: p.label_range,
                }
            })
            .collect();
        Some(SignatureHelp {
            command: s.command,
            label: s.label,
            doc: s.doc,
            params,
            active: s.active,
        })
    }

    fn sound_arg_at(&self, offset: usize) -> Option<crate::sounds::SoundArg> {
        crate::sounds::sound_arg_at(&self.doc, offset)
    }

    fn hover(&self, offset: usize) -> Option<HoverInfo> {
        let h = hover(&self.doc, offset)?;
        let line = self.doc.line_at(offset).map(|l| &self.doc.lines[l]);
        if let Some(cmd) = line.and_then(|l| l.cmd.as_ref()) {
            if cmd.name_span.touches(offset) {
                if let Some(d) = docs::lookup(&cmd.name) {
                    return Some(HoverInfo {
                        span: cmd.name_span,
                        title: Some(d.signature.clone()),
                        summary: d.summary.clone(),
                        params: d
                            .params
                            .iter()
                            .map(|p| (p.name.clone(), docs::short(&p.text)))
                            .collect(),
                        examples: d.examples.iter().take(1).cloned().collect(),
                        link: Some(("Open wiki page".to_string(), d.wiki_url(&cmd.name))),
                    });
                }
            }
        }
        // Everything else: the service's markdown, flattened.
        let mut title = None;
        let mut body = String::new();
        let mut in_fence = false;
        for l in h.markdown.lines() {
            if l.starts_with("```") {
                in_fence = !in_fence;
            } else if in_fence {
                title.get_or_insert_with(String::new).push_str(l);
            } else {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&l.replace('`', ""));
            }
        }
        // "`X`: creature `creature`" -> drop a doubled word
        let mut words: Vec<&str> = vec![];
        for w in body.split(' ') {
            if words
                .last()
                .map(|p| p.eq_ignore_ascii_case(w))
                .unwrap_or(false)
                && !w.is_empty()
            {
                continue;
            }
            words.push(w);
        }
        Some(HoverInfo {
            span: h.span,
            title,
            summary: words.join(" "),
            ..Default::default()
        })
    }

    fn outline(&self) -> Vec<OutlineItem> {
        fn conv(it: &kfx_script::OutlineItem) -> OutlineItem {
            let (tag, tone) = match it.kind {
                OutlineKind::LevelVersion => ("VER", Tone::Muted),
                OutlineKind::IfBlock => ("IF", Tone::Keyword),
                OutlineKind::Party => ("PTY", Tone::Function),
                OutlineKind::Win => ("WIN", Tone::Good),
                OutlineKind::Lose => ("LOSE", Tone::Bad),
                OutlineKind::NewType => ("NEW", Tone::Constant),
            };
            OutlineItem {
                tag,
                tone,
                label: it.label.clone(),
                span: it.span,
                line: it.line,
                children: it.children.iter().map(conv).collect(),
            }
        }
        outline(&self.doc).iter().map(conv).collect()
    }

    fn map_references(&self) -> Vec<MapReference> {
        let mut out = vec![];
        for r in references(&self.doc) {
            let target = match r.kind {
                RefKind::ActionPoint => MapTarget::ActionPoint(r.value),
                RefKind::HeroGate => MapTarget::HeroGate(r.value),
                _ => continue,
            };
            out.push(MapReference {
                span: r.span,
                target,
                command: r.command.clone(),
                line: self.doc.line_of(r.span.start),
            });
        }
        for p in positions(&self.doc) {
            let target = if p.slab {
                MapTarget::Slab { x: p.x, y: p.y }
            } else {
                MapTarget::Subtile { x: p.x, y: p.y }
            };
            out.push(MapReference {
                span: p.x_span.join(p.y_span),
                target,
                command: p.command.clone(),
                line: self.doc.line_of(p.x_span.start),
            });
        }
        out.sort_by_key(|r| r.span.start);
        out
    }

    fn block_match(&self, offset: usize) -> Option<(Span, Span)> {
        // innermost pair whose opener or closer is touched
        self.blocks
            .iter()
            .filter(|(a, b)| a.touches(offset) || b.touches(offset))
            .min_by_key(|(a, b)| b.end - a.start)
            .copied()
    }

    fn pick_target(&self, offset: usize) -> Option<PickKind> {
        let Context::Arg { cmd, slot, .. } = context_at(&self.doc, offset) else {
            return None;
        };
        match arg_kind_at(cmd, slot).0 {
            ArgKind::ApNumber => Some(PickKind::ActionPoint),
            ArgKind::HeroGate => Some(PickKind::HeroGate),
            ArgKind::Location => Some(PickKind::Location),
            ArgKind::SubtileX | ArgKind::SubtileY => Some(PickKind::Subtile),
            ArgKind::SlabX | ArgKind::SlabY => Some(PickKind::Slab),
            _ => None,
        }
    }

    fn pick_replacements(&self, offset: usize, value: PickValue) -> Vec<(Span, String)> {
        let Context::Arg { cmd, slot, word } = context_at(&self.doc, offset) else {
            return vec![];
        };
        let (kind, _) = arg_kind_at(cmd, slot);
        let single = |text: String| -> Vec<(Span, String)> {
            let span = cmd
                .args
                .iter()
                .find(|a| a.slot == slot && a.func.is_none() && a.span.touches(offset))
                .map(|a| a.span)
                .unwrap_or(word);
            vec![(span, text)]
        };
        match (kind, value) {
            (ArgKind::ApNumber, PickValue::ActionPoint(n))
            | (ArgKind::Location, PickValue::ActionPoint(n)) => single(n.to_string()),
            (ArgKind::HeroGate, PickValue::HeroGate(n)) => single(n.abs().to_string()),
            (ArgKind::Location, PickValue::HeroGate(n)) => single((-n.abs()).to_string()),
            (ArgKind::SubtileX | ArgKind::SubtileY, PickValue::Subtile { x, y })
            | (ArgKind::SlabX | ArgKind::SlabY, PickValue::Slab { x, y }) => {
                let slab = matches!(kind, ArgKind::SlabX | ArgKind::SlabY);
                if let Some(p) = positions(&self.doc).into_iter().find(|p| {
                    p.slab == slab && (p.x_span.touches(offset) || p.y_span.touches(offset))
                }) {
                    vec![(p.x_span, x.to_string()), (p.y_span, y.to_string())]
                } else {
                    single(format!("{x}, {y}"))
                }
            }
            _ => vec![],
        }
    }

    fn arg_nav(&self, offset: usize) -> Option<ArgNav> {
        let Context::Arg { cmd, .. } = context_at(&self.doc, offset) else {
            return None;
        };
        Some(ArgNav {
            args: cmd.args.iter().map(|a| a.span).collect(),
            after: cmd.close.map(|c| c.end).unwrap_or(cmd.span.end),
        })
    }

    fn block_delta(&self, line: &str) -> i32 {
        let w = first_word(line);
        if kfx_script::validate::is_if_command(&w) {
            1
        } else if w == "ENDIF" {
            -1
        } else {
            0
        }
    }

    fn format(&self, indent: &str) -> Option<Vec<u8>> {
        Some(format(
            &self.doc,
            &FormatOptions {
                indent: indent.to_string(),
                ..Default::default()
            },
        ))
    }
}
