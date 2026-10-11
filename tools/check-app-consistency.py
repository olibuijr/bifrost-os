#!/usr/bin/env python3
"""Check every BifrOSt PC desktop app against the shared design rules (AGENTS.md, bifrost-hig).

Static checks on source and manifests; no build needed. Exit 1 when a finding is at error level.
  python3 tools/check-app-consistency.py            # report
  python3 tools/check-app-consistency.py --strict   # warnings fail too
  python3 tools/check-app-consistency.py --ratchet  # fail only when a count rises above the baseline
  python3 tools/check-app-consistency.py --update-baseline   # record current counts (after fixing some)

The baseline (tools/app-consistency-baseline.json) holds the accepted findings per app and rule.
Counts may only go down; an app missing from the baseline must have none.
"""
import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APPS = ROOT / 'springboard/apps'
UI_RS = ROOT / 'springboard/render/src/ui.rs'
BASELINE = ROOT / 'tools/app-consistency-baseline.json'

# Any call passing a literal size right before a weight: ui.text("x", 13.0, Weight::..), self.right(.., 17., Weight::..)
LITERAL_SIZE = re.compile(r',\s*(\d+(?:\.\d*)?)\s*,\s*Weight::')
# `ui.fit(text, 17.0, width)` measures at a literal size: it must be the token the text is drawn with.
LITERAL_FIT = re.compile(r'\bfit\([^;]*?,\s*(\d+(?:\.\d*)?)\s*,')
# Literal colours: pack(...) or rgba(...) calls in app code.
LITERAL_COLOR = re.compile(r'\b(?:pack|rgba)\s*\(\s*\d')
# Hand-rolled large titles: a text call with the bifrost large title size or the mac one.
SHARED_TITLE_DRAW = re.compile(r'large_title\(')


# Spacing: ui.pt(<literal>) must be a SPACE_* token (on-grid) or a reviewed off-grid value (flagged).
SPACING = {'2': 'SPACE_XXS', '4': 'SPACE_XS', '8': 'SPACE_S', '12': 'SPACE_M', '16': 'SPACE_L',
           '20': 'SPACE_XL', '24': 'SPACE_XXL', '32': 'SPACE_XXXL', '40': 'SPACE_HUGE'}
PT_LITERAL = re.compile(r'\bui\.pt\(\s*(\d+)(?:\.\d*)?\s*\)')
# Layout: apps lay out from bifrost_app::layout::Layout, not from a hand-drawn status bar or hand-drawn titles.
STATUS_INSET = re.compile(r'\bSTATUS_PT\b')
# A hand-drawn title is bold text at the large-title size; glyphs (+, chevrons), values and empty states at that size are not.
HAND_TITLE = re.compile(r'\bMAC_LARGE_TITLE_PT\b\s*,\s*Weight::Bold\b')


def rust_files(app):
    for p in sorted((app / 'src').rglob('*.rs')):
        name = p.name
        if name.endswith('tests.rs') or name.startswith('test') or '/tests/' in str(p):
            continue
        yield p


def strip_tests(text):
    """Source lines with `#[cfg(test)]` items blanked (line numbers are preserved).

    An item is a braced block (module, fn, impl) or a single statement ending in `;`."""
    lines = text.splitlines()
    out, i = [], 0
    while i < len(lines):
        if '#[cfg(test)]' not in lines[i]:
            out.append(lines[i])
            i += 1
            continue
        depth, opened = 0, False
        while i < len(lines):
            line = lines[i]
            out.append('')
            code = line.split('//')[0]
            depth += code.count('{') - code.count('}')
            opened = opened or '{' in code
            i += 1
            if (opened and depth <= 0) or (not opened and code.rstrip().endswith(';')):
                break
    return out


def check_app(app):
    findings = []  # (level, rule, where, message)
    try:
        m = tomllib.loads((app / 'manifest.toml').read_text())
    except Exception as e:
        return [('error', 'MAN-01', app.name, f'manifest.toml unreadable: {e}')]
    for key in ('id', 'name', 'version', 'exec', 'icon', 'min_shell', 'summary', 'category', 'architecture'):
        if key not in m:
            findings.append(('error', 'MAN-01', 'manifest.toml', f'missing {key}'))
    if m.get('architecture') != 'x86_64-linux':
        findings.append(('error', 'MAN-02', 'manifest.toml', 'architecture must be x86_64-linux'))
    if not (app / m.get('icon', 'icon.png')).exists():
        findings.append(('error', 'MAN-03', 'manifest.toml', f"icon {m.get('icon')} not found"))
    changelog = app / 'CHANGELOG.md'
    if not changelog.exists():
        findings.append(('error', 'REL-01', 'CHANGELOG.md', 'missing'))
    elif f"{m.get('version')}" not in changelog.read_text():
        findings.append(('error', 'REL-02', 'CHANGELOG.md', f"no entry for version {m.get('version')}"))
    cargo = app / 'Cargo.toml'
    if cargo.exists():
        c = tomllib.loads(cargo.read_text())
        if c.get('package', {}).get('version') != m.get('version'):
            findings.append(('error', 'REL-03', 'Cargo.toml', 'version differs from manifest.toml'))

    for p in rust_files(app):
        lines = strip_tests(p.read_text(errors='replace'))
        for n, line in enumerate(lines, 1):
            where = f'{p.relative_to(ROOT)}:{n}'
            code = line.split('//')[0]
            if LITERAL_FIT.search(code):
                findings.append(('error', 'TYP-02', where, 'literal size in fit(); measure with the same token the text is drawn with'))
            if LITERAL_SIZE.search(code):
                findings.append(('error', 'TYP-01', where, 'literal font size; use a named token from render/src/ui.rs'))
            for g in PT_LITERAL.finditer(code):
                n = g.group(1)
                if n in SPACING:
                    findings.append(('error', 'SPC-01', where, f'literal spacing ui.pt({n}); use {SPACING[n]}'))
                elif int(n) > 1:
                    findings.append(('warning', 'SPC-02', where, f'off-grid literal ui.pt({n}); snap to the 4-pt scale (SPACE_*) or name it'))
            if STATUS_INSET.search(code) and not code.lstrip().startswith('use '):
                findings.append(('error', 'LAY-06', where, 'legacy status-bar inset; take the top inset from bifrost_app::layout::Layout'))
            if HAND_TITLE.search(code) and not code.lstrip().startswith('use ') and 'fit(' not in code:  # measuring is not drawing
                findings.append(('warning', 'LAY-07', where, 'hand-drawn large title; use bifrost_app::widgets::large_title'))
            if LITERAL_COLOR.search(code):
                findings.append(('warning', 'COL-01', where, 'literal colour; add a named constant in render/src/ui.rs'))
    return findings


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--strict', action='store_true', help='warnings fail the run')
    ap.add_argument('--ratchet', action='store_true', help='fail only when a count rises above the baseline')
    ap.add_argument('--update-baseline', action='store_true', help='write current counts as the baseline')
    args = ap.parse_args()
    apps = sorted(p for p in APPS.glob('*-desktop') if p.is_dir())
    apps += [APPS / n for n in ('files', 'browser') if (APPS / n).is_dir()]
    base = json.loads(BASELINE.read_text()) if BASELINE.exists() else {}
    counts, regressions = {}, []
    bad = 0
    for app in apps:
        found = check_app(app)
        per = {}
        for _, rule, _, _ in found:
            per[rule] = per.get(rule, 0) + 1
        counts[app.name] = per
        for rule, n in per.items():
            if n > base.get(app.name, {}).get(rule, 0):
                regressions.append(f'{app.name} {rule}: {n} > baseline {base.get(app.name, {}).get(rule, 0)}')
        errors = sum(1 for f in found if f[0] == 'error')
        warns = sum(1 for f in found if f[0] == 'warning')
        status = 'FAIL' if errors or (args.strict and warns) else 'ok  '
        print(f'{status} {app.name}: {errors} error(s), {warns} warning(s)')
        for level, rule, where, msg in found:
            print(f'     {level:7} {rule} {where}: {msg}')
        bad += errors + (warns if args.strict else 0)
    print(f'\n{len(apps)} apps checked, {bad} blocking finding(s)')
    if args.update_baseline:
        BASELINE.write_text(json.dumps(counts, indent=2, sort_keys=True) + '\n')
        print(f'baseline written to {BASELINE.relative_to(ROOT)}')
        return 0
    if args.ratchet:
        for r in regressions:
            print('REGRESSION', r)
        print('ratchet: ' + ('FAIL' if regressions else 'ok'))
        return 1 if regressions else 0
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
