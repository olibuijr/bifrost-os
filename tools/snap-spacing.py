#!/usr/bin/env python3
"""Snap small off-grid spacing literals `ui.pt(n)` (n <= 24) to the nearest SPACE_* token.

Only values up to 24 pt are touched (spacing, not widths or sizes). Each value moves by at most 2 pt, so a snap is a
visible change: check it with tools/pc-app-smoke.py and a screenshot before merging. Test code is skipped.
  python3 tools/snap-spacing.py --dry springboard/apps/calendar-desktop   # print the changes
  python3 tools/snap-spacing.py springboard/apps/calendar-desktop         # write them
"""
import argparse
import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('chk', ROOT / 'tools/check-app-consistency.py')
chk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(chk)

NAMES = {4: 'SPACE_XS', 8: 'SPACE_S', 12: 'SPACE_M', 16: 'SPACE_L', 20: 'SPACE_XL', 24: 'SPACE_XXL'}
PT = re.compile(r'\bui\.pt\(\s*(\d+)(?:\.\d*)?\s*\)')


def nearest(n):
    """Nearest 4-pt token; ties go up. Values of 2 or less are left alone."""
    if n <= 2:
        return None
    return min(NAMES, key=lambda k: (abs(k - n), -k))


def snap_file(path, write):
    text = path.read_text()
    lines = text.split('\n')
    live = chk.strip_tests(text)
    changes = []
    for i, line in enumerate(lines):
        if i < len(live) and not live[i]:
            continue
        code, sep, comment = line.partition('//')

        def sub(g):
            n = int(g.group(1))
            if n > 24 or n in chk.SPACING:
                return g.group(0)
            k = nearest(n)
            if k is None:
                return g.group(0)
            changes.append((i + 1, n, k))
            return f'ui.pt({NAMES[k]})'

        new = PT.sub(sub, code)
        if new != code:
            lines[i] = new + sep + comment
    if changes and write:
        text = ensure_imports('\n'.join(lines))
        path.write_text(text)
    return changes


def ensure_imports(text):
    """Import each SPACE_* token the file uses but does not import yet (a separate `use` line after the last use)."""
    lines = text.split('\n')
    imported = set()
    for line in lines:
        if line.lstrip().startswith('use ') and 'kit::ui' in line:
            imported.update(re.findall(r'\bSPACE_[A-Z0-9]+\b', line))
    used = {t for t in re.findall(r'\bSPACE_[A-Z0-9]+\b', text)}
    missing = sorted(used - imported)
    if not missing:
        return text
    last_use = max((i for i, l in enumerate(lines) if l.startswith('use ')), default=-1)
    if last_use < 0:  # no use lines: go after the module docs (`//!`), which must come first
        last_use = max((i for i, l in enumerate(lines) if l.startswith('//!')), default=-1)
    end = last_use  # a `use` may span lines: insert after its closing `;`
    while end < len(lines) and not lines[end].rstrip().endswith(';'):
        end += 1
    lines.insert(end + 1, f"use bifrost_app::kit::ui::{{{', '.join(missing)}}};")
    return '\n'.join(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--dry', action='store_true', help='print the changes, write nothing')
    ap.add_argument('dirs', nargs='+')
    args = ap.parse_args()
    total = 0
    for d in args.dirs:
        for p in chk.rust_files(Path(d).resolve()):
            ch = snap_file(p, write=not args.dry)
            for line, n, k in ch:
                print(f'{p.relative_to(ROOT)}:{line}: ui.pt({n}) -> ui.pt({NAMES[k]})')
            total += len(ch)
    print(f'{total} change(s)' + (' (dry run)' if args.dry else ''))
    return 0


if __name__ == '__main__':
    sys.exit(main())
