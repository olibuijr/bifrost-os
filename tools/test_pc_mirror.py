#!/usr/bin/env python3
"""Pure parts of the 8079 mirror: dependency closure, missing-file check, version matching.

Run: python3 -I tools/test_pc_mirror.py
"""
import io, os, sys, tarfile, unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pc_mirror as m


def db(*pkgs):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        for name, ver, deps, provs in pkgs:
            body = f"%NAME%\n{name}\n\n%VERSION%\n{ver}-1\n\n%FILENAME%\n{name}-{ver}-1-x86_64.pkg.tar.zst\n\n"
            body += "%DEPENDS%\n" + "\n".join(deps) + "\n\n" if deps else ""
            body += "%PROVIDES%\n" + "\n".join(provs) + "\n\n" if provs else ""
            data = body.encode()
            ti = tarfile.TarInfo(f"{name}-{ver}-1/desc")
            ti.size = len(data)
            tf.addfile(ti, io.BytesIO(data))
    return buf.getvalue()


PKGS = m.parse_db(db(("a", "1", ["b>=1", "libx.so=2-64"], []), ("b", "1", ["c"], []), ("c", "1", [], []),
                     ("xlib", "3", [], ["libx.so=2-64"]), ("unused", "1", [], [])))


class Parse(unittest.TestCase):
    def test_fields(self):
        self.assertEqual(PKGS["a"]["filename"], "a-1-1-x86_64.pkg.tar.zst")
        self.assertEqual(PKGS["a"]["depends"], ["b>=1", "libx.so=2-64"])

    def test_dep_name(self):
        self.assertEqual(m.dep_name("glibc>=2.38"), "glibc")
        self.assertEqual(m.dep_name("libx.so=2-64"), "libx.so")
        self.assertEqual(m.dep_name("plain"), "plain")


class Closure(unittest.TestCase):
    def test_transitive_and_provides(self):
        got, missing = m.closure(["a"], PKGS)
        self.assertEqual(got, {"a", "b", "c", "xlib"})
        self.assertEqual(missing, [])

    def test_unknown_reported(self):
        got, missing = m.closure(["a", "ghost"], PKGS)
        self.assertIn("ghost", missing)
        self.assertNotIn("unused", got)

    def test_cycle_terminates(self):
        cyc = m.parse_db(db(("p", "1", ["q"], []), ("q", "1", ["p"], [])))
        self.assertEqual(m.closure(["p"], cyc)[0], {"p", "q"})


class Files(unittest.TestCase):
    def test_missing(self):
        have = ["a-1-1-x86_64.pkg.tar.zst", "b-1-1-x86_64.pkg.tar.zst"]
        self.assertEqual(m.missing_files({"a", "b", "c"}, PKGS, have), ["c-1-1-x86_64.pkg.tar.zst"])

    def test_wrong_version_counts_as_missing_and_stale(self):
        have = ["c-0-1-x86_64.pkg.tar.zst"]
        self.assertEqual(m.missing_files({"c"}, PKGS, have), ["c-1-1-x86_64.pkg.tar.zst"])
        self.assertEqual(m.stale_files(PKGS, have), have)

    def test_nothing_missing(self):
        self.assertEqual(m.missing_files(set(), PKGS, []), [])


class Versions(unittest.TestCase):
    def test_match(self):
        self.assertTrue(m.filename_matches("noto-fonts", "2026.10.01-1", "noto-fonts-2026.10.01-1-any.pkg.tar.zst"))

    def test_old_version(self):
        self.assertFalse(m.filename_matches("noto-fonts", "2026.10.01-1", "noto-fonts-2026.09.01-1-any.pkg.tar.zst"))

    def test_prefix_name_not_confused(self):
        self.assertFalse(m.filename_matches("noto-fonts", "1-1", "noto-fonts-extra-1-1-any.pkg.tar.zst"))


if __name__ == "__main__":
    unittest.main()
