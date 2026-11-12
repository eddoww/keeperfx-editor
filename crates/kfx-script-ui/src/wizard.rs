//! Script wizard: a form for the usual level setup commands.
//!
//! * [`WizardModel`]: per player ([`Player`]) start gold, creature limit, computer player, room /
//!   power / trap / door / creature availability, plus the shared creature pool, portal speed,
//!   alliances and the usual win/lose conditions. Pure data, no UI.
//! * [`WizardModel::read`] / [`WizardModel::read_lua`]: the values the script already sets, from
//!   unconditional statements anywhere in the file (inside and outside the wizard block).
//!   Anything else is ignored.
//! * [`WizardModel::plan`] / [`WizardModel::plan_lua`]: one [`WizardPlan`] that replaces the wizard
//!   block (between [`BEGIN_MARK`] and [`END_MARK`]) in place, or inserts a new block after the
//!   header comments and `LEVEL_VERSION`. Nothing outside the block is ever touched: when the
//!   rest of the script also sets something the block sets, the plan lists a [`Conflict`].
//! * [`ScriptWizard`]: the egui form.
//!
//! The block only holds what differs from the rest of the script. Values that only come from
//! lines outside the block stay there; for the commands that *add* to a value (`START_MONEY`,
//! `ADD_CREATURE_TO_POOL`, the amounts of `TRAP_AVAILABLE` / `DOOR_AVAILABLE`) the block adds the
//! difference, so the total is what the form shows.
//!
//! Command semantics (KeeperFX source):
//!
//! * `START_MONEY(player, gold)` ("PN", `src/lvl_script_commands.c:7033`): adds `gold` to the
//!   player's off-map gold at load, capped at `SENSIBLE_GOLD` (`command_set_start_money`,
//!   `src/lvl_script_commands_old.c:216`). Lua `StartMoney` *sets* it instead
//!   (`src/lua_api.c:126`).
//! * `MAX_CREATURES(player, n)` ("PN", `src/lvl_script_commands.c:7046`): sets
//!   `max_creatures_attracted` (`src/lvl_script_value.c:231`).
//! * `ADD_CREATURE_TO_POOL(creature, n)` ("CN", `src/lvl_script_commands.c:7071`): adds `n` to the
//!   shared pool (`add_creature_to_pool`, `src/room_entrance.c:429`), `|n| < CREATURES_COUNT`
//!   (`src/lvl_script_commands_old.c:552`).
//! * `ROOM_AVAILABLE(player, room, can_research, available)` ("PRNN",
//!   `src/lvl_script_commands.c:7034`): `set_room_available` (`src/config_terrain.c:544`): research
//!   0 makes the room unbuildable; otherwise `available` makes it buildable now. KeeperFX also
//!   knows research modes 2..4 (`src/creature_states_rsrch.c:118`), kept as
//!   [`Availability::Other`].
//! * `MAGIC_AVAILABLE(player, power, can_research, available)` ("PANN",
//!   `src/lvl_script_commands.c:7036`): `set_power_available` (`src/config_magic.c:1492`).
//! * `TRAP_AVAILABLE` / `DOOR_AVAILABLE(player, kind, can_build, amount)` ("PANN",
//!   `src/lvl_script_commands.c:7037`, `:7048`): sets the manufacturable flag and *adds* `amount`
//!   ready-made items (`src/dungeon_data.c:346`, `:375`).
//! * `CREATURE_AVAILABLE(player, creature, can_attract, forced)` ("PCNN",
//!   `src/lvl_script_commands.c:7035`, `set_creature_available`, `src/config_creature.c:2022`). In
//!   level version 0 the game takes the *fourth* argument as the "can attract" flag and forces none
//!   (`src/lvl_script_commands_old.c:1260`), which is why original scripts write `1, 1`.
//! * `COMPUTER_PLAYER(player, model)` ("PA", `src/lvl_script_commands.c:7040`; "PN" in level
//!   version 0, `:7217`): a model number of `keepcompp.cfg` (`[computer0]`..`[computer16]`,
//!   `config/fxdata/keepcompp.cfg:809`), `ROAMING` or `OFF` (`computer_player_check`,
//!   `src/lvl_script_commands.c:6197`). `PLAYER_GOOD` is refused in level version 0 (`:6208`).
//! * `ALLY_PLAYERS(player1, player2, state)` ("PPN", `src/lvl_script_commands.c:7089`; "PP" in
//!   level version 0, `:7245`, always allied): bit 0 allied, bit 1 locked
//!   (`src/lvl_script_value.c:609`).
//! * `SET_GENERATE_SPEED(turns)` ("Np", `src/lvl_script_commands.c:7031`).
//! * `IF(player, ALL_DUNGEONS_DESTROYED == 1)` + `WIN_GAME` and
//!   `IF(player, DUNGEON_DESTROYED == 1)` + `LOSE_GAME` (`src/lvl_script_conditions.c:354`,
//!   `:292`): the game records the condition (`command_win_game`,
//!   `src/lvl_script_commands_old.c:375`) and, once it is met, applies the outcome to the human
//!   players (`process_win_and_lose_conditions`, `src/lvl_script.c:1094`).
//! * Player names and numbers: `player_desc` (`src/lvl_script_commands.c:77`), ids in
//!   `src/player_instances.h:33`; `ALL_PLAYERS` covers every slot (`get_players_range`,
//!   `src/lvl_script.c:213`). KeeperFX has PLAYER4..PLAYER6 next to the original four.
use crate::editor::ScriptEditor;
use kfx_script::{ConstKind, ConstantProvider, Document, LineKind, Span};
use std::collections::{BTreeMap, BTreeSet};

pub use crate::wizard_ui::{ScriptWizard, WizardEvent, WizardTab};

/// Marker text of the first line of the block (after `REM ` or `-- `).
pub const BEGIN_MARK: &str = "---- kfx-editor wizard begin ----";
/// Marker text of the last line of the block.
pub const END_MARK: &str = "---- kfx-editor wizard end ----";
/// Name of the Lua function the block defines; `OnGameStart` must call it.
pub const LUA_SETUP_FN: &str = "KfxWizardSetup";

/// The script the wizard writes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScriptLang {
    /// The DK level script (`mapNNNNN.txt`).
    Dk,
    /// The Lua level script (`mapNNNNN.lua`).
    Lua,
}

/// A player slot a setup command can name (`player_desc`, `src/lvl_script_commands.c:77`).
/// `PLAYER_NEUTRAL` has no dungeon and is left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Player {
    P0,
    P1,
    P2,
    P3,
    Good,
    P4,
    P5,
    P6,
}

impl Player {
    pub const ALL: [Player; 8] = [
        Player::P0,
        Player::P1,
        Player::P2,
        Player::P3,
        Player::Good,
        Player::P4,
        Player::P5,
        Player::P6,
    ];

    /// Script name, `PLAYER0` .. `PLAYER_GOOD`.
    pub fn name(self) -> &'static str {
        match self {
            Player::P0 => "PLAYER0",
            Player::P1 => "PLAYER1",
            Player::P2 => "PLAYER2",
            Player::P3 => "PLAYER3",
            Player::Good => "PLAYER_GOOD",
            Player::P4 => "PLAYER4",
            Player::P5 => "PLAYER5",
            Player::P6 => "PLAYER6",
        }
    }

    /// Short column title.
    pub fn short(self) -> &'static str {
        match self {
            Player::P0 => "Red",
            Player::P1 => "Blue",
            Player::P2 => "Green",
            Player::P3 => "Yellow",
            Player::Good => "Heroes",
            Player::P4 => "Purple",
            Player::P5 => "Black",
            Player::P6 => "Orange",
        }
    }

    /// Position in [`Player::ALL`].
    pub fn index(self) -> usize {
        Player::ALL.iter().position(|&p| p == self).unwrap_or(0)
    }

    /// The game's player number (`src/player_instances.h:33`).
    pub fn game_id(self) -> u8 {
        match self {
            Player::P0 => 0,
            Player::P1 => 1,
            Player::P2 => 2,
            Player::P3 => 3,
            Player::Good => 4,
            Player::P4 => 6,
            Player::P5 => 7,
            Player::P6 => 8,
        }
    }

    pub fn parse(s: &str) -> Option<Player> {
        let s = s.trim();
        Player::ALL
            .iter()
            .copied()
            .find(|p| p.name().eq_ignore_ascii_case(s))
    }

    /// A keeper (not the heroes).
    pub fn is_keeper(self) -> bool {
        self != Player::Good
    }

    /// Exists in level version 0 (the original four keepers and the heroes).
    pub fn in_dk1(self) -> bool {
        matches!(
            self,
            Player::P0 | Player::P1 | Player::P2 | Player::P3 | Player::Good
        )
    }
}

/// Room or power availability: the states of `ROOM_AVAILABLE` / `MAGIC_AVAILABLE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// `0, 0`: never available.
    Unavailable,
    /// `1, 0`: can be researched.
    Researchable,
    /// `1, 1`: available from the start.
    Available,
    /// Any other pair, kept as written (KeeperFX room research modes 2..4, power `0, 1` ...).
    Other { research: i64, available: i64 },
}

impl Availability {
    pub fn from_args(research: i64, available: i64) -> Availability {
        match (research, available) {
            (0, 0) => Availability::Unavailable,
            (1, 0) => Availability::Researchable,
            (1, 1) => Availability::Available,
            (research, available) => Availability::Other {
                research,
                available,
            },
        }
    }

    pub fn args(self) -> (i64, i64) {
        match self {
            Availability::Unavailable => (0, 0),
            Availability::Researchable => (1, 0),
            Availability::Available => (1, 1),
            Availability::Other {
                research,
                available,
            } => (research, available),
        }
    }

    /// Next state of the cycle button: unavailable, researchable, available, (not set).
    pub fn cycle(cur: Option<Availability>) -> Option<Availability> {
        match cur {
            None => Some(Availability::Unavailable),
            Some(Availability::Unavailable) => Some(Availability::Researchable),
            Some(Availability::Researchable) => Some(Availability::Available),
            Some(Availability::Available) => None,
            Some(Availability::Other { .. }) => Some(Availability::Unavailable),
        }
    }

    pub fn label(self) -> String {
        match self {
            Availability::Unavailable => "No".into(),
            Availability::Researchable => "Research".into(),
            Availability::Available => "Yes".into(),
            Availability::Other {
                research,
                available,
            } => format!("{research}, {available}"),
        }
    }
}

/// Trap or door state: the manufacturable flag and the ready-made amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Stock {
    pub buildable: bool,
    pub amount: i64,
}

/// Creature availability at the portal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attraction {
    pub allowed: bool,
    /// Creatures that come even without the rooms they want (level version 1 only).
    pub forced: i64,
}

/// What `COMPUTER_PLAYER` sets up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Computer {
    /// A `keepcompp.cfg` model number.
    Model(u8),
    /// `ROAMING` (level version 1).
    Roaming,
    /// `OFF` (level version 1): the slot becomes an idle keeper.
    Off,
}

impl Computer {
    pub fn label(self) -> String {
        match self {
            Computer::Model(n) => {
                let name = COMPUTER_MODELS
                    .iter()
                    .find(|m| m.0 == n)
                    .map(|m| m.1)
                    .unwrap_or("custom model");
                format!("{n}: {name}")
            }
            Computer::Roaming => "ROAMING".into(),
            Computer::Off => "OFF".into(),
        }
    }
}

/// The computer models of the stock `keepcompp.cfg` (`config/fxdata/keepcompp.cfg:809`..`:950`).
/// 6..9 are the player assistants, 13..16 the skirmish models.
pub const COMPUTER_MODELS: [(u8, &str); 17] = [
    (0, "General build, defend and attack"),
    (1, "Build and defend"),
    (2, "Build and defend"),
    (3, "Build and defend"),
    (4, "Build basic 4x4 and defend"),
    (5, "Build and defend"),
    (6, "Assist, aggressive"),
    (7, "Assist, defensive"),
    (8, "Assist, build only"),
    (9, "Assist, move only"),
    (10, "Build and defend"),
    (11, "Build and defend"),
    (12, "Rapid gold digging, imp army"),
    (13, "Skirmish defensive"),
    (14, "Skirmish peculiar"),
    (15, "Skirmish balanced"),
    (16, "Skirmish rush"),
];

/// Alliance states of `ALLY_PLAYERS` (`src/lvl_script_value.c:609`): bit 0 allied, bit 1 locked.
pub const ALLIANCE_STATES: [(i64, &str); 4] = [
    (0, "enemies"),
    (1, "allied"),
    (2, "enemies, locked"),
    (3, "allied, locked"),
];

/// Settings of one player. `None` / absent = the wizard does not set it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerSetup {
    pub start_money: Option<i64>,
    pub max_creatures: Option<i64>,
    pub computer: Option<Computer>,
    pub rooms: BTreeMap<String, Availability>,
    pub powers: BTreeMap<String, Availability>,
    pub traps: BTreeMap<String, Stock>,
    pub doors: BTreeMap<String, Stock>,
    pub creatures: BTreeMap<String, Attraction>,
    /// `IF(player, ALL_DUNGEONS_DESTROYED == 1)` `WIN_GAME`.
    pub win_when_enemies_destroyed: bool,
    /// `IF(player, DUNGEON_DESTROYED == 1)` `LOSE_GAME`.
    pub lose_when_heart_destroyed: bool,
}

/// Everything the wizard edits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WizardModel {
    /// `LEVEL_VERSION` of the script (0 = original DK rules). Selects the argument forms.
    pub version: i64,
    /// Indexed like [`Player::ALL`]; see [`WizardModel::player`].
    pub players: [PlayerSetup; 8],
    /// `SET_GENERATE_SPEED`, game turns between portal creatures.
    pub generate_speed: Option<i64>,
    /// Total of `ADD_CREATURE_TO_POOL` per creature.
    pub pool: BTreeMap<String, i64>,
    /// `ALLY_PLAYERS` state per pair (lower [`Player`] first), see [`ALLIANCE_STATES`].
    pub alliances: BTreeMap<(Player, Player), i64>,
}

/// A line outside the block that sets something the block sets too (or that the form wants
/// unset), left alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// 1-based line in the script as it is now.
    pub line: usize,
    /// The line's text.
    pub text: String,
    /// What both set, e.g. `START_MONEY for PLAYER0`.
    pub what: String,
    pub message: String,
}

/// The edit that writes the wizard block, plus what the user should know.
#[derive(Clone, Debug, PartialEq)]
pub struct WizardPlan {
    pub lang: ScriptLang,
    /// File bytes replaced: the old block, or an empty span where a new block goes.
    pub span: Span,
    /// Lines of the new block, markers included; empty when the block is removed (nothing to set).
    pub lines: Vec<String>,
    pub conflicts: Vec<Conflict>,
    /// Things the block cannot express (level version 0 limits, Lua differences ...).
    pub notes: Vec<String>,
    crlf: bool,
    /// Inserting after a last line without a newline: start with one.
    lead_newline: bool,
}

impl WizardPlan {
    fn render(&self, eol: &str) -> String {
        let mut s = String::new();
        if self.lines.is_empty() {
            return s;
        }
        if self.lead_newline {
            s.push_str(eol);
        }
        for l in &self.lines {
            s.push_str(l);
            s.push_str(eol);
        }
        s
    }

    /// Replacement text with LF line ends, as [`ScriptEditor::replace_spans`] takes it (the
    /// editor writes CR LF back when the file uses it).
    pub fn text(&self) -> String {
        self.render("\n")
    }

    /// Replacement text with the file's own line ends.
    pub fn file_text(&self) -> String {
        self.render(if self.crlf { "\r\n" } else { "\n" })
    }

    /// The script with the plan applied (bytes outside [`WizardPlan::span`] unchanged).
    pub fn apply_to(&self, script: &[u8]) -> Vec<u8> {
        let (a, b) = (
            self.span.start.min(script.len()),
            self.span.end.min(script.len()),
        );
        let mut out = Vec::with_capacity(script.len() + 256);
        out.extend_from_slice(&script[..a]);
        out.extend(self.file_text().chars().map(|c| c as u32 as u8));
        out.extend_from_slice(&script[b..]);
        out
    }

    /// Applying would change the script.
    pub fn changes(&self, script: &[u8]) -> bool {
        let (a, b) = (
            self.span.start.min(script.len()),
            self.span.end.min(script.len()),
        );
        let new: Vec<u8> = self.file_text().chars().map(|c| c as u32 as u8).collect();
        script[a..b] != new[..]
    }

    /// Apply to an open editor as one undoable edit. False when the editor refused (read-only,
    /// stale span).
    pub fn apply_to_editor(&self, ed: &mut ScriptEditor) -> bool {
        let r = ed.replace_spans(&[(self.span, self.text())]);
        ed.apply_pending();
        r
    }
}

// -------------------------------------------------------------------------------------------
// Statements

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Key {
    GenerateSpeed,
    Computer(Player),
    StartMoney(Player),
    MaxCreatures(Player),
    Room(Player, String),
    Power(Player, String),
    Trap(Player, String),
    Door(Player, String),
    Creature(Player, String),
    Pool(String),
    Ally(Player, Player),
    Win(Player),
    Lose(Player),
}

impl Key {
    fn describe(&self, lang: ScriptLang) -> String {
        let dk = lang == ScriptLang::Dk;
        let c = |d: &'static str, l: &'static str| if dk { d } else { l };
        match self {
            Key::GenerateSpeed => c("SET_GENERATE_SPEED", "SetGenerateSpeed").into(),
            Key::Computer(p) => format!(
                "{} for {}",
                c("COMPUTER_PLAYER", "ComputerPlayer"),
                p.name()
            ),
            Key::StartMoney(p) => format!("{} for {}", c("START_MONEY", "StartMoney"), p.name()),
            Key::MaxCreatures(p) => {
                format!("{} for {}", c("MAX_CREATURES", "MaxCreatures"), p.name())
            }
            Key::Room(p, n) => format!(
                "{} {n} for {}",
                c("ROOM_AVAILABLE", "RoomAvailable"),
                p.name()
            ),
            Key::Power(p, n) => format!(
                "{} {n} for {}",
                c("MAGIC_AVAILABLE", "MagicAvailable"),
                p.name()
            ),
            Key::Trap(p, n) => format!(
                "{} {n} for {}",
                c("TRAP_AVAILABLE", "TrapAvailable"),
                p.name()
            ),
            Key::Door(p, n) => format!(
                "{} {n} for {}",
                c("DOOR_AVAILABLE", "DoorAvailable"),
                p.name()
            ),
            Key::Creature(p, n) => format!(
                "{} {n} for {}",
                c("CREATURE_AVAILABLE", "CreatureAvailable"),
                p.name()
            ),
            Key::Pool(n) => format!("{} {n}", c("ADD_CREATURE_TO_POOL", "AddCreatureToPool")),
            Key::Ally(a, b) => format!(
                "{} {} / {}",
                c("ALLY_PLAYERS", "AllyPlayers"),
                a.name(),
                b.name()
            ),
            Key::Win(p) => format!("win condition of {}", p.name()),
            Key::Lose(p) => format!("lose condition of {}", p.name()),
        }
    }

    /// The command adds to the value instead of setting it.
    fn additive(&self, lang: ScriptLang) -> bool {
        match self {
            Key::StartMoney(_) => lang == ScriptLang::Dk,
            Key::Pool(_) | Key::Trap(..) | Key::Door(..) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Num(i64),
    Avail(Availability),
    Stock(Stock),
    Attr(Attraction),
    Comp(Computer),
    Flag,
}

#[derive(Clone, Debug)]
struct Stmt {
    key: Key,
    val: Val,
    /// 1-based line.
    line: usize,
    /// Text of the line, for conflicts.
    text: String,
    in_block: bool,
}

fn fold(model: &mut WizardModel, st: &Stmt, lang: ScriptLang) {
    let add = st.key.additive(lang);
    match (&st.key, &st.val) {
        (Key::GenerateSpeed, Val::Num(n)) => model.generate_speed = Some(*n),
        (Key::Computer(p), Val::Comp(c)) => model.player_mut(*p).computer = Some(*c),
        (Key::StartMoney(p), Val::Num(n)) => {
            let s = &mut model.player_mut(*p).start_money;
            *s = Some(if add { s.unwrap_or(0) + n } else { *n });
        }
        (Key::MaxCreatures(p), Val::Num(n)) => model.player_mut(*p).max_creatures = Some(*n),
        (Key::Room(p, r), Val::Avail(a)) => {
            model.player_mut(*p).rooms.insert(r.clone(), *a);
        }
        (Key::Power(p, r), Val::Avail(a)) => {
            model.player_mut(*p).powers.insert(r.clone(), *a);
        }
        (Key::Trap(p, r), Val::Stock(s)) => {
            let e = model.player_mut(*p).traps.entry(r.clone()).or_default();
            *e = Stock {
                buildable: s.buildable,
                amount: e.amount + s.amount,
            };
        }
        (Key::Door(p, r), Val::Stock(s)) => {
            let e = model.player_mut(*p).doors.entry(r.clone()).or_default();
            *e = Stock {
                buildable: s.buildable,
                amount: e.amount + s.amount,
            };
        }
        (Key::Creature(p, r), Val::Attr(a)) => {
            model.player_mut(*p).creatures.insert(r.clone(), *a);
        }
        (Key::Pool(c), Val::Num(n)) => *model.pool.entry(c.clone()).or_insert(0) += n,
        (Key::Ally(a, b), Val::Num(n)) => {
            model.alliances.insert((*a, *b), *n);
        }
        (Key::Win(p), Val::Flag) => model.player_mut(*p).win_when_enemies_destroyed = true,
        (Key::Lose(p), Val::Flag) => model.player_mut(*p).lose_when_heart_destroyed = true,
        _ => {}
    }
}

/// `strtol(s, NULL, 0)` as the game converts `N` arguments (`src/lvl_script.c:243`): sign,
/// `0x` hex, leading-zero octal, decimal; the longest valid prefix.
fn parse_num(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (radix, digits) = if rest.len() > 1 && (rest.starts_with("0x") || rest.starts_with("0X")) {
        (16, &rest[2..])
    } else if rest.len() > 1 && rest.starts_with('0') {
        (8, &rest[1..])
    } else {
        (10, rest)
    };
    let end = digits
        .find(|c: char| !c.is_digit(radix))
        .unwrap_or(digits.len());
    if end == 0 {
        return if radix == 8 { Some(0) } else { None };
    }
    let v = i64::from_str_radix(&digits[..end], radix).ok()?;
    Some(if neg { -v } else { v })
}

/// Player argument: one slot, or every slot for `ALL_PLAYERS`.
fn players_of(s: &str) -> Vec<Player> {
    if s.trim().eq_ignore_ascii_case("ALL_PLAYERS") {
        return Player::ALL.to_vec();
    }
    if let Some(p) = Player::parse(s) {
        return vec![p];
    }
    // a number: the game's player id
    match parse_num(s) {
        Some(9) => Player::ALL.to_vec(),
        Some(n) => Player::ALL
            .iter()
            .copied()
            .filter(|p| p.game_id() as i64 == n)
            .collect(),
        None => vec![],
    }
}

fn pair(a: Player, b: Player) -> (Player, Player) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

struct Lines {
    starts: Vec<usize>,
}

impl Lines {
    fn new(src: &[u8]) -> Lines {
        let mut starts = vec![0];
        for (i, &b) in src.iter().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Lines { starts }
    }
    /// 0-based line of a byte offset.
    fn line_of(&self, off: usize) -> usize {
        self.starts.partition_point(|&s| s <= off).saturating_sub(1)
    }
    fn count(&self) -> usize {
        self.starts.len()
    }
    /// Byte range of a line without its line end.
    fn content(&self, src: &[u8], line: usize) -> (usize, usize) {
        let a = self.starts[line];
        let mut b = self.starts.get(line + 1).copied().unwrap_or(src.len());
        while b > a && (src[b - 1] == b'\n' || src[b - 1] == b'\r') {
            b -= 1;
        }
        (a, b)
    }
    /// Byte offset after the line and its line end.
    fn next(&self, src: &[u8], line: usize) -> usize {
        self.starts.get(line + 1).copied().unwrap_or(src.len())
    }
    fn text(&self, src: &[u8], line: usize) -> String {
        let (a, b) = self.content(src, line);
        src[a..b].iter().map(|&c| c as char).collect()
    }
}

/// The wizard block: 0-based first and last line.
fn find_block(src: &[u8], lines: &Lines, lang: ScriptLang) -> Option<(usize, usize)> {
    let is_mark = |l: usize, mark: &str| {
        let t = lines.text(src, l);
        let t = t.trim();
        let rest = match lang {
            ScriptLang::Dk => t
                .get(..3)
                .filter(|p| p.eq_ignore_ascii_case("REM"))
                .map(|_| &t[3..]),
            ScriptLang::Lua => t.strip_prefix("--"),
        };
        rest.map_or(false, |r| r.trim() == mark)
    };
    let begin = (0..lines.count()).find(|&l| is_mark(l, BEGIN_MARK))?;
    let end = (begin + 1..lines.count())
        .find(|&l| is_mark(l, END_MARK))
        .unwrap_or(begin);
    Some((begin, end))
}

fn crlf_of(src: &[u8]) -> bool {
    src.iter()
        .position(|&b| b == b'\n')
        .map_or(false, |i| i > 0 && src[i - 1] == b'\r')
}

// -------------------------------------------------------------------------------------------
// DK script

fn dk_statements(src: &[u8]) -> (Vec<Stmt>, i64) {
    let doc = Document::parse(src);
    let lines = Lines::new(src);
    let block = find_block(src, &lines, ScriptLang::Dk);
    let mut out = vec![];
    let mut depth: i32 = 0;
    // (player, wins) of the IF block we are in, when it is one of the win/lose forms
    let mut cond: Option<(Player, bool, usize)> = None;
    for line in &doc.lines {
        if line.kind != LineKind::Command {
            continue;
        }
        let Some(c) = &line.cmd else { continue };
        let l = lines.line_of(c.name_span.start);
        let in_block = block.map_or(false, |(a, b)| l >= a && l <= b);
        let name = c.name.as_str();
        if kfx_script::validate::is_if_command(name) {
            if depth == 0 && name == "IF" {
                let var = c.text(1).to_ascii_uppercase();
                let (op, v) = (c.text(2), parse_num(c.text(3)));
                let true_test = matches!(
                    (op, v),
                    ("==", Some(1)) | (">=", Some(1)) | (">", Some(0)) | ("!=", Some(0))
                );
                cond = match (Player::parse(c.text(0)), var.as_str(), true_test) {
                    (Some(p), "ALL_DUNGEONS_DESTROYED", true) => Some((p, true, l)),
                    (Some(p), "DUNGEON_DESTROYED", true) => Some((p, false, l)),
                    _ => None,
                };
            }
            depth += 1;
            continue;
        }
        if name == "ENDIF" {
            depth = (depth - 1).max(0);
            if depth == 0 {
                cond = None;
            }
            continue;
        }
        if depth > 0 {
            if depth == 1 {
                if let Some((p, wins, if_line)) = cond {
                    let key = match (name, wins) {
                        ("WIN_GAME", true) => Some(Key::Win(p)),
                        ("LOSE_GAME", false) => Some(Key::Lose(p)),
                        _ => None,
                    };
                    if let Some(key) = key {
                        let in_block = block.map_or(false, |(a, b)| if_line >= a && if_line <= b);
                        out.push(Stmt {
                            key,
                            val: Val::Flag,
                            line: if_line + 1,
                            text: lines.text(src, if_line).trim().to_string(),
                            in_block,
                        });
                    }
                }
            }
            continue;
        }
        let mut push = |key: Key, val: Val| {
            out.push(Stmt {
                key,
                val,
                line: l + 1,
                text: lines.text(src, l).trim().to_string(),
                in_block,
            })
        };
        let up = |i: usize| c.text(i).to_ascii_uppercase();
        let num = |i: usize| parse_num(c.text(i));
        match name {
            "SET_GENERATE_SPEED" if c.text(1).is_empty() => {
                if let Some(n) = num(0) {
                    push(Key::GenerateSpeed, Val::Num(n));
                }
            }
            "START_MONEY" | "MAX_CREATURES" => {
                if let Some(n) = num(1) {
                    for p in players_of(c.text(0)) {
                        let k = if name == "START_MONEY" {
                            Key::StartMoney(p)
                        } else {
                            Key::MaxCreatures(p)
                        };
                        push(k, Val::Num(n));
                    }
                }
            }
            "COMPUTER_PLAYER" => {
                let a = up(1);
                let comp = match a.as_str() {
                    "ROAMING" => Some(Computer::Roaming),
                    "OFF" => Some(Computer::Off),
                    _ => num(1)
                        .filter(|n| (0..=255).contains(n))
                        .map(|n| Computer::Model(n as u8)),
                };
                if let Some(comp) = comp {
                    for p in players_of(c.text(0)) {
                        push(Key::Computer(p), Val::Comp(comp));
                    }
                }
            }
            "ROOM_AVAILABLE" | "MAGIC_AVAILABLE" => {
                if let (Some(r), Some(a)) = (num(2), num(3)) {
                    for p in players_of(c.text(0)) {
                        let k = if name == "ROOM_AVAILABLE" {
                            Key::Room(p, up(1))
                        } else {
                            Key::Power(p, up(1))
                        };
                        push(k, Val::Avail(Availability::from_args(r, a)));
                    }
                }
            }
            "TRAP_AVAILABLE" | "DOOR_AVAILABLE" => {
                if let (Some(b), Some(n)) = (num(2), num(3)) {
                    for p in players_of(c.text(0)) {
                        let k = if name == "TRAP_AVAILABLE" {
                            Key::Trap(p, up(1))
                        } else {
                            Key::Door(p, up(1))
                        };
                        push(
                            k,
                            Val::Stock(Stock {
                                buildable: b != 0,
                                amount: n,
                            }),
                        );
                    }
                }
            }
            "CREATURE_AVAILABLE" => {
                if let (Some(a), Some(f)) = (num(2), num(3)) {
                    // level version 0: the fourth argument is the flag, nothing is forced
                    let attr = if doc.version > 0 {
                        Attraction {
                            allowed: a != 0,
                            forced: f.max(0),
                        }
                    } else {
                        Attraction {
                            allowed: f != 0,
                            forced: 0,
                        }
                    };
                    for p in players_of(c.text(0)) {
                        push(Key::Creature(p, up(1)), Val::Attr(attr));
                    }
                }
            }
            "ADD_CREATURE_TO_POOL" => {
                if let Some(n) = num(1) {
                    push(Key::Pool(up(0)), Val::Num(n));
                }
            }
            "ALLY_PLAYERS" => {
                let state = if doc.version > 0 {
                    num(2).unwrap_or(0)
                } else {
                    1
                };
                if let Some(b) = Player::parse(c.text(1)) {
                    for a in players_of(c.text(0)) {
                        if a != b {
                            push(Key::Ally(pair(a, b).0, pair(a, b).1), Val::Num(state));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (out, doc.version)
}

impl Key {
    fn player(&self) -> Option<Player> {
        match self {
            Key::Computer(p)
            | Key::StartMoney(p)
            | Key::MaxCreatures(p)
            | Key::Win(p)
            | Key::Lose(p) => Some(*p),
            Key::Room(p, _)
            | Key::Power(p, _)
            | Key::Trap(p, _)
            | Key::Door(p, _)
            | Key::Creature(p, _) => Some(*p),
            Key::Ally(a, b) => Some(if a.in_dk1() { *b } else { *a }),
            Key::GenerateSpeed | Key::Pool(_) => None,
        }
    }
}

fn render_dk(key: &Key, val: &Val, version: i64, notes: &mut Vec<String>) -> Vec<String> {
    let one = |s: String| vec![s];
    if let Some(p) = key.player().filter(|p| version == 0 && !p.in_dk1()) {
        let n = format!("{} needs LEVEL_VERSION(1); not written.", p.name());
        if !notes.contains(&n) {
            notes.push(n);
        }
        return vec![];
    }
    match (key, val) {
        (Key::GenerateSpeed, Val::Num(n)) => one(format!("SET_GENERATE_SPEED({n})")),
        (Key::Computer(p), Val::Comp(c)) => {
            if version == 0 && *p == Player::Good {
                notes.push(
                    "COMPUTER_PLAYER for PLAYER_GOOD needs LEVEL_VERSION(1); not written.".into(),
                );
                return vec![];
            }
            let arg = match c {
                Computer::Model(n) => n.to_string(),
                Computer::Roaming | Computer::Off if version == 0 => {
                    notes.push(format!(
                        "COMPUTER_PLAYER({}, {}) needs LEVEL_VERSION(1); not written.",
                        p.name(),
                        c.label()
                    ));
                    return vec![];
                }
                Computer::Roaming => "ROAMING".into(),
                Computer::Off => "OFF".into(),
            };
            one(format!("COMPUTER_PLAYER({}, {arg})", p.name()))
        }
        (Key::StartMoney(p), Val::Num(n)) => one(format!("START_MONEY({}, {n})", p.name())),
        (Key::MaxCreatures(p), Val::Num(n)) => one(format!("MAX_CREATURES({}, {n})", p.name())),
        (Key::Room(p, r), Val::Avail(a)) => {
            let (x, y) = a.args();
            one(format!("ROOM_AVAILABLE({}, {r}, {x}, {y})", p.name()))
        }
        (Key::Power(p, r), Val::Avail(a)) => {
            let (x, y) = a.args();
            one(format!("MAGIC_AVAILABLE({}, {r}, {x}, {y})", p.name()))
        }
        (Key::Trap(p, r), Val::Stock(s)) => one(format!(
            "TRAP_AVAILABLE({}, {r}, {}, {})",
            p.name(),
            s.buildable as i32,
            s.amount
        )),
        (Key::Door(p, r), Val::Stock(s)) => one(format!(
            "DOOR_AVAILABLE({}, {r}, {}, {})",
            p.name(),
            s.buildable as i32,
            s.amount
        )),
        (Key::Creature(p, r), Val::Attr(a)) => {
            if version > 0 {
                one(format!(
                    "CREATURE_AVAILABLE({}, {r}, {}, {})",
                    p.name(),
                    a.allowed as i32,
                    a.forced
                ))
            } else {
                if a.forced > 0 {
                    notes.push(format!(
                        "Forced creatures ({r} for {}) need LEVEL_VERSION(1); written without.",
                        p.name()
                    ));
                }
                let f = a.allowed as i32;
                one(format!("CREATURE_AVAILABLE({}, {r}, {f}, {f})", p.name()))
            }
        }
        (Key::Pool(c), Val::Num(n)) => one(format!("ADD_CREATURE_TO_POOL({c}, {n})")),
        (Key::Ally(a, b), Val::Num(s)) => {
            if version > 0 {
                one(format!("ALLY_PLAYERS({}, {}, {s})", a.name(), b.name()))
            } else if s & 1 == 1 {
                one(format!("ALLY_PLAYERS({}, {})", a.name(), b.name()))
            } else {
                notes.push(format!(
                    "ALLY_PLAYERS({}, {}) can only make allies in level version 0; not written.",
                    a.name(),
                    b.name()
                ));
                vec![]
            }
        }
        (Key::Win(p), _) => vec![
            format!("IF({}, ALL_DUNGEONS_DESTROYED == 1)", p.name()),
            "\tWIN_GAME".into(),
            "ENDIF".into(),
        ],
        (Key::Lose(p), _) => vec![
            format!("IF({}, DUNGEON_DESTROYED == 1)", p.name()),
            "\tLOSE_GAME".into(),
            "ENDIF".into(),
        ],
        _ => vec![],
    }
}

/// End offset of the header: leading comments, blank lines and `LEVEL_VERSION`. The block goes
/// after the last non-blank line of that run.
fn dk_insert_at(src: &[u8]) -> usize {
    let doc = Document::parse(src);
    let mut at = 0;
    for line in &doc.lines {
        match line.kind {
            LineKind::Blank => continue,
            LineKind::Comment => at = line.span.end,
            LineKind::Command
                if line
                    .cmd
                    .as_ref()
                    .map_or(false, |c| c.name == "LEVEL_VERSION") =>
            {
                at = line.span.end
            }
            _ => break,
        }
    }
    at
}

// -------------------------------------------------------------------------------------------
// Lua script

/// Arguments of a one-line call `Name(a, "b", true)`: text without quotes; `None` when the line
/// is not exactly one call.
fn lua_call(line: &str) -> Option<(String, Vec<String>)> {
    let t = strip_lua_comment(line);
    let t = t.trim().trim_end_matches(';').trim();
    let open = t.find('(')?;
    let name = t[..open].trim();
    if name.is_empty()
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || !t.ends_with(')')
    {
        return None;
    }
    let inner = &t[open + 1..t.len() - 1];
    let mut args = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut depth = 0;
    for ch in inner.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    cur.push(ch);
                }
            }
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '(' | '{' => {
                    depth += 1;
                    cur.push(ch)
                }
                ')' | '}' => {
                    depth -= 1;
                    cur.push(ch)
                }
                ',' if depth == 0 => args.push(std::mem::take(&mut cur).trim().to_string()),
                _ => cur.push(ch),
            },
        }
    }
    if quote.is_some() || depth != 0 {
        return None;
    }
    if !cur.trim().is_empty() || !args.is_empty() {
        args.push(cur.trim().to_string());
    }
    Some((name.to_string(), args))
}

fn strip_lua_comment(line: &str) -> &str {
    let b = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < b.len() {
        match quote {
            Some(q) if b[i] == q => quote = None,
            Some(_) => {}
            None if b[i] == b'"' || b[i] == b'\'' => quote = Some(b[i]),
            None if b[i] == b'-' && b.get(i + 1) == Some(&b'-') => return &line[..i],
            None => {}
        }
        i += 1;
    }
    line
}

fn lua_num(s: &str) -> Option<i64> {
    match s.trim() {
        "true" => Some(1),
        "false" | "nil" => Some(0),
        t => parse_num(t),
    }
}

fn win_fn(p: Player) -> String {
    format!("KfxWizardWin_{}", p.name())
}
fn win_check_fn(p: Player) -> String {
    format!("KfxWizardWinCheck_{}", p.name())
}
fn lose_fn(p: Player) -> String {
    format!("KfxWizardLose_{}", p.name())
}

/// Calls inside `OnGameStart`, the wizard's setup function or at the top level of the file.
fn lua_statements(src: &[u8]) -> Vec<Stmt> {
    let lines = Lines::new(src);
    let block = find_block(src, &lines, ScriptLang::Lua);
    let mut out = vec![];
    let mut func: Option<String> = None;
    for l in 0..lines.count() {
        let text = lines.text(src, l);
        // top-level function definitions start in column 0 and end with an `end` in column 0
        if let Some(rest) = text.strip_prefix("function ") {
            func = Some(
                rest.split(|c: char| c == '(' || c.is_whitespace())
                    .next()
                    .unwrap_or("")
                    .to_string(),
            );
            continue;
        }
        if text.starts_with("end") && text[3..].trim().trim_start_matches(|c| c == ')').is_empty() {
            func = None;
            continue;
        }
        if let Some(f) = &func {
            if f != "OnGameStart" && f != LUA_SETUP_FN {
                continue;
            }
        }
        let Some((name, a)) = lua_call(&text) else {
            continue;
        };
        let in_block = block.map_or(false, |(x, y)| l >= x && l <= y);
        let mut push = |key: Key, val: Val| {
            out.push(Stmt {
                key,
                val,
                line: l + 1,
                text: text.trim().to_string(),
                in_block,
            })
        };
        let arg = |i: usize| a.get(i).map(|s| s.as_str()).unwrap_or("");
        let up = |i: usize| arg(i).to_ascii_uppercase();
        let num = |i: usize| a.get(i).and_then(|s| lua_num(s));
        match name.as_str() {
            "SetGenerateSpeed" if a.len() == 1 => {
                if let Some(n) = num(0) {
                    push(Key::GenerateSpeed, Val::Num(n));
                }
            }
            "StartMoney" | "MaxCreatures" => {
                if let Some(n) = num(1) {
                    for p in players_of(arg(0)) {
                        push(
                            if name == "StartMoney" {
                                Key::StartMoney(p)
                            } else {
                                Key::MaxCreatures(p)
                            },
                            Val::Num(n),
                        );
                    }
                }
            }
            "ComputerPlayer" => {
                let comp = if up(1) == "ROAMING" {
                    Some(Computer::Roaming)
                } else {
                    num(1)
                        .filter(|n| (0..=255).contains(n))
                        .map(|n| Computer::Model(n as u8))
                };
                if let (Some(comp), Some(p)) = (comp, Player::parse(arg(0))) {
                    push(Key::Computer(p), Val::Comp(comp));
                }
            }
            "RoomAvailable" | "MagicAvailable" => {
                if let (Some(r), Some(x)) = (num(2), num(3)) {
                    for p in players_of(arg(0)) {
                        push(
                            if name == "RoomAvailable" {
                                Key::Room(p, up(1))
                            } else {
                                Key::Power(p, up(1))
                            },
                            Val::Avail(Availability::from_args(r, x)),
                        );
                    }
                }
            }
            "TrapAvailable" | "DoorAvailable" => {
                if let (Some(b), Some(n)) = (num(2), num(3)) {
                    for p in players_of(arg(0)) {
                        push(
                            if name == "TrapAvailable" {
                                Key::Trap(p, up(1))
                            } else {
                                Key::Door(p, up(1))
                            },
                            Val::Stock(Stock {
                                buildable: b != 0,
                                amount: n,
                            }),
                        );
                    }
                }
            }
            "CreatureAvailable" => {
                if let (Some(x), Some(f)) = (num(2), num(3)) {
                    for p in players_of(arg(0)) {
                        push(
                            Key::Creature(p, up(1)),
                            Val::Attr(Attraction {
                                allowed: x != 0,
                                forced: f.max(0),
                            }),
                        );
                    }
                }
            }
            "AddCreatureToPool" => {
                if let Some(n) = num(1) {
                    push(Key::Pool(up(0)), Val::Num(n));
                }
            }
            "AllyPlayers" => {
                if let (Some(b), Some(s)) = (Player::parse(arg(1)), num(2)) {
                    for x in players_of(arg(0)) {
                        if x != b {
                            let (p, q) = pair(x, b);
                            push(Key::Ally(p, q), Val::Num(s));
                        }
                    }
                }
            }
            "RegisterOnConditionEvent" | "RegisterDungeonDestroyedEvent" => {
                for p in Player::ALL {
                    if arg(0) == win_fn(p) {
                        push(Key::Win(p), Val::Flag);
                    } else if arg(0) == lose_fn(p) {
                        push(Key::Lose(p), Val::Flag);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn render_lua(key: &Key, val: &Val, notes: &mut Vec<String>) -> Vec<String> {
    let one = |s: String| vec![s];
    let b = |x: i64| if x != 0 { "true" } else { "false" };
    match (key, val) {
        (Key::GenerateSpeed, Val::Num(n)) => one(format!("SetGenerateSpeed({n})")),
        (Key::Computer(p), Val::Comp(c)) => match c {
            Computer::Model(n) => one(format!("ComputerPlayer({}, {n})", p.name())),
            Computer::Roaming => one(format!("ComputerPlayer({}, \"ROAMING\")", p.name())),
            Computer::Off => {
                // src/lua_api.c:79
                notes.push(format!(
                    "Lua ComputerPlayer has no OFF; {} not written.",
                    p.name()
                ));
                vec![]
            }
        },
        (Key::StartMoney(p), Val::Num(n)) => one(format!("StartMoney({}, {n})", p.name())),
        (Key::MaxCreatures(p), Val::Num(n)) => one(format!("MaxCreatures({}, {n})", p.name())),
        // RoomAvailable takes the research mode as a number (src/lua_api.c:199)
        (Key::Room(p, r), Val::Avail(a)) => {
            let (x, y) = a.args();
            one(format!(
                "RoomAvailable({}, \"{r}\", {x}, {})",
                p.name(),
                b(y)
            ))
        }
        (Key::Power(p, r), Val::Avail(a)) => {
            let (x, y) = a.args();
            one(format!(
                "MagicAvailable({}, \"{r}\", {}, {})",
                p.name(),
                b(x),
                b(y)
            ))
        }
        (Key::Trap(p, r), Val::Stock(s)) => one(format!(
            "TrapAvailable({}, \"{r}\", {}, {})",
            p.name(),
            s.buildable,
            s.amount
        )),
        (Key::Door(p, r), Val::Stock(s)) => one(format!(
            "DoorAvailable({}, \"{r}\", {}, {})",
            p.name(),
            s.buildable,
            s.amount
        )),
        (Key::Creature(p, r), Val::Attr(a)) => one(format!(
            "CreatureAvailable({}, \"{r}\", {}, {})",
            p.name(),
            a.allowed,
            a.forced
        )),
        (Key::Pool(c), Val::Num(n)) => one(format!("AddCreatureToPool(\"{c}\", {n})")),
        (Key::Ally(a, x), Val::Num(s)) => {
            one(format!("AllyPlayers({}, {}, {s})", a.name(), x.name()))
        }
        (Key::Win(p), _) => one(format!(
            "RegisterOnConditionEvent(\"{}\", \"{}\")",
            win_fn(*p),
            win_check_fn(*p)
        )),
        (Key::Lose(p), _) => one(format!(
            "RegisterDungeonDestroyedEvent(\"{}\", {})",
            lose_fn(*p),
            p.name()
        )),
        _ => vec![],
    }
}

fn lua_insert_at(src: &[u8]) -> usize {
    let lines = Lines::new(src);
    let mut at = 0;
    for l in 0..lines.count() {
        let t = lines.text(src, l);
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with("--") && !t.starts_with("--[[") {
            at = lines.next(src, l);
        } else {
            break;
        }
    }
    at
}

// -------------------------------------------------------------------------------------------
// Model

impl WizardModel {
    pub fn player(&self, p: Player) -> &PlayerSetup {
        &self.players[p.index()]
    }

    pub fn player_mut(&mut self, p: Player) -> &mut PlayerSetup {
        &mut self.players[p.index()]
    }

    /// Values set by the DK script `script_text` (unconditional commands anywhere, plus the
    /// win/lose `IF` forms). Unrecognised lines are ignored.
    pub fn read(script_text: &str) -> WizardModel {
        WizardModel::read_bytes(script_text.as_bytes())
    }

    /// [`WizardModel::read`] on file bytes (Latin-1).
    pub fn read_bytes(script: &[u8]) -> WizardModel {
        let (stmts, version) = dk_statements(script);
        let mut m = WizardModel {
            version,
            ..Default::default()
        };
        for s in &stmts {
            fold(&mut m, s, ScriptLang::Dk);
        }
        m
    }

    /// Values set by a Lua level script: one-line calls of the setup functions at the top level,
    /// in `OnGameStart` or in the wizard's own setup function; win/lose triggers only in the form
    /// the wizard writes. The version is 1 (Lua needs KeeperFX).
    pub fn read_lua(script_text: &str) -> WizardModel {
        WizardModel::read_lua_bytes(script_text.as_bytes())
    }

    pub fn read_lua_bytes(script: &[u8]) -> WizardModel {
        let mut m = WizardModel {
            version: 1,
            ..Default::default()
        };
        for s in &lua_statements(script) {
            fold(&mut m, s, ScriptLang::Lua);
        }
        m
    }

    /// Read a script of either language.
    pub fn read_as(lang: ScriptLang, script: &[u8]) -> WizardModel {
        match lang {
            ScriptLang::Dk => WizardModel::read_bytes(script),
            ScriptLang::Lua => WizardModel::read_lua_bytes(script),
        }
    }

    /// Every (key, value) the model sets, in block order.
    fn entries(&self) -> Vec<(Key, Val)> {
        let mut v = vec![];
        if let Some(n) = self.generate_speed {
            v.push((Key::GenerateSpeed, Val::Num(n)));
        }
        for p in Player::ALL {
            let s = self.player(p);
            if let Some(c) = s.computer {
                v.push((Key::Computer(p), Val::Comp(c)));
            }
            if let Some(n) = s.start_money {
                v.push((Key::StartMoney(p), Val::Num(n)));
            }
            if let Some(n) = s.max_creatures {
                v.push((Key::MaxCreatures(p), Val::Num(n)));
            }
            v.extend(
                s.rooms
                    .iter()
                    .map(|(k, a)| (Key::Room(p, k.clone()), Val::Avail(*a))),
            );
            v.extend(
                s.powers
                    .iter()
                    .map(|(k, a)| (Key::Power(p, k.clone()), Val::Avail(*a))),
            );
            v.extend(
                s.traps
                    .iter()
                    .map(|(k, a)| (Key::Trap(p, k.clone()), Val::Stock(*a))),
            );
            v.extend(
                s.doors
                    .iter()
                    .map(|(k, a)| (Key::Door(p, k.clone()), Val::Stock(*a))),
            );
            v.extend(
                s.creatures
                    .iter()
                    .map(|(k, a)| (Key::Creature(p, k.clone()), Val::Attr(*a))),
            );
        }
        v.extend(
            self.pool
                .iter()
                .map(|(k, n)| (Key::Pool(k.clone()), Val::Num(*n))),
        );
        v.extend(
            self.alliances
                .iter()
                .map(|(&(a, b), s)| (Key::Ally(a, b), Val::Num(*s))),
        );
        for p in Player::ALL {
            if self.player(p).win_when_enemies_destroyed {
                v.push((Key::Win(p), Val::Flag));
            }
        }
        for p in Player::ALL {
            if self.player(p).lose_when_heart_destroyed {
                v.push((Key::Lose(p), Val::Flag));
            }
        }
        v
    }

    /// The value of `key` in this model.
    fn value(&self, key: &Key) -> Option<Val> {
        let s = |p: &Player| self.player(*p);
        match key {
            Key::GenerateSpeed => self.generate_speed.map(Val::Num),
            Key::Computer(p) => s(p).computer.map(Val::Comp),
            Key::StartMoney(p) => s(p).start_money.map(Val::Num),
            Key::MaxCreatures(p) => s(p).max_creatures.map(Val::Num),
            Key::Room(p, n) => s(p).rooms.get(n).map(|a| Val::Avail(*a)),
            Key::Power(p, n) => s(p).powers.get(n).map(|a| Val::Avail(*a)),
            Key::Trap(p, n) => s(p).traps.get(n).map(|a| Val::Stock(*a)),
            Key::Door(p, n) => s(p).doors.get(n).map(|a| Val::Stock(*a)),
            Key::Creature(p, n) => s(p).creatures.get(n).map(|a| Val::Attr(*a)),
            Key::Pool(n) => self.pool.get(n).map(|a| Val::Num(*a)),
            Key::Ally(a, b) => self.alliances.get(&(*a, *b)).map(|x| Val::Num(*x)),
            Key::Win(p) => s(p).win_when_enemies_destroyed.then_some(Val::Flag),
            Key::Lose(p) => s(p).lose_when_heart_destroyed.then_some(Val::Flag),
        }
    }

    /// The block edit for the DK script `script` (file bytes). See [`WizardModel::plan_as`].
    pub fn plan(&self, script: &[u8]) -> WizardPlan {
        self.plan_as(ScriptLang::Dk, script)
    }

    /// The block edit for a Lua level script.
    pub fn plan_lua(&self, script: &[u8]) -> WizardPlan {
        self.plan_as(ScriptLang::Lua, script)
    }

    /// Compute the wizard block for `script`: what this model sets beyond what the lines outside
    /// the block already set, the span it replaces (the old block, or the insertion point after
    /// the header), and the conflicts with lines outside the block.
    pub fn plan_as(&self, lang: ScriptLang, script: &[u8]) -> WizardPlan {
        let (stmts, version) = match lang {
            ScriptLang::Dk => dk_statements(script),
            ScriptLang::Lua => (lua_statements(script), 1),
        };
        let version = if lang == ScriptLang::Dk { version } else { 1 };
        let outside_stmts: Vec<&Stmt> = stmts.iter().filter(|s| !s.in_block).collect();
        let mut outside = WizardModel {
            version,
            ..Default::default()
        };
        for s in &outside_stmts {
            fold(&mut outside, s, lang);
        }
        let lines = Lines::new(script);
        let block = find_block(script, &lines, lang);
        let block_line = block.map(|b| b.0 + 1).unwrap_or(0);
        let mut notes = vec![];
        let mut body: Vec<String> = vec![];
        let mut written: Vec<(Key, bool)> = vec![];
        let mut conflicts = vec![];
        let mut win_lose: Vec<String> = vec![];
        let mut lua_funcs: Vec<String> = vec![];
        for (key, val) in self.entries() {
            let out = outside.value(&key);
            let additive = key.additive(lang);
            // what the block has to write so the script ends up with `val`
            let write: Option<Val> = match (&val, &out) {
                (v, Some(o)) if !additive && v == o => None,
                (Val::Num(n), Some(Val::Num(o))) if additive => (n != o).then(|| Val::Num(n - o)),
                (Val::Stock(s), Some(Val::Stock(o))) => (s != o).then(|| {
                    Val::Stock(Stock {
                        buildable: s.buildable,
                        amount: s.amount - o.amount,
                    })
                }),
                (v, _) => Some(v.clone()),
            };
            let Some(w) = write else { continue };
            let rendered = match lang {
                ScriptLang::Dk => render_dk(&key, &w, version, &mut notes),
                ScriptLang::Lua => render_lua(&key, &w, &mut notes),
            };
            if rendered.is_empty() {
                continue;
            }
            match &key {
                &Key::Win(p) if lang == ScriptLang::Lua => {
                    body.extend(rendered);
                    lua_funcs.push(format!(
                        "function {}(eventData, triggerData)",
                        win_check_fn(p)
                    ));
                    lua_funcs.push(format!(
                        "    return {}.ALL_DUNGEONS_DESTROYED == 1",
                        p.name()
                    ));
                    lua_funcs.push("end".into());
                    lua_funcs.push(format!("function {}(eventData, triggerData)", win_fn(p)));
                    lua_funcs.push(format!("    WinGame({})", p.name()));
                    lua_funcs.push("end".into());
                }
                &Key::Lose(p) if lang == ScriptLang::Lua => {
                    body.extend(rendered);
                    lua_funcs.push(format!("function {}(eventData, triggerData)", lose_fn(p)));
                    lua_funcs.push(format!("    LoseGame({})", p.name()));
                    lua_funcs.push("end".into());
                }
                Key::Win(_) | Key::Lose(_) => win_lose.extend(rendered),
                _ => body.extend(rendered),
            }
            written.push((key, additive));
        }
        // lines outside the block that touch what the block writes
        let mut seen = BTreeSet::new();
        for (key, additive) in &written {
            for s in outside_stmts.iter().filter(|s| &s.key == key) {
                if !seen.insert((s.line, key.clone())) {
                    continue;
                }
                let after = block.is_some() && s.line > block_line || block.is_none();
                let message = if *additive {
                    format!(
                        "line {} also adds to it; the block writes the difference to \
                            reach the value set here",
                        s.line
                    )
                } else if after {
                    format!(
                        "line {} also sets it and runs after the block, so it \
                            overrides the wizard's value",
                        s.line
                    )
                } else {
                    format!("line {} also sets it", s.line)
                };
                conflicts.push(Conflict {
                    line: s.line,
                    text: s.text.clone(),
                    what: key.describe(lang),
                    message,
                });
            }
        }
        // what the form leaves unset (or switched off) but lines outside the block still set
        for s in &outside_stmts {
            if self.value(&s.key).is_none() && seen.insert((s.line, s.key.clone())) {
                conflicts.push(Conflict {
                    line: s.line,
                    text: s.text.clone(),
                    what: s.key.describe(lang),
                    message: format!(
                        "set on line {} outside the wizard block; remove it there to unset it",
                        s.line
                    ),
                });
            }
        }
        conflicts.sort_by(|a, b| (a.line, &a.what).cmp(&(b.line, &b.what)));

        let mut out_lines = vec![];
        if !body.is_empty() || !win_lose.is_empty() || !lua_funcs.is_empty() {
            match lang {
                ScriptLang::Dk => {
                    out_lines.push(format!("REM {BEGIN_MARK}"));
                    out_lines.push(
                        "REM Level setup from the script wizard. Lines between these \
                            markers are rewritten when it is applied again."
                            .into(),
                    );
                    out_lines.extend(body);
                    out_lines.extend(win_lose);
                    out_lines.push(format!("REM {END_MARK}"));
                }
                ScriptLang::Lua => {
                    out_lines.push(format!("-- {BEGIN_MARK}"));
                    out_lines.push(
                        "-- Level setup from the script wizard. Lines between these \
                            markers are rewritten when it is applied again."
                            .into(),
                    );
                    out_lines.push(format!("function {LUA_SETUP_FN}()"));
                    out_lines.extend(body.into_iter().map(|l| format!("    {l}")));
                    out_lines.push("end".into());
                    out_lines.extend(lua_funcs);
                    // OnGameStart must call the setup: define it when the file has none
                    let outside_text = outside_text(script, &lines, block);
                    if !outside_text.contains("function OnGameStart") {
                        out_lines.push("function OnGameStart()".into());
                        out_lines.push(format!("    {LUA_SETUP_FN}()"));
                        out_lines.push("end".into());
                    } else if !outside_text.contains(&format!("{LUA_SETUP_FN}()")) {
                        notes.push(format!(
                            "The script has its own OnGameStart: call {LUA_SETUP_FN}() \
                                from it, or the wizard's setup never runs."
                        ));
                    }
                    out_lines.push(format!("-- {END_MARK}"));
                }
            }
        }
        let crlf = crlf_of(script);
        let (span, lead_newline) = match block {
            Some((a, b)) => (Span::new(lines.starts[a], lines.next(script, b)), false),
            None => {
                let at = match lang {
                    ScriptLang::Dk => dk_insert_at(script),
                    ScriptLang::Lua => lua_insert_at(script),
                };
                (Span::new(at, at), at > 0 && script[at - 1] != b'\n')
            }
        };
        WizardPlan {
            lang,
            span,
            lines: out_lines,
            conflicts,
            notes,
            crlf,
            lead_newline,
        }
    }
}

fn outside_text(src: &[u8], lines: &Lines, block: Option<(usize, usize)>) -> String {
    let mut s = String::new();
    for l in 0..lines.count() {
        if block.map_or(false, |(a, b)| l >= a && l <= b) {
            continue;
        }
        s.push_str(&lines.text(src, l));
        s.push('\n');
    }
    s
}

/// What a list of the form holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Room,
    Power,
    Trap,
    Door,
    Creature,
}

impl ItemKind {
    fn const_kind(self) -> ConstKind {
        match self {
            ItemKind::Room => ConstKind::Room,
            ItemKind::Power => ConstKind::Power,
            ItemKind::Trap => ConstKind::Trap,
            ItemKind::Door => ConstKind::Door,
            ItemKind::Creature => ConstKind::Creature,
        }
    }

    /// The stock names (`config/fxdata/terrain.cfg`, `magic.cfg`, `trapdoor.cfg`, `creature.cfg`
    /// `Creatures`), used when the host supplies none.
    fn stock(self) -> &'static [&'static str] {
        match self {
            ItemKind::Room => &[
                "TREASURE",
                "LAIR",
                "GARDEN",
                "TRAINING",
                "RESEARCH",
                "BRIDGE",
                "GUARD_POST",
                "WORKSHOP",
                "PRISON",
                "TORTURE",
                "BARRACKS",
                "TEMPLE",
                "GRAVEYARD",
                "SCAVENGER",
            ],
            ItemKind::Power => &[
                "POWER_HAND",
                "POWER_SLAP",
                "POWER_POSSESS",
                "POWER_IMP",
                "POWER_OBEY",
                "POWER_SIGHT",
                "POWER_CALL_TO_ARMS",
                "POWER_CAVE_IN",
                "POWER_HEAL_CREATURE",
                "POWER_HOLD_AUDIENCE",
                "POWER_LIGHTNING",
                "POWER_SPEED",
                "POWER_PROTECT",
                "POWER_CONCEAL",
                "POWER_DISEASE",
                "POWER_CHICKEN",
                "POWER_DESTROY_WALLS",
                "POWER_TIME_BOMB",
                "POWER_ARMAGEDDON",
                "POWER_REBOUND",
                "POWER_FREEZE",
                "POWER_SLOW",
                "POWER_FLIGHT",
                "POWER_VISION",
                "POWER_TUNNELLER",
            ],
            ItemKind::Trap => &[
                "BOULDER",
                "ALARM",
                "POISON_GAS",
                "LIGHTNING",
                "WORD_OF_POWER",
                "LAVA",
                "TNT",
                "SENTRY",
                "BALLISTA",
            ],
            ItemKind::Door => &["WOOD", "BRACED", "STEEL", "MAGIC", "SECRET", "MIDAS"],
            ItemKind::Creature => &[
                "IMP",
                "BUG",
                "SPIDER",
                "FLY",
                "DEMONSPAWN",
                "TROLL",
                "ORC",
                "DARK_MISTRESS",
                "BILE_DEMON",
                "HELL_HOUND",
                "SORCEROR",
                "TENTACLE",
                "VAMPIRE",
                "SKELETON",
                "GHOST",
                "DRAGON",
                "HORNY",
                "WIZARD",
                "BARBARIAN",
                "ARCHER",
                "MONK",
                "DWARFA",
                "KNIGHT",
                "AVATAR",
                "TUNNELLER",
                "WITCH",
                "GIANT",
                "FAIRY",
                "THIEF",
                "SAMURAI",
                "TIME_MAGE",
                "DRUID",
                "BIRD",
                "MAIDEN",
                "SPIDERLING",
            ],
        }
    }

    /// Names that are not real choices here: placeholders, rooms no script makes available,
    /// the hand's child powers (`ParentPower = POWER_HAND`, `config/fxdata/magic.cfg`).
    fn hidden(self, name: &str) -> bool {
        matches!(
            name,
            "NULL"
                | "NONE"
                | "NOPOWER"
                | "NOTRAP"
                | "NODOOR"
                | "NOCREATURE"
                | "ENTRANCE"
                | "DUNGEON_HEART"
                | "FLOATING_SPIRIT"
        ) || name.starts_with("POWER_PICKUP_")
    }
}

/// Names shown in a list of the form: the host's names (stock ones when it has none) plus
/// every name the model already mentions.
pub fn item_names(
    kind: ItemKind,
    consts: &dyn ConstantProvider,
    model: &WizardModel,
) -> Vec<String> {
    let host = consts.names(kind.const_kind());
    let mut out: Vec<String> = if host.is_empty() {
        kind.stock().iter().map(|s| s.to_string()).collect()
    } else {
        host.iter()
            .map(|s| s.to_ascii_uppercase())
            .filter(|n| !kind.hidden(n))
            .collect()
    };
    let mut extra: BTreeSet<String> = BTreeSet::new();
    for p in &model.players {
        let names: Vec<&String> = match kind {
            ItemKind::Room => p.rooms.keys().collect(),
            ItemKind::Power => p.powers.keys().collect(),
            ItemKind::Trap => p.traps.keys().collect(),
            ItemKind::Door => p.doors.keys().collect(),
            ItemKind::Creature => p.creatures.keys().collect(),
        };
        extra.extend(names.into_iter().cloned());
    }
    if kind == ItemKind::Creature {
        extra.extend(model.pool.keys().cloned());
    }
    for n in extra {
        if !out.contains(&n) {
            out.push(n);
        }
    }
    let mut seen = BTreeSet::new();
    out.retain(|n| seen.insert(n.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_parse_like_strtol() {
        assert_eq!(parse_num("12"), Some(12));
        assert_eq!(parse_num("-3"), Some(-3));
        assert_eq!(parse_num("0x10"), Some(16));
        assert_eq!(parse_num("010"), Some(8));
        assert_eq!(parse_num("0"), Some(0));
        assert_eq!(parse_num("7abc"), Some(7));
        assert_eq!(parse_num("abc"), None);
    }

    #[test]
    fn lua_calls_split_arguments() {
        assert_eq!(
            lua_call("    RoomAvailable(PLAYER0, \"TEMPLE\", 1, true) -- note"),
            Some((
                "RoomAvailable".into(),
                vec!["PLAYER0".into(), "TEMPLE".into(), "1".into(), "true".into()]
            ))
        );
        assert_eq!(lua_call("x = f(1)"), None);
        assert_eq!(lua_call("WinGame()"), Some(("WinGame".into(), vec![])));
    }
}
