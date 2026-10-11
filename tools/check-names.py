#!/usr/bin/env python3
"""Owner rule: the code never uses the name of the reference platform or its products.

Scans springboard/, tools/ and packaging/ (*.rs *.py *.sh *.js *.toml *.html *.css) and fails on any hit.
Allowlist: tools/check-names-allow.txt, lines `path-glob:pattern:reason` (pattern is a regex matched
against the offending line; `*` matches any). Usage: check-names.py [--summary]
"""
import fnmatch
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIRS = ["springboard", "tools", "packaging"]
EXTS = (".rs", ".py", ".sh", ".js", ".toml", ".html", ".css")
SKIP_DIRS = {"target", "node_modules", ".git"}
CI = re.compile(
    r"(?<![A-Za-z0-9])(apple\w*|mac ?os\w*|os x|mac os x|finder|safari|spotlight|launchpad|mission control|airdrop|quick ?look|"
    r"fsevent\w*|appkit|uikit|swiftui|ipados|iphone|ipad|ios|iosterm|xcode|icloud|siri|sf symbols|cupertino)(?![A-Za-z0-9])",
    re.I,
)
CS = re.compile(r"(?<![A-Za-z0-9])(NS[A-Z][a-z]\w*|WK[A-Z][a-z]\w*|UI[A-Z][a-z]\w*)")
SELF = os.path.relpath(os.path.abspath(__file__), ROOT)


def load_allow():
    out = []
    path = os.path.join(ROOT, "tools", "check-names-allow.txt")
    if os.path.exists(path):
        for ln in open(path, encoding="utf-8"):
            ln = ln.strip()
            if not ln or ln.startswith("#"):
                continue
            parts = ln.split(":", 2)
            if len(parts) == 3:
                out.append((parts[0], re.compile(parts[1] if parts[1] != "*" else "."), parts[2]))
    return out


def main():
    allow = load_allow()
    hits = []
    for d in DIRS:
        for dp, dn, fn in os.walk(os.path.join(ROOT, d)):
            dn[:] = [x for x in dn if x not in SKIP_DIRS]
            for f in fn:
                if not f.endswith(EXTS):
                    continue
                full = os.path.join(dp, f)
                rel = os.path.relpath(full, ROOT)
                if rel in (SELF, "tools/check-names-allow.txt"):
                    continue
                try:
                    lines = open(full, encoding="utf-8").read().split("\n")
                except (UnicodeDecodeError, OSError):
                    continue
                for i, ln in enumerate(lines, 1):
                    # strip allowlisted fragments, then test what remains
                    for g, pat, _ in allow:
                        if fnmatch.fnmatch(rel, g):
                            ln = pat.sub("", ln)
                    if CI.search(ln) or CS.search(ln):
                        hits.append((rel, i, lines[i - 1].strip()[:140]))
    for rel, i, text in hits:
        print(f"{rel}:{i}: {text}")
    if "--summary" in sys.argv or hits:
        by = {}
        for rel, _, _ in hits:
            k = "/".join(rel.split("/")[:2])
            by[k] = by.get(k, 0) + 1
        print(f"check-names: {len(hits)} hit(s)", file=sys.stderr)
        for k, v in sorted(by.items(), key=lambda kv: -kv[1]):
            print(f"  {v:5d} {k}", file=sys.stderr)
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main())
