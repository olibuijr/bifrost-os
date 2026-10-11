#!/usr/bin/env python3
"""Measure Midget's idle cost: CPU and wakeups for every process, aggregated by name.

usage: tools/midget-idle-measure.py LABEL [--seconds N] [--json]

Two /proc samples N seconds apart (default 10). Per name (executable basename, or the kernel comm for kernel
threads):
  cpu_percent_one_core  utime+stime delta, percent of one core
  wakeups_per_s         voluntary + nonvoluntary context switches, summed over all threads, per second
Also: cpu_total_percent (every task), cpu_userspace_percent, wakeups_userspace_per_s, and i915_flip_per_s (the
i915 flip kworkers, `i915_flip` in their name; Snow Leopard baseline 3.6/s). Runs remotely over the same ssh
command as tools/midget.py (host detection and key included). Writes out/render-performance/LABEL-idle.json.
Prints a table sorted by CPU then wakeups and a summary; `--json` prints the full per-name JSON (used by bench.py).
Read-only: nothing on Midget is changed.
"""
import argparse, json, subprocess, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
sys.path.insert(0, str(HERE))
import midget  # noqa: E402  (host detection, ssh options, lab key)

REMOTE = r'''
import os, pathlib, time, json
hz = os.sysconf('SC_CLK_TCK')
def read(p):
    try:
        return pathlib.Path(p).read_text()
    except OSError:
        return None
def sample():
    procs = {}
    for d in pathlib.Path('/proc').iterdir():
        if not d.name.isdigit() or int(d.name) == os.getpid():
            continue
        stat = read(d / 'stat')
        if stat is None:
            continue
        try:
            v = stat[stat.rindex(')') + 2:].split()
            cpu = int(v[11]) + int(v[12])          # utime + stime, clock ticks (process-wide)
            start = int(v[19])                     # starttime: tells a reused pid apart
        except (ValueError, IndexError):
            continue
        try:
            exe = os.path.basename(os.readlink(d / 'exe').replace(' (deleted)', ''))
        except OSError:
            exe = None                             # kernel thread: no exe, use comm (full name in /proc/PID/comm)
        name = exe or (read(d / 'comm') or '?').strip()
        ctx = 0
        for t in (d / 'task').iterdir():
            st = read(t / 'status') or ''
            for line in st.splitlines():
                if line.startswith(('voluntary_ctxt_switches', 'nonvoluntary_ctxt_switches')):
                    ctx += int(line.split()[1])
        procs[int(d.name)] = dict(name=name, start=start, cpu=cpu, ctx=ctx, user=exe is not None)
    return procs
# settle: the measuring ssh login starts root's systemd user manager, which talks to the bus for a few s
time.sleep(SETTLE)
before = sample(); t0 = time.monotonic(); time.sleep(SECONDS); el = time.monotonic() - t0; after = sample()
by = {}
for pid, a in after.items():
    b = before.get(pid)
    if b is None or b['start'] != a['start']:
        continue
    r = by.setdefault(a['name'], dict(cpu=0.0, wakeups=0.0, procs=0, user=False))
    r['cpu'] += (a['cpu'] - b['cpu']) / hz * 100 / el
    r['wakeups'] += (a['ctx'] - b['ctx']) / el
    r['procs'] += 1
    r['user'] = r['user'] or a['user']
print(json.dumps(dict(seconds=round(el, 2), by_name=by)))
'''


def measure(seconds):
    script = REMOTE.replace('SECONDS', repr(float(seconds))).replace('SETTLE', '10.0')
    r = subprocess.run(midget.SSH + ['python3 -'], input=script, text=True, capture_output=True, timeout=seconds + 60)
    if r.returncode:
        sys.exit(f"ssh to midget failed ({r.returncode}): {r.stderr.strip()[-400:]}")
    return json.loads(r.stdout.strip().splitlines()[-1])


def summarise(label, raw):
    by = raw['by_name']
    cpu_one = {k: round(v['cpu'], 3) for k, v in by.items()}
    wake = {k: round(v['wakeups'], 2) for k, v in by.items()}
    flip = sum(v['wakeups'] for k, v in by.items() if 'i915_flip' in k)
    return {
        'label': label,
        'seconds': raw['seconds'],
        'cpu_total_percent': round(sum(cpu_one.values()), 3),
        'cpu_userspace_percent': round(sum(v['cpu'] for v in by.values() if v['user']), 3),
        'wakeups_total_per_s': round(sum(wake.values()), 2),
        'wakeups_userspace_per_s': round(sum(v['wakeups'] for v in by.values() if v['user']), 2),
        'i915_flip_per_s': round(flip, 2),
        'cpu_percent_one_core': cpu_one,
        'wakeups_per_s': wake,
    }


def table(data):
    cpu, wake = data['cpu_percent_one_core'], data['wakeups_per_s']
    rows = sorted(cpu, key=lambda k: (-cpu[k], -wake[k], k))
    out = [f"{'name':<36}{'cpu %':>9}{'wakeups/s':>12}"]
    for k in rows[:40]:
        if cpu[k] or wake[k]:
            out.append(f"{k[:35]:<36}{cpu[k]:>9.3f}{wake[k]:>12.2f}")
    out.append(f"{'TOTAL (all tasks)':<36}{data['cpu_total_percent']:>9.3f}{data['wakeups_total_per_s']:>12.2f}")
    out.append(f"{'userspace only':<36}{data['cpu_userspace_percent']:>9.3f}{data['wakeups_userspace_per_s']:>12.2f}")
    out.append(f"{'i915 flip workers (wakeups)':<36}{'':>9}{data['i915_flip_per_s']:>12.2f}")
    return '\n'.join(out)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('label')
    p.add_argument('--seconds', type=int, default=10)
    p.add_argument('--json', action='store_true', help='print only the JSON (bench.py)')
    a = p.parse_args()
    if not 1 <= a.seconds <= 60:
        p.error('seconds must be 1..60')
    data = summarise(a.label, measure(a.seconds))
    out = REPO / 'out/render-performance'
    out.mkdir(parents=True, exist_ok=True)
    name = ''.join(c if c.isalnum() or c in '-_' else '_' for c in a.label)
    text = json.dumps(data, indent=2) + '\n'
    (out / (name + '-idle.json')).write_text(text)
    summary = {k: v for k, v in data.items() if not isinstance(v, dict)}
    if a.json:
        print(text, end='')
    else:
        print(table(data))
        print(json.dumps(summary, indent=2))
        print(f'full per-name JSON: {out / (name + "-idle.json")}')


if __name__ == '__main__':
    main()
