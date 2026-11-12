#!/usr/bin/env python3
"""Extract the KeeperFX level-script command and constant tables to JSON.

Regenerate (from the repository root; KFX_SOURCE is a checkout of the KeeperFX C repository):

    python3 -I tools/extract_script_tables.py "$KFX_SOURCE" data/script_tables.json

Reads the C sources under <c_source_root> (read only) and writes a JSON
document with the script command tables, the TbScriptCommands enum and the
named constant tables. Standard library only. Exits non-zero with a message
on stderr if an expected table is missing or parses to zero rows.
"""

import ast
import json
import re
import sys
from pathlib import Path

SOURCE_FILES = {
    "commands": "src/lvl_script_commands.c",
    "enum": "src/lvl_script_lib.h",
    "map_locations": "src/map_locations.c",
    "campaigns": "src/config_campaigns.c",
}

IDENT_RE = re.compile(r"^[A-Za-z_]\w*$")
INT_RE = re.compile(r"^-?\d+$")
HEX_RE = re.compile(r"^0[xX][0-9a-fA-F]+$")

NAMED_TABLE_RE = re.compile(
    r"const\s+struct\s+NamedCommand\s+(\w+)\s*\[\s*\]\s*=\s*\{"
)
COMMAND_TABLE_RE = re.compile(
    r"const\s+struct\s+CommandDesc\s+(\w+)\s*\[\s*\]\s*=\s*\{"
)
ENUM_RE = re.compile(r"enum\s+TbScriptCommands\s*\{")
COMMAND_DESC_DECL_RE = re.compile(r"const\s+struct\s+CommandDesc\s+command_desc\s*\[")

# Tables required by the task; their absence is a hard failure.
REQUIRED_NAMED_TABLES = {
    "commands": [],  # any number of named tables, but at least one
    "map_locations": ["head_for_desc"],
    "campaigns": ["cmpgn_human_player_options"],
}


class ExtractError(Exception):
    pass


def scan(text):
    """Return (clean, masked) strings with the same length as text.

    clean:  comments replaced by spaces (string literals kept intact).
    masked: comments replaced by spaces and string literal contents replaced
            by '_', so braces, commas and parentheses inside strings are
            invisible to the structural parser.
    """
    clean = list(text)
    masked = list(text)
    n = len(text)
    i = 0
    while i < n:
        c = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if c == "/" and nxt == "/":
            j = text.find("\n", i)
            if j == -1:
                j = n
            for k in range(i, j):
                clean[k] = masked[k] = " "
            i = j
        elif c == "/" and nxt == "*":
            j = text.find("*/", i + 2)
            if j == -1:
                raise ExtractError("unterminated block comment at offset %d" % i)
            j += 2
            for k in range(i, j):
                if text[k] != "\n":
                    clean[k] = masked[k] = " "
            i = j
        elif c in ("'", '"'):
            quote = c
            j = i + 1
            while j < n and text[j] != quote:
                if text[j] == "\n":
                    raise ExtractError("unterminated literal at offset %d" % i)
                if text[j] == "\\":
                    j += 2
                    continue
                j += 1
            if j >= n:
                raise ExtractError("unterminated literal at offset %d" % i)
            for k in range(i + 1, j):
                masked[k] = "_"
            i = j + 1
        else:
            i += 1
    return "".join(clean), "".join(masked)


def match_close(masked, open_idx, open_ch, close_ch):
    depth = 0
    for k in range(open_idx, len(masked)):
        ch = masked[k]
        if ch == open_ch:
            depth += 1
        elif ch == close_ch:
            depth -= 1
            if depth == 0:
                return k
    raise ExtractError("no matching '%s' for '%s' at offset %d" % (close_ch, open_ch, open_idx))


def split_fields(masked, start, end):
    """Split [start, end) on top-level commas. Returns non-empty (s, e) spans."""
    spans = []
    depth = 0
    seg = start
    for k in range(start, end):
        ch = masked[k]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif ch == "," and depth == 0:
            spans.append((seg, k))
            seg = k + 1
    spans.append((seg, end))
    return [(s, e) for s, e in spans if masked[s:e].strip()]


def table_rows(clean, masked, open_idx, table_name):
    """Return the inner (start, end) spans of each top-level {...} row."""
    close_idx = match_close(masked, open_idx, "{", "}")
    body = clean[open_idx:close_idx]
    if re.search(r"^\s*#", body, re.M):
        raise ExtractError("%s: preprocessor directive inside table body" % table_name)
    rows = []
    k = open_idx + 1
    while k < close_idx:
        ch = masked[k]
        if ch == "{":
            rc = match_close(masked, k, "{", "}")
            rows.append((k + 1, rc))
            k = rc + 1
        else:
            if not ch.isspace() and ch != ",":
                raise ExtractError(
                    "%s: unexpected text outside row: %r" % (table_name, clean[k:k + 40])
                )
            k += 1
    return rows


def c_string(clean, s, e, where):
    text = clean[s:e].strip()
    if len(text) < 2 or text[0] != '"' or text[-1] != '"':
        raise ExtractError("%s: expected string literal, got %r" % (where, text))
    try:
        return ast.literal_eval(text)
    except (ValueError, SyntaxError) as exc:
        raise ExtractError("%s: cannot decode string %r (%s)" % (where, text, exc))


def c_expr(clean, s, e):
    return re.sub(r"\s+", " ", clean[s:e]).strip()


def is_null(text):
    return text == "NULL"


def parse_named_table(table_name, clean, masked, open_idx):
    out = []
    terminated = False
    for rs, re_ in table_rows(clean, masked, open_idx, table_name):
        spans = split_fields(masked, rs, re_)
        if len(spans) != 2:
            raise ExtractError("%s: row with %d fields: %r" % (table_name, len(spans), clean[rs:re_]))
        key_s, key_e = spans[0]
        val_s, val_e = spans[1]
        if is_null(clean[key_s:key_e].strip()):
            terminated = True
            break
        name = c_string(clean, key_s, key_e, table_name)
        value = c_expr(clean, val_s, val_e)
        if not value:
            raise ExtractError("%s: empty value for %r" % (table_name, name))
        out.append({"name": name, "value": value})
    if not terminated:
        raise ExtractError("%s: no {NULL, ...} terminator row" % table_name)
    if not out:
        raise ExtractError("%s: parsed to zero rows" % table_name)
    return out


def parse_command_table(table_name, clean, masked, open_idx):
    out = []
    terminated = False
    for rs, re_ in table_rows(clean, masked, open_idx, table_name):
        spans = split_fields(masked, rs, re_)
        if len(spans) != 5:
            raise ExtractError("%s: row with %d fields: %r" % (table_name, len(spans), clean[rs:re_]))
        texts = [clean[s:e].strip() for s, e in spans]
        if is_null(texts[0]):
            terminated = True
            break
        name = c_string(clean, *spans[0], table_name)
        args = c_string(clean, *spans[1], table_name)
        if len(args) != 8:
            # Kept exactly as written in the source; reported so it can be fixed upstream.
            print(
                "warning: %s: args %r for %s has length %d (expected 8)"
                % (table_name, args, name, len(args)),
                file=sys.stderr,
            )
        cmd = texts[2]
        if not IDENT_RE.match(cmd):
            raise ExtractError("%s: bad Cmd_ name %r for %s" % (table_name, cmd, name))
        funcs = []
        for idx in (3, 4):
            t = texts[idx]
            if is_null(t):
                funcs.append(None)
                continue
            t = re.sub(r"^&\s*", "", t)
            if not IDENT_RE.match(t):
                raise ExtractError("%s: unexpected function field %r for %s" % (table_name, texts[idx], name))
            funcs.append(t)
        out.append({
            "name": name,
            "args": args,
            "cmd": cmd,
            "check": funcs[0],
            "process": funcs[1],
        })
    if not terminated:
        raise ExtractError("%s: no {NULL, ...} terminator row" % table_name)
    if not out:
        raise ExtractError("%s: parsed to zero rows" % table_name)
    return out


def parse_enum(clean, masked):
    m = ENUM_RE.search(masked)
    if not m:
        raise ExtractError("enum TbScriptCommands not found")
    open_idx = m.end() - 1
    close_idx = match_close(masked, open_idx, "{", "}")
    known = {}
    out = []
    value = -1
    for s, e in split_fields(masked, open_idx + 1, close_idx):
        text = clean[s:e].strip()
        if "=" in text:
            name, expr = (x.strip() for x in text.split("=", 1))
            if INT_RE.match(expr):
                value = int(expr)
            elif HEX_RE.match(expr):
                value = int(expr, 16)
            elif expr in known:
                value = known[expr]
            else:
                raise ExtractError("TbScriptCommands: cannot evaluate %r for %s" % (expr, name))
        else:
            name = text
            value += 1
        if not IDENT_RE.match(name):
            raise ExtractError("TbScriptCommands: bad enumerator %r" % name)
        if name in known:
            raise ExtractError("TbScriptCommands: duplicate enumerator %r" % name)
        known[name] = value
        out.append({"name": name, "value": value})
    if not out:
        raise ExtractError("TbScriptCommands parsed to zero rows")
    return out


def find_command_table(name, clean, masked):
    for m in COMMAND_TABLE_RE.finditer(masked):
        if m.group(1) == name:
            return m.end() - 1
    raise ExtractError("command table %s not found" % name)


def find_named_tables(clean, masked):
    found = []
    for m in NAMED_TABLE_RE.finditer(masked):
        found.append((m.group(1), m.end() - 1))
    return found


def arg_letters_doc(raw, masked_decl_offset):
    """Return the block comment immediately above command_desc[], verbatim."""
    end_of_comment = raw.rfind("*/", 0, masked_decl_offset)
    if end_of_comment == -1:
        raise ExtractError("no comment block found above command_desc")
    start = raw.rfind("/**", 0, end_of_comment)
    if start == -1:
        raise ExtractError("no /** comment block found above command_desc")
    end = end_of_comment + 2
    if raw[end:masked_decl_offset].strip():
        raise ExtractError("code between the argument comment and command_desc")
    return raw[start:end]


def load(root, rel):
    path = root / rel
    if not path.is_file():
        raise ExtractError("source file not found: %s" % path)
    raw = path.read_text(encoding="utf-8", errors="replace")
    clean, masked = scan(raw)
    return raw, clean, masked


def main(argv):
    if len(argv) != 3:
        print("usage: %s <c_source_root> <output_json>" % argv[0], file=sys.stderr)
        return 2
    root = Path(argv[1])
    out_path = Path(argv[2])

    raw_cmd, clean_cmd, masked_cmd = load(root, SOURCE_FILES["commands"])
    raw_enum, clean_enum, masked_enum = load(root, SOURCE_FILES["enum"])
    raw_map, clean_map, masked_map = load(root, SOURCE_FILES["map_locations"])
    raw_camp, clean_camp, masked_camp = load(root, SOURCE_FILES["campaigns"])

    # Command tables
    decl = COMMAND_DESC_DECL_RE.search(masked_cmd)
    if not decl:
        raise ExtractError("command_desc declaration not found")
    doc = arg_letters_doc(raw_cmd, decl.start())

    commands = parse_command_table(
        "command_desc",
        clean_cmd, masked_cmd,
        find_command_table("command_desc", clean_cmd, masked_cmd),
    )
    dk1_commands = parse_command_table(
        "dk1_command_desc",
        clean_cmd, masked_cmd,
        find_command_table("dk1_command_desc", clean_cmd, masked_cmd),
    )
    subfunctions = parse_command_table(
        "subfunction_desc",
        clean_cmd, masked_cmd,
        find_command_table("subfunction_desc", clean_cmd, masked_cmd),
    )

    # Named tables
    named = {}
    cmd_named = find_named_tables(clean_cmd, masked_cmd)
    if not cmd_named:
        raise ExtractError("no 'const struct NamedCommand' tables found in %s" % SOURCE_FILES["commands"])
    # Every NamedCommand table in lvl_script_commands.c, plus only the named
    # tables listed for the other two files.
    sources = [
        ("commands", clean_cmd, masked_cmd, cmd_named),
        ("map_locations", clean_map, masked_map,
         [t for t in find_named_tables(clean_map, masked_map) if t[0] in REQUIRED_NAMED_TABLES["map_locations"]]),
        ("campaigns", clean_camp, masked_camp,
         [t for t in find_named_tables(clean_camp, masked_camp) if t[0] in REQUIRED_NAMED_TABLES["campaigns"]]),
    ]
    for key, clean, masked, tables in sources:
        available = {name for name, _ in tables}
        for required in REQUIRED_NAMED_TABLES[key]:
            if required not in available:
                raise ExtractError("named table %s not found in %s" % (required, SOURCE_FILES[key]))
        for name, open_idx in tables:
            if name in named:
                raise ExtractError("duplicate named table name %s" % name)
            named[name] = parse_named_table(name, clean, masked, open_idx)

    # Enum
    command_ids = parse_enum(clean_enum, masked_enum)

    # Consistency: every command's Cmd_ name must be an enumerator.
    enum_names = {entry["name"] for entry in command_ids}
    for table in (commands, dk1_commands, subfunctions):
        for row in table:
            if row["cmd"] not in enum_names:
                raise ExtractError("%s is not in TbScriptCommands" % row["cmd"])

    result = {
        "source": ", ".join(SOURCE_FILES[k] for k in ("commands", "enum", "map_locations", "campaigns")),
        "arg_letters_doc": doc,
        "commands": commands,
        "dk1_commands": dk1_commands,
        "subfunctions": subfunctions,
        "named_tables": named,
        "command_ids": command_ids,
    }

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(result, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(
        "wrote %s: %d commands, %d dk1_commands, %d subfunctions, %d named tables, %d enum values"
        % (out_path, len(commands), len(dk1_commands), len(subfunctions), len(named), len(command_ids))
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except ExtractError as exc:
        print("error: %s" % exc, file=sys.stderr)
        sys.exit(1)
