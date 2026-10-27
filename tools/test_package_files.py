#!/usr/bin/env python3
"""The package no longer ships or depends on the retired third-party compositor.

Run: python3 -I tools/test_package_files.py
"""
import os
import subprocess
import tempfile
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
        for key in ("XDG_CURRENT_DESKTOP", "MOZ_ENABLE_WAYLAND", "QT_QPA_PLATFORM", "QT_QPA_PLATFORMTHEME", "GDK_BACKEND", "XCURSOR_THEME", "XCURSOR_SIZE"):
            self.assertIn("'%s':" % key, text)

    def test_launchers_pass_the_qt_theme(self):
        # Qt follows the Settings portal's color-scheme only with its GNOME theme
        self.assertIn("'QT_QPA_PLATFORMTHEME': 'gnome'", read(ARCH, "wayland", "session-env"))
        self.assertIn("'QT_QPA_PLATFORMTHEME'", read(ARCH, "wayland", "apps"))


class RepositoryKey(unittest.TestCase):
    def test_package_ships_the_key_when_present_and_the_scriptlet_imports_it(self):
        files = read(ARCH, "package-files.sh")
        self.assertIn("keys/bifrost-repo.gpg", files)
        self.assertIn("/usr/share/bifrost/keys/bifrost-repo.gpg", files)
        scriptlet = read(ARCH, "springboard.install")
        self.assertIn("pacman-key --add", scriptlet)
        self.assertIn("pacman-key --lsign-key", scriptlet)
        self.assertTrue(os.path.exists(os.path.join(ARCH, "keys", "README.md")))

    def test_the_committed_public_key_is_an_armored_public_key(self):
        key = read(ARCH, "keys", "bifrost-repo.gpg")
        self.assertTrue(key.startswith("-----BEGIN PGP PUBLIC KEY BLOCK-----"))
        self.assertNotIn("PRIVATE", key)

    def run_siglevel(self, conf):
        """Run the scriptlet's bifrost_repo_siglevel on `conf` text; return the new text."""
        scriptlet = read(ARCH, "springboard.install")
        start = scriptlet.index("bifrost_repo_siglevel() {")
        end = scriptlet.index("\n}\n", start) + 3
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "pacman.conf")
            with open(path, "w") as f:
                f.write(conf)
            script = scriptlet[start:end] + "bifrost_repo_siglevel\nbifrost_repo_siglevel\n"
            subprocess.run(["bash", "-c", script], check=True, env={**os.environ, "BIFROST_PACMAN_CONF": path})
            return read(path)

    def test_an_old_install_is_switched_to_required_signatures_after_the_key_is_in(self):
        old = ("[options]\nSigLevel = Required DatabaseOptional\n\n[core]\nInclude = /etc/pacman.d/mirrorlist\n\n"
               "[bifrost]\nSigLevel = Optional TrustAll\nServer = https://bifrost.olibuijr.com/arch/$arch\n\n[extra]\nInclude = /x\n")
        new = self.run_siglevel(old)
        self.assertEqual(new, old.replace("Optional TrustAll", "Required DatabaseRequired"), "only the [bifrost] line changes, once")

    def test_a_bifrost_section_without_a_siglevel_line_gets_one_and_the_last_section_works(self):
        new = self.run_siglevel("[options]\nSigLevel = Never\n[bifrost]\nServer = https://r/$arch\n")
        self.assertEqual(new, "[options]\nSigLevel = Never\n[bifrost]\nServer = https://r/$arch\nSigLevel = Required DatabaseRequired\n")
        same = "[bifrost]\nSigLevel = Required DatabaseRequired\nServer = s\n"
        self.assertEqual(self.run_siglevel(same), same)

    def test_the_switch_happens_only_after_the_key_import_succeeded(self):
        scriptlet = read(ARCH, "springboard.install")
        trust = scriptlet[scriptlet.index("bifrost_repo_trust() {"):]
        trust = trust[:trust.index("\n}\n")]
        self.assertLess(trust.index("if bifrost_repo_key"), trust.index("bifrost_repo_siglevel"))
        for hook in ("post_install() {", "post_upgrade() {"):
            body = scriptlet[scriptlet.index(hook):]
            self.assertIn("bifrost_repo_trust", body[:body.index("\n}\n")], hook)


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
