//! Per-command documentation extracted from the KeeperFX wiki (`data/script_docs.json`).
//!
//! With the `wiki-docs` feature (default) the wiki text is embedded. Without it the documentation
//! is synthesised from `data/script_arg_overlay.json` (summary, argument names and kinds), the
//! command table (optional arguments) and the comments the C source has for the command
//! (`data/script_c_comments.json`, made by `tools/extract_script_c_comments.py`).
//!
//! The wiki spells two commands differently from the game's command table
//! (`RANDOMISE_FLAG`/`RANDOMIZE_FLAG`, `SET_PLAYER_COLOR`/`SET_PLAYER_COLOUR`); lookups accept
//! either spelling.
use kfx_script::signature_of;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[cfg(feature = "wiki-docs")]
const DOCS_JSON: &str = include_str!("../../../data/script_docs.json");
#[cfg(not(feature = "wiki-docs"))]
const DOCS_JSON: &str = "";
const C_COMMENTS_JSON: &str = include_str!("../../../data/script_c_comments.json");

/// Whether the wiki text is embedded (the `wiki-docs` feature).
pub const WIKI_EMBEDDED: bool = cfg!(feature = "wiki-docs");

#[derive(Clone, Debug, Deserialize)]
pub struct ParamDoc {
    pub name: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CommandDoc {
    pub page: String,
    pub signature: String,
    pub summary: String,
    #[serde(default)]
    pub params: Vec<ParamDoc>,
    #[serde(default)]
    pub markdown: String,
    #[serde(default)]
    pub examples: Vec<String>,
    /// Built from the overlay and the C comments instead of the wiki text.
    #[serde(default)]
    pub synthesized: bool,
}

impl CommandDoc {
    /// Wiki page URL with the command's anchor.
    pub fn wiki_url(&self, name: &str) -> String {
        wiki_url(&self.page, name)
    }
    /// Documentation of a parameter by (case-insensitive) display name.
    pub fn param(&self, name: &str) -> Option<&ParamDoc> {
        self.params
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

pub fn wiki_url(page: &str, name: &str) -> String {
    format!(
        "https://github.com/dkfans/keeperfx/wiki/{}#{}",
        page,
        name.to_ascii_lowercase()
    )
}

#[derive(Deserialize)]
struct Raw {
    commands: HashMap<String, CommandDoc>,
}

#[derive(Deserialize)]
struct CNote {
    text: String,
}
#[derive(Deserialize)]
struct CRaw {
    commands: HashMap<String, Vec<CNote>>,
}

/// What a value of an argument kind is, for the synthesised parameter text.
fn kind_text(kind: kfx_script::ArgKind) -> String {
    use kfx_script::ArgKind as K;
    match kind {
        K::Text => "A free text.".into(),
        K::Number | K::Count | K::Amount | K::Level | K::Turns => "A number.".into(),
        K::Player => "A player (PLAYER0 ... PLAYER3, PLAYER_GOOD, ALL_PLAYERS ...).".into(),
        K::Location => {
            "A player, creature, room, action point number or hero gate (negative number).".into()
        }
        K::SubtileX | K::SubtileY => "A subtile coordinate.".into(),
        K::SlabX | K::SlabY => "A slab coordinate.".into(),
        K::MessageId | K::QuickMsg => "A string ID.".into(),
        k => {
            let n = k.name().replace('_', " ");
            let art = if n.starts_with(['a', 'e', 'i', 'o', 'u']) {
                "An"
            } else {
                "A"
            };
            format!("{art} {n} name.")
        }
    }
}

/// Documentation built from the argument overlay, the command table and the C comments.
fn synthesize() -> HashMap<String, CommandDoc> {
    let tables = kfx_script::Tables::get();
    let notes: HashMap<String, Vec<CNote>> = serde_json::from_str::<CRaw>(C_COMMENTS_JSON)
        .map(|r| r.commands)
        .unwrap_or_default();
    let mut out = HashMap::new();
    for name in tables
        .commands
        .iter()
        .chain(tables.dk1.iter())
        .map(|c| c.name.clone())
    {
        let Some(sig) = signature_of(&name) else {
            continue;
        };
        let mut summary = sig.doc.clone();
        if let Some(n) = notes.get(&name) {
            let c: Vec<&str> = n.iter().map(|x| x.text.as_str()).collect();
            if !c.is_empty() {
                if !summary.is_empty() {
                    summary.push(' ');
                }
                summary.push_str(&format!("({})", c.join("; ")));
            }
        }
        let params = sig
            .params
            .iter()
            .map(|p| ParamDoc {
                name: p.name.clone(),
                text: kind_text(p.kind),
            })
            .collect();
        out.entry(name).or_insert(CommandDoc {
            page: "Level-Script-commands".into(),
            signature: sig.label,
            summary,
            params,
            markdown: String::new(),
            examples: vec![],
            synthesized: true,
        });
    }
    out
}

fn synthesized() -> &'static HashMap<String, CommandDoc> {
    static S: OnceLock<HashMap<String, CommandDoc>> = OnceLock::new();
    S.get_or_init(synthesize)
}

/// The wiki documentation, or the synthesised one without the `wiki-docs` feature.
fn table() -> &'static HashMap<String, CommandDoc> {
    if !WIKI_EMBEDDED {
        return synthesized();
    }
    static T: OnceLock<HashMap<String, CommandDoc>> = OnceLock::new();
    T.get_or_init(|| {
        serde_json::from_str::<Raw>(DOCS_JSON)
            .map(|r| r.commands)
            .unwrap_or_default()
    })
}

/// The other spelling of a command, if it has one.
pub fn alias_of(name: &str) -> Option<&'static str> {
    match name {
        "RANDOMISE_FLAG" => Some("RANDOMIZE_FLAG"),
        "RANDOMIZE_FLAG" => Some("RANDOMISE_FLAG"),
        "SET_PLAYER_COLOR" => Some("SET_PLAYER_COLOUR"),
        "SET_PLAYER_COLOUR" => Some("SET_PLAYER_COLOR"),
        _ => None,
    }
}

/// Documentation of `name` (any case, either spelling). A command of the table the wiki does
/// not describe (yet) gets the synthesised documentation, so a command new upstream is never
/// shown without any.
pub fn lookup(name: &str) -> Option<&'static CommandDoc> {
    let up = name.to_ascii_uppercase();
    let find = |t: &'static HashMap<String, CommandDoc>| {
        t.get(&up).or_else(|| alias_of(&up).and_then(|a| t.get(a)))
    };
    find(table()).or_else(|| {
        if WIKI_EMBEDDED {
            find(synthesized())
        } else {
            None
        }
    })
}

/// All documented command names (as spelled by the wiki), sorted.
pub fn documented_names() -> Vec<&'static str> {
    let mut v: Vec<&str> = table().keys().map(|s| s.as_str()).collect();
    v.sort();
    v
}

/// First sentence of a summary, for one-line lists.
pub fn short(summary: &str) -> String {
    let s = summary.trim();
    let mut end = s.len();
    let mut from = 0;
    while let Some(i) = s[from..].find(". ") {
        let at = from + i;
        let before = s[..at]
            .rsplit(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(before.as_str(), "e.g" | "i.e" | "etc" | "vs" | "no") {
            from = at + 2;
            continue;
        }
        end = at + 1;
        break;
    }
    let t = &s[..end];
    if t.chars().count() > 140 {
        let cut: String = t.chars().take(139).collect();
        format!("{cut}…")
    } else {
        t.to_string()
    }
}

/// Reference-panel category of a command.
pub fn category(name: &str) -> &'static str {
    let n = name;
    let has = |p: &str| n.contains(p);
    if n == "REM"
        || n.starts_with("IF")
        || n == "ENDIF"
        || matches!(
            n,
            "WIN_GAME"
                | "LOSE_GAME"
                | "NEXT_COMMAND_REUSABLE"
                | "LEVEL_VERSION"
                | "RUN_AFTER_VICTORY"
        )
    {
        "Flow and conditions"
    } else if n == "RUN_LUA_CODE" {
        "Lua"
    } else if has("PARTY")
        || n.starts_with("ADD_TUNNELLER")
        || n == "ADD_CREATURE_TO_LEVEL"
        || n == "CREATURE_ENTRANCE_LEVEL"
    {
        "Parties and spawning"
    } else if n.starts_with("SET_COMPUTER") || n.starts_with("COMPUTER") {
        "Computer players"
    } else if n.starts_with("SET_") && n.ends_with("_CONFIGURATION")
        || n.starts_with("NEW_")
        || n == "COPY_CREATURE_TYPE"
        || matches!(
            n,
            "SET_GAME_RULE"
                | "SET_SACRIFICE_RECIPE"
                | "REMOVE_SACRIFICE_RECIPE"
                | "SET_HAND_RULE"
                | "SET_HAND_GRAPHIC"
                | "SET_MUSIC"
                | "SET_BOX_TOOLTIP"
                | "SET_BOX_TOOLTIP_ID"
                | "SET_INCREASE_ON_EXPERIENCE"
        )
    {
        "Configuration"
    } else if n.ends_with("_AVAILABLE") || n.starts_with("RESEARCH") {
        "Availability and research"
    } else if has("FLAG") || has("TIMER") || has("VARIABLE") || has("COUNTDOWN") {
        "Flags, timers and variables"
    } else if has("OBJECTIVE")
        || has("INFORMATION")
        || has("MESSAGE")
        || n == "PRINT"
        || n == "TUTORIAL_FLASH_BUTTON"
    {
        "Messages and objectives"
    } else if n.starts_with("USE_")
        || n == "MAKE_SAFE"
        || n == "MAKE_UNSAFE"
        || n == "LOCATE_HIDDEN_WORLD"
    {
        "Powers and specials"
    } else if has("CREATURE")
        || n.starts_with("MOVE_")
        || n == "SWAP_CREATURE"
        || n == "TRANSFER_CREATURE"
        || n == "SET_DIGGER"
        || n == "DEAD_CREATURES_RETURN_TO_POOL"
    {
        "Creatures"
    } else if has("LEVEL") || has("BONUS") || n == "SET_LEVEL_ENSIGN" {
        "Levels and campaign"
    } else if matches!(
        n,
        "START_MONEY"
            | "ADD_GOLD_TO_PLAYER"
            | "ALLY_PLAYERS"
            | "LOCK_POSSESSION"
            | "MAX_CREATURES"
            | "SET_GENERATE_SPEED"
    ) || n.contains("PLAYER")
        || n.contains("HEART")
    {
        "Players and economy"
    } else if has("MAP")
        || has("SLAB")
        || has("EFFECT")
        || has("ACTION_POINT")
        || has("DOOR")
        || has("TRAP")
        || has("OBJECT")
        || has("TEXTURE")
        || has("HERO_GATE")
        || has("ZOOM")
    {
        "Map and terrain"
    } else {
        "Other"
    }
}

/// Category display order.
pub const CATEGORY_ORDER: &[&str] = &[
    "Flow and conditions",
    "Parties and spawning",
    "Creatures",
    "Players and economy",
    "Availability and research",
    "Flags, timers and variables",
    "Messages and objectives",
    "Powers and specials",
    "Map and terrain",
    "Levels and campaign",
    "Computer players",
    "Configuration",
    "Lua",
    "Other",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_lookup() {
        assert!(lookup("RANDOMISE_FLAG").is_some());
        assert!(lookup("RANDOMIZE_FLAG").is_some());
        assert!(lookup("randomize_flag").is_some());
        if WIKI_EMBEDDED {
            assert_eq!(
                lookup("RANDOMIZE_FLAG").unwrap().signature,
                lookup("RANDOMISE_FLAG").unwrap().signature
            );
        }
        assert!(lookup("SET_PLAYER_COLOUR").is_some());
        assert!(lookup("SET_PLAYER_COLOR").is_some());
        assert!(lookup("ADD_CREATURE_TO_LEVEL").is_some());
        assert!(lookup("NO_SUCH_COMMAND").is_none());
    }

    #[test]
    fn every_game_command_has_docs() {
        let cmds = &kfx_script::Tables::get().commands;
        let missing: Vec<&str> = cmds
            .iter()
            .filter(|c| lookup(&c.name).is_none())
            .map(|c| c.name.as_str())
            .collect();
        assert!(missing.is_empty(), "no documentation: {missing:?}");
        if WIKI_EMBEDDED {
            // The wiki lags behind new commands (and has no entry for PRINT and MESSAGE), so
            // only a sharp drop in coverage means the extractor broke.
            let from_wiki = cmds
                .iter()
                .filter(|c| lookup(&c.name).is_some_and(|d| !d.synthesized))
                .count();
            assert!(
                from_wiki * 10 >= cmds.len() * 9,
                "only {from_wiki} of {} commands documented by the wiki",
                cmds.len()
            );
            for name in ["ADD_CREATURE_TO_LEVEL", "IF", "WIN_GAME", "SET_FLAG"] {
                assert!(!lookup(name).unwrap().synthesized, "{name}");
            }
        }
    }

    #[test]
    fn synthesized_docs_are_complete_and_marked() {
        let d = synthesize();
        for c in &kfx_script::Tables::get().commands {
            let e = d
                .get(&c.name)
                .unwrap_or_else(|| panic!("{} not synthesised", c.name));
            assert!(
                e.synthesized && e.signature.starts_with(&c.name),
                "{}: {}",
                c.name,
                e.signature
            );
            assert!(!e.summary.is_empty(), "{} has no summary", c.name);
        }
        let a = &d["ADD_CREATURE_TO_LEVEL"];
        assert!(
            a.signature.contains("creature")
                && a.params
                    .iter()
                    .any(|p| p.name == "creature" && p.text.contains("creature")),
            "{a:?}"
        );
        assert!(d["USE_SPECIAL_MAKE_SAFE"]
            .summary
            .contains("Legacy command"));
    }

    #[test]
    fn categories_cover_everything() {
        for c in &kfx_script::Tables::get().commands {
            assert!(CATEGORY_ORDER.contains(&category(&c.name)), "{}", c.name);
        }
        assert_eq!(category("ENDIF"), "Flow and conditions");
        assert_eq!(category("ADD_PARTY_TO_LEVEL"), "Parties and spawning");
    }

    #[test]
    fn wiki_link() {
        assert!(lookup("ADD_CREATURE_TO_LEVEL")
            .unwrap()
            .wiki_url("ADD_CREATURE_TO_LEVEL")
            .ends_with("#add_creature_to_level"));
    }
}

#[cfg(test)]
mod short_tests {
    #[test]
    fn short_skips_abbreviations() {
        assert_eq!(
            super::short("The player's name, e.g. PLAYER1. More text."),
            "The player's name, e.g. PLAYER1."
        );
        assert_eq!(super::short("One. Two."), "One.");
        assert_eq!(super::short("No period"), "No period");
    }
}
