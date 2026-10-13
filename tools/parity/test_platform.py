#!/usr/bin/env python3
"""Unit tests for the second reference platform in tools/parity/parity.py: the tablet index refresh, reuse of macOS
assessments by reference, and --platform on status, coverage, show, gaps and check."""
import argparse
import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import parity  # noqa: E402


def row(feature, status, bifrost="", note=""):
    return {"feature": feature, "apple": "x", "bifrost": bifrost, "status": status, "note": note}


def fw(name, status, rows, **extra):
    return dict({"framework": name, "doc": "https://developer.apple.com/documentation/" + name.lower(),
                 "status": status, "bifrost": "", "summary": name + " summary", "rea_evidence": "", "features": rows}, **extra)


def idx(title, group="App Frameworks", macos=None, ipados=None):
    slug = title.lower().replace(" ", "-")
    r = {"title": title, "slug": slug, "path": slug.replace("-", ""), "group": group,
         "doc": "https://developer.apple.com/documentation/" + slug.replace("-", ""), "deprecated": None, "abstract": ""}
    if macos:
        r["macos_since"] = macos
    if ipados:
        r["ipados_since"] = ipados
    return r


class Tree:
    """A temporary docs/parity-matrix with a macOS side and a tablet side, wired into parity for one test."""

    def __init__(self, macos_index, macos_data, ipados_index, ipados_data, files=()):
        self.root = tempfile.mkdtemp(prefix="parity-platform-")
        base = os.path.join(self.root, "docs", "parity-matrix")
        self.macos_data = os.path.join(base, "data")
        self.ipados = os.path.join(base, "ipados")
        os.makedirs(self.macos_data)
        os.makedirs(os.path.join(self.ipados, "data"))
        self.macos_index = os.path.join(base, "index.json")
        self.write(self.macos_index, macos_index)
        self.write(os.path.join(self.ipados, "index.json"), ipados_index)
        for slug, d in macos_data.items():
            self.write(os.path.join(self.macos_data, slug + ".json"), d)
        for slug, d in ipados_data.items():
            self.write(os.path.join(self.ipados, "data", slug + ".json"), d)
        for rel in files:
            p = Path(self.root, rel)
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("x")
        self.patches = [mock.patch.object(parity, "ROOT", self.root), mock.patch.object(parity, "DATA", self.macos_data),
                        mock.patch.object(parity, "INDEX", self.macos_index), mock.patch.object(parity, "IPADOS", self.ipados)]

    @staticmethod
    def write(path, data):
        with open(path, "w") as fh:
            fh.write(data if isinstance(data, str) else json.dumps(data))

    def __enter__(self):
        for p in self.patches:
            p.start()
        return self

    def __exit__(self, *exc):
        for p in self.patches:
            p.stop()


def run(fn, **kw):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        try:
            fn(argparse.Namespace(**kw))
            code = 0
        except SystemExit as e:
            code = e.code
    return out.getvalue(), code


def standard():
    """Foundation-like shared framework, a split shared one, a macOS-only one and a tablet-only one."""
    macos_index = [idx("Core Data", macos="10.4"), idx("SwiftUI", macos="10.15"), idx("Desktop Only", macos="10.0")]
    macos_data = {
        "core-data": fw("Core Data", "partial", [row("Contexts", "parity", "springboard/a.rs"), row("Migration", "gap")],
                        gaps=["Migration"]),
        "swiftui-views": fw("SwiftUI", "partial", [row("Text", "parity"), row("Table", "partial")]),
        "swiftui-layout": fw("SwiftUI", "partial", [row("Stacks", "parity")]),
        "desktop-only": fw("Desktop Only", "partial", [row("Menus", "parity")]),
    }
    ipados_index = [idx("Core Data", macos="10.4", ipados="3.0"), idx("SwiftUI", macos="10.15", ipados="13.0"),
                    idx("Touch Kit", group="Hardware", ipados="2.0")]
    return macos_index, macos_data, ipados_index


class IndexRows(unittest.TestCase):
    def pages(self, plats):
        sections = [{"kind": "technologies", "groups": [{"name": "G", "technologies": [
            {"title": t, "destination": {"identifier": f"doc://x/documentation/{t.replace(' ', '')}"}} for t in plats]}]}]
        pages = {"technologies": {"sections": sections}}
        for t, p in plats.items():
            if p is not None:
                pages[t.replace(" ", "").lower()] = {"metadata": {"platforms": p}, "abstract": [{"type": "text", "text": f"About {t}."}]}

        def get(path):
            if path not in pages:
                raise OSError("404")
            return pages[path]
        return get

    def test_tablet_rows_record_both_availabilities(self):
        get = self.pages({
            "Alpha": [{"name": "iPadOS", "introducedAt": "13.0"}, {"name": "macOS", "introducedAt": "10.15"}],
            "Beta": [{"name": "iPadOS", "introducedAt": "26.0"}],
            "Gamma": [{"name": "macOS", "introducedAt": "10.0"}],
        })
        rows = parity.index_rows(get, "ipados")
        self.assertEqual([r["slug"] for r in rows], ["alpha", "beta"])
        self.assertEqual((rows[0]["ipados_since"], rows[0]["macos_since"]), ("13.0", "10.15"))
        self.assertEqual(rows[1]["ipados_since"], "26.0")
        self.assertNotIn("macos_since", rows[1])
        self.assertEqual(rows[0]["doc"], "https://developer.apple.com/documentation/alpha")

    def test_macos_rows_keep_their_shape(self):
        get = self.pages({"Alpha": [{"name": "iPadOS", "introducedAt": "13.0"}, {"name": "macOS", "introducedAt": "10.15"}]})
        (r,) = parity.index_rows(get, "macos")
        self.assertEqual(list(r), ["title", "slug", "path", "group", "doc", "macos_since", "deprecated", "abstract"])
        self.assertNotIn("ipados_since", r)

    def test_a_missing_tablet_version_falls_back_to_the_phone_version(self):
        get = self.pages({"Alpha": [{"name": "iOS", "introducedAt": "17.0"}, {"name": "iPadOS"}]})
        self.assertEqual(parity.index_rows(get, "ipados")[0]["ipados_since"], "17.0")

    def test_deprecation_comes_from_the_requested_platform(self):
        get = self.pages({"Alpha": [{"name": "iPadOS", "introducedAt": "5.0", "deprecated": True, "deprecatedAt": "17.0"},
                                    {"name": "macOS", "introducedAt": "10.0"}]})
        self.assertEqual(parity.index_rows(get, "ipados")[0]["deprecated"], "17.0")
        self.assertIsNone(parity.index_rows(get, "macos")[0]["deprecated"])

    def test_pages_without_data_are_skipped_and_slugs_stay_unique(self):
        get = self.pages({"Alpha": [{"name": "iPadOS", "introducedAt": "1.0"}], "Missing": None})
        self.assertEqual([r["slug"] for r in parity.index_rows(get, "ipados")], ["alpha"])

    def test_known_abstracts_are_kept_and_blanks_filled(self):
        get = self.pages({"Alpha": [{"name": "iPadOS", "introducedAt": "1.0"}], "Beta": [{"name": "iPadOS", "introducedAt": "1.0"}]})
        rows = parity.index_rows(get, "ipados", {"alpha": {"abstract": "Kept."}})
        self.assertEqual([r["abstract"] for r in rows], ["Kept.", "About Beta."])

    def test_cmd_refresh_index_writes_the_platform_index(self):
        macos_index, macos_data, ipados_index = standard()
        get = self.pages({"Core Data": [{"name": "iPadOS", "introducedAt": "3.0"}, {"name": "macOS", "introducedAt": "10.4"}]})
        with Tree(macos_index, macos_data, [], {}) as t:
            os.remove(os.path.join(t.ipados, "index.json"))
            with mock.patch.object(parity, "docc_get", get):
                out, _ = run(parity.cmd_refresh_index, platform="ipados")
            written = json.load(open(os.path.join(t.ipados, "index.json")))
            self.assertEqual(json.load(open(t.macos_index)), macos_index)  # the macOS index is untouched
        self.assertEqual([r["slug"] for r in written], ["core-data"])
        self.assertIn("1 frameworks with iPadOS availability", out)
        self.assertIn("1 also on macOS", out)


class Loading(unittest.TestCase):
    def test_platform_paths(self):
        with Tree(*standard(), {}):
            self.assertEqual(parity.platform_paths("macos"), (parity.DATA, parity.INDEX))
            data, index = parity.platform_paths("ipados")
            self.assertTrue(data.endswith(os.path.join("ipados", "data")) and index.endswith(os.path.join("ipados", "index.json")))
        with self.assertRaises(ValueError):
            parity.platform_paths("watchos")

    def test_shared_frameworks_reuse_the_macos_rows_by_reference(self):
        with Tree(*standard(), {}):
            entries, bad = parity.load_entries("ipados")
        self.assertEqual(bad, [])
        self.assertEqual(sorted(e["slug"] for e in entries), ["core-data", "swiftui-layout", "swiftui-views"])
        self.assertTrue(all(e["source"] == "macos" and e["platform"] == "ipados" for e in entries))
        cd = next(e for e in entries if e["slug"] == "core-data")
        self.assertEqual((cd["since"], cd["coverage"]), ("3.0", 50.0))
        self.assertEqual(cd["gaps"], ["Migration"])

    def test_macos_only_frameworks_are_not_on_the_tablet_and_macos_is_unchanged(self):
        with Tree(*standard(), {}):
            names = [e["slug"] for e in parity.load_entries("ipados")[0]]
            macos, _ = parity.load_entries()
            self.assertNotIn("desktop-only", names)
            self.assertEqual(len(macos), 4)
            self.assertEqual({e["source"] for e in macos}, {"own"})
            self.assertEqual(next(e for e in macos if e["slug"] == "core-data")["since"], "10.4")

    def test_a_tablet_only_framework_has_its_own_file(self):
        own = {"touch-kit": fw("Touch Kit", "partial", [row("Touch", "parity"), row("Pencil", "gap")])}
        with Tree(*standard(), own):
            entries, _ = parity.load_entries("ipados")
            miss = parity.missing_slugs(entries, "ipados")
        tk = next(e for e in entries if e["slug"] == "touch-kit")
        self.assertEqual((tk["source"], tk["group"], tk["since"], tk["coverage"]), ("own", "Hardware", "2.0", 50.0))
        self.assertEqual(miss, [])

    def test_an_own_file_replaces_the_macos_rows(self):
        own = {"core-data": fw("Core Data", "gap", [row("Contexts", "gap")])}
        with Tree(*standard(), own):
            entries, _ = parity.load_entries("ipados")
        (cd,) = [e for e in entries if e["slug"] == "core-data"]
        self.assertEqual((cd["source"], len(cd["rows"]), cd["status"]), ("own", 1, "gap"))

    def test_reuse_extends_the_macos_rows(self):
        own = {"core-data": fw("Core Data", "partial", [row("Migration", "partial", "springboard/b.rs"), row("Touch menus", "gap")],
                               reuse="macos")}
        del own["core-data"]["status"]
        with Tree(*standard(), own, files=["springboard/b.rs"]):
            entries, bad = parity.load_entries("ipados")
        self.assertEqual(bad, [])
        (cd,) = [e for e in entries if e["slug"] == "core-data"]
        self.assertEqual(cd["source"], "macos+own")
        self.assertEqual([(r["feature"], r["status"]) for r in cd["rows"]],
                         [("Contexts", "parity"), ("Migration", "partial"), ("Touch menus", "gap")])
        self.assertEqual(cd["status"], "partial")
        self.assertEqual(cd["gaps"], ["Migration"])  # not stated by the file: taken from the macOS entry
        self.assertEqual(cd["coverage"], round(100 * 1.5 / 3, 1))

    def test_reuse_with_parts_merges_them_into_one_entry(self):
        own = {"swiftui": fw("SwiftUI", "partial", [row("Pencil", "gap")], reuse={"platform": "macos"})}
        with Tree(*standard(), own):
            entries, bad = parity.load_entries("ipados")
        self.assertEqual(bad, [])
        (s,) = [e for e in entries if e["index_slug"] == "swiftui"]
        self.assertEqual([r["feature"] for r in s["rows"]], ["Stacks", "Text", "Table", "Pencil"])

    def test_a_reuse_with_no_macos_assessment_is_reported(self):
        own = {"touch-kit": fw("Touch Kit", "gap", [row("Touch", "gap")], reuse="macos:nothing-here")}
        with Tree(*standard(), own):
            entries, bad = parity.load_entries("ipados")
        self.assertEqual(len(bad), 1)
        self.assertIn("nothing-here", bad[0])
        self.assertIn("touch-kit", [e["slug"] for e in entries])  # its own rows are kept

    def test_a_bad_reuse_spec_is_reported(self):
        own = {"touch-kit": fw("Touch Kit", "gap", [row("Touch", "gap")], reuse="ios")}
        with Tree(*standard(), own):
            _, bad = parity.load_entries("ipados")
        self.assertTrue(any("ios" in b for b in bad))

    def test_parse_reuse(self):
        self.assertEqual(parity.parse_reuse("macos", "a"), ("macos", "a"))
        self.assertEqual(parity.parse_reuse("macos:b", "a"), ("macos", "b"))
        self.assertEqual(parity.parse_reuse({"platform": "macos", "slug": "c"}, "a"), ("macos", "c"))
        for bad in (3, "ios:x", {"platform": "tv"}):
            with self.assertRaises(ValueError):
                parity.parse_reuse(bad, "a")

    def test_derive_status(self):
        import collections
        c = collections.Counter
        self.assertEqual(parity.derive_status(c()), "n/a")
        self.assertEqual(parity.derive_status(c(parity=2)), "parity")
        self.assertEqual(parity.derive_status(c(parity=1, gap=1)), "partial")
        self.assertEqual(parity.derive_status(c(os=1, gap=1)), "partial")
        self.assertEqual(parity.derive_status(c(gap=2, **{"n/a": 1})), "gap")

    def test_source_counts(self):
        own = {"touch-kit": fw("Touch Kit", "gap", [row("Touch", "gap")]),
               "core-data": fw("Core Data", "gap", [row("Contexts", "gap")])}
        with Tree(*standard(), own):
            entries, _ = parity.load_entries("ipados")
            sc = parity.source_counts(entries, parity.load_index("ipados"))
        self.assertEqual(sc, dict(frameworks=3, shared=2, reused=1, shared_own=1, tablet_only=1, tablet_only_own=1))


class Commands(unittest.TestCase):
    def setUp(self):
        self.own = {"touch-kit": fw("Touch Kit", "partial", [row("Touch", "parity"), row("Pencil", "gap")], gaps=["Pencil"])}
        self.tree = Tree(*standard(), self.own, files=["springboard/a.rs"])
        self.tree.__enter__()
        self.addCleanup(self.tree.__exit__, None, None, None)

    def test_status_defaults_to_macos_and_takes_the_tablet(self):
        out, _ = run(parity.cmd_status)
        self.assertIn("Assessed: 4 frameworks/parts", out)
        self.assertNotIn("Platform ipados", out)
        out, _ = run(parity.cmd_status, platform="ipados")
        self.assertIn("Platform ipados: 3 frameworks in the index; 2 also on macOS (2 reuse the macOS rows, 0 have their own file); "
                      "1 tablet-only (1 assessed)", out)
        self.assertIn("Assessed: 4 frameworks/parts; not yet assessed: 0", out)

    def test_coverage_lists_the_platforms_frameworks(self):
        out, _ = run(parity.cmd_coverage, platform="ipados", group=None, min=None, max=None, sort="name", json=False)
        self.assertIn("Core Data", out)
        self.assertIn("Touch Kit", out)
        self.assertNotIn("Desktop Only", out)
        out, _ = run(parity.cmd_coverage, platform="macos", group=None, min=None, max=None, sort="name", json=False)
        self.assertIn("Desktop Only", out)
        self.assertNotIn("Touch Kit", out)

    def test_show_names_where_the_rows_came_from(self):
        out, _ = run(parity.cmd_show, platform="ipados", name="core data")
        self.assertIn("since 3.0, rows from macOS", out)
        self.assertIn("Contexts", out)
        out, _ = run(parity.cmd_show, platform="ipados", name="touch")
        self.assertNotIn("from macOS", out)
        _, code = run(parity.cmd_show, platform="ipados", name="desktop")
        self.assertIn("no framework matches", str(code))

    def test_gaps_for_the_tablet(self):
        out, _ = run(parity.cmd_gaps, platform="ipados", group=None, status=None)
        self.assertIn("## Hardware", out)
        self.assertIn("Touch Kit", out)
        self.assertIn("Core Data", out)
        self.assertNotIn("Desktop Only", out)

    def test_check_passes_when_every_tablet_framework_is_assessed(self):
        out, code = run(parity.cmd_check, platform="ipados")
        self.assertEqual(code, 0, out)
        self.assertIn("4 entries", out)

    def test_check_fails_for_an_unassessed_tablet_framework(self):
        os.remove(os.path.join(self.tree.ipados, "data", "touch-kit.json"))
        out, code = run(parity.cmd_check, platform="ipados")
        self.assertEqual(code, 1)
        self.assertIn("not assessed: touch-kit", out)

    def test_check_flags_a_tablet_file_that_is_not_in_the_index(self):
        Tree.write(os.path.join(self.tree.ipados, "data", "stray.json"), fw("Stray", "gap", [row("x", "gap")]))
        out, code = run(parity.cmd_check, platform="ipados")
        self.assertEqual(code, 1)
        self.assertIn("FAIL stray: not in the ipados index", out)

    def test_check_flags_a_cited_path_that_is_missing(self):
        Tree.write(os.path.join(self.tree.ipados, "data", "touch-kit.json"),
                   fw("Touch Kit", "partial", [row("Touch", "parity", "springboard/missing.rs")]))
        out, code = run(parity.cmd_check, platform="ipados")
        self.assertEqual(code, 1)
        self.assertIn("cited path not found: springboard/missing.rs", out)

    def test_macos_check_ignores_the_tablet_side(self):
        os.remove(os.path.join(self.tree.ipados, "data", "touch-kit.json"))
        with mock.patch.object(parity, "os_check", return_value=[]), mock.patch.object(parity, "check_apps", return_value=[]):
            out, code = run(parity.cmd_check)
        self.assertEqual(code, 0, out)

    def test_the_cli_accepts_platform(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            parity.main(["status", "--platform", "ipados"])
            parity.main(["coverage", "--platform", "ipados", "--sort", "name"])
        self.assertIn("Platform ipados", out.getvalue())
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parity.main(["status", "--platform", "watchos"])


class Committed(unittest.TestCase):
    def test_the_committed_tablet_index_is_well_formed(self):
        index = parity.load_index("ipados")
        self.assertTrue(index)
        slugs = [r["slug"] for r in index]
        self.assertEqual(len(slugs), len(set(slugs)))
        for r in index:
            self.assertTrue(r["ipados_since"] != "" and r["doc"].startswith("https://developer.apple.com/"), r["slug"])

    def test_the_committed_tablet_entries_load(self):
        entries, bad = parity.load_entries("ipados")
        self.assertEqual(bad, [])
        self.assertTrue(entries)


if __name__ == "__main__":
    unittest.main()
