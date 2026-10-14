import importlib.util
import os
import re
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("check_names", os.path.join(HERE, "check-names.py"))
cn = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cn)


class ScanText(unittest.TestCase):
    def test_clean_text_has_no_hits(self):
        self.assertEqual(cn.scan_text("springboard/a.rs", "fn main() {}\n", []), [])

    def test_hit_reports_the_line(self):
        hits = cn.scan_text("springboard/a.rs", "ok\nlet x = \"iPhone\";\n", [])
        self.assertEqual([(r, i) for r, i, _ in hits], [("springboard/a.rs", 2)])

    def test_allowlist_applies_only_to_matching_paths(self):
        allow = [("springboard/ref/*", re.compile("iPhone"), "reference data")]
        self.assertEqual(cn.scan_text("springboard/ref/x.rs", "iPhone", allow), [])
        self.assertEqual(len(cn.scan_text("springboard/other.rs", "iPhone", allow)), 1)


class FileArguments(unittest.TestCase):
    def test_only_given_files_are_checked(self):
        with tempfile.TemporaryDirectory(dir=os.path.join(HERE, "..", "springboard")) as d:
            clean = os.path.join(d, "clean.rs")
            dirty = os.path.join(d, "dirty.rs")
            open(clean, "w").write("fn ok() {}\n")
            open(dirty, "w").write("// iPhone\n")
            run = lambda *f: subprocess.run([sys.executable, os.path.join(HERE, "check-names.py"), *f],
                                            capture_output=True, text=True)
            self.assertEqual(run(clean).returncode, 0)
            bad = run(clean, dirty)
            self.assertEqual(bad.returncode, 1)
            self.assertIn("dirty.rs:1", bad.stdout)


if __name__ == "__main__":
    unittest.main()
