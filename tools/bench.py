#!/usr/bin/env python3
"""Run the BifrOSt benchmarks and append the results to docs/perf-history.jsonl.

Benchmarks are #[ignore]d tests that print `BENCH <name> <value> ms`. Each run records the git sha,
system version and host, prints the change against the previous run, and flags regressions over 15 %.
Usage: tools/bench.py [--frames] [--audio] [--no-record] [--history] [--midget [--reboot] [--apps]] [--graphics [--click X Y]]
--frames runs only the shell frame-time scenes (every `bench_frame_*` test: desktop idle, Settings open, Window menu open,
window drag, Files with 5,000 items; springboard/src/shell/frame_budget.rs and files/bench.rs). Each scene prints the
`frame_<scene>_median` and `frame_<scene>_p95` keys in ms and fails the run (exit 1) when a p95 frame is over 16.7 ms.
--graphics (with --midget) adds the window-server graphics suite (tools/graphics_bench.py: compose CPU and GPU
percentiles, present latency, dropped frames, input-to-photon with --click) and gates it against
docs/graphics-architecture.md ("Performance budgets"); a breach exits 3.
--audio adds the audio latency benches (tools/audio-bench.py; on Midget with --midget, else on the host).
--midget records the shell's on-device GPU presenter averages (journal line "gpu present") and the Midget idle and
footprint numbers (midget_* keys: tools/midget-idle-measure.py, tools/midget-memory.py). It then checks every key in
docs/budgets.toml [midget] (a maximum) and exits 3 listing each key over budget. Acceptance runs it with --no-record.
--midget also runs the app-launch phase (APPS below); `--midget --apps` runs only that phase (keys midget_launch_ms_<app>,
midget_pss_app_<app>_mib, midget_threads_app_<app>), bounded to 55 s.
"""
import json, os, re, subprocess, sys, time, tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HIST = os.path.join(ROOT, "docs", "perf-history.jsonl")
BUDGETS = os.path.join(ROOT, "docs", "budgets.toml")
TOOLS = os.path.dirname(os.path.abspath(__file__))
SUITES = [["-p", "bifrost-render", "glass_cost"], ["-p", "springboard", "--features", "desktop-wayland,testkit", "--lib", "bench_"], ["-p", "bifrost-ui", "--lib", "bench_ui_"], ["-p", "bifrost-files", "--bins", "bench_files_"]]
FRAME_SUITES = [["-p", "springboard", "--features", "desktop-wayland,testkit", "--lib", "bench_frame_"]]


def load():
    if not os.path.exists(HIST):
        return []
    return [json.loads(l) for l in open(HIST) if l.strip()]


def run():
    out = {}
    for suite in (FRAME_SUITES if "--frames" in sys.argv else SUITES):
        pkg, flt = suite[:-1], suite[-1]
        cmd = ["cargo", "test", "--offline", "--profile", "fast"] + pkg + [flt, "--", "--ignored", "--nocapture"]
        r = subprocess.run(cmd, cwd=os.path.join(ROOT, "springboard"), capture_output=True, text=True, timeout=1500)
        for m in re.finditer(r"BENCH (\S+) ([\d.]+) ms", r.stdout):
            out[m.group(1)] = float(m.group(2))
        if r.returncode:
            print(r.stdout[-2000:], r.stderr[-2000:], file=sys.stderr)
            sys.exit(1)
    return out


PROBE = r'''
import json, os, pathlib, subprocess
def pids(name):
    found = []
    for d in pathlib.Path('/proc').iterdir():
        if d.name.isdigit():
            try:
                if os.path.basename(os.readlink(d / 'exe').replace(' (deleted)', '')) == name:
                    found.append(d.name)
            except OSError:
                pass
    return found
def threads(name):
    for p in pids(name):
        for line in pathlib.Path(f'/proc/{p}/status').read_text().splitlines():
            if line.startswith('Threads:'):
                return int(line.split()[1])
    return 0
def du(path):
    return int(subprocess.run(['du', '-sb', path], capture_output=True, text=True).stdout.split()[0])
mem = dict(l.split(':', 1) for l in pathlib.Path('/proc/meminfo').read_text().splitlines())
kb = lambda k: int(mem[k].split()[0])
print(json.dumps(dict(threads_springboard=threads('springboard'), threads_labwc=threads('labwc'),
                      springboard_bytes=os.path.getsize('/usr/bin/springboard'),
                      bifrost_bytes=du('/usr/lib/bifrost') + du('/var/lib/springboard/apps'),
                      ram_used_kb=kb('MemTotal') - kb('MemAvailable'))))
'''


def midget_footprint():
    """Idle CPU and wakeups (midget-idle-measure.py, 30 s), PSS (midget-memory.py), threads, sizes and RAM."""
    sys.path.insert(0, TOOLS)
    import midget  # host detection and ssh options shared with `bifrost pc midget`
    res = {}
    idle = subprocess.run([sys.executable, os.path.join(TOOLS, "midget-idle-measure.py"), "bench", "--seconds", "30", "--json"],
                          capture_output=True, text=True, timeout=300)
    if idle.returncode:
        print(idle.stdout[-1000:], idle.stderr[-1000:], file=sys.stderr)
        sys.exit(1)
    data = json.loads(idle.stdout)
    cpu, wake = data["cpu_percent_one_core"], data["wakeups_per_s"]
    res["midget_idle_cpu_total"] = data["cpu_total_percent"]
    for key, name in (("springboard", "springboard"), ("labwc", "labwc"), ("dbus_broker", "dbus-broker")):
        res[f"midget_idle_cpu_{key}"] = cpu.get(name, 0.0)
    for key, name in (("springboard", "springboard"), ("labwc", "labwc"), ("dbus_broker", "dbus-broker"), ("bootweave", "bifrost-bootweave")):
        res[f"midget_idle_wakeups_{key}"] = wake.get(name, 0.0)
    res["midget_idle_wakeups_userspace"] = data["wakeups_userspace_per_s"]

    with open(os.path.join(TOOLS, "midget-memory.py")) as f:
        mem = subprocess.run(midget.SSH + ["python3 -"], stdin=f, capture_output=True, text=True, timeout=120)
    pss = dict(re.findall(r"^(BifrOSt shell|Wayland compositor): (\d+) MiB PSS", mem.stdout, re.M))
    if "BifrOSt shell" not in pss or "Wayland compositor" not in pss:
        print(mem.stdout[-1000:], mem.stderr[-1000:], file=sys.stderr)
        sys.exit(1)
    res["midget_pss_shell_mib"] = float(pss["BifrOSt shell"])
    res["midget_pss_compositor_mib"] = float(pss["Wayland compositor"])

    # lock, wait, unlock, settle, then the same PSS reading (~20 s)
    midget.cmd_control(["lock"])
    time.sleep(8)
    midget.cmd_key(["enter"])  # the first key can only wake the display; press again while still locked
    for _ in range(3):
        time.sleep(3)
        if "screen=lock" not in (midget.control_text("state", 15) or ""):
            break
        midget.cmd_key(["enter"])
    if "screen=lock" in (midget.control_text("state", 15) or ""):
        # opening apps on the lock screen restarts the session (0.5.18, HANDOFF round 51): stop here
        print("midget still locked after the lock cycle; unlock failed", file=sys.stderr)
        sys.exit(1)
    time.sleep(7)
    with open(os.path.join(TOOLS, "midget-memory.py")) as f:
        mem2 = subprocess.run(midget.SSH + ["python3 -"], stdin=f, capture_output=True, text=True, timeout=120)
    m = re.search(r"^BifrOSt shell: (\d+) MiB PSS", mem2.stdout, re.M)
    if not m:
        print(mem2.stdout[-1000:], mem2.stderr[-1000:], file=sys.stderr)
        sys.exit(1)
    res["midget_pss_shell_after_lock_mib"] = float(m.group(1))

    probe = subprocess.run(midget.SSH + ["python3 -"], input=PROBE, capture_output=True, text=True, timeout=120)
    if probe.returncode:
        print(probe.stderr[-1000:], file=sys.stderr)
        sys.exit(1)
    d = json.loads(probe.stdout.strip().splitlines()[-1])
    res["midget_threads_springboard"] = float(d["threads_springboard"])
    res["midget_threads_labwc"] = float(d["threads_labwc"])
    res["midget_size_springboard_mib"] = d["springboard_bytes"] / 2 ** 20
    res["midget_size_bifrost_total_mib"] = d["bifrost_bytes"] / 2 ** 20
    res["midget_ram_used_mib"] = d["ram_used_kb"] / 1024
    return res


APPS = 5  # first N installed store apps (/var/lib/springboard/apps), sorted, so runs compare
LAUNCH = r'''
import json, os, pathlib, socket, subprocess, sys, time
N, DEADLINE = json.loads(sys.argv[1]), time.time() + 50
APPS = sorted(d.name for d in pathlib.Path("/var/lib/springboard/apps").iterdir() if d.is_dir())[:N]
def ctl(line):
    s = socket.create_connection(("127.0.0.1", 7777), 5); s.settimeout(5)
    s.sendall((line + "\n").encode()); d = b""
    try:
        while True:
            c = s.recv(65536)
            if not c: break
            d += c
    except OSError:
        pass
    return d.decode(errors="replace")
def procs(app_id):
    out = []
    for d in pathlib.Path("/proc").iterdir():
        if d.name.isdigit():
            try:
                if f"/apps/{app_id}/" in os.readlink(d / "exe"):
                    out.append(int(d.name))
            except OSError:
                pass
    return out
def windows():
    return ctl("state").count("window=External")
def pss_threads(pids):
    pss = thr = 0
    for p in pids:
        try:
            for l in pathlib.Path(f"/proc/{p}/smaps_rollup").read_text().splitlines():
                if l.startswith("Pss:"): pss += int(l.split()[1])
            for l in pathlib.Path(f"/proc/{p}/status").read_text().splitlines():
                if l.startswith("Threads:"): thr += int(l.split()[1])
        except OSError:
            pass
    return pss / 1024, thr
res = {}
for app_id in APPS:
    if time.time() > DEADLINE - 8: break
    a = app_id.removeprefix("is.olibuijr.").removesuffix(".desktop").replace(".", "_")
    before, t0 = windows(), time.time()
    ctl("open-app " + app_id)
    # ready = a process runs from the app's directory AND the shell maps one more external window
    while time.time() - t0 < 6:
        if procs(app_id) and windows() > before: break
        time.sleep(0.02)
    else:
        print(f"launch {a}: no window within 6 s", file=sys.stderr)
    if time.time() - t0 < 6:
        res[f"midget_launch_ms_{a}"] = (time.time() - t0) * 1000
        time.sleep(3)
        res[f"midget_pss_app_{a}_mib"], res[f"midget_threads_app_{a}"] = pss_threads(procs(app_id))
    for p in procs(app_id):
        try: os.kill(p, 15)
        except OSError: pass
    time.sleep(0.5)
print(json.dumps(res))
'''


def midget_launch():
    """Cold launch of APPS on Midget: request to first window, then PSS and threads after a 3 s settle, then quit (<55 s)."""
    sys.path.insert(0, TOOLS)
    import midget
    r = subprocess.run(midget.SSH + ["python3 - " + json.dumps(json.dumps(APPS))], input=LAUNCH, capture_output=True, text=True, timeout=55)
    try:
        res = json.loads(r.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        sys.exit("bench: app-launch phase failed: " + (r.stderr or r.stdout)[-300:])
    if not res:
        sys.exit("bench: app-launch phase measured nothing: " + r.stderr[-300:])
    return res


def midget_clean_boot():
    """--reboot: Midget reboots, the shell reports ready, then 15 s for boot work to finish (bounded, ~1.5 min)."""
    sys.path.insert(0, TOOLS)
    import midget as m
    t = time.time()
    if m.cmd_reboot([]):
        sys.exit("bench: Midget did not come back from the reboot")
    if not m.wait(lambda: "ready=1" in (m.control_text("state") or ""), 60, 1):
        sys.exit("bench: shell not ready 60 s after boot")
    # the lab session boots to the lock screen (Return unlocks); measure the desktop, not the lock wallpaper copies
    m.cmd_key(["enter"])
    time.sleep(15)
    print(f"clean boot ready in {time.time() - t:.0f}s")


def midget():
    if "--reboot" in sys.argv:
        midget_clean_boot()
    if "--apps" in sys.argv:
        return midget_launch()
    return {**midget_gpu(), **midget_footprint(), **midget_launch()}



def _browser_procs():
    """pids of the Browser app, its engine and content processes (matched by command line)."""
    out = subprocess.run(["pgrep", "-af", "bifrost-browser|bifrost-webview|is.olibuijr.browser"], capture_output=True, text=True).stdout
    return [int(l.split()[0]) for l in out.splitlines() if l.strip() and "pgrep" not in l]


def _pss_mib(pids):
    kib = 0
    for p in pids:
        try:
            for l in open(f"/proc/{p}/smaps_rollup"):
                if l.startswith("Pss:"):
                    kib += int(l.split()[1])
        except OSError:
            pass
    return kib / 1024


def bench_browser_startup():
    """Cold start to the control socket appearing (a proxy for the first frame). Needs a running desktop session."""
    sock = "/var/lib/springboard/appdata/is.olibuijr.browser.desktop/control.sock"
    t0 = time.time()
    subprocess.run(["bifrost", "pc", "open-app", "is.olibuijr.browser.desktop"], capture_output=True, timeout=30)
    while time.time() - t0 < 20 and not os.path.exists(sock):
        time.sleep(0.05)
    return {"browser_startup_ms": (time.time() - t0) * 1000}


def bench_browser_idle(seconds=10):
    """Idle CPU percent and context-switch wakeups per second of all browser processes."""
    def sample():
        cpu = sw = 0
        for p in _browser_procs():
            try:
                f = open(f"/proc/{p}/stat").read().rsplit(")", 1)[1].split()
                cpu += int(f[11]) + int(f[12])
                for l in open(f"/proc/{p}/status"):
                    if "ctxt_switches" in l:
                        sw += int(l.split()[1])
            except OSError:
                pass
        return cpu, sw
    c0, w0 = sample()
    time.sleep(seconds)
    c1, w1 = sample()
    hz = os.sysconf("SC_CLK_TCK")
    return {"browser_idle_cpu": 100.0 * (c1 - c0) / hz / seconds, "browser_idle_wakeups": (w1 - w0) / seconds}


def bench_browser_memory():
    """PSS of app + engine + content processes at the current tab count (run with 1, 5, 10 tabs open)."""
    return {"browser_pss_mib": _pss_mib(_browser_procs())}


# budgets (maximums) for the browser keys; results are appended to docs/perf-history.jsonl by main()
BROWSER_BUDGETS = {"browser_startup_ms": 1500, "browser_idle_cpu": 1.0, "browser_idle_wakeups": 60, "browser_pss_mib": 600}

def unit(k):
    if k.endswith("kpx"):
        return "kpx"
    if k.endswith("_ms") or k.startswith("midget_gpu") or k == "midget_cpu_draw":
        return "ms"
    if k.startswith("midget_launch_ms"):
        return "ms"
    for part, u in (("_cpu", "%"), ("wakeups", "/s"), ("pss", "MiB"), ("size", "MiB"), ("ram", "MiB"), ("threads", "threads")):
        if part in k:
            return u
    return "ms"


def budget_check(res):
    """Keys in docs/budgets.toml [midget] are maxima. Returns the list of breaches (missing keys count too)."""
    with open(BUDGETS, "rb") as f:
        budgets = tomllib.load(f).get("midget", {})
    breaches = []
    for k, limit in sorted(budgets.items()):
        v = res.get(k)
        if v is None and "--apps" in sys.argv and not k.startswith(("midget_launch", "midget_pss_app", "midget_threads_app")):
            continue
        if v is None and k.startswith(("midget_launch", "midget_pss_app", "midget_threads_app")):
            continue  # per-app budgets apply to the apps installed on Midget at the time
        if v is None:
            breaches.append(f"{k}: not measured (budget {limit})")
        elif v > limit:
            breaches.append(f"{k}: {v:.3f} > budget {limit}")
    return breaches


def midget_gpu():
    r = subprocess.run(["bifrost", "pc", "midget", "ssh", "journalctl -u springboard -b --no-pager | grep 'gpu present' | tail -6"],
                       capture_output=True, text=True, timeout=60)
    rows = re.findall(r"(\d+) frames, upload ([\d.]+) ms, render ([\d.]+) ms, ([\d.]+) kpx uploaded per frame(?:, cpu draw ([\d.]+) ms)?", r.stdout)
    if not rows:
        # right after a clean boot the presenter has not logged an average yet (it logs after drawing)
        print("bench: no 'gpu present' line in this boot's journal; GPU timings skipped", file=sys.stderr)
        return {}
    n = sum(int(f) for f, *_ in rows)
    avg = lambda i: sum(int(row[0]) * float(row[i] or 0) for row in rows) / max(n, 1)
    out = {"midget_gpu_upload": avg(1), "midget_gpu_render": avg(2), "midget_gpu_upload_kpx": avg(3)}
    if all(row[4] for row in rows):
        out["midget_cpu_draw"] = avg(4)
    return out


def _audio_bench():
    import importlib.util
    spec = importlib.util.spec_from_file_location("audio_bench", os.path.join(TOOLS, "audio-bench.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def _bench_audio(bench, mode):
    """p99 per quantum as keys audio_<bench>_q<N>_p99_ms (the full JSON goes through tools/audio-bench.py --append)."""
    ab = _audio_bench()
    results, breaches = ab.run_mode(mode, benches=[bench])
    for b in breaches:
        print("over budget: " + b)
    return {f"audio_{bench.replace('-', '_')}_q{r['quantum']}_p99_ms": r["p99_ms"] for r in results}


def bench_audio_midi_latency(mode="host"):
    return _bench_audio("midi-latency", mode)


def bench_audio_roundtrip(mode="host"):
    return _bench_audio("audio-roundtrip", mode)


def bench_audio_xrun_sweep(mode="midget"):
    return _bench_audio("xrun-sweep", mode)


def bench_audio_midi_throughput(mode="host"):
    return _bench_audio("midi-throughput", mode)


def main():
    hist = load()
    if "--history" in sys.argv:
        names = sorted({k for h in hist for k in h["results"]})
        for n in names:
            print(n, " -> ".join(f'{h["results"][n]:.2f}' for h in hist if n in h["results"]))
        return
    res = midget() if "--midget" in sys.argv else run()
    graphics_breaches = []
    if "--graphics" in sys.argv:  # window-server frame stats, tools/graphics_bench.py (budgets in docs/graphics-architecture.md)
        import graphics_bench
        click = tuple(sys.argv[sys.argv.index("--click") + 1:sys.argv.index("--click") + 3]) if "--click" in sys.argv else None
        res.update(graphics_bench.midget_results(click))
        graphics_breaches = graphics_bench.gate(res)
    if "--browser" in sys.argv:  # needs a running desktop session with the browser open
        for fn in (bench_browser_startup, bench_browser_idle, bench_browser_memory):
            res.update(fn())
        for k, lim in BROWSER_BUDGETS.items():
            if res.get(k, 0) > lim:
                print(f"over budget: {k} {res[k]:.1f} > {lim}")
    if "--audio" in sys.argv:  # needs bifrost-audio-bench built (host) or installed (Midget with --midget)
        mode = "midget" if "--midget" in sys.argv else "host"
        for fn in (bench_audio_midi_latency, bench_audio_roundtrip, bench_audio_midi_throughput):
            res.update(fn(mode))
        if mode == "midget":
            res.update(bench_audio_xrun_sweep(mode))
    breaches = (budget_check(res) if "--midget" in sys.argv else []) + graphics_breaches
    sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    ver = re.search(r'^version = "([^"]+)"', open(os.path.join(ROOT, "springboard", "Cargo.toml")).read(), re.M).group(1)
    prev = hist[-1]["results"] if hist else {}
    bad = 0
    for k, v in sorted(res.items()):
        p = prev.get(k)
        note = "" if not p else f"  ({(v - p) / p * 100:+.1f}% vs {p:.3f})"
        if p and v > p * 1.15:
            note += "  REGRESSION"
            bad += 1
        print(f"{k:34s} {v:9.3f} {unit(k)}{note}")
    if "--no-record" not in sys.argv:
        with open(HIST, "a") as f:
            f.write(json.dumps({"date": time.strftime("%Y-%m-%d %H:%M"), "sha": sha, "version": ver, "host": os.uname().nodename, "results": res}) + "\n")
    if breaches:
        print("\nover budget (docs/budgets.toml [midget]):")
        for b in breaches:
            print("  " + b)
    sys.exit(3 if breaches else 2 if bad else 0)


main()
