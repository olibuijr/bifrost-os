#!/usr/bin/env python3
"""Unit tests for tools/parity/verify.py: test discovery, classification, run outcomes and the closure format."""
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import verify  # noqa: E402


def repo(files):
    """A temporary repo root with the given {relative path: text}; verify.SRC_ROOT is pointed at it."""
    root = tempfile.mkdtemp(prefix="verify-test-")
    for rel, text in files.items():
        p = Path(root, rel)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)
    return root


CARGO = '[package]\nname = "kit-ui"\nversion = "0.1.0"\n'
LOGIC = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() {}\n}\n"


class Discovery(unittest.TestCase):
    def test_tests_in_prefixes_the_module_chain(self):
        src = "#[test]\nfn top() {}\nmod tests {\n    #[test]\n    fn inner() {}\n    fn helper() {}\n}\n"
        self.assertEqual(verify.tests_in(src), ["top", "tests::inner"])

    def test_tests_in_skips_attributes_between_test_and_fn(self):
        self.assertEqual(verify.tests_in("#[test]\n#[ignore]\nfn slow() {}\n"), ["slow"])

    def test_cited_paths_handles_symbols_and_prose(self):
        b = "springboard/ui/src/a.rs (Scene); wired: springboard/src/shell/b.rs."
        self.assertEqual(verify.cited_paths(b), ["springboard/ui/src/a.rs", "springboard/src/shell/b.rs"])

    def test_module_path(self):
        with mock.patch.object(verify, "SRC_ROOT", "/r"):
            self.assertEqual(verify.module_path("/r/ui/src/a/b.rs", "ui"), ["a", "b"])
            self.assertEqual(verify.module_path("/r/ui/src/a/mod.rs", "ui"), ["a"])
            self.assertEqual(verify.module_path("/r/ui/src/lib.rs", "ui"), [])


class Planning(unittest.TestCase):
    def plan(self, files, bifrost):
        with mock.patch.object(verify, "SRC_ROOT", repo(files)):
            return verify.plan_row("slug", {"feature": "F", "bifrost": bifrost})

    def test_logic_with_unit_tests(self):
        r = self.plan({"ui/Cargo.toml": CARGO, "ui/src/calc.rs": LOGIC}, "ui/src/calc.rs")
        self.assertEqual((r["class"], r["crate"], r["test_filters"]), ("logic", "kit-ui", ["lib:calc::tests::adds"]))

    def test_untested_without_tests(self):
        r = self.plan({"ui/Cargo.toml": CARGO, "ui/src/calc.rs": "pub fn f() {}\n"}, "ui/src/calc.rs")
        self.assertEqual(r["class"], "untested")

    def test_ui_wins_over_tests(self):
        r = self.plan({"ui/Cargo.toml": CARGO, "ui/src/paint.rs": LOGIC}, "ui/src/paint.rs")
        self.assertEqual(r["class"], "ui")

    def test_hardware_from_use_lines(self):
        r = self.plan({"ui/Cargo.toml": CARGO, "ui/src/calc.rs": "use crate::alsa::Pcm;\n" + LOGIC}, "ui/src/calc.rs")
        self.assertEqual(r["class"], "hardware")

    def test_integration_tests_that_use_the_module(self):
        files = {"ui/Cargo.toml": CARGO, "ui/src/calc.rs": "pub fn f() {}\n",
                 "ui/tests/calc_it.rs": "use kit_ui::calc;\n#[test]\nfn works() {}\n",
                 "ui/tests/other.rs": "#[test]\nfn unrelated() {}\n"}
        r = self.plan(files, "ui/src/calc.rs")
        self.assertEqual((r["class"], r["test_filters"]), ("logic", ["calc_it:works"]))

    def test_missing_path_is_untested_without_crate(self):
        r = self.plan({}, "ui/src/gone.rs")
        self.assertEqual((r["class"], r["crate"], r["test_filters"]), ("untested", None, []))

    def test_summary_counts_and_gain(self):
        rows = [{"class": "logic"}, {"class": "logic"}, {"class": "ui"}]
        s = verify.summarize(rows, 100)
        self.assertEqual((s["classes"]["logic"], s["classes"]["ui"], s["coverage_gain_pct"]), (2, 1, 1.0))

    def test_build_plan_only_partial_rows_and_filters(self):
        entries = [{"slug": "a", "group": "G", "applicable": 2, "rows": [
            {"feature": "x", "status": "partial", "bifrost": ""}, {"feature": "y", "status": "gap"}]}]
        self.assertEqual(len(verify.build_plan(entries)["rows"]), 1)
        self.assertEqual(len(verify.build_plan(entries, group="other")["rows"]), 0)


class Running(unittest.TestCase):
    def test_parse_results(self):
        out = "test a::ok_one ... ok\ntest a::bad ... FAILED\ntest result: x\n"
        self.assertEqual(verify.parse_results(out), {"a::ok_one": True, "a::bad": False})

    def test_resolve_exact_suffix_and_ambiguous(self):
        listed = ["calc::tests::adds", "other::tests::adds"]
        self.assertEqual(verify.resolve("calc::tests::adds", listed), "calc::tests::adds")
        self.assertEqual(verify.resolve("calc::tests::adds", ["x::calc::tests::adds"]), "x::calc::tests::adds")
        self.assertIsNone(verify.resolve("tests::adds", listed))

    def test_row_result(self):
        row = {"slug": "s"}
        self.assertEqual(verify.row_result(row, {"f": True})["status"], "pass")
        self.assertEqual(verify.row_result(row, {"f": True, "g": None})["status"], "fail")
        self.assertEqual(verify.row_result(row, {"f": False})["failed"], ["f"])
        self.assertEqual(verify.row_result(row, {})["status"], "fail")


class Closures(unittest.TestCase):
    RESULTS = {"rows": [
        {"slug": "s", "feature": "A", "class": "logic", "status": "pass", "test_filters": ["lib:t1", "lib:t2"]},
        {"slug": "s", "feature": "B", "class": "logic", "status": "fail", "test_filters": ["lib:t3"]},
        {"slug": "s", "feature": "C", "class": "ui", "status": "skipped", "test_filters": ["lib:t4"]},
        {"slug": "s", "feature": "D", "class": "untested", "status": "skipped", "test_filters": []}]}

    def test_closure_format_for_apply(self):
        rows = {("s", "A"): {"bifrost": "springboard/ui/src/a.rs", "note": "old"}}
        self.assertEqual(verify.make_closures(self.RESULTS, rows, "0.7.25"), [{
            "slug": "s", "feature": "A", "status": "parity", "bifrost": "springboard/ui/src/a.rs",
            "note": "verified on Midget 0.7.25 by 2 tests."}])

    def test_never_promotes_ui_hardware_untested_or_failed(self):
        features = {c["feature"] for c in verify.make_closures(self.RESULTS, {}, "1")}
        self.assertEqual(features, {"A"})

    def test_manual_report_grouped_by_framework(self):
        md = verify.manual_report(self.RESULTS, {"s": "Contacts"})
        self.assertIn("## Contacts", md)
        self.assertIn("- B (logic, tests failed)", md)
        self.assertIn("- C (ui)", md)
        self.assertIn("- D (untested)", md)
        self.assertNotIn("- A", md)


if __name__ == "__main__":
    unittest.main()
