#!/usr/bin/env python3
"""Graphics benchmark for the window server (bifrost-wm): frame statistics, input-to-photon and animation smoothness.

Reads the frame records that `springboard/wm/src/render_server/stats.rs` appends to
`$XDG_RUNTIME_DIR/bifrost-wm-frames.jsonl` when `BIFROST_WM_STATS=1`, one JSON object per line:

    {"seq":7,"cpu_us":1500,"gpu_us":2000,"latency_us":16000,"missed":1,"damaged_px":12345}

and optionally a `"t_us"` key (CLOCK_MONOTONIC microseconds at which the frame reached the screen), which the
input-to-photon measure needs. Without `t_us` the input-to-photon numbers are reported as not measured.

Measures (budgets from docs/graphics-architecture.md, "Performance budgets", Midget 2560x1440 at 60 Hz):
  compose CPU p50/p95/p99 (ms)        gate: p95 <= 1.0 ms
  GPU time p50/p95/p99 (ms)           gate: p95 <= 4.0 ms
  present latency p50/p95/p99 (ms)    reported (submission to the vblank that showed it)
  dropped-frame ratio                 gate: <= 1 % of frames with a missed vblank, over busy frames (damage > 0)
  input-to-photon p50/p95/p99 (ms)    gate: p95 < 25 ms (click injected over uinput, first frame showing the change)

Usage:
  tools/graphics_bench.py --file FILE [--no-record]       analyse a local copy of the frame file
  tools/graphics_bench.py --midget [--click X Y] [--no-record]
                                                          read the file from Midget (ssh), optionally inject a click
                                                          with `tools/midget.py tap X Y` and bracket it with the
                                                          device clock for input-to-photon
  --history records go to docs/perf-history.jsonl unless --no-record.
Exit codes: 0 pass, 2 no data or an error, 3 a budget breach (listed on stdout).

Percentiles are nearest-rank, as in stats.rs: the p-th percentile of n sorted samples is the value at rank
ceil(p/100 * n), 1-based, clamped to 1..n.
"""
import json, math, os, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TOOLS = os.path.dirname(os.path.abspath(__file__))
HIST = os.path.join(ROOT, "docs", "perf-history.jsonl")
STATS_REMOTE = "/run/user/1000/bifrost-wm-frames.jsonl"

# Budgets from docs/graphics-architecture.md. Each is a (key, limit, kind) with kind "max" (value must be <= limit)
# or "lt" (value must be < limit).
BUDGETS = [
    ("gfx_cpu_p95_ms", 1.0, "max"),
    ("gfx_gpu_p95_ms", 4.0, "max"),
    ("gfx_dropped_ratio", 0.01, "max"),
    ("gfx_photon_p95_ms", 25.0, "lt"),
]


def percentile(values, p):
    """Nearest-rank percentile (p in 0..100). Empty input gives None."""
    if not values:
        return None
    s = sorted(values)
    p = min(max(float(p), 0.0), 100.0)
    rank = max(1, math.ceil(p / 100.0 * len(s) - 1e-9))
    return s[min(rank, len(s)) - 1]


def parse_frames(text):
    """JSON lines to frame dicts. A trailing partial line (the server may be writing) and bad lines are skipped."""
    frames = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            f = json.loads(line)
        except ValueError:
            continue
        if isinstance(f, dict) and "seq" in f and "cpu_us" in f and "gpu_us" in f:
            frames.append(f)
    return frames


def _pcts(values_ms):
    return {p: percentile(values_ms, p) for p in (50, 95, 99)}


def dropped_ratio(frames):
    """Fraction of frames whose vblank was missed (missed > 0). None when there are no frames."""
    if not frames:
        return None
    return sum(1 for f in frames if f.get("missed", 0) > 0) / len(frames)


def busy_frames(frames):
    """Frames that changed pixels: the animation and busy-app frames that the no-drop budget covers."""
    return [f for f in frames if f.get("damaged_px", 0) > 0]


def photon_latencies_ms(events, frames):
    """Input-to-photon per click. `events` is a list of (t_before_us, t_after_us): the device clock read just before
    and just after the injected click. The photon time is the first frame with `t_us` at or after the click's
    `t_after_us`, which shows the change, so the latency is bounded by `first_t - t_before_us`. Frames without `t_us`
    are ignored. Clicks with no later frame are dropped."""
    shown = sorted(f["t_us"] for f in frames if "t_us" in f)
    out = []
    for before, after in events:
        for t in shown:
            if t >= after:
                out.append((t - before) / 1000.0)
                break
    return out


def summarize(frames, events=None):
    """All numbers the gate and the history use. Times are milliseconds."""
    res = {"frames": len(frames)}
    cpu = _pcts([f["cpu_us"] / 1000.0 for f in frames])
    gpu = _pcts([f["gpu_us"] / 1000.0 for f in frames])
    lat = _pcts([f.get("latency_us", 0) / 1000.0 for f in frames])
    for name, d in (("gfx_cpu", cpu), ("gfx_gpu", gpu), ("gfx_latency", lat)):
        for p, v in d.items():
            if v is not None:
                res[f"{name}_p{p}_ms"] = v
    busy = busy_frames(frames)
    res["gfx_busy_frames"] = len(busy)
    # An idle desktop has no busy frames: the ratio then covers every frame.
    res["gfx_dropped_ratio"] = dropped_ratio(busy or frames)
    if events:
        photon = _pcts(photon_latencies_ms(events, frames))
        for p, v in photon.items():
            if v is not None:
                res[f"gfx_photon_p{p}_ms"] = v
        res["gfx_photon_clicks"] = len(photon_latencies_ms(events, frames))
    return res


def gate(res):
    """Breaches of BUDGETS. A missing measure counts as a breach (it was not measured)."""
    breaches = []
    for key, limit, kind in BUDGETS:
        v = res.get(key)
        if v is None:
            breaches.append(f"{key}: not measured (budget {'<' if kind == 'lt' else '<='} {limit})")
        elif (kind == "max" and v > limit) or (kind == "lt" and v >= limit):
            breaches.append(f"{key}: {v:.3f} over budget {'<' if kind == 'lt' else '<='} {limit}")
    return breaches


def _ssh_text(cmd, timeout=60):
    sys.path.insert(0, TOOLS)
    import midget  # the shared ssh options and lab key
    r = subprocess.run(midget.SSH + [cmd], capture_output=True, text=True, timeout=timeout)
    return r.stdout


DEVICE_CLOCK = "import time; print(time.monotonic_ns() // 1000)"


def inject_click(x, y):
    """One click at screen pixels (x, y) through the uinput helper; returns (t_before_us, t_after_us)
    read from the device monotonic clock around the injection. Uses `tools/midget.py tap`."""
    before = int(_ssh_text(f"python3 -c '{DEVICE_CLOCK}'").strip() or 0)
    subprocess.run([sys.executable, os.path.join(TOOLS, "midget.py"), "tap", str(x), str(y)], timeout=30)
    after = int(_ssh_text(f"python3 -c '{DEVICE_CLOCK}'").strip() or 0)
    return before, after


def record(res, no_record=False):
    if no_record:
        return
    sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    entry = {"date": time.strftime("%Y-%m-%d %H:%M"), "sha": sha, "host": os.uname().nodename, "results": res}
    with open(HIST, "a") as f:
        f.write(json.dumps(entry) + "\n")


def report(res, breaches):
    for k, v in sorted(res.items()):
        print(f"{k:28s} {v:10.3f}" if isinstance(v, float) else f"{k:28s} {v}")
    if breaches:
        print("\nover budget (docs/graphics-architecture.md):")
        for b in breaches:
            print("  " + b)


def midget_results(click=None):
    """Reads the frame file from Midget and returns the summarized results (the `bench.py --graphics` hook).
    `click` is an optional (x, y) pair injected before the read; it adds the input-to-photon numbers."""
    events = [inject_click(*click)] if click else None
    if click:
        time.sleep(0.5)
    frames = parse_frames(_ssh_text(f"cat {STATS_REMOTE} 2>/dev/null"))
    return summarize(frames, events) if frames else {}


def main(argv):
    no_record = "--no-record" in argv
    events = None
    if "--file" in argv:
        with open(argv[argv.index("--file") + 1]) as f:
            text = f.read()
    elif "--midget" in argv:
        text = _ssh_text(f"cat {STATS_REMOTE} 2>/dev/null")
        if "--click" in argv:
            i = argv.index("--click")
            x, y = argv[i + 1], argv[i + 2]
            events = [inject_click(x, y)]
            time.sleep(0.5)
            text = _ssh_text(f"cat {STATS_REMOTE} 2>/dev/null")
    else:
        sys.exit("graphics_bench: pass --file FILE or --midget")
    frames = parse_frames(text)
    if not frames:
        print("graphics_bench: no frame records (is BIFROST_WM_STATS=1 set and the window server running?)", file=sys.stderr)
        sys.exit(2)
    res = summarize(frames, events)
    breaches = gate(res)
    report(res, breaches)
    record(res, no_record)
    sys.exit(3 if breaches else 0)


if __name__ == "__main__":
    main(sys.argv[1:])
