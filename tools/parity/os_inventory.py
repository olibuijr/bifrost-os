#!/usr/bin/env python3
"""Walk an extracted reference OS image into the OS layer of the parity matrix (docs/macos-os-inventory.md).

Usage:
  os_inventory.py --index INDEX.tsv [--repo DIR] [--out docs/parity-matrix/os] [--source "macOS 27 26A434"]
  os_inventory.py ROOT [--dyld-json FILE] [--out docs/parity-matrix/os] [--source "macOS 27.0.1 26A434"]

--index reads the reference file index (tab-separated type F/D/L, size, path, one line per file, e.g.
~/Projects/macos-ref/extract/ref27-index.tsv) instead of an extracted tree. Index mode has names and paths only, so
document types and URL schemes (which live in Info.plist contents) and dyld images need ROOT mode.

ROOT is the extracted system volume (the tree with System/, usr/, Applications/ ...). The walker records names,
identifiers, versions, structure and interfaces only (clean-room rule): no file contents are copied. It writes one
JSON file per (category, shard). Each entry gets a heuristic BifrOSt counterpart and status (gap, partial, os or n/a)
from names that exist in this repository (method "heuristic"); a person replaces that by setting status and dropping
"method", after which re-running keeps the entry's assessed fields (matched by id). Heuristic entries are re-guessed
on every run and keep only their description, role, doc and rea fields.
"""
import argparse
import json
import os
import plistlib
import re
import struct
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CATEGORIES = ("apps", "system-ui", "settings", "daemons", "frameworks", "drivers", "shaders", "cli", "formats", "assets",
              "input-methods")
ASSESSED_FIELDS = ("description", "bifrost", "status", "note", "rea", "doc", "role")
HEURISTIC_KEEP = ("description", "role", "doc", "rea")
STATUSES = ("parity", "partial", "os", "gap", "n/a")
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
            keep = HEURISTIC_KEEP if o.get("method") == "heuristic" else ASSESSED_FIELDS
            for k in keep:
                if o.get(k):
                    e[k] = o[k]
    return fresh


# ---- index mode: names and paths from the reference file index (no extracted tree needed) ----

APP_ALIASES = {  # reference app name (lowercase, alphanumerics) -> BifrOSt counterpart path (checked to exist)
    "systemsettings": "springboard/src/shell/settings.rs", "finder": "springboard/apps/files",
    "safari": "springboard/apps/browser", "dock": "springboard/src/shell/desktop_dock.rs",
    "controlcenter": "springboard/src/shell/control_center.rs",
}
SETTINGS_RE = re.compile(r"Settings|Preferences|Pref|About|Password|Privacy|Security|Display|Network|Sharing|Keyboard|"
                         r"Mouse|Trackpad|Sound|Bluetooth|Battery|Wallpaper|Appearance|Accessibility|Notification|Focus|"
                         r"Account|Passkey|Siri|Language|Region|Date|Time|Storage|Software|Update|Login|Users", re.I)
UI_DAEMON_RE = re.compile(r"Spotlight|Dock|Finder|Notification|Wallpaper|Siri|Clipboard|Screen|Login|Window|Menu|"
                          r"Control|Keyboard|Input|Search|Launchpad|Share", re.I)
APPLE_CLI_RE = re.compile(r"^(codesign|xcrun|xcodebuild|defaults|plutil|lsregister|mdfind|mdutil|open|pbcopy|pbpaste|"
                          r"osascript|scutil|networksetup|diskutil|hdiutil|tmutil|launchctl|sw_vers|softwareupdate|pmset|"
                          r"system_profiler|say|screencapture|caffeinate|airport)$")
INDEX_RULES = (  # (record type, regex on the relative path, category, kind)
    ("D", r"^System/Applications/System Settings\.app/Contents/PlugIns/[^/]+\.appex$", "settings", "settings extension"),
    ("D", r"/Input Methods/[^/]+\.(app|component|bundle)$", "input-methods", "input method"),
    ("D", r"/Keyboard Layouts/[^/]+\.bundle$", "input-methods", "keyboard layout"),
    ("D", r"^System/Applications/(Utilities/)?[^/]+\.app$", "apps", "app"),
    ("D", r"^Applications/[^/]+\.app$", "apps", "bundled app"),
    ("D", r"^System/Library/CoreServices/Applications/[^/]+\.app$", "apps", "core service app"),
    ("D", r"^System/Library/CoreServices/[^/]+\.app$", "system-ui", "core service"),
    ("D", r"^System/Library/ExtensionKit/Extensions/[^/]+\.appex$", "settings", "settings extension"),
    ("D", r"^System/Library/PreferencePanes/[^/]+\.prefPane$", "settings", "settings pane"),
    ("D", r"^System/Library/Frameworks/[^/]+\.framework$", "frameworks", "public framework"),
    ("D", r"^System/Library/PrivateFrameworks/[^/]+\.framework$", "frameworks", "private framework"),
    ("D", r"^System/(KernelKit/)?System/Library/Extensions/[^/]+\.kext$", "drivers", "kext"),
    ("D", r"^System/Library/DriverExtensions/[^/]+\.dext$", "drivers", "dext"),
    ("F", r"^System/Library/(LaunchDaemons|LaunchAgents)/[^/]+\.plist$", "daemons", "launchd plist"),
    ("F", r"^(usr/bin|usr/sbin|usr/libexec|bin|sbin)/[^/]+$", "cli", "tool"),
)
INDEX_ASSET_DIRS = ("/Sounds/", "/Desktop Pictures/", "/Wallpaper", "/Cursors/", "/Fonts/")


def parse_index_line(line):
    """One reference index line: type, size, path. Returns None for headers, blanks and malformed lines."""
    parts = line.rstrip("\r\n").split("\t")
    if len(parts) < 3 or parts[0] not in ("F", "D", "L"):
        return None
    path = parts[2].strip("/")
    if not path:
        return None
    try:
        size = int(parts[1])
    except ValueError:
        size = 0
    return {"type": parts[0], "size": size, "path": path}


def classify_index_record(rec):
    """Category, kind and name for one index record, or None when it is not an OS component this layer records."""
    t, p = rec["type"], rec["path"]
    base = p.rsplit("/", 1)[-1]
    bundle = base.rsplit(".", 1)[0] if "." in base else base
    for rt, rx, cat, kind in INDEX_RULES:
        if rt != t or not re.search(rx, p):
            continue
        if cat == "daemons":
            return cat, kind, base[:-len(".plist")], ""
        if cat == "cli":
            return cat, kind, base, ""
        if cat == "settings" and kind == "settings extension" and "/PlugIns/" not in p and not SETTINGS_RE.search(base):
            cat, kind = "system-ui", "system extension"
        if cat == "system-ui" and bundle.endswith("Agent"):
            kind = "core service agent"
        if cat == "apps" and bundle in SYSTEM_UI:
            cat = "system-ui"
        owner = ""
        if "/Contents/PlugIns/" in p:
            owner = p.split("/Contents/PlugIns/")[0].rsplit("/", 1)[-1][:-len(".app")]
        return cat, kind, bundle, owner
    if t == "F" and base.endswith(".metallib"):
        m = re.search(r"/([^/]+)\.(framework|app|bundle|dylib)/", "/" + p)
        return "shaders", "metal library", base, m.group(1) if m else ""
    ext = os.path.splitext(base)[1].lower()
    if t == "F" and ext in (".ttf", ".otf", ".ttc"):
        return "assets", "font", base, ""
    if t == "F" and ext in ASSET_EXT and any(d in "/" + p for d in INDEX_ASSET_DIRS):
        kind = "wallpaper" if re.search(r"Wallpaper|Desktop Pictures", p) else ASSET_EXT[ext]
        return "assets", kind, base, ""
    return None


def walk_index(lines):
    """Entries for every recorded OS component in a reference index (lines of the tab-separated file)."""
    out = []
    for line in lines:
        rec = parse_index_line(line)
        if not rec:
            continue
        hit = classify_index_record(rec)
        if not hit:
            continue
        cat, kind, name, owner = hit
        path = "/" + rec["path"]
        out.append(entry(cat, f"path:{path}", path, kind, name, owner=owner, facts={"size": rec["size"]}))
    return out


class Assessor:
    """Heuristic BifrOSt counterpart and status, from names that exist in the repository (cached file reads)."""

    def __init__(self, repo):
        self.repo = repo
        self.cache = {}

    def exists(self, rel):
        return os.path.exists(os.path.join(self.repo, rel))

    def text(self, rel):
        if rel not in self.cache:
            try:
                with open(os.path.join(self.repo, rel), encoding="utf-8", errors="replace") as f:
                    self.cache[rel] = re.sub(r"[^a-z0-9]", "", f.read().lower())
            except OSError:
                self.cache[rel] = ""
        return self.cache[rel]

    def app(self, name):
        slug = re.sub(r"[^a-z0-9]", "", name.lower())
        if APP_ALIASES.get(slug) and self.exists(APP_ALIASES[slug]):
            return APP_ALIASES[slug]
        apps = os.path.join(self.repo, "springboard", "apps")
        if os.path.isdir(apps):
            for d in sorted(os.listdir(apps)):
                key = re.sub(r"[^a-z0-9]", "", d.lower())
                if key.endswith("desktop"):
                    key = key[:-len("desktop")]
                if key == slug:
                    return f"springboard/apps/{d}"
        return ""

    def guess(self, e):
        cat, kind, name = e["category"], e["kind"], e["name"]
        slug = re.sub(r"[^a-z0-9]", "", name.lower())
        if cat in ("apps", "system-ui") and kind not in ("core service agent", "system extension", "core service"):
            c = self.app(name)
            return (c, "partial", "heuristic: app with a matching name") if c else ("", "gap", "no counterpart yet")
        if cat == "settings":
            if self.text("springboard/src/shell/pages.rs") and slug and slug in self.text("springboard/src/shell/pages.rs"):
                return "springboard/src/shell/pages.rs", "partial", "heuristic: pane name appears in the settings specs"
            home = "springboard/src/shell/pages.rs" if self.exists("springboard/src/shell/pages.rs") else ""
            return home, "gap", "heuristic: candidate home is the settings specs"
        if cat in ("system-ui", "daemons"):
            if cat == "system-ui" and slug in APP_ALIASES and self.exists(APP_ALIASES[slug]):
                return APP_ALIASES[slug], "partial", "heuristic: shell counterpart by name"
            if cat == "daemons" and not UI_DAEMON_RE.search(name):
                return "", "os", "heuristic: system service, provided by the Linux base or a BifrOSt unit"
            return "", "gap", "heuristic: user-facing service or shell component"
        if cat == "frameworks":
            if kind == "private framework":
                return "", "os", "heuristic: private framework, behaviour only, no binary"
            data = "docs/parity-matrix/data/" + re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-") + ".json"
            if self.exists(data):
                return data, "gap", "heuristic: no status evidence; rows in the framework data file decide"
            return "", "gap", "heuristic: public framework without a data file"
        if cat == "drivers":
            return ("", "n/a", "heuristic: Apple hardware, not on PC") if name.startswith("Apple") else ("", "os", "heuristic: hardware driver, kernel side")
        if cat == "cli":
            return ("", "gap", "heuristic: Apple-specific tool") if APPLE_CLI_RE.match(name) else ("", "os", "heuristic: POSIX tool, Linux base")
        if cat == "input-methods":
            if kind == "keyboard layout" and self.exists("springboard/src/shell/kb_layouts.rs"):
                return "springboard/src/shell/kb_layouts.rs", "partial", "heuristic: keyboard layouts module"
            return "", "gap", "heuristic: no counterpart yet"
        if cat == "shaders":
            return "", "gap", "heuristic: BifrOSt writes its own shaders"
        return "", "gap", "heuristic: asset, inventory only, never copied"


def assess(entries, repo=REPO):
    """Set the heuristic counterpart and status on every entry that has no person-assessed status yet."""
    a = Assessor(repo)
    for e in entries:
        if e.get("status") in (None, "", "unassessed") or e.get("method") == "heuristic":
            bifrost, status, note = a.guess(e)
            e.update(bifrost=bifrost, status=status, note=note, method="heuristic")
    return entries


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("root", nargs="?", help="extracted system volume (ROOT mode)")
    ap.add_argument("--index", help="reference file index TSV (index mode, no extracted tree)")
    ap.add_argument("--dyld-json")
    ap.add_argument("--repo", default=REPO, help="repository the heuristic counterparts are checked against")
    ap.add_argument("--out", default=os.path.join(os.path.dirname(__file__), "..", "..", "docs", "parity-matrix", "os"))
    ap.add_argument("--source", default="macOS 27.0.1 Golden Gate 26A434")
    a = ap.parse_args(argv)
    if bool(a.index) == bool(a.root):
        ap.error("give either --index INDEX.tsv or ROOT")
    if a.index:
        with open(a.index, encoding="utf-8", errors="replace") as f:
            entries = walk_index(f)
    else:
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
        es = assess(merge(old, sorted(es, key=lambda e: e["id"])), a.repo)
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
