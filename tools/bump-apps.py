#!/usr/bin/env python3
"""Patch-bump app versions for a release.

  bump-apps.py DIR=NOTE [DIR=NOTE ...]   bump the named apps (DIR under springboard/apps, or a path)
  bump-apps.py --all-sdk NOTE            bump every app that depends on bifrost-app or bifrost-proto,
                                         except the Craft apps (ai.storyteller.*), FilmCraft and Deildu

Each bump raises manifest.toml and Cargo.toml together, the app's entry in springboard/Cargo.lock, and adds
"## X.Y.Z" with "- NOTE" at the top of the app's CHANGELOG.md. Prints `dir old->new id`. An empty NOTE is refused.
Stdlib only.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APPS = os.path.join(ROOT, "springboard", "apps")
LOCK = os.path.join(ROOT, "springboard", "Cargo.lock")
VERSION = re.compile(r'^(version\s*=\s*")(\d+)\.(\d+)\.(\d+)(")', re.M)
SDK_DEP = re.compile(r"^bifrost-(app|proto)\s*[=.]", re.M)


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def write(path, text):
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def bump_version(v):
    major, minor, patch = (int(x) for x in v.split("."))
    return f"{major}.{minor}.{patch + 1}"


def section(text, header):
    """(start, end) of the body of `[header]`, or of the top level when header is None."""
    if header is None:
        m = re.search(r"^\[", text, re.M)
        return 0, m.start() if m else len(text)
    m = re.search(r"^\[" + re.escape(header) + r"\]\s*$", text, re.M)
    if not m:
        return None
    nxt = re.search(r"^\[", text[m.end():], re.M)
    return m.end(), m.end() + nxt.start() if nxt else len(text)


def file_version(text, header):
    span = section(text, header)
    m = span and VERSION.search(text, *span)
    return m and "{}.{}.{}".format(*m.group(2, 3, 4))


def set_version(text, header, new):
    start, end = section(text, header)
    m = VERSION.search(text, start, end)
    return text[: m.start()] + m.group(1) + new + m.group(5) + text[m.end():]


def set_lock_version(text, pkg, old, new):
    pat = re.compile(r'(^name = "' + re.escape(pkg) + r'"\nversion = ")' + re.escape(old) + '"', re.M)
    return pat.subn(r"\g<1>" + new + '"', text)


def add_changelog(text, new, note):
    entry = f"## {new}\n\n- {note}\n\n"
    m = re.search(r"^## ", text, re.M)
    if m:
        return text[: m.start()] + entry + text[m.start():]
    return (text.rstrip("\n") or "# Changelog") + "\n\n" + entry.rstrip("\n") + "\n"


def app_id(manifest):
    m = re.search(r'^id\s*=\s*"([^"]+)"', manifest, re.M)
    return m.group(1) if m else "?"


def bump_app(app_dir, note, lock=LOCK):
    """Bump one app; returns (old, new, id). Raises ValueError before writing anything on a bad app."""
    note = note.strip()
    if not note:
        raise ValueError(f"{app_dir}: empty NOTE")
    cargo_path = os.path.join(app_dir, "Cargo.toml")
    manifest_path = os.path.join(app_dir, "manifest.toml")
    if not os.path.exists(cargo_path):
        raise ValueError(f"{app_dir}: no Cargo.toml")
    cargo = read(cargo_path)
    old = file_version(cargo, "package")
    if not old:
        raise ValueError(f"{app_dir}: no [package] version")
    manifest = read(manifest_path) if os.path.exists(manifest_path) else None
    if manifest is not None and file_version(manifest, None) != old:
        raise ValueError(f"{app_dir}: manifest version {file_version(manifest, None)} differs from Cargo.toml {old}")
    new = bump_version(old)
    pkg = re.search(r'^name\s*=\s*"([^"]+)"', cargo[section(cargo, "package")[0]:], re.M).group(1)
    write(cargo_path, set_version(cargo, "package", new))
    if manifest is not None:
        write(manifest_path, set_version(manifest, None, new))
    if os.path.exists(lock):
        text, n = set_lock_version(read(lock), pkg, old, new)
        if n:
            write(lock, text)
        else:
            print(f"warning: {pkg} {old} not in {lock}", file=sys.stderr)
    log_path = os.path.join(app_dir, "CHANGELOG.md")
    write(log_path, add_changelog(read(log_path) if os.path.exists(log_path) else "", new, note))
    return old, new, app_id(manifest) if manifest is not None else pkg


def sdk_apps(apps=APPS):
    """Apps that depend on the SDK, minus the Craft apps, FilmCraft and Deildu."""
    out = []
    for name in sorted(os.listdir(apps)):
        d = os.path.join(apps, name)
        cargo = os.path.join(d, "Cargo.toml")
        if not os.path.exists(cargo):
            continue
        text = read(cargo)
        span = section(text, "dependencies")
        if not span or not SDK_DEP.search(text, *span):
            continue
        manifest_path = os.path.join(d, "manifest.toml")
        ident = app_id(read(manifest_path)) if os.path.exists(manifest_path) else ""
        if "deildu" in name or "filmcraft" in name or "deildu" in ident or ident.startswith("ai.storyteller."):
            continue
        out.append(d)
    return out


def main(argv):
    if argv[:1] == ["--all-sdk"] and len(argv) == 2:
        jobs = [(d, argv[1]) for d in sdk_apps()]
    elif argv and "--all-sdk" not in argv and all("=" in a for a in argv):
        jobs = []
        for a in argv:
            name, note = a.split("=", 1)
            jobs.append((name if os.path.isdir(name) else os.path.join(APPS, name), note))
    else:
        print(__doc__, file=sys.stderr)
        return 2
    if any(not note.strip() for _, note in jobs):
        print("refused: empty NOTE", file=sys.stderr)
        return 1
    for d, note in jobs:
        try:
            old, new, ident = bump_app(d, note)
        except ValueError as e:
            print(f"error: {e}", file=sys.stderr)
            return 1
        print(f"{os.path.basename(os.path.normpath(d))} {old}->{new} {ident}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
