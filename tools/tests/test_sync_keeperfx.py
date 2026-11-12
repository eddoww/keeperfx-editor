#!/usr/bin/env python3
"""Tests of tools/sync_keeperfx.py and of the hardening of the extractors.

    python3 -I tools/tests/test_sync_keeperfx.py

The fixtures are a synthetic KeeperFX source tree in two states (fixtures/keeperfx_old and
fixtures/keeperfx_new: a command added, one whose argument string gained an optional argument,
one removed from both command tables, a constant added) and an overlay written for the old one.
The results of the new state are kept in fixtures/expected/; the Rust test
crates/kfx-script/tests/sync_fixture.rs runs the script services on those files. After a
deliberate change of the reconciliation, rewrite them with KFX_SYNC_FIXTURE_UPDATE=1.
Standard library only.
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
TOOLS = HERE.parent
FIX = HERE / "fixtures"
EXPECTED = FIX / "expected"
sys.path.insert(0, str(TOOLS))
sys.dont_write_bytecode = True  # keep tools/ free of __pycache__

import sync_keeperfx as sync  # noqa: E402


def extract_tables(tree, out):
    subprocess.run([sys.executable, "-I", str(TOOLS / "extract_script_tables.py"), str(tree),
                    str(out)], check=True, capture_output=True)
    return out.read_text(encoding="utf-8")


def load_tool(name):
    """Import one of the extractor scripts as a module."""
    import importlib.util
    spec = importlib.util.spec_from_file_location(name, TOOLS / (name + ".py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class SyncFixture(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix="kfx-sync-test-")
        t = Path(cls.tmp.name)
        cls.old_tables_text = extract_tables(FIX / "keeperfx_old", t / "old.json")
        cls.new_tables_text = extract_tables(FIX / "keeperfx_new", t / "new.json")
        cls.old_tables = json.loads(cls.old_tables_text)
        cls.new_tables = json.loads(cls.new_tables_text)
        cls.overlay_text = (FIX / "overlay_old.json").read_text(encoding="utf-8")
        cls.overlay, cls.changes = sync.reconcile_overlay(
            json.loads(cls.overlay_text), cls.old_tables, cls.new_tables, "abcdef1")
        cls.overlay_out = sync.dump_overlay(cls.overlay, cls.overlay_text.endswith("\n"))
        if os.environ.get("KFX_SYNC_FIXTURE_UPDATE"):
            EXPECTED.mkdir(exist_ok=True)
            (EXPECTED / "script_tables.json").write_text(cls.new_tables_text, encoding="utf-8")
            (EXPECTED / "script_arg_overlay.json").write_text(cls.overlay_out, encoding="utf-8")

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_outputs_match_the_checked_in_expectations(self):
        self.assertEqual(self.new_tables_text,
                         (EXPECTED / "script_tables.json").read_text(encoding="utf-8"))
        self.assertEqual(self.overlay_out,
                         (EXPECTED / "script_arg_overlay.json").read_text(encoding="utf-8"))

    def test_changes_are_classified(self):
        self.assertEqual(self.changes, {"added": ["SET_FIXTURE_GLOW"],
                                        "changed": ["ADD_GOLD_TO_PLAYER"],
                                        "removed": ["FIXTURE_OLD_BEACON"],
                                        "restored": []})

    def test_new_command_gets_a_generated_entry_in_table_order(self):
        cmds = self.overlay["commands"]
        self.assertEqual(list(cmds), ["LEVEL_VERSION", "WIN_GAME", "SET_FLAG",
                                      "ADD_GOLD_TO_PLAYER", "SET_FIXTURE_GLOW", "MAX_CREATURES"])
        e = cmds["SET_FIXTURE_GLOW"]
        self.assertEqual(e["args"], ["player*:player", "number2*:number", "number3*:number"])
        self.assertEqual(e["doc"], "")
        self.assertIs(e["auto"], True)
        self.assertIn("abcdef1", e["auto_note"])

    def test_changed_command_keeps_hand_written_positions(self):
        e = self.overlay["commands"]["ADD_GOLD_TO_PLAYER"]
        self.assertEqual(e["args"], ["player:player", "gold:amount", "number3*:number"])
        self.assertEqual(e["doc"], "Gives gold to a player.")
        self.assertIs(e["auto"], True)
        self.assertIn('"PN" to "PNn"', e["auto_note"])

    def test_removed_command_moves_to_removed(self):
        self.assertNotIn("FIXTURE_OLD_BEACON", self.overlay["commands"])
        self.assertEqual(self.overlay["removed"]["FIXTURE_OLD_BEACON"]["args"],
                         ["player:player", "strength*:number"])

    def test_untouched_entries_keep_their_text(self):
        old = json.loads(self.overlay_text)["commands"]
        for name in ("LEVEL_VERSION", "WIN_GAME", "SET_FLAG", "MAX_CREATURES"):
            self.assertEqual(self.overlay["commands"][name], old[name])
            # byte for byte, including the layout
            snippet = json.dumps({name: old[name]}, indent=1, ensure_ascii=False)[2:-2]
            self.assertIn(snippet.replace("\n", "\n "), self.overlay_out)

    def test_overlay_matches_arities_and_letter_kinds(self):
        rows = {r["name"]: r["args"] for r in self.new_tables["commands"]}
        self.assertEqual(set(rows), set(self.overlay["commands"]))
        for name, e in self.overlay["commands"].items():
            letters = sync.arg_letters(rows[name])
            self.assertEqual(len(e["args"]), len(letters), name)
            for a, c in zip(e["args"], letters):
                self.assertTrue(sync.kind_fits(a.split(":", 1)[1], c), (name, a, c))

    def test_reconcile_is_idempotent(self):
        again, changes = sync.reconcile_overlay(self.overlay, self.new_tables, self.new_tables,
                                                "abcdef1")
        self.assertEqual(again, self.overlay)
        self.assertEqual(changes, {"added": [], "changed": [], "removed": [], "restored": []})

    def test_command_coming_back_is_restored(self):
        back, changes = sync.reconcile_overlay(self.overlay, self.new_tables, self.old_tables,
                                               "1234567")
        self.assertEqual(changes["restored"], ["FIXTURE_OLD_BEACON"])
        self.assertEqual(changes["removed"], ["SET_FIXTURE_GLOW"])
        self.assertEqual(back["commands"]["FIXTURE_OLD_BEACON"]["args"],
                         ["player:player", "strength*:number"])
        self.assertNotIn("FIXTURE_OLD_BEACON", back.get("removed", {}))

    def test_report_lists_every_kind_of_change(self):
        rep = sync.Report()
        sync.diff_commands(rep, self.old_tables, self.new_tables)
        sync.overlay_section(rep, self.overlay, self.changes)
        revs = {"keeperfx": {"commit": "b" * 40, "date": "2026-11-12T10:00:00+00:00"},
                "wiki": {"commit": "c" * 40}}
        old_up = {"keeperfx": {"commit": "a" * 40, "date": "2026-11-10T10:00:00+00:00"},
                  "wiki": {"commit": "c" * 40}}
        text = sync.build_report(old_up, revs, [sync.TABLES, sync.OVERLAY], rep)
        self.assertIn("# Sync data with KeeperFX bbbbbbb", text)
        self.assertIn("compare/%s...%s" % ("a" * 40, "b" * 40), text)
        self.assertIn("added: `SET_FIXTURE_GLOW` (`PNn`)", text)
        self.assertIn("`ADD_GOLD_TO_PLAYER`: `PN` -> `PNn`", text)
        self.assertIn("removed: `FIXTURE_OLD_BEACON`", text)
        self.assertIn("`flag_desc`: added `FLAG2`", text)
        self.assertIn("**Review needed:**", text)
        self.assertIn("`SET_FIXTURE_GLOW`: new in KeeperFX abcdef1", text)

    def test_additive_change_is_not_flagged(self):
        added_only = json.loads(self.old_tables_text)
        added_only["commands"].append({"name": "SET_FIXTURE_GLOW", "args": "PNn     ",
                                       "cmd": "Cmd_SET_FIXTURE_GLOW", "check": None,
                                       "process": None})
        rep = sync.Report()
        sync.diff_commands(rep, self.old_tables, added_only)
        self.assertEqual(rep.risks, [])


class Hardening(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kfx-sync-test-")
        self.dir = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_staging_skips_symlinks_and_git(self):
        src = self.dir / "src"
        (src / "sub").mkdir(parents=True)
        (src / ".git").mkdir()
        (src / "a.c").write_text("int a;\n")
        (src / "sub" / "b.h").write_text("int b;\n")
        (src / ".git" / "config").write_text("[core]\n")
        secret = self.dir / "secret.h"
        secret.write_text("SECRET\n")
        try:
            os.symlink(secret, src / "link.h")
            os.symlink(self.dir, src / "linkdir")
        except (OSError, NotImplementedError):
            self.skipTest("no symbolic links here")
        n = sync.copy_tree(src, self.dir / "out", {".c", ".h"})
        got = sorted(p.relative_to(self.dir / "out").as_posix()
                     for p in (self.dir / "out").rglob("*") if p.is_file())
        self.assertEqual(got, ["a.c", "sub/b.h"])
        self.assertEqual(n, 2)

    def test_c_strings_are_decoded_not_evaluated(self):
        tables = load_tool("extract_script_tables")
        s = '"A\\"B\\\\C\\n"'
        self.assertEqual(tables.c_string(s, 0, len(s), "t"), 'A"B\\C\n')
        for bad in ('"a" + "b"', '"\\x41"', '"__import__(\'os\')" "x"', "'a'"):
            with self.assertRaises(tables.ExtractError):
                tables.c_string(bad, 0, len(bad), "t")

    def test_constant_expressions_stay_bounded(self):
        schema = load_tool("extract_config_schema")
        root = self.dir / "src"
        root.mkdir()
        (root / "a.h").write_text("#define BIG (1 << 4000)\n#define TWO (1 + 1)\n"
                                  "#define HUGE (TWO * 99999999999999999999 * 99999999999)\n")
        consts = schema.Consts(root)
        self.assertEqual(consts.value("TWO"), 2)
        self.assertIsNone(consts.value("BIG"))
        self.assertIsNone(consts.value("HUGE"))
        self.assertIsNone(consts.eval("__import__('os').system('true')"))
        self.assertIsNone(consts.eval("(" * 5000 + "1" + ")" * 5000))

    def test_wiki_head_cannot_point_outside_git(self):
        docs = load_tool("extract_script_docs")
        wiki = self.dir / "wiki"
        (wiki / ".git").mkdir(parents=True)
        (self.dir / "outside").write_text("a" * 40 + "\n")
        (wiki / ".git" / "HEAD").write_text("ref: refs/../../../outside\n")
        self.assertEqual(docs.wiki_commit(wiki), "unknown")
        (wiki / ".git" / "HEAD").write_text("ref: refs/heads/master\n")
        (wiki / ".git" / "refs" / "heads").mkdir(parents=True)
        (wiki / ".git" / "refs" / "heads" / "master").write_text("not a hash\n")
        self.assertEqual(docs.wiki_commit(wiki), "unknown")
        (wiki / ".git" / "refs" / "heads" / "master").write_text("d" * 40 + "\n")
        self.assertEqual(docs.wiki_commit(wiki), "d" * 40)
        self.assertEqual(sync.git_head_from_files(wiki), "d" * 40)


if __name__ == "__main__":
    unittest.main()
