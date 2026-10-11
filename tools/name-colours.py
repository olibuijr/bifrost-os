#!/usr/bin/env python3
"""Replace literal `pack(r, g, b, 255)` colours in app code with the named constant that has the same value.

The constants come from springboard/render/src/ui.rs (`pub const NAME: u32 = pack(r, g, b, 255);`). A literal is
only replaced when exactly one name matches, or the first match in ui.rs order when several do; the pixel value is
the same, so nothing moves. Colours with no named constant are left and reported (they need a new token).
  python3 tools/name-colours.py --dry springboard/apps/*-desktop   # print the changes
  python3 tools/name-colours.py springboard/apps/*-desktop         # write them
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

CONST = re.compile(r'^pub const ([A-Z][A-Z0-9_]*): u32 = pack\(\s*(\d+),\s*(\d+),\s*(\d+),\s*255\s*\);', re.M)
PACK = re.compile(r'\bpack\(\s*(\d+),\s*(\d+),\s*(\d+),\s*255\s*\)')


def named_colours():
    text = chk.UI_RS.read_text()
    table = {}
    for m in CONST.finditer(text):
        rgb = tuple(int(x) for x in m.group(2, 3, 4))
        table.setdefault(rgb, m.group(1))  # first in ui.rs order wins
    return table


def ensure_imports(text, names):
    """Add `use bifrost_app::kit::ui::{...};` for the names the file does not import yet."""
    lines = text.split('\n')
    imported = set()  # every name any `use` statement brings in, including multi-line `use a::{ ... };` blocks
    for stmt in re.findall(r'^use [^;]*;', text, re.M | re.S):
        imported.update(re.findall(r'\b[A-Z][A-Z0-9_]*\b', stmt))
    missing = sorted(set(names) - imported)
    if not missing:
        return text
    last = max((i for i, l in enumerate(lines) if l.startswith('use ')), default=-1)
    if last < 0:
        last = max((i for i, l in enumerate(lines) if l.startswith('//!')), default=-1)
    end = last
    while end < len(lines) and end >= 0 and not lines[end].rstrip().endswith(';'):
        end += 1
    lines.insert(end + 1, f"use bifrost_app::kit::ui::{{{', '.join(missing)}}};")
    return '\n'.join(lines)


def run(path, table, write):
    text = path.read_text()
    live = chk.strip_tests(text)
    lines = text.split('\n')
    used, unnamed, changes = set(), [], []
    for i, line in enumerate(lines):
        if i < len(live) and not live[i]:
            continue
        code, sep, comment = line.partition('//')

        def sub(m):
            rgb = tuple(int(x) for x in m.group(1, 2, 3))
            if rgb in table:
                used.add(table[rgb])
                changes.append((i + 1, rgb, table[rgb]))
                return table[rgb]
            unnamed.append((i + 1, rgb))
            return m.group(0)

        new = PACK.sub(sub, code)
        if new != code:
            lines[i] = new + sep + comment
    if changes and write:
        path.write_text(ensure_imports('\n'.join(lines), used))
    return changes, unnamed


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--dry', action='store_true')
    ap.add_argument('dirs', nargs='+')
    args = ap.parse_args()
    table = named_colours()
    total, left = 0, 0
    for d in args.dirs:
        for p in chk.rust_files(Path(d).resolve()):
            changes, unnamed = run(p, table, write=not args.dry)
            for line, rgb, name in changes:
                print(f'{p.relative_to(ROOT)}:{line}: pack{rgb} -> {name}')
            total += len(changes)
            left += len(unnamed)
            for line, rgb in unnamed:
                print(f'{p.relative_to(ROOT)}:{line}: no named colour for {rgb}')
    print(f'{total} replaced, {left} without a name' + (' (dry run)' if args.dry else ''))
    return 0


if __name__ == '__main__':
    sys.exit(main())
