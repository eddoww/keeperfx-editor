#!/usr/bin/env python3
"""Collect the comments the KeeperFX C source has for each level-script command.

Regenerate (from the repository root; KFX_SOURCE is a checkout of the KeeperFX C repository):

    python3 -I tools/extract_script_c_comments.py "$KFX_SOURCE/src" data/script_c_comments.json

For every row of `command_desc[]` / `dk1_command_desc[]` in `lvl_script_commands.c` it gathers

  * the `//` comment at the end of the table row,
  * the comment block directly above the row's `*_check` and `*_process` functions,
  * `//` comments at the end of `case Cmd_NAME:` labels.

Nothing else is taken. Source file and line are recorded for every comment. The C source is GPL
code; only comment text is copied. Standard library only; prints a coverage line.
"""
import json
import re
import sys
from pathlib import Path

ROW = re.compile(r'^\s*\{"([A-Z0-9_]+)"\s*,\s*"[^"]*"\s*,\s*(Cmd_[A-Za-z0-9_]+)\s*,\s*(&?\w+)\s*,\s*(&?\w+)\s*\}\s*,?\s*(//.*)?$')
CASE = re.compile(r'^\s*case\s+(Cmd_[A-Za-z0-9_]+)\s*:\s*(//.*)?$')


def clean(s):
    s = s.strip()
    if s.startswith("//"):
        s = s[2:]
    return s.strip(" \t*/")


def comment_above(lines, idx):
    """Comment lines directly above line idx (0-based), nearest last."""
    out = []
    i = idx - 1
    while i >= 0:
        t = lines[i].strip()
        if t.startswith("//"):
            out.append(clean(t))
        elif t.endswith("*/"):
            j = i
            blk = []
            while j >= 0:
                blk.append(clean(lines[j]))
                if lines[j].strip().startswith("/*"):
                    break
                j -= 1
            out.extend(blk)
            i = j
        else:
            break
        i -= 1
    out = [x for x in reversed(out) if x]
    return out


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: extract_script_c_comments.py <keeperfx_source>/src <output.json>")
    src, outp = Path(sys.argv[1]), Path(sys.argv[2])
    path = src / "lvl_script_commands.c"
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    funcs = {}
    for i, l in enumerate(lines):
        m = re.match(r"^(?:static\s+)?[\w\s\*]+?\b(\w+_(?:check|process))\s*\(", l)
        if m and not l.rstrip().endswith(";"):
            funcs[m.group(1)] = i
    rows = []
    for i, l in enumerate(lines):
        m = ROW.match(l)
        if m:
            rows.append((i, m))
    cmd_of_enum = {}
    out = {}
    for i, m in rows:
        name, enum, chk, prc, trail = m.groups()
        cmd_of_enum.setdefault(enum, name)
        notes = []
        if trail and clean(trail):
            notes.append({"text": clean(trail), "line": i + 1, "where": "table row"})
        for f in (chk, prc):
            f = f.lstrip("&")
            if f in funcs:
                c = comment_above(lines, funcs[f])
                if c:
                    notes.append({"text": " ".join(c), "line": funcs[f] + 1, "where": f})
        if notes:
            out.setdefault(name, []).extend(n for n in notes if n not in out.get(name, []))
    for i, l in enumerate(lines):
        m = CASE.match(l)
        if m and m.group(2) and clean(m.group(2)) and m.group(1) in cmd_of_enum:
            out.setdefault(cmd_of_enum[m.group(1)], []).append({"text": clean(m.group(2)), "line": i + 1, "where": "case label"})
    doc = {"source": "src/lvl_script_commands.c (comment text only)", "commands": {k: out[k] for k in sorted(out)}}
    outp.write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"{len(rows)} table rows, {len(out)} commands with a C comment")


if __name__ == "__main__":
    main()
