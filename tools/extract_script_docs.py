#!/usr/bin/env python3
"""Extract KeeperFX level-script command documentation from the wiki.

Regenerate (from the repository root; KFX_WIKI is the folder of a checkout of the KeeperFX
wiki that holds the Markdown pages):

    python3 -I tools/extract_script_docs.py "$KFX_WIKI" data/script_tables.json data/script_docs.json

Reads the markdown pages of a checkout of https://github.com/dkfans/keeperfx.wiki
(read only) and writes a JSON document with one entry per level-script command
heading (an h3 whose text is an upper-case identifier). Every field is taken
from the wiki text; fields the wiki does not provide are left empty.

The wiki is untrusted data: this script only reads files. It does not run git
or any command found in the wiki. The commit hash is read from the .git
directory files directly.

Standard library only. Prints a coverage report on stdout.
"""

import argparse
import json
import os
import re
import sys
from pathlib import Path

HEADING_RE = re.compile(r"^(#{1,6})[ \t]+(.+?)[ \t]*#*[ \t]*$")
FENCE_RE = re.compile(r"^[ \t]*(`{3,}|~{3,})")
COMMAND_HEADING_RE = re.compile(r"^[A-Z][A-Z0-9_]*$")
PARAM_RE = re.compile(r"^[ \t]*\[([^\[\]]+)\][ \t]*\*?[ \t]*(?:-|–|:)[ \t]*(.*)$")
BULLET_RE = re.compile(r"^[ \t]*(?:[*+-]|\d+\.)[ \t]")
LICENCE_RE = re.compile(
    r"licen[cs]e|copyright|creative commons|all rights reserved",
    re.IGNORECASE,
)


def read_text(path):
    return Path(path).read_text(encoding="utf-8", errors="replace")


def wiki_commit(wiki_dir):
    """Resolve HEAD of the wiki checkout by reading .git files directly."""
    git_dir = Path(wiki_dir) / ".git"
    try:
        head = read_text(git_dir / "HEAD").strip()
        if head.startswith("ref: "):
            ref = head[len("ref: "):].strip()
            ref_file = git_dir / ref
            if ref_file.is_file():
                return read_text(ref_file).strip()
            packed = git_dir / "packed-refs"
            if packed.is_file():
                for line in read_text(packed).splitlines():
                    parts = line.split()
                    if len(parts) == 2 and parts[1] == ref:
                        return parts[0]
            return "unknown"
        return head
    except OSError:
        return "unknown"


def wiki_licence_note(wiki_dir):
    """Return the first sentence-ish line that states a licence, else 'not stated'."""
    for path in sorted(Path(wiki_dir).glob("*.md")):
        for line in read_text(path).splitlines():
            if LICENCE_RE.search(line):
                return "%s: %s" % (path.name, line.strip())
    return "not stated"


def split_blocks(lines):
    """Yield (kind, index, text) for each line; kind is 'fence', 'code', 'heading' or 'prose'.

    Lines inside fenced code blocks are marked 'code'; the fence delimiters are
    marked 'fence'. Headings are only recognised outside fences.
    """
    out = []
    marker = None
    for i, line in enumerate(lines):
        if marker is None:
            m = FENCE_RE.match(line)
            if m:
                marker = m.group(1)
                out.append(("fence", i, line))
                continue
            if HEADING_RE.match(line):
                out.append(("heading", i, line))
            else:
                out.append(("prose", i, line))
        else:
            stripped = line.strip()
            if stripped.startswith(marker[0] * len(marker)) and set(stripped) <= {marker[0]}:
                marker = None
                out.append(("fence", i, line))
            else:
                out.append(("code", i, line))
    return out


def find_sections(lines):
    """Return list of (level, title, body_start, body_end) for every heading."""
    kinds = split_blocks(lines)
    heads = []
    for kind, i, line in kinds:
        if kind == "heading":
            m = HEADING_RE.match(line)
            heads.append((len(m.group(1)), m.group(2).strip(), i))
    sections = []
    for n, (level, title, idx) in enumerate(heads):
        end = len(lines)
        for lvl2, _t2, idx2 in heads[n + 1:]:
            if lvl2 <= level:
                end = idx2
                break
        sections.append((level, title, idx + 1, end))
    return sections


def fenced_groups(body):
    """Split body lines into an ordered list of ('prose', [lines]) / ('code', [lines]) groups."""
    groups = []
    marker = None
    current = None
    for line in body:
        if marker is None:
            m = FENCE_RE.match(line)
            if m:
                marker = m.group(1)
                groups.append(("code", []))
                continue
            if groups and groups[-1][0] == "prose":
                groups[-1][1].append(line)
            else:
                groups.append(("prose", [line]))
        else:
            stripped = line.strip()
            if stripped.startswith(marker[0] * len(marker)) and set(stripped) <= {marker[0]}:
                marker = None
                continue
            groups[-1][1].append(line)
    return groups


def trim_blank(lines):
    start, end = 0, len(lines)
    while start < end and not lines[start].strip():
        start += 1
    while end > start and not lines[end - 1].strip():
        end -= 1
    return lines[start:end]


def join_text(lines):
    return " ".join(part.strip() for part in lines if part.strip())


def first_paragraph(groups):
    """First prose paragraph of a section (fenced code skipped), joined into one line.

    Returns "" if a sub-heading comes before any prose paragraph.
    """
    para = []
    for kind, lines in groups:
        if kind != "prose":
            continue
        for line in lines:
            if HEADING_RE.match(line):
                return join_text(para)
            if not line.strip():
                if para:
                    return join_text(para)
                continue
            para.append(line)
        if para:
            return join_text(para)
    return join_text(para)


def signature_of(name, groups):
    """First fenced block whose first line starts with the command name."""
    for kind, lines in groups:
        if kind != "code":
            continue
        content = trim_blank(lines)
        if not content:
            continue
        first = content[0].strip()
        if first == name or first.startswith(name + "(") or first.startswith(name + " "):
            return "\n".join(line.rstrip() for line in content), lines
    return "", None


def parse_params(body):
    """Collect '[name] - text' parameter entries from prose lines outside fences."""
    params = []
    current = None
    blank_pending = False
    marker = None
    for line in body:
        if marker is None and FENCE_RE.match(line):
            marker = FENCE_RE.match(line).group(1)
            current = None
            blank_pending = False
            continue
        if marker is not None:
            stripped = line.strip()
            if stripped.startswith(marker[0] * len(marker)) and set(stripped) <= {marker[0]}:
                marker = None
            continue
        if HEADING_RE.match(line):
            current = None
            blank_pending = False
            continue
        m = PARAM_RE.match(line)
        if m:
            current = {"name": m.group(1).strip(), "text_lines": [m.group(2)]}
            params.append(current)
            blank_pending = False
            continue
        if current is None:
            continue
        if not line.strip():
            blank_pending = True
            continue
        if blank_pending and not BULLET_RE.match(line) and not line.startswith(" "):
            # A new unindented paragraph ends the parameter text.
            current = None
            blank_pending = False
            continue
        blank_pending = False
        current["text_lines"].append(line)
    return [{"name": p["name"], "text": join_text(p["text_lines"])} for p in params]


def parse_command(name, body_lines):
    groups = fenced_groups(body_lines)
    signature, sig_lines = signature_of(name, groups)
    examples = []
    for kind, lines in groups:
        if kind != "code" or lines is sig_lines:
            continue
        content = trim_blank(lines)
        if content:
            examples.append("\n".join(line.rstrip() for line in content))
    return {
        "signature": signature,
        "summary": first_paragraph(groups),
        "params": parse_params(body_lines),
        "markdown": "\n".join(trim_blank(body_lines)),
        "examples": examples,
    }


def collect_docs(wiki_dir):
    docs = {}
    duplicates = {}
    for path in sorted(Path(wiki_dir).glob("*.md")):
        lines = read_text(path).splitlines()
        for level, title, body_start, body_end in find_sections(lines):
            if level != 3 or not COMMAND_HEADING_RE.match(title):
                continue
            entry = parse_command(title, lines[body_start:body_end])
            entry = {"page": path.stem, **entry}
            if title in docs:
                duplicates.setdefault(title, []).append(path.stem)
            # The last occurrence wins: later sections in the wiki describe newer edits.
            docs[title] = entry
    return docs, duplicates


def load_table_names(path):
    data = json.loads(read_text(path))
    names = []
    for row in data["commands"]:
        names.append(row["name"])
    return names


def build(wiki_dir, tables_path, out_path):
    wiki_dir = Path(wiki_dir)
    if not wiki_dir.is_dir():
        sys.exit("wiki directory not found: %s" % wiki_dir)
    table_names = load_table_names(tables_path)
    docs, duplicates = collect_docs(wiki_dir)
    commit = wiki_commit(wiki_dir)

    commands = {}
    for name in sorted(docs):
        entry = docs[name]
        commands[name] = {
            "page": entry["page"],
            "signature": entry["signature"],
            "summary": entry["summary"],
            "params": entry["params"],
            "markdown": entry["markdown"],
            "examples": entry["examples"],
        }

    out = {
        "source": "dkfans/keeperfx wiki, %s" % commit,
        "license_note": wiki_licence_note(wiki_dir),
        "commands": commands,
    }
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    tmp = out_path.with_suffix(out_path.suffix + ".tmp")
    tmp.write_text(json.dumps(out, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(tmp, out_path)

    table_set = set(table_names)
    doc_set = set(docs)
    documented_in_table = [n for n in table_names if n in doc_set]
    missing = sorted(table_set - doc_set)
    extra = sorted(doc_set - table_set)

    print("source: %s" % out["source"])
    print("license_note: %s" % out["license_note"])
    print("wiki command headings parsed: %d" % len(docs))
    print("table commands: %d (unique: %d)" % (len(table_names), len(table_set)))
    print("table commands with docs: %d / %d" % (len(documented_in_table), len(table_names)))
    print("table commands without docs (%d):" % len(missing))
    for name in missing:
        print("  " + name)
    print("documented names not in table (%d):" % len(extra))
    for name in extra:
        print("  %s (page %s)" % (name, docs[name]["page"]))
    if duplicates:
        print("duplicated headings (last occurrence kept):")
        for name in sorted(duplicates):
            print("  %s: also in %s" % (name, ", ".join(duplicates[name])))
    no_sig = sorted(n for n in docs if not commands[n]["signature"])
    print("entries without signature (%d): %s" % (len(no_sig), ", ".join(no_sig)))
    no_summary = sorted(n for n in docs if not commands[n]["summary"])
    print("entries without summary (%d): %s" % (len(no_summary), ", ".join(no_summary)))
    print("written: %s" % out_path)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    parser.add_argument("wiki_dir", help="checkout of dkfans/keeperfx.wiki (read only)")
    parser.add_argument("script_tables", help="path to script_tables.json")
    parser.add_argument("output", help="path of the JSON file to write")
    args = parser.parse_args(argv)
    return build(args.wiki_dir, args.script_tables, args.output)


if __name__ == "__main__":
    sys.exit(main())
