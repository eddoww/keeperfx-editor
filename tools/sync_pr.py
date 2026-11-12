#!/usr/bin/env python3
"""Pull request title and text of the KeeperFX sync workflow (.github/workflows/sync-keeperfx.yml).

    python3 -I tools/sync_pr.py --report REPORT.md --summary SUMMARY.json --test-log TEST.log \
        --fmt pass|fail --tests pass|fail --body-out BODY.md

Prints the title on stdout and writes the body: a status note (whether formatting and the tests
passed, with the names of the failing tests), then the change report of tools/sync_keeperfx.py.
Standard library only.
"""

import argparse
import json
import re
import sys
from pathlib import Path

FAILED_RE = re.compile(r"^test (\S+) \.\.\. FAILED$", re.M)
TARGET_RE = re.compile(r"^error: test failed, to rerun pass `([^`]+)`$", re.M)
PY_FAILED_RE = re.compile(r"^(?:FAIL|ERROR): (\S+) \(([^)]+)\)$", re.M)
MAX_FAILURES = 50


def failures(log):
    names = FAILED_RE.findall(log)
    names += ["%s.%s" % (cls, name) for name, cls in PY_FAILED_RE.findall(log)]
    targets = TARGET_RE.findall(log)
    seen = []
    for n in names:
        if n not in seen:
            seen.append(n)
    return seen, targets


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--report", required=True)
    ap.add_argument("--summary", required=True)
    ap.add_argument("--test-log", required=True)
    ap.add_argument("--fmt", choices=("pass", "fail"), required=True)
    ap.add_argument("--tests", choices=("pass", "fail"), required=True)
    ap.add_argument("--body-out", required=True)
    args = ap.parse_args(argv)

    summary = json.loads(Path(args.summary).read_text(encoding="utf-8"))
    report = Path(args.report).read_text(encoding="utf-8")
    log_path = Path(args.test_log)
    log = log_path.read_text(encoding="utf-8", errors="replace") if log_path.is_file() else ""
    ok = args.fmt == "pass" and args.tests == "pass"
    title = "Sync data with KeeperFX %s" % summary["keeperfx_short"]
    if not ok:
        title = "[tests failing] " + title

    note = []
    if ok:
        note.append("> **Tests passed.** `cargo fmt --all --check` and the tests of the crates "
                    "that embed the generated data pass on this branch.")
    else:
        note.append("> **Tests failing: upstream drift that needs hand work.** The data was "
                    "regenerated, but this branch does not pass as it is.")
        if args.fmt == "fail":
            note.append(">")
            note.append("> * `cargo fmt --all --check` failed (a generated Rust table?).")
        names, targets = failures(log)
        if args.tests == "fail":
            note.append(">")
            if names:
                note.append("> Failing tests:")
                note.append(">")
                for n in names[:MAX_FAILURES]:
                    note.append("> * `%s`" % n)
                if len(names) > MAX_FAILURES:
                    note.append("> * and %d more" % (len(names) - MAX_FAILURES))
            else:
                note.append("> The test build failed (no test ran); see the workflow log.")
            if targets:
                note.append(">")
                note.append("> Failing test targets: " + ", ".join("`%s`" % t for t in targets))
    if summary.get("overlay_auto"):
        note.append("")
        note.append("Overlay entries to refine by hand: %d (listed at the end)." %
                    len(summary["overlay_auto"]))
    note.append("")
    note.append("This pull request is opened and updated by the `sync-keeperfx` workflow; it is "
                "force-pushed on every run that finds new upstream changes.")
    note.append("")
    body = "\n".join(note) + "\n" + report
    Path(args.body_out).write_text(body, encoding="utf-8")
    print(title)
    return 0


if __name__ == "__main__":
    sys.exit(main())
