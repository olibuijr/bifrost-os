#!/usr/bin/env python3
"""The package no longer ships or depends on the retired third-party compositor.

Run: python3 -I tools/test_package_files.py
"""
import os
import unittest

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ARCH = os.path.join(HERE, "packaging", "arch")


def read(*parts):
    with open(os.path.join(*parts)) as f:
        return f.read()


class NoRetiredCompositor(unittest.TestCase):
    def test_package_files_script(self):
        self.assertNotIn("labwc", read(ARCH, "package-files.sh"))

    def test_runtime_packages(self):
        self.assertNotIn("labwc", read(ARCH, "runtime-packages.txt"))

    def test_archiso_profile_tool(self):
        self.assertNotIn("labwc", read(HERE, "tools", "apply-archiso-profile.py"))

    def test_config_files_removed(self):
        for name in ("rc.xml", "autostart", "environment", "themes"):
            self.assertFalse(os.path.exists(os.path.join(ARCH, "wayland", name)), name)

    def test_window_server_is_mandatory(self):
        self.assertNotIn("no PC_WM", read(ARCH, "package-files.sh"))

    def test_session_env_carries_toolkit_defaults(self):
        text = read(ARCH, "wayland", "session-env")
        for key in ("XDG_CURRENT_DESKTOP", "MOZ_ENABLE_WAYLAND", "QT_QPA_PLATFORM", "GDK_BACKEND", "XCURSOR_THEME", "XCURSOR_SIZE"):
            self.assertIn("'%s':" % key, text)


class RepositoryKey(unittest.TestCase):
    def test_package_ships_the_key_when_present_and_the_scriptlet_imports_it(self):
        files = read(ARCH, "package-files.sh")
        self.assertIn("keys/bifrost-repo.gpg", files)
        self.assertIn("/usr/share/bifrost/keys/bifrost-repo.gpg", files)
        scriptlet = read(ARCH, "springboard.install")
        self.assertIn("pacman-key --add", scriptlet)
        self.assertIn("pacman-key --lsign-key", scriptlet)
        self.assertTrue(os.path.exists(os.path.join(ARCH, "keys", "README.md")))


class AgentAccessIsLabOnly(unittest.TestCase):
    def test_unit_needs_the_lab_marker_and_listens_on_loopback_by_default(self):
        unit = read(ARCH, "bifrost-mcp.service")
        self.assertIn("ConditionPathExists=/etc/bifrost/lab-install", unit)
        self.assertNotIn("0.0.0.0", unit)
        self.assertIn("EnvironmentFile=-/etc/bifrost/mcp.env", unit)

    def test_scriptlet_enables_it_only_with_the_marker(self):
        text = read(ARCH, "springboard.install")
        self.assertNotIn("# agent access (docs/bifrost-mcp.md), on by default", text)
        self.assertIn("bifrost_mcp_policy", text)
        self.assertIn("systemctl disable --now bifrost-mcp.service", text)
        self.assertIn("05-lab.conf", text, "a lab machine installed before the marker keeps agent access")

    def test_lab_file_limits_peers_to_private_ranges(self):
        env = read(ARCH, "mcp-lab.env")
        self.assertIn("BIFROST_MCP_ALLOW=", env)
        self.assertIn("mcp-lab.env", read(ARCH, "package-files.sh"))


if __name__ == "__main__":
    unittest.main()
