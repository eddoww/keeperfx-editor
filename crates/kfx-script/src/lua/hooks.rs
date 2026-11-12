//! Engine hooks: global functions the game calls by name (`src/lua_triggers.c`), and the trigger
//! registration helpers of the shipped library.

pub struct Hook {
    pub name: &'static str,
    /// `(parameter, type)` in the order the engine pushes them.
    pub params: &'static [(&'static str, &'static str)],
    pub doc: &'static str,
    /// Line of the `lua_getglobal` in `src/lua_triggers.c`.
    pub c_line: u32,
}

pub const HOOKS: &[Hook] = &[
    Hook {
        name: "OnGameStart",
        params: &[],
        doc: "Runs once when the level starts (also in \
        campaign order after OnCampaignGameStart). Register triggers and set up the level here.",
        c_line: 115,
    },
    Hook {
        name: "OnCampaignGameStart",
        params: &[],
        doc: "Runs once at level start, before \
        OnGameStart; defined by the campaign's lua/init.lua.",
        c_line: 100,
    },
    Hook {
        name: "OnGameTick",
        params: &[],
        doc: "Runs on every game turn (20 per second). Keep it \
        cheap.",
        c_line: 134,
    },
    Hook {
        name: "OnPowerCast",
        params: &[
            ("pwkind", "power_kind"),
            ("caster", "Player"),
            ("target_thing", "Thing|nil"),
            ("stl_x", "integer"),
            ("stl_y", "integer"),
            ("splevel", "integer"),
        ],
        doc: "A keeper power was cast.",
        c_line: 149,
    },
    Hook {
        name: "OnSpecialActivated",
        params: &[
            ("player", "Player"),
            ("crate_thing", "Thing"),
            ("special_box_id", "integer"),
        ],
        doc: "A special box was opened.",
        c_line: 170,
    },
    Hook {
        name: "OnTrapPlaced",
        params: &[("trap", "Thing")],
        doc: "A trap was placed.",
        c_line: 188,
    },
    Hook {
        name: "OnCreatureDeath",
        params: &[("unit", "Creature")],
        doc: "A creature died.",
        c_line: 204,
    },
    Hook {
        name: "OnObjectDestroyed",
        params: &[("object", "Object")],
        doc: "An object was destroyed.",
        c_line: 220,
    },
    Hook {
        name: "OnCreatureRebirth",
        params: &[("unit", "Creature")],
        doc: "A creature was resurrected.",
        c_line: 236,
    },
    Hook {
        name: "OnCreatureFellIntoAbyss",
        params: &[("unit", "Creature")],
        doc: "A creature fell into the abyss.",
        c_line: 251,
    },
    Hook {
        name: "OnApplyDamage",
        params: &[
            ("thing", "Thing"),
            ("damage", "integer"),
            ("dealing_player", "Player"),
        ],
        doc: "Damage was applied to a thing.",
        c_line: 264,
    },
    Hook {
        name: "OnLevelUp",
        params: &[("creature", "Creature")],
        doc: "A creature gained a level.",
        c_line: 282,
    },
    Hook {
        name: "OnPickUp",
        params: &[("thing", "Thing"), ("player", "Player")],
        doc: "A thing was picked up by a hand.",
        c_line: 297,
    },
    Hook {
        name: "OnSlap",
        params: &[("thing", "Thing"), ("player", "Player")],
        doc: "A thing was slapped.",
        c_line: 313,
    },
    Hook {
        name: "OnSlabKindChange",
        params: &[("slab", "Slab"), ("old_slab_kind", "slab_type")],
        doc: "A slab changed kind.",
        c_line: 330,
    },
    Hook {
        name: "OnSlabOwnerChange",
        params: &[("slab", "Slab"), ("old_owner", "Player")],
        doc: "A slab changed owner.",
        c_line: 347,
    },
    Hook {
        name: "OnRoomOwnerChange",
        params: &[("room", "Room"), ("old_owner", "Player")],
        doc: "A room changed owner.",
        c_line: 364,
    },
    Hook {
        name: "OnShotHit",
        params: &[
            ("shot", "Shot"),
            ("shooter", "Thing|nil"),
            ("target", "Thing|nil"),
            ("next_stl_x", "integer"),
            ("next_stl_y", "integer"),
            ("rebound_hit", "boolean"),
        ],
        doc: "A shot hit something.",
        c_line: 380,
    },
    Hook {
        name: "OnDungeonDestroyed",
        params: &[("player", "Player")],
        doc: "A player lost their dungeon heart.",
        c_line: 29,
    },
    Hook {
        name: "OnChatMsg",
        params: &[("player", "Player"), ("message", "string")],
        doc: "A chat message was sent.",
        c_line: 45,
    },
];

pub fn hook(name: &str) -> Option<&'static Hook> {
    HOOKS.iter().find(|h| h.name == name)
}

/// `OnModGameStart_<modname>` is called for each loaded mod (`src/lua_triggers.c:61-70`).
pub fn is_hook_name(name: &str) -> bool {
    hook(name).is_some() || name.starts_with("OnModGameStart_")
}

/// Registration helpers whose function arguments become saved trigger callbacks.
pub fn is_trigger_registration(name: &str) -> bool {
    matches!(
        name,
        "CreateTrigger" | "TriggerAddCondition" | "OnDeath" | "OnDamage" | "OnDestroyed"
    ) || (name.starts_with("Register") && name.ends_with("Event"))
}

/// Snippets for registering triggers, `(label, snippet)`.
pub const TRIGGER_SNIPPETS: &[(&str, &str)] = &[
    (
        "RegisterTimerEvent",
        "RegisterTimerEvent(function()\n\t$0\nend, 100, false)",
    ),
    (
        "RegisterOnActionPointEvent",
        "RegisterOnActionPointEvent(function()\n\t$0\nend, 1, PLAYER0)",
    ),
    (
        "RegisterPowerCastEvent",
        "RegisterPowerCastEvent(function(eventData, triggerData)\n\t$0\nend, \"POWER_SLAP\")",
    ),
    (
        "RegisterCreatureDeathEvent",
        "RegisterCreatureDeathEvent(function(eventData, triggerData)\n\t$0\nend)",
    ),
    (
        "RegisterOnConditionEvent",
        "RegisterOnConditionEvent(function()\n\t$0\nend, function()\n\treturn false\nend)",
    ),
];
