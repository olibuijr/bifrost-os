#!/usr/bin/env python3
"""SDK API coverage guard: which `pub fn` in springboard/sdk/src are named by any test.

A `pub fn NAME` counts as covered when NAME appears as a whole word in test code:
inside a `#[cfg(test)]` item of a source file, in a file named tests*.rs, or in
springboard/sdk/tests/*.rs. This is a name match, not a call-graph analysis, so a
common name such as `new` is covered by any test that mentions it.

Usage: tools/check-sdk-coverage.py [--min PCT] [--json]
  --min PCT  exit 1 when total coverage is below PCT percent
  --json     print the report as JSON instead of text
Stdlib only, reads files only (no build).
"""
import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SDK = ROOT / "springboard" / "sdk"

PUB_FN = re.compile(r"^\s*pub(?:\([^)]*\))?\s+(?:(?:async|unsafe|const|extern\s+\"C\")\s+)*fn\s+([A-Za-z_]\w*)", re.M)
CFG_TEST = re.compile(r"#\[cfg\(test\)\]")
ITEM_BODY = re.compile(r"\b(?:mod|fn)\s+\w+[^;{]*\{")


def block_end(text, open_idx):
    """Index just past the brace that closes the one at open_idx (naive: ignores braces in strings)."""
    depth = 0
    for i in range(open_idx, len(text)):
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i + 1
    return len(text)


def test_ranges(text):
    """Character ranges of `#[cfg(test)]` items (the module or fn body that follows the attribute)."""
    ranges = []
    for m in CFG_TEST.finditer(text):
        body = ITEM_BODY.search(text, m.end())  # no `;` allowed in between: a `use` or `const` is not a body
        if body:
            ranges.append((body.end() - 1, block_end(text, body.end() - 1)))
    return ranges


def module_name(path):
    rel = path.relative_to(SDK / "src").with_suffix("")
    parts = list(rel.parts)
    if parts[-1] == "mod":
        parts.pop()
    return "::".join(parts) or "crate"


def scan(root=SDK):
    """Return (per_module, uncovered, total, covered) from the SDK sources and tests."""
    defs = []  # (module, name, file, line)
    test_text = []
    for path in sorted((root / "src").rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        whole_file_is_test = path.name.startswith("tests") and path.name.endswith(".rs")
        if whole_file_is_test:
            test_text.append(text)
            continue
        ranges = test_ranges(text)
        for start, end in ranges:
            test_text.append(text[start:end])
        for m in PUB_FN.finditer(text):
            if any(s <= m.start() < e for s, e in ranges):
                continue
            line = text.count("\n", 0, m.start()) + 1
            defs.append((module_name(path), m.group(1), str(path.relative_to(ROOT)), line))
    for path in sorted((root / "tests").glob("*.rs")):
        test_text.append(path.read_text(encoding="utf-8", errors="replace"))

    corpus = "\n".join(test_text)
    names = set(re.findall(r"\b[A-Za-z_]\w*\b", corpus))
    per_module = {}
    uncovered = []
    for module, name, file, line in defs:
        stats = per_module.setdefault(module, {"covered": 0, "total": 0})
        stats["total"] += 1
        if name in names:
            stats["covered"] += 1
        else:
            uncovered.append({"module": module, "fn": name, "file": file, "line": line})
    total = sum(s["total"] for s in per_module.values())
    covered = sum(s["covered"] for s in per_module.values())
    return per_module, uncovered, total, covered


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--min", type=float, default=None, metavar="PCT", help="fail (exit 1) below this percentage")
    ap.add_argument("--json", action="store_true", help="print JSON")
    args = ap.parse_args()

    per_module, uncovered, total, covered = scan()
    pct = 100.0 * covered / total if total else 100.0
    ok = args.min is None or pct >= args.min

    if args.json:
        print(json.dumps({"covered": covered, "total": total, "percent": round(pct, 2),
                          "min": args.min, "modules": per_module, "uncovered": uncovered}, indent=2))
    else:
        width = max([len(m) for m in per_module] + [6])
        print(f"{'module':<{width}}  covered/total")
        for module in sorted(per_module):
            s = per_module[module]
            print(f"{module:<{width}}  {s['covered']}/{s['total']}")
        print(f"{'TOTAL':<{width}}  {covered}/{total}  ({pct:.1f} %)")
        if uncovered:
            print(f"\nUncovered pub fns ({len(uncovered)}):")
            for u in uncovered:
                print(f"  {u['module']}::{u['fn']}  {u['file']}:{u['line']}")
        if args.min is not None:
            print(f"\n{'PASS' if ok else 'FAIL'}: coverage {pct:.1f} % {'>=' if ok else '<'} minimum {args.min:.1f} %")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
