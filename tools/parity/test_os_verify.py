#!/usr/bin/env python3
"""Unit tests for tools/parity/os_verify.py: check schema, plan grouping, libtest parsing, promote rewriting."""
import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location("os_verify", HERE / "os_verify.py")
ov = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ov)


def row(eid, status="partial", ready=None, checks=None, **kw):
    e = {"id": eid, "category": "assets", "name": eid, "status": status, "note": "n", "bifrost": "", "rea": []}
    if ready is not None:
        e["ready"] = ready
    if checks is not None:
        e["checks"] = checks
    e.update(kw)
    return e


def write_shard(root, cat, shard, entries):
    d = os.path.join(root, cat)
    os.makedirs(d, exist_ok=True)
    doc = {"category": cat, "shard": shard, "source": "test", "entries": entries}
    with open(os.path.join(d, shard + ".json"), "w", encoding="utf-8") as fh:
        json.dump(doc, fh, indent=1, ensure_ascii=False)
        fh.write("\n")


class CheckSchema(unittest.TestCase):
    def test_well_formed_checks_pass(self):
        good = [
            {"kind": "file", "path": "/usr/share/springboard/assets/x.raw"},
            {"kind": "cmd", "run": "foo --version", "expect_exit": 0},
            {"kind": "cmd", "run": "foo --help"},
            {"kind": "unit", "name": "bifrost-zram.service", "state": "enabled"},
            {"kind": "unit", "name": "bifrost-shell.service", "state": "active", "user": True},
            {"kind": "test", "crate": "springboard", "filter": "shell::services::speech::tests"},
        ]
        for c in good:
            self.assertEqual(ov.check_problems(c), [], c)

    def test_unknown_kind_and_non_object_are_errors(self):
        self.assertTrue(ov.check_problems({"kind": "http", "url": "x"}))
        self.assertTrue(ov.check_problems("file:/usr/bin/x"))

    def test_file_path_must_be_absolute(self):
        self.assertTrue(ov.check_problems({"kind": "file", "path": "usr/bin/x"}))
        self.assertTrue(ov.check_problems({"kind": "file"}))

    def test_cmd_needs_run_and_integer_exit(self):
        self.assertTrue(ov.check_problems({"kind": "cmd", "run": "  "}))
        self.assertTrue(ov.check_problems({"kind": "cmd", "run": "x", "expect_exit": True}))
        self.assertTrue(ov.check_problems({"kind": "cmd", "run": "x", "expect_exit": "0"}))

    def test_cmd_must_be_read_only_and_harmless(self):
        for bad in ["sudo x --version", "x && rm -rf /tmp/y", "curl http://e", "x > /etc/conf", "systemctl restart x",
                    "pacman -S x", "ssh host x"]:
            self.assertTrue(ov.check_problems({"kind": "cmd", "run": bad}), bad)
        self.assertEqual(ov.check_problems({"kind": "cmd", "run": "foo --version 2>&1"}), [])
        self.assertEqual(ov.check_problems({"kind": "cmd", "run": "foo -V >/dev/null 2>&1"}), [])

    def test_unit_state_and_user_flag(self):
        self.assertTrue(ov.check_problems({"kind": "unit", "name": "x.service", "state": "running"}))
        self.assertTrue(ov.check_problems({"kind": "unit", "name": "x.service", "state": "active", "user": "yes"}))
        self.assertTrue(ov.check_problems({"kind": "unit", "state": "active"}))

    def test_test_check_needs_crate_and_filter(self):
        self.assertTrue(ov.check_problems({"kind": "test", "crate": "springboard"}))
        self.assertTrue(ov.check_problems({"kind": "test", "filter": "a::b"}))

    def test_unknown_key_is_an_error(self):
        self.assertTrue(ov.check_problems({"kind": "file", "path": "/x", "expect": 1}))

    def test_entry_ready_needs_checks(self):
        self.assertTrue(ov.validate_entry(row("a", ready=True)))
        self.assertTrue(ov.validate_entry(row("a", ready=True, checks=[])))
        self.assertEqual(ov.validate_entry(row("a", ready=True, checks=[{"kind": "file", "path": "/x"}])), [])

    def test_entry_ready_must_be_bool(self):
        self.assertTrue(ov.validate_entry(row("a", ready="yes", checks=[{"kind": "file", "path": "/x"}])))

    def test_entry_ready_on_parity_or_na_is_an_error(self):
        chk = [{"kind": "file", "path": "/x"}]
        self.assertTrue(ov.validate_entry(row("a", status="parity", ready=True, checks=chk)))
        self.assertTrue(ov.validate_entry(row("a", status="n/a", ready=True, checks=chk)))

    def test_malformed_check_on_unready_row_is_still_an_error(self):
        self.assertTrue(ov.validate_entry(row("a", ready=False, checks=[{"kind": "unit", "name": "x", "state": "x"}])))

    def test_checks_must_be_a_list(self):
        self.assertTrue(ov.validate_entry(row("a", checks={"kind": "file", "path": "/x"})))


class PlanGrouping(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name
        write_shard(self.root, "assets", "a-c", [
            row("p:1", ready=True, checks=[{"kind": "file", "path": "/usr/share/a"},
                                           {"kind": "test", "crate": "springboard", "filter": "x::tests"}]),
            row("p:2", status="os", ready=True, checks=[{"kind": "test", "crate": "springboard", "filter": "x::tests"},
                                                        {"kind": "test", "crate": "bifrost-app", "filter": "sound"}]),
            row("p:3", status="partial", checks=[{"kind": "file", "path": "/x"}]),
            row("p:4", status="n/a", note="apple only"),
        ])
        write_shard(self.root, "cli", "a", [
            row("c:1", status="partial", ready=True, checks=[{"kind": "cmd", "run": "vis --help", "expect_exit": 0}]),
            row("c:2", status="gap", ready=False),
        ])

    def tearDown(self):
        self.tmp.cleanup()

    def test_only_ready_rows_are_planned(self):
        plan, problems = ov.build_plan(self.root)
        self.assertEqual(problems, [])
        self.assertEqual([r["id"] for r in plan["rows"]], ["p:1", "p:2", "c:1"])
        self.assertEqual(plan["rows"][0]["shard"], "assets/a-c")
        self.assertEqual(set(plan["rows"][0]), {"shard", "id", "name", "checks"})

    def test_test_filters_are_unique_and_sorted(self):
        plan, _ = ov.build_plan(self.root)
        self.assertEqual(plan["tests"], [
            {"crate": "bifrost-app", "filter": "sound"},
            {"crate": "springboard", "filter": "x::tests"},
        ])

    def test_schema_problems_come_back_and_block_the_plan(self):
        write_shard(self.root, "daemons", "x", [row("d:1", ready=True, checks=[{"kind": "unit", "name": "y"}])])
        plan, problems = ov.build_plan(self.root)
        self.assertTrue(any(p.startswith("d:1") for p in problems))
        self.assertNotIn("d:1", [r["id"] for r in plan["rows"]])

    def test_missing_dir_gives_empty_plan(self):
        plan, problems = ov.build_plan(os.path.join(self.root, "nope"))
        self.assertEqual((plan["rows"], plan["tests"], problems), ([], [], []))


class LibtestParsing(unittest.TestCase):
    OUT = """
running 3 tests
test shell::services::speech::tests::parses ... ok
test shell::services::speech::tests::empty ... FAILED
test shell::other::tests::skipped ... ignored, slow
test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out
"""

    def test_parse_reads_each_test_outcome(self):
        got = ov.parse_libtest(self.OUT)
        self.assertEqual(got, {
            "shell::services::speech::tests::parses": "ok",
            "shell::services::speech::tests::empty": "FAILED",
            "shell::other::tests::skipped": "ignored",
        })

    def test_summary_lines_are_not_tests(self):
        self.assertEqual(ov.parse_libtest("running 0 tests\ntest result: ok. 0 passed\n"), {})

    def test_filter_that_runs_no_tests_fails(self):
        out = ov.parse_libtest("running 1 test\ntest other::thing ... ok\n")
        res = ov.judge_filters(["shell::services::speech"], out)
        self.assertFalse(res["shell::services::speech"][0])
        self.assertIn("0 tests", res["shell::services::speech"][1])

    def test_zero_tests_with_cargo_failure_reports_the_cause(self):
        res = ov.judge_filters(["x"], {}, rc=101, detail="error[E0432]: unresolved import")
        self.assertFalse(res["x"][0])
        self.assertIn("cargo exit 101", res["x"][1])
        self.assertIn("E0432", res["x"][1])

    def test_each_filter_is_judged_on_its_own_tests(self):
        out = ov.parse_libtest(self.OUT)
        res = ov.judge_filters(["parses", "empty", "skipped"], out)
        self.assertTrue(res["parses"][0])
        self.assertFalse(res["empty"][0])
        self.assertFalse(res["skipped"][0])  # only ignored tests: nothing ran

    def test_filter_with_a_failing_test_fails_even_if_another_passed(self):
        out = ov.parse_libtest(self.OUT)
        ok, reason = ov.judge_filters(["shell::services::speech"], out)["shell::services::speech"]
        self.assertFalse(ok)
        self.assertIn("1 failed", reason)

    def test_passing_filter_reports_how_many_ran(self):
        out = ov.parse_libtest(self.OUT)
        ok, reason = ov.judge_filters(["parses"], out)["parses"]
        self.assertTrue(ok)
        self.assertEqual(reason, "1 passed")


class RunHelpers(unittest.TestCase):
    def test_ssh_target_wraps_argv(self):
        t = ov.Target("ssh:midget")
        argv = t.argv(["test", "-e", "/usr/share/x y"])
        self.assertEqual(argv[0], "ssh")
        self.assertIn("midget", argv)
        self.assertEqual(argv[-1], "test -e '/usr/share/x y'")

    def test_local_target_runs_argv_unchanged(self):
        self.assertEqual(ov.Target("local").argv(["systemctl", "is-active", "x"]), ["systemctl", "is-active", "x"])

    def test_bad_target_is_rejected(self):
        with self.assertRaises(SystemExit):
            ov.Target("ssh:")
        with self.assertRaises(SystemExit):
            ov.Target("vm")


class Promote(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name
        write_shard(self.root, "assets", "a-c", [
            row("p:1", ready=True, note="synthesised tone", checks=[{"kind": "file", "path": "/x"}]),
            row("p:2", ready=True, note="partial", checks=[{"kind": "file", "path": "/y"}]),
            row("p:3", status="os", note="keep"),
        ])
        self.path = os.path.join(self.root, "assets", "a-c.json")
        self.results = {
            "target": "ssh:midget",
            "rows": [
                {"shard": "assets/a-c", "id": "p:1", "name": "p:1", "pass": True,
                 "checks": [{"check": {"kind": "file", "path": "/x"}, "pass": True, "reason": ""}]},
                {"shard": "assets/a-c", "id": "p:2", "name": "p:2", "pass": False,
                 "checks": [{"check": {"kind": "file", "path": "/y"}, "pass": False, "reason": "/y is missing"}]},
            ],
        }

    def tearDown(self):
        self.tmp.cleanup()

    def entries(self):
        return {e["id"]: e for e in json.load(open(self.path, encoding="utf-8"))["entries"]}

    def test_passing_row_becomes_parity_with_note(self):
        lines = ov.promote(self.results, self.root, "0.9.1", "vm")
        self.assertTrue(lines[0].startswith("+ assets/a-c p:1"))
        e = self.entries()["p:1"]
        self.assertEqual(e["status"], "parity")
        self.assertNotIn("ready", e)
        self.assertEqual(e["note"], "synthesised tone Verified on the vm with BifrOSt 0.9.1 (os_verify).")

    def test_failing_row_keeps_status_and_records_reason(self):
        ov.promote(self.results, self.root, "0.9.1", "vm")
        e = self.entries()["p:2"]
        self.assertEqual(e["status"], "partial")
        self.assertIs(e["ready"], False)
        self.assertIn("/y is missing", e["ready_failed"])

    def test_unrelated_rows_are_untouched(self):
        ov.promote(self.results, self.root, "0.9.1", "vm")
        self.assertEqual(self.entries()["p:3"]["status"], "os")

    def test_dry_run_writes_nothing(self):
        before = open(self.path, encoding="utf-8").read()
        lines = ov.promote(self.results, self.root, "0.9.1", "vm", dry_run=True)
        self.assertEqual(len(lines), 2)
        self.assertEqual(open(self.path, encoding="utf-8").read(), before)

    def test_written_json_keeps_the_house_format(self):
        ov.promote(self.results, self.root, "0.9.1", "vm")
        text = open(self.path, encoding="utf-8").read()
        self.assertTrue(text.endswith("}\n"))
        self.assertIn('\n "entries": [', text)  # indent 1
        self.assertEqual(text, json.dumps(json.loads(text), indent=1, ensure_ascii=False) + "\n")

    def test_note_is_not_repeated_on_a_second_promote(self):
        ov.promote(self.results, self.root, "0.9.1", "vm")
        # the row is made ready again (a re-run of the same check) and promoted a second time
        doc = json.load(open(self.path, encoding="utf-8"))
        for e in doc["entries"]:
            if e["id"] == "p:1":
                e["ready"] = True
                e["status"] = "partial"
        with open(self.path, "w", encoding="utf-8") as fh:
            json.dump(doc, fh, indent=1, ensure_ascii=False)
        ov.promote(self.results, self.root, "0.9.1", "vm")
        self.assertEqual(self.entries()["p:1"]["note"].count("(os_verify)."), 1)

    def test_unknown_row_is_reported_not_created(self):
        self.results["rows"].append({"shard": "assets/a-c", "id": "missing", "name": "m", "pass": True, "checks": []})
        lines = ov.promote(self.results, self.root, "0.9.1", "vm")
        self.assertTrue(any(ln.startswith("? assets/a-c missing") for ln in lines))
        self.assertNotIn("missing", self.entries())


if __name__ == "__main__":
    unittest.main()
