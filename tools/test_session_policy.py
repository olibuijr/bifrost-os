#!/usr/bin/env python3
"""Tests for packaging/arch/wayland/session-policy (the session script's pure decisions).

Run: python3 -I tools/test_session_policy.py
"""
import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
POLICY = os.path.join(HERE, "packaging", "arch", "wayland", "session-policy")


def run_policy(call, *args):
    quoted = " ".join("'" + a.replace("'", "'\\''") + "'" for a in args)
    proc = subprocess.run(
        ["bash", "-c", f"set -euo pipefail; source '{POLICY}'; {call} {quoted}"],
        capture_output=True, text=True, check=True,
    )
    return proc.stdout


def flags(compositor, renderer, chrome):
    out = run_policy("session_compositor_flags", compositor, renderer, chrome)
    return [line for line in out.splitlines() if line]


class SessionCompositorFlags(unittest.TestCase):
    def test_labwc_gets_no_flags(self):
        self.assertEqual(flags("labwc", "vulkan", "1"), [])

    def test_empty_compositor_gets_no_flags(self):
        self.assertEqual(flags("", "vulkan", "1"), [])

    def test_bifrost_wm_alone_selects_direct_apps(self):
        self.assertEqual(flags("bifrost-wm", "", ""), ["BIFROST_DIRECT=1"])

    def test_vulkan_renderer_adds_the_renderer_word(self):
        self.assertEqual(flags("bifrost-wm", "vulkan", ""),
                         ["BIFROST_DIRECT=1", "BIFROST_WM_RENDERER=vulkan"])

    def test_chrome_switch_adds_chrome_word(self):
        self.assertEqual(flags("bifrost-wm", "gles", "1"), ["BIFROST_DIRECT=1", "BIFROST_CHROME=1"])

    def test_full_switch_sets_all_three(self):
        self.assertEqual(flags("bifrost-wm", "vulkan", "1"),
                         ["BIFROST_DIRECT=1", "BIFROST_WM_RENDERER=vulkan", "BIFROST_CHROME=1"])

    def test_chrome_zero_is_off(self):
        self.assertNotIn("BIFROST_CHROME=1", flags("bifrost-wm", "vulkan", "0"))


class WmExitAction(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.file = os.path.join(self.dir.name, "wm-exits")

    def tearDown(self):
        self.dir.cleanup()

    def write(self, text):
        with open(self.file, "w") as f:
            f.write(text)

    def read(self):
        with open(self.file) as f:
            return [line.strip() for line in f if line.strip()]

    def test_first_exit_restarts_and_is_recorded(self):
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "restart")
        self.assertEqual(self.read(), ["1000"])

    def test_exit_older_than_60_seconds_is_forgotten(self):
        self.write("900\n980\n")  # 900 is 100 s old, 980 is 20 s old
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "restart")
        self.assertEqual(self.read(), ["980", "1000"])

    def test_exit_exactly_60_seconds_old_is_forgotten(self):
        self.write("940\n970\n")  # 940 is exactly 60 s old: not within 60 s
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "restart")
        self.assertEqual(self.read(), ["970", "1000"])

    def test_third_exit_within_60_seconds_falls_back(self):
        self.write("950\n970\n")
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "fallback")
        self.assertEqual(self.read(), ["950", "970", "1000"])

    def test_three_spread_out_exits_do_not_fall_back(self):
        self.write("100\n500\n")
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "restart")

    def test_garbage_lines_are_ignored(self):
        self.write("junk\n970\n\n")
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "restart")
        self.assertEqual(self.read(), ["970", "1000"])


if __name__ == "__main__":
    unittest.main()
