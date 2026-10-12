#!/usr/bin/env python3
"""Unit tests for tools/parity/os_inventory.py index mode: fixture TSV lines, no reference image needed."""
import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location("os_inventory", HERE / "parity" / "os_inventory.py")
oi = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oi)


def rec(t, path, size=0):
    return f"{t}\t{size}\t{path}\n"


def one(t, path, size=0):
    """Entries the index walker records for a single fixture line (empty list when the line is not recorded)."""
    return oi.walk_index([rec(t, path, size)])


def repo_with(files=(), dirs=()):
    """A temporary repository root holding the given files (text) and empty directories."""
    root = tempfile.mkdtemp(prefix="os-inv-test-")
    for d in dirs:
        os.makedirs(os.path.join(root, d), exist_ok=True)
    for f, text in files:
        p = os.path.join(root, f)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8") as fh:
            fh.write(text)
    return root


class ParseIndexLine(unittest.TestCase):
    def test_header_blank_and_malformed_lines_are_skipped(self):
        self.assertIsNone(oi.parse_index_line("type\tsize\tpath\n"))
        self.assertIsNone(oi.parse_index_line("\n"))
        self.assertIsNone(oi.parse_index_line("X\t1\tSystem/Foo\n"))
        self.assertIsNone(oi.parse_index_line("F\t1\n"))
        self.assertIsNone(oi.parse_index_line("D\t0\t/\n"))

    def test_size_and_path_are_parsed(self):
        r = oi.parse_index_line("F\t802\tSystem/Library/LaunchDaemons/bootps.plist\n")
        self.assertEqual(r, {"type": "F", "size": 802, "path": "System/Library/LaunchDaemons/bootps.plist"})
        self.assertEqual(oi.parse_index_line("D\tnope\tSystem/Applications\n")["size"], 0)


class Classify(unittest.TestCase):
    def test_system_app_is_an_app_named_without_extension(self):
        (e,) = one("D", "System/Applications/Mail.app")
        self.assertEqual((e["category"], e["kind"], e["name"], e["path"]), ("apps", "app", "Mail", "/System/Applications/Mail.app"))

    def test_nested_bundles_are_not_top_level_components(self):
        self.assertEqual(one("D", "System/Library/Frameworks/MPS.framework/Frameworks/MPSNDArray.framework"), [])
        self.assertEqual(one("F", "System/Applications/Mail.app/Contents/Info.plist"), [])

    def test_launch_plist_name_drops_suffix(self):
        (e,) = one("F", "System/Library/LaunchDaemons/bootps.plist", 802)
        self.assertEqual((e["category"], e["kind"], e["name"]), ("daemons", "launchd plist", "bootps"))
        self.assertEqual(e["facts"]["size"], 802)

    def test_launch_agents_are_daemons_too(self):
        (e,) = one("F", "System/Library/LaunchAgents/com.apple.Dock.plist")
        self.assertEqual((e["category"], e["name"]), ("daemons", "com.apple.Dock"))

    def test_public_and_private_frameworks(self):
        (pub,) = one("D", "System/Library/Frameworks/AGL.framework")
        (priv,) = one("D", "System/Library/PrivateFrameworks/CoreRE.framework")
        self.assertEqual((pub["category"], pub["kind"], pub["name"]), ("frameworks", "public framework", "AGL"))
        self.assertEqual((priv["category"], priv["kind"], priv["name"]), ("frameworks", "private framework", "CoreRE"))

    def test_cli_tools_in_bin_directories(self):
        (e,) = one("F", "usr/bin/codesign")
        self.assertEqual((e["category"], e["kind"], e["name"]), ("cli", "tool", "codesign"))
        self.assertEqual(one("F", "usr/local/bin/tool"), [])

    def test_kernel_extensions_and_driver_extensions(self):
        (k,) = one("D", "System/KernelKit/System/Library/Extensions/Libm.kext")
        (d,) = one("D", "System/Library/DriverExtensions/AppleCentauriAlpha.dext")
        self.assertEqual((k["category"], k["kind"], k["name"]), ("drivers", "kext", "Libm"))
        self.assertEqual((d["category"], d["kind"], d["name"]), ("drivers", "dext", "AppleCentauriAlpha"))

    def test_metal_library_owner_is_the_framework(self):
        (e,) = one("F", "System/Library/Frameworks/CoreImage.framework/Versions/A/Resources/ci_uberwrapper_bin.metallib", 9)
        self.assertEqual((e["category"], e["kind"], e["owner"]), ("shaders", "metal library", "CoreImage"))

    def test_fonts_and_wallpapers_are_assets(self):
        (f,) = one("F", "System/Library/Fonts/Helvetica.ttc")
        (w,) = one("F", "System/Library/Desktop Pictures/Dunes.heic")
        self.assertEqual((f["category"], f["kind"]), ("assets", "font"))
        self.assertEqual((w["category"], w["kind"]), ("assets", "wallpaper"))
        self.assertEqual(one("F", "System/Library/Frameworks/Foo.framework/Resources/icon.icns"), [])

    def test_settings_extensions_and_panes(self):
        (nested,) = one("D", "System/Applications/System Settings.app/Contents/PlugIns/SettingsImportExtension.appex")
        (about,) = one("D", "System/Library/ExtensionKit/Extensions/AboutExtension.appex")
        (av,) = one("D", "System/Library/ExtensionKit/Extensions/AVCPlugin.appex")
        (pane,) = one("D", "System/Library/PreferencePanes/Network.prefPane")
        self.assertEqual((nested["category"], nested["owner"]), ("settings", "System Settings"))
        self.assertEqual(about["category"], "settings")
        self.assertEqual((av["category"], av["kind"]), ("system-ui", "system extension"))
        self.assertEqual((pane["category"], pane["kind"], pane["name"]), ("settings", "settings pane", "Network"))

    def test_core_services_agents_and_shell_apps(self):
        (agent,) = one("D", "System/Library/CoreServices/WallpaperAgent.app")
        (finder,) = one("D", "System/Library/CoreServices/Finder.app")
        (cs_app,) = one("D", "System/Library/CoreServices/Applications/Archive Utility.app")
        self.assertEqual((agent["category"], agent["kind"]), ("system-ui", "core service agent"))
        self.assertEqual(finder["category"], "system-ui")
        self.assertEqual((cs_app["category"], cs_app["kind"]), ("apps", "core service app"))

    def test_input_methods_and_keyboard_layouts(self):
        (im,) = one("D", "System/Library/Input Methods/Pinyin.app")
        (kl,) = one("D", "System/Library/Keyboard Layouts/Dvorak.bundle")
        self.assertEqual((im["category"], im["kind"]), ("input-methods", "input method"))
        self.assertEqual((kl["category"], kl["kind"]), ("input-methods", "keyboard layout"))

    def test_unrecorded_paths_give_nothing(self):
        self.assertEqual(one("F", "System/Library/Frameworks/AGL.framework/Versions/A/AGL"), [])
        self.assertEqual(one("D", "private/var/db"), [])


class Heuristics(unittest.TestCase):
    def assessed(self, lines, repo):
        return oi.assess(oi.walk_index(lines), repo)

    def test_app_with_a_matching_springboard_app_is_partial(self):
        repo = repo_with(dirs=["springboard/apps/calculator-desktop"])
        (e,) = self.assessed([rec("D", "System/Applications/Calculator.app")], repo)
        self.assertEqual((e["status"], e["bifrost"], e["method"]), ("partial", "springboard/apps/calculator-desktop", "heuristic"))

    def test_alias_counterpart_needs_the_file_to_exist(self):
        present = repo_with(files=[("springboard/apps/files/main.rs", "")])
        absent = repo_with()
        (hit,) = self.assessed([rec("D", "System/Library/CoreServices/Finder.app")], present)
        (miss,) = self.assessed([rec("D", "System/Library/CoreServices/Finder.app")], absent)
        self.assertEqual((hit["status"], hit["bifrost"]), ("partial", "springboard/apps/files"))
        self.assertEqual((miss["status"], miss["bifrost"]), ("gap", ""))

    def test_unknown_app_is_a_gap_without_a_citation(self):
        (e,) = self.assessed([rec("D", "System/Applications/Journal.app")], repo_with())
        self.assertEqual((e["status"], e["bifrost"]), ("gap", ""))

    def test_settings_pane_found_in_the_settings_specs_is_partial(self):
        repo = repo_with(files=[("springboard/src/shell/pages.rs", "const PAGES: &[&str] = &[\"Network\"];")])
        (e,) = self.assessed([rec("D", "System/Library/PreferencePanes/Network.prefPane")], repo)
        self.assertEqual((e["status"], e["bifrost"]), ("partial", "springboard/src/shell/pages.rs"))

    def test_settings_pane_not_found_names_the_candidate_home(self):
        (e,) = self.assessed([rec("D", "System/Library/PreferencePanes/Gizmo.prefPane")], repo_with())
        self.assertEqual((e["status"], e["bifrost"]), ("gap", ""))

    def test_daemons_are_os_unless_user_facing(self):
        (sys_d,) = self.assessed([rec("F", "System/Library/LaunchDaemons/bootps.plist")], repo_with())
        (ui_d,) = self.assessed([rec("F", "System/Library/LaunchAgents/com.apple.Spotlight.plist")], repo_with())
        self.assertEqual(sys_d["status"], "os")
        self.assertEqual(ui_d["status"], "gap")

    def test_private_frameworks_are_os_and_public_cite_their_data_file(self):
        repo = repo_with(files=[("docs/parity-matrix/data/coreimage.json", "{}")])
        (priv,) = self.assessed([rec("D", "System/Library/PrivateFrameworks/CoreRE.framework")], repo)
        (pub,) = self.assessed([rec("D", "System/Library/Frameworks/CoreImage.framework")], repo)
        self.assertEqual(priv["status"], "os")
        self.assertEqual((pub["status"], pub["bifrost"]), ("gap", "docs/parity-matrix/data/coreimage.json"))

    def test_apple_hardware_driver_is_n_a_and_cli_split(self):
        (drv,) = self.assessed([rec("D", "System/Library/DriverExtensions/AppleCentauriAlpha.dext")], repo_with())
        (tool,) = self.assessed([rec("F", "usr/bin/ls")], repo_with())
        (apple_tool,) = self.assessed([rec("F", "usr/bin/codesign")], repo_with())
        self.assertEqual(drv["status"], "n/a")
        self.assertEqual(tool["status"], "os")
        self.assertEqual(apple_tool["status"], "gap")

    def test_every_status_is_valid_and_cited_paths_exist(self):
        repo = repo_with(files=[("springboard/src/shell/pages.rs", "x"), ("springboard/src/shell/kb_layouts.rs", "x")])
        lines = [rec("D", "System/Library/PreferencePanes/Network.prefPane"), rec("D", "System/Library/Input Methods/A.app"),
                 rec("D", "System/Library/Keyboard Layouts/B.bundle"), rec("F", "System/Library/Fonts/X.ttf")]
        for e in self.assessed(lines, repo):
            self.assertIn(e["status"], oi.STATUSES)
            if e["bifrost"]:
                self.assertTrue(os.path.exists(os.path.join(repo, e["bifrost"])), e["bifrost"])


class Merge(unittest.TestCase):
    def test_a_person_assessment_survives_a_rerun(self):
        old = [{"id": "path:/x", "status": "os", "bifrost": "springboard/x", "note": "kept", "description": "d"}]
        fresh = oi.walk_index([rec("F", "usr/bin/ls")])
        fresh[0]["id"] = "path:/x"
        (merged,) = oi.assess(oi.merge(old, fresh), repo_with())
        self.assertEqual((merged["status"], merged["bifrost"], merged["note"], merged["description"]),
                         ("os", "springboard/x", "kept", "d"))
        self.assertNotIn("method", merged)

    def test_a_heuristic_entry_is_re_guessed_but_keeps_its_description(self):
        old = [{"id": "path:/x", "status": "gap", "method": "heuristic", "bifrost": "stale", "description": "d"}]
        fresh = oi.walk_index([rec("F", "usr/bin/ls")])
        fresh[0]["id"] = "path:/x"
        (merged,) = oi.assess(oi.merge(old, fresh), repo_with())
        self.assertEqual((merged["status"], merged["bifrost"], merged["description"]), ("os", "", "d"))

    def test_assess_leaves_person_status_alone(self):
        e = oi.entry("cli", "cli:/usr/bin/ls", "/usr/bin/ls", "tool", "ls", status="partial", bifrost="springboard/x")
        (out,) = oi.assess([e], repo_with())
        self.assertEqual((out["status"], out["bifrost"]), ("partial", "springboard/x"))


class IndexFile(unittest.TestCase):
    def test_walk_index_reads_lines_and_ids_are_unique(self):
        lines = ["type\tsize\tpath\n", rec("D", "System/Applications/Mail.app"), rec("D", "System/Applications/Mail.app/Contents"),
                 rec("F", "usr/bin/ls", 5), rec("F", "System/Library/Fonts/X.otf", 7), "garbage\n"]
        entries = oi.walk_index(lines)
        ids = [e["id"] for e in entries]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertEqual(sorted(e["category"] for e in entries), ["apps", "assets", "cli"])
        self.assertTrue(all(e["path"].startswith("/") for e in entries))

    def test_shard_names_for_the_output_files(self):
        self.assertEqual(oi.shard_of("Mail"), "m-o")
        self.assertEqual(oi.shard_of("3DKit"), "0-9")
        self.assertEqual(oi.shard_of("_hidden"), "0-9")


if __name__ == "__main__":
    unittest.main()
