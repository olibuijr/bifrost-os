#!/usr/bin/env python3
"""Tests for tools/check-package-licenses.py. Every font is generated here (a bare sfnt with only a name table);
no real font file is read or copied.

Run: python3 tools/test_check_package_licenses.py
"""
import contextlib
import importlib.util
import io
import os
import shutil
import struct
import subprocess
import tarfile
import tempfile
import unittest
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("licenses", os.path.join(HERE, "check-package-licenses.py"))
lic = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lic)

OFL = "This Font Software is licensed under the SIL Open Font License, Version 1.1."
OFL_URL = "https://openfontlicense.org"
RESERVED = "Copyright 2020 Example Foundry. All rights reserved. Use is subject to the foundry's agreement."


# ---------------------------------------------------------------- generated fonts

def name_table(records):
    """A format 0 `name` table from (platform, encoding, language, name ID, text) records."""
    strings = b""
    rows = []
    for platform, encoding, language, name_id, text in records:
        raw = text.encode("utf-16-be") if platform in (0, 3) else text.encode("mac_roman")
        rows.append(struct.pack(">6H", platform, encoding, language, name_id, len(raw), len(strings)))
        strings += raw
    return struct.pack(">3H", 0, len(rows), 6 + 12 * len(rows)) + b"".join(rows) + strings


def names(family="Test Sans", licence=None, url=None, copyright=None):
    """Windows-platform records for a face."""
    records = [(3, 1, 0x409, 1, family), (3, 1, 0x409, 2, "Regular")]
    for name_id, text in ((0, copyright), (13, licence), (14, url)):
        if text is not None:
            records.append((3, 1, 0x409, name_id, text))
    return records


def sfnt(tables, tag=b"\x00\x01\x00\x00", base=0):
    """An sfnt whose table offsets are absolute for a file where this font starts at `base`."""
    head_len = 12 + 16 * len(tables)
    head = struct.pack(">4sHHHH", tag, len(tables), 0, 0, 0)
    blobs = b""
    for table_tag in sorted(tables):
        data = tables[table_tag]
        head += struct.pack(">4sIII", table_tag, 0, base + head_len + len(blobs), len(data))
        blobs += data + b"\0" * (-len(data) % 4)
    return head + blobs


def ttf(records, tag=b"\x00\x01\x00\x00"):
    return sfnt({b"name": name_table(records)}, tag)


def ttc(*faces):
    """A collection: each face is a record list."""
    offset = 12 + 4 * len(faces)
    body = b""
    offsets = []
    for records in faces:
        font = sfnt({b"name": name_table(records)}, base=offset + len(body))
        offsets.append(offset + len(body))
        body += font
    return struct.pack(">4sII", b"ttcf", 0x00010000, len(faces)) + b"".join(struct.pack(">I", o) for o in offsets) + body


def woff(records):
    table = name_table(records)
    packed = zlib.compress(table)
    stored = packed if len(packed) < len(table) else table
    offset = 44 + 20
    header = struct.pack(">4s4sIHHIHHIIIII", b"wOFF", b"\x00\x01\x00\x00", offset + len(stored), 1, 0, 0, 1, 0, 0, 0, 0, 0, 0)
    entry = struct.pack(">4sIIII", b"name", offset, len(stored), len(table), 0)
    return header + entry + stored


# ---------------------------------------------------------------- generated packages

def write_tree(root, files):
    for rel, data in files.items():
        path = os.path.join(root, *rel.split("/"))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)


def tar_bytes(files, links=None, fmt=tarfile.USTAR_FORMAT):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=fmt) as tar:
        for rel, data in files.items():
            info = tarfile.TarInfo(rel)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        for rel, target in (links or {}).items():
            info = tarfile.TarInfo(rel)
            info.type = tarfile.SYMTYPE
            info.linkname = target
            tar.addfile(info)
    return buf.getvalue()


def run_main(*argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        status = lic.main(list(argv))
    return status, out.getvalue(), err.getvalue()


class Scratch(unittest.TestCase):
    def setUp(self):
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.dir = self._dir.name
        self.allow_file = os.path.join(self.dir, "allow.txt")
        with open(self.allow_file, "w") as f:
            f.write("# no excuses\n")

    def path(self, *parts):
        return os.path.join(self.dir, *parts)


# ---------------------------------------------------------------- readers

class NameReaders(unittest.TestCase):
    def faces(self, data, fonttools):
        return lic.font_faces(data, use_fonttools=fonttools)

    def test_builtin_reader_reads_every_container(self):
        records = names("Test Sans", OFL, OFL_URL, "Copyright 2020 Example Authors")
        long_records = names("Test Sans", OFL * 20, OFL_URL)
        for label, data in (("ttf", ttf(records)), ("otf", ttf(records, b"OTTO")), ("woff", woff(records)),
                            ("woff compressed", woff(long_records))):
            with self.subTest(label):
                (face,) = self.faces(data, False)
                self.assertEqual(face[1], ["Test Sans"])
                self.assertIn("Open Font License", face[13][0])
                self.assertEqual(face[14], [OFL_URL])

    def test_collection_reports_every_face(self):
        data = ttc(names("First", OFL), names("Second", RESERVED))
        first, second = self.faces(data, False)
        self.assertEqual(first[1], ["First"])
        self.assertEqual(second[1], ["Second"])

    def test_mac_platform_records_are_decoded(self):
        data = ttf([(1, 0, 0, 1, "Mac Face"), (1, 0, 0, 13, OFL)])
        (face,) = self.faces(data, False)
        self.assertEqual(face[1], ["Mac Face"])
        self.assertTrue(lic.open_licence(lic.licence_text(face)))

    def test_font_without_a_name_table_has_no_names(self):
        (face,) = self.faces(sfnt({b"cmap": b"\0\0\0\0"}), False)
        self.assertEqual(face, {})

    def test_garbage_is_a_font_error(self):
        for data in (b"", b"not a font at all", b"\x00\x01\x00\x00\x00", b"wOF2" + b"\0" * 40):
            with self.subTest(data[:8]):
                with self.assertRaises(lic.FontError):
                    self.faces(data, False)

    def test_fonttools_and_the_builtin_reader_agree(self):
        if not lic.have_fonttools():
            self.skipTest("fontTools is not installed")
        for data in (ttf(names("Agree Sans", OFL, OFL_URL)), ttc(names("A", OFL), names("B", RESERVED))):
            self.assertEqual(self.faces(data, True), self.faces(data, False))


# ---------------------------------------------------------------- licences

class Licences(unittest.TestCase):
    def accepted(self, **kw):
        return lic.licence_problem({13: [kw.get("d", "")], 14: [kw.get("u", "")], 0: [kw.get("c", "")]}) is None

    def test_known_open_licences_are_accepted(self):
        texts = [
            "Licensed under the SIL Open Font License, Version 1.1",
            "This Font Software is licensed under the Open Font License",
            "Apache License, Version 2.0",
            "Ubuntu Font Licence 1.0",
            "GNU General Public License with the font exception: you may embed the font in a document",
            "The GNU Affero General Public License applies, with a font exception",
            "Released under the LaTeX Project Public License",
            "GUST Font License",
            "Permission is hereby granted, free of charge, to any person obtaining a copy of the fonts accompanying this license",
            "Bitstream Vera Fonts Copyright",
            "DejaVu fonts are in the public domain",
            "The MIT License",
            "CC0 1.0 Universal",
            "Dedicated to the public domain",
            "IPA Font License Agreement v1.0",
        ]
        for text in texts:
            with self.subTest(text):
                self.assertTrue(self.accepted(d=text))
                self.assertEqual(lic.licence_problem({0: [text]}), None, "copyright ID 0 may carry the licence")

    def test_urls_alone_are_enough(self):
        for url in ("https://openfontlicense.org", "http://scripts.sil.org/OFL", "https://www.apache.org/licenses/LICENSE-2.0"):
            with self.subTest(url):
                self.assertTrue(self.accepted(u=url))

    def test_all_rights_reserved_without_an_open_licence_fails(self):
        problem = lic.licence_problem({0: [RESERVED], 13: ["Your use of this font is subject to the foundry's terms."]})
        self.assertIn("not a known open licence", problem)
        self.assertIn("all rights reserved", problem)

    def test_no_licence_text_fails(self):
        self.assertIn("no licence description", lic.licence_problem({1: ["Plain"]}))
        self.assertIn("no licence description", lic.licence_problem({0: ["Copyright 2020 Someone"]}))

    def test_an_open_licence_outranks_the_boilerplate_in_the_copyright(self):
        self.assertTrue(self.accepted(c="Copyright 2019 Example Corp. All rights reserved.", d=OFL))

    def test_free_for_personal_use_and_negated_public_domain_fail(self):
        for text in ("Free for personal use only", "This font is not in the public domain", "Do not redistribute"):
            with self.subTest(text):
                self.assertFalse(self.accepted(d=text))

    def test_gpl_without_an_exception_fails(self):
        self.assertFalse(self.accepted(d="GNU General Public License version 3"))


# ---------------------------------------------------------------- allowlist

class Allowlist(unittest.TestCase):
    def test_parses_paths_and_families_with_reasons(self):
        allow = lic.parse_allowlist("# c\n\npath usr/share/fonts/x/*.ttf -- licence file ships beside it\nfamily Odd Family -- upstream says free\n")
        self.assertTrue(allow.path_reason("usr/share/fonts/x/A.ttf"))
        self.assertFalse(allow.path_reason("usr/share/fonts/y/A.ttf"))
        self.assertTrue(allow.family_reason({1: ["ODD family"]}))
        self.assertFalse(allow.family_reason({1: ["Other"]}))

    def test_a_glob_without_a_slash_matches_the_file_name(self):
        allow = lic.parse_allowlist("path Odd.ttf -- reason\n")
        self.assertTrue(allow.path_reason("any/where/Odd.ttf"))

    def test_a_reason_is_mandatory_and_blanket_patterns_are_refused(self):
        for bad in ("path usr/x.ttf\n", "path usr/x.ttf -- \n", "family -- reason\n", "dir usr/x -- reason\n", "path * -- everything\n", "path **/* -- everything\n"):
            with self.subTest(bad):
                with self.assertRaises(lic.AllowError):
                    lic.parse_allowlist(bad)

    def test_the_committed_allowlist_parses(self):
        lic.load_allowlist()


# ---------------------------------------------------------------- directory trees

class Trees(Scratch):
    def test_a_clean_tree_passes(self):
        write_tree(self.path("pkg"), {
            "usr/share/springboard/assets/SFUI.ttf": ttf(names("Inter", OFL, OFL_URL)),
            "usr/share/springboard/assets/fonts/Mix.ttc": ttc(names("A", OFL), names("B", None, OFL_URL)),
            "usr/bin/tool": b"#!/bin/sh\n",
        })
        status, out, err = run_main("--allow", self.allow_file, self.path("pkg"))
        self.assertEqual((status, err), (0, ""))
        self.assertIn("ok, 2 font file(s) checked", out)

    def test_a_font_without_an_open_licence_fails_and_names_its_path(self):
        write_tree(self.path("pkg"), {
            "usr/share/springboard/assets/SFUI.ttf": ttf(names("Inter", OFL)),
            "usr/share/springboard/assets/VendorSymbols-Regular.otf": ttf(names("Vendor Symbols", "Your use of this font is subject to the vendor's terms.", None, RESERVED), b"OTTO"),
        })
        status, _, err = run_main("--allow", self.allow_file, self.path("pkg"))
        self.assertEqual(status, 1)
        self.assertIn("usr/share/springboard/assets/VendorSymbols-Regular.otf", err)
        self.assertIn("all rights reserved", err)
        self.assertNotIn("SFUI.ttf", err)

    def test_every_face_of_a_collection_is_checked(self):
        write_tree(self.path("pkg"), {"fonts/Mix.ttc": ttc(names("Open", OFL), names("Closed", None, None, RESERVED))})
        status, _, err = run_main("--allow", self.allow_file, self.path("pkg"))
        self.assertEqual(status, 1)
        self.assertIn("fonts/Mix.ttc (face 1)", err)
        self.assertIn("Closed", err)
        self.assertNotIn("face 0", err)

    def test_reference_assets_fail_whatever_they_are(self):
        write_tree(self.path("pkg"), {"usr/share/bifrost/ref/INDEX.tsv": b"sound\tx\tx.wav\n", "usr/share/bifrost/refs/ok.txt": b"fine"})
        status, _, err = run_main("--allow", self.allow_file, self.path("pkg"))
        self.assertEqual(status, 1)
        self.assertIn("usr/share/bifrost/ref/INDEX.tsv", err)
        self.assertNotIn("ok.txt", err)

    def test_an_empty_reference_folder_and_a_bifrost_ref_folder_fail(self):
        os.makedirs(self.path("pkg", "usr", "share", "bifrost", "ref"))
        os.makedirs(self.path("other", "opt", "bifrost-ref"))
        for target in ("pkg", "other"):
            with self.subTest(target):
                status, _, _ = run_main("--allow", self.allow_file, self.path(target))
                self.assertEqual(status, 1)

    def test_a_link_into_the_reference_folder_fails(self):
        os.makedirs(self.path("pkg", "usr", "share"))
        os.symlink("/usr/share/bifrost/ref", self.path("pkg", "usr", "share", "sounds"))
        status, _, err = run_main("--allow", self.allow_file, self.path("pkg"))
        self.assertEqual(status, 1)
        self.assertIn("link target", err)

    def test_links_are_not_followed(self):
        write_tree(self.path("outside"), {"Closed.ttf": ttf(names("Closed", None, None, RESERVED))})
        os.makedirs(self.path("pkg"))
        os.symlink(self.path("outside", "Closed.ttf"), self.path("pkg", "Link.ttf"))
        status, _, _ = run_main("--allow", self.allow_file, self.path("pkg"))
        self.assertEqual(status, 0)

    def test_an_unreadable_font_fails(self):
        write_tree(self.path("pkg"), {"a/Broken.ttf": b"nothing like a font", "a/Empty.otf": b""})
        status, _, err = run_main("--allow", self.allow_file, self.path("pkg"))
        self.assertEqual(status, 1)
        self.assertIn("a/Broken.ttf", err)
        self.assertIn("a/Empty.otf", err)

    def test_the_allowlist_excuses_a_path_or_a_family(self):
        write_tree(self.path("pkg"), {
            "usr/share/fonts/odd/Odd.ttf": ttf(names("Odd Sans", None, None, RESERVED)),
            "usr/share/fonts/other/Other.ttf": ttf(names("Other Sans", None, None, RESERVED)),
        })
        with open(self.allow_file, "w") as f:
            f.write("path usr/share/fonts/odd/* -- licence file ships beside it\nfamily other sans -- upstream says free\n")
        status, out, err = run_main("--allow", self.allow_file, self.path("pkg"))
        self.assertEqual((status, err), (0, ""))
        self.assertIn("2 excused by the allowlist", out)

    def test_the_built_in_reader_gives_the_same_verdicts(self):
        write_tree(self.path("pkg"), {"a/Open.ttf": ttf(names("Open", OFL)), "a/Closed.ttf": ttf(names("Closed", None, None, RESERVED))})
        status, _, err = run_main("--allow", self.allow_file, "--no-fonttools", self.path("pkg"))
        self.assertEqual(status, 1)
        self.assertIn("a/Closed.ttf", err)
        self.assertNotIn("a/Open.ttf", err)

    def test_every_target_is_reported(self):
        write_tree(self.path("one"), {"usr/share/bifrost/ref/x": b"x"})
        write_tree(self.path("two"), {"usr/share/bifrost/ref/y": b"y"})
        status, _, err = run_main("--allow", self.allow_file, self.path("one"), self.path("two"))
        self.assertEqual(status, 1)
        self.assertIn("ref/x", err)
        self.assertIn("ref/y", err)

    def test_a_missing_target_or_allowlist_is_a_usage_error(self):
        self.assertEqual(run_main("--allow", self.allow_file, self.path("nowhere"))[0], 2)
        os.makedirs(self.path("pkg"))
        self.assertEqual(run_main("--allow", self.path("no-such-file"), self.path("pkg"))[0], 2)
        with open(self.allow_file, "w") as f:
            f.write("path x.ttf\n")
        self.assertEqual(run_main("--allow", self.allow_file, self.path("pkg"))[0], 2)


# ---------------------------------------------------------------- packages

class Packages(Scratch):
    GOOD = {
        "usr/bin/springboard": b"\x7fELF",
        "usr/share/springboard/assets/SFUI.ttf": ttf(names("Inter", OFL, OFL_URL)),
    }
    BAD = {
        "usr/share/springboard/assets/VendorSymbols-Regular.otf": ttf(names("Vendor Symbols", None, None, RESERVED), b"OTTO"),
        "usr/share/bifrost/ref/INDEX.tsv": b"x",
    }

    def write(self, name, files, links=None, compress=None):
        path = self.path(name)
        data = tar_bytes(files, links)
        if compress == "gz":
            import gzip
            data = gzip.compress(data)
        elif compress == "zst":
            data = self.zstd(data)
        with open(path, "wb") as f:
            f.write(data)
        return path

    def zstd(self, data):
        try:
            from compression import zstd
            return zstd.compress(data)
        except ImportError:
            exe = shutil.which("zstd")
            if exe is None:
                self.skipTest("no zstd available to build the fixture")
            return subprocess.run([exe, "-q", "-c"], input=data, stdout=subprocess.PIPE, check=True).stdout

    def test_an_app_package_is_read_as_an_uncompressed_tar(self):
        good = self.write("app-1.0.aap", {"manifest.toml": b'id = "x"\n', "assets/OFL.ttf": ttf(names("Open", OFL))})
        bad = self.write("app-2.0.aap", {"manifest.toml": b'id = "x"\n', "assets/Closed.ttf": ttf(names("Closed", None, None, RESERVED))})
        self.assertEqual(run_main("--allow", self.allow_file, good)[0], 0)
        status, _, err = run_main("--allow", self.allow_file, bad)
        self.assertEqual(status, 1)
        self.assertIn("assets/Closed.ttf", err)

    def test_a_compressed_package_is_read(self):
        for compress, name in (("gz", "a-1-1-x86_64.pkg.tar.gz"), ("zst", "a-1-1-x86_64.pkg.tar.zst")):
            with self.subTest(compress):
                good = self.write("good-" + name, self.GOOD, compress=compress)
                bad = self.write("bad-" + name, {**self.GOOD, **self.BAD}, compress=compress)
                self.assertEqual(run_main("--allow", self.allow_file, good)[0], 0)
                status, _, err = run_main("--allow", self.allow_file, bad)
                self.assertEqual(status, 1)
                self.assertIn("VendorSymbols-Regular.otf", err)
                self.assertIn("usr/share/bifrost/ref/INDEX.tsv", err)

    def test_leading_dot_slash_paths_are_normalised(self):
        pkg = self.write("p.pkg.tar.gz", {"./usr/share/bifrost/ref/x": b"x"}, compress="gz")
        status, _, err = run_main("--allow", self.allow_file, pkg)
        self.assertEqual(status, 1)
        self.assertIn(": usr/share/bifrost/ref/x:", err)

    def test_a_symlink_into_the_reference_folder_fails(self):
        pkg = self.write("p.aap", {"manifest.toml": b"x"}, links={"assets/sounds": "/usr/share/bifrost/ref/sounds"})
        status, _, err = run_main("--allow", self.allow_file, pkg)
        self.assertEqual(status, 1)
        self.assertIn("link target", err)

    def test_something_that_is_not_an_archive_is_a_usage_error(self):
        path = self.path("not-a-package.pkg.tar.zst")
        with open(path, "wb") as f:
            f.write(b"this is plain text, not a package")
        self.assertEqual(run_main("--allow", self.allow_file, path)[0], 2)


class Wiring(unittest.TestCase):
    """The scripts that build packages and ISOs must call the guard (read from the sources)."""

    def read(self, *parts):
        with open(os.path.join(os.path.dirname(HERE), *parts)) as f:
            return f.read()

    def test_publishing_paths_run_the_guard(self):
        for parts in (("tools", "pc-publish.py"), ("tools", "pc-app.py"), ("tools", "build-iso.sh"), ("packaging", "arch", "PKGBUILD")):
            with self.subTest(parts[-1]):
                self.assertIn("check-package-licenses.py", self.read(*parts))


if __name__ == "__main__":
    unittest.main()
