//! DK script to Lua assistant.
//!
//! **A starting point, not a translation.** The two languages differ in more than syntax (a DK
//! script is a list of commands executed once at load, `IF` blocks are one-shot conditions checked
//! every turn; Lua registers event handlers), and the wiki gives no complete command mapping. The
//! assistant therefore only writes what it can justify and keeps everything else as a comment
//! marked `TODO(dk2lua)`:
//!
//! * a command becomes a call when a library function with the rule-based name
//!   (`SET_GENERATE_SPEED` is `SetGenerateSpeed`, the naming of the shipped bindings) exists in the
//!   [`ApiIndex`], the given arguments fit its arity, every argument kind fits the declared
//!   parameter type (and, for plain numbers and words, the parameter names are related, so a
//!   shifted parameter list is not mistaken for a match), and the generated line raises no Lua
//!   diagnostic of its own (`SWAP_CREATURE` is refused outright: its Lua twin swaps the
//!   parameters). Preloaded commands (`LEVEL_VERSION`, `NEW_*_TYPE`) are never converted.
//! * `SET_FLAG`/`ADD_TO_FLAG` and the campaign flag twins become `PLAYER0.FLAG1 = ...` (the
//!   `Player` table reads and writes variables with the names `parse_get_varib` knows,
//!   `src/lua_api_player.c:201`, `parse_set_varib`), `ADD_GOLD_TO_PLAYER` the `add_gold` method,
//!   `WIN_GAME`/`LOSE_GAME` `WinGame()`/`LoseGame()` (all players, like the command,
//!   `src/lua_api.c:260`).
//! * `IF` blocks become trigger registrations only for the patterns the shipped
//!   `fxdata/lua/triggers/Events.lua` has a helper for: `IF_ACTION_POINT(n, player)` with
//!   `RegisterOnActionPointEvent`, `IF(player, GAME_TURN >= n)` / `== n` with `RegisterTimerEvent`
//!   (one shot), and `IF(player, VARIABLE op number)` with `RegisterOnConditionEvent` (checked once
//!   per game tick, fires once). Block bodies and conditions are global named functions, as the
//!   saves need. An `IF` with another shape, a nested `IF`, or a block whose condition cannot be
//!   expressed is kept entirely as comments.
use super::annot::{parse_ty, Ty};
use super::diagnostics::{diagnostics, kinds_of_type};
use super::doc::LuaDoc;
use super::index::{ApiIndex, FuncDef};
use super::infer::Env;
use super::template::dk_to_lua_name;
use crate::consts::is_number;
use crate::diag::Severity;
use crate::doc::{Arg, ArgTok, Command, Document, LineKind};
use crate::overlay::{ArgKind, Overlay};
use crate::provider::{ConstKind, ConstantProvider};
use crate::services::arg_kind_at;
use crate::tables::Tables;

/// Marker written into the generated Lua, as `-- TODO(dk2lua) <original line>`, before every
/// line the assistant could not convert: the user ports those lines by hand, and an editor's
/// TODO search finds them all. Part of the output format: keep it stable.
pub const TODO_MARK: &str = "TODO(dk2lua)";

#[derive(Clone, Debug, Default)]
pub struct ConvertOptions {
    /// File name of the DK script, for the header comment.
    pub source_name: Option<String>,
}

/// Counts of command lines (blank and comment lines are not commands).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConvertStats {
    /// Lines that hold a command (including `IF`/`ENDIF` and invalid commands).
    pub commands: usize,
    /// Command lines turned into Lua code (an `IF` and its `ENDIF` count when the block became a
    /// registration).
    pub converted: usize,
    /// Command lines kept as comments.
    pub commented: usize,
    pub if_blocks: usize,
    pub if_blocks_converted: usize,
}

impl ConvertStats {
    /// Share of command lines that became code, 0.0..=1.0.
    pub fn share_converted(&self) -> f64 {
        if self.commands == 0 {
            1.0
        } else {
            self.converted as f64 / self.commands as f64
        }
    }
}

/// Why a command stayed a comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertNote {
    /// 1-based line of the DK script.
    pub line: usize,
    pub command: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct Conversion {
    pub lua: String,
    pub stats: ConvertStats,
    pub notes: Vec<ConvertNote>,
}

// ---- helpers ------------------------------------------------------------------------------

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

const SYNONYMS: &[(&str, &str)] = &[
    ("amount", "gold"),
    ("amount", "annoyance"),
    ("amount", "numberavailable"),
    ("count", "amount"),
    ("count", "maxamount"),
    ("level", "experience"),
    ("turns", "interval"),
    ("turns", "gameturns"),
    ("message", "msgid"),
    ("message", "msg"),
    ("message", "tooltiptextid"),
    ("text", "message"),
    ("text", "tooltip"),
    ("index", "slot"),
    ("researchable", "canbeavailable"),
    ("buildable", "canbeavailable"),
    ("available", "canbeattracted"),
    ("value", "val1"),
    ("levelnumber", "lvnum"),
    ("realtime", "clocktime"),
];

fn names_related(dk: &str, lua: &str) -> bool {
    let (a, b) = (norm(dk), norm(lua));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b || (a.len() >= 3 && b.contains(&a)) || (b.len() >= 3 && a.contains(&b)) {
        return true;
    }
    SYNONYMS
        .iter()
        .any(|(x, y)| (a == *x && b == *y) || (a == *y && b == *x))
}

/// Kinds whose meaning is fixed by their type: no parameter name comparison needed.
fn specific_kind(k: ArgKind) -> bool {
    matches!(
        k,
        ArgKind::Player
            | ArgKind::Creature
            | ArgKind::Room
            | ArgKind::Power
            | ArgKind::Spell
            | ArgKind::Trap
            | ArgKind::Door
            | ArgKind::Object
            | ArgKind::Effect
            | ArgKind::EffectGen
            | ArgKind::Slab
            | ArgKind::Location
            | ArgKind::SubtileX
            | ArgKind::SubtileY
            | ArgKind::SlabX
            | ArgKind::SlabY
            | ArgKind::ApNumber
            | ArgKind::HeroGate
            | ArgKind::Bool
            | ArgKind::FreeState
            | ArgKind::LockState
            | ArgKind::Timer
    )
}

fn is_number_kind(k: ArgKind) -> bool {
    matches!(
        k,
        ArgKind::Number
            | ArgKind::Count
            | ArgKind::Amount
            | ArgKind::Level
            | ArgKind::Turns
            | ArgKind::SubtileX
            | ArgKind::SubtileY
            | ArgKind::SlabX
            | ArgKind::SlabY
            | ArgKind::ApNumber
            | ArgKind::HeroGate
            | ArgKind::BoxNumber
            | ArgKind::MessageId
            | ArgKind::QuickMsg
            | ArgKind::IfValue
    )
}

fn const_kind_of(k: ArgKind) -> Option<ConstKind> {
    Some(match k {
        ArgKind::Creature => ConstKind::Creature,
        ArgKind::Room => ConstKind::Room,
        ArgKind::Power => ConstKind::Power,
        ArgKind::Spell => ConstKind::Spell,
        ArgKind::Trap => ConstKind::Trap,
        ArgKind::Door => ConstKind::Door,
        ArgKind::Object => ConstKind::Object,
        ArgKind::Effect => ConstKind::Effect,
        ArgKind::EffectGen => ConstKind::EffectGen,
        ArgKind::Slab => ConstKind::Slab,
        _ => return None,
    })
}

/// What a Lua parameter type accepts.
#[derive(Default, Debug)]
struct LuaTy {
    int: bool,
    string: bool,
    boolean: bool,
    player: bool,
    any: bool,
    kinds: Vec<ConstKind>,
    lits: Vec<String>,
}

fn classify(env: &Env, ty: &Ty, optional: bool) -> LuaTy {
    let mut t = LuaTy::default();
    let _ = optional;
    for k in kinds_of_type(env, ty) {
        t.kinds.push(k);
    }
    let vals = env.type_values(ty);
    for l in &vals.literals {
        if l.is_str {
            t.string = true;
            t.lits.push(l.value.clone());
        } else {
            t.int = true;
        }
    }
    if vals.open {
        t.string = true;
    }
    fn go(env: &Env, ty: &Ty, t: &mut LuaTy, depth: u32) {
        if depth > 6 {
            return;
        }
        for m in ty.members() {
            match &m {
                Ty::Name(n) => match n.as_str() {
                    "any" | "unknown" => t.any = true,
                    "string" => t.string = true,
                    "number" | "integer" => t.int = true,
                    "boolean" => t.boolean = true,
                    "Player" => t.player = true,
                    other => {
                        if let Some(a) = env.alias(other) {
                            go(env, &parse_ty(&a.ty), t, depth + 1);
                        }
                    }
                },
                Ty::Str(_) => t.string = true,
                Ty::Num(_) => t.int = true,
                Ty::Bool(_) => t.boolean = true,
                _ => {}
            }
        }
    }
    go(env, ty, &mut t, 0);
    t
}

fn lua_string(s: &str) -> Option<String> {
    if s.chars().any(|c| c.is_control() || !c.is_ascii()) {
        return None;
    }
    Some(format!(
        "\"{}\"",
        s.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn dk_bool(text: &str) -> Option<bool> {
    match text.to_ascii_uppercase().as_str() {
        "0" | "OFF" | "NO" | "FALSE" | "DISABLE" | "DISABLED" | "UNLOCKED" | "PAID" => Some(false),
        "1" | "ON" | "YES" | "TRUE" | "ENABLE" | "ENABLED" | "LOCKED" | "FREE" => Some(true),
        _ => None,
    }
}

fn int_text(text: &str) -> Option<String> {
    is_number(text).then(|| crate::doc::strtol0(text).0.to_string())
}

/// `PLAYER0`, `PLAYER_GOOD`...: names the library defines as `Player` globals.
fn player_global<'a>(index: &ApiIndex, text: &'a str) -> Option<&'a str> {
    index
        .globals
        .get(text)
        .filter(|g| g.ty == "Player")
        .map(|_| text)
}

struct Ctx<'a> {
    index: &'a ApiIndex,
    env: Env<'a>,
    provider: &'a dyn ConstantProvider,
}

impl<'a> Ctx<'a> {
    /// Render one DK argument for a Lua parameter, `Err(reason)` when it does not fit.
    fn render_arg(
        &self,
        dk_name: &str,
        kind: ArgKind,
        arg: &Arg,
        lua_name: &str,
        lt: &LuaTy,
    ) -> Result<String, String> {
        if arg.func.is_some() {
            return Err(format!(
                "argument '{}' is computed at run time (RANDOM/DRAWFROM/IMPORT)",
                arg.text
            ));
        }
        if arg.text.is_empty() {
            return Err("empty argument".into());
        }
        if !specific_kind(kind) && !names_related(dk_name, lua_name) {
            return Err(format!(
                "DK parameter '{dk_name}' does not obviously correspond to Lua \
                parameter '{lua_name}'"
            ));
        }
        let text = arg.text.as_str();
        let numeric = int_text(text);
        let word_ok = arg.tok != ArgTok::Operator && arg.tok != ArgTok::Tilde;
        if !word_ok {
            return Err("unsupported argument".into());
        }
        let bad = |what: &str| {
            Err(format!(
                "'{text}' ({what}) does not fit parameter '{lua_name}'"
            ))
        };
        match kind {
            ArgKind::Player => {
                if let Some(g) = player_global(self.index, text) {
                    if lt.player || lt.any {
                        return Ok(g.to_string());
                    }
                }
                if lt.lits.iter().any(|l| l == text) {
                    return lua_string(text).ok_or_else(|| "unprintable text".into());
                }
                bad("player")
            }
            ArgKind::Location => {
                if let Some(n) = &numeric {
                    return if n.starts_with('-') || n == "0" || !(lt.int || lt.any) {
                        bad("hero gate or zero")
                    } else {
                        Ok(n.clone())
                    };
                }
                if let Some(g) = player_global(self.index, text) {
                    if lt.player || lt.any || lt.lits.iter().any(|l| l == g) {
                        return Ok(g.to_string());
                    }
                }
                if lt.lits.iter().any(|l| l == text) {
                    return lua_string(text).ok_or_else(|| "unprintable text".into());
                }
                bad("location")
            }
            ArgKind::Bool | ArgKind::FreeState | ArgKind::LockState => {
                if lt.boolean {
                    return dk_bool(text)
                        .map(|b| b.to_string())
                        .ok_or_else(|| format!("'{text}' is not a boolean word"));
                }
                if let Some(n) = &numeric {
                    if lt.int || lt.any {
                        return Ok(n.clone());
                    }
                }
                if lt.lits.iter().any(|l| l.eq_ignore_ascii_case(text)) {
                    return lua_string(&text.to_ascii_uppercase())
                        .ok_or_else(|| "unprintable text".into());
                }
                bad("boolean")
            }
            k if is_number_kind(k) => {
                let Some(n) = numeric else {
                    return bad("not a number");
                };
                if lt.int || lt.any {
                    Ok(n)
                } else if lt.boolean && (n == "0" || n == "1") {
                    Ok((n == "1").to_string())
                } else {
                    bad("number")
                }
            }
            k => {
                // named constants and words
                if let Some(n) = &numeric {
                    return if lt.int || lt.any {
                        Ok(n.clone())
                    } else {
                        bad("number")
                    };
                }
                if arg.tok == ArgTok::Str || arg.tok == ArgTok::Ident {
                    let kinds_ok = match const_kind_of(k) {
                        Some(ck) => {
                            lt.kinds.contains(&ck)
                                || lt.any
                                || (lt.string && lt.kinds.is_empty() && lt.lits.is_empty())
                        }
                        None => lt.string || lt.any,
                    };
                    if !kinds_ok {
                        return bad("word");
                    }
                    let shown = if arg.tok == ArgTok::Ident {
                        text.to_ascii_uppercase()
                    } else {
                        text.to_string()
                    };
                    return lua_string(&shown).ok_or_else(|| "unprintable text".into());
                }
                bad("word")
            }
        }
    }

    /// Generic conversion through a library function of the rule-based name.
    fn call(&self, cmd: &Command) -> Result<String, String> {
        let want = dk_to_lua_name(&cmd.name);
        let (fname, def): (&String, &FuncDef) = self
            .index
            .functions
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&want))
            .ok_or_else(|| format!("no library function named {want}"))?;
        // known to differ in parameter order or meaning although the kinds fit
        if matches!(cmd.name.as_str(), "SWAP_CREATURE") {
            return Err("the Lua function takes its parameters in another order".into());
        }
        if def.method || def.vararg.is_some() {
            return Err("the library function takes a variable argument list".into());
        }
        let given = cmd
            .args
            .iter()
            .filter(|a| a.letter != b' ' && a.letter != 0 || !a.text.is_empty())
            .count();
        let required = def.required_params();
        if given > def.params.len() {
            return Err(format!(
                "{fname} takes at most {} arguments, the command has {given}",
                def.params.len()
            ));
        }
        if given < required {
            return Err(format!(
                "{fname} needs {required} arguments, the command has {given}"
            ));
        }
        let ov = Overlay::get();
        let mut out: Vec<String> = Vec::with_capacity(given);
        for a in cmd.args.iter().take(given) {
            let Some(p) = def.params.get(a.slot) else {
                return Err("argument position does not exist in Lua".into());
            };
            let (kind, _) = arg_kind_at(cmd, a.slot);
            let dk_name = ov
                .arg(&cmd.name, a.slot)
                .map(|o| o.name.clone())
                .unwrap_or_default();
            let lt = classify(&self.env, &parse_ty(&p.ty), p.optional);
            out.push(self.render_arg(&dk_name, kind, a, &p.name, &lt)?);
        }
        let line = format!("{fname}({})", out.join(", "));
        self.check_line(&line)?;
        Ok(line)
    }

    /// The generated line must parse and raise nothing (warnings and errors reject it).
    fn check_line(&self, line: &str) -> Result<(), String> {
        let doc = LuaDoc::parse(line);
        if !doc.syntax_errors.is_empty() || !doc.lex_errors.is_empty() {
            return Err("the generated Lua does not parse".into());
        }
        let bad: Vec<_> = diagnostics(&doc, self.index, self.provider)
            .into_iter()
            .filter(|d| d.severity != Severity::Info)
            .collect();
        match bad.first() {
            Some(d) => Err(format!(
                "the generated call would be flagged: {}",
                d.message
            )),
            None => Ok(()),
        }
    }

    fn single_player<'t>(&self, arg: Option<&'t Arg>) -> Result<&'t str, String> {
        let a = arg.ok_or("missing player")?;
        player_global(self.index, &a.text)
            .ok_or_else(|| format!("'{}' is not a single player", a.text))
    }

    fn variable_name<'t>(&self, arg: Option<&'t Arg>, tables: &[&str]) -> Result<&'t str, String> {
        let a = arg.ok_or("missing variable")?;
        let t = Tables::get();
        if a.func.is_none()
            && tables.iter().any(|n| t.is_named(n, &a.text))
            && a.text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            Ok(&a.text)
        } else {
            Err(format!("'{}' is not a plain variable name", a.text))
        }
    }

    fn special(&self, cmd: &Command) -> Option<Result<String, String>> {
        let num = |i: usize| -> Result<String, String> {
            let a = cmd.args.get(i).ok_or("missing value")?;
            if a.func.is_some() {
                return Err("value computed at run time".into());
            }
            int_text(&a.text).ok_or_else(|| format!("'{}' is not a number", a.text))
        };
        let r = match cmd.name.as_str() {
            "SET_FLAG" | "SET_CAMPAIGN_FLAG" | "ADD_TO_FLAG" | "ADD_TO_CAMPAIGN_FLAG" => (|| {
                let table: &[&str] = if cmd.name.contains("CAMPAIGN") {
                    &["campaign_flag_desc"]
                } else {
                    &["flag_desc"]
                };
                let p = self.single_player(cmd.args.first())?;
                let v = self.variable_name(cmd.args.get(1), table)?;
                let n = num(2)?;
                Ok(if cmd.name.starts_with("SET_") {
                    format!("{p}.{v} = {n}")
                } else {
                    format!("{p}.{v} = {p}.{v} + ({n})")
                })
            })(
            ),
            "ADD_GOLD_TO_PLAYER" => (|| {
                let p = self.single_player(cmd.args.first())?;
                Ok(format!("{p}:add_gold({})", num(1)?))
            })(),
            "WIN_GAME" if self.index.function("WinGame").is_some() => Ok("WinGame()".to_string()),
            "LOSE_GAME" if self.index.function("LoseGame").is_some() => {
                Ok("LoseGame()".to_string())
            }
            _ => return None,
        };
        Some(r.and_then(|line| self.check_line(&line).map(|_| line)))
    }

    fn command(&self, cmd: &Command) -> Result<String, String> {
        if cmd.desc.is_none() || !cmd.syntax_ok {
            return Err("the command is not valid DK script".into());
        }
        if cmd.preloaded {
            return Err(
                "runs while the level loads, before Lua; configure it in the campaign or \
                map config files"
                    .into(),
            );
        }
        if cmd.name == "NEXT_COMMAND_REUSABLE" {
            return Err("reusable commands need a hand written trigger".into());
        }
        if let Some(r) = self.special(cmd) {
            return r;
        }
        self.call(cmd)
    }
}

/// Trigger registration for an `IF` line, `Err(reason)` when it is not one of the simple patterns.
enum Trigger {
    ActionPoint { ap: String, player: String },
    Timer { turns: i64 },
    Condition { expr: String },
}

fn trigger_of(ctx: &Ctx, cmd: &Command) -> Result<Trigger, String> {
    match cmd.name.as_str() {
        "IF_ACTION_POINT" => {
            let ap = cmd
                .args
                .first()
                .and_then(|a| int_text(&a.text))
                .filter(|n| !n.starts_with('-') && n != "0")
                .ok_or("action point is not a plain number")?;
            let player = ctx.single_player(cmd.args.get(1))?.to_string();
            Ok(Trigger::ActionPoint { ap, player })
        }
        "IF" => {
            let player = ctx.single_player(cmd.args.first())?.to_string();
            if cmd.args.iter().any(|a| a.func.is_some())
                || cmd.args.get(4).map(|a| !a.text.is_empty()).unwrap_or(false)
            {
                return Err("condition uses a function or a second variable".into());
            }
            let var = ctx
                .variable_name(
                    cmd.args.get(1),
                    &[
                        "variable_desc",
                        "flag_desc",
                        "timer_desc",
                        "campaign_flag_desc",
                    ],
                )?
                .to_string();
            let op = cmd
                .args
                .get(2)
                .map(|a| a.text.as_str())
                .ok_or("missing comparison")?;
            let lua_op = match op {
                "==" | "<" | ">" | "<=" | ">=" => op,
                "<>" | "!=" => "~=",
                _ => return Err(format!("comparison '{op}' is not supported")),
            };
            let n = cmd
                .args
                .get(3)
                .and_then(|a| int_text(&a.text))
                .ok_or("compared value is not a plain number")?;
            if var == "GAME_TURN" {
                let v: i64 = n.parse().unwrap_or(0);
                match op {
                    "==" | ">=" if v >= 0 => return Ok(Trigger::Timer { turns: v }),
                    ">" if v >= 0 => return Ok(Trigger::Timer { turns: v + 1 }),
                    _ => {}
                }
            }
            Ok(Trigger::Condition {
                expr: format!("{player}.{var} {lua_op} {n}"),
            })
        }
        other => Err(format!("{other} has no trigger helper")),
    }
}

fn comment_of(doc: &Document, li: usize) -> String {
    let l = &doc.lines[li];
    let text = doc.text_lossy(l.span);
    text.trim_end_matches(['\r', '\n']).trim().to_string()
}

struct Frame {
    /// Name of the function the body goes into (`None`: commented block).
    func: Option<String>,
    body: Vec<String>,
    if_line: usize,
    reg: Option<String>,
    cond_fn: Option<String>,
}

/// Convert a DK script into a Lua draft. See the module docs: a starting point, not a translation.
pub fn convert(
    doc: &Document,
    index: &ApiIndex,
    provider: &dyn ConstantProvider,
    opts: &ConvertOptions,
) -> Conversion {
    let empty = LuaDoc::parse("");
    let ctx = Ctx {
        index,
        env: Env::new(&empty, index),
        provider,
    };
    let mut stats = ConvertStats::default();
    let mut notes: Vec<ConvertNote> = vec![];
    let mut top: Vec<String> = vec![];
    let mut funcs: Vec<String> = vec![];
    let mut stack: Vec<Frame> = vec![];
    let mut block_no = 0usize;

    let todo = |notes: &mut Vec<ConvertNote>, li: usize, cmd: &str, reason: &str| {
        notes.push(ConvertNote {
            line: li + 1,
            command: cmd.to_string(),
            reason: reason.to_string(),
        });
    };

    for (li, line) in doc.lines.iter().enumerate() {
        match line.kind {
            LineKind::Blank => {
                if stack.is_empty() && top.last().map(|l| !l.is_empty()).unwrap_or(false) {
                    top.push(String::new());
                }
            }
            LineKind::Comment => {
                let t = comment_of(doc, li);
                let t = t.strip_prefix("REM").map(|r| r.trim()).unwrap_or(&t);
                let c = format!("-- {t}");
                match stack.last_mut() {
                    Some(f) => f.body.push(c),
                    None => top.push(c),
                }
            }
            LineKind::Error => {
                stats.commands += 1;
                stats.commented += 1;
                let c = format!(
                    "-- {TODO_MARK} not valid DK script: {}",
                    comment_of(doc, li)
                );
                todo(&mut notes, li, "", "line is not valid DK script");
                match stack.last_mut() {
                    Some(f) => f.body.push(c),
                    None => top.push(c),
                }
            }
            LineKind::Command => {
                let cmd = line.cmd.as_ref().unwrap();
                stats.commands += 1;
                let original = comment_of(doc, li);
                if crate::validate::is_if_command(&cmd.name) {
                    stats.if_blocks += 1;
                    let nested = !stack.is_empty();
                    block_no += 1;
                    let mut frame = Frame {
                        func: None,
                        body: vec![],
                        if_line: li,
                        reg: None,
                        cond_fn: None,
                    };
                    let verdict = if nested {
                        Err(
                            "nested IF: the inner condition cannot be combined with the outer \
                            helper"
                                .to_string(),
                        )
                    } else if cmd.desc.is_none() || !cmd.syntax_ok {
                        Err("the IF line is not valid DK script".to_string())
                    } else {
                        trigger_of(&ctx, cmd)
                    };
                    match verdict {
                        Ok(t) => {
                            let f = format!("DkBlock_{block_no}");
                            frame.reg = Some(match &t {
                                Trigger::ActionPoint { ap, player } => {
                                    format!("RegisterOnActionPointEvent({f}, {ap}, {player})")
                                }
                                Trigger::Timer { turns } => {
                                    format!("RegisterTimerEvent({f}, {turns}, false)")
                                }
                                Trigger::Condition { .. } => {
                                    format!("RegisterOnConditionEvent({f}, DkCond_{block_no})")
                                }
                            });
                            if let Trigger::Condition { expr } = &t {
                                let name = format!("DkCond_{block_no}");
                                funcs.push(format!(
                                    "---Condition of the DK block at line {} \
                                    ({original}).\nfunction {name}()\n    return {expr}\nend\n",
                                    li + 1
                                ));
                                frame.cond_fn = Some(name);
                            }
                            // the registration lines are checked like any generated call
                            let reg = frame.reg.clone().unwrap();
                            let probe = LuaDoc::parse(&format!(
                                "function {f}() end\nfunction DkCond_{block_no}() end\n{reg}\n"
                            ));
                            let bad = diagnostics(&probe, index, provider)
                                .into_iter()
                                .find(|d| d.severity != Severity::Info);
                            if let Some(d) = bad {
                                frame.reg = None;
                                frame.cond_fn = None;
                                if matches!(t, Trigger::Condition { .. }) {
                                    funcs.pop();
                                }
                                frame.body.push(format!("-- {TODO_MARK} {original}"));
                                todo(
                                    &mut notes,
                                    li,
                                    &cmd.name,
                                    &format!("registration would be flagged: {}", d.message),
                                );
                            } else {
                                frame.func = Some(f);
                                frame
                                    .body
                                    .push(format!("-- DK line {}: {original}", li + 1));
                            }
                        }
                        Err(reason) => {
                            todo(&mut notes, li, &cmd.name, &reason);
                            frame
                                .body
                                .push(format!("-- {TODO_MARK} {original}   ({reason})"));
                        }
                    }
                    stack.push(frame);
                    continue;
                }
                if cmd.name == "ENDIF" {
                    let Some(frame) = stack.pop() else {
                        stats.commented += 1;
                        let c = format!("-- {TODO_MARK} {original}   (ENDIF without IF)");
                        top.push(c);
                        continue;
                    };
                    let converted_block = frame.func.is_some();
                    if converted_block {
                        let f = frame.func.clone().unwrap();
                        let mut s = format!(
                            "---Body of the DK block at line {}.\nfunction {f}()\n",
                            frame.if_line + 1
                        );
                        for l in &frame.body {
                            s.push_str("    ");
                            s.push_str(l);
                            s.push('\n');
                        }
                        s.push_str("end\n");
                        funcs.push(s);
                        top.push(format!(
                            "{}   -- DK line {}",
                            frame.reg.clone().unwrap(),
                            frame.if_line + 1
                        ));
                        stats.if_blocks_converted += 1;
                        // IF and ENDIF (the commands inside were counted as they came)
                        stats.converted += 2;
                    } else {
                        // a commented block: its commands were kept as comments
                        stats.commented += 2; // IF and ENDIF
                        let dest: Vec<String> = frame.body;
                        match stack.last_mut() {
                            Some(parent) => parent.body.extend(dest),
                            None => {
                                top.extend(dest);
                                top.push(format!("-- {TODO_MARK} {original}"));
                            }
                        }
                    }
                    continue;
                }
                // ordinary command
                let in_commented_block = stack.last().map(|f| f.func.is_none()).unwrap_or(false);
                match ctx.command(cmd) {
                    Ok(code) if !in_commented_block => {
                        stats.converted += 1;
                        let c = format!("{code}   -- {}", cmd.name);
                        match stack.last_mut() {
                            Some(f) => f.body.push(c),
                            None => top.push(c),
                        }
                    }
                    other => {
                        let reason = match other {
                            Err(r) => r,
                            Ok(_) => "inside an IF block that was not converted".to_string(),
                        };
                        stats.commented += 1;
                        todo(&mut notes, li, &cmd.name, &reason);
                        let c = format!("-- {TODO_MARK} {original}   ({reason})");
                        match stack.last_mut() {
                            Some(f) => f.body.push(c),
                            None => top.push(c),
                        }
                    }
                }
            }
        }
    }
    // unclosed IF blocks: keep their text as comments
    while let Some(frame) = stack.pop() {
        stats.commented += 1;
        let mut dest = vec![format!(
            "-- {TODO_MARK} IF block at line {} is never closed; kept as comments",
            frame.if_line + 1
        )];
        dest.extend(frame.body.into_iter().map(|l| {
            if l.starts_with("--") {
                l
            } else {
                format!("-- {l}")
            }
        }));
        match stack.last_mut() {
            Some(p) => p.body.extend(dest),
            None => top.extend(dest),
        }
    }
    // converted commands inside converted blocks were counted when seen; commented blocks'
    // commands are already counted as commented

    let mut out = String::new();
    out.push_str(
        "-- ============================================================================\n",
    );
    match &opts.source_name {
        Some(n) => out.push_str(&format!(
            "-- Lua DRAFT made by the DK-script assistant from {n}.\n"
        )),
        None => out.push_str("-- Lua DRAFT made by the DK-script assistant.\n"),
    }
    out.push_str("-- This is a STARTING POINT, not a faithful translation: read every line.\n");
    out.push_str(&format!(
        "-- Commands without a safe Lua equivalent stay as comments marked {TODO_MARK}.\n"
    ));
    out.push_str(
        "-- Trigger blocks only approximate the IF semantics (one shot, checked every \
        game tick).\n",
    );
    out.push_str(&format!(
        "-- {} of {} command lines converted, {} kept as comments.\n",
        stats.converted, stats.commands, stats.commented
    ));
    out.push_str(
        "-- ============================================================================\n\n",
    );
    for f in &funcs {
        out.push_str(f);
        out.push('\n');
    }
    out.push_str("function OnGameStart()\n");
    let mut prev_blank = true;
    for l in &top {
        if l.is_empty() {
            if !prev_blank {
                out.push('\n');
            }
            prev_blank = true;
            continue;
        }
        prev_blank = false;
        out.push_str("    ");
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("end\n");
    Conversion {
        lua: out,
        stats,
        notes,
    }
}
