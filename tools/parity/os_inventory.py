#!/usr/bin/env python3
"""Walk an extracted reference OS image into the OS layer of the parity matrix (docs/macos-os-inventory.md).

Usage: os_inventory.py ROOT [--dyld-json FILE] [--out docs/parity-matrix/os] [--source "macOS 27.0.1 26A434"]

ROOT is the extracted system volume (the tree with System/, usr/, Applications/ ...). The walker records names,
identifiers, versions, structure and interfaces only (clean-room rule): no file contents are copied. It writes one
JSON file per (category, shard) with entries whose description, BifrOSt counterpart and status are left for the
assessment team (status "unassessed"). Re-running keeps every assessed field of an entry with the same id.
"""
import argparse
import json
import os
import plistlib
import re
import struct
import sys

CATEGORIES = ("apps", "system-ui", "settings", "daemons", "frameworks", "drivers", "shaders", "cli", "formats", "assets")
ASSESSED_FIELDS = ("description", "bifrost", "status", "note", "rea", "doc", "role")
SYSTEM_UI = {
    "Dock", "Finder", "ControlCenter", "NotificationCenter", "SystemUIServer", "Spotlight", "loginwindow",
    "WindowManager", "Siri", "TextInputMenuAgent", "Wallpaper", "WallpaperAgent", "AirPlayUIAgent", "Ticket Viewer",
    "screencaptureui", "UserNotificationCenter", "CoreServicesUIAgent", "universalAccessAuthWarn",
}
ASSET_EXT = {".ttf": "font", ".ttc": "font", ".otf": "font", ".aiff": "sound", ".caf": "sound", ".m4a": "sound",
             ".heic": "image", ".car": "asset catalog", ".icns": "icon", ".cur": "cursor", ".pdf": "pdf"}


def read_plist(path):
    try:
        with open(path, "rb") as f:
            return plistlib.load(f)
    except Exception:
        return None


def bundle_info(bundle):
    for p in (os.path.join(bundle, "Contents", "Info.plist"), os.path.join(bundle, "Info.plist"),
              os.path.join(bundle, "Resources", "Info.plist"), os.path.join(bundle, "Versions", "A", "Resources", "Info.plist")):
        d = read_plist(p)
        if isinstance(d, dict):
            return d
    return {}


def metallib_functions(path, limit=4096):
    """Entry-point names of a Metal library: the NAME tags of its function list (header 'MTLB')."""
    try:
        with open(path, "rb") as f:
            data = f.read(64 << 20)
    except Exception:
        return None, []
    if data[:4] != b"MTLB":
        return None, []
    names = []
    for m in re.finditer(rb"NAME(..)", data):
        (n,) = struct.unpack("<H", m.group(1))
        start = m.end()
        raw = data[start:start + n]
        if 0 < n < 512 and raw.endswith(b"\x00"):
            s = raw[:-1].decode("utf-8", "replace")
            if s.isprintable() and s not in names:
                names.append(s)
        if len(names) >= limit:
            break
    version = struct.unpack("<HH", data[4:8]) if len(data) >= 8 else None
    return version, names


def shard_of(name):
    c = name[:1].lower()
    if c.isdigit() or not c.isalpha():
        return "0-9"
    for lo, hi in (("a", "c"), ("d", "f"), ("g", "i"), ("j", "l"), ("m", "o"), ("p", "r"), ("s", "s"), ("t", "v"), ("w", "z")):
        if lo <= c <= hi:
            return f"{lo}-{hi}"
    return "other"


def entry(category, eid, path, kind, name, **kw):
    e = {"id": eid, "category": category, "name": name, "path": path, "kind": kind, "identifier": "", "version": "",
         "owner": "", "facts": {}, "description": "", "role": "", "doc": "", "bifrost": "", "status": "unassessed",
         "note": "", "rea": []}
    e.update(kw)
    return e


def walk(root, dyld_json):
    out = []
    rel = lambda p: "/" + os.path.relpath(p, root)

    def apps_in(d, category):
        if not os.path.isdir(d):
            return
        for n in sorted(os.listdir(d)):
            p = os.path.join(d, n)
            if n.endswith(".app"):
                info = bundle_info(p)
                base = n[:-4]
                cat = "system-ui" if base in SYSTEM_UI else category
                out.append(entry(cat, f"app:{info.get('CFBundleIdentifier', base)}", rel(p), "app", base,
                                 identifier=info.get("CFBundleIdentifier", ""),
                                 version=str(info.get("CFBundleShortVersionString", "")),
                                 facts={k: info[k] for k in ("LSUIElement", "NSPrincipalClass", "LSApplicationCategoryType",
                                                             "CFBundleDocumentTypes") if k in info and k != "CFBundleDocumentTypes"}))
                for dt in info.get("CFBundleDocumentTypes", []) or []:
                    if isinstance(dt, dict):
                        for u in dt.get("LSItemContentTypes", []) or []:
                            out.append(entry("formats", f"doctype:{u}:{info.get('CFBundleIdentifier', base)}", rel(p),
                                             "document type", u, owner=info.get("CFBundleIdentifier", base),
                                             facts={"role": dt.get("CFBundleTypeRole", "")}))
                for scheme in info.get("CFBundleURLTypes", []) or []:
                    if isinstance(scheme, dict):
                        for s in scheme.get("CFBundleURLSchemes", []) or []:
                            out.append(entry("formats", f"scheme:{s}:{info.get('CFBundleIdentifier', base)}", rel(p),
                                             "url scheme", s, owner=info.get("CFBundleIdentifier", base)))

    apps_in(os.path.join(root, "Applications"), "apps")
    apps_in(os.path.join(root, "System", "Applications"), "apps")
    apps_in(os.path.join(root, "System", "Applications", "Utilities"), "apps")
    apps_in(os.path.join(root, "System", "Library", "CoreServices"), "system-ui")
    apps_in(os.path.join(root, "System", "Library", "CoreServices", "Applications"), "apps")

    for d in ("ExtensionKit/Extensions", "PreferencePanes"):
        base = os.path.join(root, "System", "Library", d)
        if os.path.isdir(base):
            for n in sorted(os.listdir(base)):
                p = os.path.join(base, n)
                info = bundle_info(p)
                ident = info.get("CFBundleIdentifier", n)
                if d.startswith("Pref") or "settings" in ident.lower() or "Settings" in n:
                    out.append(entry("settings", f"settings:{ident}", rel(p), "settings pane" if d.startswith("Pref") else "settings extension",
                                     os.path.splitext(n)[0], identifier=ident, version=str(info.get("CFBundleShortVersionString", ""))))

    for d in ("LaunchDaemons", "LaunchAgents"):
        base = os.path.join(root, "System", "Library", d)
        if os.path.isdir(base):
            for n in sorted(os.listdir(base)):
                pl = read_plist(os.path.join(base, n)) or {}
                label = pl.get("Label", n[:-6] if n.endswith(".plist") else n)
                prog = pl.get("Program") or (pl.get("ProgramArguments") or [""])[0]
                out.append(entry("daemons", f"launchd:{label}", rel(os.path.join(base, n)), d[:-1].lower(), label, identifier=label,
                                 facts={"program": prog, "mach_services": sorted((pl.get("MachServices") or {}).keys())[:32],
                                        "run_at_load": pl.get("RunAtLoad", False), "keep_alive": bool(pl.get("KeepAlive"))}))

    for d in ("Frameworks", "PrivateFrameworks"):
        base = os.path.join(root, "System", "Library", d)
        if os.path.isdir(base):
            for n in sorted(os.listdir(base)):
                if n.endswith(".framework"):
                    p = os.path.join(base, n)
                    info = bundle_info(p)
                    out.append(entry("frameworks", f"framework:{n[:-10]}", rel(p), "public framework" if d == "Frameworks" else "private framework",
                                     n[:-10], identifier=info.get("CFBundleIdentifier", ""), version=str(info.get("CFBundleShortVersionString", ""))))

    for d in ("Extensions", "DriverExtensions"):
        base = os.path.join(root, "System", "Library", d)
        if os.path.isdir(base):
            for n in sorted(os.listdir(base)):
                p = os.path.join(base, n)
                info = bundle_info(p)
                out.append(entry("drivers", f"driver:{info.get('CFBundleIdentifier', n)}", rel(p), "kext" if n.endswith(".kext") else "dext",
                                 os.path.splitext(n)[0], identifier=info.get("CFBundleIdentifier", ""),
                                 version=str(info.get("CFBundleShortVersionString", "")),
                                 facts={"personalities": sorted((info.get("IOKitPersonalities") or {}).keys())[:24]}))

    for d in ("usr/bin", "usr/sbin", "usr/libexec", "bin", "sbin"):
        base = os.path.join(root, d)
        if os.path.isdir(base):
            for n in sorted(os.listdir(base)):
                p = os.path.join(base, n)
                if os.path.isfile(p):
                    out.append(entry("cli", f"cli:/{d}/{n}", rel(p), "tool", n))

    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [x for x in dirnames if not x.startswith(".")]
        for n in filenames:
            p = os.path.join(dirpath, n)
            ext = os.path.splitext(n)[1].lower()
            if ext == ".metallib":
                ver, fns = metallib_functions(p)
                owner = re.search(r"/([^/]+)\.(framework|app|bundle|dylib)/", rel(p))
                out.append(entry("shaders", f"metallib:{rel(p)}", rel(p), "metal library", n, owner=owner.group(1) if owner else "",
                                 facts={"functions": fns, "function_count": len(fns), "format_version": ver}))
            elif ext in ASSET_EXT and ("/Fonts/" in p or "/Sounds/" in p or "/Desktop Pictures/" in p or "/Cursors/" in p
                                       or "/Wallpaper" in p or ext in (".ttf", ".otf", ".ttc")):
                out.append(entry("assets", f"asset:{rel(p)}", rel(p), ASSET_EXT[ext], n))

    if dyld_json:
        try:
            images = json.load(open(dyld_json))
        except Exception:
            images = []
        have = {e["path"] for e in out}
        for img in images if isinstance(images, list) else images.get("dylibs", []):
            path = img.get("name") or img.get("path") or ""
            if not path or path in have:
                continue
            m = re.search(r"/([^/]+)\.framework/", path)
            name = m.group(1) if m else os.path.basename(path)
            cat = "frameworks"
            out.append(entry(cat, f"image:{path}", path, "dyld image", name, version=str(img.get("version", ""))))
    return out


def merge(existing, fresh):
    old = {e["id"]: e for e in existing}
    for e in fresh:
        o = old.get(e["id"])
        if o:
            for k in ASSESSED_FIELDS:
                if o.get(k):
                    e[k] = o[k]
    return fresh


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("--dyld-json")
    ap.add_argument("--out", default=os.path.join(os.path.dirname(__file__), "..", "..", "docs", "parity-matrix", "os"))
    ap.add_argument("--source", default="macOS 27.0.1 Golden Gate 26A434")
    a = ap.parse_args(argv)
    entries = walk(a.root, a.dyld_json)
    seen = set()
    uniq = []
    for e in entries:
        if e["id"] not in seen:
            seen.add(e["id"])
            uniq.append(e)
    by = {}
    for e in uniq:
        by.setdefault((e["category"], shard_of(e["name"])), []).append(e)
    total = 0
    for (cat, shard), es in sorted(by.items()):
        d = os.path.join(a.out, cat)
        os.makedirs(d, exist_ok=True)
        p = os.path.join(d, shard + ".json")
        old = json.load(open(p)).get("entries", []) if os.path.exists(p) else []
        es = merge(old, sorted(es, key=lambda e: e["id"]))
        with open(p, "w") as f:
            json.dump({"category": cat, "shard": shard, "source": a.source, "entries": es}, f, indent=1, ensure_ascii=False)
            f.write("\n")
        total += len(es)
    counts = {}
    for e in uniq:
        counts[e["category"]] = counts.get(e["category"], 0) + 1
    print(f"{total} entries in {len(by)} shards: " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))


if __name__ == "__main__":
    main()
