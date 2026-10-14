import hashlib
import importlib.util
import json
import os
import tempfile
import unittest
from unittest import mock


HERE = os.path.dirname(__file__)
spec = importlib.util.spec_from_file_location("bundle_offline", os.path.join(HERE, "bundle-iso-offline.py"))
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


class OfflineBundleTests(unittest.TestCase):
    def test_destination_must_be_in_generated_out_tree(self):
        with self.assertRaises(RuntimeError):
            bundle.checked_destination("/tmp/offline")

    def test_bundle_manifest_hashes_exact_copied_payloads(self):
        with tempfile.TemporaryDirectory(dir=os.path.join(bundle.ROOT, "out")) as parent:
            dest = os.path.join(parent, "offline")
            payloads = {}
            for key in ("shell", "browser", "files", "arch"):
                p = os.path.join(parent, key)
                with open(p, "wb") as f:
                    f.write(key.encode())
                payloads[key] = p
            selected = [("base", "base-1-1-x86_64.pkg.tar.zst", payloads["arch"])]
            apps = [("is.olibuijr.browser.desktop", "0.1.0", payloads["browser"]),
                    ("is.olibuijr.files.desktop", "0.2.0", payloads["files"])]

            def repo_add(cmd, check):
                open(cmd[2], "wb").write(b"offline index")

            with mock.patch.object(bundle, "package_closure", return_value=selected), \
                 mock.patch.object(bundle, "shell_package", return_value=payloads["shell"]), \
                 mock.patch.object(bundle, "app_package", side_effect=apps), \
                 mock.patch.object(bundle.subprocess, "run", side_effect=repo_add):
                bundle.build(dest)

            with open(os.path.join(dest, "manifest.json")) as f:
                manifest = json.load(f)
            self.assertEqual(manifest["repo"], {"path": "repo", "database": "offline.db",
                                                 "sha256": hashlib.sha256(b"offline index").hexdigest()})
            self.assertEqual(manifest["packages"][0]["file"], selected[0][1])
            for row, key in ((manifest["packages"][0], "arch"), (manifest["shell"], "shell"),
                             (manifest["apps"][0], "browser"), (manifest["apps"][1], "files")):
                self.assertEqual(row["sha256"], hashlib.sha256(key.encode()).hexdigest())
            self.assertTrue(os.path.isfile(os.path.join(dest, "repo", "offline.db")))
            self.assertTrue(os.path.isfile(os.path.join(dest, bundle.MARKER)))

    def test_unmanaged_destination_is_never_replaced(self):
        with tempfile.TemporaryDirectory(dir=os.path.join(bundle.ROOT, "out")) as parent:
            dest = os.path.join(parent, "offline")
            os.mkdir(dest)
            sentinel = os.path.join(dest, "keep")
            open(sentinel, "w").write("unmanaged")
            with mock.patch.object(bundle, "package_closure", return_value=[]), \
                 mock.patch.object(bundle, "shell_package", return_value="unused"), \
                 mock.patch.object(bundle, "app_package", return_value=("id", "1", "unused")):
                with self.assertRaises(RuntimeError):
                    bundle.build(dest)
            self.assertTrue(os.path.isfile(sentinel))


if __name__ == "__main__":
    unittest.main()
