import importlib.util
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("bump_apps", os.path.join(HERE, "bump-apps.py"))
ba = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ba)

CARGO = '[package]\nname = "bifrost-demo"\nversion = "1.2.9"\nedition = "2021"\n\n[dependencies]\n{dep}\n'
MANIFEST = 'id = "is.example.demo"\nname = "Demo"\nversion = "1.2.9"\nmin_shell = "0.4.84"\n\n[[intent]]\nid = "x"\nversion = "9.9.9"\n'
LOG = "# Changelog\n\n## 1.2.9\n\n- Older note.\n"
LOCK = 'version = 3\n\n[[package]]\nname = "bifrost-demo"\nversion = "1.2.9"\n\n[[package]]\nname = "other"\nversion = "1.2.9"\n'


def make_app(root, name, dep='bifrost-app = { path = "../../sdk" }', ident="is.example.demo", changelog=LOG):
    d = os.path.join(root, name)
    os.makedirs(d)
    with open(os.path.join(d, "Cargo.toml"), "w") as f:
        f.write(CARGO.format(dep=dep))
    with open(os.path.join(d, "manifest.toml"), "w") as f:
        f.write(MANIFEST.replace("is.example.demo", ident))
    if changelog is not None:
        with open(os.path.join(d, "CHANGELOG.md"), "w") as f:
            f.write(changelog)
    return d


def read(*parts):
    with open(os.path.join(*parts)) as f:
        return f.read()


class Bump(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name
        self.lock = os.path.join(self.root, "Cargo.lock")
        with open(self.lock, "w") as f:
            f.write(LOCK)

    def tearDown(self):
        self.tmp.cleanup()

    def test_version_is_a_patch_bump(self):
        self.assertEqual(ba.bump_version("1.2.9"), "1.2.10")
        self.assertEqual(ba.bump_version("0.0.0"), "0.0.1")

    def test_bump_updates_every_file(self):
        d = make_app(self.root, "demo-desktop")
        self.assertEqual(ba.bump_app(d, "Fixed it.", self.lock), ("1.2.9", "1.2.10", "is.example.demo"))
        self.assertIn('name = "bifrost-demo"\nversion = "1.2.10"', read(d, "Cargo.toml"))
        # only the top-level manifest version moves, not a table's own version
        m = read(d, "manifest.toml")
        self.assertIn('version = "1.2.10"\nmin_shell', m)
        self.assertIn('version = "9.9.9"', m)
        self.assertTrue(read(d, "CHANGELOG.md").startswith("# Changelog\n\n## 1.2.10\n\n- Fixed it.\n\n## 1.2.9\n"))
        lock = read(self.lock)
        self.assertIn('name = "bifrost-demo"\nversion = "1.2.10"', lock)
        self.assertIn('name = "other"\nversion = "1.2.9"', lock)

    def test_empty_note_is_refused_and_nothing_changes(self):
        d = make_app(self.root, "demo-desktop")
        with self.assertRaises(ValueError):
            ba.bump_app(d, "   ", self.lock)
        self.assertIn('version = "1.2.9"', read(d, "Cargo.toml"))
        self.assertEqual(read(d, "CHANGELOG.md"), LOG)

    def test_mismatched_manifest_is_refused(self):
        d = make_app(self.root, "demo-desktop")
        with open(os.path.join(d, "manifest.toml"), "w") as f:
            f.write(MANIFEST.replace('version = "1.2.9"', 'version = "1.2.8"', 1))
        with self.assertRaises(ValueError):
            ba.bump_app(d, "Note.", self.lock)
        self.assertIn('version = "1.2.9"', read(d, "Cargo.toml"))

    def test_missing_changelog_is_created(self):
        d = make_app(self.root, "demo-desktop", changelog=None)
        ba.bump_app(d, "First.", self.lock)
        self.assertEqual(read(d, "CHANGELOG.md"), "# Changelog\n\n## 1.2.10\n\n- First.\n")

    def test_main_refuses_an_empty_note_before_touching_anything(self):
        d = make_app(self.root, "demo-desktop")
        self.assertEqual(ba.main([d + "="]), 1)
        self.assertEqual(ba.main([]), 2)
        self.assertIn('version = "1.2.9"', read(d, "Cargo.toml"))


class AllSdk(unittest.TestCase):
    def test_selects_sdk_apps_and_skips_craft_filmcraft_deildu(self):
        with tempfile.TemporaryDirectory() as root:
            make_app(root, "a-desktop")
            make_app(root, "proto-desktop", dep='bifrost-proto = { path = "../../proto" }')
            make_app(root, "plain-desktop", dep='serde = "1"')
            make_app(root, "craft-desktop", ident="ai.storyteller.craft")
            make_app(root, "filmcraft-desktop", ident="is.example.film")
            make_app(root, "deildu-desktop", ident="is.olibuijr.deildu")
            os.makedirs(os.path.join(root, "manifest-only"))
            picked = [os.path.basename(p) for p in ba.sdk_apps(root)]
        self.assertEqual(picked, ["a-desktop", "proto-desktop"])


if __name__ == "__main__":
    unittest.main()
