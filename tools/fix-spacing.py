#!/usr/bin/env python3
"""Replace on-grid spacing literals `ui.pt(16.0)` with the SPACE_* tokens from render/src/ui.rs.

Value-preserving: only 2, 4, 8, 12, 16, 20, 24, 32 and 40 are rewritten, so no pixel moves and the apps' own
tests keep passing. Off-grid literals (10, 14, 18, 30, 44, ...) are left for a deliberate snap.
  python3 tools/fix-spacing.py springboard/apps/*-desktop springboard/apps/files
"""
import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('chk', ROOT / 'tools/check-app-consistency.py')
chk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(chk)


def fix_file(path):
    text0 = path.read_text()
    lines = text0.split('\n')
    live = chk.strip_tests(text0)  # blank where the line belongs to a #[cfg(test)] item
    used = set()
    for i, line in enumerate(lines):
        if i < len(live) and not live[i] and line.strip():
            continue
        code, sep, comment = line.partition('//')

        def sub(g):
            tok = chk.SPACING.get(g.group(1))
            if not tok:
                return g.group(0)
            used.add(tok)
            return f'ui.pt({tok})'
        new = chk.PT_LITERAL.sub(sub, code)
        lines[i] = new + sep + comment
    if not used:
        return used
    text = '\n'.join(lines)
    imported = '\n'.join(l for l in text0.split('\n') if l.startswith('use '))
    names = ', '.join(sorted(t for t in used if t not in imported))
    if names:
        uses = list(re.finditer(r'^use .*;$', text, re.M))
        anchor = uses[0].start() if uses else 0
        text = text[:anchor] + f'use bifrost_app::kit::ui::{{{names}}};\n' + text[anchor:]
    path.write_text(text)
    return used


for arg in sys.argv[1:]:
    for p in chk.rust_files(Path(arg).resolve()):
        used = fix_file(p)
        if used:
            print(f'{p.relative_to(ROOT)}: {len(used)} token(s)')
