//! DK script -> [`LogicDoc`].
//!
//! * `IF*` ... `ENDIF` blocks become [`Event`]s (directly nested IFs with nothing between them
//!   become one event with several conditions); commands outside any block are grouped into
//!   "level start" events, split at blank lines.
//! * Top-level `CREATE_PARTY` with the `ADD_TO_PARTY` lines that follow it becomes a [`Party`].
//! * `NEXT_COMMAND_REUSABLE` directly followed by a command becomes that action's `reusable` flag.
//! * Comments and blank lines are attached to the next node (`pre`) or the block end (`tail`).
//! * Anything else that is not a typed command (unknown commands, syntax errors, text that would
//!   not regenerate identically) becomes an opaque action holding the line verbatim.
//!
//! After building the model the importer exports it again and compares both scripts command by
//! command ([`crate::compare`]); when they differ, or the block structure is broken (an `ENDIF`
//! without `IF`, an `IF` never closed, an `IF` line the game discards), the document is marked
//! [`LogicDoc::read_only`] and the editor never rewrites it.
use crate::model::*;
use crate::syntax::{arg_texts, canonical, command_key, is_if, line_key};
use kfx_script::{Document, LineKind};

struct Frame {
    ev: Event,
    /// Conditions of `ev` whose ENDIF has not been seen.
    open: usize,
}

struct Importer<'a> {
    doc: &'a Document,
    out: LogicDoc,
    stack: Vec<Frame>,
    top: Vec<Node>,
    /// Top-level "level start" group being filled.
    group: Option<Event>,
    pending: Vec<String>,
    problems: Vec<String>,
    reusable: Option<(String, usize)>,
}

fn line_text(doc: &Document, li: usize) -> String {
    let l = &doc.lines[li];
    let mut t = doc.text_lossy(l.span);
    if t.ends_with('\n') {
        t.pop();
    }
    if t.ends_with('\r') {
        t.pop();
    }
    t
}

/// Split a REM trailing comment off a command line: (command part, comment).
fn split_trail(doc: &Document, li: usize) -> Option<String> {
    let l = &doc.lines[li];
    let r = l.rem?;
    let t = doc.text_lossy(r);
    Some(t.trim_end_matches('\r').to_string())
}

impl<'a> Importer<'a> {
    fn take_pre(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    fn body(&mut self) -> &mut Vec<Node> {
        match self.stack.last_mut() {
            Some(f) => &mut f.ev.body,
            None => &mut self.top,
        }
    }

    fn flush_group(&mut self) {
        if let Some(g) = self.group.take() {
            self.top.push(Node::Event(g));
        }
    }

    /// Add an action to the current body (or the top-level group).
    fn push_action(&mut self, mut a: Action) {
        if let Some((raw, _)) = self.reusable.take() {
            a.reusable = true;
            a.reusable_raw = Some(raw);
        }
        if self.stack.is_empty() {
            // top level: group "level start" actions, a blank line starts a new group
            let starts_new = a
                .pre
                .iter()
                .any(|l| l.trim().is_empty() || l.starts_with(OFF_MARK.trim_end()));
            if starts_new || self.group.is_none() {
                self.flush_group();
                let pre = std::mem::take(&mut a.pre);
                let id = self.out.new_id();
                self.group = Some(Event {
                    id,
                    pre,
                    conditions: vec![],
                    body: vec![],
                    tail: vec![],
                    enabled: true,
                    line: a.line,
                });
            }
            self.group.as_mut().unwrap().body.push(Node::Action(a));
        } else {
            self.unmerge();
            self.body().push(Node::Action(a));
        }
    }

    /// A chain event whose inner conditions were closed but the outer ones get more content:
    /// split it into an outer event holding the inner one.
    fn unmerge(&mut self) {
        let Some(f) = self.stack.last_mut() else {
            return;
        };
        if f.open == f.ev.conditions.len() || f.open == 0 {
            return;
        }
        let inner_conds: Vec<Condition> = f.ev.conditions.split_off(f.open);
        let mut inner_pre = vec![];
        let mut conds = inner_conds;
        std::mem::swap(&mut inner_pre, &mut conds[0].pre);
        let id = self.out.new_id();
        let line = conds[0].line;
        let inner = Event {
            id,
            pre: inner_pre,
            conditions: conds,
            body: std::mem::take(&mut f.ev.body),
            tail: std::mem::take(&mut f.ev.tail),
            enabled: true,
            line,
        };
        f.ev.body.push(Node::Event(inner));
    }

    fn action_from(&mut self, li: usize, opaque: bool) -> Action {
        let doc = self.doc;
        let line = &doc.lines[li];
        let mut pre = self.take_pre();
        let mut raw = line_text(doc, li);
        if let (Some(b), Some(_)) = (line.block, line.cmd.as_ref()) {
            // a leading block comment goes into `pre` so the command can be regenerated
            let cut = b.end - line.span.start;
            pre.push(raw[..cut].to_string());
            raw = raw[cut..].to_string();
        }
        let (cmd, opaque) = match line.cmd.as_ref() {
            Some(c) if !opaque && c.desc.is_some() && c.syntax_ok => {
                let cmd = Cmd {
                    name: c.name.clone(),
                    args: arg_texts(doc, c),
                };
                let ok = line_key(&canonical(&cmd, doc.version), doc.version).as_deref()
                    == Some(command_key(doc, c).as_str());
                (cmd, !ok)
            }
            Some(c) => (
                Cmd {
                    name: c.name.clone(),
                    args: vec![],
                },
                true,
            ),
            None => (Cmd::default(), true),
        };
        let id = self.out.new_id();
        Action {
            id,
            pre,
            cmd,
            reusable: false,
            reusable_raw: None,
            raw: Some(raw),
            trail: split_trail(doc, li),
            opaque,
            line: doc.line_of(line.span.start),
        }
    }

    fn line(&mut self, li: usize) {
        let doc = self.doc;
        let line = &doc.lines[li];
        let text = line_text(doc, li);
        match line.kind {
            LineKind::Blank | LineKind::Comment => {
                if self.reusable.is_some() {
                    // NEXT_COMMAND_REUSABLE not followed by a command: it does nothing, keep it
                    self.flush_reusable();
                }
                self.pending.push(text);
                return;
            }
            LineKind::Error => {
                self.flush_reusable();
                let a = self.action_from(li, true);
                self.push_action(a);
                return;
            }
            LineKind::Command => {}
        }
        let c = line.cmd.as_ref().unwrap();
        let valid = c.desc.is_some() && c.syntax_ok;
        let lno = doc.line_of(line.span.start);
        if c.name == "NEXT_COMMAND_REUSABLE" && valid {
            self.flush_reusable();
            let next_is_cmd = doc.lines.get(li + 1).and_then(|l| l.cmd.as_ref()).map(|n| {
                n.desc.is_some()
                    && n.syntax_ok
                    && !is_if(&n.name)
                    && n.name != "ENDIF"
                    && n.name != "NEXT_COMMAND_REUSABLE"
                    && n.name != "CREATE_PARTY"
                    && n.name != "ADD_TO_PARTY"
            });
            if next_is_cmd == Some(true) && self.pending.is_empty() && line.block.is_none() {
                self.reusable = Some((text, lno));
                return;
            }
            let a = self.action_from(li, false);
            self.push_action(a);
            return;
        }
        if is_if(&c.name) && valid {
            self.flush_reusable();
            self.flush_group();
            let mut pre = self.take_pre();
            let mut raw = text;
            if let Some(b) = line.block {
                let cut = b.end - line.span.start;
                pre.push(raw[..cut].to_string());
                raw = raw[cut..].to_string();
            }
            let cmd = Cmd {
                name: c.name.clone(),
                args: arg_texts(doc, c),
            };
            if line_key(&canonical(&cmd, doc.version), doc.version).as_deref()
                != Some(command_key(doc, c).as_str())
            {
                self.problems.push(format!(
                    "line {lno}: the condition {} cannot be regenerated",
                    c.name
                ));
            }
            let id = self.out.new_id();
            let cond = Condition {
                id,
                pre: vec![],
                cmd,
                raw: Some(raw),
                trail: split_trail(doc, li),
                endif_raw: None,
                unclosed: true,
                line: lno,
                endif_line: 0,
            };
            // merge into the current event when it has no body yet and nothing was closed
            if let Some(f) = self.stack.last_mut() {
                if f.ev.body.is_empty() && f.ev.tail.is_empty() && f.open == f.ev.conditions.len() {
                    let mut cond = cond;
                    cond.pre = pre;
                    f.ev.conditions.push(cond);
                    f.open += 1;
                    return;
                }
                self.unmerge();
            }
            let id = self.out.new_id();
            self.stack.push(Frame {
                ev: Event {
                    id,
                    pre,
                    conditions: vec![cond],
                    body: vec![],
                    tail: vec![],
                    enabled: true,
                    line: lno,
                },
                open: 1,
            });
            return;
        }
        if is_if(&c.name) {
            self.problems.push(format!(
                "line {lno}: the game discards this {} line, so the block \
                    structure differs from what it looks like",
                c.name
            ));
        }
        if c.name == "ENDIF" && valid && !self.stack.is_empty() {
            self.flush_reusable();
            let pending = self.take_pre();
            let f = self.stack.last_mut().unwrap();
            if f.open < f.ev.conditions.len() && !pending.is_empty() {
                // trivia between two ENDIFs of a chain: split the chain first
                self.pending = pending;
                self.unmerge();
                let pending = self.take_pre();
                let f = self.stack.last_mut().unwrap();
                f.ev.tail = pending;
            } else if f.open == f.ev.conditions.len() {
                f.ev.tail = pending;
            } else {
                // ENDIF right after the previous ENDIF of the chain: nothing to keep
                debug_assert!(pending.is_empty());
            }
            let f = self.stack.last_mut().unwrap();
            f.open -= 1;
            let ci = f.open;
            f.ev.conditions[ci].endif_raw = Some(text);
            f.ev.conditions[ci].unclosed = false;
            f.ev.conditions[ci].endif_line = lno;
            if f.open == 0 {
                let f = self.stack.pop().unwrap();
                let ev = f.ev;
                self.body().push(Node::Event(ev));
            }
            return;
        }
        if c.name == "ENDIF" && valid {
            self.problems.push(format!("line {lno}: ENDIF without IF"));
        }
        // ordinary command
        if self.stack.is_empty() && valid && c.name == "CREATE_PARTY" && self.reusable.is_none() {
            let cmd = Cmd {
                name: c.name.clone(),
                args: arg_texts(doc, c),
            };
            if line_key(&canonical(&cmd, doc.version), doc.version).as_deref()
                == Some(command_key(doc, c).as_str())
                && line.block.is_none()
            {
                self.flush_group();
                let pre = self.take_pre();
                let id = self.out.new_id();
                self.top.push(Node::Party(Party {
                    id,
                    pre,
                    name: c.text(0).to_ascii_uppercase(),
                    members: vec![],
                    raw: Some(text),
                    trail: split_trail(doc, li),
                    line: lno,
                }));
                return;
            }
        }
        if self.stack.is_empty()
            && valid
            && c.name == "ADD_TO_PARTY"
            && self.group.is_none()
            && self.reusable.is_none()
            && line.block.is_none()
        {
            if let Some(Node::Party(p)) = self.top.last() {
                let cmd = Cmd {
                    name: c.name.clone(),
                    args: arg_texts(doc, c),
                };
                let canon_ok = line_key(&canonical(&cmd, doc.version), doc.version).as_deref()
                    == Some(command_key(doc, c).as_str());
                if canon_ok && c.text(0).eq_ignore_ascii_case(&p.name) && cmd.args.len() == 6 {
                    let pre = self.take_pre();
                    let id = self.out.new_id();
                    let m = Member {
                        id,
                        pre,
                        creature: cmd.args[1].clone(),
                        level: cmd.args[2].clone(),
                        gold: cmd.args[3].clone(),
                        objective: cmd.args[4].clone(),
                        countdown: cmd.args[5].clone(),
                        raw: Some(text),
                        trail: split_trail(doc, li),
                        line: lno,
                    };
                    if let Some(Node::Party(p)) = self.top.last_mut() {
                        p.members.push(m);
                    }
                    return;
                }
            }
        }
        let a = self.action_from(li, false);
        self.push_action(a);
    }

    fn flush_reusable(&mut self) {
        if let Some((raw, line)) = self.reusable.take() {
            let pre = self.take_pre();
            let id = self.out.new_id();
            let a = Action {
                id,
                pre,
                cmd: Cmd::new("NEXT_COMMAND_REUSABLE", &[]),
                reusable: false,
                reusable_raw: None,
                raw: Some(raw),
                trail: None,
                opaque: false,
                line,
            };
            self.push_action(a);
        }
    }

    fn finish(mut self) -> (LogicDoc, Vec<String>) {
        self.flush_reusable();
        while let Some(mut f) = self.stack.pop() {
            self.problems.push(format!(
                "line {}: IF block never closed (missing ENDIF)",
                f.ev.conditions
                    .get(f.open.saturating_sub(1))
                    .map(|c| c.line)
                    .unwrap_or(f.ev.line)
            ));
            f.ev.tail = self.take_pre();
            let ev = f.ev;
            self.body().push(Node::Event(ev));
        }
        self.flush_group();
        let mut out = self.out;
        out.nodes = self.top;
        out.tail = self.pending;
        (out, self.problems)
    }
}

/// Pull `REM kfx-var` lines out of trivia into `variables`, and turn `REM kfx-off` runs into
/// disabled events.
fn post_process(doc: &mut LogicDoc) {
    fn vars_in(lines: &mut Vec<String>, out: &mut Vec<VarAlias>) {
        lines.retain(|l| {
            let t = l.trim_start().trim_end_matches('\r');
            if let Some(rest) = t.strip_prefix(VAR_MARK) {
                if let Some((name, target)) = rest.split_once('=') {
                    if let Some((p, v)) = target.trim().split_once('.') {
                        if let Some((family, index)) = VarFamily::parse(v.trim()) {
                            out.push(VarAlias {
                                name: name.trim().to_string(),
                                player: p.trim().to_ascii_uppercase(),
                                family,
                                index,
                            });
                            return false;
                        }
                    }
                }
            }
            true
        });
    }
    let mut vars = vec![];
    fn walk(nodes: &mut [Node], vars: &mut Vec<VarAlias>) {
        for n in nodes {
            vars_in(n.pre_mut(), vars);
            if let Node::Event(e) = n {
                walk(&mut e.body, vars);
                vars_in(&mut e.tail, vars);
            }
        }
    }
    walk(&mut doc.nodes, &mut vars);
    vars_in(&mut doc.tail, &mut vars);
    doc.variables = vars;

    let mut next = doc.next_id;
    let (nodes, tail) = (&mut doc.nodes, &mut doc.tail);
    disabled_in(nodes, tail, &mut next);
    doc.next_id = next;
}

/// Turn `REM kfx-off` runs found in the trivia of `nodes` (and `tail`) into disabled events,
/// recursively.
fn disabled_in(nodes: &mut Vec<Node>, tail: &mut Vec<String>, next_id: &mut NodeId) {
    let off = OFF_MARK.trim_end();
    let is_off = |l: &String| {
        l.starts_with(off) && (l.len() == off.len() || l[off.len()..].starts_with(' '))
    };
    let mut i = 0;
    while i <= nodes.len() {
        if i < nodes.len() {
            if let Node::Event(e) = &mut nodes[i] {
                let (b, t) = (&mut e.body, &mut e.tail);
                disabled_in(b, t, next_id);
            }
        }
        let lines: Vec<String> = if i < nodes.len() {
            nodes[i].pre().clone()
        } else {
            tail.clone()
        };
        let Some(a) = lines.iter().position(|l| is_off(l)) else {
            i += 1;
            continue;
        };
        let mut b = a;
        while b < lines.len() && is_off(&lines[b]) {
            b += 1;
        }
        let text: Vec<String> = lines[a..b]
            .iter()
            .map(|l| l.get(OFF_MARK.len()..).unwrap_or("").to_string())
            .collect();
        let joined = text.join("\n") + "\n";
        let mut sub = import_inner(&crate::syntax::latin1_bytes(&joined), *next_id);
        let ok = sub.read_only.is_none()
            && !sub.nodes.is_empty()
            && sub.nodes.iter().all(|n| matches!(n, Node::Event(_)))
            && sub.tail.is_empty();
        if !ok {
            i += 1;
            continue;
        }
        let mut sub_next = sub.next_id;
        {
            let (sn, st) = (&mut sub.nodes, &mut sub.tail);
            disabled_in(sn, st, &mut sub_next);
        }
        *next_id = sub_next;
        let mut new_nodes: Vec<Node> = sub.nodes;
        let before: Vec<String> = lines[..a].to_vec();
        let after: Vec<String> = lines[b..].to_vec();
        if let Some(Node::Event(e)) = new_nodes.first_mut() {
            let mut p = before;
            p.append(&mut e.pre);
            e.pre = p;
        }
        for n in &mut new_nodes {
            if let Node::Event(e) = n {
                e.enabled = false;
            }
        }
        let count = new_nodes.len();
        if i < nodes.len() {
            *nodes[i].pre_mut() = after;
        } else {
            *tail = after;
        }
        for (k, n) in new_nodes.into_iter().enumerate() {
            nodes.insert(i + k, n);
        }
        i += count;
    }
}

fn import_inner(src: &[u8], first_id: NodeId) -> LogicDoc {
    let doc = Document::parse(src);
    let mut out = LogicDoc {
        next_id: first_id,
        version: doc.version,
        ..LogicDoc::default()
    };
    out.crlf = src.windows(2).any(|w| w == b"\r\n");
    out.final_newline = src.is_empty() || src.ends_with(b"\n");
    let mut imp = Importer {
        doc: &doc,
        out,
        stack: vec![],
        top: vec![],
        group: None,
        pending: vec![],
        problems: vec![],
        reusable: None,
    };
    for li in 0..doc.lines.len() {
        imp.line(li);
    }
    let (mut out, problems) = imp.finish();
    // A file ending with a newline yields no empty last logical line; one ending without keeps it.
    if !problems.is_empty() {
        out.read_only = Some(problems.join("; "));
    }
    out
}

/// Import a DK script. Never fails: see [`LogicDoc::read_only`] for scripts that cannot be
/// edited faithfully.
pub fn import(src: &[u8]) -> LogicDoc {
    let mut doc = import_inner(src, 1);
    post_process(&mut doc);
    if doc.read_only.is_none() {
        let back = crate::export::export_bytes(&doc);
        if let Err(diff) = crate::compare::compare(src, &back) {
            doc.read_only = Some(format!(
                "the editor would not write this script back faithfully ({diff})"
            ));
        }
    }
    doc
}
