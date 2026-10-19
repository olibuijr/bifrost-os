#!/usr/bin/env python3
"""Copy the redistributable part of the packed assets into a package or ISO tree.

usage: tools/stage-assets.py [--allowlist FILE] [--replace] [-v] SRC DEST
       tools/stage-assets.py [--allowlist FILE] --list SRC

SRC is the packed asset directory (springboard/assets, untracked). Only regular files whose path relative to SRC
matches a line of packaging/assets-allowlist.txt are copied to DEST (mode 0644); everything else is skipped, so a
stray file in SRC (a vendor font, an icon or sound from another system) can never ship. Symbolic links are never
followed or copied. --replace empties DEST first (the ISO tree is rebuilt, not merged). --list prints the allowed
paths and copies nothing (tools/lab-push.sh builds its manifest from it).

Packages and the ISO are then checked by tools/check-package-licenses.py. Tests: tools/test_stage_assets.py.
"""
import argparse
import os
import re
import shutil
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_ALLOWLIST = os.path.join(ROOT, "packaging", "assets-allowlist.txt")


class AllowlistError(ValueError):
    pass


def glob_to_regex(pattern):
    """`*` and `?` match inside one path segment; every other character is literal."""
    out = []
    for ch in pattern:
        if ch == "*":
            out.append("[^/]*")
        elif ch == "?":
            out.append("[^/]")
        else:
            out.append(re.escape(ch))
    return re.compile("".join(out) + r"\Z")


def parse_allowlist(text, source="allowlist"):
    """Compiled patterns from the allowlist text: one glob per line, `#` starts a comment.
    Blanket and escaping patterns are refused, so the list stays an explicit one."""
    patterns = []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        where = f"{source}:{number}"
        if line.startswith("/") or ".." in line.split("/"):
            raise AllowlistError(f"{where}: patterns are relative to the assets directory, no leading / or ..: {line}")
        if "**" in line:
            raise AllowlistError(f"{where}: ** is not allowed, name the directory: {line}")
        if line[0] in "*?":
            raise AllowlistError(f"{where}: a pattern must start with a literal name, not a wildcard: {line}")
        patterns.append(glob_to_regex(line))
    return patterns


def load_allowlist(path=DEFAULT_ALLOWLIST):
    with open(path, encoding="utf-8") as f:
        return parse_allowlist(f.read(), path)


def classify(src, patterns):
    """(allowed, skipped): sorted POSIX paths of the files under `src`, split by the allowlist.
    Symbolic links and anything that is not a regular file are skipped."""
    allowed, skipped = [], []
    for dirpath, dirnames, filenames in os.walk(src, followlinks=False):
        dirnames.sort()
        for name in sorted(filenames):
            full = os.path.join(dirpath, name)
            rel = os.path.relpath(full, src).replace(os.sep, "/")
            if os.path.islink(full) or not os.path.isfile(full):
                skipped.append(rel)
            elif any(p.match(rel) for p in patterns):
                allowed.append(rel)
            else:
                skipped.append(rel)
    return allowed, skipped


def stage(src, dest, patterns, replace=False):
    """Copy the allowed files of `src` into `dest`; returns (allowed, skipped)."""
    if not os.path.isdir(src):
        raise FileNotFoundError(f"asset directory not found: {src}")
    if os.path.islink(dest):
        # never write through a link (it could lead back into SRC); --replace swaps the link itself for a directory
        if not replace:
            raise ValueError(f"{dest} is a symbolic link; use --replace to swap it for a directory")
    else:
        real_src, real_dest = os.path.realpath(src), os.path.realpath(dest)
        nested = real_src.startswith(real_dest.rstrip(os.sep) + os.sep) or real_dest.startswith(real_src.rstrip(os.sep) + os.sep)
        if nested or real_dest in (real_src, os.path.realpath(os.sep)):
            raise ValueError(f"refusing to stage {src} into {dest}: one holds the other")
    allowed, skipped = classify(src, patterns)
    if replace and os.path.lexists(dest):
        if os.path.islink(dest) or os.path.isfile(dest):
            os.unlink(dest)
        else:
            shutil.rmtree(dest)
    os.makedirs(dest, exist_ok=True)
    for rel in allowed:
        parts = rel.split("/")
        target = os.path.join(dest, *parts)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        shutil.copyfile(os.path.join(src, *parts), target)
        os.chmod(target, 0o644)
    return allowed, skipped


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("src", help="packed asset directory (springboard/assets)")
    ap.add_argument("dest", nargs="?", help="directory to fill (not with --list)")
    ap.add_argument("--allowlist", default=DEFAULT_ALLOWLIST, help="default: packaging/assets-allowlist.txt")
    ap.add_argument("--replace", action="store_true", help="empty DEST first")
    ap.add_argument("--list", action="store_true", help="print the allowed paths, copy nothing")
    ap.add_argument("-v", "--verbose", action="store_true", help="name every skipped file")
    args = ap.parse_args(argv)
    if bool(args.dest) == args.list:
        ap.error("give DEST, or --list without DEST")
    try:
        patterns = load_allowlist(args.allowlist)
        if args.list:
            allowed, skipped = classify(args.src, patterns)
        else:
            allowed, skipped = stage(args.src, args.dest, patterns, args.replace)
    except (OSError, ValueError) as exc:
        print(f"stage-assets: {exc}", file=sys.stderr)
        return 2
    if args.list:
        for rel in allowed:
            print(rel)
    note = f"stage-assets: {len(allowed)} files {'allowed' if args.list else 'copied'}, {len(skipped)} not on the allowlist (skipped)"
    print(note, file=sys.stderr)
    if args.verbose:
        for rel in skipped:
            print(f"  skipped {rel}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
