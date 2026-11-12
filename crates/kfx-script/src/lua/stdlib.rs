//! The standard library as KeeperFX exposes it: LuaJIT base, `table`, `io`, `os`, `string`,
//! `math`, `debug`, `package` and `bit` (`ffi` and `jit` are NOT opened,
//! `src/lua_base.c:441-451`), with the sandbox applied (`src/lua_base.c:83-103`).

/// Base-library globals (functions and values) available in level scripts.
pub const BASE_GLOBALS: &[&str] = &[
    "assert",
    "collectgarbage",
    "dofile",
    "error",
    "gcinfo",
    "getfenv",
    "getmetatable",
    "ipairs",
    "load",
    "loadfile",
    "loadstring",
    "newproxy",
    "next",
    "pairs",
    "pcall",
    "print",
    "rawequal",
    "rawget",
    "rawset",
    "require",
    "select",
    "setfenv",
    "setmetatable",
    "tonumber",
    "tostring",
    "type",
    "unpack",
    "xpcall",
    "_G",
    "_VERSION",
    "module",
    "coroutine",
    "string",
    "table",
    "math",
    "io",
    "os",
    "debug",
    "package",
    "bit",
];

/// `(library, members)` of the libraries that are opened.
pub const LIBS: &[(&str, &[&str])] = &[
    (
        "string",
        &[
            "byte", "char", "dump", "find", "format", "gmatch", "gsub", "len", "lower", "match",
            "rep", "reverse", "sub", "upper",
        ],
    ),
    (
        "table",
        &[
            "concat", "foreach", "foreachi", "getn", "insert", "maxn", "move", "remove", "sort",
        ],
    ),
    (
        "math",
        &[
            "abs",
            "acos",
            "asin",
            "atan",
            "atan2",
            "ceil",
            "cos",
            "cosh",
            "deg",
            "exp",
            "floor",
            "fmod",
            "frexp",
            "huge",
            "ldexp",
            "log",
            "log10",
            "max",
            "min",
            "modf",
            "pi",
            "pow",
            "rad",
            "random",
            "randomseed",
            "sin",
            "sinh",
            "sqrt",
            "tan",
            "tanh",
        ],
    ),
    (
        "bit",
        &[
            "arshift", "band", "bnot", "bor", "bswap", "bxor", "lshift", "rol", "ror", "rshift",
            "tobit", "tohex",
        ],
    ),
    (
        "coroutine",
        &[
            "create",
            "isyieldable",
            "resume",
            "running",
            "status",
            "wrap",
            "yield",
        ],
    ),
    (
        "debug",
        &[
            "debug",
            "getfenv",
            "gethook",
            "getinfo",
            "getlocal",
            "getmetatable",
            "getregistry",
            "getupvalue",
            "getuservalue",
            "setfenv",
            "sethook",
            "setlocal",
            "setmetatable",
            "setupvalue",
            "setuservalue",
            "traceback",
            "upvalueid",
            "upvaluejoin",
        ],
    ),
    (
        "package",
        &[
            "config",
            "cpath",
            "loaded",
            "loaders",
            "path",
            "preload",
            "searchers",
            "searchpath",
            "seeall",
        ],
    ),
    // sandbox: `io` keeps only `open`, `os` keeps only `remove` (replaced by erroring stubs for
    // time/date/clock in multiplayer)
    ("io", &["open"]),
    ("os", &["remove"]),
];

/// Members removed by the sandbox, with the reason. `(lib, member, message)`.
pub const SANDBOX_REMOVED: &[(&str, &str, &str)] = &[
    (
        "os",
        "time",
        "os.time is disabled in multiplayer (not synchronised); use GetGameTimestamp",
    ),
    (
        "os",
        "date",
        "os.date is disabled in multiplayer (not synchronised); use GetGameTimestamp",
    ),
    (
        "os",
        "clock",
        "os.clock is disabled in multiplayer (not synchronised); use GetGameTimestamp",
    ),
    (
        "os",
        "execute",
        "os.execute is removed by the KeeperFX Lua sandbox",
    ),
    (
        "os",
        "exit",
        "os.exit is removed by the KeeperFX Lua sandbox",
    ),
    (
        "os",
        "getenv",
        "os.getenv is removed by the KeeperFX Lua sandbox",
    ),
    (
        "os",
        "rename",
        "os.rename is removed by the KeeperFX Lua sandbox",
    ),
    (
        "os",
        "tmpname",
        "os.tmpname is removed by the KeeperFX Lua sandbox",
    ),
    (
        "os",
        "setlocale",
        "os.setlocale is removed by the KeeperFX Lua sandbox",
    ),
    (
        "os",
        "difftime",
        "os.difftime is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "popen",
        "io.popen is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "lines",
        "io.lines is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "read",
        "io.read is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "write",
        "io.write is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "close",
        "io.close is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "input",
        "io.input is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "output",
        "io.output is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "tmpfile",
        "io.tmpfile is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "type",
        "io.type is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "stdin",
        "io.stdin is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "stdout",
        "io.stdout is removed by the KeeperFX Lua sandbox",
    ),
    (
        "io",
        "stderr",
        "io.stderr is removed by the KeeperFX Lua sandbox",
    ),
    (
        "package",
        "loadlib",
        "package.loadlib is removed by the KeeperFX Lua sandbox",
    ),
];

/// Libraries that exist in LuaJIT but are not opened by KeeperFX.
pub const UNOPENED_LIBS: &[&str] = &["ffi", "jit"];

/// Functions that exist but make multiplayer clients diverge or touch the host machine.
/// `(path, message)`.
pub const MP_UNSAFE: &[(&str, &str)] = &[
    (
        "math.randomseed",
        "math.randomseed changes the shared random state; the game seeds it \
        deterministically at level start",
    ),
    (
        "io.open",
        "io.open reads/writes host files; results differ between multiplayer clients",
    ),
    (
        "os.remove",
        "os.remove deletes host files; it is not synchronised between multiplayer \
        clients",
    ),
    (
        "collectgarbage",
        "collectgarbage results (memory counts) differ between multiplayer clients",
    ),
    (
        "gcinfo",
        "gcinfo results differ between multiplayer clients",
    ),
    (
        "dofile",
        "dofile loads host files; results differ between multiplayer clients",
    ),
    (
        "loadfile",
        "loadfile loads host files; results differ between multiplayer clients",
    ),
];

pub fn is_std_global(name: &str) -> bool {
    BASE_GLOBALS.contains(&name) || name == "arg"
}

pub fn lib_members(lib: &str) -> Option<&'static [&'static str]> {
    LIBS.iter().find(|l| l.0 == lib).map(|l| l.1)
}

pub fn sandbox_removed(lib: &str, member: &str) -> Option<&'static str> {
    SANDBOX_REMOVED
        .iter()
        .find(|r| r.0 == lib && r.1 == member)
        .map(|r| r.2)
}

pub fn mp_unsafe(path: &str) -> Option<&'static str> {
    MP_UNSAFE.iter().find(|r| r.0 == path).map(|r| r.1)
}

/// Signature strings for hover and signature help of common functions.
pub const SIGNATURES: &[(&str, &str)] = &[
    ("print", "print(...)"),
    ("pairs", "pairs(t: table): fun(), table, nil"),
    ("ipairs", "ipairs(t: table): fun(), table, integer"),
    ("next", "next(t: table, key?: any): any, any"),
    ("type", "type(v: any): string"),
    ("tostring", "tostring(v: any): string"),
    ("tonumber", "tonumber(v: any, base?: integer): number|nil"),
    ("select", "select(index: integer|\"#\", ...): any"),
    ("unpack", "unpack(t: table, i?: integer, j?: integer): ..."),
    ("pcall", "pcall(f: function, ...): boolean, ..."),
    (
        "xpcall",
        "xpcall(f: function, err: function, ...): boolean, ...",
    ),
    ("error", "error(message: any, level?: integer)"),
    ("assert", "assert(v: any, message?: any): any"),
    (
        "setmetatable",
        "setmetatable(t: table, mt: table|nil): table",
    ),
    ("getmetatable", "getmetatable(v: any): table|nil"),
    ("rawget", "rawget(t: table, k: any): any"),
    ("rawset", "rawset(t: table, k: any, v: any): table"),
    ("require", "require(module: string): any"),
    ("string.format", "string.format(fmt: string, ...): string"),
    (
        "string.sub",
        "string.sub(s: string, i: integer, j?: integer): string",
    ),
    (
        "string.find",
        "string.find(s: string, pattern: string, init?: integer, plain?: boolean): \
        integer, integer",
    ),
    (
        "string.gsub",
        "string.gsub(s: string, pattern: string, repl: string|table|function, n?: \
        integer): string, integer",
    ),
    (
        "string.gmatch",
        "string.gmatch(s: string, pattern: string): fun()",
    ),
    (
        "string.match",
        "string.match(s: string, pattern: string, init?: integer): string",
    ),
    ("string.rep", "string.rep(s: string, n: integer): string"),
    ("string.lower", "string.lower(s: string): string"),
    ("string.upper", "string.upper(s: string): string"),
    ("string.len", "string.len(s: string): integer"),
    (
        "string.byte",
        "string.byte(s: string, i?: integer, j?: integer): integer",
    ),
    ("string.char", "string.char(...): string"),
    ("string.reverse", "string.reverse(s: string): string"),
    (
        "table.insert",
        "table.insert(t: table, [pos: integer,] value: any)",
    ),
    ("table.remove", "table.remove(t: table, pos?: integer): any"),
    (
        "table.concat",
        "table.concat(t: table, sep?: string, i?: integer, j?: integer): string",
    ),
    ("table.sort", "table.sort(t: table, comp?: function)"),
    ("math.floor", "math.floor(x: number): integer"),
    ("math.ceil", "math.ceil(x: number): integer"),
    ("math.abs", "math.abs(x: number): number"),
    ("math.max", "math.max(x: number, ...): number"),
    ("math.min", "math.min(x: number, ...): number"),
    ("math.sqrt", "math.sqrt(x: number): number"),
    (
        "math.random",
        "math.random(m?: integer, n?: integer): number",
    ),
    ("math.fmod", "math.fmod(x: number, y: number): number"),
    ("bit.band", "bit.band(x: integer, ...): integer"),
    ("bit.bor", "bit.bor(x: integer, ...): integer"),
    ("bit.bxor", "bit.bxor(x: integer, ...): integer"),
    ("bit.bnot", "bit.bnot(x: integer): integer"),
    ("bit.lshift", "bit.lshift(x: integer, n: integer): integer"),
    ("bit.rshift", "bit.rshift(x: integer, n: integer): integer"),
    ("os.remove", "os.remove(filename: string): boolean, string"),
    (
        "io.open",
        "io.open(filename: string, mode?: string): file|nil, string",
    ),
];

pub fn signature(path: &str) -> Option<&'static str> {
    SIGNATURES.iter().find(|s| s.0 == path).map(|s| s.1)
}

/// Return type of well-known standard functions (as a LuaLS type string).
pub fn return_type(path: &str) -> Option<&'static str> {
    Some(match path {
        "tostring" | "type" => "string",
        "tonumber" => "number",
        p if p.starts_with("string.")
            && !matches!(
                p,
                "string.gmatch" | "string.find" | "string.byte" | "string.len" | "string.gsub"
            ) =>
        {
            "string"
        }
        "string.len" | "string.byte" | "string.find" => "integer",
        p if p.starts_with("math.") && p != "math.pi" && p != "math.huge" => "number",
        p if p.starts_with("bit.") && p != "bit.tohex" => "integer",
        "bit.tohex" => "string",
        "table.concat" => "string",
        _ => return None,
    })
}
