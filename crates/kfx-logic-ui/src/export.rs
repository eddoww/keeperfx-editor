//! [`LogicDoc`] -> DK script text.
//!
//! Unedited nodes are written verbatim (their `raw` line); edited ones are regenerated in the
//! canonical form `NAME(a,b,c)`, indented with one tab per nesting level. Disabled events are
//! written as their normal lines prefixed with [`OFF_MARK`] (the game reads them as comments).
use crate::model::*;
use crate::syntax::{canonical, latin1_bytes};

struct Out<'a> {
    lines: Vec<String>,
    version: i64,
    doc: &'a LogicDoc,
}

fn indent(depth: usize) -> String {
    "\t".repeat(depth)
}

fn with_trail(s: String, trail: &Option<String>) -> String {
    match trail {
        Some(t) => format!("{s} {t}"),
        None => s,
    }
}

impl Out<'_> {
    fn trivia(&mut self, lines: &[String]) {
        self.lines.extend(lines.iter().cloned());
    }

    fn action(&mut self, a: &Action, depth: usize) {
        self.trivia(&a.pre);
        if a.reusable {
            match &a.reusable_raw {
                Some(r) => self.lines.push(r.clone()),
                None => self
                    .lines
                    .push(format!("{}NEXT_COMMAND_REUSABLE", indent(depth))),
            }
        }
        match &a.raw {
            Some(r) => self.lines.push(r.clone()),
            None => self.lines.push(with_trail(
                format!("{}{}", indent(depth), canonical(&a.cmd, self.version)),
                &a.trail,
            )),
        }
    }

    fn party(&mut self, p: &Party) {
        self.trivia(&p.pre);
        match &p.raw {
            Some(r) => self.lines.push(r.clone()),
            None => self
                .lines
                .push(with_trail(format!("CREATE_PARTY({})", p.name), &p.trail)),
        }
        for m in &p.members {
            self.trivia(&m.pre);
            match &m.raw {
                Some(r) => self.lines.push(r.clone()),
                None => self.lines.push(with_trail(
                    format!(
                        "\tADD_TO_PARTY({},{},{},{},{},{})",
                        p.name, m.creature, m.level, m.gold, m.objective, m.countdown
                    ),
                    &m.trail,
                )),
            }
        }
    }

    fn event(&mut self, e: &Event, depth: usize) {
        if !e.enabled {
            self.trivia(&e.pre);
            let mut sub = Out {
                lines: vec![],
                version: self.version,
                doc: self.doc,
            };
            let mut copy = e.clone();
            copy.pre.clear();
            copy.enabled = true;
            sub.event(&copy, depth);
            for l in sub.lines {
                for phys in l.split('\n') {
                    let phys = phys.trim_end_matches('\r');
                    self.lines.push(format!("{OFF_MARK}{phys}"));
                }
            }
            return;
        }
        self.trivia(&e.pre);
        let n = e.conditions.len();
        for (i, c) in e.conditions.iter().enumerate() {
            self.trivia(&c.pre);
            match &c.raw {
                Some(r) => self.lines.push(r.clone()),
                None => self.lines.push(with_trail(
                    format!("{}{}", indent(depth + i), canonical(&c.cmd, self.version)),
                    &c.trail,
                )),
            }
        }
        let inner = depth + n;
        for node in &e.body {
            self.node(node, inner);
        }
        self.trivia(&e.tail);
        for i in (0..n).rev() {
            let c = &e.conditions[i];
            if c.unclosed {
                continue;
            }
            match &c.endif_raw {
                Some(r) => self.lines.push(r.clone()),
                None => self.lines.push(format!("{}ENDIF", indent(depth + i))),
            }
        }
    }

    fn node(&mut self, n: &Node, depth: usize) {
        match n {
            Node::Action(a) => self.action(a, depth),
            Node::Event(e) => self.event(e, depth),
            Node::Party(p) => self.party(p),
        }
    }
}

/// Export as text (Latin-1 chars, see [`export_bytes`] for the file bytes).
pub fn export(doc: &LogicDoc) -> String {
    let mut o = Out {
        lines: vec![],
        version: doc.version,
        doc,
    };
    let mut vars_written = doc.variables.is_empty();
    for n in &doc.nodes {
        if !vars_written {
            // aliases go after the header comments, before the first command
            o.trivia(n.pre());
            for v in &doc.variables {
                o.lines
                    .push(format!("{VAR_MARK}{} = {}.{}", v.name, v.player, v.var()));
            }
            vars_written = true;
            let mut copy = n.clone();
            copy.pre_mut().clear();
            o.node(&copy, 0);
            continue;
        }
        o.node(n, 0);
    }
    o.trivia(&doc.tail);
    if !vars_written {
        for v in &doc.variables {
            o.lines
                .push(format!("{VAR_MARK}{} = {}.{}", v.name, v.player, v.var()));
        }
    }
    let eol = if doc.crlf { "\r\n" } else { "\n" };
    if o.lines.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    for (i, l) in o.lines.iter().enumerate() {
        if i > 0 {
            s.push_str(eol);
        }
        s.push_str(l);
    }
    if doc.final_newline {
        s.push_str(eol);
    }
    s
}

/// Export as file bytes (Latin-1).
pub fn export_bytes(doc: &LogicDoc) -> Vec<u8> {
    latin1_bytes(&export(doc))
}

/// Text of one event as it would be written (for previews and the "open in text" view).
pub fn event_text(doc: &LogicDoc, e: &Event) -> String {
    let mut o = Out {
        lines: vec![],
        version: doc.version,
        doc,
    };
    let mut copy = e.clone();
    copy.pre.clear();
    o.event(&copy, 0);
    o.lines.join("\n")
}

/// The line a single action would produce (no comments).
pub fn action_line(doc: &LogicDoc, a: &Action) -> String {
    match &a.raw {
        Some(r) => r.trim().to_string(),
        None => canonical(&a.cmd, doc.version),
    }
}

/// The line a condition would produce.
pub fn condition_line(doc: &LogicDoc, c: &Condition) -> String {
    match &c.raw {
        Some(r) => r.trim().to_string(),
        None => canonical(&c.cmd, doc.version),
    }
}
