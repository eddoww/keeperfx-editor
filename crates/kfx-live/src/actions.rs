//! Requests for what an editor wants during a play-test, built from the game's own commands.
//!
//! What is possible without cheat mode, what needs it, and what is not possible at all, from the
//! code (see [`crate::proto`] for the transport):
//!
//! | goal | how | needs cheats |
//! |---|---|---|
//! | run a DK script command | `map_command` (one line, immediate) | no |
//! | run Lua | `map_command RUN_LUA_CODE("..")` (eats the level's 8192 byte string pool) | no |
//! | run Lua without the pool cost | console `lua <code>` (`src/console_cmd.c:2533`) | **yes** |
//! | add gold | script `ADD_GOLD_TO_PLAYER(P, n)` | no |
//! | reveal the map | script `REVEAL_MAP_RECT(P, cx, cy, w, h)` (subtiles, centred, clamped) | no |
//! | give a room / a power | script `ROOM_AVAILABLE(P, ROOM, 1, 1)` / `MAGIC_AVAILABLE(P, POWER, 1, 1)` | no |
//! | add a creature | script `ADD_CREATURE_TO_LEVEL(P, CREATURE, LOCATION, count, level, gold)` | no |
//! | read / write flags, timers, any script variable | `read_var` / `set_var` (set: flags only) | no |
//! | jump the camera to an action point / hero gate / heart | script `ZOOM_TO_LOCATION(P, n)` (`src/lvl_script_commands.c:7169`; `n` is an action point number, negative a hero gate) | no |
//! | jump the camera to any subtile | console `zoomto.subtilecoord x y` (`src/console_cmd.c:2315`) | **yes** |
//! | pause | console `pause` (toggle) | no, but the API cannot unpause: both `console_command` and `map_command` are refused while paused |
//! | game speed | console `fps <turns per second>` (`src/console_cmd.c:579`) | no |
//! | save / load a game | console `game.save <slot>` / `game.load <slot>` | no |
//! | list things and their positions | Lua `GetThingsOfClass` printing to the game log, read with [`crate::logtail::LogTail`] | no |
//! | push an edited script | **not possible**, see below | |
//!
//! # Pushing an edited script
//!
//! There is no reload request. `script_scan_line` (what `map_command` uses) takes **one line**,
//! executes it immediately when the current condition is `ALWAYS` and otherwise stores it in the
//! script's value table; it does not register `IF` / `WHILE` / `REPEAT` triggers the way loading a
//! file does, and a stray `IF(...)` line changes the script's global condition for every later
//! line. Re-running a whole `mapNNNNN.txt` mid-game would repeat its set-up commands
//! (`ADD_CREATURE_TO_LEVEL`, `ROOM_AVAILABLE`, ...) on top of the running state, and commands such
//! as `NEW_*_TYPE` are "preloaded" ones that `map_command` ignores. So an edited script takes
//! effect by restarting the level (write the files, restart the game). What can be changed live:
//! flags and campaign flags, resources, availability, one-off effects, and Lua helper functions or
//! event registrations.
use crate::proto::{Player, Request};

/// Scripts' room names (`fxdata/terrain.cfg` `[roomN]` `Name`, as used in `ROOM_AVAILABLE`).
pub const ROOMS: &[&str] = &[
    "TREASURE",
    "LAIR",
    "GARDEN",
    "LIBRARY",
    "TRAINING",
    "WORKSHOP",
    "SCAVENGER",
    "PRISON",
    "TORTURE",
    "TEMPLE",
    "GRAVEYARD",
    "BARRACKS",
    "GUARD_POST",
    "BRIDGE",
];

/// Power names of the vanilla `fxdata/magic.cfg` (`MAGIC_AVAILABLE`).
pub const POWERS: &[&str] = &[
    "POWER_HAND",
    "POWER_IMP",
    "POWER_SLAP",
    "POWER_SIGHT",
    "POWER_CALL_TO_ARMS",
    "POWER_HOLD_AUDIENCE",
    "POWER_CAVE_IN",
    "POWER_HEAL_CREATURE",
    "POWER_LIGHTNING",
    "POWER_SPEED",
    "POWER_PROTECT",
    "POWER_CONCEAL",
    "POWER_DISEASE",
    "POWER_CHICKEN",
    "POWER_DESTROY_WALLS",
    "POWER_POSSESS",
    "POWER_ARMAGEDDON",
];

fn ident(s: &str) -> Result<&str, String> {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        Ok(s)
    } else {
        Err(format!(
            "'{s}' is not a script identifier (letters, digits, _)"
        ))
    }
}

/// A DK script line as a request.
pub fn script(line: &str) -> Request {
    Request::MapCommand(line.to_string())
}

/// A console command as a request (the `!` prefix is optional).
pub fn console(command: &str) -> Request {
    Request::ConsoleCommand {
        command: command.trim_start_matches('!').to_string(),
        player: None,
    }
}

pub fn add_gold(player: &Player, amount: i32) -> Request {
    script(&format!(
        "ADD_GOLD_TO_PLAYER({},{amount})",
        player.script_name()
    ))
}

/// Reveal the whole map for a player. `mapsize` is the map size in slabs (`get_level_info`'s
/// `mapsize_x` / `mapsize_y`); the rectangle is given in subtiles, centred, and clamped by the
/// game.
pub fn reveal_map(player: &Player, mapsize: (i32, i32)) -> Request {
    let (w, h) = (mapsize.0 * 3 + 6, mapsize.1 * 3 + 6);
    script(&format!(
        "REVEAL_MAP_RECT({},{},{},{w},{h})",
        player.script_name(),
        w / 2,
        h / 2
    ))
}

pub fn give_room(player: &Player, room: &str) -> Result<Request, String> {
    Ok(script(&format!(
        "ROOM_AVAILABLE({},{},1,1)",
        player.script_name(),
        ident(room)?
    )))
}

pub fn give_power(player: &Player, power: &str) -> Result<Request, String> {
    Ok(script(&format!(
        "MAGIC_AVAILABLE({},{},1,1)",
        player.script_name(),
        ident(power)?
    )))
}

/// `location` is a script location: a player name (that player's heart), an action point number,
/// ...
pub fn add_creature(
    player: &Player,
    creature: &str,
    location: &str,
    count: u32,
    level: u32,
    gold: u32,
) -> Result<Request, String> {
    Ok(script(&format!(
        "ADD_CREATURE_TO_LEVEL({},{},{},{count},{level},{gold})",
        player.script_name(),
        ident(creature)?,
        ident(location)?
    )))
}

pub fn set_flag(player: &Player, flag: u8, value: i32) -> Request {
    Request::SetVar {
        var: format!("FLAG{}", flag.min(7)),
        player: Some(player.clone()),
        value,
    }
}

pub fn read_var(player: &Player, var: &str) -> Request {
    Request::ReadVar {
        var: var.to_string(),
        player: Some(player.clone()),
    }
}

/// Camera to action point `n` (negative: hero gate `-n`). Works without cheats.
pub fn zoom_to_action_point(player: &Player, n: i32) -> Request {
    script(&format!("ZOOM_TO_LOCATION({},{n})", player.script_name()))
}

/// Camera to a subtile. **Needs cheat mode** (`-alex`): the reply is
/// `FAILED_TO_EXECUTE_CONSOLE_COMMAND` otherwise.
pub fn zoom_to_subtile(x: i32, y: i32) -> Request {
    console(&format!("zoomto.subtilecoord {x} {y}"))
}

/// Camera to a slab. Needs cheat mode.
pub fn zoom_to_slab(x: i32, y: i32) -> Request {
    console(&format!("zoomto.slabcoord {x} {y}"))
}

/// Toggle pause. The API refuses commands while paused, so it cannot unpause again.
pub fn pause() -> Request {
    console("pause")
}

/// Turns per second (the game's default is 20; `fps` without a value resets it).
pub fn set_speed(turns_per_second: u32) -> Request {
    console(&format!("fps {}", turns_per_second.clamp(1, 500)))
}

pub fn save_game(slot: u32) -> Request {
    console(&format!("game.save {slot}"))
}

pub fn load_game(slot: u32) -> Request {
    console(&format!("game.load {slot}"))
}

/// Quit the game (single player).
pub fn quit() -> Request {
    console("quit")
}

/// Run Lua through the script command `RUN_LUA_CODE("..")`. The code goes into a double quoted
/// script string, so it cannot contain `"` or newlines (use `'`), and it permanently uses `len + 1`
/// bytes of the level's 8192 byte string pool: see [`LuaChannel`] for the usual pattern.
pub fn run_lua(code: &str) -> Result<Request, String> {
    if code.contains('"') {
        return Err(
            "Lua code sent through RUN_LUA_CODE cannot contain double quotes (use single quotes)"
                .into(),
        );
    }
    if code.contains('\n') || code.contains('\r') {
        return Err("Lua code sent through RUN_LUA_CODE must be one line".into());
    }
    Ok(script(&format!("RUN_LUA_CODE(\"{code}\")")))
}

/// Run Lua through the console `lua` command. Needs cheat mode; costs no script string space.
pub fn run_lua_console(code: &str) -> Request {
    console(&format!("lua {code}"))
}

/// How an editor drives Lua in the game: define helper functions once, call them with tiny
/// snippets, and bring results back (the API has no Lua result channel, see below).
///
/// * Results of one integer: Lua assigns it to a scratch flag (`PLAYER0.FLAG7 = expr`, an
///   assignment to a player field is `set_variable`, `src/lua_api_player.c:194`) and the client reads
///   it with `read_var`. The scratch flag is overwritten: pick one the level script does not use.
/// * Larger results (thing lists): Lua `print`s lines starting with [`crate::logtail::MARKER`] into
///   the game log, read with [`crate::logtail::LogTail`].
#[derive(Clone, Debug)]
pub struct LuaChannel {
    /// Player and flag number of the scratch flag.
    pub scratch: (Player, u8),
    /// Use the console `lua` command (cheats) instead of `RUN_LUA_CODE`.
    pub use_console: bool,
    /// Bytes of the script string pool this channel has used so far.
    pub pool_used: usize,
    defined: bool,
}

impl Default for LuaChannel {
    fn default() -> Self {
        LuaChannel {
            scratch: (Player::P0, 7),
            use_console: false,
            pool_used: 0,
            defined: false,
        }
    }
}

/// The Lua helper installed once: lists things of the given classes into the log.
pub const DUMP_HELPER: &str = "function KfxLiveDump(c) for _,t in ipairs(GetThingsOfClass(c)) do \
    print('KFXLIVE THING '..c..' '..tostring(t.model)..' '..tostring(t.owner.playerId)..' \
    '..t.pos.stl_x..' '..t.pos.stl_y) end end";

impl LuaChannel {
    fn lua(&mut self, code: &str) -> Result<Request, String> {
        if self.use_console {
            Ok(run_lua_console(code))
        } else {
            self.pool_used += code.len() + 1;
            run_lua(code)
        }
    }

    /// Requests to run, in order: install the helper (once) and dump the things of `class`
    /// (`Creature`, `Object`, `Trap`, `Door`, `EffectGen`). The positions then appear in the log.
    pub fn dump_things(&mut self, class: &str) -> Result<Vec<Request>, String> {
        let mut v = vec![];
        if !self.defined {
            v.push(self.lua(DUMP_HELPER)?);
            self.defined = true;
        }
        v.push(self.lua(&format!("KfxLiveDump('{}')", ident(class)?))?);
        Ok(v)
    }

    /// Requests that evaluate an integer Lua expression into the scratch flag; then
    /// [`LuaChannel::result_request`] reads it.
    pub fn eval_int(&mut self, expr: &str) -> Result<Request, String> {
        let (p, f) = self.scratch.clone();
        self.lua(&format!("{}.FLAG{}=({expr})", p.script_name(), f))
    }

    pub fn result_request(&self) -> Request {
        read_var(&self.scratch.0, &format!("FLAG{}", self.scratch.1))
    }

    /// Fraction of the 8192 byte pool used by this channel (the level script uses some too).
    pub fn pool_fraction(&self) -> f32 {
        self.pool_used as f32 / 8192.0
    }
}

/// The `-alex` hint shown with failed console commands.
pub fn cheat_hint(cmd: &str) -> Option<&'static str> {
    let first = cmd
        .trim_start_matches('!')
        .split_whitespace()
        .next()?
        .to_ascii_lowercase();
    const CHEATS: &[&str] = &[
        "lua",
        "reveal",
        "conceal",
        "player.addgold",
        "player.gold.add",
        "create.creature",
        "creature.create",
        "create.gold",
        "thing.create",
        "thing.destroy",
        "thing.move",
        "room.available",
        "power.give",
        "spell.give",
        "door.give",
        "trap.give",
        "give.door",
        "give.trap",
        "zoomto.subtilecoord",
        "zoomto.slabcoord",
        "zoomto.actionpoint",
        "zoomto.herogate",
        "player.flag",
        "slab.place",
        "place.slab",
        "comp.me",
        "comp.kill",
    ];
    CHEATS
        .contains(&first.as_str())
        .then_some("needs cheat mode: start the game with the -alex command line option")
}
