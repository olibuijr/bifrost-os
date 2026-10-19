#!/usr/bin/env python3
"""Tests for tools/stage-assets.py and the packaging allowlist. Files are generated here; nothing real is read.

Run: python3 tools/test_stage_assets.py
"""
import contextlib
import importlib.util
import io
import os
import stat
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
spec = importlib.util.spec_from_file_location("stage_assets", os.path.join(HERE, "stage-assets.py"))
stage_assets = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage_assets)

LIST = """
# comment
SFUI.ttf                 # trailing comment
fonts/Open-Variable.ttf
wallpapers/*.raw
icons/bifrost.*.raw
"""


def write_tree(root, rels):
    for rel in rels:
        path = os.path.join(root, *rel.split("/"))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(rel.encode())


def run_main(*argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        status = stage_assets.main(list(argv))
    return status, out.getvalue(), err.getvalue()


class Patterns(unittest.TestCase):
    def test_globs_stay_inside_one_path_segment(self):
        patterns = stage_assets.parse_allowlist(LIST)
        match = lambda rel: any(p.match(rel) for p in patterns)
        self.assertTrue(match("SFUI.ttf"))
        self.assertTrue(match("fonts/Open-Variable.ttf"))
        self.assertTrue(match("wallpapers/a_b.raw"))
        self.assertTrue(match("icons/bifrost.settings.raw"))
        self.assertFalse(match("wallpapers/sub/a.raw"))
        self.assertFalse(match("fonts/Other.ttf"))
        self.assertFalse(match("SFUI.ttf.bak"))
        self.assertFalse(match("xSFUI.ttf"))
        self.assertFalse(match("icons/other.vendor.raw"))

    def test_blanket_and_escaping_patterns_are_refused(self):
        for bad in ("*\n", "*.ttf\n", "**/x.ttf\n", "fonts/**\n", "/etc/passwd\n", "../x\n", "a/../b\n", "?x\n"):
            with self.subTest(bad):
                with self.assertRaises(stage_assets.AllowlistError):
                    stage_assets.parse_allowlist(bad)


class Staging(unittest.TestCase):
    def setUp(self):
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.src = os.path.join(self._dir.name, "src")
        self.dest = os.path.join(self._dir.name, "dest")
        self.list_file = os.path.join(self._dir.name, "allow.txt")
        with open(self.list_file, "w") as f:
            f.write(LIST)

    def staged(self):
        out = []
        for dirpath, _, files in os.walk(self.dest):
            out += [os.path.relpath(os.path.join(dirpath, f), self.dest).replace(os.sep, "/") for f in files]
        return sorted(out)

    def test_only_allowlisted_files_are_copied(self):
        write_tree(self.src, ["SFUI.ttf", "VendorSymbols-Regular.otf", "fonts/Open-Variable.ttf", "fonts/Stray.ttf",
                              "wallpapers/a.raw", "wallpapers/index.ini", "icons/bifrost.files.raw", "icons/other.maker.raw",
                              "sounds/click.pcm", "notes.txt"])
        status, _, err = run_main("--allowlist", self.list_file, self.src, self.dest)
        self.assertEqual(status, 0)
        self.assertEqual(self.staged(), ["SFUI.ttf", "fonts/Open-Variable.ttf", "icons/bifrost.files.raw", "wallpapers/a.raw"])
        self.assertIn("4 files copied, 6 not on the allowlist", err)

    def test_copies_are_plain_files_with_the_source_bytes(self):
        write_tree(self.src, ["SFUI.ttf"])
        os.chmod(os.path.join(self.src, "SFUI.ttf"), 0o600)
        run_main("--allowlist", self.list_file, self.src, self.dest)
        target = os.path.join(self.dest, "SFUI.ttf")
        with open(target, "rb") as f:
            self.assertEqual(f.read(), b"SFUI.ttf")
        self.assertEqual(stat.S_IMODE(os.stat(target).st_mode), 0o644)

    def test_links_are_never_copied_or_followed(self):
        elsewhere = os.path.join(self._dir.name, "elsewhere")
        write_tree(self.src, ["SFUI.ttf"])
        write_tree(elsewhere, ["Linked.ttf", "wallpapers/z.raw"])
        os.makedirs(os.path.join(self.src, "fonts"))
        # a link whose name is on the allowlist, a linked directory and a link back to the tree itself
        os.symlink(os.path.join(elsewhere, "Linked.ttf"), os.path.join(self.src, "fonts", "Open-Variable.ttf"))
        os.symlink(os.path.join(elsewhere, "wallpapers"), os.path.join(self.src, "wallpapers"))
        os.symlink(self.src, os.path.join(self.src, "assets"))
        run_main("--allowlist", self.list_file, self.src, self.dest)
        self.assertEqual(self.staged(), ["SFUI.ttf"])

    def test_replace_empties_the_destination_first(self):
        write_tree(self.src, ["SFUI.ttf"])
        write_tree(self.dest, ["old/Stale.ttf"])
        run_main("--allowlist", self.list_file, "--replace", self.src, self.dest)
        self.assertEqual(self.staged(), ["SFUI.ttf"])

    def test_a_link_as_destination_is_swapped_never_written_through(self):
        write_tree(self.src, ["SFUI.ttf", "Stray.otf"])
        os.symlink(self.src, self.dest)
        self.assertEqual(run_main("--allowlist", self.list_file, self.src, self.dest)[0], 2)
        run_main("--allowlist", self.list_file, "--replace", self.src, self.dest)
        self.assertFalse(os.path.islink(self.dest))
        self.assertEqual(self.staged(), ["SFUI.ttf"])
        self.assertTrue(os.path.isfile(os.path.join(self.src, "Stray.otf")), "the source is untouched")

    def test_without_replace_other_files_stay(self):
        write_tree(self.src, ["SFUI.ttf"])
        write_tree(self.dest, ["old/Keep.txt"])
        run_main("--allowlist", self.list_file, self.src, self.dest)
        self.assertEqual(self.staged(), ["SFUI.ttf", "old/Keep.txt"])

    def test_list_prints_the_allowed_paths_and_copies_nothing(self):
        write_tree(self.src, ["SFUI.ttf", "Stray.otf", "wallpapers/a.raw"])
        status, out, _ = run_main("--allowlist", self.list_file, "--list", self.src)
        self.assertEqual(status, 0)
        self.assertEqual(out.split(), ["SFUI.ttf", "wallpapers/a.raw"])
        self.assertFalse(os.path.exists(self.dest))

    def test_errors_exit_2(self):
        self.assertEqual(run_main("--allowlist", self.list_file, os.path.join(self._dir.name, "nowhere"), self.dest)[0], 2)
        write_tree(self.src, ["SFUI.ttf"])
        self.assertEqual(run_main("--allowlist", os.path.join(self._dir.name, "missing.txt"), self.src, self.dest)[0], 2)
        self.assertEqual(run_main("--allowlist", self.list_file, self.src, self.src)[0], 2)

    def test_dest_and_list_are_exclusive(self):
        write_tree(self.src, ["SFUI.ttf"])
        for argv in (("--list", self.src, self.dest), (self.src,)):
            with self.subTest(argv):
                with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                    stage_assets.main(["--allowlist", self.list_file, *argv])


class CommittedAllowlist(unittest.TestCase):
    """packaging/assets-allowlist.txt decides what ships: pin what it must and must not let through."""

    def setUp(self):
        self.patterns = stage_assets.load_allowlist()
        self.match = lambda rel: any(p.match(rel) for p in self.patterns)

    def test_the_system_assets_are_allowed(self):
        for rel in ("SFUI.ttf", "SFUIMono.ttf", "Symbols.ttf", "NotoColorEmoji.ttf", "wallpaper.raw", "wallpapers/index.ini",
                    "wallpapers/gullfoss_canyon.raw", "wallpapers/gullfoss_canyon.thumb.raw", "icons/bifrost.settings.raw",
                    "icons/bifrost.appstore.raw", "icons/bifrost.files.raw", "icons/bifrost.browser.raw", "icons/is.olibuijr.term.raw",
                    "fonts/Geist-Variable.ttf", "fonts/FiraCode-Variable.ttf"):
            with self.subTest(rel):
                self.assertTrue(self.match(rel), rel)

    def test_vendor_files_and_strays_are_not(self):
        for rel in ("SFUISymbols-Regular.otf", "Anything.otf", "fonts/Stray.ttf", "fonts/sub/Geist-Variable.ttf", "sounds/lock.pcm",
                    "icons/com.apple.Home.HomeUIService.raw", "icons/ws.hbang.Terminal.raw", "icons/org.coolstar.SileoStore.raw",
                    "assets", "bifrost-ref/x.png", "ref/INDEX.tsv", ".deployed", "wallpapers/leftover.tmp"):
            with self.subTest(rel):
                self.assertFalse(self.match(rel), rel)

    def test_the_default_allowlist_is_the_packaging_one(self):
        self.assertEqual(stage_assets.DEFAULT_ALLOWLIST, os.path.join(ROOT, "packaging", "assets-allowlist.txt"))


class NoBypass(unittest.TestCase):
    """Packaging must reach the assets only through the allowlist (read from the sources)."""

    def read(self, *parts):
        with open(os.path.join(ROOT, *parts)) as f:
            return f.read()

    def test_no_script_copies_the_asset_directory_whole(self):
        self.assertNotIn('cp -a "$_assets"', self.read("packaging", "arch", "PKGBUILD"))
        self.assertIn("stage-assets.py", self.read("packaging", "arch", "PKGBUILD"))
        iso = self.read("tools", "build-iso.sh")
        self.assertNotIn('rsync -a --delete "$ASSET_SRC/"', iso)
        self.assertIn("stage-assets.py", iso)
        self.assertIn("stage-assets.py", self.read("tools", "lab-push.sh"))

    def test_no_package_script_reads_the_reference_folder(self):
        for parts in (("packaging", "arch", "package-files.sh"), ("packaging", "arch", "PKGBUILD"), ("tools", "pc-publish.py")):
            text = self.read(*parts)
            with self.subTest(parts[-1]):
                self.assertNotIn("bifrost-ref", text)
                self.assertNotIn("PC_REFASSETS", text)
        self.assertNotIn("bifrost-ref", self.read("springboard", "tools", "aap.py"))


if __name__ == "__main__":
    unittest.main()
