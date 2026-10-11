#!/usr/bin/env python3
"""Replace the legacy status-bar inset (STATUS_PT) in desktop apps with the SDK's safe inset.

  const NAV_BOTTOM: f32 = STATUS_PT + 44.0;   ->   fn nav_bottom_pt() -> f32 { top_inset() + 44.0 }
  ui.pt(STATUS_PT + 12.0)                     ->   ui.pt(top_inset() + 12.0)
`top_inset()` is the shell's safe inset from the last Configure (0 on the desktop). Run per app:
  python3 tools/fix-status-inset.py springboard/apps/news-desktop
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONST = re.compile(r'^(?P<ind>[ \t]*)(?P<vis>pub(?:\([a-z]+\))?\s+)?const (?P<name>[A-Z0-9_]+): f32 = STATUS_PT(?:\s*\+\s*(?P<n>[0-9.]+))?;[ \t]*$', re.M)
LOCAL = re.compile(r'^[ \t]*const STATUS_PT: f32 = [0-9.]+;[ \t]*\n', re.M)
# A constant built from an already converted one (`const LIST_TOP: f32 = search_top_pt() + 52.0;`).
CHAINED = re.compile(r'^(?P<ind>[ \t]*)(?P<vis>pub(?:\([a-z]+\))?\s+)?const (?P<name>[A-Z0-9_]+): f32 = (?P<rhs>[^;]*\b[a-z0-9_]+_pt\(\)[^;]*);[ \t]*$', re.M)
USE = re.compile(r'^use springboard::scene::STATUS_PT;\n', re.M)


def fn_name(name):
    low = name.lower()
    return low if low.endswith('_pt') else low + '_pt'


def main():
    for arg in sys.argv[1:]:
        files = sorted(p for p in (Path(arg).resolve() / 'src').rglob('*.rs'))
        renames = {}
        texts = {p: p.read_text() for p in files}
        for p, t in texts.items():
            t = LOCAL.sub('', t)
            def const(m):
                renames[m['name']] = fn_name(m['name'])
                n = m['n'] or '0.0'
                body = 'top_inset()' if n in ('0', '0.0') else f'top_inset() + {n}'
                return f"{m['ind']}{m['vis'] or ''}fn {fn_name(m['name'])}() -> f32 {{ {body} }}"
            texts[p] = CONST.sub(const, t)
        for _ in range(6):  # chained constants, to a fixpoint
            changed = False
            for p, t in list(texts.items()):
                def chained(m):
                    renames[m['name']] = fn_name(m['name'])
                    return f"{m['ind']}{m['vis'] or ''}fn {fn_name(m['name'])}() -> f32 {{ {m['rhs']} }}"
                new = CHAINED.sub(chained, t)
                changed |= new != t
                texts[p] = new
            if not changed:
                break
        for p, t in texts.items():
            for old, new in renames.items():
                # uses, not the definition and not `use` lines
                t = re.sub(r'(?<![A-Za-z0-9_])' + old + r'(?![A-Za-z0-9_(])(?!: f32)', new + '()', t)
            t = USE.sub('', t)
            t = re.sub(r'\bSTATUS_PT\b', 'top_inset()', t)
            if 'top_inset()' in t and 'layout::top_inset' not in t and not str(p).endswith('tests.rs'):
                uses = list(re.finditer(r'^use .*;$', t, re.M))
                a = uses[0].start() if uses else 0
                t = t[:a] + 'use bifrost_app::layout::top_inset;\n' + t[a:]
            if t != p.read_text():
                p.write_text(t)
                print(p.relative_to(ROOT))


main()
