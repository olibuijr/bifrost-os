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


class ShippedMapTests(unittest.TestCase):
    """Rows of the real map: macOS-internal daemons are n/a with a specific reason, user tools map to Linux names."""

    @classmethod
    def setUpClass(cls):
        cls.m = oc.load_map()

    def test_internal_daemons_are_na_with_reason(self):
        for name, path, word in [("ASPCarryLog", "/usr/libexec/", "macOS"), ("IOMFB_bics_daemon", "/usr/libexec/", "Apple"),
                                 ("MobileStorageMounter", "/usr/libexec/", "BifrOSt"), ("kextcache", "/usr/sbin/", "kmod"),
                                 ("mDNSResponderHelper", "/usr/libexec/", "netd"), ("DumpPanic", "/usr/libexec/", "coredump"),
                                 ("BootCacheControl", "/usr/sbin/", "BifrOSt")]:
            reason = oc.na_reason(name, path + name, self.m)
            self.assertTrue(reason, name)
            self.assertIn(word, reason, name)

    def test_dtrace_scripts_are_na(self):
        self.assertIn("DTrace", oc.na_reason("loads.d", "/usr/bin/loads.d", self.m))
        self.assertIn("DTrace", oc.na_reason("opensnoop", "/usr/bin/opensnoop", self.m))

    def test_pasteboard_daemon_points_at_pbcopy(self):
        self.assertIn("pbcopy", oc.na_reason("pboard", "/usr/libexec/pboard", self.m))

    def test_renamed_equivalents(self):
        for name, cmd in {"md5": "md5sum", "jot": "seq", "ifconfig": "ip", "route": "ip", "netstat": "ss", "PlistBuddy": "plutil",
                          "dns-sd": "avahi-browse", "mDNSResponder": "avahi-daemon", "dtruss": "strace", "vm_stat": "vmstat",
                          "xattr": "getfattr", "newsyslog": "logrotate", "sandbox-exec": "bwrap", "locate.updatedb": "updatedb",
                          "getty": "agetty", "tftpd": "in.tftpd", "afplay": "aplay", "audit": "auditctl"}.items():
            self.assertEqual(oc.command_for(name, self.m), cmd, name)

    def test_explicit_mapping_beats_generic_daemon_pattern(self):
        # snmpd, slapd, atrun, getty, bootpd end in "d" under /usr/libexec but are real Linux services
        for name in ("snmpd", "slapd", "atrun", "getty", "bootpd", "dhcp6d", "tftpd", "ippusbd", "rpc.rquotad"):
            self.assertIsNone(oc.na_reason(name, "/usr/libexec/" + name, self.m), name)

    def test_name_classes_still_win_over_commands(self):
        self.assertIn("Xcode", oc.na_reason("xcrun", "/usr/bin/xcrun", self.m))

    def test_common_tools_stay_mapped(self):
        for name in ("less", "lsof", "jq", "dig", "bc", "ed", "cpio", "zip", "unzip", "rsync", "iperf3-darwin", "crontab", "lpstat", "ldapsearch"):
            self.assertIsNone(oc.na_reason(name, "/usr/bin/" + name, self.m), name)
            self.assertTrue(oc.command_for(name, self.m), name)

    def test_gaps_have_no_command(self):
        for name in ("osascript", "units", "banner", "lastcomm"):
            self.assertIsNone(oc.command_for(name, self.m), name)

    def test_every_renamed_or_package_command_is_listed_in_packages(self):
        listed = set()
        for line in (HERE / "os_cli_packages.txt").read_text().splitlines():
            if line.startswith("# ") and ": " in line:
                listed.update(t.split("->")[0] for t in line[2:].split(": ", 1)[1].split(", "))
        for name in ("less", "lsof", "jq", "dig", "mDNSResponder", "dtruss", "lpstat", "snmpget", "unzip"):
            self.assertIn(name, listed, name)

    def test_no_name_is_both_na_and_gap(self):
        classes = [c for c in self.m["na_names"].values() if isinstance(c, dict)]
        for name, cmd in self.m["commands"].items():
            if cmd is None:
                continue
            for c in classes:
                self.assertNotIn(name, c["names"], name)

    def test_batch_script_extends_path_for_perl_scripts(self):
        self.assertIn("core_perl", oc.batch_script([]))


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
