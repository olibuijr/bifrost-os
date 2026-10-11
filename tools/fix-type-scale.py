#!/usr/bin/env python3
"""Rewrite literal mobile-scale font sizes in app text calls to the Golden Gate tokens (render/src/ui.rs).

Uses the same detector as check-app-consistency.py, so it touches exactly what TYP-01 flags.
Role mapping (mobile text style -> Golden Gate style): 34->large title, 28/22->title 1, 20->title 3, 17->body,
16->callout, 15->subhead, 13->footnote, 12->caption 1; off-scale sizes snap to the nearest style, display
numerals (36, 64, 92, 100) take DISPLAY_*. Any other size is reported and left alone.
  python3 tools/fix-type-scale.py springboard/apps/music-desktop
"""
import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('chk', ROOT / 'tools/check-app-consistency.py')
chk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(chk)

TOKENS = {'34': 'MAC_LARGE_TITLE_PT', '28': 'MAC_TITLE1_PT', '22': 'MAC_TITLE1_PT', '20': 'MAC_TITLE3_PT',
          '17': 'MAC_BODY_PT', '16': 'MAC_CALLOUT_PT', '15': 'MAC_SUBHEAD_PT', '13': 'MAC_FOOTNOTE_PT',
          '12': 'MAC_CAPTION1_PT',
          # off-scale sizes snap to the nearest HIG style
          '11': 'MAC_SUBHEAD_PT', '14': 'MAC_BODY_PT', '18': 'MAC_TITLE2_PT', '19': 'MAC_TITLE2_PT',
          '21': 'MAC_TITLE1_PT', '24': 'MAC_TITLE1_PT', '25': 'MAC_LARGE_TITLE_PT', '26': 'MAC_LARGE_TITLE_PT',
          '30': 'MAC_LARGE_TITLE_PT', '35': 'MAC_LARGE_TITLE_PT',
          # display numerals
          '36': 'DISPLAY_SMALL_PT', '64': 'DISPLAY_MEDIUM_PT', '92': 'DISPLAY_LARGE_PT', '100': 'DISPLAY_LARGE_PT'}


def fix_file(path):
    lines = path.read_text().split('\n')
    text0 = '\n'.join(lines)
    used, skipped = set(), []
    for i, line in enumerate(lines):
        # `fit(text, 17.0, width)` first: its size is the one the text is drawn with
        for _ in range(4):
            fm = chk.LITERAL_FIT.search(line.split('//')[0])
            if not fm:
                break
            n = re.match(r'\d+', fm.group(1)).group(0)
            tok = TOKENS.get(n)
            if not tok:
                skipped.append((i + 1, fm.group(1)))
                break
            new = re.sub(r'(\bfit\([^;]*?,\s*)' + n + r'(?:\.\d*)?(\s*,)', lambda g: g.group(1) + tok + g.group(2), line, count=1)
            if new == line:
                break
            line = new
            used.add(tok)
        code = line.split('//')[0]
        for _ in range(8):  # several sizes can share a line
            m = chk.LITERAL_SIZE.search(code)
            if not m:
                break
            n = re.match(r'\d+', m.group(1)).group(0)
            tok = TOKENS.get(n)
            if not tok:
                skipped.append((i + 1, m.group(1)))
                break
            # the literal before `Weight::`, and the same size as the width argument of a `fit(` on the line
            new = re.sub(r'(,\s*)' + n + r'(?:\.\d*)?(\s*,\s*Weight::)', lambda g: g.group(1) + tok + g.group(2), line)
            new = re.sub(r'(fit\([^()]*?,\s*)' + n + r'(?:\.\d*)?(\s*,)', lambda g: g.group(1) + tok + g.group(2), new)
            if new == line:
                break
            line = new
            code = line.split('//')[0]
            used.add(tok)
        lines[i] = line
    if not used:
        return used, skipped
    text = '\n'.join(lines)
    imported = '\n'.join(l for l in text0.split('\n') if l.startswith('use '))
    names = ', '.join(sorted(t for t in used if t not in imported))
    if names:
        uses = list(re.finditer(r'^use .*;$', text, re.M))
        anchor = uses[0].start() if uses else 0
        text = text[:anchor] + f'use bifrost_app::kit::ui::{{{names}}};\n' + text[anchor:]
    path.write_text(text)
    return used, skipped


def main():
    for arg in sys.argv[1:]:
        for p in chk.rust_files(Path(arg).resolve()):
            used, skipped = fix_file(p)
            if used:
                print(f'{p.relative_to(ROOT)}: {len(used)} token(s)')
            for ln, size in skipped:
                print(f'{p.relative_to(ROOT)}:{ln}: no token for size {size}, left alone')


main()
