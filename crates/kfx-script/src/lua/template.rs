//! New-file template for `mapNNNNN.lua`, with optional hints for porting a DK script.
//!
//! The wiki (`Lua-level-script.md`) only says that "most commands from DKScript" exist in Lua with
//! the same meaning and that some behave differently; it gives no complete mapping. The template
//! hints are therefore light (the fuller, still conservative, conversion is [`super::convert`]):
//! they only apply the naming rule that the shipped bindings follow (`SET_GENERATE_SPEED` is
//! `SetGenerateSpeed`) when such a function exists in the index, and otherwise copy the command as
//! a comment.
use super::index::ApiIndex;
use crate::doc::Document;

#[derive(Clone, Debug, Default)]
pub struct TemplateOptions {
    pub map_number: u32,
    /// Free text written into the header comment (map name).
    pub title: Option<String>,
    /// Include commented examples of trigger registration.
    pub examples: bool,
}

/// `SET_GENERATE_SPEED` -> `SetGenerateSpeed`.
pub fn dk_to_lua_name(cmd: &str) -> String {
    cmd.split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase(),
                None => String::new(),
            }
        })
        .collect()
}

/// File name the game loads for a level script: `map00042.lua` (`src/lua_base.c`, `map%05lu.lua`).
pub fn script_file_name(map_number: u32) -> String {
    format!("map{:05}.lua", map_number)
}

/// Skeleton for a new level script.
pub fn new_script(opts: &TemplateOptions) -> String {
    let mut s = String::new();
    s.push_str(&format!("-- {}", script_file_name(opts.map_number)));
    if let Some(t) = &opts.title {
        s.push_str(&format!(": {t}"));
    }
    s.push_str(
        "\n-- Level script. The game loads fxdata/lua/init.lua, then the campaign's \
        lua/init.lua, then this file.\n",
    );
    s.push_str(
        "--\n-- Saves only store the global `Game` table: keep state that must survive \
        save/load in it\n",
    );
    s.push_str("-- (Game.counter = 0), not in locals or other globals.\n");
    s.push_str(
        "-- Give trigger callbacks global names, or write anonymous callbacks on their own \
        lines and\n",
    );
    s.push_str(
        "-- capture only numbers, strings and booleans (docs/lua_saves_for_mappers.md).\n\n",
    );
    s.push_str(
        "---Called once when the level starts. Set up the level and register triggers \
        here.\nfunction OnGameStart()\n",
    );
    if opts.examples {
        s.push_str("    -- Game.wave = 0\n");
        s.push_str(
            "    -- RegisterTimerEvent(OnWaveTimer, 20 * 60, true)   -- every minute (20 \
            turns per second)\n",
        );
        s.push_str("    -- RegisterOnActionPointEvent(OnActionPointReached, 1, PLAYER0)\n");
        s.push_str("    -- AddCreatureToLevel(PLAYER_GOOD, \"KNIGHT\", 1, 2, 1, 0)\n");
    } else {
        s.push_str("    \n");
    }
    s.push_str("end\n");
    if opts.examples {
        s.push_str(
            "\n-- function OnWaveTimer(eventData, triggerData)\n--     Game.wave = \
            Game.wave + 1\n-- end\n",
        );
        s.push_str("\n-- function OnActionPointReached(eventData, triggerData)\n-- end\n");
    }
    s
}

/// Hint lines (Lua comments) for porting the commands of a DK script. Each line holds the original
/// command and, when the naming rule finds a function in the index, its Lua signature.
pub fn conversion_hints(dk: &Document, index: &ApiIndex) -> Vec<String> {
    let mut out = Vec::new();
    for (_, c) in dk.commands() {
        let args: Vec<&str> = c.args.iter().map(|a| a.text.as_str()).collect();
        let original = format!("{}({})", c.name, args.join(", "));
        let lua = dk_to_lua_name(&c.name);
        match index.function(&lua) {
            Some(f) => out.push(format!("-- DK: {original}  ->  {}", f.signature())),
            None => out.push(format!(
                "-- DK: {original}  (no Lua function named {lua} in the \
                library; see the wiki's Lua-level-script page)"
            )),
        }
    }
    out
}

/// `new_script` with the hint lines placed after the header comment.
pub fn new_script_with_hints(opts: &TemplateOptions, dk: &Document, index: &ApiIndex) -> String {
    let base = new_script(opts);
    let hints = conversion_hints(dk, index);
    if hints.is_empty() {
        return base;
    }
    let marker = "---Called once when the level starts.";
    let at = base.find(marker).unwrap_or(base.len());
    let mut s = String::new();
    s.push_str(&base[..at]);
    s.push_str("-- Commands of the DK script this level used to have (hints, not a conversion):\n");
    for h in hints {
        s.push_str(&h);
        s.push('\n');
    }
    s.push('\n');
    s.push_str(&base[at..]);
    s
}
