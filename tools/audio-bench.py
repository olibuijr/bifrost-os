#!/usr/bin/env python3
"""Run the audio benches (bifrost-audio-bench) on the host or Midget, compare with history, check budgets.

Usage: tools/audio-bench.py host|midget|compare [--append] [--quanta 16,32,64,128] [--bench NAME] [--aloop]
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
MAX_SWEEP_QUANTUM = 64


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


def pick_card(cards):
    """Prefer a real card over snd-aloop's Loopback."""
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


def run_one(argv, bench, driver, quantum, timeout=120):
    cmd = argv + [bench, "--driver", driver, "--quantum", str(quantum)]
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if p.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)}: exit {p.returncode}: {p.stderr.strip()[:300]}")
    return parse_result(p.stdout)


def collect(argv, driver, benches, quanta):
    out = []
    for b in benches:
        # the sweep chooses its own quanta; run it once
        for q in ([min(quanta)] if b == "xrun-sweep" else quanta):
            out.append(run_one(argv, b, driver, q))
    return out


def run_mode(mode, quanta=QUANTA, benches=BENCHES, aloop=False):
    """Run the benches for 'host' or 'midget'. Returns (results, breaches)."""
    midget = mode == "midget"
    if midget:
        ssh = ["ssh", "midget"]
        if aloop:
            subprocess.run(ssh + ["modprobe snd-aloop"], check=False)
        card = pick_card(parse_cards(subprocess.run(ssh + ["aplay -l"], capture_output=True, text=True).stdout))
        if not card:
            sys.exit("audio-bench: no sound card on Midget (aplay -l)")
        argv, driver = ssh + [REMOTE_BIN], "alsa:" + card
    else:
        argv, driver = [find_local_bin()], "loopback"
    results = collect(argv, driver, benches, quanta)
    return results, [x for r in results for x in budget_breaches(r, midget)]


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
    results, breaches = run_mode(mode, quanta, benches, "--aloop" in args)
    print(compare_table(results, hist))
    if "--append" in args:
        append_history(results)
    if breaches:
        print("\nover budget:")
        for b in breaches:
            print("  " + b)
    sys.exit(3 if breaches else 0)


if __name__ == "__main__":
    main()
