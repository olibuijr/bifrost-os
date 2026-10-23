#!/usr/bin/env python3
"""App compliance gate: every app under springboard/apps against our SDKs, UI kit and design rules.

Static checks on source and manifests (no build). Each finding is file:line + rule ID + fix hint; rule IDs map to
the bifrost-hig checklist (~/.claude/skills/bifrost-hig/SKILL.md). Runtime checks live in each app's
tests/compliance.rs (generated from tools/templates/app-compliance.rs). Docs: docs/app-compliance.md.

  python3 tools/check-app-compliance.py                 # ratchet: fail only when an app is WORSE than the baseline
  python3 tools/check-app-compliance.py --strict        # fail on any finding
  python3 tools/check-app-compliance.py --app DIR       # one app (dir name), findings listed
  python3 tools/check-app-compliance.py --json          # machine output (findings, counts, scores)
  python3 tools/check-app-compliance.py --update-baseline   # write docs/app-compliance-baseline.json (after fixing apps)
  python3 tools/check-app-compliance.py --write-doc     # regenerate the per-app table in docs/app-compliance.md
  python3 tools/check-app-compliance.py --gen-tests [--write]   # (check or write) tests/compliance.rs in every app crate
  python3 tools/check-app-compliance.py --changed-sdk [--base REF]   # apps using SDK/render/ui/proto items changed since REF

A finding can be accepted on its line (or the line above) with `// compliance-allow: RULE reason`.
"""
import argparse
import json
import re
import struct
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SB = ROOT / 'springboard'
APPS = SB / 'apps'
BASELINE = ROOT / 'docs/app-compliance-baseline.json'
DOC = ROOT / 'docs/app-compliance.md'
TEMPLATE = ROOT / 'tools/templates/app-compliance.rs'
SDK_DIRS = ['springboard/sdk', 'springboard/render', 'springboard/ui', 'springboard/proto']
DOC_BEGIN, DOC_END = '<!-- app-compliance-table:begin -->', '<!-- app-compliance-table:end -->'

# rule id -> (level, bifrost-hig rule, short title, fix hint)
RULES = {
    'GG-01': ('error', 'GG-01', 'sidebar/kit app is not a full-size window', 'set window = "full-size" in manifest.toml (and min_shell = "0.4.84"); keep the leading 87 pt of the top band free for the traffic lights'),
    'GG-02': ('warning', 'GG-02', 'hand-rolled toolbar', 'draw it with widgets::toolbar_band + widgets::toolbar (ToolbarItem::icon, in_group) and publish each button as a control'),
    'GG-03': ('warning', 'GG-03', 'hand-rolled search field', 'use widgets::search_field + controls::search::{layout, hits_clear}'),
    'GG-04': ('warning', 'LAY-04, GG-04', 'hand-rolled sidebar', 'use widgets::source_list with SidebarItem::row(..).with_symbol(..) and SourceList::hit'),
    'GG-05': ('warning', 'GG-02, LAY-04', 'hand-rolled path bar', 'use widgets::path_bar + controls::pathbar::{layout, hit}'),
    'COL-01': ('warning', 'COL-01', 'literal colour', 'use a named constant from render/src/ui.rs (add one if needed); never pack()/rgba()/0xRRGGBBAA in app code'),
    'COL-02': ('warning', 'GG-05, COL-05', 'fixed dark token (does not follow Light/Dark)', 'use the Pair token (LABEL, LABEL_2, CARD_BG, DETAIL_BG, SIDEBAR_BG, SEPARATOR_C, FIELD_BG, ...) with .now()'),
    'COL-03': ('warning', 'COL-05', 'hard-coded white/black background', 'fill with a scheme Pair (DETAIL_BG, CARD_BG, FIELD_BG ...); WHITE is right only for text on the accent'),
    'TYP-01': ('warning', 'TYP-01', 'literal font size', 'use a MAC_*_PT token from render/src/ui.rs'),
    'TYP-02': ('warning', 'TYP-01', 'literal size in fit()', 'measure with the same MAC_*_PT token the text is drawn with'),
    'TYP-03': ('warning', 'TYP-04', 'mobile text-size constant on the PC', 'use the MAC_*_PT constant (Golden Gate HIG table)'),
    'LAY-01': ('warning', 'LAY-04, COL-01', 'literal corner radius', 'use prefs_layout m::* (window 17, card 13, plate 9, tile 5) or a named token; capsules pass a huge radius'),
    'LAY-02': ('warning', 'LAY-04', 'literal radius constant', 'point the constant at prefs_layout m::* instead of a number'),
    'LAY-03': ('warning', 'LAY-03', 'assumes a fixed resolution', 'derive layout from the window size (ctx.cfg, Layout) and ui.pt(); no screen-size literals'),
    'LAY-04': ('warning', 'LAY-03', 'large literal ui.pt() position/size', 'derive the position from the window size or use a layout token (SIDEBAR_W, UNIFIED_BAR_PT, ...)'),
    'LAY-05': ('warning', 'LAY-03', 'literal spacing in ui.pt()', 'use the SPACE_* token'),
    'LAY-06': ('warning', 'LAY-01', 'legacy status-bar inset', 'take the top inset from bifrost_app::layout::Layout'),
    'LAY-07': ('warning', 'TYP-03', 'hand-drawn large title', 'use bifrost_app::widgets::large_title'),
    'APP-01': ('error', 'APP-01', 'manifest incomplete', 'add the missing key to manifest.toml'),
    'APP-02': ('error', 'APP-01', 'architecture is not x86_64-linux', 'set architecture = "x86_64-linux"'),
    'APP-03': ('error', 'APP-01', 'icon missing', 'add the icon file named by the manifest'),
    'APP-04': ('error', 'APP-02', 'CHANGELOG has no entry for the version', 'add a CHANGELOG.md entry for the manifest version'),
    'APP-05': ('error', 'APP-02', 'Cargo.toml version differs from manifest', 'bump manifest.toml and Cargo.toml together'),
    'APP-06': ('warning', 'APP-01', 'permissions not minimal', 'drop a permission the code does not use; an unknown one is rejected at install'),
    'APP-07': ('error', 'APP-01', 'invalid kind or window', 'kind is app|system-app|system|service; window is "full-size", "borderless" or "native" (or absent)'),
    'APP-08': ('warning', 'GG-01', 'full-size window with old min_shell', 'full-size windows need min_shell >= 0.4.84'),
    'APP-10': ('error', 'APP-01', 'icon is not a 180x180 PNG', 'export the manifest icon as a square 180x180 PNG (PNG signature and IHDR size are read; no re-encoding needed)'),
    'APP-11': ('error', 'APP-01', 'manifest declares an app without an executable target', 'add a binary target before publishing this app'),
    'APP-09': ('error', 'APP-01', 'upstream package incomplete', 'upstream.toml needs repo, url, sha256, binary, licenses; manifest needs publisher and license'),
    'RES-01': ('warning', 'CTL-04', 'raw thread::spawn', 'use bifrost_app::dispatch (DispatchQueue::global(Qos).async_, serial queue, DispatchGroup); see docs/bifrost-ui.md "Migrating thread::spawn"'),
    'RES-02': ('warning', 'CTL-04', 'polling next_wake with a worker thread', 'finish work on the main queue (DispatchQueue::main().async_) or wake_fd; return None from next_wake when idle'),
    'RES-03': ('warning', 'CTL-04', 'no ctx.enable_sudden_termination() but no unsaved state', 'call ctx.enable_sudden_termination() once in configure (docs/bifrost-ui.md "Sudden termination")'),
    'SDK-01': ('error', 'CTL-01', 'deprecated SDK API', 'use the replacement named at the item (#[deprecated] note) or in docs/bifrost-ui.md'),
    'CHR-01': ('error', 'CHR-07', 'app menu repeats the shell Edit menu', 'an app menu titled File, View, Window or Help merges into the shell menu of that name (its items lead); the shell Edit already holds the standard items, so route them through the shell instead of a second Edit'),
    'CHR-02': ('warning', 'CHR-02', 'hand-built standard Edit items', 'standard Cut/Copy/Paste/Undo come from the shell Edit menu and ctx commands; do not rebuild them'),
    'TST-01': ('error', 'APP-02', 'tests/compliance.rs missing', 'python3 tools/check-app-compliance.py --gen-tests --write'),
    'TST-02': ('warning', 'APP-02', 'tests/compliance.rs differs from the template', 'python3 tools/check-app-compliance.py --gen-tests --write (keep KNOWN_GAPS)'),
}
ERROR_W, WARN_W, CAP = 4.0, 1.5, 18.0

FIXED = ['SECONDARY', 'FILL', 'FILL_PRESSED', 'FILL_GRAY', 'FILL_TERTIARY', 'SIDEBAR_FILL', 'SIDEBAR_SELECTED',
         'WINDOW_BAR_FOCUSED', 'WINDOW_BAR_BLURRED', 'SEPARATOR', 'CELL', 'TERTIARY', 'HAIRLINE', 'TRACK']
SPACING = {'2': 'SPACE_XXS', '4': 'SPACE_XS', '8': 'SPACE_S', '12': 'SPACE_M', '16': 'SPACE_L',
           '20': 'SPACE_XL', '24': 'SPACE_XXL', '32': 'SPACE_XXXL', '40': 'SPACE_HUGE'}
RESOLUTIONS = '480|600|640|720|768|800|900|960|1000|1024|1080|1200|1280|1366|1440|1600|1920|2560'
KNOWN_PERMS = {'network', 'storage', 'audio', 'camera', 'screen', 'notifications', 'documents', 'keychain', 'remote-desktop'}
PERM_APP = {'camera': 'photobooth-desktop', 'screen': 'colormeter-desktop'}
# permission -> code that shows it is used
PERM_USE = {
    'network': r'\b(http|https|TcpStream|TcpListener|UdpSocket|ureq|fetch|download|open_url|webview|WebProcessPool|net)\b',
    'documents': r'\b(open_document|save_document|commit_document|reopen_document|Document|DocumentEvent)\b',
    'audio': r'\b(audio|Audio|pcm|Pcm|play|sound|Sound|alsa|cpal|speaker|microphone|record)\b',
    'camera': r'\b(v4l2::|Device::open|CameraInfo|camera_path|camera)\b',
    'screen': r'\b(screen_sample|Media::Screen|require_screen_recording)\b',
    'notifications': r'\b(notify|usernotif|set_badge|notification|notification_schedule)\b',
    'keychain': r'\bkeychain\b',
    'storage': r'\b(data_dir|std::fs|fs::|File::|UserDefaults|defaults|store)\b',
}
UNSAVED = re.compile(r'\b(save_document|commit_document|set_document_edited|doc_edited|dirty|unsaved|autosave|modified)\b', re.I)
VOLATILE = {'clock-desktop': 0.15}
SOFT_VOLATILE = {'calendar-desktop', 'reminders-desktop', 'weather-desktop', 'news-desktop', 'podcasts-desktop', 'music-desktop',
                 'voicememos-desktop', 'games-desktop', 'console-desktop', 'daw', 'browser', 'remote', 'assistant-desktop', 'freeform-desktop'}

LITERAL_SIZE = re.compile(r',\s*(\d+(?:\.\d*)?)\s*,\s*Weight::')
LITERAL_FIT = re.compile(r'\bfit\([^;]*?,\s*(\d+(?:\.\d*)?)\s*,')
LITERAL_COLOR = re.compile(r'\b(?:pack|rgba)\s*\(\s*\d')
HEX_COLOR = re.compile(r'\b0x[0-9A-Fa-f]{8}\b')
COLOURISH = re.compile(r'fill|stroke|colou?r|tint|\bbg\b|background|rgba|pack', re.I)
MOBILE_PT = re.compile(r'\b(?!MAC_)[A-Z][A-Z_]*(?:LARGE_TITLE|TITLE_[123]|TITLE|BODY|CAPTION|FOOTNOTE|HEADLINE|CALLOUT|SUBHEAD)_PT\b')
RRECT = re.compile(r'\b(?:fill_rrect|stroke_rrect|rrect|round_rect|fill_round_rect)\s*\(')
RADIUS_CONST = re.compile(r'\bconst\s+(\w*(?:RADIUS|RAD|CORNER)\w*|R)\s*:\s*f32\s*=\s*\d')
PT_LITERAL = re.compile(r'\bui\.pt\(\s*(\d+)(?:\.\d*)?\s*\)')
STATUS_INSET = re.compile(r'\bSTATUS_PT\b')
HAND_TITLE = re.compile(r'\bMAC_LARGE_TITLE_PT\b\s*,\s*Weight::Bold\b')
BG_FILL = re.compile(r'\b(?:fill\w*|clear|set_background|background)\s*\([^;]*\b(WHITE|BLACK)\b')
BG_PACK = re.compile(r'\bpack\(\s*(?:255\s*,\s*255\s*,\s*255|0\s*,\s*0\s*,\s*0)\b')
FIXED_RES = re.compile(r'\b(?:w|h|width|height|screen|size|res)\w*[^;]*\b(' + RESOLUTIONS + r')(?:\.0*)?\b(?![\d.])', re.I)
THREAD_SPAWN = re.compile(r'\bthread::(?:spawn|Builder)\b')
SHELL_MENU_TITLES = 'File|Edit|View|Window|Help'
MENU_DECL = re.compile(r'(?:\btitle:\s*|\(\s*)"(' + SHELL_MENU_TITLES + r')"(?:\.into\(\)|\.to_string\(\)|\.to_owned\(\))?\s*(?:,|\))')
EDIT_MENU_CALL = re.compile(r'\bedit_menu\s*\(\s*\)')
STD_EDIT_ITEM = re.compile(r'(?:AppMenuItem|MenuEntry)::new\([^"]*"(Undo|Redo|Cut|Copy|Paste|Select All)"')
KIT = {
    'sidebar': re.compile(r'\bsource_list\(|\bSourceList\b|\bSidebarItem\b|\bsidebar_list\b'),
    # Do not mistake an app model method such as Action::toolbar() for the shared kit widget.
    'toolbar': re.compile(r'\b(?:widgets::|bifrost_app::widgets::)toolbar(?:_band)?\(|(?<!::)\btoolbar(?:_band)?\(|\bToolbar(?:Item|Layout)?\b'),
    'search': re.compile(r'\bsearch_field\(|\bSearchField\b'),
    'path': re.compile(r'\bpath_bar\(|\bPathBar\b|\bpathbar::'),
}
HAND = {  # hand-rolled markers (first match line is reported)
    'GG-04': re.compile(r'\bfn\s+\w*sidebar\w*|\bSIDEBAR_(?:BG|FILL|SELECTED|W)\b', re.I),
    'GG-02': re.compile(r'\bfn\s+\w*toolbar\w*|\bTOOLBAR_\w+\b', re.I),
    'GG-03': re.compile(r'\bfn\s+\w*search_(?:bar|box|field)\w*|\bfn\s+draw_search\w*', re.I),
    'GG-05': re.compile(r'\bfn\s+\w*(?:breadcrumb|pathbar|path_bar)\w*', re.I),
}
HAND_KIT = {'GG-04': 'sidebar', 'GG-02': 'toolbar', 'GG-03': 'search', 'GG-05': 'path'}


# ---------------------------------------------------------------------------------------------------------------
def strip_tests(text):
    """Source lines with `#[cfg(test)]` items blanked (line numbers are preserved)."""
    lines = text.splitlines()
    out, i = [], 0
    while i < len(lines):
        if '#[cfg(test)]' not in lines[i]:
            out.append(lines[i])
            i += 1
            continue
        depth, opened = 0, False
        while i < len(lines):
            code = lines[i].split('//')[0]
            out.append('')
            depth += code.count('{') - code.count('}')
            opened = opened or '{' in code
            i += 1
            if (opened and depth <= 0) or (not opened and code.rstrip().endswith(';')):
                break
    return out


def rust_function_body(lines, start):
    """Return the brace-delimited body beginning at a Rust function declaration line."""
    body, depth, opened = [], 0, False
    for line in lines[start:]:
        code = line.split('//')[0]
        if not opened:
            if '{' not in code:
                continue
            opened = True
        body.append(code)
        depth += code.count('{') - code.count('}')
        if opened and depth == 0:
            break
    return '\n'.join(body)


def res02_required(next_wake_body, code_all, spawn_sites):
    """Flag worker-fed polling only when next_wake itself schedules a timed wait on pending work."""
    timed_deadline = re.search(r'\b(?:Some|then_some)\s*\(\s*t\s*\+\s*(?:\d+(?:\.\d*)?|\.\d+)', next_wake_body)
    pending_worker = re.search(r'\b(?:self\.)?(?:job|work|search|task|worker|pending|in_flight)\b', next_wake_body)
    worker_channel = bool(spawn_sites or re.search(r'\b(try_recv|mpsc::)', code_all))
    return bool(timed_deadline and pending_worker and worker_channel)


def rust_files(app):
    for p in sorted((app / 'src').rglob('*.rs')) if (app / 'src').is_dir() else []:
        n = p.name
        if n.endswith('tests.rs') or n.startswith('test') or 'tests' in p.relative_to(app).parts:
            continue
        yield p


def split_args(s):
    """Top-level comma split of the text after a call's opening paren; None when the call is not closed on the line."""
    depth, cur, out = 0, '', []
    for ch in s:
        if ch in '([{':
            depth += 1
        elif ch in ')]}':
            if depth == 0:
                out.append(cur.strip())
                return out
            depth -= 1
        if ch == ',' and depth == 0:
            out.append(cur.strip())
            cur = ''
        else:
            cur += ch
    return None


def num(s):
    try:
        return float(s.replace('_', ''))
    except ValueError:
        return None


def allowed(raw, i, rule):
    for j in (i, i - 1):
        if 0 <= j < len(raw):
            m = re.search(r'compliance-allow:\s*([A-Z0-9, -]+)', raw[j])
            if m and (rule in m.group(1) or 'ALL' in m.group(1)):
                return True
    return False


def load_deprecated():
    """name -> hint, from #[deprecated] items in the SDK crates and 'replaced by' notes in docs/bifrost-ui.md."""
    dep = {}
    item = re.compile(r'(?:pub(?:\([a-z]+\))?\s+)?(?:fn|const|static|struct|enum|trait|type)\s+(\w+)')
    for d in ('sdk', 'render', 'ui', 'core', 'proto'):
        for p in (SB / d / 'src').rglob('*.rs'):
            lines = p.read_text(errors='replace').splitlines()
            for i, l in enumerate(lines):
                if '#[deprecated' in l:
                    note = re.search(r'(?:note\s*=|since\s*=)\s*"([^"]*)"', ' '.join(lines[i:i + 3]))
                    for j in range(i, min(i + 6, len(lines))):
                        m = item.search(lines[j])
                        if m and '#[' not in lines[j]:
                            dep[m.group(1)] = note.group(1) if note else 'deprecated'
                            break
    doc = (ROOT / 'docs/bifrost-ui.md').read_text(errors='replace')
    for m in re.finditer(r'`([A-Za-z_][\w:]*)`[^.\n]{0,40}\b(?:is )?(?:replaced by|superseded by|deprecated in favou?r of)\s+`([A-Za-z_][\w:().]*)`', doc):
        dep.setdefault(m.group(1).split('::')[-1], f'replaced by {m.group(2)}')
    for m in re.finditer(r'`([A-Za-z_][\w:().]*)`\s+replaces\s+`([A-Za-z_][\w:]*)`', doc):
        dep.setdefault(m.group(2).split('::')[-1], f'replaced by {m.group(1)}')
    for generic in ('spawn', 'sleep'):  # std calls named in migration prose; RES-01 covers thread::spawn
        dep.pop(generic, None)
    return dep


DEPRECATED = None


def check_manifest(app, native, findings):
    f = lambda rule, where, msg: findings.append((rule, where, msg))
    mp = app / 'manifest.toml'
    if not mp.exists():
        if native and app.name != 'keyboard':  # the keyboard is a system app without a store manifest
            f('APP-01', f'{rel(mp)}:1', 'manifest.toml missing')
        return None
    try:
        m = tomllib.loads(mp.read_text())
    except Exception as e:
        f('APP-01', f'{rel(mp)}:1', f'manifest.toml unreadable: {e}')
        return None
    for key in ('id', 'name', 'version', 'exec', 'icon', 'min_shell', 'summary', 'category', 'architecture'):
        if key not in m:
            f('APP-01', f'{rel(mp)}:1', f'missing {key}')
    if m.get('architecture') != 'x86_64-linux':
        f('APP-02', f'{rel(mp)}:1', f"architecture is {m.get('architecture')!r}")
    if not (app / m.get('icon', 'icon.png')).exists():
        f('APP-03', f'{rel(mp)}:1', f"icon {m.get('icon', 'icon.png')} not found")
    elif (app / m.get('icon', 'icon.png')).is_file():
        size = png_size(app / m.get('icon', 'icon.png'))
        if size is None:
            f('APP-10', f'{rel(mp)}:1', f"icon {m.get('icon', 'icon.png')} is not a PNG")
        elif size != (180, 180):
            f('APP-10', f'{rel(mp)}:1', f"icon {m.get('icon', 'icon.png')} is {size[0]}x{size[1]}, must be 180x180")
    cl = app / 'CHANGELOG.md'
    ver = str(m.get('version'))
    if not cl.exists():
        f('APP-04', f'{rel(cl)}:1', 'CHANGELOG.md missing')
    elif ver not in cl.read_text():
        f('APP-04', f'{rel(cl)}:1', f'no entry for version {ver}')
    cargo = app / 'Cargo.toml'
    if cargo.exists():
        c = tomllib.loads(cargo.read_text())
        if c.get('package', {}).get('version') != m.get('version'):
            f('APP-05', f'{rel(cargo)}:1', f"Cargo.toml version {c.get('package', {}).get('version')} != manifest {ver}")
    if m.get('kind', '') not in ('', 'app', 'system-app', 'system', 'service'):
        f('APP-07', f'{rel(mp)}:1', f"unknown kind {m.get('kind')!r}")
    if m.get('window', '') not in ('', 'full-size', 'borderless', 'native'):
        f('APP-07', f'{rel(mp)}:1', f"unknown window {m.get('window')!r}")
    if m.get('window') == 'full-size':
        have = tuple(int(x) for x in re.findall(r'\d+', str(m.get('min_shell', '0')))[:3])
        if have < (0, 4, 84):
            f('APP-08', f'{rel(mp)}:1', f"min_shell {m.get('min_shell')} < 0.4.84 for a full-size window")
    if (app / 'upstream.toml').exists():
        u = tomllib.loads((app / 'upstream.toml').read_text())
        for k in ('repo', 'url', 'sha256', 'binary', 'licenses'):
            if k not in u:
                f('APP-09', f'{rel(app / "upstream.toml")}:1', f'upstream.toml missing {k}')
        for k in ('publisher', 'license'):
            if k not in m:
                f('APP-09', f'{rel(mp)}:1', f'manifest missing {k}')
    return m


def png_size(path):
    """(width, height) from the IHDR chunk of a PNG (stdlib only); None when the file is not a PNG."""
    try:
        head = Path(path).read_bytes()[:24]
    except OSError:
        return None
    if len(head) < 24 or head[:8] != b'\x89PNG\r\n\x1a\n' or head[12:16] != b'IHDR':
        return None
    return struct.unpack('>II', head[16:24])


def rel(p):
    return str(Path(p).relative_to(ROOT))


def check_app(app):
    """Returns (findings [(rule, where, message)], info)."""
    global DEPRECATED
    if DEPRECATED is None:
        DEPRECATED = load_deprecated()
    native = (app / 'Cargo.toml').exists()
    cargo = tomllib.loads((app / 'Cargo.toml').read_text()) if native else {}
    bins = cargo.get('bin', [])
    executable = (app / 'src/main.rs').is_file()
    if not executable and (app / 'src/bin').is_dir():
        executable = any((app / 'src/bin').rglob('*.rs'))
    if not executable:
        executable = any((app / spec.get('path', f"src/bin/{spec.get('name', '')}.rs")).is_file() for spec in bins)
    # Model-only crates live beside apps in the workspace but have no package manifest or lifecycle to audit.
    if native and not executable and not (app / 'manifest.toml').exists():
        return [], {'native': native, 'manifest': None, 'sidebar': False, 'toolbar': False, 'search': False}
    findings = []
    m = check_manifest(app, native, findings)
    info = {'native': native, 'manifest': m, 'sidebar': False, 'toolbar': False, 'search': False}
    if not native:
        return findings, info
    if m is not None and not executable:
        findings.append(('APP-11', f'{rel(app / "manifest.toml")}:1', 'manifest declares an app without a Cargo executable target'))
    files = {}
    for p in rust_files(app):
        raw = p.read_text(errors='replace').splitlines()
        files[p] = (raw, strip_tests(p.read_text(errors='replace')))
    code_all = '\n'.join(l.split('//')[0] for _, lines in files.values() for l in lines)
    kit = {k: bool(r.search(code_all)) for k, r in KIT.items()}
    info.update({k: kit[k] for k in ('sidebar', 'toolbar', 'search')})

    def add(p, i, rule, msg, raw):
        if not allowed(raw, i, rule):
            findings.append((rule, f'{rel(p)}:{i + 1}', msg))

    # window kind
    if m is not None and m.get('window') != 'full-size' and m.get('kind', '') in ('', 'app'):
        findings.append(('GG-01', f'{rel(app / "manifest.toml")}:1', f"window = {m.get('window', '(absent)')!r}"))
    perms = set((m or {}).get('permissions', []))
    for pm in sorted(perms):
        if pm not in KNOWN_PERMS:
            findings.append(('APP-06', f'{rel(app / "manifest.toml")}:1', f'unknown permission {pm!r}'))
        elif pm in PERM_APP and app.name != PERM_APP[pm]:
            findings.append(('APP-06', f'{rel(app / "manifest.toml")}:1', f'permission {pm!r} is restricted to {PERM_APP[pm]}'))
        elif pm in PERM_USE and not re.search(PERM_USE[pm], code_all):
            findings.append(('APP-06', f'{rel(app / "manifest.toml")}:1', f'permission {pm!r} requested but no code uses it'))

    hand_seen, spawn_sites, next_wake_site = set(), [], None
    for p, (raw, lines) in files.items():
        for i, line in enumerate(lines):
            code = line.split('//')[0]
            if not code.strip():
                continue
            is_use = code.lstrip().startswith('use ')
            if LITERAL_FIT.search(code):
                add(p, i, 'TYP-02', 'literal size in fit()', raw)
            if LITERAL_SIZE.search(code):
                add(p, i, 'TYP-01', 'literal font size', raw)
            if not is_use and MOBILE_PT.search(code):
                add(p, i, 'TYP-03', f'mobile text constant {MOBILE_PT.search(code).group(0)}', raw)
            if LITERAL_COLOR.search(code) or (HEX_COLOR.search(code) and COLOURISH.search(code)):
                add(p, i, 'COL-01', 'literal colour', raw)
            if not is_use:
                for tok in FIXED:
                    if re.search(r'\b' + tok + r'\b(?!\s*:)', code):
                        add(p, i, 'COL-02', f'fixed dark token {tok}', raw)
                        break
            if (BG_FILL.search(code) and not re.search(r'text|blit_colored|glyph', code)) or BG_PACK.search(code):
                add(p, i, 'COL-03', 'hard-coded white/black fill', raw)
            for g in RRECT.finditer(code):
                args = split_args(code[g.end():])
                if args and len(args) > 4:
                    r = args[4]
                    v = num(r)
                    mm = re.fullmatch(r'ui\.pt\(\s*([\d.]+)\s*\)', r)
                    if (v is not None and 0 < v < 1000) or (mm and num(mm.group(1))):
                        add(p, i, 'LAY-01', f'literal radius {r}', raw)
            if RADIUS_CONST.search(code):
                add(p, i, 'LAY-02', 'literal radius constant', raw)
            if FIXED_RES.search(code) and not is_use:
                add(p, i, 'LAY-03', f'fixed resolution {FIXED_RES.search(code).group(1)}', raw)
            for g in PT_LITERAL.finditer(code):
                n = g.group(1)
                if n in SPACING:
                    add(p, i, 'LAY-05', f'literal ui.pt({n}); use {SPACING[n]}', raw)
                elif int(n) >= 100:
                    add(p, i, 'LAY-04', f'ui.pt({n}) fixed size/position', raw)
            if STATUS_INSET.search(code) and not is_use:
                add(p, i, 'LAY-06', 'STATUS_PT', raw)
            if HAND_TITLE.search(code) and not is_use and 'fit(' not in code:
                add(p, i, 'LAY-07', 'bold MAC_LARGE_TITLE_PT text', raw)
            if THREAD_SPAWN.search(code):
                spawn_sites.append((p, i, raw))
            if re.search(r'\bfn\s+next_wake\b', code):
                body = rust_function_body(lines, i)
                next_wake_site = (p, i, raw, body)
            if MENU_DECL.search(code) and MENU_DECL.search(code).group(1) == 'Edit' and re.search(r'AppMenu|MenuEntry|set_menus|main_menu', '\n'.join(lines[max(0, i - 3):i + 1]) + code):
                add(p, i, 'CHR-01', f'menu "{MENU_DECL.search(code).group(1)}" duplicates the shell bar', raw)
            elif EDIT_MENU_CALL.search(code):
                add(p, i, 'CHR-01', 'edit_menu() adds a second Edit menu next to the shell Edit', raw)
            if STD_EDIT_ITEM.search(code):
                add(p, i, 'CHR-02', f'hand-built "{STD_EDIT_ITEM.search(code).group(1)}" item', raw)
            if not is_use:
                for name, hint in DEPRECATED.items():
                    if len(name) > 3 and re.search(r'\b' + re.escape(name) + r'\b\s*[(:<]', code) and 'fn ' + name not in code:
                        add(p, i, 'SDK-01', f'{name}: {hint}', raw)
            for rule, rx in HAND.items():
                if not kit[HAND_KIT[rule]] and rule not in hand_seen and rx.search(code):
                    hand_seen.add(rule)
                    add(p, i, rule, f'{RULES[rule][2]} ({rx.search(code).group(0)})', raw)
    for p, i, raw in spawn_sites:
        add(p, i, 'RES-01', 'thread::spawn', raw)
    if next_wake_site and res02_required(next_wake_site[3], code_all, spawn_sites):
        add(*next_wake_site[:2], 'RES-02', 'next_wake schedules a timed poll while worker work is pending', next_wake_site[2])
    if executable and not re.search(r'enable_sudden_termination', code_all) and not UNSAVED.search(code_all) and 'documents' not in perms:
        findings.append(('RES-03', f'{rel(app / "src")}:1', 'no unsaved state found and enable_sudden_termination() is never called'))

    # tests
    tp = app / 'tests/compliance.rs'
    if uses_bifrost_app(app):
        if not tp.exists():
            findings.append(('TST-01', f'{rel(tp)}:1', 'missing'))
        else:
            want = generate_test(app, m, info)
            if want is not None and test_body(tp.read_text()) != test_body(want):
                findings.append(('TST-02', f'{rel(tp)}:1', 'code below the per-app block differs from tools/templates/app-compliance.rs'))
    return findings, info


# ---------------------------------------------------------------------------------------------------------------
def uses_bifrost_app(app):
    c = app / 'Cargo.toml'
    return c.exists() and 'bifrost-app' in c.read_text()


def test_body(text):
    return text.split('// ---- end per-app ----', 1)[-1]


def bin_name(app, m):
    c = tomllib.loads((app / 'Cargo.toml').read_text())
    bins = [b.get('name') for b in c.get('bin', [])]
    exec_ = (m or {}).get('exec')
    if exec_ and (not bins or exec_ in bins):
        return exec_
    return c['package']['name'] if not bins or c['package']['name'] in bins else bins[0]


def generate_test(app, m, info, old=None):
    if not TEMPLATE.exists() or not uses_bifrost_app(app):
        return None
    bn = bin_name(app, m)
    app_id = (m or {}).get('id') or ('is.olibuijr.keyboard' if app.name == 'keyboard' else app.name)
    gaps = ['menus'] if any(r == 'CHR-01' for r, _, _ in (old or [])) else []
    if old is None:
        gaps = list(KNOWN.get(app.name, []))
    frac = VOLATILE.get(app.name, 0.02 if app.name in SOFT_VOLATILE else 0.002)
    keyboard = app.name == 'keyboard'
    initial_chrome = INITIAL_CHROME.get(app.name, {})
    expect_sidebar = initial_chrome.get('sidebar', info['sidebar'] and not keyboard)
    expect_toolbar = initial_chrome.get('toolbar', info['toolbar'] and not keyboard)
    expect_search = initial_chrome.get('search', info['search'] and not keyboard)
    head = [
        f'const BIN: &str = env!("CARGO_BIN_EXE_{bn}");',
        f'const BIN_NAME: &str = "{bn}";',
        f'const APP_ID: &str = "{app_id}";',
        f'const SNAP_NAME: &str = "{app.name}";',
        f'const EXPECT_SIDEBAR: bool = {str(expect_sidebar).lower()};  // source_list in the source',
        f'const EXPECT_TOOLBAR: bool = {str(expect_toolbar).lower()};  // widgets::toolbar in the source',
        f'const EXPECT_SEARCH: bool = {str(expect_search).lower()};  // search_field in the source',
        f'/// {"TODO(lead): the system keyboard draws only after keyboard_config; it renders no frame when started alone." if keyboard else "An app must draw a frame on start."}',
        f'const FRAME_REQUIRED: bool = {str(not keyboard).lower()};',
        f'/// Fraction of 1/4-size pixels allowed to differ from the golden ({"clock face and time" if app.name in VOLATILE else "dates, feeds and live data" if app.name in SOFT_VOLATILE else "static UI"}).',
        f'const MAX_DIFF_FRAC: f64 = {frac};',
        '/// Check ids (render, chrome, labels, menus, snapshot) that fail today; printed instead of failed. Delete an id when fixed.',
        'const KNOWN_GAPS: &[&str] = &[' + ', '.join(f'"{g}"' for g in gaps) + '];',
    ]
    return TEMPLATE.read_text().replace('@HEADER@', '\n'.join(head))


# The runtime harness starts apps with no open document or journal entries. Preview's sidebar appears after a file
# opens; Journal's timeline is empty until the first entry is created. Neither launch frame can publish sidebar rows.
INITIAL_CHROME = {
    # The user must open these context views before their kit chrome becomes visible.
    'calculator-desktop': {'sidebar': False},
    'freeform-desktop': {'search': False},
    'podcasts-desktop': {'search': False},
    'preview-desktop': {'sidebar': False},
    'journal-desktop': {'sidebar': False},
}
KNOWN = {}


def preserve_known(tp):
    """KNOWN_GAPS of an existing test file, so regenerating keeps the ratchet."""
    if not tp.exists():
        return None
    m = re.search(r'const KNOWN_GAPS: &\[&str\] = &\[([^\]]*)\]', tp.read_text())
    return re.findall(r'"([^"]+)"', m.group(1)) if m else None


def gen_tests(write):
    stale = 0
    for app in sorted(APPS.iterdir()):
        if not app.is_dir() or not uses_bifrost_app(app):
            continue
        findings, info = check_app(app)
        tp = app / 'tests/compliance.rs'
        prev = preserve_known(tp)
        if prev is not None:
            KNOWN[app.name] = prev
        else:
            KNOWN[app.name] = ['menus'] if any(r == 'CHR-01' for r, _, _ in findings) else []
        text = generate_test(app, info['manifest'], info)
        if tp.exists() and tp.read_text() == text:
            continue
        stale += 1
        if write:
            tp.parent.mkdir(exist_ok=True)
            tp.write_text(text)
            print('wrote', rel(tp))
        else:
            print('stale', rel(tp))
        cargo = app / 'Cargo.toml'
        if write and 'springboard' not in cargo.read_text():
            cargo.write_text(cargo.read_text().rstrip('\n') + '\n\n[dev-dependencies]\nspringboard = { path = "../..", features = ["testkit"] }\n')
            print('added springboard dev-dependency to', rel(cargo))
    return 0 if write or not stale else 1


# ---------------------------------------------------------------------------------------------------------------
def score(counts, findings):
    pen = Counter()
    for rule, n in counts.items():
        w = ERROR_W if RULES[rule][0] == 'error' else WARN_W
        import math
        pen[rule] = min(CAP, w * (1 + math.log2(n)))
    return max(0, round(100 - sum(pen.values())))


def all_apps():
    return sorted(p for p in APPS.iterdir() if p.is_dir())


def run_checks(only=None):
    out = {}
    for app in all_apps():
        if only and app.name != only:
            continue
        findings, info = check_app(app)
        counts = dict(Counter(r for r, _, _ in findings))
        out[app.name] = {'findings': findings, 'counts': counts, 'score': score(counts, findings), 'native': info['native']}
    return out


def top_rules(counts, k=3):
    return ', '.join(f'{r} x{n}' for r, n in sorted(counts.items(), key=lambda kv: (-(ERROR_W if RULES[kv[0]][0] == "error" else WARN_W) * kv[1], kv[0]))[:k]) or 'none'


def write_doc(res):
    rows = sorted(res.items(), key=lambda kv: (kv[1]['score'], kv[0]))
    lines = [DOC_BEGIN, '', '| App | Score | Findings | Top 3 rules |', '|---|---:|---:|---|']
    for name, r in rows:
        lines.append(f"| {name} | {r['score']} | {sum(r['counts'].values())} | {top_rules(r['counts'])} |")
    lines += ['', DOC_END]
    text = DOC.read_text() if DOC.exists() else ''
    if DOC_BEGIN in text:
        text = re.sub(re.escape(DOC_BEGIN) + r'.*?' + re.escape(DOC_END), '\n'.join(lines), text, flags=re.S)
    else:
        text = text.rstrip('\n') + '\n\n' + '\n'.join(lines) + '\n'
    DOC.write_text(text)


# ---------------------------------------------------------------------------------------------------------------
COMMON = {'new', 'default', 'from', 'into', 'test', 'tests', 'main', 'draw', 'layout', 'event', 'update', 'format', 'clone', 'name',
          'text', 'size', 'rect', 'color', 'value', 'label', 'data', 'state', 'view', 'init', 'build', 'with', 'then', 'some', 'none',
          'self', 'mod', 'pub', 'type', 'const', 'static', 'impl', 'trait', 'struct', 'enum', 'true', 'false'}


def changed_sdk(base):
    paths = SDK_DIRS
    for b in ([base] if base else ['main', 'origin/main', 'HEAD~1']):
        r = subprocess.run(['git', '-C', str(ROOT), 'diff', '-U0', '--src-prefix=a/', '--dst-prefix=b/', b, '--'] + paths, capture_output=True, text=True)
        if r.returncode == 0:
            base = b
            break
    else:
        print('no usable base ref (pass --base REF)', file=sys.stderr)
        return 2
    names, per_file = {}, {}
    decl = re.compile(r'\b(?:pub(?:\([a-z]+\))?\s+)?(?:fn|struct|enum|trait|type|const|static|mod)\s+([A-Za-z_]\w*)')
    impl = re.compile(r'\bimpl(?:<[^>]*>)?\s+(?:[\w:]+(?:<[^>]*>)?\s+for\s+)?([A-Z]\w*)')
    cur = None
    for line in r.stdout.splitlines():
        if line.startswith('--- a/'):
            cur = line[6:]  # deleted files have no +++ b/ line
            per_file.setdefault(cur, set())
        elif line.startswith('+++ b/'):
            cur = line[6:]
            per_file.setdefault(cur, set())
        elif line.startswith('@@'):
            ctx = line.split('@@')[-1]
            for rx in (decl, impl):
                for m in rx.finditer(ctx):
                    names.setdefault(m.group(1), set()).add(cur)
        elif (line.startswith('+') or line.startswith('-')) and not line.startswith(('+++', '---')):
            for rx in (decl, impl):
                for m in rx.finditer(line[1:]):
                    names.setdefault(m.group(1), set()).add(cur)
    names = {n: f for n, f in names.items() if n.lower() not in COMMON and not n.startswith('test') and (n[0].isupper() or '_' in n or len(n) >= 7)}
    # a changed file with no named item (constants, data tables): every public item of the file counts
    for f, _ in per_file.items():
        if not any(f in fs for fs in names.values()):
            for m in decl.finditer((ROOT / f).read_text(errors='replace') if (ROOT / f).exists() else ''):
                if m.group(1).lower() not in COMMON and (m.group(1)[0].isupper() or '_' in m.group(1) or len(m.group(1)) >= 7):
                    names.setdefault(m.group(1), set()).add(f)
    hits = {}
    for app in all_apps():
        if not (app / 'src').is_dir():
            continue
        text = '\n'.join(p.read_text(errors='replace') for p in sorted((app / 'src').rglob('*.rs')))
        used = sorted(n for n in names if re.search(r'\b' + re.escape(n) + r'\b', text))
        if used:
            hits[app.name] = used
    print(f'SDK files changed vs {base}: {len(per_file)}; public items: {len(names)}')
    for f in sorted(per_file):
        print('  ', f)
    if not hits:
        print('no app uses a changed item')
        return 0
    print('\nApps using changed items (re-run their gate and review their snapshots):')
    for a, used in sorted(hits.items()):
        pkg = tomllib.loads((APPS / a / 'Cargo.toml').read_text())['package']['name']
        shown = ', '.join(used[:12]) + (f' (+{len(used) - 12} more)' if len(used) > 12 else '')
        print(f'  {a}: {shown}')
        print(f'      cargo test --offline --profile fast -p {pkg} --test compliance')
    return 0


# ---------------------------------------------------------------------------------------------------------------
def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--app', help='check one app (directory name under springboard/apps)')
    ap.add_argument('--json', action='store_true')
    ap.add_argument('--strict', action='store_true', help='fail on any finding')
    ap.add_argument('--update-baseline', action='store_true')
    ap.add_argument('--write-doc', action='store_true')
    ap.add_argument('--gen-tests', action='store_true')
    ap.add_argument('--write', action='store_true', help='with --gen-tests: write the files (default: check only)')
    ap.add_argument('--changed-sdk', action='store_true')
    ap.add_argument('--base', help='git ref for --changed-sdk (default: main)')
    a = ap.parse_args()
    if a.changed_sdk:
        return changed_sdk(a.base)
    if a.gen_tests:
        return gen_tests(a.write)
    only = Path(a.app).name if a.app else None
    res = run_checks(only)
    if only and only not in res:
        print(f'no such app: {a.app}', file=sys.stderr)
        return 2
    base = json.loads(BASELINE.read_text()) if BASELINE.exists() else {}
    regress = []
    for name, r in res.items():
        b = base.get(name, {})
        for rule, n in r['counts'].items():
            if n > b.get(rule, 0):
                regress.append(f'{name} {rule}: {n} > baseline {b.get(rule, 0)}')
    if a.json:
        print(json.dumps({n: {'score': r['score'], 'counts': r['counts'], 'findings': [{'rule': x, 'where': w, 'message': m, 'hig': RULES[x][1], 'hint': RULES[x][3]} for x, w, m in r['findings']]} for n, r in res.items()}, indent=2))
        return 1 if regress and not a.update_baseline else 0
    else:
        for name, r in sorted(res.items(), key=lambda kv: kv[1]['score']):
            print(f"{r['score']:>3}  {name}: {sum(r['counts'].values())} finding(s)  [{top_rules(r['counts'])}]")
            if only:
                for rule, where, msg in sorted(r['findings'], key=lambda t: (t[0], t[1])):
                    level, hig, title, hint = RULES[rule]
                    print(f'     {level:7} {rule} (hig {hig}) {where}: {msg}\n             fix: {hint}')
        total = sum(sum(r['counts'].values()) for r in res.values())
        print(f'\n{len(res)} app(s), {total} finding(s), mean score {sum(r["score"] for r in res.values()) / max(1, len(res)):.0f}')
    if a.update_baseline:
        if only:
            print('--update-baseline needs the whole tree (omit --app)', file=sys.stderr)
            return 2
        BASELINE.write_text(json.dumps({n: dict(sorted(r['counts'].items())) for n, r in sorted(res.items())}, indent=2) + '\n')
        print(f'baseline written to {rel(BASELINE)}')
    if a.write_doc:
        write_doc(res)
        print(f'table written to {rel(DOC)}')
    if a.update_baseline or a.write_doc:
        return 0
    if a.strict:
        n = sum(sum(r['counts'].values()) for r in res.values())
        print(f'strict: {"FAIL" if n else "ok"} ({n} finding(s))')
        return 1 if n else 0
    if not BASELINE.exists():
        print('no baseline yet: run with --update-baseline')
        return 1
    for g in regress:
        print('REGRESSION', g)
    better = [n for n, r in res.items() for rule, c in base.get(n, {}).items() if r['counts'].get(rule, 0) < c]
    if better and not regress and not only:
        print(f'improved: {", ".join(sorted(set(better)))} (run --update-baseline to lock it in)')
    print('app compliance ratchet: ' + ('FAIL' if regress else 'ok'))
    return 1 if regress else 0


if __name__ == '__main__':
    sys.exit(main())
