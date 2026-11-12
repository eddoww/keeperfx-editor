//! Hand-annotated semantic argument kinds (`data/script_arg_overlay.json`).
//!
//! The command table only has one letter per argument; the `A` letter hides many typed values
//! that the `*_check` functions of `src/lvl_script_commands.c` resolve themselves. The overlay
//! names each argument and gives it a semantic [`ArgKind`].
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

const OVERLAY_JSON: &str = include_str!("../../../data/script_arg_overlay.json");

macro_rules! kinds {
    ($($v:ident = $s:literal),* $(,)?) => {
        /// Semantic kind of an argument.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum ArgKind { $($v),* }
        impl ArgKind {
            pub fn from_name(s: &str) -> Option<ArgKind> {
                match s {
                    $($s => Some(ArgKind::$v),)*
                    _ => None,
                }
            }
            pub fn name(self) -> &'static str {
                match self {
                    $(ArgKind::$v => $s),*
                }
            }
        }
    };
}

kinds! {
    Text = "text", Number = "number", Creature = "creature", Player = "player", Room = "room",
    Slab = "slab", Location = "location", Operator = "operator", Bool = "bool",
    PartyDef = "party_def", Party = "party", Variable = "variable",
    AvailVariable = "avail_variable", CtrlVariable = "ctrl_variable", FlagVar = "flag_var",
    Timer = "timer", CampaignFlag = "campaign_flag", Power = "power", Spell = "spell",
    Trap = "trap", Door = "door", Object = "object", Effect = "effect", EffectGen = "effect_gen",
    CreatureProperty = "creature_property", RuleName = "rule_name", RuleValue = "rule_value",
    TexturePack = "texture_pack", HeroObjective = "hero_objective", MessageId = "message_id",
    QuickMsg = "quick_msg", Sound = "sound", Music = "music", Criterion = "criterion",
    FillType = "fill_type", LockState = "lock_state", FreeState = "free_state",
    Orientation = "orientation", SpawnType = "spawn_type", ResearchType = "research_type",
    ResearchItem = "research_item", SacrificeAction = "sacrifice_action",
    SacrificeParam = "sacrifice_param", Tendency = "tendency", Modifier = "modifier",
    OnExperience = "on_experience", MsgType = "msg_type", HandRule = "hand_rule",
    RuleSlot = "rule_slot", RuleAction = "rule_action", Icon = "icon", ChatIcon = "chat_icon",
    Ensign = "ensign", GuiButton = "gui_button", ConfigProperty = "config_property",
    ConfigValue = "config_value", ComputerModel = "computer_model", ComputerName = "computer_name",
    HeadFor = "head_for", Instance = "instance", Count = "count", Amount = "amount",
    Level = "level", Turns = "turns", SubtileX = "subtile_x", SubtileY = "subtile_y",
    SlabX = "slab_x", SlabY = "slab_y", ApNumber = "ap_number", HeroGate = "hero_gate",
    BoxNumber = "box_number", IfValue = "if_value", ScriptOperator = "script_operator",
    PlayerColor = "player_color", HandGraphic = "hand_graphic", ConcealMode = "conceal_mode",
    NewName = "new_name", Lua = "lua",
}

impl ArgKind {
    /// Kind implied by a plain type letter.
    pub fn from_letter(c: u8) -> ArgKind {
        match c.to_ascii_uppercase() {
            b'N' => ArgKind::Number,
            b'C' => ArgKind::Creature,
            b'P' => ArgKind::Player,
            b'R' => ArgKind::Room,
            b'S' => ArgKind::Slab,
            b'L' => ArgKind::Location,
            b'O' => ArgKind::Operator,
            b'B' => ArgKind::Bool,
            _ => ArgKind::Text,
        }
    }
    /// Kinds that stand for a map coordinate the map view can pick.
    pub fn is_coordinate(self) -> bool {
        matches!(
            self,
            ArgKind::SubtileX | ArgKind::SubtileY | ArgKind::SlabX | ArgKind::SlabY
        )
    }
}

#[derive(Debug, Deserialize)]
struct RawCmd {
    doc: String,
    args: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct RawOverlay {
    lists: HashMap<String, Vec<String>>,
    commands: HashMap<String, RawCmd>,
}

/// One annotated argument.
#[derive(Clone, Debug)]
pub struct OverlayArg {
    pub name: String,
    pub kind: ArgKind,
    /// The display name was guessed (the C code does not name it).
    pub guessed: bool,
}

#[derive(Clone, Debug)]
pub struct CmdInfo {
    pub doc: String,
    pub args: Vec<OverlayArg>,
}

#[derive(Debug)]
pub struct Overlay {
    pub commands: HashMap<String, CmdInfo>,
    pub lists: HashMap<String, Vec<String>>,
}

impl Overlay {
    pub fn get() -> &'static Overlay {
        static O: OnceLock<Overlay> = OnceLock::new();
        O.get_or_init(|| {
            let raw: RawOverlay =
                serde_json::from_str(OVERLAY_JSON).expect("embedded script_arg_overlay.json");
            let commands = raw
                .commands
                .into_iter()
                .map(|(k, v)| {
                    let args = v
                        .args
                        .iter()
                        .map(|a| {
                            let (n, kind) = a.split_once(':').expect("name:kind");
                            let (n, guessed) = match n.strip_suffix('*') {
                                Some(x) => (x, true),
                                None => (n, false),
                            };
                            OverlayArg {
                                name: n.to_string(),
                                kind: ArgKind::from_name(kind)
                                    .unwrap_or_else(|| panic!("overlay kind {kind}")),
                                guessed,
                            }
                        })
                        .collect();
                    (k, CmdInfo { doc: v.doc, args })
                })
                .collect();
            Overlay {
                commands,
                lists: raw.lists,
            }
        })
    }

    pub fn cmd(&self, name: &str) -> Option<&CmdInfo> {
        self.commands.get(name)
    }

    pub fn list(&self, name: &str) -> &[String] {
        self.lists.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Annotation of argument `slot` of `cmd`. Repeating commands reuse their last annotation.
    pub fn arg(&self, cmd: &str, slot: usize) -> Option<&OverlayArg> {
        let info = self.commands.get(cmd)?;
        info.args.get(slot).or_else(|| {
            let repeats = crate::tables::Tables::get()
                .find_full(cmd)
                .map(|d| d.repeats())
                .unwrap_or(false);
            if repeats {
                info.args.last()
            } else {
                None
            }
        })
    }
}
