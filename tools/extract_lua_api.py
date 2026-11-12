#!/usr/bin/env python3
"""Extract the Lua API that the KeeperFX C code registers (src/lua_api*.c) into data/lua_c_api.json.

Regenerate (from the repository root):

    python3 -I tools/extract_lua_api.py "$KFX_SOURCE" data/lua_c_api.json

The first argument is a checkout of the KeeperFX C repository (its `src/` folder is read); it
defaults to the environment variable KFX_SOURCE. The output defaults to data/lua_c_api.json of this
repository. The output records the source folder relative to the checkout (`"source_dir": "src"`).

What is extracted
  * registration tables (`static const luaL_Reg NAME[] = { {"Lua name", c_func}, ... };`), including
    entries that are commented out (`//{"..."}` or inside `/* */`), flagged `commented`;
  * direct `lua_register(L, "Name", func)` calls (sound API);
  * class members: `strcmp(key, "field") == 0` tests inside `<class>_get_field` / `<class>_set_field`
    (the field names scripts can read/write), with the thing-family guess for lua_api_things.c;
  * the engine hooks looked up with `lua_getglobal(Lvl_script, "OnXxx")` in lua_triggers.c.
Only the Python standard library is used; the C source is read, never executed.
"""
import json
import os
import re
import sys


# registration array -> (kind, class)
ARRAYS = {
    "global_methods": ("global", None),
    "lens_methods": ("global", None),
    "player_methods": ("method", "Player"),
    "thing_methods": ("method", "Thing"),
    "room_methods": ("method", "Room"),
    "slab_methods": ("method", "Slab"),
    "camera_methods": ("method", "Camera"),
}
# *_get_field / *_set_field function prefix -> class
FIELD_FUNCS = {
    "player": "Player",
    "thing": "Thing",
    "room": "Room",
    "slab": "Slab",
    "camera": "Camera",
    "map": "Map",
}


def read(path):
    with open(path, "r", encoding="latin-1") as f:
        return f.read()


def strip_comments_and_strings(text):
    """Blank out comments and string/char literals, keeping offsets and newlines."""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("".join(ch if ch == "\n" else " " for ch in text[i:j]))
            i = j
        elif c in "\"'":
            j = i + 1
            while j < n and text[j] != c:
                j += 2 if text[j] == "\\" else 1
            j = min(j + 1, n)
            out.append(c + " " * (j - i - 2) + (c if j - i >= 2 else ""))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


ENTRY = re.compile(r'\{\s*"([A-Za-z_][A-Za-z0-9_]*)"\s*,\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}')


def parse_arrays(path, text):
    """Yield (array_name, [entries]) for every luaL_Reg table of the file."""
    res = []
    for m in re.finditer(r"static\s+const\s+(?:struct\s+)?luaL_Reg\s+(\w+)\s*\[\s*\]\s*=\s*\{", text):
        name = m.group(1)
        start = m.end()
        end = text.find("};", start)
        body = text[start:end]
        entries = []
        in_block = False
        offset = start
        for raw in body.split("\n"):
            line_no = text.count("\n", 0, offset) + 1
            line = raw
            commented = False
            # block comment tracking (only whole-entry granularity is needed)
            work = line
            if in_block:
                commented = True
                if "*/" in work:
                    in_block = False
                    work = work[work.index("*/") + 2:]
            stripped = work.lstrip()
            if stripped.startswith("//"):
                commented = True
                work = stripped[2:]
            elif "/*" in work and "*/" not in work[work.index("/*"):]:
                before = work[: work.index("/*")]
                if ENTRY.search(before):
                    work = before
                else:
                    commented = True
                    work = work[work.index("/*") + 2:]
                    in_block = True
            for e in ENTRY.finditer(work):
                entries.append({"name": e.group(1), "func": e.group(2), "line": line_no, "commented": commented})
            offset += len(raw) + 1
        res.append((name, entries))
    return res


def function_body(text, start):
    """Text of the brace-balanced body that begins at the first `{` at or after start."""
    i = text.find("{", start)
    if i < 0:
        return "", start
    depth, j = 0, i
    while j < len(text):
        if text[j] == "{":
            depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0:
                return text[i : j + 1], i
        j += 1
    return text[i:], i


STRCMP = re.compile(r'strcmp\(\s*key\s*,\s*"([A-Za-z_][A-Za-z0-9_]*)"\s*\)\s*==\s*0')
FAMILY = re.compile(r"thing_is_(\w+)\s*\(|class_id\s*==\s*TCls_(\w+)|TCls_(\w+)")


def parse_fields(path, raw, clean):
    out = {}
    for m in re.finditer(r"static\s+int\s+(\w+?)_(get|set)_field\s*\(\s*lua_State\s*\*\s*L\s*\)\s*\{", clean):
        prefix, which = m.group(1), m.group(2)
        cls = FIELD_FUNCS.get(prefix)
        if cls is None:
            continue
        body, body_start = function_body(clean, m.start())
        fam = "shared"
        entry = out.setdefault(cls, {"get": [], "set": [], "dynamic_player_variables": False})
        # walk tokens in order so the family guess follows the code layout
        events = [(mm.start(), "fam", mm) for mm in FAMILY.finditer(body)] + [(mm.start(), "key", mm) for mm in STRCMP.finditer(body)]
        for _, kind, mm in sorted(events, key=lambda e: e[0]):
            if kind == "fam":
                fam = (mm.group(1) or mm.group(2) or mm.group(3) or fam).lower()
            else:
                line_no = clean.count("\n", 0, body_start + mm.start()) + 1
                rec = {"name": mm.group(1), "line": line_no}
                if cls == "Thing":
                    rec["family"] = fam
                lst = entry[which]
                if not any(r["name"] == rec["name"] and r.get("family") == rec.get("family") for r in lst):
                    lst.append(rec)
        if cls == "Player" and "parse_get_varib" in body:
            entry["dynamic_player_variables"] = True
    return out


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("KFX_SOURCE")
    if not root:
        sys.exit("usage: extract_lua_api.py KEEPERFX_SOURCE [OUTPUT_JSON]\n"
                 "KEEPERFX_SOURCE: a checkout of the KeeperFX C repository (or set KFX_SOURCE)")
    src = os.path.join(root, "src")
    if not os.path.isfile(os.path.join(src, "lua_api.c")):
        sys.exit("%s: no src/lua_api.c; expected a checkout of the KeeperFX C repository" % root)
    here = os.path.dirname(os.path.abspath(__file__))
    outp = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "data", "lua_c_api.json")
    result = {"source_dir": "src", "globals": [], "methods": {}, "fields": {}, "raw_fields": {}, "hooks": [], "notes": []}
    files = sorted(f for f in os.listdir(src) if f.startswith("lua_api") and f.endswith(".c"))
    for f in files:
        path = os.path.join(src, f)
        raw = read(path)
        clean = strip_comments_and_strings(raw)
        for arr, entries in parse_arrays(path, raw):
            if arr not in ARRAYS:
                continue
            kind, cls = ARRAYS[arr]
            for e in entries:
                e = dict(e, file=f, array=arr)
                if kind == "global":
                    result["globals"].append(e)
                else:
                    result["methods"].setdefault(cls, []).append(e)
        # lua_register with string literals: search the raw text (strings are blanked in `clean`)
        for m in re.finditer(r'^\s*lua_register\(\s*L\s*,\s*"(\w+)"\s*,\s*(\w+)\s*\)', raw, re.M):
            line_no = raw.count("\n", 0, m.start()) + 1
            result["globals"].append({"name": m.group(1), "func": m.group(2), "line": line_no, "commented": False, "file": f, "array": "lua_register"})
        for cls, d in parse_fields(path, raw, strip_keep_strings(raw)).items():
            cur = result["fields"].setdefault(cls, {"get": [], "set": [], "dynamic_player_variables": False})
            cur["get"] += d["get"]
            cur["set"] += d["set"]
            cur["dynamic_player_variables"] = cur["dynamic_player_variables"] or d["dynamic_player_variables"]
    # raw table fields written by the push helpers of lua_params.c (`lua_pushSlab` sets slb_x, ...)
    params = os.path.join(src, "lua_params.c")
    if os.path.exists(params):
        raw = read(params)
        clean = strip_keep_strings(raw)
        for m in re.finditer(r"\n(?:void|int)\s+lua_push(\w+)\s*\([^)]*\)\s*\{", clean):
            body, _ = function_body(clean, m.start())
            names = sorted(set(re.findall(r'lua_setfield\(\s*L\s*,\s*-?\d+\s*,\s*"(\w+)"\s*\)', body)))
            if names:
                result["raw_fields"][m.group(1)] = names
    trig = os.path.join(src, "lua_triggers.c")
    if os.path.exists(trig):
        t = read(trig)
        for m in re.finditer(r'lua_getglobal\(\s*Lvl_script\s*,\s*"(\w+)"\s*\)', t):
            result["hooks"].append({"name": m.group(1), "line": t.count("\n", 0, m.start()) + 1})
        m = re.search(r'sprintf\(call_name,\s*"(OnModGameStart_%s)"', t)
        if m:
            result["hooks"].append({"name": "OnModGameStart_<modname>", "line": t.count("\n", 0, m.start()) + 1})
    result["notes"].append("Player variables (MONEY, FLAG0, ...) are resolved dynamically by parse_get_varib and are not listed here.")
    result["notes"].append("Fields of lua_api_things.c are shared by all thing classes; `family` is a guess from the surrounding thing_is_* test.")
    with open(outp, "w") as f:
        json.dump(result, f, indent=1, sort_keys=True)
        f.write("\n")
    print("wrote %s: %d globals (%d commented), %s" % (
        outp,
        len(result["globals"]),
        sum(1 for g in result["globals"] if g["commented"]),
        ", ".join("%s: %d methods/%d get/%d set" % (c, len(result["methods"].get(c, [])), len(result["fields"].get(c, {}).get("get", [])), len(result["fields"].get(c, {}).get("set", []))) for c in sorted(set(result["methods"]) | set(result["fields"]))),
    ))


def strip_keep_strings(text):
    """Blank comments only (string literals are needed to read `strcmp(key, "name")`)."""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("".join(ch if ch == "\n" else " " for ch in text[i:j]))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            j = min(j + 1, n)
            out.append(text[i:j])
            i = j
        elif c == "'":
            j = i + 1
            while j < n and text[j] != "'":
                j += 2 if text[j] == "\\" else 1
            j = min(j + 1, n)
            out.append(" " * (j - i))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


if __name__ == "__main__":
    main()
