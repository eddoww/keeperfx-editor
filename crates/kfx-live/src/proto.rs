//! The wire protocol of the game's API server, as the C code implements it.
//!
//! Everything below cites KeeperFX's `src/api.c` unless another file is named. The wiki page
//! `API.md` is the public description; where it and the code differ, the code is followed and the
//! difference is listed under "Wiki vs code".
//!
//! # Enabling the server
//!
//! * Config keys of `keeperfx.cfg`: `API_ENABLED=ON` and `API_PORT=<n>`
//!   (`src/config_keeperfx.c:163-164` and `838-860`; defaults `false` / 5599,
//!   `src/config_keeperfx.c:65-66`). No dedicated command line switch, but two override paths leave the
//!   user's file alone: `-cfg KEY=VALUE` (repeatable, `main.cpp:1968-1985`, applied after the base
//!   file and the mods' copies, `src/config_keeperfx.c:1301-1302`; added in 2026-10, #5554) and
//!   `-config <file>` (a whole other base config file, an absolute path or a file of the install
//!   folder, `main.cpp:1962-1967`, `src/config_keeperfx.c:1273-1290`). `-alex` (cheats) is unrelated.
//! * `api_init_server()` is called once, right before `game_loop()` (`main.cpp:2236`), and does
//!   nothing when `API_ENABLED` is off (`src/api.c:219`).
//! * The socket is IPv4, `bind(INADDR_LOOPBACK)`: **127.0.0.1 only**, `listen(srv, 1)`, non
//!   blocking (`src/api.c:265`, `src/api.c:276`). The wiki does not say it is loopback only.
//! * One client at a time: a second connection is accepted and immediately closed with the log line
//!   "Got another connection while API connection is still active" (`src/api.c:1637`). The server is
//!   polled by `api_update_server()` from the in-game `update()` once per game turn
//!   (`game_update.cpp:480`) and once per iteration of the frontend (menu) loop
//!   (`src/game_loop.c:1011`). Subscriptions are dropped when the client disconnects (`src/api.c:1680`,
//!   `src/api.c:1695`).
//!
//! # Framing
//!
//! * Plain TCP, UTF-8 JSON objects. Requests need no delimiter, responses and events end with `\n`.
//! * **Server side parsing** (`api_update_server` `src/api.c:1614-1705`, `api_process_multipart_json`
//!   `src/api.c:1564`): one non-blocking `recv` of at most 4095 bytes per poll; when the received text
//!   ends with `\n` that single newline is removed; then the text is cut into objects by counting
//!   `{` and `}` (strings are *not* understood, so a brace inside a string value must be balanced
//!   or escaped as `{` / `}`, which this crate does). Every object found is handled in that poll,
//!   in order. An unterminated object gives `INVALID_JSON_IN_PACKET` and its bytes are lost (a
//!   request split across two `recv`s is therefore broken: send each request with one `write` and
//!   keep it well under 4095 bytes; this crate refuses longer ones and, by default, keeps one
//!   request in flight). A NUL byte ends the text (`strlen`).
//! * **Responses**: one minimised JSON object + `\n`. The game's JSON library writes object members
//!   **sorted by key**, so the order is `ack`, `data` / `error`, `success`:
//!   `{"ack":11,"data":9234,"success":true}`, `{"ack":4,"error":"UNKNOWN_VAR","success":false}`
//!   (`api_ok` `src/api.c:362`, `api_err` `src/api.c:307`, `api_return_data` `src/api.c:423`,
//!   `api_return_data_number` `src/api.c:561`; observed in the integration test). Without `ack`, plain
//!   ok and numbers are literal strings: `{"success":true}`, `{"success":true,"data":N}`. `ack` is
//!   echoed as received (any JSON value) and only when the request had one. Requests that fail
//!   before the `ack` was read (`NO_JSON`, `INVALID_JSON`, `INVALID_JSON_OBJECT`,
//!   `INVALID_JSON_IN_PACKET`) are answered without it.
//! * **Size limits**: ok / error / data responses are built in a **1024 byte** buffer
//!   (`src/api.c:336,393,452,531,597`); a data response that does not fit becomes the error
//!   `FAILED_TO_CREATE_JSON` (`src/api.c:459`). Events use a 4096 byte buffer (`API_SERVER_BUFFER`,
//!   `src/api.c:47`, `src/api.c:994`). There is no ordering guarantee between a response and an event
//!   except what the single thread gives: replies of one poll are sent in request order, then
//!   variable updates of that poll (`api_check_var_update`, called at `src/api.c:1704`).
//!
//! # Requests (`action`)
//!
//! Optional on every request: `"ack"` (any JSON value, echoed), `"player"` (an integer, or a name
//! such as `"PLAYER1"`, resolved with `get_id(player_desc, ..)`; default `my_player_number`; an
//! unknown name gives -1 and is **not** rejected, `src/api.c:1086-1096`; this crate validates). The
//! action name is matched case-insensitively (`strcasecmp`).
//!
//! Work in any state:
//!
//! | action | extra members | result |
//! |---|---|---|
//! | `get_kfx_info` | | `data: {"kfx_version": "..."}` |
//! | `subscribe_var` | `var` (a script variable name, `parse_get_varib`) | ok; `MISSING_VAR`, `UNKNOWN_VAR`, `SUB_FAILED` |
//! | `unsubscribe_var` | `var` | ok; same errors (an unsubscribe failure is also `SUB_FAILED`) |
//! | `subscribe_event` | `event` (a name, at most 256 chars: `STRING_TOO_LONG`) | ok; `MISSING_EVENT`, `SUB_FAILED` |
//! | `unsubscribe_event` | `event` | ok |
//! | `unsubscribe_all` | | ok |
//!
//! Subscriptions: at most 256 slots (`API_SUBSCRIBE_LIST_SIZE`, `src/api.c:49`); event names are not
//! checked against a list, anything can be subscribed. A subscribed variable is compared each poll
//! with the value remembered at subscription time and a `VAR_UPDATE` is sent on change
//! (`api_check_var_update`, `src/api.c:863`).
//!
//! Everything below requires `game.game_kind == GKind_LocalGame`; otherwise **any** action that
//! is not in the table above (even an unknown one) is answered `NOT_IN_LOCAL_GAME` (`src/api.c:1272`).
//! That includes the menus: in the frontend no local game exists.
//!
//! | action | members | result |
//! |---|---|---|
//! | `map_command` | `command`: one DK script line | ok / `GAME_IS_PAUSED` / `MISSING_COMMAND` / `FAILED_TO_EXECUTE_MAP_COMMAND` |
//! | `console_command` | `command`: console line (a leading `!` is dropped), `player` | ok / `GAME_IS_PAUSED` / `MISSING_COMMAND` / `FAILED_TO_EXECUTE_CONSOLE_COMMAND` |
//! | `get_all_player_flags` | | `data: {"PLAYER0": {"FLAG0": n, ...}, ... "PLAYER_NEUTRAL": {...}}` |
//! | `read_var` | `var`, `player` | `data: <int>` |
//! | `set_var` | `var`, `player`, `value` (int) | ok; only `FLAG`, `CAMPAIGN_FLAG`, `BOX_ACTIVATED`, `TRAP_ACTIVATED`, `SACRIFICED`, `REWARDED` (`UNABLE_TO_SET_VAR`, `VALUE_MUST_BE_INT`) |
//! | `get_level_info` / `get_map_info` | | `data: {level_name, level_number, players, mapsize_x, mapsize_y, is_multiplayer, campaign: {campaign_name, campaign_display_name, campaign_fname, is_map_pack}}` |
//! | `get_current_game_info` | | `data: {"game_turn": n}` |
//!
//! Anything else: `UNKNOWN_ACTION`.
//!
//! * **`map_command`** runs `script_scan_line(line, preloaded=false, version 99)` (`src/api.c:1298`):
//!   exactly one DK script line, executed *immediately* when the script's current condition is
//!   `ALWAYS` (`PROCESS_SCRIPT_VALUE`, `src/lvl_script_lib.h:398`). Not available: scopes and
//!   conditions (`IF`/`WHILE` would change the global current condition and make later commands
//!   wait), and the six preloaded commands (`LEVEL_VERSION`, `NEW_*_TYPE`, `COPY_CREATURE_TYPE`,
//!   `src/lvl_script.c:196`) which are **silently ignored with `success:true`**. `RUN_LUA_CODE("...")`
//!   runs Lua, but copies its text into the level's 8192 byte script string pool for good
//!   (`script_strdup`, `src/lvl_script_lib.c:57`; `game.script.strings[8192]`, `src/lvl_script.h:186`):
//!   define helper functions once and call them with short snippets.
//! * **`console_command`**: `cmd_exec(player, line, cursor, cursor, issued_here=true)`
//!   (`src/api.c:1342`). `success` is true for `CmdR_Succeeded` / `CmdR_SucceededLocally`. Commands
//!   marked "cheats required" in `console_cmd.c` (`lua`, `reveal`, `zoomto.*`, `player.addgold`,
//!   `create.*`, `thing.*`, `room.available`, `power.give`, ...) fail unless the game was started
//!   with `-alex` (`start_params.easter_egg`, `main.cpp:1893`, `src/net_game.c:180`) or cheats were
//!   enabled for a skirmish. Unrestricted ones that matter: `pause`, `step`, `fps <n>`, `quit`,
//!   `game.save <slot>`, `game.load <slot>`, `turn`, `volume*`, `music.set`.
//! * **Pause**: `map_command` and `console_command` are refused while the game is paused
//!   (`GOF_Paused`, `src/api.c:1283`, `src/api.c:1318`) so the API cannot unpause (`pause` and `step` are
//!   console commands). The read-only requests are not refused.
//!
//! # Events
//!
//! Sent only to a client subscribed to that name (`api_event_with_data`, `src/api.c:929`). Shape:
//! `{"event":"NAME"}` or `{"event":"NAME","data":{...}}`.
//!
//! | event | emitted from | data |
//! |---|---|---|
//! | `GAME_STARTED` | `src/main_game.c:289` | |
//! | `GAME_ENDED` | `src/game_loop.c:767` | |
//! | `CAMPAIGN_LOADED` | `src/front_landview.c:1176` | |
//! | `GAME_SAVED` / `GAME_LOADED` | `SaveManager.cpp:101` / `:370` | |
//! | `WIN_GAME` / `LOSE_GAME` | `src/player_utils.c:167` / `:218` | |
//! | `SPECIAL_ACTIVATED` | `src/power_specials.c:545` | `player, special, special_name, special_value, thing, model, x, y, z, level_number, game_turn` |
//! | `RESEARCH_COMPLETED` | `src/room_library.c:458` | `player, category, kind, kind_description, level_number` |
//! | `MANUFACTURE_COMPLETED` | `src/room_workshop.c:960` | `player, class, class_description, kind, kind_description, level_number` |
//! | `ACTION_POINT` | `src/actionpt.c:289` | `player, action_point` |
//!
//! Variable updates have their own shape (`api_return_var_update`, `src/api.c:497`):
//! `{"event":"VAR_UPDATE","var":{"player":"PLAYER0","name":"MONEY","value":123}}`.
//!
//! # Wiki vs code
//!
//! * Wiki: "one message is read and handled on every game tick". Code: one `recv` per tick, which
//!   can hold several objects, all handled.
//! * Wiki lists five events; the code emits the eleven above (and accepts any name).
//! * Wiki: `unsubscribe_var` can fail with `UNSUB_FAILED`; code uses `SUB_FAILED` for both.
//! * Wiki does not document `get_current_game_info`, the 1024 byte response limit,
//!   `NOT_IN_LOCAL_GAME` or the loopback-only bind. It says `unsubscribe_all` has
//!   `"action":"unsubscribe_event"` (typo).
//! * Wiki: `VAR_UPDATE` is "a var_update event"; the code's member is `var`, not `data`.
//! * `get_all_player_flags` reads the flags of the request's `player` for **every** player entry
//!   (`src/api.c:1375` uses `player_id`, not `player_index`), so all ten entries show one player's
//!   flags.
//! * `get_level_info` dereferences the level info without a NULL check (`src/api.c:1509`): on a map the
//!   campaign does not list it can crash the game.
//! * `read_var` / `set_var` with an unknown `player` name pass -1 on (no validation).
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Largest request the server reads in one go (`API_SERVER_BUFFER - 1`).
pub const MAX_REQUEST_BYTES: usize = 4095;
/// Largest ok / error / data response (the 1024 byte buffer minus the newline).
pub const MAX_REPLY_BYTES: usize = 1023;
/// Default `API_PORT`.
pub const DEFAULT_PORT: u16 = 5599;
/// Longest event name the server accepts (`COMMAND_WORD_LEN`).
pub const MAX_EVENT_NAME: usize = 256;

/// A player in a request: an index or a name (`PLAYER0`..`PLAYER6`, `PLAYER_GOOD`,
/// `PLAYER_NEUTRAL`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Player {
    Index(i32),
    Name(String),
}

impl Player {
    pub const P0: Player = Player::Index(0);
    pub fn name(&self) -> String {
        match self {
            Player::Index(i) => format!("PLAYER{i}"),
            Player::Name(n) => n.clone(),
        }
    }
    /// The script spelling (`PLAYER0`), for DK script lines.
    pub fn script_name(&self) -> String {
        match self {
            Player::Index(i) if (0..=6).contains(i) => format!("PLAYER{i}"),
            Player::Index(i) => i.to_string(),
            Player::Name(n) => n.clone(),
        }
    }
    fn to_json(&self) -> Value {
        match self {
            Player::Index(i) => json!(i),
            Player::Name(n) => json!(n),
        }
    }
}

impl From<i32> for Player {
    fn from(i: i32) -> Self {
        Player::Index(i)
    }
}

/// A request, typed. [`Request::to_json`] builds the object the server expects.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    GetKfxInfo,
    GetLevelInfo,
    GetCurrentGameInfo,
    GetAllPlayerFlags,
    SubscribeEvent(String),
    UnsubscribeEvent(String),
    SubscribeVar {
        var: String,
        player: Option<Player>,
    },
    UnsubscribeVar {
        var: String,
        player: Option<Player>,
    },
    UnsubscribeAll,
    ReadVar {
        var: String,
        player: Option<Player>,
    },
    SetVar {
        var: String,
        player: Option<Player>,
        value: i32,
    },
    MapCommand(String),
    ConsoleCommand {
        command: String,
        player: Option<Player>,
    },
    /// Any object (must contain `action`); `ack` is still added.
    Raw(Value),
}

impl Request {
    pub fn action(&self) -> &str {
        match self {
            Request::GetKfxInfo => "get_kfx_info",
            Request::GetLevelInfo => "get_level_info",
            Request::GetCurrentGameInfo => "get_current_game_info",
            Request::GetAllPlayerFlags => "get_all_player_flags",
            Request::SubscribeEvent(_) => "subscribe_event",
            Request::UnsubscribeEvent(_) => "unsubscribe_event",
            Request::SubscribeVar { .. } => "subscribe_var",
            Request::UnsubscribeVar { .. } => "unsubscribe_var",
            Request::UnsubscribeAll => "unsubscribe_all",
            Request::ReadVar { .. } => "read_var",
            Request::SetVar { .. } => "set_var",
            Request::MapCommand(_) => "map_command",
            Request::ConsoleCommand { .. } => "console_command",
            Request::Raw(v) => v.get("action").and_then(Value::as_str).unwrap_or("?"),
        }
    }

    pub fn to_json(&self, ack: Option<u64>) -> Value {
        let mut m = Map::new();
        if let Some(a) = ack {
            m.insert("ack".into(), json!(a));
        }
        m.insert("action".into(), json!(self.action()));
        let player = |p: &Option<Player>, m: &mut Map<String, Value>| {
            if let Some(p) = p {
                m.insert("player".into(), p.to_json());
            }
        };
        match self {
            Request::SubscribeEvent(e) | Request::UnsubscribeEvent(e) => {
                m.insert("event".into(), json!(e));
            }
            Request::SubscribeVar { var, player: p }
            | Request::UnsubscribeVar { var, player: p }
            | Request::ReadVar { var, player: p } => {
                m.insert("var".into(), json!(var));
                player(p, &mut m);
            }
            Request::SetVar {
                var,
                player: p,
                value,
            } => {
                m.insert("var".into(), json!(var));
                player(p, &mut m);
                m.insert("value".into(), json!(value));
            }
            Request::MapCommand(c) => {
                m.insert("command".into(), json!(c));
            }
            Request::ConsoleCommand { command, player: p } => {
                m.insert("command".into(), json!(command));
                player(p, &mut m);
            }
            Request::Raw(Value::Object(o)) => {
                for (k, v) in o {
                    if k != "ack" {
                        m.insert(k.clone(), v.clone());
                    }
                }
            }
            _ => {}
        }
        Value::Object(m)
    }
}

/// Serialize a request as the game wants it: minimal JSON where `{` and `}` inside strings are
/// written as `{` / `}` (the server frames objects by counting braces), no newline.
pub fn encode(req: &Request, ack: Option<u64>) -> Result<Vec<u8>, ProtoError> {
    fn has_nul(v: &Value) -> bool {
        match v {
            Value::String(s) => s.contains('\0'),
            Value::Array(a) => a.iter().any(has_nul),
            Value::Object(o) => o.iter().any(|(k, v)| k.contains('\0') || has_nul(v)),
            _ => false,
        }
    }
    let json = req.to_json(ack);
    if has_nul(&json) {
        return Err(ProtoError::Encode(
            "a NUL byte in a string would end the server's text".into(),
        ));
    }
    let text = escape_braces_in_strings(
        &serde_json::to_string(&json).map_err(|e| ProtoError::Encode(e.to_string()))?,
    );
    if text.len() > MAX_REQUEST_BYTES {
        return Err(ProtoError::TooLong(text.len()));
    }
    Ok(text.into_bytes())
}

/// Replace `{` / `}` that are inside JSON string literals by their `\u00XX` escapes.
pub fn escape_braces_in_strings(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let (mut in_str, mut esc) = (false, false);
    for c in json.chars() {
        if in_str {
            if esc {
                esc = false;
                out.push(c);
                continue;
            }
            match c {
                '\\' => {
                    esc = true;
                    out.push(c);
                }
                '"' => {
                    in_str = false;
                    out.push(c);
                }
                '{' => out.push_str("\\u007b"),
                '}' => out.push_str("\\u007d"),
                _ => out.push(c),
            }
        } else {
            if c == '"' {
                in_str = true;
            }
            out.push(c);
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtoError {
    #[error("request is {0} bytes, the game reads at most 4095 per poll")]
    TooLong(usize),
    #[error("cannot encode request: {0}")]
    Encode(String),
}

/// What the server does with received bytes (`api_update_server` + `api_process_multipart_json`):
/// strip one trailing `\n`, cut into objects by brace depth. Returns the objects and whether an
/// unterminated object was left (the server answers `INVALID_JSON_IN_PACKET` then). Used by the
/// mock server of the tests, and handy to predict what the game will see.
pub fn server_split(recv: &[u8]) -> (Vec<Vec<u8>>, bool) {
    let mut buf = recv;
    if let Some(p) = buf.iter().position(|&b| b == 0) {
        buf = &buf[..p]; // strlen
    }
    if buf.last() == Some(&b'\n') {
        buf = &buf[..buf.len() - 1];
    }
    let (mut depth, mut start) = (0i32, None);
    let mut out = vec![];
    for (i, &b) in buf.iter().enumerate() {
        if b == b'{' {
            if depth == 0 {
                start = Some(i);
            }
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                if let Some(s) = start.take() {
                    out.push(buf[s..=i].to_vec());
                }
            }
        }
    }
    (out, depth > 0)
}

/// Error code of a failed response.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ApiError {
    NoJson,
    InvalidJson,
    InvalidJsonObject,
    InvalidJsonInPacket,
    MissingAction,
    MissingVar,
    UnknownVar,
    SubFailed,
    MissingEvent,
    StringTooLong,
    NotInLocalGame,
    GameIsPaused,
    MissingCommand,
    FailedToExecuteMapCommand,
    FailedToExecuteConsoleCommand,
    UnableToSetVar,
    ValueMustBeInt,
    UnknownAction,
    FailedToCreateJson,
    Other(String),
}

impl ApiError {
    pub fn from_code(c: &str) -> ApiError {
        match c {
            "NO_JSON" => ApiError::NoJson,
            "INVALID_JSON" => ApiError::InvalidJson,
            "INVALID_JSON_OBJECT" => ApiError::InvalidJsonObject,
            "INVALID_JSON_IN_PACKET" => ApiError::InvalidJsonInPacket,
            "MISSING_ACTION" => ApiError::MissingAction,
            "MISSING_VAR" => ApiError::MissingVar,
            "UNKNOWN_VAR" => ApiError::UnknownVar,
            "SUB_FAILED" => ApiError::SubFailed,
            "MISSING_EVENT" => ApiError::MissingEvent,
            "STRING_TOO_LONG" => ApiError::StringTooLong,
            "NOT_IN_LOCAL_GAME" => ApiError::NotInLocalGame,
            "GAME_IS_PAUSED" => ApiError::GameIsPaused,
            "MISSING_COMMAND" => ApiError::MissingCommand,
            "FAILED_TO_EXECUTE_MAP_COMMAND" => ApiError::FailedToExecuteMapCommand,
            "FAILED_TO_EXECUTE_CONSOLE_COMMAND" => ApiError::FailedToExecuteConsoleCommand,
            "UNABLE_TO_SET_VAR" => ApiError::UnableToSetVar,
            "VALUE_MUST_BE_INT" => ApiError::ValueMustBeInt,
            "UNKNOWN_ACTION" => ApiError::UnknownAction,
            "FAILED_TO_CREATE_JSON" => ApiError::FailedToCreateJson,
            other => ApiError::Other(other.to_string()),
        }
    }

    pub fn code(&self) -> &str {
        match self {
            ApiError::NoJson => "NO_JSON",
            ApiError::InvalidJson => "INVALID_JSON",
            ApiError::InvalidJsonObject => "INVALID_JSON_OBJECT",
            ApiError::InvalidJsonInPacket => "INVALID_JSON_IN_PACKET",
            ApiError::MissingAction => "MISSING_ACTION",
            ApiError::MissingVar => "MISSING_VAR",
            ApiError::UnknownVar => "UNKNOWN_VAR",
            ApiError::SubFailed => "SUB_FAILED",
            ApiError::MissingEvent => "MISSING_EVENT",
            ApiError::StringTooLong => "STRING_TOO_LONG",
            ApiError::NotInLocalGame => "NOT_IN_LOCAL_GAME",
            ApiError::GameIsPaused => "GAME_IS_PAUSED",
            ApiError::MissingCommand => "MISSING_COMMAND",
            ApiError::FailedToExecuteMapCommand => "FAILED_TO_EXECUTE_MAP_COMMAND",
            ApiError::FailedToExecuteConsoleCommand => "FAILED_TO_EXECUTE_CONSOLE_COMMAND",
            ApiError::UnableToSetVar => "UNABLE_TO_SET_VAR",
            ApiError::ValueMustBeInt => "VALUE_MUST_BE_INT",
            ApiError::UnknownAction => "UNKNOWN_ACTION",
            ApiError::FailedToCreateJson => "FAILED_TO_CREATE_JSON",
            ApiError::Other(s) => s,
        }
    }

    /// A short explanation for the UI.
    pub fn explain(&self) -> &'static str {
        match self {
            ApiError::NotInLocalGame => {
                "no single player game is running (menu, or a multiplayer \
                game)"
            }
            ApiError::GameIsPaused => {
                "the game is paused; the API refuses commands while paused \
                and cannot unpause"
            }
            ApiError::FailedToExecuteConsoleCommand => {
                "unknown console command, bad arguments, or \
                the command needs cheat mode (start the game with -alex)"
            }
            ApiError::FailedToExecuteMapCommand => {
                "the script line did not parse (one DK script \
                command per line, no scopes)"
            }
            ApiError::FailedToCreateJson => {
                "the answer did not fit the game's 1024 byte reply \
                buffer"
            }
            ApiError::UnknownVar => "not a script variable name",
            ApiError::UnableToSetVar => {
                "only FLAG, CAMPAIGN_FLAG, BOX/TRAP_ACTIVATED, SACRIFICED, \
                REWARDED can be set"
            }
            _ => "",
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code())
    }
}

/// A response to a request.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub ack: Option<u64>,
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<ApiError>,
}

impl Reply {
    pub fn into_result(self) -> Result<Option<Value>, ApiError> {
        if self.success {
            Ok(self.data)
        } else {
            Err(self.error.unwrap_or(ApiError::Other("unknown".into())))
        }
    }
    /// `data` as an integer (`read_var`).
    pub fn int(&self) -> Option<i64> {
        self.data.as_ref().and_then(Value::as_i64)
    }
}

/// An event (`{"event":..}`), including variable updates.
#[derive(Clone, Debug, PartialEq)]
pub struct GameEvent {
    pub name: String,
    pub data: Option<Value>,
    /// Set for `VAR_UPDATE`.
    pub var: Option<VarUpdate>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VarUpdate {
    pub player: String,
    pub name: String,
    pub value: i64,
}

/// One parsed line from the server.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Reply(Reply),
    Event(GameEvent),
    /// Valid JSON that is neither.
    Other(Value),
}

/// Parse a line of server output.
pub fn parse_incoming(line: &str) -> Result<Incoming, String> {
    let v: Value = serde_json::from_str(line.trim()).map_err(|e| e.to_string())?;
    let Some(o) = v.as_object() else {
        return Ok(Incoming::Other(v));
    };
    if let Some(name) = o.get("event").and_then(Value::as_str) {
        let var = o.get("var").and_then(|x| {
            Some(VarUpdate {
                player: x.get("player")?.as_str()?.to_string(),
                name: x.get("name")?.as_str()?.to_string(),
                value: x.get("value")?.as_i64()?,
            })
        });
        return Ok(Incoming::Event(GameEvent {
            name: name.to_string(),
            data: o.get("data").cloned(),
            var,
        }));
    }
    if let Some(s) = o.get("success").and_then(Value::as_bool) {
        return Ok(Incoming::Reply(Reply {
            ack: o.get("ack").and_then(Value::as_u64),
            success: s,
            data: o.get("data").cloned(),
            error: o
                .get("error")
                .and_then(Value::as_str)
                .map(ApiError::from_code),
        }));
    }
    Ok(Incoming::Other(v))
}

/// Typed `get_level_info` data.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LevelInfo {
    pub level_name: Option<String>,
    pub level_number: i64,
    pub players: i64,
    pub mapsize_x: i64,
    pub mapsize_y: i64,
    pub is_multiplayer: bool,
    pub campaign: CampaignInfo,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CampaignInfo {
    pub campaign_name: Option<String>,
    pub campaign_display_name: Option<String>,
    pub campaign_fname: Option<String>,
    pub is_map_pack: bool,
}

impl LevelInfo {
    pub fn from_value(v: &Value) -> Option<LevelInfo> {
        serde_json::from_value(v.clone()).ok()
    }
}
