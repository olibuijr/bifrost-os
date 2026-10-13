#!/usr/bin/env python3
"""Read-only Linux process-tree PSS report, executed remotely by midget mem."""
from pathlib import Path
import subprocess

subprocess.run(['free', '-h'], check=True)
rows = {}
for path in Path('/proc').iterdir():
    if not path.name.isdigit():
        continue
    try:
        stat = (path / 'stat').read_text()
        comm = stat[stat.index('(') + 1:stat.rindex(')')]
        parent = int(stat[stat.rindex(')') + 2:].split()[1])
        exe = str((path / 'exe').resolve())
        rows[int(path.name)] = (parent, comm, exe)
    except (OSError, ValueError):
        pass


def descendants(roots):
    ids = set(roots)
    while True:
        expanded = ids | {p for p, (parent, _, _) in rows.items() if parent in ids}
        if expanded == ids:
            return ids
        ids = expanded


def report(name, ids):
    total, count = 0, 0
    for pid in ids:
        try:
            for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
                if line.startswith('Pss:'):
                    total += int(line.split()[1])
                    count += 1
        except OSError:
            pass
    if count:
        print(f'{name}: {total / 1024:.0f} MiB PSS ({count} processes)', flush=True)


report('BifrOSt shell', {p for p, (_, comm, _) in rows.items() if comm == 'springboard'})
report('Wayland compositor', {p for p, (_, comm, _) in rows.items() if comm == 'bifrost-wm'})
report('Firefox including children', descendants({p for p, (_, comm, _) in rows.items() if comm == 'firefox'}))
report('BifrOSt Browser including children', descendants({p for p, (_, _, exe) in rows.items() if '/bifrost-browser/' in exe and exe.endswith('/servoshell')}))
