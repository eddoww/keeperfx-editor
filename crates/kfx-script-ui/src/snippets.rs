//! The snippet library shared by both languages.
//!
//! A template uses `${n:default}` placeholders (Tab and Shift+Tab walk them in order of `n`) and
//! `$0` for the final caret; `\t` is the indentation unit. Every template with its placeholder
//! defaults filled in is validated by the tests against the DK script validator and the Lua
//! diagnostics.
use crate::textops::snippet_defaults;

/// Where a Lua template belongs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LuaPlace {
    /// Top level of the file.
    Top,
    /// Statements for the body of `function OnGameStart()`. Inserting into a file without that
    /// function wraps them in one.
    GameStart,
}

#[derive(Clone, Copy, Debug)]
pub struct Snippet {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    /// DK script form (`None` when the language has no equivalent).
    pub dk: Option<&'static str>,
    /// Lua form and where it goes.
    pub lua: Option<(&'static str, LuaPlace)>,
}

impl Snippet {
    /// Template for a language id (`"dk-script"` or `"lua"`).
    pub fn template(&self, lang_id: &str) -> Option<&'static str> {
        match lang_id {
            "dk-script" => self.dk,
            "lua" => self.lua.map(|l| l.0),
            _ => None,
        }
    }

    /// The text the snippet inserts into a language, before placeholders are walked.
    pub fn plain(&self, lang_id: &str) -> Option<String> {
        self.template(lang_id).map(snippet_defaults)
    }

    pub fn lua_place(&self) -> Option<LuaPlace> {
        self.lua.map(|l| l.1)
    }
}

pub const LIBRARY: &[Snippet] = &[
    Snippet {
        id: "setup",
        title: "Initial setup",
        category: "Setup",
        description: "Starting gold, creature limit, the creatures, rooms and \
            spells the keeper begins with.",
        dk: Some(
            "REM Initial setup\nSTART_MONEY(${1:PLAYER0}, ${2:3000})\n\
                MAX_CREATURES(${1:PLAYER0}, ${3:20})\nCREATURE_AVAILABLE(${1:PLAYER0}, \
                ${4:FLY}, 1, 1)\nROOM_AVAILABLE(${1:PLAYER0}, ${5:TREASURE}, 1, 1)\n\
                ROOM_AVAILABLE(${1:PLAYER0}, LAIR, 1, 1)\nROOM_AVAILABLE(${1:PLAYER0}, GARDEN, \
                1, 1)\nMAGIC_AVAILABLE(${1:PLAYER0}, ${6:POWER_IMP}, 1, 1)$0",
        ),
        lua: Some((
            "-- Initial setup\nStartMoney(${1:PLAYER0}, ${2:3000})\n\
                MaxCreatures(${1:PLAYER0}, ${3:20})\n\
                CreatureAvailable(${1:PLAYER0}, \"${4:FLY}\", true, 1)\n\
                RoomAvailable(${1:PLAYER0}, \"${5:TREASURE}\", 1, true)\n\
                RoomAvailable(${1:PLAYER0}, \"LAIR\", 1, true)\n\
                RoomAvailable(${1:PLAYER0}, \"GARDEN\", 1, true)\n\
                MagicAvailable(${1:PLAYER0}, \"${6:POWER_IMP}\", true, true)$0",
            LuaPlace::GameStart,
        )),
    },
    Snippet {
        id: "win-lose",
        title: "Win and lose conditions",
        category: "Conditions",
        description: "The keeper wins when every enemy dungeon is destroyed and \
            loses when their own is.",
        dk: Some(
            "IF(${1:PLAYER0}, ALL_DUNGEONS_DESTROYED == 1)\n\tWIN_GAME\n\
                ENDIF\nIF(${1:PLAYER0}, DUNGEON_DESTROYED == 1)\n\tLOSE_GAME\nENDIF$0",
        ),
        lua: Some((
            "function OnDungeonDestroyed(player)\n\tif player.index == \
                ${1:0} then\n\t\tLoseGame(PLAYER${1:0})\n\tend\n$0end",
            LuaPlace::Top,
        )),
    },
    Snippet {
        id: "win-on-flag",
        title: "Win when an objective is done",
        category: "Conditions",
        description: "Sets a flag when the objective is reached and wins on it. \
            Lua keeps the counter in the Game table so saves keep it.",
        dk: Some(
            "IF(${1:PLAYER0}, ${2:FLAG0} == ${3:1})\n\
                \tQUICK_OBJECTIVE(${4:1}, \"${5:Objective complete}\", ${6:PLAYER0})\n\
                \tWIN_GAME\nENDIF$0",
        ),
        lua: Some((
            "Game.${1:kills} = 0\nRegisterOnConditionEvent(\n\
                \tfunction(eventData, triggerData)\n\t\tQuickObjective(\"${2:Objective \
                complete}\")\n\t\tWinGame(PLAYER0)\n\tend,\n\tfunction()\n\t\treturn \
                Game.${1:kills} >= ${3:10}\n\tend)$0",
            LuaPlace::GameStart,
        )),
    },
    Snippet {
        id: "hero-waves",
        title: "Timed hero party wave",
        category: "Heroes",
        description: "A hero party enters from hero gate 1 after a delay. DK: a \
            timer condition; Lua: a repeating timer (20 turns per second).",
        dk: Some(
            "CREATE_PARTY(WAVE1)\n\tADD_TO_PARTY(WAVE1, ${1:KNIGHT}, \
                ${2:3}, 0, ATTACK_ENEMIES, 0)\n\tADD_TO_PARTY(WAVE1, ARCHER, 2, 0, \
                ATTACK_ENEMIES, 0)\nSET_TIMER(PLAYER_GOOD, TIMER0)\nIF(PLAYER_GOOD, TIMER0 >= \
                ${3:6000})\n\tADD_PARTY_TO_LEVEL(PLAYER_GOOD, WAVE1, ${4:-1}, 1)\nENDIF$0",
        ),
        lua: Some((
            "CreateParty(\"WAVE1\")\n\
                AddToParty(\"WAVE1\", \"${1:KNIGHT}\", ${2:3}, 0, \"ATTACK_ENEMIES\", 0)\n\
                AddToParty(\"WAVE1\", \"ARCHER\", 2, 0, \"ATTACK_ENEMIES\", 0)\n\
                RegisterTimerEvent(\n\tfunction(eventData, triggerData)\n\
                \t\tAddPartyToLevel(PLAYER_GOOD, \"WAVE1\", ${4:-1})\n\tend,\n\t${3:6000}, true)$0",
            LuaPlace::GameStart,
        )),
    },
    Snippet {
        id: "ambush",
        title: "Action point ambush",
        category: "Heroes",
        description: "Heroes appear on an action point when the keeper's creatures reach it.",
        dk: Some(
            "IF_ACTION_POINT(${1:1}, ${2:PLAYER0})\n\
                \tADD_CREATURE_TO_LEVEL(PLAYER_GOOD, ${3:KNIGHT}, ${1:1}, ${4:3}, 5, 0)\n\
                \tADD_CREATURE_TO_LEVEL(PLAYER_GOOD, ARCHER, ${1:1}, 3, 5, 0)\nENDIF$0",
        ),
        lua: Some((
            "RegisterOnActionPointEvent(\n\tfunction(eventData, \
                triggerData)\n\t\tAddCreatureToLevel(PLAYER_GOOD, \"${3:KNIGHT}\", ${1:1}, \
                ${4:3}, 0)\n\t\tAddCreatureToLevel(PLAYER_GOOD, \"ARCHER\", ${1:1}, 3, 0)\n\
                \tend,\n\t${1:1}, ${2:PLAYER0})$0",
            LuaPlace::GameStart,
        )),
    },
    Snippet {
        id: "reveal-objective",
        title: "Reveal map and set an objective",
        category: "Map and messages",
        description: "When an action point is reached, uncover the area around it \
            and show an objective.",
        dk: Some(
            "IF_ACTION_POINT(${1:2}, ${2:PLAYER0})\n\
                \tREVEAL_MAP_LOCATION(${2:PLAYER0}, ${1:2}, ${3:9})\n\
                \tQUICK_OBJECTIVE(${4:1}, \"${5:Find the hidden vault}\", ${1:2})\nENDIF$0",
        ),
        lua: Some((
            "RegisterOnActionPointEvent(\n\tfunction(eventData, \
                triggerData)\n\t\tRevealMapLocation(PLAYER0, ${1:2}, ${3:9})\n\
                \t\tQuickObjective(\"${5:Find the hidden vault}\", ${1:2})\n\tend,\n\t${1:2}, \
                ${2:PLAYER0})$0",
            LuaPlace::GameStart,
        )),
    },
    Snippet {
        id: "creature-pool",
        title: "Creature pool",
        category: "Availability",
        description: "How many of each creature can still be attracted to the \
            dungeon, and which are available to the player.",
        dk: Some(
            "ADD_CREATURE_TO_POOL(${1:FLY}, ${2:10})\n\
                ADD_CREATURE_TO_POOL(BUG, 10)\nADD_CREATURE_TO_POOL(SPIDER, 6)\n\
                CREATURE_AVAILABLE(${3:PLAYER0}, FLY, 1, 1)\nCREATURE_AVAILABLE(${3:PLAYER0}, \
                BUG, 1, 1)\nCREATURE_AVAILABLE(${3:PLAYER0}, SPIDER, 1, 1)$0",
        ),
        lua: Some((
            "local pool = { ${1:FLY} = ${2:10}, BUG = 10, SPIDER = 6 }\n\
                for name, amount in pairs(pool) do\n\tAddCreatureToPool(name, amount)\n\
                \tCreatureAvailable(${3:PLAYER0}, name, true, 1)\nend$0",
            LuaPlace::GameStart,
        )),
    },
    Snippet {
        id: "room-spell-table",
        title: "Room and spell availability",
        category: "Availability",
        description: "Which rooms and spells the keeper can build and cast, written as lists.",
        dk: Some(
            "ROOM_AVAILABLE(${1:PLAYER0}, TREASURE, 1, 1)\n\
                ROOM_AVAILABLE(${1:PLAYER0}, LAIR, 1, 1)\nROOM_AVAILABLE(${1:PLAYER0}, GARDEN, \
                1, 1)\nROOM_AVAILABLE(${1:PLAYER0}, ${2:TRAINING}, 1, 1)\n\
                MAGIC_AVAILABLE(${1:PLAYER0}, POWER_IMP, 1, 1)\nMAGIC_AVAILABLE(${1:PLAYER0}, \
                POWER_HAND, 1, 1)\nMAGIC_AVAILABLE(${1:PLAYER0}, ${3:POWER_SLAP}, 1, 1)$0",
        ),
        lua: Some((
            "local rooms = \
                { \"TREASURE\", \"LAIR\", \"GARDEN\", \"${2:TRAINING}\" }\nfor _, room in \
                ipairs(rooms) do\n\tRoomAvailable(${1:PLAYER0}, room, 1, true)\nend\nlocal \
                spells = { \"POWER_IMP\", \"POWER_HAND\", \"${3:POWER_SLAP}\" }\nfor _, spell \
                in ipairs(spells) do\n\tMagicAvailable(${1:PLAYER0}, spell, true, true)\nend$0",
            LuaPlace::GameStart,
        )),
    },
    Snippet {
        id: "hook-skeleton",
        title: "Game start hook",
        category: "Hooks",
        description: "The function the game calls once when the level starts.",
        dk: None,
        lua: Some((
            "function OnGameStart()\n\t${1:-- set up the level here}$0\nend",
            LuaPlace::Top,
        )),
    },
    Snippet {
        id: "creature-death",
        title: "React to a creature dying",
        category: "Hooks",
        description: "The game calls OnCreatureDeath for every creature that dies.",
        dk: None,
        lua: Some((
            "function OnCreatureDeath(unit)\n\tif unit.model \
                == \"${1:KNIGHT}\" then\n\t\tGame.${2:kills} = (Game.${2:kills} or 0) + 1\n\
                \tend\n$0end",
            LuaPlace::Top,
        )),
    },
];

/// Snippets that exist for a language id.
pub fn for_language(lang_id: &str) -> Vec<&'static Snippet> {
    LIBRARY
        .iter()
        .filter(|s| s.template(lang_id).is_some())
        .collect()
}

/// The insertion text for the Lua form of `s` given whether the file already defines the hook
/// the snippet goes into. A `GameStart` snippet without `function OnGameStart()` in the file is
/// wrapped in one.
pub fn lua_insertion(s: &Snippet, has_game_start: bool) -> Option<String> {
    let (t, place) = s.lua?;
    Some(match place {
        LuaPlace::GameStart if !has_game_start => {
            format!("function OnGameStart()\n{}\nend", indent_block(t))
        }
        _ => t.to_string(),
    })
}

fn indent_block(t: &str) -> String {
    t.lines()
        .map(|l| format!("\t{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Plain text of a Lua snippet wrapped the way it would be inserted into an empty file.
pub fn lua_validation_text(s: &Snippet) -> Option<String> {
    lua_insertion(s, false).map(|t| snippet_defaults(&t))
}
