#!/usr/bin/env python3
"""How far each desktop app has moved to the Golden Gate SDK kit (docs/golden-gate-sdk-adoption.md).

For every app under springboard/apps it counts uses of the fixed dark colour tokens (which do not follow Light and
Dark), uses of the scheme Pair tokens, calls into bifrost_app::widgets, and whether the manifest asks for a
full-size window. Run with --json for machine output. Counting is by token name in the source, so it is a progress
measure, not a proof that every pixel follows the scheme.
"""
import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# WHITE is left out: it is the right colour for text on the accent (a selected row), in both schemes.
FIXED = ['SECONDARY', 'FILL', 'FILL_PRESSED', 'FILL_GRAY', 'FILL_TERTIARY', 'SIDEBAR_FILL', 'SIDEBAR_SELECTED',
         'WINDOW_BAR_FOCUSED', 'WINDOW_BAR_BLURRED', 'SEPARATOR', 'CELL', 'TERTIARY', 'HAIRLINE', 'TRACK']
PAIRS = ['LABEL', 'LABEL_2', 'LABEL_3', 'CARD_BG', 'DETAIL_BG', 'SIDEBAR_BG', 'SEPARATOR_C', 'SELECTION', 'FIELD_BG',
         'ROW_PRESSED', 'SIDEBAR_PLATE', 'CAPSULE_BG', 'HAIRLINE_C', 'CONTROL_PRESSED', 'TRACK_C']
WIDGET = re.compile(r'widgets::([a-z_]+)\s*\(')


def scan(app: Path) -> dict:
    src = '\n'.join(p.read_text(errors='replace') for p in sorted(app.rglob('*.rs')) if 'target' not in p.parts)
    # code only: drop line comments so documentation does not count
    code = re.sub(r'//[^\n]*', '', src)
    count = lambda names: sum(len(re.findall(r'\b' + n + r'\b(?!\s*:)', code)) for n in names)
    manifest = app / 'manifest.toml'
    text = manifest.read_text() if manifest.exists() else ''
    widgets = sorted(set(WIDGET.findall(code)))
    return {
        'app': app.name,
        'fixed': count(FIXED),
        'pairs': count(PAIRS),
        'widgets': widgets,
        'full_size': bool(re.search(r'^window\s*=\s*"(full-size|borderless)"', text, re.M)),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--json', action='store_true')
    args = parser.parse_args()
    rows = [scan(d) for d in sorted((ROOT / 'springboard/apps').iterdir()) if (d / 'src').is_dir()]
    if args.json:
        print(json.dumps(rows, indent=2))
        return
    print(f"{'app':22} {'fixed':>5} {'pairs':>5}  full-size  widgets")
    for r in rows:
        print(f"{r['app']:22} {r['fixed']:5} {r['pairs']:5}  {'yes' if r['full_size'] else 'no':9}  {', '.join(r['widgets']) or '-'}")
    adopted = sum(1 for r in rows if r['widgets'] and r['fixed'] == 0)
    print(f"\n{adopted} of {len(rows)} apps use the kit with no fixed dark colours")


if __name__ == '__main__':
    main()
