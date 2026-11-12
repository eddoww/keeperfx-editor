//! The validator: semantic diagnostics for a parsed script.
//!
//! It replays `load_script` (src/lvl_script.c:924) over the parsed lines: IF/ENDIF nesting
//! (`lvl_script_conditions.c`), party bookkeeping (`src/creature_groups.c:522`), the resource counters
//! reported at the end of `load_script`, and the checks done by the `*_check` functions (summarised
//! in `data/script_arg_overlay.json`).
use crate::consts::{is_number, Env, Extras};
use crate::diag::{Diagnostic, Severity, Span};
use crate::doc::{strtol0, Arg, ArgTok, Command, Document, LineKind};
use crate::overlay::{ArgKind, Overlay};
use crate::provider::{ConstKind, ConstantProvider};
use crate::schema::{check_value, ConfigTarget, Schema};
use crate::tables::Tables;

// src/lvl_script.h:30-37
pub const PARTY_TRIGGERS_COUNT: usize = 256;
pub const CREATURE_PARTYS_COUNT: usize = 256;
pub const CONDITIONS_COUNT: usize = 512;
pub const TUNNELLER_TRIGGERS_COUNT: usize = 256;
pub const SCRIPT_VALUES_COUNT: usize = 2048;
pub const WIN_CONDITIONS_COUNT: usize = 12;
/// `GROUP_MEMBERS_COUNT` (src/creature_groups.h:30).
pub const GROUP_MEMBERS_COUNT: usize = 30;
const CREATURES_COUNT: i64 = 1024;
const CREATURE_MAX_LEVEL: i64 = 10;

/// Resource usage as printed at the end of `load_script`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub commands: usize,
    pub tunneller_triggers: usize,
    pub party_triggers: usize,
    pub values: usize,
    pub conditions: usize,
    pub parties: usize,
    pub win_conditions: usize,
    pub lose_conditions: usize,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    pub stats: Stats,
}

/// Commands that never allocate a `ScriptValue` (they use party/tunneller triggers, conditions
/// or act immediately), so they do not count towards `SCRIPT_VALUES_COUNT`.
const NON_VALUE: &[&str] = &[
    "CREATE_PARTY",
    "ADD_TO_PARTY",
    "DELETE_FROM_PARTY",
    "ADD_PARTY_TO_LEVEL",
    "ADD_CREATURE_TO_LEVEL",
    "IF",
    "IF_ACTION_POINT",
    "IF_AVAILABLE",
    "IF_CONTROLS",
    "IF_SLAB_OWNER",
    "IF_SLAB_TYPE",
    "IF_ALLIED",
    "ENDIF",
    "REM",
    "START_MONEY",
    "ADD_TUNNELLER_TO_LEVEL",
    "ADD_TUNNELLER_PARTY_TO_LEVEL",
    "WIN_GAME",
    "LOSE_GAME",
    "NEXT_COMMAND_REUSABLE",
    "PRINT",
    "MESSAGE",
    "LEVEL_VERSION",
    "NEW_TRAP_TYPE",
    "NEW_OBJECT_TYPE",
    "NEW_ROOM_TYPE",
    "NEW_CREATURE_TYPE",
    "COPY_CREATURE_TYPE",
    "RUN_AFTER_VICTORY",
];

pub fn is_if_command(name: &str) -> bool {
    matches!(
        name,
        "IF" | "IF_ACTION_POINT"
            | "IF_AVAILABLE"
            | "IF_CONTROLS"
            | "IF_SLAB_OWNER"
            | "IF_SLAB_TYPE"
            | "IF_ALLIED"
    )
}

struct Party {
    name: String,
    members: usize,
}

struct V<'a> {
    doc: &'a Document,
    provider: &'a dyn ConstantProvider,
    extras: Extras,
    out: Vec<Diagnostic>,
    stats: Stats,
    conds: Vec<(usize, Span)>,
    parties: Vec<Party>,
    party_defs: Vec<(String, usize)>,
    reusable: u32,
    seen_command: bool,
    seen_level_version: bool,
    values_reported: bool,
}

fn declared_extras(doc: &Document) -> Extras {
    let mut e = Extras::default();
    for (_, c) in doc.commands() {
        if c.desc.is_none() || !c.syntax_ok {
            continue;
        }
        let (kind, idx) = match c.name.as_str() {
            "NEW_TRAP_TYPE" => (ConstKind::Trap, 0),
            "NEW_OBJECT_TYPE" => (ConstKind::Object, 0),
            "NEW_ROOM_TYPE" => (ConstKind::Room, 0),
            "NEW_CREATURE_TYPE" => (ConstKind::Creature, 0),
            "COPY_CREATURE_TYPE" => (ConstKind::Creature, 1),
            _ => continue,
        };
        if let Some(a) = c.args.get(idx) {
            if !a.text.is_empty() {
                e.names
                    .entry(kind)
                    .or_default()
                    .push(a.text.to_ascii_uppercase());
            }
        }
    }
    e
}

/// Validate a parsed script against the constants known to `provider`.
pub fn validate(doc: &Document, provider: &dyn ConstantProvider) -> Report {
    let mut v = V {
        doc,
        provider,
        extras: declared_extras(doc),
        out: doc.diagnostics.clone(),
        stats: Stats::default(),
        conds: vec![],
        parties: vec![],
        party_defs: vec![],
        reusable: 0,
        seen_command: false,
        seen_level_version: false,
        values_reported: false,
    };
    for (li, c) in doc.commands() {
        if c.name == "CREATE_PARTY" && c.syntax_ok && c.desc.is_some() {
            if let Some(a) = c.args.first() {
                v.party_defs.push((a.text.to_ascii_uppercase(), li));
            }
        }
    }
    for li in 0..doc.lines.len() {
        v.line(li);
    }
    v.finish();
    let mut diagnostics = v.out;
    diagnostics.sort_by_key(|d| (d.span.start, d.severity as u8));
    Report {
        diagnostics,
        stats: v.stats,
    }
}

impl<'a> V<'a> {
    fn push(&mut self, severity: Severity, span: Span, code: &'static str, message: String) {
        let line = self.doc.line_of(span.start);
        self.out.push(Diagnostic {
            severity,
            span,
            line,
            message,
            code,
        });
    }

    fn finish(&mut self) {
        for (_, span) in std::mem::take(&mut self.conds) {
            self.push(
                Severity::Warning,
                span,
                "missing-endif",
                "Missing ENDIF: this IF block is never closed".into(),
            );
        }
        if self.stats.win_conditions == 0 {
            self.push(
                Severity::Info,
                Span::new(0, 0),
                "no-win-condition",
                "No WIN_GAME condition in the script (fine when a Lua script wins the level)"
                    .into(),
            );
        }
    }

    fn line(&mut self, li: usize) {
        let doc = self.doc;
        let line = &doc.lines[li];
        if self.reusable > 0 {
            self.reusable -= 1;
        }
        let Some(cmd) = &line.cmd else { return };
        self.stats.commands += 1;
        if cmd.preloaded && cmd.desc.is_none() {
            return;
        }
        let Some(desc) = cmd.desc else { return };
        if !cmd.syntax_ok {
            self.unfinished_hint(li, cmd);
            return;
        }
        if !self.seen_command && cmd.name != "LEVEL_VERSION" {
            self.seen_command = true;
        }
        let _ = desc;
        self.command(li, cmd);
    }

    /// A line that ends inside its arguments is discarded by the game (the parser reports the
    /// missing parenthesis). The last argument typed so far is still checked, but only as an `Info`
    /// hint (`unfinished-arg`): it may simply not be complete yet.
    fn unfinished_hint(&mut self, li: usize, cmd: &Command) {
        let doc = self.doc;
        let span = doc.lines[li].span;
        let unfinished = doc.diagnostics.iter().any(|d| {
            matches!(d.code, "missing-close-paren" | "unexpected-eol")
                && d.span.start >= span.start
                && d.span.start <= span.end
        });
        if !unfinished {
            return;
        }
        let Some(arg) = cmd.args.last() else { return };
        if arg.func.is_some()
            || arg.text.is_empty()
            || arg.tok == ArgTok::Tilde
            || arg.tok == ArgTok::Operator
        {
            return;
        }
        let Some(desc) = cmd.desc else { return };
        let letter = if arg.letter != 0 {
            arg.letter
        } else {
            desc.letter(arg.slot).to_ascii_uppercase()
        };
        if letter == b' ' || letter == 0 {
            return;
        }
        let probe = Arg {
            letter,
            ..arg.clone()
        };
        let kind = self.arg_kind(cmd, &probe);
        if matches!(
            kind,
            ArgKind::Text | ArgKind::PartyDef | ArgKind::Party | ArgKind::NewName | ArgKind::Number
        ) {
            return;
        }
        let extras = std::mem::take(&mut self.extras);
        let env = Env {
            provider: self.provider,
            extras: &extras,
            version: doc.version,
        };
        let problem = env.check(kind, letter, arg.extended, &arg.text);
        self.extras = extras;
        if let Some(p) = problem {
            self.push(
                Severity::Info,
                arg.text_span,
                "unfinished-arg",
                format!("{} (the line is not finished)", p.message),
            );
        }
    }

    fn arg_kind(&self, cmd: &Command, arg: &Arg) -> ArgKind {
        let ov = Overlay::get();
        let lk = ArgKind::from_letter(arg.letter);
        match ov.arg(&cmd.name, arg.slot) {
            Some(o)
                if matches!(o.kind, ArgKind::Text | ArgKind::Number)
                    && !matches!(lk, ArgKind::Text | ArgKind::Number) =>
            {
                lk
            }
            Some(o) => o.kind,
            None => lk,
        }
    }

    /// Check one value; returns true when the game would discard the command.
    fn check_value(&mut self, env: &Env, kind: ArgKind, arg: &Arg, text: &str, span: Span) -> bool {
        if let Some(p) = env.check(kind, arg.letter, arg.extended, text) {
            self.push(p.severity, span, p.code, p.message);
            return p.fatal && p.severity == Severity::Error;
        }
        false
    }

    fn command(&mut self, li: usize, cmd: &Command) {
        let doc = self.doc;
        let name = cmd.name.as_str();
        let env_version = if cmd.preloaded {
            cmd.version
        } else {
            doc.version
        };
        let extras = std::mem::take(&mut self.extras);
        let provider = self.provider;
        let env = Env {
            provider,
            extras: &extras,
            version: env_version,
        };
        let mut fatal = false;
        let desc = cmd.desc.unwrap();
        let is_if = is_if_command(name);
        let cond_depth = self.conds.len();

        // --- per-argument checks ---
        for arg in &cmd.args {
            if arg.letter == b' ' || arg.letter == 0 {
                if !arg.text.is_empty() {
                    self.push(
                        Severity::Warning,
                        arg.span,
                        "excess-args",
                        format!(
                            "Excessive parameter of command \"{name}\", value \"{}\"; ignoring \
                            (it takes at most {} arguments)",
                            arg.text,
                            desc.arity()
                        ),
                    );
                }
                continue;
            }
            if arg.tok == ArgTok::Tilde && arg.func.is_none() {
                // only meaningful inside DRAWFROM; elsewhere it occupies a slot like the game does
            }
            // IF-family tail: either `player, variable` (double variable mode) or a number
            if is_if
                && matches!(name, "IF" | "IF_AVAILABLE" | "IF_CONTROLS")
                && arg.slot == 3
                && arg.func.is_none()
            {
                let double = cmd.args.get(4).map(|a| !a.text.is_empty()).unwrap_or(false);
                let k = if double {
                    ArgKind::Player
                } else {
                    ArgKind::Number
                };
                if let Some(mut p) =
                    env.check(k, if double { b'P' } else { b'N' }, false, &arg.text)
                {
                    if double {
                        p.severity = Severity::Warning;
                        p.fatal = false;
                    }
                    self.push(p.severity, arg.span, p.code, p.message);
                }
                continue;
            }
            let kind = self.arg_kind(cmd, arg);
            if let Some(f) = &arg.func {
                fatal |= self.check_func(&env, cmd, arg, f, kind);
                continue;
            }
            if arg.tok == ArgTok::Tilde {
                continue;
            }
            // Empty-by-design optional values are never present as args.
            if matches!(
                kind,
                ArgKind::Text | ArgKind::PartyDef | ArgKind::Party | ArgKind::NewName
            ) {
                continue;
            }
            fatal |= self.check_value(&env, kind, arg, &arg.text, arg.text_span);
        }

        // --- command specific rules ---
        if !fatal {
            fatal |= self.specific(li, cmd, &env);
        }
        self.extras = extras;

        let in_cond = cond_depth > 0;
        let reusable = self.reusable > 0;
        if fatal {
            // Discarded by the game: no state change (but IF still must not be pushed).
            return;
        }
        // state effects
        match name {
            "ENDIF" => {
                if self.conds.pop().is_none() {
                    self.push(
                        Severity::Error,
                        cmd.name_span,
                        "unexpected-endif",
                        "unexpected ENDIF (no open IF)".into(),
                    );
                }
            }
            "NEXT_COMMAND_REUSABLE" => {
                self.reusable = 2;
                let next_is_cmd = self
                    .doc
                    .lines
                    .get(li + 1)
                    .map(|l| l.kind == LineKind::Command)
                    .unwrap_or(false);
                if !next_is_cmd {
                    self.push(
                        Severity::Warning,
                        cmd.name_span,
                        "reusable-lost",
                        "NEXT_COMMAND_REUSABLE only reaches a command on the very next line: the \
                            game also counts blank and comment lines"
                            .into(),
                    );
                }
            }
            "LEVEL_VERSION" => {
                if self.seen_level_version {
                    self.push(
                        Severity::Warning,
                        cmd.name_span,
                        "level-version-repeated",
                        "LEVEL_VERSION appears more than once; the last one wins".into(),
                    );
                }
                if self.seen_command {
                    self.push(
                        Severity::Info,
                        cmd.name_span,
                        "level-version-late",
                        "LEVEL_VERSION is not at the top of the script; commands before it are \
                            logged as invalid by the preload pass"
                            .into(),
                    );
                }
                self.seen_level_version = true;
                if in_cond {
                    self.push(
                        Severity::Warning,
                        cmd.name_span,
                        "preload-in-if",
                        "LEVEL_VERSION runs at load time and ignores the surrounding IF".into(),
                    );
                }
            }
            "NEW_TRAP_TYPE" | "NEW_OBJECT_TYPE" | "NEW_ROOM_TYPE" | "NEW_CREATURE_TYPE"
            | "COPY_CREATURE_TYPE" => {
                if in_cond {
                    self.push(
                        Severity::Warning,
                        cmd.name_span,
                        "preload-in-if",
                        format!("{name} runs at load time and ignores the surrounding IF"),
                    );
                }
            }
            _ => {}
        }
        if !cmd.preloaded && !NON_VALUE.contains(&name) && (in_cond || reusable) {
            self.stats.values += 1;
            if self.stats.values > SCRIPT_VALUES_COUNT && !self.values_reported {
                self.values_reported = true;
                self.push(
                    Severity::Error,
                    cmd.name_span,
                    "too-many-values",
                    format!("Too many VALUEs in script (limit is {SCRIPT_VALUES_COUNT})"),
                );
            }
        }
    }

    fn check_func(
        &mut self,
        env: &Env,
        cmd: &Command,
        arg: &Arg,
        f: &crate::doc::FuncCall,
        kind: ArgKind,
    ) -> bool {
        let version = env.version;
        match f.name.as_str() {
            "IMPORT" => {
                if f.ok && f.args.len() >= 2 {
                    let p = &f.args[0].text;
                    let ok_player = !(matches!(
                        p.as_str(),
                        "ALL_PLAYERS" | "PLAYER_NEUTRAL" | "PLAYER4" | "PLAYER5" | "PLAYER6"
                    ));
                    if !ok_player {
                        self.push(
                            Severity::Error,
                            f.args[0].span,
                            "bad-import",
                            format!("Cannot fetch flag values for player, '{p}'"),
                        );
                    }
                    let fl = &f.args[1].text;
                    if !Tables::get().is_named("campaign_flag_desc", fl) {
                        self.push(
                            Severity::Error,
                            f.args[1].span,
                            "unknown-flag",
                            format!("Unknown campaign flag name, '{fl}'"),
                        );
                    }
                }
                false
            }
            "RANDOM" | "DRAWFROM" => {
                if f.name == "DRAWFROM" && version <= 0 {
                    self.push(
                        Severity::Error,
                        f.name_span,
                        "function-version",
                        "The function \"DRAWFROM\" is not supported in old level format \
                        (add LEVEL_VERSION(1))"
                            .into(),
                    );
                    return false;
                }
                if f.name == "RANDOM" && version > 0 {
                    self.push(
                        Severity::Error,
                        f.name_span,
                        "function-version",
                        "The function \"RANDOM\" is not supported yet with \
                        LEVEL_VERSION(1) (use DRAWFROM, or level version 0)"
                            .into(),
                    );
                    return false;
                }
                let mut fatal = false;
                let mut prev_num: Option<i64> = None;
                let mut i = 0;
                let mut total = 0;
                while i < f.args.len() {
                    let e = &f.args[i];
                    if e.tok == ArgTok::Tilde {
                        if i + 1 < f.args.len() {
                            let hi = &f.args[i + 1];
                            let hv = strtol0(&hi.text).0;
                            if let Some(lo) = prev_num {
                                if hv < lo {
                                    self.push(
                                        Severity::Warning,
                                        hi.span,
                                        "range-order",
                                        "Range definition should have the lower value first".into(),
                                    );
                                }
                            }
                            if kind != ArgKind::Text {
                                fatal |= self.check_value(env, kind, arg, &hi.text, hi.span);
                            }
                            i += 2;
                            continue;
                        }
                        i += 1;
                        continue;
                    }
                    prev_num = Some(strtol0(&e.text).0);
                    total += 1;
                    if !is_number(&e.text)
                        || !matches!(kind, ArgKind::Number | ArgKind::Count | ArgKind::Amount)
                    {
                        fatal |= self.check_value(env, kind, arg, &e.text, e.span);
                    }
                    i += 1;
                }
                if total == 0 && f.ok {
                    self.push(
                        Severity::Error,
                        f.name_span,
                        "empty-range",
                        format!(
                            "Arguments of function \"{}\" define no values to select from",
                            f.name
                        ),
                    );
                }
                let _ = cmd;
                fatal
            }
            _ => false,
        }
    }

    /// `SET_GAME_RULE(rule, value[, player])`: the rule must be a field of the rules blocks and the
    /// value is read like `value_default` (src/lvl_script_commands.c:5603, src/config.c:438). The game logs
    /// an error for an unknown rule and drops the command; a bad value is clamped or becomes 0 with
    /// a warning. Reported here as warnings because the list is the one of this game version.
    fn game_rule(&mut self, cmd: &Command) -> bool {
        let Some(name) = cmd
            .args
            .first()
            .filter(|a| a.func.is_none() && !a.text.is_empty())
        else {
            return false;
        };
        let schema = Schema::get();
        let Some(rule) = schema.rule(&name.text) else {
            let sug = crate::lua::diagnostics::suggest(
                &name.text,
                schema.rules().iter().map(|r| r.name.as_str()),
            );
            let hint = sug
                .map(|s| format!(" (did you mean {}?)", s.to_ascii_uppercase()))
                .unwrap_or_default();
            self.push(
                Severity::Warning,
                name.span,
                "unknown-rule",
                format!(
                    "Unknown game rule '{}'{hint}; SET_GAME_RULE ignores the line",
                    name.text
                ),
            );
            return true;
        };
        // a numeric third argument is glued to the value ("v1 v2"): leave multi-value rules alone
        let glued = cmd
            .args
            .get(2)
            .map(|a| crate::consts::is_number(&a.text))
            .unwrap_or(false);
        if let (Some(v), Some(arg), false) = (
            cmd.args
                .get(1)
                .filter(|a| a.func.is_none() && !a.text.is_empty()),
            rule.args.first(),
            glued,
        ) {
            if let Some(p) = check_value(rule, arg, &v.text) {
                self.push(
                    Severity::Warning,
                    v.span,
                    if p.code == "value-range" {
                        "rule-value-range"
                    } else {
                        "rule-value"
                    },
                    format!("SET_GAME_RULE {}: {}", rule.name, p.message),
                );
            }
        }
        false
    }

    /// `SET_*_CONFIGURATION(id, property, values...)`: the property must be one of the target's
    /// fields (`set_config_check`, src/lvl_script_commands.c:637; creatures 2418; powers 5174).
    fn config_property(&mut self, cmd: &Command, target: ConfigTarget) -> bool {
        let Some(prop_arg) = cmd
            .args
            .get(1)
            .filter(|a| a.func.is_none() && !a.text.is_empty())
        else {
            return false;
        };
        let schema = Schema::get();
        let Some(prop) = schema.prop(target, &prop_arg.text) else {
            if schema.props(target).is_empty() {
                return false;
            }
            let sug = crate::lua::diagnostics::suggest(
                &prop_arg.text,
                schema.props(target).iter().map(|r| r.name.as_str()),
            );
            let hint = sug
                .map(|s| format!(" (did you mean {}?)", s.to_ascii_uppercase()))
                .unwrap_or_default();
            self.push(
                Severity::Warning,
                prop_arg.span,
                "unknown-config-property",
                format!(
                    "Unknown {} property '{}'{hint}; {} ignores the line",
                    target.name(),
                    prop_arg.text,
                    cmd.name
                ),
            );
            return true;
        };
        for (i, arg) in prop.args.iter().enumerate() {
            let Some(v) = cmd
                .args
                .get(2 + i)
                .filter(|a| a.func.is_none() && !a.text.is_empty())
            else {
                break;
            };
            if let Some(p) = check_value(prop, arg, &v.text) {
                self.push(
                    Severity::Warning,
                    v.span,
                    "config-value",
                    format!("{} {}: {}", cmd.name, prop.name, p.message),
                );
            }
        }
        false
    }

    /// `SET_COMPUTER_PROCESS/CHECKS/EVENT(player, name, ...)`: the name is matched against the
    /// processes/checks/events of the player's computer (src/lvl_script_commands.c:6502, 6574, 6653),
    /// which come from keepcompp.cfg; an unknown name changes nothing.
    fn computer_name(&mut self, cmd: &Command, env: &Env) {
        let (kind, what) = match cmd.name.as_str() {
            "SET_COMPUTER_PROCESS" => (ConstKind::ComputerProcess, "process"),
            "SET_COMPUTER_CHECKS" => (ConstKind::ComputerCheck, "check"),
            _ => (ConstKind::ComputerEvent, "event"),
        };
        let Some(name) = cmd
            .args
            .get(1)
            .filter(|a| a.func.is_none() && !a.text.is_empty())
        else {
            return;
        };
        if env.in_const(kind, &name.text) == Some(false) {
            let names = env.const_names(kind);
            let sug =
                crate::lua::diagnostics::suggest(&name.text, names.iter().map(|s| s.as_str()));
            let hint = sug
                .map(|s| format!(" (did you mean {s}?)"))
                .unwrap_or_default();
            self.push(
                Severity::Warning,
                name.span,
                "unknown-computer-name",
                format!(
                    "No computer \
                {what} named '{}' in keepcompp.cfg{hint}; the command changes nothing",
                    name.text
                ),
            );
        }
    }

    fn party_index(&self, name: &str) -> Option<usize> {
        self.parties
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Party reference check. `Err` = reported (the game discards the command); `Ok(None)` = the
    /// argument is a DRAWFROM list whose members were all checked.
    fn need_party(&mut self, cmd: &Command, slot: usize) -> Result<Option<usize>, ()> {
        let Some(a) = cmd.args.get(slot) else {
            return Ok(None);
        };
        let names: Vec<(String, Span)> = match &a.func {
            Some(f) => f
                .args
                .iter()
                .filter(|x| x.tok != ArgTok::Tilde)
                .map(|x| (x.text.clone(), x.span))
                .collect(),
            None => vec![(a.text.clone(), a.span)],
        };
        let mut first = None;
        let mut failed = false;
        for (n, sp) in &names {
            match self.party_index(n) {
                Some(i) => {
                    first.get_or_insert(i);
                }
                None => {
                    let later = self
                        .party_defs
                        .iter()
                        .find(|(d, _)| d.eq_ignore_ascii_case(n))
                        .map(|(_, l)| *l);
                    let extra = match later {
                        Some(l) => format!(
                            " (it is created later, on line {}; parties must be \
                            created before use)",
                            self.doc.line_of(self.doc.lines[l].span.start)
                        ),
                        None => " (no CREATE_PARTY with this name)".into(),
                    };
                    self.push(
                        Severity::Error,
                        *sp,
                        "unknown-party",
                        format!("Party of requested name, '{n}', is not defined{extra}"),
                    );
                    failed = true;
                }
            }
        }
        if failed {
            Err(())
        } else if a.func.is_some() {
            Ok(None)
        } else {
            Ok(first)
        }
    }

    fn num(cmd: &Command, slot: usize) -> Option<(i64, Span)> {
        let a = cmd.args.get(slot)?;
        if a.func.is_some() {
            return None; // value only known at run time (RANDOM/DRAWFROM/IMPORT)
        }
        Some((strtol0(&a.text).0, a.span))
    }

    fn single_player(&mut self, cmd: &Command, slot: usize) -> bool {
        if let Some(a) = cmd.args.get(slot) {
            if a.text == "ALL_PLAYERS" {
                self.push(
                    Severity::Error,
                    a.span,
                    "player-not-single",
                    "Given owning player is not supported in this command (ALL_PLAYERS)".into(),
                );
                return true;
            }
        }
        false
    }

    fn trigger_limit(
        &mut self,
        cmd: &Command,
        count: usize,
        limit: usize,
        what: &str,
        code: &'static str,
    ) -> bool {
        if count >= limit {
            self.push(
                Severity::Error,
                cmd.name_span,
                code,
                format!("Too many {what} in script (limit {limit})"),
            );
            return true;
        }
        false
    }

    /// Rules from the command handlers. Returns true when the game discards the command.
    fn specific(&mut self, li: usize, cmd: &Command, env: &Env) -> bool {
        let name = cmd.name.as_str();
        let in_cond = !self.conds.is_empty();
        let trig = in_cond || self.reusable > 0;
        let mut fatal = false;
        match name {
            "CREATE_PARTY" => {
                if in_cond {
                    self.push(
                        Severity::Warning,
                        cmd.name_span,
                        "party-in-if",
                        format!(
                            "Party '{}' defined inside conditional statement",
                            cmd.text(0)
                        ),
                    );
                }
                if self.party_index(cmd.text(0)).is_some() {
                    self.push(
                        Severity::Warning,
                        cmd.args[0].span,
                        "duplicate-party",
                        format!(
                            "Party '{}' is already defined; the first definition keeps being used",
                            cmd.text(0)
                        ),
                    );
                }
                if self.stats.parties >= CREATURE_PARTYS_COUNT {
                    self.push(
                        Severity::Error,
                        cmd.name_span,
                        "too-many-parties",
                        "Too many partys in script".into(),
                    );
                    return true;
                }
                self.stats.parties += 1;
                self.parties.push(Party {
                    name: cmd.text(0).to_ascii_uppercase(),
                    members: 0,
                });
            }
            "ADD_TO_PARTY" | "DELETE_FROM_PARTY" => {
                let Ok(p) = self.need_party(cmd, 0) else {
                    return true;
                };
                if name == "ADD_TO_PARTY" {
                    if let Some((lv, sp)) = Self::num(cmd, 2) {
                        if lv < 1 || lv > CREATURE_MAX_LEVEL {
                            self.push(
                                Severity::Error,
                                sp,
                                "level-range",
                                format!(
                                    "Invalid Creature Level parameter; {lv} not in range \
                                (1,{CREATURE_MAX_LEVEL})"
                                ),
                            );
                            return true;
                        }
                    }
                    if trig {
                        self.stats.party_triggers += 1;
                        if self.stats.party_triggers > PARTY_TRIGGERS_COUNT {
                            self.push(
                                Severity::Error,
                                cmd.name_span,
                                "too-many-party-triggers",
                                format!(
                                    "Max party triggers reached (limit {PARTY_TRIGGERS_COUNT})"
                                ),
                            );
                        }
                    } else if let Some(i) = p {
                        self.parties[i].members += 1;
                        if self.parties[i].members > GROUP_MEMBERS_COUNT {
                            let n = self.parties[i].name.clone();
                            self.push(
                                Severity::Error,
                                cmd.name_span,
                                "party-too-big",
                                format!(
                                    "Too many creatures in party '{n}' (limit is \
                                {GROUP_MEMBERS_COUNT} members)"
                                ),
                            );
                        }
                    }
                } else if trig {
                    self.stats.party_triggers += 1;
                    if self.stats.party_triggers > PARTY_TRIGGERS_COUNT {
                        self.push(
                            Severity::Error,
                            cmd.name_span,
                            "too-many-party-triggers",
                            format!("Max party triggers reached (limit {PARTY_TRIGGERS_COUNT})"),
                        );
                    }
                } else if let Some(i) = p {
                    if self.parties[i].members > 0 {
                        self.parties[i].members -= 1;
                    }
                }
            }
            "ADD_PARTY_TO_LEVEL" => {
                if let Some((n, sp)) = Self::num(cmd, 3) {
                    if n < 1 {
                        self.push(
                            Severity::Error,
                            sp,
                            "count-range",
                            "Invalid NUMBER parameter".into(),
                        );
                        return true;
                    }
                }
                if self.trigger_limit(
                    cmd,
                    self.stats.party_triggers,
                    PARTY_TRIGGERS_COUNT,
                    "ADD_CREATURE commands",
                    "too-many-party-triggers",
                ) {
                    return true;
                }
                if self.single_player(cmd, 0) {
                    return true;
                }
                if self.need_party(cmd, 1).is_err() {
                    return true;
                }
                if trig {
                    self.stats.party_triggers += 1;
                }
            }
            "ADD_CREATURE_TO_LEVEL" => {
                if let Some((lv, sp)) = Self::num(cmd, 4) {
                    if lv < 1 || lv > CREATURE_MAX_LEVEL {
                        self.push(
                            Severity::Error,
                            sp,
                            "level-range",
                            format!(
                                "Invalid CREATURE \
                            LEVEL parameter ({lv}, valid 1..{CREATURE_MAX_LEVEL})"
                            ),
                        );
                        return true;
                    }
                }
                if let Some((n, sp)) = Self::num(cmd, 3) {
                    if n <= 0 || n >= CREATURES_COUNT {
                        self.push(
                            Severity::Error,
                            sp,
                            "count-range",
                            format!("Invalid number of creatures to add ({n})"),
                        );
                        return true;
                    }
                }
                if self.trigger_limit(
                    cmd,
                    self.stats.party_triggers,
                    PARTY_TRIGGERS_COUNT,
                    "ADD_CREATURE commands",
                    "too-many-party-triggers",
                ) {
                    return true;
                }
                if self.single_player(cmd, 0) {
                    return true;
                }
                if in_cond {
                    self.stats.party_triggers += 1;
                }
            }
            "ADD_TUNNELLER_TO_LEVEL" | "ADD_TUNNELLER_PARTY_TO_LEVEL" => {
                let lvslot = if name == "ADD_TUNNELLER_TO_LEVEL" {
                    4
                } else {
                    5
                };
                if let Some((lv, sp)) = Self::num(cmd, lvslot) {
                    if lv < 1 || lv > CREATURE_MAX_LEVEL {
                        self.push(
                            Severity::Error,
                            sp,
                            "level-range",
                            format!(
                                "Invalid CREATURE \
                            LEVEL parameter ({lv}, valid 1..{CREATURE_MAX_LEVEL})"
                            ),
                        );
                        return true;
                    }
                }
                if self.trigger_limit(
                    cmd,
                    self.stats.tunneller_triggers,
                    TUNNELLER_TRIGGERS_COUNT,
                    "ADD_TUNNELLER commands",
                    "too-many-tunneller-triggers",
                ) {
                    return true;
                }
                if self.single_player(cmd, 0) {
                    return true;
                }
                if name == "ADD_TUNNELLER_PARTY_TO_LEVEL" && self.need_party(cmd, 1).is_err() {
                    return true;
                }
                if in_cond {
                    self.stats.tunneller_triggers += 1;
                }
            }
            "WIN_GAME" | "LOSE_GAME" => {
                if !in_cond {
                    self.push(
                        Severity::Error,
                        cmd.name_span,
                        "win-without-condition",
                        format!(
                            "Command {} found with no condition (it takes effect immediately)",
                            name.replace('_', " ")
                        ),
                    );
                }
                let (n, what) = if name == "WIN_GAME" {
                    (&mut self.stats.win_conditions, "WIN GAME")
                } else {
                    (&mut self.stats.lose_conditions, "LOSE GAME")
                };
                if *n >= WIN_CONDITIONS_COUNT {
                    self.push(
                        Severity::Error,
                        cmd.name_span,
                        "too-many-win-conditions",
                        format!(
                            "Too many {what} conditions in script (limit {WIN_CONDITIONS_COUNT})"
                        ),
                    );
                    return true;
                }
                *n += 1;
            }
            "START_MONEY" if in_cond => {
                self.push(
                    Severity::Warning,
                    cmd.name_span,
                    "start-money-in-if",
                    "Start money set inside conditional block; condition ignored".into(),
                );
            }
            n if is_if_command(n) => {
                if self.stats.conditions >= CONDITIONS_COUNT {
                    self.push(
                        Severity::Error,
                        cmd.name_span,
                        "too-many-conditions",
                        format!("Too many (over {CONDITIONS_COUNT}) conditions in script"),
                    );
                    return true;
                }
                if n == "IF_ACTION_POINT" {
                    if let Some((ap, sp)) = Self::num(cmd, 0) {
                        if env.ap_exists(ap) == Some(false) {
                            self.push(
                                Severity::Error,
                                sp,
                                "missing-action-point",
                                format!("Non-existing Action Point, no {ap}"),
                            );
                            return true;
                        }
                    }
                }
                self.stats.conditions += 1;
                self.conds.push((li, cmd.name_span));
            }
            "COMPUTER_PLAYER" => {
                if env.version <= 0 && cmd.text(0) == "PLAYER_GOOD" {
                    self.push(
                        Severity::Error,
                        cmd.args[0].span,
                        "level-version-feature",
                        "PLAYER_GOOD COMPUTER_PLAYER cannot be set in level version 0".into(),
                    );
                    return true;
                }
            }
            "RESEARCH" | "RESEARCH_ORDER" => {
                if cmd.args.len() >= 3 {
                    let ty = cmd.args[1].text.as_str();
                    let item = &cmd.args[2];
                    let k = match ty {
                        "MAGIC" => Some(ConstKind::Power),
                        "ROOM" => Some(ConstKind::Room),
                        "CREATURE" => Some(ConstKind::Creature),
                        _ => None,
                    };
                    if let Some(k) = k {
                        if env.in_const(k, &item.text) == Some(false) {
                            self.push(
                                Severity::Error,
                                item.span,
                                "unknown-constant",
                                format!("Unknown {ty} research item, \"{}\"", item.text),
                            );
                            return true;
                        }
                    }
                }
            }
            "SET_GAME_RULE" => fatal |= self.game_rule(cmd),
            "SET_COMPUTER_PROCESS" | "SET_COMPUTER_CHECKS" | "SET_COMPUTER_EVENT" => {
                self.computer_name(cmd, env)
            }
            n if ConfigTarget::of_command(n).is_some() => {
                fatal |= self.config_property(cmd, ConfigTarget::of_command(n).unwrap())
            }
            _ => {}
        }
        fatal |= false;
        fatal
    }
}
