#!/usr/bin/env python3
"""Sync the OS layer framework rows (docs/parity-matrix/os/frameworks/*.json) from the SDK matrix.

Usage:
  os_frameworks.py            dry run: print what would change
  os_frameworks.py --write    rewrite the shards

Public frameworks take their status from the SDK matrix files (docs/parity-matrix/data/*.json) that assess them.
Private frameworks inherit from the public framework or feature that owns them (tools/parity/os_private_owner_map.json).
A private row is never better than partial: parity needs a Midget check. Rows a person assessed by hand (no
"heuristic" method and no note written by this tool) are left alone. Rerun after SDK rows change.
"""
import argparse
import fnmatch
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DATA = os.path.join(REPO, "docs", "parity-matrix", "data")
SHARDS = os.path.join(REPO, "docs", "parity-matrix", "os", "frameworks")
OWNER_MAP = os.path.join(HERE, "os_private_owner_map.json")
NOTE_SDK = "from SDK matrix "
NOTE_PRIVATE = "private framework, owner "
NOTE_PRIVATE_NONE = "private framework, no plausible owner found"

# OS framework name key -> SDK framework name keys, for renames and split frameworks. Plain names (Core MIDI for
# CoreMIDI, Image I/O for ImageIO) need no entry: keys drop case, spaces and punctuation.
ALIASES = {
    "photos": ["photokit"], "photosui": ["photokit"], "quartzcore": ["coreanimation"],
    "cryptokit": ["applecryptokit"], "charts": ["swiftcharts"], "intents": ["sirikit"], "intentsui": ["sirikit"],
    "identitylookup": ["smsandcallreportingidentitylookup"], "usdkit": ["usdkitappframeworks"],
    "cocoa": ["appkit", "foundation"], "latentsemanticmapping": ["latentsemanticmappingsystem"],
    "mlcompute": ["mlcomputesystem"], "swiftuicore": ["swiftui"], "coremidiserver": ["coremidi"],
    "gamekit": ["gamekitgamecenter"], "passkit": ["passkitapplepayandwallet"],
    "shazamkit": ["shazamkitmedia"], "ituneslibrary": ["ituneslibrary"],
}


def key(s):
    """Loose match key: lower case, letters and digits only."""
    return re.sub(r"[^a-z0-9]", "", (s or "").lower())


def base_name(framework):
    """SDK framework title without the topic part in parentheses: 'Foundation (Fundamentals)' -> 'Foundation'."""
    return re.sub(r"\s*\(.*\)\s*$", "", framework or "").strip()


def load_sdk(data_dir=DATA):
    """{key: [file data]} grouping SDK files by framework base name key (and by file stem for single files)."""
    out = {}
    for f in sorted(os.listdir(data_dir)):
        if not f.endswith(".json"):
            continue
        d = json.load(open(os.path.join(data_dir, f)))
        d["_file"] = f
        for k in {key(base_name(d.get("framework"))), key(d.get("framework"))}:
            if k:
                bucket = out.setdefault(k, [])
                if d not in bucket:
                    bucket.append(d)
    return out


def sdk_files_for(name, sdk):
    """SDK data files that assess an OS framework name, through exact key then the alias table."""
    keys = [key(name)] + ALIASES.get(key(name), [])
    files = []
    for k in keys:
        for d in sdk.get(k, []):
            if d not in files:
                files.append(d)
    return files


def file_status(d):
    """One SDK file's status for the OS layer: n/a and os stay; parity only if every applicable row is parity."""
    st = d.get("status", "gap")
    if st in ("n/a", "os", "gap"):
        return st
    rows = [r.get("status") for r in d.get("features") or [] if r.get("status") != "n/a"]
    if rows and all(r == "parity" for r in rows) and st == "parity":
        return "parity"
    if st == "parity" or any(r in ("parity", "partial") for r in rows) or st == "partial":
        return "partial"
    return "gap"


def combine(statuses):
    """Status of a framework made of several SDK files."""
    s = set(statuses)
    if not s:
        return "gap"
    if s == {"n/a"}:
        return "n/a"
    if s == {"os"}:
        return "os"
    if s == {"parity"}:
        return "parity"
    live = s - {"n/a", "os"}
    if live <= {"gap"}:
        return "gap" if "gap" in live else "os"
    return "partial"


def status_from_files(files):
    return combine([file_status(d) for d in files])


def cross_parts(name):
    """'_AppIntents_AppKit' -> ['AppIntents', 'AppKit']: overlay modules named after the two frameworks they join."""
    if not name.startswith("_") or "_" not in name[1:]:
        return []
    return [p for p in name[1:].split("_") if p]


def assess_public(name, sdk):
    """(status, files, note) for a public framework row, or None when no SDK file assesses it."""
    files = sdk_files_for(name, sdk)
    owner_note = ""
    if not files:
        for part in cross_parts(name):
            files = sdk_files_for(part, sdk)
            if files:
                owner_note = f"overlay module, owner {part}; "
                break
    if not files:
        return None
    names = ", ".join(d["_file"] for d in files)
    return status_from_files(files), files, f"{owner_note}{NOTE_SDK}{names}"


def load_rules(path=OWNER_MAP):
    return json.load(open(path))["rules"]


def owner_for(name, rules, public_names):
    """(owner, reason) for a private framework: first matching rule, else longest public name prefix, else None."""
    low = name.lower()
    for r in rules:
        if fnmatch.fnmatchcase(low, r["pattern"].lower()):
            return r["owner"], r.get("reason", "")
    best = None
    for p in public_names:
        if low.startswith(p.lower()) and (best is None or len(p) > len(best)) and len(p) >= 4:
            best = p
    return (best, "") if best else (None, "")


def assess_private(name, rules, public_names, sdk):
    """(status, files, note) for a private framework row. Never better than partial."""
    owner, reason = owner_for(name, rules, public_names)
    if owner is None:
        return "os", [], NOTE_PRIVATE_NONE
    if owner == "n/a":
        return "n/a", [], f"{NOTE_PRIVATE}n/a: {reason}"
    res = assess_public(owner, sdk)
    if res is None:
        return "os", [], f"{NOTE_PRIVATE}{owner}, which no SDK file assesses yet"
    status, files, _ = res
    if status == "parity":
        status = "partial"
    return status, files, f"{NOTE_PRIVATE}{owner}; {NOTE_SDK}{', '.join(d['_file'] for d in files)}"


def ours(row):
    """True for rows this tool may rewrite: heuristic ones and ones it wrote earlier."""
    n = row.get("note", "")
    return (row.get("method") == "heuristic" or NOTE_SDK in n or n.startswith(NOTE_PRIVATE)
            or "overlay module" in n) and row.get("method") in ("heuristic", "assessed", None)


def apply(row, status, files, note):
    row["status"] = status
    row["bifrost"] = ", ".join(f"docs/parity-matrix/data/{d['_file']}" for d in files)
    row["note"] = note
    row["method"] = "assessed"


def sync(rows, sdk, rules):
    """Reassess rows in place; returns the number changed."""
    public_names = [r["name"] for r in rows if r["kind"] == "public framework" and not r["name"].startswith("_")]
    changed = 0
    for r in rows:
        if not ours(r):
            continue
        if r["kind"] == "public framework":
            res = assess_public(r["name"], sdk)
        elif r["kind"] == "private framework":
            res = assess_private(r["name"], rules, public_names, sdk)
        else:
            continue
        if res is None:
            continue
        before = (r["status"], r["bifrost"], r["note"], r.get("method"))
        apply(r, *res)
        changed += before != (r["status"], r["bifrost"], r["note"], r["method"])
    return changed


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--write", action="store_true", help="rewrite the shards (default is a dry run)")
    ap.add_argument("--data", default=DATA)
    ap.add_argument("--shards", default=SHARDS)
    a = ap.parse_args(argv)
    sdk = load_sdk(a.data)
    rules = load_rules()
    total, changed, tally = 0, 0, {}
    for f in sorted(os.listdir(a.shards)):
        if not f.endswith(".json"):
            continue
        p = os.path.join(a.shards, f)
        doc = json.load(open(p))
        changed += sync(doc["entries"], sdk, rules)
        total += len(doc["entries"])
        for e in doc["entries"]:
            tally[(e["kind"], e["status"])] = tally.get((e["kind"], e["status"]), 0) + 1
        if a.write:
            with open(p, "w") as fh:
                json.dump(doc, fh, indent=1, ensure_ascii=False)
                fh.write("\n")
    print(f"{total} rows, {changed} changed" + ("" if a.write else " (dry run, use --write)"))
    for (kind, st), n in sorted(tally.items()):
        print(f"  {kind:18} {st:8} {n}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
