import importlib.util, os, tempfile, unittest

spec = importlib.util.spec_from_file_location("pc_publish", os.path.join(os.path.dirname(__file__), "pc-publish.py"))
pub = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pub)


class RefAssetFiles(unittest.TestCase):
    def test_absent_folder_is_empty(self):
        self.assertEqual(pub.ref_asset_files("/nonexistent/assets"), [])
        self.assertEqual(pub.ref_asset_files(None), [])

    def test_present_tree_lists_known_kinds_only(self):
        with tempfile.TemporaryDirectory() as d:
            for rel in ("INDEX.tsv", "sounds/alerts/Basso.wav", "cursors/arrow.png", "stray/x.png"):
                p = os.path.join(d, rel)
                os.makedirs(os.path.dirname(p), exist_ok=True)
                open(p, "w").write("x")
            got = sorted(dst for _, dst in pub.ref_asset_files(d))
            self.assertEqual(got, ["INDEX.tsv", "cursors/arrow.png", "sounds/alerts/Basso.wav"])


if __name__ == "__main__":
    unittest.main()
