#!/usr/bin/env python3
"""Unit tests for tools/parity/os_frameworks.py: name matching, aliases, status mapping and owner patterns."""
import importlib.util
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location("os_frameworks", HERE / "os_frameworks.py")
of = importlib.util.module_from_spec(spec)
spec.loader.exec_module(of)


def sdk_file(framework, status, rows=(), file=None):
    return {"framework": framework, "status": status, "features": [{"status": s} for s in rows],
            "_file": file or of.key(framework) + ".json"}


def sdk_of(*files):
    out = {}
    for d in files:
        for k in {of.key(of.base_name(d["framework"])), of.key(d["framework"])}:
            out.setdefault(k, []).append(d)
    return out


def row(name, kind="public framework", **kw):
    r = {"name": name, "kind": kind, "status": "gap", "bifrost": "", "note": "heuristic: x", "method": "heuristic"}
    r.update(kw)
    return r


class Matching(unittest.TestCase):
    def test_key_and_base_name(self):
        self.assertEqual(of.key("Core MIDI"), "coremidi")
        self.assertEqual(of.base_name("Foundation (Fundamentals)"), "Foundation")
        self.assertEqual(of.base_name("Core Text"), "Core Text")

    def test_exact_name_ignores_spaces(self):
        sdk = sdk_of(sdk_file("Core MIDI", "partial", ["partial"]))
        self.assertEqual([d["_file"] for d in of.sdk_files_for("CoreMIDI", sdk)], ["coremidi.json"])

    def test_split_framework_matches_all_parts(self):
        sdk = sdk_of(sdk_file("Foundation (Fundamentals)", "partial", file="foundation-fundamentals.json"),
                     sdk_file("Foundation (App Support)", "partial", file="foundation-app-support.json"),
                     sdk_file("Foundation Models", "partial", file="foundation-models.json"))
        names = [d["_file"] for d in of.sdk_files_for("Foundation", sdk)]
        self.assertEqual(sorted(names), ["foundation-app-support.json", "foundation-fundamentals.json"])

    def test_alias_rename(self):
        sdk = sdk_of(sdk_file("PhotoKit", "partial", file="photokit.json"))
        self.assertEqual(len(of.sdk_files_for("Photos", sdk)), 1)

    def test_alias_cocoa_spans_two_frameworks(self):
        sdk = sdk_of(sdk_file("AppKit (Text)", "partial"), sdk_file("Foundation (Fundamentals)", "partial"))
        self.assertEqual(len(of.sdk_files_for("Cocoa", sdk)), 2)

    def test_unknown_name_has_no_files(self):
        self.assertEqual(of.sdk_files_for("Nonexistent", sdk_of()), [])


class StatusMapping(unittest.TestCase):
    def test_parity_needs_every_applicable_row(self):
        self.assertEqual(of.file_status(sdk_file("X", "parity", ["parity", "n/a"])), "parity")
        self.assertEqual(of.file_status(sdk_file("X", "parity", ["parity", "gap"])), "partial")

    def test_partial_when_any_built(self):
        self.assertEqual(of.file_status(sdk_file("X", "partial", ["gap", "partial"])), "partial")

    def test_na_and_os_stay(self):
        self.assertEqual(of.file_status(sdk_file("X", "n/a", ["gap"])), "n/a")
        self.assertEqual(of.file_status(sdk_file("X", "os", [])), "os")

    def test_gap_stays_gap(self):
        self.assertEqual(of.file_status(sdk_file("X", "gap", ["gap"])), "gap")

    def test_combine(self):
        self.assertEqual(of.combine(["n/a", "n/a"]), "n/a")
        self.assertEqual(of.combine(["os", "os"]), "os")
        self.assertEqual(of.combine(["parity", "parity"]), "parity")
        self.assertEqual(of.combine(["parity", "partial"]), "partial")
        self.assertEqual(of.combine(["n/a", "partial"]), "partial")
        self.assertEqual(of.combine(["n/a", "gap"]), "gap")
        self.assertEqual(of.combine(["n/a", "os"]), "os")


class PublicRows(unittest.TestCase):
    def test_public_row_gets_status_bifrost_and_note(self):
        sdk = sdk_of(sdk_file("Core Text", "partial", ["partial"], file="core-text.json"))
        r = row("CoreText")
        of.sync([r], sdk, [])
        self.assertEqual((r["status"], r["method"]), ("partial", "assessed"))
        self.assertEqual(r["bifrost"], "docs/parity-matrix/data/core-text.json")
        self.assertEqual(r["note"], "from SDK matrix core-text.json")

    def test_row_without_sdk_file_is_untouched(self):
        r = row("Carbon")
        self.assertEqual(of.sync([r], sdk_of(), []), 0)
        self.assertEqual((r["status"], r["method"]), ("gap", "heuristic"))

    def test_overlay_module_uses_first_part(self):
        sdk = sdk_of(sdk_file("App Intents", "partial", ["partial"], file="app-intents.json"))
        res = of.assess_public("_AppIntents_AppKit", sdk)
        self.assertEqual(res[0], "partial")
        self.assertIn("owner AppIntents", res[2])

    def test_hand_assessed_row_is_left_alone(self):
        sdk = sdk_of(sdk_file("Core Text", "partial", ["partial"]))
        r = row("CoreText", method=None, note="reviewed by hand", status="parity")
        self.assertEqual(of.sync([r], sdk, []), 0)
        self.assertEqual(r["status"], "parity")

    def test_rerun_is_stable(self):
        sdk = sdk_of(sdk_file("Core Text", "partial", ["partial"]))
        r = row("CoreText")
        of.sync([r], sdk, [])
        self.assertEqual(of.sync([r], sdk, []), 0)


class Owners(unittest.TestCase):
    RULES = [{"pattern": "*AppIntents*", "owner": "AppIntents"},
             {"pattern": "*Health*", "owner": "n/a", "reason": "HealthKit data is not mirrored"},
             {"pattern": "CoreSpotlight*", "owner": "CoreSpotlight"}]

    def test_pattern_is_case_insensitive_and_first_wins(self):
        self.assertEqual(of.owner_for("_appintentsservices_toolkit", self.RULES, []), ("AppIntents", ""))
        self.assertEqual(of.owner_for("CoreSpotlightUI", self.RULES, [])[0], "CoreSpotlight")

    def test_na_owner_carries_reason(self):
        self.assertEqual(of.owner_for("HealthDaemon", self.RULES, []), ("n/a", "HealthKit data is not mirrored"))

    def test_prefix_fallback_takes_longest_public_name(self):
        self.assertEqual(of.owner_for("AccessibilityUI", [], ["Access", "Accessibility"])[0], "Accessibility")

    def test_short_prefixes_are_ignored(self):
        self.assertEqual(of.owner_for("SystemFoo", [], ["Sys"]), (None, ""))

    def test_no_owner_is_none(self):
        self.assertEqual(of.owner_for("Zzzz", self.RULES, ["Metal"]), (None, ""))

    def test_private_inherits_owner_status_capped_at_partial(self):
        sdk = sdk_of(sdk_file("Core Spotlight", "parity", ["parity"], file="core-spotlight.json"))
        st, files, note = of.assess_private("CoreSpotlightUI", self.RULES, [], sdk)
        self.assertEqual(st, "partial")
        self.assertIn("owner CoreSpotlight", note)
        self.assertEqual(files[0]["_file"], "core-spotlight.json")

    def test_private_na_owner(self):
        st, files, note = of.assess_private("HealthDaemon", self.RULES, [], sdk_of())
        self.assertEqual((st, files), ("n/a", []))
        self.assertIn("HealthKit", note)

    def test_private_without_owner_stays_os_with_note(self):
        st, _, note = of.assess_private("Zzzz", self.RULES, [], sdk_of())
        self.assertEqual(st, "os")
        self.assertEqual(note, of.NOTE_PRIVATE_NONE)

    def test_owner_without_sdk_file_stays_os(self):
        st, _, note = of.assess_private("AppIntentsX", self.RULES, [], sdk_of())
        self.assertEqual(st, "os")
        self.assertIn("no SDK file", note)


class OwnerMapFile(unittest.TestCase):
    def test_every_rule_has_pattern_and_owner(self):
        for r in of.load_rules():
            self.assertTrue(r["pattern"] and r["owner"])
            if r["owner"] == "n/a":
                self.assertTrue(r.get("reason"), r["pattern"])


if __name__ == "__main__":
    unittest.main()
