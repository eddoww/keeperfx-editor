#!/usr/bin/env python3
"""Regenerate every file this repository derives from KeeperFX, in one command.

    python3 -I tools/sync_keeperfx.py KEEPERFX_SOURCE WIKI_DIR [--check] [--report FILE]

KEEPERFX_SOURCE is a checkout of https://github.com/dkfans/keeperfx and WIKI_DIR the folder of a
checkout of https://github.com/dkfans/keeperfx.wiki that holds the Markdown pages. Both are only
read. The tool

  1. copies the parts of both trees the extractors read (regular files only: symbolic links and
     the .git folders are skipped) into a work folder, including a stand-in install whose
     `fxdata` is the source's `config/fxdata` and whose `creatrs` is `config/creatrs`, so no game
     install is needed;
  2. runs every extractor of tools/ on that copy, in dependency order, and builds the Lua stub
     index (data/lua_api.json) with `cargo run -p kfx-script --example lua_index`;
  3. reconciles the hand-written data/script_arg_overlay.json with the new command table:
     commands new upstream get a generated entry marked `"auto": true`, entries of commands whose
     argument string changed are adjusted and marked `"auto": true`, entries of removed commands
     move to the `"removed"` section; untouched entries keep their text and order;
  4. compares the results with the files of this repository and, unless --check is given, copies
     the changed ones over and records the upstream revisions in data/UPSTREAM.json. That file
     is only written when a generated file changed (or when it does not exist yet), so an
     upstream commit that changes nothing leaves the tree untouched.

--check writes nothing to the repository and exits with 1 when something would change.
--report FILE writes a Markdown change report (the pull request text of the sync workflow);
--summary FILE a JSON summary for scripts (changed files, whether the change is additive only).

Exit codes: 0 done (or nothing to do), 1 --check found changes, 2 error.

Everything read from the two trees is untrusted data: the extractors parse it as text and never
evaluate, import or run it; nothing from the trees is executed. The revisions are read with
`git log` (or given with --keeperfx-rev, --keeperfx-date and --wiki-rev). Standard library only.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TOOLS = REPO / "tools"

KEEPERFX_URL = "https://github.com/dkfans/keeperfx"
WIKI_URL = "https://github.com/dkfans/keeperfx.wiki"

TABLES = "data/script_tables.json"
DOCS = "data/script_docs.json"
C_COMMENTS = "data/script_c_comments.json"
CONFIG_SCHEMA = "data/config_schema.json"
LUA_C_API = "data/lua_c_api.json"
LUA_API = "data/lua_api.json"
OVERLAY = "data/script_arg_overlay.json"
GEN_CREATURES = "crates/kfx-gfx/src/gen_creature_list.rs"
GEN_SPRITES = "crates/kfx-gfx/src/gen_sprite_names.rs"
UPSTREAM = "data/UPSTREAM.json"

# Every generated file, in the order they are produced.
OUTPUTS = [TABLES, DOCS, C_COMMENTS, CONFIG_SCHEMA, LUA_C_API, LUA_API, OVERLAY, GEN_CREATURES,
           GEN_SPRITES]

HASH_RE = re.compile(r"^[0-9a-f]{40}([0-9a-f]{24})?$")
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(Z|[+-]\d{2}:\d{2})$")
REF_RE = re.compile(r"^refs/[A-Za-z0-9._/-]+$")
# Files larger than this are not copied from the upstream trees (none of the inputs is near it).
MAX_FILE_BYTES = 32 * 1024 * 1024


class SyncError(Exception):
    pass


# --------------------------------------------------------------------------------------------
# Staging: copy the inputs out of the untrusted trees
# --------------------------------------------------------------------------------------------

def copy_tree(src, dst, suffixes=None, recursive=True):
    """Copy the regular files of src into dst. Symbolic links (to files or folders), .git
    folders, special files and very large files are skipped. Returns the number of files."""
    src, dst = Path(src), Path(dst)
    if src.is_symlink() or not src.is_dir():
        raise SyncError("%s: not a folder" % src)
    count = 0
    for dirpath, dirnames, filenames in os.walk(src, followlinks=False):
        here = Path(dirpath)
        dirnames[:] = sorted(d for d in dirnames
                             if d != ".git" and not (here / d).is_symlink()) if recursive else []
        for name in sorted(filenames):
            p = here / name
            if p.is_symlink() or not p.is_file():
                continue
            if suffixes is not None and p.suffix.lower() not in suffixes:
                continue
            if p.stat().st_size > MAX_FILE_BYTES:
                print("warning: skipping large file %s" % p.relative_to(src), file=sys.stderr)
                continue
            out = dst / p.relative_to(src)
            out.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(p, out, follow_symlinks=False)
            count += 1
    return count


def inside(root, rel):
    """root/rel, refused when a symbolic link on the way leads out of root."""
    p = root / rel
    if not p.resolve().is_relative_to(root.resolve()):
        raise SyncError("%s leads outside %s" % (p, root))
    return p


def stage(source, wiki, work):
    """Copy what the extractors read. Returns (source_root, install_root, wiki_root)."""
    src_root, install, wiki_root = work / "keeperfx", work / "install", work / "wiki"
    if not (source / "src" / "lvl_script_commands.c").is_file():
        raise SyncError("%s: not a KeeperFX source tree (no src/lvl_script_commands.c)" % source)
    if copy_tree(inside(source, "src"), src_root / "src", {".c", ".h", ".cpp", ".hpp"}) == 0:
        raise SyncError("%s/src: no C files" % source)
    for sub in ("fxdata", "creatrs"):
        if copy_tree(inside(source, "config/" + sub), install / sub) == 0:
            raise SyncError("%s/config/%s: no files" % (source, sub))
    if copy_tree(wiki, wiki_root, {".md"}, recursive=False) == 0:
        raise SyncError("%s: no Markdown pages" % wiki)
    return src_root, install, wiki_root


# --------------------------------------------------------------------------------------------
# Revisions
# --------------------------------------------------------------------------------------------

def git_head(path):
    """(commit, ISO date) of HEAD of the checkout at path, or (None, None)."""
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GIT_PAGER="cat")
    try:
        r = subprocess.run(
            ["git", "--no-pager", "-c", "log.showSignature=false", "-C", str(path), "log", "-1",
             "--format=%H%n%cI"],
            capture_output=True, text=True, env=env, timeout=60, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return None, None
    lines = r.stdout.split()
    if r.returncode != 0 or len(lines) != 2:
        return None, None
    return lines[0], lines[1]


def git_head_from_files(path):
    """Commit of HEAD read from the .git folder (no date), or None."""
    git_dir = Path(path) / ".git"
    try:
        head = (git_dir / "HEAD").read_text(encoding="utf-8").strip()
        if not head.startswith("ref: "):
            return head if HASH_RE.match(head) else None
        ref = head[5:].strip()
        if not REF_RE.match(ref) or ".." in ref.split("/"):
            return None
        f = git_dir / ref
        if f.is_file() and not f.is_symlink():
            v = f.read_text(encoding="utf-8").strip()
            return v if HASH_RE.match(v) else None
        packed = git_dir / "packed-refs"
        if packed.is_file():
            for line in packed.read_text(encoding="utf-8").splitlines():
                parts = line.split()
                if len(parts) == 2 and parts[1] == ref and HASH_RE.match(parts[0]):
                    return parts[0]
    except (OSError, UnicodeDecodeError):
        return None
    return None


def revisions(args):
    kfx_rev, kfx_date = args.keeperfx_rev, args.keeperfx_date
    if not kfx_rev or not kfx_date:
        r, d = git_head(args.source)
        kfx_rev, kfx_date = kfx_rev or r, kfx_date or d
    wiki_rev = args.wiki_rev or git_head(args.wiki)[0] or git_head_from_files(args.wiki)
    if not kfx_rev or not HASH_RE.match(kfx_rev):
        raise SyncError("cannot read the KeeperFX commit; pass --keeperfx-rev")
    if not kfx_date or not DATE_RE.match(kfx_date):
        raise SyncError("cannot read the KeeperFX commit date; pass --keeperfx-date (ISO 8601)")
    if not wiki_rev or not HASH_RE.match(wiki_rev):
        raise SyncError("cannot read the wiki commit; pass --wiki-rev")
    return {
        "keeperfx": {"repository": KEEPERFX_URL, "commit": kfx_rev, "date": kfx_date},
        "wiki": {"repository": WIKI_URL, "commit": wiki_rev},
    }


def upstream_json(revs):
    doc = {
        "_note": "Written by tools/sync_keeperfx.py: the upstream revisions the generated data "
                 "(data/*.json except the overlay, crates/kfx-gfx/src/gen_*.rs) was made from.",
        "keeperfx": revs["keeperfx"],
        "wiki": revs["wiki"],
    }
    return json.dumps(doc, indent=1, ensure_ascii=False) + "\n"


# --------------------------------------------------------------------------------------------
# Generation
# --------------------------------------------------------------------------------------------

def run(cmd, what, cwd=None):
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, check=False)
    if r.returncode != 0:
        raise SyncError("%s failed (exit %d):\n%s%s" % (what, r.returncode, r.stdout[-4000:],
                                                       r.stderr[-4000:]))
    return r


def python_tool(name, *args):
    return run([sys.executable, "-I", str(TOOLS / name), *map(str, args)], name)


def generate(src_root, install, wiki_root, wiki_rev, out, jobs, cargo):
    """Run every extractor; outputs land under out/ with their repository paths."""
    for rel in OUTPUTS:
        (out / rel).parent.mkdir(parents=True, exist_ok=True)
    python_tool("extract_script_tables.py", src_root, out / TABLES)
    python_tool("extract_script_docs.py", wiki_root, out / TABLES, out / DOCS, "--commit", wiki_rev)
    python_tool("extract_script_c_comments.py", src_root / "src", out / C_COMMENTS)
    python_tool("extract_config_schema.py", "--src", src_root, "--install", install,
                "--wiki", wiki_root, "--out", (out / CONFIG_SCHEMA).resolve())
    python_tool("extract_lua_api.py", src_root, out / LUA_C_API)
    python_tool("gen_gfx_tables.py", src_root, out / GEN_CREATURES.rsplit("/", 1)[0])
    lua_root = install / "fxdata" / "lua"
    if not lua_root.is_dir():
        raise SyncError("config/fxdata/lua: not found in the KeeperFX source")
    run([cargo, "run", "--quiet", "--locked", "-p", "kfx-script", "--example", "lua_index",
         "-j", str(jobs), "--", str(lua_root), str((out / LUA_API).resolve())],
        "cargo run --example lua_index", cwd=REPO)


# --------------------------------------------------------------------------------------------
# Overlay reconciliation
# --------------------------------------------------------------------------------------------

# ArgKind::from_letter in crates/kfx-script/src/overlay.rs
LETTER_KIND = {"N": "number", "C": "creature", "P": "player", "R": "room", "S": "slab",
               "L": "location", "O": "operator", "B": "bool"}
# Kinds that refine a typed letter (the consistency test in crates/kfx-script/tests/units.rs).
BOOL_REFINEMENTS = {"conceal_mode", "lock_state", "free_state"}


def letter_kind(letter):
    return LETTER_KIND.get(letter.upper(), "text")


def arg_letters(args):
    """CmdDesc::arg_letters: the type letters without padding, `!` and `+`."""
    return [c for c in args[:8] if c not in " !+\0"]


def kind_fits(kind, letter):
    lk = letter_kind(letter)
    return lk in ("text", "number") or kind == lk or (lk == "bool" and kind in BOOL_REFINEMENTS)


def generated_args(letters, keep):
    """Overlay args for letters; keep[i] is an existing "name:kind" string or None."""
    bases = [letter_kind(c) for c in letters]
    out = []
    for i, c in enumerate(letters):
        if keep[i] is not None:
            out.append(keep[i])
            continue
        base = bases[i]
        name = base if bases.count(base) == 1 else "%s%d" % (base, i + 1)
        out.append("%s*:%s" % (name, base))
    return out


def _arg_kind(arg):
    return arg.split(":", 1)[1] if ":" in arg else ""


def reconcile_overlay(overlay, old_tables, new_tables, rev_short):
    """Return (new overlay dict, changes) for the new command table.

    changes = {"added": [...], "changed": [...], "removed": [...], "restored": [...]}."""
    old_args = {r["name"]: r["args"] for r in old_tables.get("commands", [])}
    new_rows = new_tables["commands"]
    new_names = [r["name"] for r in new_rows]
    new_set = set(new_names)
    commands = overlay.get("commands", {})
    removed = dict(overlay.get("removed", {}))
    changes = {"added": [], "changed": [], "removed": [], "restored": []}

    # Existing order, without the commands that left the table.
    order = [n for n in commands if n in new_set]
    entries = {n: commands[n] for n in order}
    for n in commands:
        if n not in new_set:
            removed[n] = commands[n]
            changes["removed"].append(n)

    for idx, row in enumerate(new_rows):
        name, args = row["name"], row["args"]
        letters = arg_letters(args)
        if name in entries:
            e = entries[name]
            prev = old_args.get(name)
            count_ok = len(e.get("args", [])) == len(letters)
            if (prev is None or prev.rstrip() == args.rstrip()) and count_ok:
                continue
            old_list = e.get("args", [])
            prev_letters = arg_letters(prev) if prev is not None else None
            keep = []
            for i, c in enumerate(letters):
                a = old_list[i] if i < len(old_list) else None
                if a is not None and prev_letters is not None:
                    same = i < len(prev_letters) and prev_letters[i].upper() == c.upper()
                    a = a if same and kind_fits(_arg_kind(a), c) else None
                elif a is not None and not kind_fits(_arg_kind(a), c):
                    a = None
                keep.append(a)
            new_entry = dict(e)
            new_entry["args"] = generated_args(letters, keep)
            new_entry["auto"] = True
            new_entry["auto_note"] = (
                "argument string changed upstream from %s to %s (KeeperFX %s); names and kinds "
                "kept where the type letter did not change" % (
                    json.dumps((prev or "").rstrip()), json.dumps(args.rstrip()), rev_short))
            entries[name] = new_entry
            changes["changed"].append(name)
            continue
        # New command (or one that came back): insert after its predecessor in the table.
        if name in removed:
            back = removed.pop(name)
            e = dict(back)
            if len(e.get("args", [])) != len(letters) or not all(
                    kind_fits(_arg_kind(a), c) for a, c in zip(e.get("args", []), letters)):
                e["args"] = generated_args(letters, [None] * len(letters))
            e["auto"] = True
            e["auto_note"] = "back in the command table (KeeperFX %s); check the entry" % rev_short
            changes["restored"].append(name)
        else:
            e = {
                "doc": "",
                "args": generated_args(letters, [None] * len(letters)),
                "auto": True,
                "auto_note": "new in KeeperFX %s; argument names are generated from the type "
                             "letters %s" % (rev_short, json.dumps(args.rstrip())),
            }
            changes["added"].append(name)
        pos = 0
        for prev_name in reversed(new_names[:idx]):
            if prev_name in entries:
                pos = order.index(prev_name) + 1
                break
        order.insert(pos, name)
        entries[name] = e

    out = {}
    for k, v in overlay.items():
        if k == "commands":
            out[k] = {n: entries[n] for n in order}
        elif k != "removed":
            out[k] = v
    if "commands" not in out:
        out["commands"] = {n: entries[n] for n in order}
    if removed:
        out["removed"] = removed
    return out, changes


def dump_overlay(doc, trailing_newline):
    return json.dumps(doc, indent=1, ensure_ascii=False) + ("\n" if trailing_newline else "")


# --------------------------------------------------------------------------------------------
# Comparison and report
# --------------------------------------------------------------------------------------------

def load_json(text):
    if text is None:
        return None
    try:
        return json.loads(text)
    except ValueError:
        return None


def strip_keys(v, drop):
    if isinstance(v, dict):
        return {k: strip_keys(x, drop) for k, x in v.items() if k not in drop}
    if isinstance(v, list):
        return [strip_keys(x, drop) for x in v]
    return v


class Report:
    def __init__(self):
        self.sections = []      # (title, [markdown lines])
        self.risks = []         # reasons the change is not additive only

    def add(self, title, lines):
        if lines:
            self.sections.append((title, lines))

    def risk(self, why):
        self.risks.append(why)


def names_list(names, limit=60):
    names = list(names)
    shown = ", ".join("`%s`" % n for n in names[:limit])
    if len(names) > limit:
        shown += " and %d more" % (len(names) - limit)
    return shown


def diff_commands(rep, old, new):
    lines = []
    for key, title in (("commands", "Commands"), ("dk1_commands", "Legacy (level version 0) "
                                                                  "commands"),
                       ("subfunctions", "Subfunctions")):
        o = {r["name"]: r["args"].rstrip() for r in (old or {}).get(key, [])}
        n = {r["name"]: r["args"].rstrip() for r in (new or {}).get(key, [])}
        added = [x for x in n if x not in o]
        removed = [x for x in o if x not in n]
        changed = [x for x in n if x in o and o[x] != n[x]]
        if not (added or removed or changed):
            continue
        lines.append("**%s**" % title)
        lines.append("")
        if added:
            lines.append("* added: " + ", ".join("`%s` (`%s`)" % (x, n[x]) for x in added))
        if removed:
            lines.append("* removed: " + names_list(removed))
            rep.risk("%s removed" % title.lower())
        if changed:
            lines.append("* argument string changed:")
            for x in changed:
                lines.append("  * `%s`: `%s` -> `%s`" % (x, o[x], n[x]))
            rep.risk("%s with a changed argument string" % title.lower())
        lines.append("")
    rep.add("Level script commands", lines)

    lines = []
    ot = (old or {}).get("named_tables", {})
    nt = (new or {}).get("named_tables", {})
    for t in sorted(set(ot) | set(nt)):
        o = {r["name"]: r["value"] for r in ot.get(t, [])}
        n = {r["name"]: r["value"] for r in nt.get(t, [])}
        if t not in ot:
            lines.append("* `%s`: new table (%d names)" % (t, len(n)))
            continue
        if t not in nt:
            lines.append("* `%s`: table removed" % t)
            rep.risk("named table %s removed" % t)
            continue
        added = [x for x in n if x not in o]
        removed = [x for x in o if x not in n]
        changed = [x for x in n if x in o and o[x] != n[x]]
        parts = []
        if added:
            parts.append("added " + names_list(added))
        if removed:
            parts.append("removed " + names_list(removed))
            rep.risk("constants removed from %s" % t)
        if changed:
            parts.append("value changed " + names_list(changed))
            rep.risk("constant values changed in %s" % t)
        if parts:
            lines.append("* `%s`: %s" % (t, "; ".join(parts)))
    rep.add("Named constants", lines)


KEY_DOC_FIELDS = {"source", "loader", "doc", "doc_source", "declared"}


def diff_config(rep, old, new):
    lines = []
    of = {f["file"]: f for f in (old or {}).get("files", [])}
    nf = {f["file"]: f for f in (new or {}).get("files", [])}
    doc_changes = 0
    for fname in list(nf) + [x for x in of if x not in nf]:
        if fname not in of:
            lines.append("* `%s`: new file" % fname)
            continue
        if fname not in nf:
            lines.append("* `%s`: file removed" % fname)
            rep.risk("config file %s removed" % fname)
            continue
        os_ = {s["name"]: s for s in of[fname].get("sections", [])}
        ns = {s["name"]: s for s in nf[fname].get("sections", [])}
        for sname in list(ns) + [x for x in os_ if x not in ns]:
            where = "`%s` [%s]" % (fname, sname)
            if sname not in os_:
                lines.append("* %s: new section (%d keys)" % (where, len(ns[sname].get("keys", []))))
                continue
            if sname not in ns:
                lines.append("* %s: section removed" % where)
                rep.risk("config section %s [%s] removed" % (fname, sname))
                continue
            ok = {k["name"]: k for k in os_[sname].get("keys", [])}
            nk = {k["name"]: k for k in ns[sname].get("keys", [])}
            added = [k for k in nk if k not in ok]
            removed = [k for k in ok if k not in nk]
            changed = []
            for k in nk:
                if k in ok:
                    a, b = strip_keys(ok[k], KEY_DOC_FIELDS), strip_keys(nk[k], KEY_DOC_FIELDS)
                    if a != b:
                        changed.append(k)
                    elif ok[k].get("doc") != nk[k].get("doc"):
                        doc_changes += 1
            sec_a = strip_keys({x: y for x, y in os_[sname].items() if x != "keys"}, KEY_DOC_FIELDS)
            sec_b = strip_keys({x: y for x, y in ns[sname].items() if x != "keys"}, KEY_DOC_FIELDS)
            parts = []
            if added:
                parts.append("added " + names_list(added))
            if removed:
                parts.append("removed " + names_list(removed))
                rep.risk("config keys removed")
            if changed:
                parts.append("changed " + names_list(changed))
                rep.risk("config keys changed")
            if sec_a != sec_b:
                parts.append("section layout changed")
                rep.risk("config section layout changed")
            if parts:
                lines.append("* %s: %s" % (where, "; ".join(parts)))
    oe = (old or {}).get("enums", {})
    ne = (new or {}).get("enums", {})
    for e in sorted(set(oe) | set(ne)):
        o = {v["name"]: v.get("value") for v in oe.get(e, [])}
        n = {v["name"]: v.get("value") for v in ne.get(e, [])}
        added = [x for x in n if x not in o]
        removed = [x for x in o if x not in n]
        changed = [x for x in n if x in o and o[x] != n[x]]
        parts = []
        if added:
            parts.append("added " + names_list(added))
        if removed:
            parts.append("removed " + names_list(removed))
            rep.risk("config enum values removed")
        if changed:
            parts.append("value changed " + names_list(changed))
            rep.risk("config enum values changed")
        if parts:
            lines.append("* enum `%s`: %s" % (e, "; ".join(parts)))
    if doc_changes:
        lines.append("* %d key descriptions changed" % doc_changes)
    rep.add("Configuration files", lines)


LUA_DOC_FIELDS = {"source", "doc", "line", "start", "end"}


def diff_named_dicts(old, new, label, rep, risky_removal=True):
    """Added / removed / changed names of two {name: obj} maps. Returns markdown parts."""
    added = [x for x in new if x not in old]
    removed = [x for x in old if x not in new]
    changed = [x for x in new if x in old
               and strip_keys(old[x], LUA_DOC_FIELDS) != strip_keys(new[x], LUA_DOC_FIELDS)]
    parts = []
    if added:
        parts.append("added " + names_list(added))
    if removed:
        parts.append("removed " + names_list(removed))
        if risky_removal:
            rep.risk("Lua %s removed" % label)
    if changed:
        parts.append("signature or type changed " + names_list(changed))
        rep.risk("Lua %s changed" % label)
    return parts


def diff_lua(rep, old, new, old_c, new_c):
    lines = []
    old, new = old or {}, new or {}
    for key in ("functions", "globals", "aliases", "enums"):
        parts = diff_named_dicts(old.get(key, {}), new.get(key, {}), key, rep)
        if parts:
            lines.append("* %s: %s" % (key, "; ".join(parts)))
    oc, nc = old.get("classes", {}), new.get("classes", {})
    for cls in sorted(set(oc) | set(nc)):
        if cls not in oc:
            lines.append("* class `%s`: new" % cls)
            continue
        if cls not in nc:
            lines.append("* class `%s`: removed" % cls)
            rep.risk("Lua classes removed")
            continue
        parts = []
        for member in ("methods", "fields"):
            p = diff_named_dicts(oc[cls].get(member, {}), nc[cls].get(member, {}), member, rep)
            parts += ["%s %s" % (member, x) for x in p]
        if oc[cls].get("parents") != nc[cls].get("parents"):
            parts.append("parents changed")
            rep.risk("Lua class parents changed")
        if parts:
            lines.append("* class `%s`: %s" % (cls, "; ".join(parts)))
    files_added = [f for f in new.get("files", []) if f not in old.get("files", [])]
    files_removed = [f for f in old.get("files", []) if f not in new.get("files", [])]
    if files_added:
        lines.append("* stub files added: " + names_list(files_added))
    if files_removed:
        lines.append("* stub files removed: " + names_list(files_removed))

    def live(v):
        return {e["name"] for e in v if not e.get("commented")}

    old_c, new_c = old_c or {}, new_c or {}
    og, ng = live(old_c.get("globals", [])), live(new_c.get("globals", []))
    if ng - og:
        lines.append("* C registrations added: " + names_list(sorted(ng - og)))
    if og - ng:
        lines.append("* C registrations removed: " + names_list(sorted(og - ng)))
        rep.risk("Lua C registrations removed")
    for cls in sorted(set(old_c.get("methods", {})) | set(new_c.get("methods", {}))):
        a = live(old_c.get("methods", {}).get(cls, []))
        b = live(new_c.get("methods", {}).get(cls, []))
        if b - a:
            lines.append("* C methods of `%s` added: %s" % (cls, names_list(sorted(b - a))))
        if a - b:
            lines.append("* C methods of `%s` removed: %s" % (cls, names_list(sorted(a - b))))
            rep.risk("Lua C methods removed")
    for cls in sorted(set(old_c.get("fields", {})) | set(new_c.get("fields", {}))):
        for which in ("get", "set"):
            a = {e["name"] for e in old_c.get("fields", {}).get(cls, {}).get(which, [])}
            b = {e["name"] for e in new_c.get("fields", {}).get(cls, {}).get(which, [])}
            if b - a:
                lines.append("* C %s fields of `%s` added: %s" % (which, cls, names_list(sorted(b - a))))
            if a - b:
                lines.append("* C %s fields of `%s` removed: %s" % (which, cls,
                                                                   names_list(sorted(a - b))))
                rep.risk("Lua C fields removed")
    oh = {h["name"] for h in old_c.get("hooks", [])}
    nh = {h["name"] for h in new_c.get("hooks", [])}
    if nh - oh:
        lines.append("* engine hooks added (add them to crates/kfx-script/src/lua/hooks.rs): "
                     + names_list(sorted(nh - oh)))
        rep.risk("new Lua engine hooks need an entry in hooks.rs")
    if oh - nh:
        lines.append("* engine hooks removed: " + names_list(sorted(oh - nh)))
        rep.risk("Lua engine hooks removed")
    rep.add("Lua API", lines)


RS_LIST_RE = re.compile(r"pub const (\w+): \[[^\]]+\] = \[(.*?)\];", re.S)


def parse_gen_rs(text):
    out = {}
    for m in RS_LIST_RE.finditer(text or ""):
        body = m.group(2)
        pairs = re.findall(r'\("(\w+)", (\d+)\)', body)
        out[m.group(1)] = pairs if pairs else re.findall(r"\d+", body)
    return out


def diff_gfx(rep, old_texts, new_texts):
    lines = []
    for rel in (GEN_CREATURES, GEN_SPRITES):
        o, n = parse_gen_rs(old_texts.get(rel)), parse_gen_rs(new_texts.get(rel))
        for name in sorted(set(o) | set(n)):
            a, b = o.get(name, []), n.get(name, [])
            if a == b:
                continue
            if a and isinstance(a[0], tuple) or b and isinstance(b[0], tuple):
                an, bn = dict(a), dict(b)
                parts = []
                added = [x for x in bn if x not in an]
                removed = [x for x in an if x not in bn]
                renum = [x for x in bn if x in an and an[x] != bn[x]]
                if added:
                    parts.append("added " + names_list(added))
                if removed:
                    parts.append("removed " + names_list(removed))
                if renum:
                    parts.append("renumbered " + names_list(renum))
                lines.append("* `%s` (%d -> %d entries): %s" % (name, len(a), len(b),
                                                               "; ".join(parts) or "reordered"))
            else:
                diffs = sum(1 for x, y in zip(a, b) if x != y)
                lines.append("* `%s`: %d -> %d entries, %d values differ" % (
                    name, len(a), len(b), diffs))
    if lines:
        rep.risk("sprite or animation tables changed")
    rep.add("Sprites and animations", lines)


def diff_docs(rep, old_docs, new_docs, old_c, new_c):
    lines = []
    o = (old_docs or {}).get("commands", {})
    n = (new_docs or {}).get("commands", {})
    added = [x for x in n if x not in o]
    removed = [x for x in o if x not in n]
    changed = [x for x in n if x in o and o[x] != n[x]]
    if added:
        lines.append("* wiki entries added: " + names_list(added))
    if changed:
        lines.append("* wiki entries changed: " + names_list(changed))
    if removed:
        lines.append("* wiki entries removed: " + names_list(removed))

    def texts(d):
        return {k: [x.get("text") for x in v] for k, v in (d or {}).get("commands", {}).items()}

    oc, nc = texts(old_c), texts(new_c)
    cc = [x for x in sorted(set(oc) | set(nc)) if oc.get(x) != nc.get(x)]
    if cc:
        lines.append("* C source comments changed: " + names_list(cc))
    rep.add("Documentation", lines)


def overlay_section(rep, overlay, changes):
    lines = []
    if changes["added"]:
        lines.append("* generated for new commands: " + names_list(changes["added"]))
    if changes["changed"]:
        lines.append("* adjusted to a changed argument string: " + names_list(changes["changed"]))
    if changes["restored"]:
        lines.append("* restored from `removed`: " + names_list(changes["restored"]))
    if changes["removed"]:
        lines.append("* moved to `removed`: " + names_list(changes["removed"]))
    pending = [n for n, e in overlay.get("commands", {}).items() if e.get("auto")]
    if pending:
        lines.append("")
        lines.append("Entries marked `\"auto\": true` that need a human pass (argument names, "
                     "kinds and a one-line description from the command's `*_check` function in "
                     "`src/lvl_script_commands.c`; then delete `auto` and `auto_note`):")
        lines.append("")
        for n in pending:
            e = overlay["commands"][n]
            lines.append("* `%s`: %s" % (n, e.get("auto_note", "")))
    rep.add("Argument overlay (data/script_arg_overlay.json)", lines)


def short(h):
    return (h or "")[:7]


def build_report(old_up, revs, changed_files, rep):
    k_old = (old_up or {}).get("keeperfx", {})
    k_new = revs["keeperfx"]
    w_old = (old_up or {}).get("wiki", {})
    w_new = revs["wiki"]
    out = ["# Sync data with KeeperFX %s" % short(k_new["commit"]), ""]
    if k_old.get("commit"):
        rng = "[`%s`](%s/commit/%s) (%s) -> [`%s`](%s/commit/%s) (%s)" % (
            short(k_old["commit"]), KEEPERFX_URL, k_old["commit"], k_old.get("date", "?")[:10],
            short(k_new["commit"]), KEEPERFX_URL, k_new["commit"], k_new["date"][:10])
        if k_old["commit"] != k_new["commit"]:
            rng += ", [compare](%s/compare/%s...%s)" % (KEEPERFX_URL, k_old["commit"],
                                                       k_new["commit"])
    else:
        rng = "(no previous revision recorded) -> [`%s`](%s/commit/%s) (%s)" % (
            short(k_new["commit"]), KEEPERFX_URL, k_new["commit"], k_new["date"][:10])
    out.append("KeeperFX: " + rng)
    out.append("")
    out.append("Wiki: `%s` -> `%s`" % (short(w_old.get("commit")) or "none",
                                         short(w_new["commit"])))
    out.append("")
    if not changed_files:
        out.append("No generated file changes.")
        return "\n".join(out) + "\n"
    if rep.risks:
        out.append("**Review needed:** " + "; ".join(sorted(set(rep.risks))) + ".")
    else:
        out.append("**Additive only:** new commands, constants, keys, functions or documentation; "
                   "nothing was removed and no signature changed.")
    out.append("")
    out.append("Changed files: " + ", ".join("`%s`" % f for f in changed_files))
    out.append("")
    for title, lines in rep.sections:
        out.append("## " + title)
        out.append("")
        out.extend(lines)
        if lines[-1] != "":
            out.append("")
    return "\n".join(out).rstrip("\n") + "\n"


# --------------------------------------------------------------------------------------------
# Main
# --------------------------------------------------------------------------------------------

def read_repo(rel):
    p = REPO / rel
    return p.read_text(encoding="utf-8") if p.is_file() else None


def sync(args):
    revs = revisions(args)
    work_parent = Path(args.work) if args.work else None
    if work_parent:
        work_parent.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="kfx-sync-", dir=work_parent))
    try:
        src_root, install, wiki_root = stage(args.source, args.wiki, work)
        out = work / "out"
        generate(src_root, install, wiki_root, revs["wiki"]["commit"], out, args.jobs, args.cargo)

        old = {rel: read_repo(rel) for rel in OUTPUTS}
        new = {rel: (out / rel).read_text(encoding="utf-8") for rel in OUTPUTS if rel != OVERLAY}

        old_overlay_text = old[OVERLAY]
        if old_overlay_text is None:
            raise SyncError("%s is missing" % OVERLAY)
        overlay, changes = reconcile_overlay(
            json.loads(old_overlay_text), load_json(old[TABLES]) or {}, json.loads(new[TABLES]),
            short(revs["keeperfx"]["commit"]))
        new[OVERLAY] = dump_overlay(overlay, old_overlay_text.endswith("\n"))
        if new[OVERLAY] == old_overlay_text:
            new[OVERLAY] = old_overlay_text

        changed = [rel for rel in OUTPUTS if new[rel] != old[rel]]
        old_up_text = read_repo(UPSTREAM)
        write_upstream = bool(changed) or old_up_text is None

        rep = Report()
        diff_commands(rep, load_json(old[TABLES]), load_json(new[TABLES]))
        diff_config(rep, load_json(old[CONFIG_SCHEMA]), load_json(new[CONFIG_SCHEMA]))
        diff_lua(rep, load_json(old[LUA_API]), load_json(new[LUA_API]), load_json(old[LUA_C_API]),
                 load_json(new[LUA_C_API]))
        diff_gfx(rep, old, new)
        diff_docs(rep, load_json(old[DOCS]), load_json(new[DOCS]), load_json(old[C_COMMENTS]),
                  load_json(new[C_COMMENTS]))
        overlay_section(rep, overlay, changes)
        report = build_report(load_json(old_up_text), revs, changed, rep)

        if args.report:
            Path(args.report).write_text(report, encoding="utf-8")
        if args.summary:
            summary = {
                "changed": bool(changed),
                "changed_files": changed,
                "upstream_json_written": write_upstream and not args.check,
                "additive_only": bool(changed) and not rep.risks,
                "risks": sorted(set(rep.risks)),
                "keeperfx_commit": revs["keeperfx"]["commit"],
                "keeperfx_short": short(revs["keeperfx"]["commit"]),
                "keeperfx_date": revs["keeperfx"]["date"],
                "wiki_commit": revs["wiki"]["commit"],
                "overlay_auto": [n for n, e in overlay["commands"].items() if e.get("auto")],
            }
            Path(args.summary).write_text(json.dumps(summary, indent=1) + "\n", encoding="utf-8")

        for rel in changed:
            print("%s %s" % ("would change" if args.check else "changed", rel))
        if not changed:
            print("generated files are up to date with KeeperFX %s" % short(
                revs["keeperfx"]["commit"]))
        if args.check:
            if old_up_text is None:
                print("would create %s" % UPSTREAM)
            return 1 if write_upstream else 0
        for rel in changed:
            dest = REPO / rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            tmp = dest.with_name(dest.name + ".sync-tmp")
            tmp.write_text(new[rel], encoding="utf-8", newline="\n")
            os.replace(tmp, dest)
        if write_upstream:
            text = upstream_json(revs)
            if text != old_up_text:
                (REPO / UPSTREAM).write_text(text, encoding="utf-8", newline="\n")
                print("wrote %s" % UPSTREAM)
        return 0
    finally:
        if not args.keep_work:
            shutil.rmtree(work, ignore_errors=True)
        else:
            print("work folder kept: %s" % work)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("source", type=Path, help="checkout of the KeeperFX repository (read only)")
    ap.add_argument("wiki", type=Path, help="Markdown folder of a KeeperFX wiki checkout (read only)")
    ap.add_argument("--check", action="store_true",
                    help="write nothing; exit 1 when a generated file would change")
    ap.add_argument("--report", help="write a Markdown change report to this file")
    ap.add_argument("--summary", help="write a JSON summary to this file")
    ap.add_argument("--keeperfx-rev", help="KeeperFX commit hash (default: git log of SOURCE)")
    ap.add_argument("--keeperfx-date", help="KeeperFX commit date, ISO 8601 (default: git log)")
    ap.add_argument("--wiki-rev", help="wiki commit hash (default: git log of WIKI)")
    ap.add_argument("--work", help="parent folder of the temporary work folder (default: system "
                                   "temporary folder)")
    ap.add_argument("--keep-work", action="store_true", help="keep the work folder")
    ap.add_argument("--jobs", type=int, default=4, help="cargo build jobs (default 4)")
    ap.add_argument("--cargo", default=os.environ.get("CARGO", "cargo"), help="cargo executable")
    args = ap.parse_args(argv)
    try:
        return sync(args)
    except SyncError as e:
        print("error: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
