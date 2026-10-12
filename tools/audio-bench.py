#!/usr/bin/env python3
"""Run the audio benches (bifrost-audio-bench) on the host or Midget, compare with history, check budgets.

Usage: tools/audio-bench.py host|midget|compare [--append] [--quanta 16,32,64,128] [--bench NAME] [--aloop]
On Midget the device benches run with --live: through the running bifrost-audiod (HAL client), driver "live:<uid>".
Every bench prints one JSON object (docs/audio-latency.md). Budgets (a breach exits 3):
  midi-latency    p99 <= quantum + output latency + 0.5 ms
  audio-roundtrip measured p50 within 1 frame of reported_ms
  xrun-sweep      (Midget only) smallest stable quantum <= 64
"""
import json, os, re, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HIST = os.path.join(ROOT, "docs", "perf-history.jsonl")
BENCHES = ["midi-latency", "audio-roundtrip", "xrun-sweep", "midi-throughput"]
QUANTA = [16, 32, 64, 128]
REMOTE_BIN = "/usr/bin/bifrost-audio-bench"
SLACK_MS = 0.5
# a live latency outside 0..MAX_SANE_MS is a measurement error (mixed clocks or units), never a result
MAX_SANE_MS = 1000.0
# bifrost-audio-bench exit code: audio-roundtrip --live heard nothing (no cable / loopback)
EXIT_NO_LOOPBACK = 4


class Skipped(Exception):
    """A bench that cannot run on this hardware (not a failure, not a result)."""
MAX_SWEEP_QUANTUM = 64
# benches that have a --live mode (midi-throughput measures the scheduler only)
LIVE_BENCHES = ("midi-latency", "audio-roundtrip", "xrun-sweep")


def parse_result(text):
    """The last stdout line that is a JSON object with a 'bench' key."""
    for line in reversed([l.strip() for l in text.splitlines() if l.strip()]):
        if line.startswith("{"):
            try:
                obj = json.loads(line)
            except ValueError:
                continue
            if isinstance(obj, dict) and "bench" in obj:
                return obj
    raise ValueError("no bench JSON object in output")


def parse_cards(aplay_l):
    """Card ids from `aplay -l` ('card 0: PCH [HDA Intel PCH], device 0: ...'), in order, without duplicates."""
    cards = []
    for m in re.finditer(r"^card \d+: (\S+) \[", aplay_l, re.M):
        if m.group(1) not in cards:
            cards.append(m.group(1))
    return cards


def pick_card(cards, prefer_loopback=False):
    """Prefer a real card over snd-aloop's Loopback (--aloop: the Loopback card, so audio-roundtrip hears itself)."""
    if prefer_loopback:
        loop = [c for c in cards if c.lower() == "loopback"]
        if loop:
            return loop[0]
    real = [c for c in cards if c.lower() != "loopback"]
    return (real or cards or [None])[0]


def quantum_ms(r):
    return r["quantum"] * 1000.0 / r["rate"]


def budget_breaches(r, midget=False):
    """List of breach strings for one result."""
    b, out = r.get("bench"), []
    if b == "midi-latency":
        # reported_ms is the output path the HAL predicts (output latency + safety offset + one quantum); without it,
        # fall back to one quantum
        rep = r.get("reported_ms")
        lim = (rep if rep is not None else quantum_ms(r)) + SLACK_MS
        if r["p99_ms"] > lim:
            out.append(f"midi-latency q{r['quantum']}: p99 {r['p99_ms']:.3f} > {lim:.3f} ms")
    elif b == "audio-roundtrip":
        rep = r.get("reported_ms")
        if rep is not None:
            frame = 1000.0 / r["rate"]
            if abs(r["p50_ms"] - rep) > frame:
                out.append(f"audio-roundtrip q{r['quantum']}: p50 {r['p50_ms']:.3f} vs reported {rep:.3f} ms (> 1 frame {frame:.3f})")
    elif b == "xrun-sweep" and midget:
        q = (r.get("extra") or {}).get("smallest_clean_quantum")
        if q is None or q > MAX_SWEEP_QUANTUM:
            out.append(f"xrun-sweep: smallest stable quantum {q} > {MAX_SWEEP_QUANTUM}")
    return out


def history_line(r, date, git):
    return {"date": date, "git": git, "bench": "audio-" + r["bench"], **{k: v for k, v in r.items() if k != "bench"}}


def load_history(path=HIST):
    rows = []
    if os.path.exists(path):
        for l in open(path):
            l = l.strip()
            if l.startswith("{"):
                try:
                    d = json.loads(l)
                except ValueError:
                    continue
                if str(d.get("bench", "")).startswith("audio-"):
                    rows.append(d)
    return rows


def compare_table(results, history):
    """Rows bench, quantum, p50, p99, max, and the p99 change against the latest earlier entry (same bench, quantum, host)."""
    lines = [f"{'bench':18s} {'q':>4s} {'p50':>8s} {'p99':>8s} {'max':>8s}  vs previous p99"]
    for r in sorted(results, key=lambda r: (r["bench"], r["quantum"])):
        prev = None
        for h in history:
            if h.get("bench") == "audio-" + r["bench"] and h.get("quantum") == r["quantum"] and h.get("host") == r.get("host"):
                prev = h
        note = "(no previous)"
        if prev and prev.get("p99_ms"):
            note = f"{prev['p99_ms']:.3f} ({(r['p99_ms'] - prev['p99_ms']) / prev['p99_ms'] * 100:+.1f}%)"
        lines.append(f"{r['bench']:18s} {r['quantum']:4d} {r['p50_ms']:8.3f} {r['p99_ms']:8.3f} {r['max_ms']:8.3f}  {note}")
    return "\n".join(lines)


def find_local_bin():
    for prof in ("fast", "release"):
        p = os.path.join(ROOT, "springboard", "target", prof, "bifrost-audio-bench")
        if os.path.exists(p):
            return p
    sys.exit("audio-bench: bifrost-audio-bench is not built (springboard/target/fast or release)")


def bench_cmd(argv, bench, driver, quantum, live=False):
    cmd = argv + [bench, "--driver", driver, "--quantum", str(quantum), "--json"]
    if live and bench in LIVE_BENCHES:
        cmd.append("--live")
    return cmd


def run_one(argv, bench, driver, quantum, timeout=120, live=False):
    cmd = bench_cmd(argv, bench, driver, quantum, live)
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if p.returncode == EXIT_NO_LOOPBACK:
        raise Skipped(p.stderr.strip()[:300] or "no loopback: connect line out to line in, or run with --aloop")
    if p.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)}: exit {p.returncode}: {p.stderr.strip()[:300]}")
    if "usage" in p.stderr.lower() and "bench" not in p.stdout:
        raise RuntimeError(f"{' '.join(cmd)}: usage text instead of a result: {p.stderr.strip()[:300]}")
    return parse_result(p.stdout)


def result_problem(r):
    """Why a parsed result is not a measurement (None when it is): latency benches with all-zero statistics ran nothing."""
    if r.get("bench") in ("midi-latency", "audio-roundtrip"):
        if all(not r.get(k) for k in ("p50_ms", "p99_ms", "max_ms")):
            return "no measurements (p50, p99 and max are all 0)"
        for k in ("min_ms", "p50_ms", "p99_ms", "max_ms"):
            v = r.get(k)
            if isinstance(v, (int, float)) and not (0 <= v <= MAX_SANE_MS):
                return f"measurement error: {k} {v} outside 0..{MAX_SANE_MS:g} ms (clock or unit mix-up)"
    return None


def collect(argv, driver, benches, quanta, live=False, skipped=None):
    """(results, failures): skipped (a list, if given) collects "bench qN: reason" for benches that cannot run here. A bench that fails (exit code, usage text, no JSON, timeout, empty statistics) is a
    failure string and never a result, so it cannot be compared, budget-checked or appended to the history."""
    out, failures = [], []
    for b in benches:
        # the sweep chooses its own quanta; run it once
        for q in ([min(quanta)] if b == "xrun-sweep" else quanta):
            try:
                r = run_one(argv, b, driver, q, live=live)
            except Skipped as e:
                if skipped is not None:
                    skipped.append(f"{b} q{q}: {e}")
                continue
            except (RuntimeError, ValueError, subprocess.TimeoutExpired, OSError) as e:
                failures.append(f"{b} q{q}: {e}")
                continue
            problem = result_problem(r)
            if problem:
                failures.append(f"{b} q{q}: {problem}")
            else:
                out.append(r)
    return out, failures


def run_mode(mode, quanta=QUANTA, benches=BENCHES, aloop=False, skipped=None):
    """Run the benches for 'host' or 'midget'. Returns (results, breaches, failures)."""
    midget = mode == "midget"
    if midget:
        # midget.py's ssh: the lab key and whichever Midget address answers (a fresh install has only the lab NIC, so the
        # `midget` alias for the VLAN address finds nothing)
        import importlib.util
        spec = importlib.util.spec_from_file_location("midget", os.path.join(os.path.dirname(os.path.abspath(__file__)), "midget.py"))
        lab = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(lab)
        ssh = list(lab.SSH)
        if aloop:
            subprocess.run(ssh + ["modprobe snd-aloop"], check=False)
        card = pick_card(parse_cards(subprocess.run(ssh + ["aplay -l"], capture_output=True, text=True).stdout), prefer_loopback=aloop)
        if not card:
            sys.exit("audio-bench: no sound card on Midget (aplay -l)")
        argv, driver = ssh + [REMOTE_BIN], "alsa:" + card
    else:
        argv, driver = [find_local_bin()], "loopback"
    results, failures = collect(argv, driver, benches, quanta, live=midget, skipped=skipped)
    return results, [x for r in results for x in budget_breaches(r, midget)], failures


def append_history(results):
    git = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    date = time.strftime("%Y-%m-%d %H:%M")
    with open(HIST, "a") as f:
        for r in results:
            f.write(json.dumps(history_line(r, date, git)) + "\n")


def main():
    args = sys.argv[1:]
    if not args or args[0] not in ("host", "midget", "compare"):
        sys.exit(__doc__)
    mode = args[0]
    opt = lambda n, d=None: args[args.index(n) + 1] if n in args else d
    quanta = [int(x) for x in opt("--quanta", ",".join(map(str, QUANTA))).split(",")]
    benches = [opt("--bench")] if opt("--bench") else BENCHES
    hist = load_history()
    if mode == "compare":
        last = {}
        for h in hist:
            last[(h["bench"], h["quantum"], h.get("host"))] = h
        latest = [{**h, "bench": h["bench"][6:]} for h in last.values()]
        earlier = [h for h in hist if last[(h["bench"], h["quantum"], h.get("host"))] is not h]
        print(compare_table(latest, earlier))
        return
    skipped = []
    results, breaches, failures = run_mode(mode, quanta, benches, "--aloop" in args, skipped)
    if results:
        print(compare_table(results, hist))
    # a failed bench never reaches the history, even when others succeeded
    if "--append" in args:
        if failures:
            print("\nnot appending to the history: some benches failed", file=sys.stderr)
        elif results:
            append_history(results)
    if breaches:
        print("\nover budget:")
        for b in breaches:
            print("  " + b)
    if skipped:
        print("\nSKIPPED (not failed, not appended):")
        for x in skipped:
            print("  " + x)
    if failures:
        print("\nfailed benches:", file=sys.stderr)
        for f in failures:
            print("  " + f, file=sys.stderr)
        sys.exit(1)
    sys.exit(3 if breaches else 0)


if __name__ == "__main__":
    main()
