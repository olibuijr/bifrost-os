#!/usr/bin/env python3
"""Unit tests for tools/parity/os_cli.py: map loading, probes, result parsing, row updates."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("os_cli", HERE / "os_cli.py")
oc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oc)

M = {"commands": {"bsdtar": "tar", "afplay": None},
     "no_exec": {"names": ["reboot"]},
     "na_names": {"x": {"reason": "Xcode shim", "names": ["xcrun"]}},
     "na_patterns": [{"reason": "daemon", "path_prefix": "/usr/libexec/", "name_regex": "d$"}]}


def row(name, path="/usr/bin/"):
    return {"id": f"path:{path}{name}", "name": name, "path": path + name, "status": "os", "bifrost": "",
            "note": "heuristic: POSIX tool, Linux base", "method": "heuristic"}


class MapTests(unittest.TestCase):
    def test_shipped_map_loads(self):
        m = oc.load_map()
        self.assertIn("commands", m)
        self.assertIsNone(oc.command_for("afplay", m))

    def test_command_rename_default_and_null(self):
        self.assertEqual(oc.command_for("bsdtar", M), "tar")
        self.assertEqual(oc.command_for("cat", M), "cat")
        self.assertIsNone(oc.command_for("afplay", M))

    def test_na_name_and_pattern(self):
        self.assertEqual(oc.na_reason("xcrun", "/usr/bin/xcrun", M), "Xcode shim")
        self.assertEqual(oc.na_reason("food", "/usr/libexec/food", M), "daemon")
        self.assertIsNone(oc.na_reason("food", "/usr/bin/food", M))
        self.assertIsNone(oc.na_reason("cat", "/bin/cat", M))


class ProbeTests(unittest.TestCase):
    def test_probe_runs_version_or_help(self):
        p = oc.probe_for("cat", M)
        self.assertIn("command -v cat", p)
        self.assertIn("--version", p)
        self.assertIn("--help", p)

    def test_no_exec_never_runs_the_tool(self):
        p = oc.probe_for("reboot", M)
        self.assertNotIn("--version", p)
        self.assertIn("PRESENT", p)

    def test_plan_skips_na_and_null(self):
        rows = [row("cat"), row("xcrun"), row("afplay"), row("bsdtar")]
        plan = oc.build_plan(rows, M)
        self.assertEqual([p["command"] for p in plan], ["cat", "tar"])
        self.assertEqual(plan[0]["id"], "path:/usr/bin/cat")


class ResultTests(unittest.TestCase):
    def test_parse_output(self):
        plan = [{"id": "a"}, {"id": "b"}, {"id": "c"}]
        text = "noise\n@@0\tPASS\t/usr/bin/a\n@@1\tMISSING\t\n@@9\tPASS\t/x\n"
        r = oc.parse_output(text, plan)
        self.assertEqual(r["a"], {"state": "PASS", "path": "/usr/bin/a"})
        self.assertEqual(r["b"]["state"], "MISSING")
        self.assertEqual(r["c"]["state"], "FAIL")

    def test_batch_script_has_one_line_per_probe(self):
        plan = oc.build_plan([row("cat"), row("ls")], M)
        self.assertEqual(oc.batch_script(plan).count("printf '@@%s"), 2)


class UpdateTests(unittest.TestCase):
    def check(self, name, result, status, path="/usr/bin/"):
        e = row(name, path)
        self.assertEqual(oc.apply_result(e, M, result, "0.7.0"), status)
        self.assertEqual(e["method"], "assessed")
        return e

    def test_parity(self):
        e = self.check("cat", {"state": "PASS", "path": "/usr/bin/cat"}, "parity")
        self.assertEqual(e["bifrost"], "/usr/bin/cat")
        self.assertIn("verified on Midget 0.7.0", e["note"])

    def test_partial_fail_and_present(self):
        self.check("cat", {"state": "FAIL", "path": "/usr/bin/cat"}, "partial")
        self.check("reboot", {"state": "PRESENT", "path": "/usr/bin/reboot"}, "partial")

    def test_gap_missing_and_null(self):
        self.check("cat", {"state": "MISSING", "path": ""}, "gap")
        self.assertEqual(self.check("afplay", None, "gap")["bifrost"], "")

    def test_na_has_reason(self):
        self.assertEqual(self.check("xcrun", None, "n/a")["note"], "Xcode shim")

    def test_closures_rewrites_shards(self):
        d = tempfile.mkdtemp(prefix="os-cli-test-")
        Path(d, "a.json").write_text(json.dumps({"category": "cli", "shard": "a", "entries": [row("cat")]}))
        orig = oc.load_map
        oc.load_map = lambda: M
        try:
            counts = oc.closures({"path:/usr/bin/cat": {"state": "PASS", "path": "/bin/cat"}}, "0.7.0", directory=d)
        finally:
            oc.load_map = orig
        self.assertEqual(counts, {"parity": 1})
        self.assertEqual(json.loads(Path(d, "a.json").read_text())["entries"][0]["status"], "parity")


if __name__ == "__main__":
    unittest.main()
