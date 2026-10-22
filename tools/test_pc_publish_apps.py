"""pc-publish also publishes the apps that are newer than the store (a system release brings the apps)."""
import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location("pc_publish", HERE / "pc-publish.py")
pp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pp)


class OutdatedApps(unittest.TestCase):
    def test_versions_compare_as_numbers(self):
        self.assertGreater(pp.version_key("0.16.2"), pp.version_key("0.9.10"))
        self.assertGreater(pp.version_key("1.12.2"), pp.version_key("1.12.0"))

    def test_newer_or_unpublished_x86_apps_are_published_and_the_rest_are_not(self):
        store = {"a": "1.0.0", "b": "2.0.0", "c": "0.9.10"}
        manifests = [("a-desktop", "a", "1.0.1", "x86_64-linux"), ("b-desktop", "b", "2.0.0", "x86_64-linux"),
                     ("c-desktop", "c", "0.10.0", "x86_64-linux"), ("new-desktop", "new", "0.1.0", "x86_64-linux"),
                     ("tab", "t", "9.9.9", "aarch64-linux"), ("old-desktop", "b", "1.9.0", "x86_64-linux")]
        self.assertEqual(pp.outdated_apps(store, manifests), ["a-desktop", "c-desktop", "new-desktop"])

    def test_manifests_are_read_with_their_architecture(self):
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, "x-desktop"))
            with open(os.path.join(d, "x-desktop", "manifest.toml"), "w") as f:
                f.write('id = "x"\nversion = "1.2.3"\narchitecture = "x86_64-linux"\n')
            os.makedirs(os.path.join(d, "x-desktop", "src"))
            open(os.path.join(d, "x-desktop", "src", "main.rs"), "w").close()
            self.assertEqual(pp.app_manifests(d), [("x-desktop", "x", "1.2.3", "x86_64-linux")])

    def test_a_library_without_an_executable_is_not_published(self):
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, "lib-desktop", "src"))
            with open(os.path.join(d, "lib-desktop", "manifest.toml"), "w") as f:
                f.write('id = "l"\nversion = "0.1.0"\narchitecture = "x86_64-linux"\n')
            open(os.path.join(d, "lib-desktop", "src", "lib.rs"), "w").close()
            self.assertEqual(pp.app_manifests(d), [])


if __name__ == "__main__":
    unittest.main()
