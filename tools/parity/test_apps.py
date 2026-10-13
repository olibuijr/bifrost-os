#!/usr/bin/env python3
"""Unit tests for the per-app inventories in tools/parity/parity.py: coverage, ordering, --gaps and the check rules."""
import argparse
import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import parity  # noqa: E402


def row(feature, status, bifrost="", wire=""):
    return {"feature": feature, "area": "a", "apple": "x", "status": status, "bifrost": bifrost, "note": "n", "wire": wire}


def app(name, rows, bifrost_app="demo-desktop"):
    return {"app": name, "bifrost_app": bifrost_app, "evidence": ["e"], "features": rows}


class Tree:
    """A temporary repo root holding docs/parity-matrix/apps/<slug>.json files, wired into parity for one test."""

    def __init__(self, apps, files=()):
        self.root = tempfile.mkdtemp(prefix="parity-apps-")
        self.dir = os.path.join(self.root, "docs", "parity-matrix", "apps")
        os.makedirs(self.dir)
        for slug, data in apps.items():
            with open(os.path.join(self.dir, slug + ".json"), "w") as fh:
                fh.write(data if isinstance(data, str) else json.dumps(data))
        for rel in files:
            p = Path(self.root, rel)
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("x")

    def __enter__(self):
        self.patches = [mock.patch.object(parity, "APPS", self.dir), mock.patch.object(parity, "ROOT", self.root)]
        for p in self.patches:
            p.start()
        return self

    def __exit__(self, *exc):
        for p in self.patches:
            p.stop()


def run(**kw):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        parity.cmd_apps(argparse.Namespace(**{"gaps": None, "json": False, **kw}))
    return out.getvalue()


class Coverage(unittest.TestCase):
    def test_parity_one_partial_half_gap_zero_and_na_left_out(self):
        counts, cov = parity.app_counts(app("A", [row("1", "parity"), row("2", "partial", wire="w"), row("3", "gap", wire="w"), row("4", "n/a")]))
        self.assertEqual(counts, {"parity": 1, "partial": 1, "gap": 1, "n/a": 1})
        self.assertEqual(cov, 50.0)

    def test_an_app_with_only_na_rows_has_no_coverage(self):
        self.assertIsNone(parity.app_counts(app("A", [row("1", "n/a")]))[1])

    def test_apps_are_listed_by_coverage_ascending(self):
        data = {
            "high": app("High", [row("1", "parity")]),
            "low": app("Low", [row("1", "gap", wire="w")]),
            "mid": app("Mid", [row("1", "partial", wire="w")]),
        }
        with Tree(data):
            self.assertEqual([r["app"] for r in parity.app_summaries()], ["Low", "Mid", "High"])
            lines = run().strip().splitlines()
            self.assertTrue(lines[1].startswith("Low"))
            self.assertIn("0.0%", lines[1])
            self.assertTrue(lines[3].startswith("High"))

    def test_json_output_carries_counts_and_coverage(self):
        with Tree({"a": app("A", [row("1", "parity"), row("2", "gap", wire="w")])}):
            rows = json.loads(run(json=True))
        self.assertEqual(rows[0]["slug"], "a")
        self.assertEqual(rows[0]["features"], 2)
        self.assertEqual(rows[0]["counts"]["gap"], 1)
        self.assertEqual(rows[0]["coverage"], 50.0)


class Gaps(unittest.TestCase):
    def setUp(self):
        self.tree = Tree({"find-my": app("Find My", [row("Done", "parity"), row("Locate", "gap", wire="Add the locate call"), row("Share", "partial", wire="Finish sharing")])})

    def test_gaps_lists_gap_and_partial_rows_with_wire_text(self):
        with self.tree:
            out = run(gaps="find my")
        self.assertIn("[gap] Locate", out)
        self.assertIn("wire: Add the locate call", out)
        self.assertIn("[partial] Share", out)
        self.assertNotIn("Done", out)
        self.assertLess(out.index("Locate"), out.index("Share"))

    def test_gaps_json(self):
        with self.tree:
            data = json.loads(run(gaps="find-my", json=True))
        self.assertEqual([r["feature"] for r in data["rows"]], ["Locate", "Share"])

    def test_an_unknown_app_exits(self):
        with self.tree, self.assertRaises(SystemExit):
            run(gaps="nothing")

    def test_a_name_matching_two_apps_exits(self):
        with Tree({"mail": app("Mail", [row("1", "gap", wire="w")]), "mailbox": app("Mailbox", [row("1", "gap", wire="w")])}):
            self.assertEqual(parity.find_app("mail")["slug"], "mail")  # exact beats substring
            with self.assertRaises(SystemExit):
                run(gaps="mai")


class Check(unittest.TestCase):
    def problems(self, data, files=()):
        with Tree({"a": data}, files):
            return parity.check_apps()

    def test_a_clean_file_has_no_problems(self):
        self.assertEqual(self.problems(app("A", [row("1", "parity", "docs/x.md:3"), row("2", "gap", wire="w")]), ["docs/x.md"]), [])

    def test_unknown_status_is_reported(self):
        self.assertTrue(any("status 'done'" in p for p in self.problems(app("A", [row("1", "done")]))))

    def test_missing_row_key_and_top_key(self):
        data = app("A", [{"feature": "1", "status": "parity"}])
        del data["evidence"]
        found = self.problems(data)
        self.assertTrue(any("missing key evidence" in p for p in found))
        self.assertTrue(any("1: missing key wire" in p for p in found))

    def test_gap_and_partial_need_wire_text(self):
        found = self.problems(app("A", [row("1", "gap"), row("2", "partial", wire="  "), row("3", "parity"), row("4", "n/a")]))
        self.assertEqual(len(found), 2)

    def test_cited_path_must_exist(self):
        found = self.problems(app("A", [row("1", "parity", "springboard/apps/none/src/lib.rs:9")]))
        self.assertTrue(any("cited path not found: springboard/apps/none/src/lib.rs" in p for p in found))

    def test_summary_must_match_the_rows(self):
        data = app("A", [row("1", "parity")])
        data["summary"] = {"features": 2, "parity": 1}
        self.assertTrue(any("summary does not match" in p for p in self.problems(data)))

    def test_unreadable_json(self):
        with Tree({"a": "{not json"}):
            self.assertTrue(any("unreadable" in p for p in parity.check_apps()))


class Repo(unittest.TestCase):
    def test_the_committed_inventories_pass_the_check(self):
        self.assertEqual(parity.check_apps(), [])


if __name__ == "__main__":
    unittest.main()
