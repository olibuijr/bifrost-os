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
JSON file per (category, shard). Each entry gets a BifrOSt counterpart and status (gap, partial, os or n/a). For the apps, settings and system-ui
layers the counterpart comes from the reviewed tables (REVIEWED_APPS, REVIEWED_SETTINGS, REVIEWED_SYSTEM_UI, keyed by the
reference name): those entries are written without "method" and the tables are the record, so a re-run reproduces them.
Other entries get a heuristic counterpart from names that exist in this repository (method "heuristic"); a person
replaces that by setting status and dropping "method", after which re-running keeps the entry's assessed fields
(matched by id). Heuristic entries are re-guessed on every run and keep only their description, role, doc and rea fields.
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
    "appstore": "springboard/src/shell/store/mod.rs", "photos": "springboard/src/shell/photos/mod.rs",
    "terminal": "springboard/src/shell/terminal/mod.rs", "magnifier": "springboard/src/shell/magnifier.rs",
    "missioncontrol": "springboard/src/shell/overview.rs", "audiomidisetup": "springboard/apps/audio-setup-desktop",
    "keychainaccess": "springboard/src/keychain.rs", "screensharing": "springboard/apps/remote",
    "screenshot": "springboard/src/shell/screenshot.rs", "photobooth": "springboard/src/shell/camera/mod.rs",
    "imagecapture": "springboard/src/shell/camera/import.rs", "quicktimeplayer": "springboard/src/shell/video.rs",
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


# ---- reviewed counterparts: the person-assessed apps, settings and system-ui layer ----
# Each entry is (BifrOSt path, status, note). Path "" means no counterpart. A reviewed entry is assessed: it is written
# without "method", so re-runs keep it. Names are the reference names of the index (a settings extension and its pane share
# one mapping when they are the same feature). A status of "partial" needs a real BifrOSt file or page behind it.
SHELL_PAGES = "springboard/src/shell/pages.rs"


def _p(path, note):
    return (path, "partial", note)


def _g(note):
    return ("", "gap", note)


def _na(note):
    return ("", "n/a", note)


def _os(note):
    return ("", "os", note)


REVIEWED_APPS = {
    "Dictionary": _g("the Dictionary settings page exists, no dictionary app"),
    "Keychain Access": _p("springboard/src/keychain.rs", "keychain store, no keychain viewer app"),
    "Mail": _g("no mail app; defaultapps.rs names a mail handler only"),
    "Maps": _g("no maps app"),
    "Messages": _g("no messages app; catalog id bifrost.messages only"),
    "Photos": _p("springboard/src/shell/photos/mod.rs", "Photos (bifrost.photos) in the shell"),
    "Tips": _g("no tips app"),
    "Desk View": _na("Apple hardware camera view"),
    "Expansion Slot Utility": _na("Apple hardware slot diagnostics"),
    "App Store": _p("springboard/src/shell/store/mod.rs", "App Store (bifrost.appstore) in the shell"),
    "Apps": _p("springboard/src/shell/appgrid.rs", "app grid, the launch surface for installed apps"),
    "Automator": _g("no workflow builder"),
    "Books": _p("springboard/apps/books-desktop", "Books app"),
    "Calculator": _p("springboard/apps/calculator-desktop", "Calculator app"),
    "Calendar": _p("springboard/apps/calendar-desktop", "Calendar app"),
    "Chess": _g("no chess app; games-desktop has other games"),
    "Clock": _p("springboard/apps/clock-desktop", "Clock app, shell/clock holds alarms and timers"),
    "Contacts": _p("springboard/apps/contacts-desktop", "Contacts app"),
    "Activity Monitor": _g("no process monitor app yet"),
    "AirPort Utility": _na("Apple AirPort router administration"),
    "Audio MIDI Setup": _p("springboard/apps/audio-setup-desktop", "audio device setup app"),
    "Bluetooth File Exchange": _g("no Bluetooth file transfer app"),
    "ColorSync Utility": _g("no colour profile utility"),
    "Console": _p("springboard/apps/console-desktop", "Console app, unified log viewer"),
    "About This Mac": _p(SHELL_PAGES, "About page in the settings specs"),
    "Archive Utility": _g("archives are handled by the files app only"),
    "FaceTime": _na("Apple calling service"),
    "FindMy": _na("Apple device-location network"),
    "Font Book": _p(SHELL_PAGES, "Fonts settings page, no font manager app"),
    "Freeform": _p("springboard/apps/freeform-desktop", "Freeform app"),
    "Digital Color Meter": _g("no colour picker utility"),
    "Disk Utility": _g("no disk utility app"),
    "DVD Player": _g("no optical media player"),
    "Directory Utility": _na("Apple directory service administration"),
    "Feedback Assistant": _na("Apple feedback service"),
    "Folder Actions Setup": _g("no folder action hooks"),
    "Games": _p("springboard/apps/games-desktop", "Games app"),
    "Home": _na("Apple HomeKit home app"),
    "Image Capture": _p("springboard/src/shell/camera/import.rs", "camera import, device image transfer"),
    "Image Playground": _na("Apple generative image service"),
    "Grapher": _g("no graphing app"),
    "iPhone Mirroring": _na("iPhone device mirroring"),
    "iOS App Installer": _na("installs iOS apps on Apple silicon"),
    "Journal": _g("no journal app"),
    "Mission Control": _p("springboard/src/shell/overview.rs", "Overview, the shell's window overview"),
    "Music": _p("springboard/apps/music-desktop", "Music app"),
    "News": _p("springboard/apps/news-desktop", "News app"),
    "Notes": _p("springboard/apps/notes-desktop", "Notes app"),
    "Magnifier": _p("springboard/src/shell/magnifier.rs", "Magnifier (bifrost.magnifier) in the shell"),
    "Migration Assistant": _g("no migration assistant"),
    "Passwords": _p("springboard/src/shell/passwords_page.rs", "passwords page, no separate passwords app"),
    "Phone": _na("Apple telephony"),
    "Photo Booth": _p("springboard/src/shell/camera/mod.rs", "Camera (bifrost.camera), capture app"),
    "Podcasts": _p("springboard/apps/podcasts-desktop", "Podcasts app"),
    "Preview": _p("springboard/apps/pdfcraft-desktop", "PDF viewing and editing; no image preview"),
    "QuickTime Player": _p("springboard/src/shell/video.rs", "video playback in the shell"),
    "Reminders": _p("springboard/apps/reminders-desktop", "Reminders app"),
    "Print Center": _g("no print queue app; print_sheet.rs only"),
    "Shortcuts": _p("springboard/apps/shortcuts-desktop", "Shortcuts app"),
    "Siri AI": _p("springboard/apps/assistant-desktop", "assistant app"),
    "Stickies": _g("no sticky notes app yet"),
    "Stocks": _g("no stocks app"),
    "System Settings": _p("springboard/src/shell/settings.rs", "Settings app"),
    "Screen Sharing": _p("springboard/apps/remote", "remote desktop app"),
    "Screenshot": _p("springboard/src/shell/screenshot.rs", "screenshot tool in the shell"),
    "Script Editor": _g("no script editor"),
    "System Information": _g("no system information app yet"),
    "TV": _g("no TV app"),
    "TextEdit": _g("no plain text editor app"),
    "Terminal": _p("springboard/src/shell/terminal/mod.rs", "Term (is.olibuijr.term) terminal"),
    "VoiceOver Utility": _p("springboard/src/shell/accessibility/mod.rs", "accessibility tree and reader; no VoiceOver utility"),
    "VoiceMemos": _p("springboard/apps/voicememos-desktop", "Voice Memos app"),
    "Weather": _p("springboard/apps/weather-desktop", "Weather app"),
    "Wireless Diagnostics": _g("no wireless diagnostics app"),
}

_SET_PAGE = "Settings page in the specs"
REVIEWED_SETTINGS = {
    "AboutExtension": _p(SHELL_PAGES, "About page"),
    "AccessibilitySettingsExtension": _p(SHELL_PAGES, "Accessibility page"),
    "AccessibilitySettingsSearchExtension": _p("springboard/src/shell/settings_search.rs", "settings search index"),
    "Accounts": _p("springboard/src/shell/account.rs", "BifrOSt Account sign-in and profile"),
    "AppleIDSettings": _p("springboard/src/shell/account.rs", "BifrOSt Account page"),
    "AppleIDPrefPane": _p("springboard/src/shell/account.rs", "BifrOSt Account page"),
    "Appearance": _p(SHELL_PAGES, "Appearance page (Light, Dark, Auto)"),
    "Battery": _p(SHELL_PAGES, "Battery page"),
    "CDs & DVDs Settings Extension": _g("no optical disc settings"),
    "CDsAndDvDsSettingsIntents": _g("no optical disc settings"),
    "ClassKitSettings": _na("Apple classroom service"),
    "ClassKitPreferencePane": _na("Apple classroom service"),
    "ClassroomSettings": _na("Apple classroom service"),
    "ControlCenterSettings": _p(SHELL_PAGES, "Control Centre page"),
    "CoverageSettings": _na("Apple AppleCare coverage"),
    "DateAndTime": _p(SHELL_PAGES, "Date & Time page (datetime_page.rs)"),
    "DateAndTime Extension": _p(SHELL_PAGES, "Date & Time page (datetime_page.rs)"),
    "DesktopScreenEffectsPref": _p("springboard/src/shell/screensaver.rs", "screen saver and desktop effects"),
    "DesktopSettings": _p(SHELL_PAGES, "Desktop & Dock page"),
    "Dock": _p(SHELL_PAGES, "Desktop & Dock page"),
    "Displays": _p(SHELL_PAGES, "Displays page"),
    "DisplaysExt": _p(SHELL_PAGES, "Displays page"),
    "EnergySaver": _p(SHELL_PAGES, "Energy page"),
    "EnergySaverPref": _p(SHELL_PAGES, "Energy page"),
    "PowerPreferences": _p(SHELL_PAGES, "Energy page"),
    "Expose": _p("springboard/src/shell/overview.rs", "Overview and hot corners"),
    "Extensions": _g("no extension manager page"),
    "FaceTimeNotificationExtension": _na("Apple calling service"),
    "FamilySettings": _na("Apple Family Sharing service"),
    "FamilySharingPrefPane": _na("Apple Family Sharing service"),
    "FocusSettingsExtension": _p("springboard/src/shell/focus.rs", "Focus modes"),
    "FollowUpSettingsExtension": _g("no follow-up reminders settings"),
    "GameCenterMacOSSettingsExtension": _na("Apple Game Center service"),
    "GameCenterSettingsDeviceExpertExtension": _na("Apple Game Center service"),
    "GameCenterSettingsSearchExtension": _na("Apple Game Center service"),
    "GameControllerMacSettings": _p("springboard/src/shell/game_controllers_page.rs", "game controller settings"),
    "GameControllerSettingsIntents": _p("springboard/src/shell/game_controllers_page.rs", "game controller settings"),
    "GeneralSettings": _p(SHELL_PAGES, "General page"),
    "GenerativePartnerSettingsExtension": _na("Apple generative service"),
    "HeadphoneSettingsExtension": _g("no headphone settings page"),
    "HomePrivacySettingsExtension": _na("Apple HomeKit privacy"),
    "InternationalSettingsExtension": _p(SHELL_PAGES, "Language & Region page"),
    "Localization": _p(SHELL_PAGES, "Language & Region page"),
    "InternetAccounts": _g("no internet accounts page"),
    "InternetAccountsSettingsExtension": _g("no internet accounts page"),
    "Keyboard": _p(SHELL_PAGES, "Keyboard page"),
    "KeyboardSettings": _p(SHELL_PAGES, "Keyboard page"),
    "LoginItems": _p("springboard/src/shell/login_items_page.rs", "Login Items page"),
    "Mouse": _g("no pointer settings page; services/cursor_service.rs only"),
    "MouseExtension": _g("no pointer settings page; services/cursor_service.rs only"),
    "Network": _p(SHELL_PAGES, "Network page"),
    "NeptuneOneWallpaper": _g("Apple wallpaper resource, inventory only"),
    "Notifications": _p(SHELL_PAGES, "Notifications page"),
    "NotificationsSettings": _p(SHELL_PAGES, "Notifications page"),
    "Passwords": _p("springboard/src/shell/passwords_page.rs", "passwords page"),
    "PrefPaneSearchIntent": _p("springboard/src/shell/settings_search.rs", "settings search"),
    "PrintAndFax": _g("no printer settings page"),
    "PrintAndScan": _g("no printer settings page"),
    "PrinterScannerSettings": _g("no printer settings page"),
    "Profiles": _g("no configuration profiles page"),
    "ProfilesSettingsExt": _g("no configuration profiles page"),
    "ScreenTime": _p("springboard/src/shell/screentime/page.rs", "Screen Time page"),
    "ScreenTimePreferencesExtension": _p("springboard/src/shell/screentime/page.rs", "Screen Time page"),
    "Security": _p(SHELL_PAGES, "Privacy page"),
    "SecurityPrivacyExtension": _p(SHELL_PAGES, "Privacy page"),
    "SettingsImportExtension": _g("no settings import from another system"),
    "Sharing": _g("the share sheet exists (shell/share); no sharing settings page"),
    "SharingPref": _g("the share sheet exists (shell/share); no sharing settings page"),
    "SiriPreferenceExtension": _g("assistant app exists, no Siri settings page"),
    "SoftwareUpdate": _p("springboard/src/shell/update.rs", "Software Update"),
    "SoftwareUpdateSettingsExtension": _p("springboard/src/shell/update.rs", "Software Update"),
    "Sound": _p("springboard/src/shell/sound_page.rs", "Sound page"),
    "Spotlight": _p("springboard/src/shell/quicksearch.rs", "search and index; no Spotlight settings page"),
    "SpotlightPreferenceExtension": _p("springboard/src/shell/quicksearch.rs", "search and index; no Spotlight settings page"),
    "Storage": _p("springboard/src/shell/storage_page.rs", "Storage page"),
    "StartupDisk": _na("Apple boot picker, PC uses the firmware boot menu"),
    "TimeMachine": _p("springboard/src/shell/backup_page.rs", "Backup page"),
    "TimeMachineSettings": _p("springboard/src/shell/backup_page.rs", "Backup page"),
    "Touch ID & Password": _na("Apple Touch ID hardware"),
    "TouchID": _na("Apple Touch ID hardware"),
    "Trackpad": _g("no trackpad settings page"),
    "TrackpadExtension": _g("no trackpad settings page"),
    "UniversalAccessPref": _p(SHELL_PAGES, "Accessibility page"),
    "UsersGroups": _g("no users and groups page"),
    "Wallet": _na("Apple Wallet service"),
    "WalletSettingsExtension": _na("Apple Wallet service"),
    "WallpaperImageExtension": _p("springboard/src/shell/wallpaper_page.rs", "Wallpaper page, image wallpapers"),
    "WallpaperGradientExtension": _p("springboard/src/shell/wallpaper_page.rs", "Wallpaper page, gradients"),
    "WallpaperAerialsExtension": _na("Apple streamed aerial video wallpapers"),
    "WallpaperDynamicExtension": _g("no dynamic wallpapers"),
    "WallpaperLegacyExtension": _g("Apple wallpaper resource bundle, inventory only"),
    "WallpaperMacintoshExtension": _g("Apple wallpaper resource bundle, inventory only"),
    "WallpaperMontereyExtension": _g("Apple wallpaper resource bundle, inventory only"),
    "WallpaperSequoiaExtension": _g("Apple wallpaper resource bundle, inventory only"),
    "WallpaperSonomaExtension": _g("Apple wallpaper resource bundle, inventory only"),
    "WallpaperVenturaExtension": _g("Apple wallpaper resource bundle, inventory only"),
    "WiFiMacSettingsIntents": _g("Wi-Fi intents, no counterpart yet"),
    "Speech": _g("no speech settings page"),
}

REVIEWED_SYSTEM_UI = {
    "AirPlayUIAgent": _g("no AirPlay receiver"),
    "AirDropUI": _na("Apple device-to-device sharing protocol"),
    "AccessibilityUIServer": _p("springboard/src/shell/accessibility/mod.rs", "accessibility tree and actions"),
    "Accessibility Reader": _p("springboard/src/shell/accessibility/reader.rs", "reader view"),
    "UniversalAccessControl": _p("springboard/src/shell/accessibility/mod.rs", "accessibility controls"),
    "VoiceOver": _p("springboard/src/shell/atspi/mod.rs", "AT-SPI bridge, screen reader support"),
    "Dwell Control": _g("no dwell control"),
    "ControlCenter": _p("springboard/src/shell/control_center.rs", "Control Centre"),
    "ControlStrip": _na("Touch Bar control strip"),
    "SpacesTouchBarAgent": _na("Touch Bar"),
    "Dock": _p("springboard/src/shell/desktop_dock.rs", "desktop dock"),
    "Finder": _p("springboard/apps/files", "Files app"),
    "WindowManager": _p("springboard/src/shell/desk.rs", "window management on the desktop"),
    "NotificationCenter": _p("springboard/src/shell/notify/center.rs", "notification centre"),
    "UserNotificationCenter": _p("springboard/src/shell/usernotif.rs", "user notifications"),
    "Spotlight": _p("springboard/src/shell/quicksearch.rs", "quick search"),
    "Siri": _p("springboard/apps/assistant-desktop", "assistant app"),
    "loginwindow": _p("springboard/src/shell/login.rs", "login and lock"),
    "LockScreen": _p("springboard/src/shell/login.rs", "lock screen"),
    "ScreenSaverEngine": _p("springboard/src/shell/screensaver.rs", "screen saver"),
    "screencaptureui": _p("springboard/src/shell/screenshot.rs", "screenshot UI"),
    "SystemUIServer": _p("springboard/src/shell/desktop_bar.rs", "menu bar"),
    "MenuBarAgent": _p("springboard/src/shell/desktop_bar.rs", "menu bar and menu extras"),
    "WallpaperAgent": _p("springboard/src/shell/wallpaper_page.rs", "wallpaper"),
    "TextInputMenuAgent": _p("springboard/src/shell/kb_layouts.rs", "keyboard layouts and input menu"),
    "TextInputSwitcher": _p("springboard/src/shell/kb_layouts.rs", "input source switching"),
    "KeyboardAccessAgent": _p("springboard/src/shell/keyboard.rs", "keyboard access"),
    "MediaRemoteUI": _p("springboard/src/shell/music/nowplaying.rs", "now playing"),
    "MediaRemoteUIService": _p("springboard/src/shell/services/now_playing.rs", "now playing service"),
    "NowPlayingTouchUI": _na("touch-only now playing UI"),
    "OSDUIHelper": _p("springboard/src/shell/volume_hud.rs", "volume and brightness HUD"),
    "PreviewShell": _p("springboard/src/shell/services/quick_look.rs", "Quick Look"),
    "PreviewShellMac": _p("springboard/src/shell/services/quick_look.rs", "Quick Look"),
    "Software Update": _p("springboard/src/shell/update.rs", "Software Update"),
    "Batteries": _p("springboard/src/shell/battery.rs", "battery status"),
    "BluetoothUIServer": _p("springboard/src/shell/services/bluetooth_service.rs", "Bluetooth service"),
    "BluetoothUIService": _p("springboard/src/shell/services/bluetooth_service.rs", "Bluetooth service"),
    "BluetoothSetupAssistant": _g("no Bluetooth setup assistant"),
    "Wi-Fi": _p("springboard/src/shell/wifi_page.rs", "Wi-Fi page"),
    "WiFiAgent": _p("springboard/src/shell/desktop_network.rs", "network status"),
    "VPN": _p(SHELL_PAGES, "VPN page"),
    "Screen Time": _p("springboard/src/shell/screentime/mod.rs", "Screen Time"),
    "Diagnostics Reporter": _p("springboard/src/shell/diagnostics_page.rs", "diagnostics page"),
    "Problem Reporter": _p("springboard/src/shell/diag.rs", "diagnostics"),
    "Enhanced Logging": _p("springboard/apps/console-desktop", "Console app, unified logging"),
    "Language Chooser": _g("installer language step only"),
    "Setup Assistant": _g("no first-run setup assistant yet"),
    "Shortcuts Events": _p("springboard/apps/shortcuts-desktop", "Shortcuts app"),
    "ShortcutsActions": _p("springboard/apps/shortcuts-desktop", "Shortcuts app"),
    "Keychain Circle Notification": _na("Apple iCloud Keychain service"),
    "PhotosPicker": _p("springboard/src/shell/photos/mod.rs", "photo picker"),
    "PhotoPicker": _p("springboard/src/shell/photos/mod.rs", "photo picker"),
    "AppleScript Utility": _na("AppleScript runtime"),
    "Script Menu": _g("no script menu"),
    "TipsSpotlightHandler": _g("no tips app"),
    "Ticket Viewer": _na("Apple Wallet ticket viewer"),
    "Pass Viewer": _na("Apple Wallet pass viewer"),
    "PassViewer": _na("Apple Wallet pass viewer"),
    "Game Center": _na("Apple Game Center service"),
    "GameOverlayUI": _g("no game overlay"),
    "Install in Progress": _g("installer progress UI in the installer, not in the desktop"),
    "Installer Progress": _g("installer progress UI in the installer, not in the desktop"),
    "Erase Assistant": _na("Apple hardware erase assistant"),
    "Apple Diagnostics": _na("Apple hardware diagnostics"),
    "Rosetta 2 Updater": _na("Apple silicon translation layer"),
    "Medical Imaging Calibrator": _na("Apple hardware display calibration"),
    "Memory Slot Utility": _na("Apple hardware memory diagnostics"),
    "Paired Devices": _na("Apple device pairing service"),
    "Family": _na("Apple Family Sharing service"),
    "iCloud": _na("Apple iCloud service"),
    "iCloud+": _na("Apple iCloud service"),
    "WatchFaceAlert": _na("Apple Watch service"),
    "WorkoutAlert-Mac": _na("Apple Watch service"),
    "StageManagerOnboarding": _g("no stage manager"),
    "Shell": _g("internal shell extension"),
    "StartupDisk": _na("Apple boot picker, PC uses the firmware boot menu"),
    "Computer Name": _g("no computer name page; About page shows the name"),
    "Localization": _p(SHELL_PAGES, "Language & Region page"),
    "LegacyPluginEnablement": _g("no legacy plugin support"),
    "Hello": _na("Apple onboarding"),
    "Drift": _na("Apple internal service"),
}

# Second pass: the entries that have a PC feature behind them, or that the Linux base provides, or that are Apple-only.
REVIEWED_APPS.update({
    "Time Machine": _p("springboard/src/shell/backup_page.rs", "Backup page, the backup settings"),
})
REVIEWED_SETTINGS.update({
    "Bluetooth": _p(SHELL_PAGES, "Bluetooth page"),
    "Wallpaper": _p("springboard/src/shell/wallpaper_page.rs", "Wallpaper page"),
    "ScreenTimeSettingsResponseExtension": _p("springboard/src/shell/screentime/page.rs", "Screen Time page"),
    "AccessibilitySettingsWidgetExtension": _g("accessibility widget, no counterpart yet"),
    "AppleAccountIntents_macOS": _g("Apple account intents, no counterpart yet"),
    "DigiHubDiscs": _g("optical disc media settings, no counterpart yet"),
    "MediaExtensionsSettingsController": _g("media extensions, no counterpart yet"),
    "SecurityImprovementsExtension": _g("security improvements, no counterpart yet"),
    "SettingsSystemExtensionController": _g("settings controller, no counterpart yet"),
    "SiriAttentionAndInvocationExtension": _g("Siri invocation settings, no Siri settings page"),
    "SiriTurnRestatementExtension": _g("Siri settings, no Siri settings page"),
    "SiriAutoEvalPlugin": _na("Apple Siri evaluation service"),
    "SiriSuggestionsLightHousePlugin": _na("Apple Siri suggestions service"),
    "SiriUserSegmentation": _na("Apple Siri user segmentation service"),
    "StorageDESDXC": _g("encrypted disk settings, no counterpart yet"),
    "StorageDESSD": _g("encrypted disk settings, no counterpart yet"),
})
REVIEWED_SYSTEM_UI.update({
    "AOSUIPrefPaneLauncher": _na("Apple pane launcher, the Settings app covers it"),
    "AVB Configuration": _na("Apple audio video bridging hardware configuration"),
    "AddressBookUrlForwarder": _na("Apple Contacts URL handler"),
    "AuthorizationPromptService": _p("springboard/src/shell/permission_sheet.rs", "permission sheet in the shell"),
    "AppExtensionManagement": _g("no extension manager"),
    "AskPermissionAskToResponseExtension": _na("Apple Family Sharing approvals"),
    "AskToExtension": _na("Apple Family Sharing approvals"),
    "AskToTestResponseUIExtension": _na("Apple Family Sharing approvals"),
    "AskToViewExtension": _na("Apple Family Sharing approvals"),
    "ChallengesMessageExtension": _na("Apple Messages challenges"),
    "ComposeReviewExtension": _na("Apple Mail compose review"),
    "ConfigurationProfilesFollowUpExtension": _g("no configuration profiles"),
    "CoreMotionFoundationModelExtension": _na("Apple machine learning model service"),
    "com.apple.fskit.exfat": _os("filesystem extension, the Linux kernel provides the filesystem"),
    "com.apple.fskit.ftp": _os("filesystem extension, no FSKit on PC"),
    "com.apple.fskit.msdos": _os("filesystem extension, the Linux kernel provides the filesystem"),
    "FSKitModuleManagement": _os("filesystem module manager, the Linux kernel provides filesystems"),
    "Database Events": _na("AppleScript database events"),
    "DiskImageMounter": _os("disk image mounting, the Linux base mounts images"),
    "EscrowSecurityAlert": _na("Apple iCloud Keychain escrow"),
    "EventKitUIRemoteUIExtension": _p("springboard/apps/calendar-desktop", "calendar event UI"),
    "FileProvider-Feedback": _na("Apple file provider feedback"),
    "IOUIAgent": _os("IO device UI agent, the Linux base handles device prompts"),
    "Install Command Line Developer Tools": _na("Apple developer tools installer"),
    "Installer": _g("the native installer is not a desktop component"),
    "iLifeSlideshows": _na("Apple iLife slideshow"),
    "JavaLauncher": _na("Java runtime launcher, not shipped"),
    "KeyboardSetupAssistant": _p("springboard/src/shell/kb_layouts.rs", "keyboard layouts"),
    "liquiddetectiond": _na("Apple liquid detection hardware service"),
    "MADViewServiceExtension": _na("Apple Maps view service"),
    "MKRemoteUI": _na("Apple Maps remote UI"),
    "MDMMigrationTrampoline": _na("device management migration"),
    "MacMailSearchLogProcessor": _na("Apple Mail search log"),
    "MailDiagnosticExtension": _na("Apple Mail diagnostics"),
    "MusicRecognitionMac": _g("song recognition, no counterpart yet"),
    "NetAuthAgent": _os("network authentication agent, the Linux base supplies 802.1X"),
    "NewDeviceOutreachIntents_macOS": _na("Apple new device outreach"),
    "ODDFeatureDigestsExtension": _na("Apple diagnostics and telemetry"),
    "OBCEngagePlugin": _na("Apple diagnostics and telemetry"),
    "PCCAgentClientExtension": _na("Apple private cloud compute client"),
    "PRIMLCKPreemptivePing": _na("Apple private machine learning service"),
    "PaperKitExtension": _p("springboard/src/shell/markup.rs", "markup tools in the shell"),
    "PlatformSSOToken": _na("Apple platform single sign-on"),
    "PowerChime": _g("power connect sound, no counterpart yet"),
    "RapportUIAgent": _na("Apple device handoff (Rapport)"),
    "SAMaintenance": _na("Apple Safari maintenance"),
    "SafariUsageRetentionExtension": _na("Apple Safari telemetry"),
    "SearchToolDiagnosticExtension": _na("Apple search diagnostics"),
    "SpeakerIdSamplingExtension": _na("Apple speaker identification sampling"),
    "StartupDiskWidgetExtension": _na("Apple boot picker, PC uses the firmware boot menu"),
    "SuggestedActionsRemoteUI": _g("suggested actions UI, no counterpart yet"),
    "TestFileGeneratorDE": _na("Apple internal test tool"),
    "ThermalTrap": _os("thermal event trap, the Linux kernel manages thermal control"),
    "UIKitSystem": _na("iOS UI kit system service"),
    "UniversalControl": _g("cross-device control, no counterpart yet"),
    "UnmountAssistantAgent": _os("unmount assistant, the Linux base unmounts volumes"),
    "WidgetRenderer_Activities": _na("Apple Activities widget"),
    "WidgetKit Simulator": _na("Apple developer simulator"),
    "Word of the Day": _na("Apple Dictionary feature"),
    "TransferResetExtension": _na("Apple device transfer"),
    "CoreLocationAgent": _p("springboard/ui/src/location_auth.rs", "location authorisation"),
    "ClockAngel": _g("clock screen saver, no counterpart yet"),
    "Automator Application Stub": _na("Automator support, no Automator on PC"),
    "Automator Installer": _na("Automator support, no Automator on PC"),
    "rcd": _na("Apple accessory control daemon"),
    "PeopleMessageService": _g("people and messaging service, no counterpart yet"),
    "PeopleViewService": _g("people view service, no counterpart yet"),
})
REVIEWED_TABLES = {"apps": REVIEWED_APPS, "settings": REVIEWED_SETTINGS, "system-ui": REVIEWED_SYSTEM_UI}
# Apple App Intents plumbing that belongs to a feature, never a counterpart of its own.
INTENT_RE = re.compile(r"Intents?$|IntentsExtension|AppIntent|Intents(Extension)?$")
# Apple hardware, cloud, telemetry, machine-learning and account services that a PC does not have.
NOT_ON_PC_RE = re.compile(r"SELFIngestor|Inference|Telemetry|Unilog|Biome|ODDI|^Fed|Flurry|PrivateML|PrivateEvolution|"
                          r"MLHost|MediaML|Alchemist|Generative|ZeoliteEval|Bitacora|AssetMetrics|Monterey|Ventura|Sonoma|"
                          r"Sequoia|Vermillion|Arabesque|NullPoster|Worker$|Analytics|GameCenter|Game Center|Classroom|"
                          r"ClassKit|Family|FaceTime|AirDrop|Wallet|FindMy|Touch ?ID|iCloud|AppleID|Apple ID|Intelligence|"
                          r"Rosetta|Hello|Drift|Coverage|Calibrator|Mirror|iOS|iPhone|Watch|Pass(Viewer)?$|Ticket|Paired|Erase")


def reviewed_for(e):
    """The reviewed (status, counterpart, note) for an apps, settings or system-ui entry, or None when not reviewed."""
    table = REVIEWED_TABLES.get(e.get("category"))
    if table is None:
        return None
    name = e.get("name", "")
    if name in table:
        return table[name]
    if INTENT_RE.search(name):
        return _g("App Intents plumbing for this feature, no counterpart yet")
    if NOT_ON_PC_RE.search(name):
        return _na("Apple service or hardware with no PC counterpart")
    return _g("reviewed: no BifrOSt counterpart yet")


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
        reviewed = reviewed_for(e)
        if reviewed is not None:  # the reviewed table is the record: it is written without "method", so it is kept
            bifrost, status, note = reviewed
            e.update(bifrost=bifrost, status=status, note=note)
            e.pop("method", None)
        elif e.get("status") in (None, "", "unassessed") or e.get("method") == "heuristic":
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
