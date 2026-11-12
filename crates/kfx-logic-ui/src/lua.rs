//! [`LogicDoc`] -> Lua level script on the shipped trigger library
//! (`fxdata/lua/triggers/Events.lua`, `TriggerSystem.lua`).
//!
//! Mapping:
//!
//! | model | Lua |
//! |---|---|
//! | level start event | statements in `OnGameStart()` |
//! | `IF(p, GAME_TURN >= n)` alone | `RegisterTimerEvent(f, n, false)` (counted from registration, i.e. game start) |
//! | `IF_ACTION_POINT(n, p)` alone | `RegisterOnActionPointEvent(f, n, p)` |
//! | other `IF(p, VAR op x)` / `IF_ACTION_POINT` / AND chains | `RegisterOnConditionEvent(f, cond)`, `cond` returns `p.VAR op x and ...` (`Player` fields, `src/lua_api_player.c`) |
//! | nested event | registered from the parent's function when the parent fires |
//! | reusable action | its own `CreateTrigger("GameTick", f)` + `TriggerAddCondition` (runs every turn the condition holds, like `TrgF_REUSABLE`) |
//! | action | the call [`kfx_script::lua::convert`] produces for that line; otherwise `RunDKScriptCommand("...")` (`src/lua_api.c:2606`: runs the line as an unconditional script command) |
//!
//! Flagged as having no equivalent (kept as comments): conditions other than `IF` on a single
//! player and `IF_ACTION_POINT` (`IF_AVAILABLE`, `IF_CONTROLS`, `IF_SLAB_*`, `IF_ALLIED`, player
//! ranges such as `ALL_PLAYERS`, `RANDOM(...)` values), load-time commands (`LEVEL_VERSION`,
//! `NEW_*_TYPE`), opaque lines and disabled events. Approximations are flagged too.
use crate::model::*;
use crate::syntax::{canonical, latin1_bytes, num};
use kfx_script::lua::convert::{convert, ConvertOptions};
use kfx_script::lua::ApiIndex;
use kfx_script::{ConstantProvider, Document, Tables};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LuaFlagKind {
    /// Written, but the Lua behaves slightly differently.
    Approximate,
    /// No Lua equivalent: kept as a comment.
    NoEquivalent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LuaFlag {
    pub node: NodeId,
    pub kind: LuaFlagKind,
    pub what: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct LuaExport {
    pub lua: String,
    pub flags: Vec<LuaFlag>,
}

struct Gen<'a> {
    doc: &'a LogicDoc,
    index: &'a ApiIndex,
    provider: &'a dyn ConstantProvider,
    funcs: Vec<String>,
    flags: Vec<LuaFlag>,
    n: usize,
}

fn lua_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Gen<'_> {
    fn flag(&mut self, node: NodeId, kind: LuaFlagKind, what: &str, reason: &str) {
        self.flags.push(LuaFlag {
            node,
            kind,
            what: what.to_string(),
            reason: reason.to_string(),
        });
    }

    fn player(&self, p: &str) -> Option<String> {
        let u = p.to_ascii_uppercase();
        self.index
            .globals
            .get(&u)
            .filter(|g| g.ty == "Player")
            .map(|_| u)
    }

    /// Lua expression of a condition, `Err(reason)` when there is none.
    fn cond_expr(&self, c: &Condition) -> Result<String, String> {
        let m = &c.cmd;
        match m.name.as_str() {
            "IF_ACTION_POINT" => {
                let ap = num(m.arg(0))
                    .filter(|n| *n > 0)
                    .ok_or("action point is not a plain number")?;
                let p = self
                    .player(m.arg(1))
                    .ok_or_else(|| format!("'{}' is not a single player", m.arg(1)))?;
                Ok(format!("IsActionpointActivatedByPlayer({p}, {ap})"))
            }
            "IF" => {
                let p = self
                    .player(m.arg(0))
                    .ok_or_else(|| format!("'{}' is not a single player", m.arg(0)))?;
                let var = m.arg(1).to_ascii_uppercase();
                if !var.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    return Err(format!("variable '{var}' has no Player field"));
                }
                let op = match m.arg(2) {
                    "==" | "=" => "==",
                    "!=" | "<>" => "~=",
                    o @ ("<" | ">" | "<=" | ">=") => o,
                    o => return Err(format!("comparison '{o}' is not supported")),
                };
                let rhs = if !m.arg(4).is_empty() {
                    let p2 = self
                        .player(m.arg(3))
                        .ok_or_else(|| format!("'{}' is not a single player", m.arg(3)))?;
                    let v2 = m.arg(4).to_ascii_uppercase();
                    if !v2.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                        return Err(format!("variable '{v2}' has no Player field"));
                    }
                    format!("{p2}.{v2}")
                } else {
                    num(m.arg(3))
                        .ok_or("compared value is not a plain number")?
                        .to_string()
                };
                Ok(format!("{p}.{var} {op} {rhs}"))
            }
            other => Err(format!("{other} has no Lua condition equivalent")),
        }
    }

    /// Lua statement of an action, plus whether it is only approximate.
    fn action_stmt(&mut self, a: &Action) -> Result<(String, Option<String>), String> {
        if a.opaque {
            return Err("the line is not a valid command".into());
        }
        if kfx_script::tables::is_preloaded(&a.cmd.name) {
            return Err("runs while the level loads, before Lua".into());
        }
        if a.cmd.name == "NEXT_COMMAND_REUSABLE" {
            return Err("has no effect here".into());
        }
        let line = canonical(&a.cmd, self.doc.version);
        let src = if self.doc.version > 0 {
            format!("LEVEL_VERSION({})\n{line}\n", self.doc.version)
        } else {
            format!("{line}\n")
        };
        let d = Document::parse(&latin1_bytes(&src));
        let conv = convert(&d, self.index, self.provider, &ConvertOptions::default());
        let tag = format!("   -- {}", a.cmd.name);
        if let Some(l) = conv
            .lua
            .lines()
            .find(|l| l.trim_end().ends_with(&tag) && !l.trim_start().starts_with("--"))
        {
            return Ok((l.trim().trim_end_matches(&tag).trim().to_string(), None));
        }
        let why = conv
            .notes
            .iter()
            .find(|n| n.command == a.cmd.name)
            .map(|n| n.reason.clone())
            .unwrap_or_default();
        if self.index.function("RunDKScriptCommand").is_some()
            && Tables::get().find_full(&a.cmd.name).is_some()
        {
            return Ok((
                format!("RunDKScriptCommand({})", lua_str(&line)),
                Some(format!("through RunDKScriptCommand ({why})")),
            ));
        }
        Err(why)
    }

    fn body(&mut self, nodes: &[Node], conds: &[String], out: &mut Vec<String>) {
        for n in nodes {
            match n {
                Node::Action(a) => match self.action_stmt(a) {
                    Ok((s, approx)) => {
                        if let Some(r) = approx {
                            self.flag(a.id, LuaFlagKind::Approximate, &a.cmd.name, &r);
                        }
                        if a.reusable && conds.is_empty() {
                            self.flag(
                                a.id,
                                LuaFlagKind::Approximate,
                                &a.cmd.name,
                                "an unconditional reusable command runs every turn in DK; \
                                    written once",
                            );
                        }
                        if a.reusable && !conds.is_empty() {
                            self.n += 1;
                            let f = format!("KfxReusable_{}", self.n);
                            let c = format!("KfxReusableCond_{}", self.n);
                            self.funcs.push(format!(
                                "function {f}()\n    {s}\nend\n\nfunction \
                                    {c}()\n    return {}\nend\n",
                                conds.join(" and ")
                            ));
                            out.push(format!(
                                "TriggerAddCondition(CreateTrigger(\"GameTick\", {f}), \
                                    {c})   -- every turn the condition holds (NEXT_COMMAND_REUSABLE)"
                            ));
                        } else {
                            out.push(s);
                        }
                    }
                    Err(r) => {
                        self.flag(a.id, LuaFlagKind::NoEquivalent, &a.cmd.name, &r);
                        out.push(format!(
                            "-- NO LUA EQUIVALENT ({r}): {}",
                            crate::export::action_line(self.doc, a)
                        ));
                    }
                },
                Node::Party(p) => {
                    out.push(format!("CreateParty({})", lua_str(&p.name)));
                    for m in &p.members {
                        out.push(format!(
                            "AddToParty({}, {}, {}, {}, {}, {})",
                            lua_str(&p.name),
                            lua_str(&m.creature.to_ascii_uppercase()),
                            m.level,
                            m.gold,
                            lua_str(&m.objective.to_ascii_uppercase()),
                            m.countdown
                        ));
                    }
                }
                Node::Event(e) => self.event(e, conds, out),
            }
        }
    }

    fn event(&mut self, e: &Event, outer: &[String], out: &mut Vec<String>) {
        if !e.enabled {
            self.flag(
                e.id,
                LuaFlagKind::NoEquivalent,
                "disabled event",
                "disabled events are not exported",
            );
            for l in crate::export::event_text(self.doc, e).lines() {
                out.push(format!("-- (disabled) {l}"));
            }
            return;
        }
        if e.conditions.is_empty() {
            self.body(&e.body, outer, out);
            return;
        }
        let mut exprs = vec![];
        for c in &e.conditions {
            match self.cond_expr(c) {
                Ok(x) => exprs.push(x),
                Err(r) => {
                    self.flag(c.id, LuaFlagKind::NoEquivalent, &c.cmd.name, &r);
                    for l in crate::export::event_text(self.doc, e).lines() {
                        out.push(format!("-- NO LUA EQUIVALENT ({r}): {l}"));
                    }
                    return;
                }
            }
        }
        self.n += 1;
        let k = self.n;
        let f = format!("KfxEvent_{k}");
        let mut all = outer.to_vec();
        all.extend(exprs.iter().cloned());
        let mut body = vec![];
        self.body(&e.body, &all, &mut body);
        let notes = crate::analysis::notes_of(&e.pre);
        let mut s = String::new();
        for l in notes.lines() {
            s.push_str(&format!("-- {l}\n"));
        }
        s.push_str(&format!(
            "-- {}\nfunction {f}()\n",
            crate::analysis::event_summary(self.doc, e)
        ));
        for l in &body {
            s.push_str("    ");
            s.push_str(l);
            s.push('\n');
        }
        s.push_str("end\n");
        self.funcs.push(s);
        if !outer.is_empty() {
            self.flag(
                e.id,
                LuaFlagKind::Approximate,
                "nested event",
                "registered when the parent event fires; the parent \
                    condition is not re-checked afterwards",
            );
        }
        let single = e.conditions.len() == 1;
        let c = &e.conditions[0];
        let turn = (single
            && c.cmd.name == "IF"
            && c.cmd.arg(1).eq_ignore_ascii_case("GAME_TURN")
            && c.cmd.arg(4).is_empty())
        .then(|| match (c.cmd.arg(2), num(c.cmd.arg(3))) {
            (">=" | "==" | "=", Some(n)) if n >= 0 => Some(n),
            (">", Some(n)) if n >= 0 => Some(n + 1),
            _ => None,
        })
        .flatten();
        if let (Some(t), true) = (turn, outer.is_empty()) {
            out.push(format!("RegisterTimerEvent({f}, {t}, false)"));
            if c.cmd.arg(2) == "==" || c.cmd.arg(2) == "=" {
                self.flag(
                    c.id,
                    LuaFlagKind::Approximate,
                    "GAME_TURN ==",
                    "a timer event fires once; the DK condition holds only on that turn too",
                );
            }
            return;
        }
        if single && c.cmd.name == "IF_ACTION_POINT" && outer.is_empty() {
            let p = self.player(c.cmd.arg(1)).unwrap_or_default();
            out.push(format!(
                "RegisterOnActionPointEvent({f}, {}, {p})",
                num(c.cmd.arg(0)).unwrap_or(0)
            ));
            return;
        }
        let cf = format!("KfxCond_{k}");
        self.funcs.push(format!(
            "function {cf}()\n    return {}\nend\n",
            exprs.join(" and ")
        ));
        out.push(format!("RegisterOnConditionEvent({f}, {cf})"));
    }
}

/// Export the model as a Lua level script.
pub fn export_lua(doc: &LogicDoc, index: &ApiIndex, provider: &dyn ConstantProvider) -> LuaExport {
    let mut g = Gen {
        doc,
        index,
        provider,
        funcs: vec![],
        flags: vec![],
        n: 0,
    };
    let mut start = vec![];
    for n in &doc.nodes {
        g.body(std::slice::from_ref(n), &[], &mut start);
    }
    let mut s = String::from(
        "-- Generated by the KeeperFX map editor logic view from the DK level script.\n",
    );
    let ne = g
        .flags
        .iter()
        .filter(|f| f.kind == LuaFlagKind::NoEquivalent)
        .count();
    let ap = g
        .flags
        .iter()
        .filter(|f| f.kind == LuaFlagKind::Approximate)
        .count();
    s.push_str(&format!(
        "-- {ne} construct(s) without a Lua equivalent (kept as comments), {ap} approximate.\n\n"
    ));
    for f in &g.funcs {
        s.push_str(f);
        s.push('\n');
    }
    s.push_str("function OnGameStart()\n");
    for l in &start {
        s.push_str("    ");
        s.push_str(l);
        s.push('\n');
    }
    s.push_str("end\n");
    LuaExport {
        lua: s,
        flags: g.flags,
    }
}
