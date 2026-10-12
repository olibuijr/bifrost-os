#!/usr/bin/env python3
"""Boot benchmark on Midget: reboot the installed system N times and time every stage of each boot.

Stages (docs/boot-architecture.md, "Budgets"; the clock is CLOCK_BOOTTIME, so kernel start is 0 ms):
  loader      the BifrOSt loader's run time, from the Boot Loader Interface EFI variables LoaderTimeInitUSec and
              LoaderTimeExecUSec (TSC microseconds since reset). Loader start relative to kernel start is derived
              from them: it is the negative of this loader time, because the loader starts the kernel.
  firmware, kernel, initrd, userspace   systemd-analyze's own totals for the whole boot (reported, not budgeted).
  marks       /run/bifrost/boot-marks, ms since kernel start: bootweave-start, display-handoff-start,
              compositor-ready, desktop-first-frame, desktop-ready, desktop-first-ui, desktop-faded.
  shutdown    shutdown-start (journal of the previous boot, `boot-mark` lines) to the previous boot's last journal
              entry, which is the power-off. A run's shutdown is read in the next run, so N runs give N-1 samples.
  modesets    display mode programming lines in this boot's journal (tools/boot-frames.py counts them the same way).

Budgets are checked against the medians. The shutdown and modeset budgets are the contract's; the stage budgets are
the contract's too: loader under 0.3 s, kernel to bootweave under 1.5 s, bootweave to desktop-ready under 2.0 s.

The reboot goes through tools/midget.py (`reboot`, which waits for ssh and runs the update guard). Nothing here
flashes, wipes or installs; build and publish first (`bifrost pc publish`, `bifrost pc midget ota`).

usage: tools/boot-bench.py installed [--runs N] [--label TEXT] [--record] [--no-reboot]
       tools/boot-bench.py selftest
"""
import argparse, importlib.util, json, os, statistics, subprocess, sys, time, unittest

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOADER_VARS = "/sys/firmware/efi/efivars/{}-4a67b082-0a4c-41cf-b6c7-440b29bb8c4f"
MARK_KEYS = ["bootweave-start", "display-handoff-start", "compositor-ready", "desktop-first-frame", "desktop-ready",
             "desktop-first-ui", "desktop-faded"]
# Contract budgets in ms (docs/boot-architecture.md, "Budgets").
BUDGET_MS = {
    "loader_ms": 300,
    "kernel_to_bootweave_ms": 1500,
    "bootweave_to_desktop_ready_ms": 2000,
    "shutdown_ms": 2500,
}
WAIT_FADED_S = 120


def seconds(text):
    """'1min 2.5s', '250ms', '3.2s' -> seconds."""
    total = 0.0
    for tok in text.split():
        for unit, k in (("ms", 0.001), ("min", 60.0), ("s", 1.0), ("us", 1e-6)):
            if tok.endswith(unit) and tok[: -len(unit)].replace(".", "", 1).isdigit():
                total += float(tok[: -len(unit)]) * k
                break
    return round(total, 3)


def parse_analyze(line):
    """systemd-analyze's one-line summary: 'Startup finished in 1.2s (firmware) + 0.4s (loader) + ... = 6.7s'."""
    out = {}
    for part in line.replace("Startup finished in", "").split("+"):
        part = part.split("=")[0].strip()
        if "(" not in part:
            continue
        val, name = part.split("(", 1)
        out[name.rstrip(") ").strip()] = seconds(val.strip())
    return out


def loader_ms(init_us, exec_us):
    """The loader's own run time from the two Boot Loader Interface timestamps (microseconds)."""
    return round((exec_us - init_us) / 1000.0, 1)


def parse_marks(text):
    """`/run/bifrost/boot-marks` lines 'ms event'; the first mark of an event wins, as the writer does."""
    out = {}
    for line in text.splitlines():
        p = line.split()
        if len(p) == 2:
            try:
                out.setdefault(p[1], float(p[0]))
            except ValueError:
                pass
    return out


def stage_times(marks, loader):
    """Stage durations in ms from the marks and the loader time. Missing marks give None, never a guess."""
    bw = marks.get("bootweave-start")
    dr = marks.get("desktop-ready")
    return {
        "loader_ms": loader,
        "kernel_to_bootweave_ms": bw,
        "bootweave_to_desktop_ready_ms": round(dr - bw, 1) if bw is not None and dr is not None else None,
        "desktop_faded_ms": marks.get("desktop-faded"),
    }


def shutdown_ms(shutdown_start_unix_ms, last_journal_unix_ms):
    """Desktop fade to power-off: the previous boot's last journal entry is the closest the journal gets to power-off."""
    if shutdown_start_unix_ms is None or last_journal_unix_ms is None:
        return None
    return round(last_journal_unix_ms - shutdown_start_unix_ms, 1)


def parse_boot_mark(line):
    """'boot-mark EVENT UNIX_MS BOOTTIME_MS' from the journal -> (event, unix_ms) or None."""
    p = line.split()
    if len(p) >= 4 and p[0] == "boot-mark":
        try:
            return p[1], float(p[2])
        except ValueError:
            return None
    return None


def journal_unix_ms(json_line):
    """`journalctl -o json` line -> __REALTIME_TIMESTAMP in ms (microseconds in the journal)."""
    try:
        return int(json.loads(json_line)["__REALTIME_TIMESTAMP"]) / 1000.0
    except (ValueError, KeyError, TypeError):
        return None


def medians(runs, prefix="boot_installed"):
    """Median of every numeric field over the runs that have it, keyed `prefix_<field>`."""
    keys = {}
    for r in runs:
        for k, v in r["flat"].items():
            if isinstance(v, (int, float)):
                keys.setdefault(k, []).append(v)
    return {f"{prefix}_{k}": round(statistics.median(vs), 3) for k, vs in sorted(keys.items())}


def budget_rows(meds):
    """(name, median, budget, PASS|FAIL|NO DATA) for each contract budget that has a measurement."""
    rows = []
    for key, budget in BUDGET_MS.items():
        val = meds.get(f"boot_installed_{key}")
        status = "NO DATA" if val is None else ("PASS" if val < budget else "FAIL")
        rows.append((key, val, budget, status))
    return rows


def load_midget():
    spec = importlib.util.spec_from_file_location("midget", os.path.join(REPO, "tools/midget.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def collect(midget):
    """Reads everything one boot leaves behind, over ssh. Returns the run record (no derived keys)."""
    analyze = parse_analyze(midget.ssh("systemd-analyze 2>/dev/null | head -1", 20).strip())
    # efivarfs file: 4 attribute bytes, then the value as a little-endian u64 (od prints it in decimal)
    init = midget.ssh(f"od -An -tu8 -j4 -N8 {LOADER_VARS.format('LoaderTimeInitUSec')} 2>/dev/null", 10).strip()
    exe = midget.ssh(f"od -An -tu8 -j4 -N8 {LOADER_VARS.format('LoaderTimeExecUSec')} 2>/dev/null", 10).strip()
    marks = parse_marks(midget.ssh("cat /run/bifrost/boot-marks 2>/dev/null", 10))
    modesets = int(midget.ssh("journalctl -b -o cat --no-pager 2>/dev/null | grep -acE "
                              "'Modesetting with|display mode .*\\(selected\\)'", 60).strip() or 0)
    prev = midget.ssh("journalctl -b -1 -o cat --no-pager 2>/dev/null | grep '^boot-mark shutdown-start' | tail -1", 60)
    prev_start = parse_boot_mark(prev.strip())
    last = midget.ssh("journalctl -b -1 -o json -n 1 --no-pager 2>/dev/null", 60).strip()
    loader = loader_ms(int(init), int(exe)) if init.isdigit() and exe.isdigit() else None
    return {"analyze_s": analyze, "marks_ms": marks, "loader_ms": loader, "modesets": modesets,
            "prev_shutdown_start": prev_start, "prev_last_journal_ms": journal_unix_ms(last) if last else None}


def flatten(rec):
    """The run's numeric results, keyed as the perf history stores them."""
    flat = {f"stage_{k}_s": v for k, v in rec["analyze_s"].items()}
    flat.update({f"mark_{k.replace('-', '_')}_ms": v for k, v in rec["marks_ms"].items() if k in MARK_KEYS})
    st = stage_times(rec["marks_ms"], rec["loader_ms"])
    flat.update({k: v for k, v in st.items() if v is not None})
    flat["modesets"] = rec["modesets"]
    return flat


def wait_for_desktop(midget):
    end = time.time() + WAIT_FADED_S
    while time.time() < end:
        marks = parse_marks(midget.ssh("cat /run/bifrost/boot-marks 2>/dev/null", 10))
        if "desktop-faded" in marks or "desktop-ready" in marks:
            return True
        time.sleep(1)
    return False


def one_run(midget, reboot):
    t0 = time.time()
    if reboot:
        if midget.cmd_reboot([]):
            return {"error": "reboot failed"}
    if midget.state() != "installed":
        return {"error": f"not installed after reboot (state {midget.state()})"}
    wait_for_desktop(midget)
    time.sleep(1)
    rec = collect(midget)
    rec["wall_s"] = round(time.time() - t0, 2)
    rec["flat"] = flatten(rec)
    return rec


def record(meds, label, runs_ok):
    sha = subprocess.run(["git", "-C", REPO, "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    ver = next((l.split('"')[1] for l in open(os.path.join(REPO, "springboard/Cargo.toml")) if l.startswith("version")), "")
    entry = {"date": time.strftime("%Y-%m-%d %H:%M"), "sha": sha, "version": ver, "host": "midget", "label": label,
             "boots": runs_ok, "results": meds}
    with open(os.path.join(REPO, "docs/perf-history.jsonl"), "a") as f:
        f.write(json.dumps(entry) + "\n")


def main_installed(a):
    midget = load_midget()
    if not a.no_reboot and midget.state() != "installed":
        print("midget is not running the installed system; boot it first", file=sys.stderr)
        return 1
    runs = []
    prev_ok = False  # the previous run's boot is the one whose shutdown this boot's journal holds
    for i in range(a.runs):
        rec = one_run(midget, reboot=not a.no_reboot)
        if "error" in rec:
            print(f"run {i + 1}: {rec['error']}")
            prev_ok = False
            continue
        # the previous boot's shutdown is in this boot's journal: attach it to the previous run, if that run was good
        if prev_ok and rec["prev_shutdown_start"] is not None:
            runs[-1]["flat"]["shutdown_ms"] = shutdown_ms(rec["prev_shutdown_start"][1], rec["prev_last_journal_ms"])
        runs.append(rec)
        prev_ok = True
        print(f"run {i + 1}: desktop-faded at {rec['marks_ms'].get('desktop-faded')} ms, loader {rec['loader_ms']} ms")
    if not runs:
        return 1
    meds = medians(runs)
    print(f"installed boot, {len(runs)}/{a.runs} runs, medians:")
    for k, v in meds.items():
        print(f"  {k:52} {v}")
    print("budgets (docs/boot-architecture.md):")
    for name, val, budget, status in budget_rows(meds):
        print(f"  {name:32} {str(val):>10} ms  budget < {budget:<6} {status}")
    if a.record:
        record(meds, a.label, len(runs))
    return 0


class Tests(unittest.TestCase):
    def test_seconds_reads_systemd_units(self):
        self.assertEqual(seconds("1min 2.5s"), 62.5)
        self.assertEqual(seconds("250ms"), 0.25)
        self.assertEqual(seconds("3.2s"), 3.2)

    def test_parse_analyze_names_each_stage_and_skips_the_total(self):
        line = "Startup finished in 1.200s (firmware) + 400ms (loader) + 3.100s (kernel) + 800ms (initrd) + 5.500s (userspace) = 11.000s"
        self.assertEqual(parse_analyze(line), {"firmware": 1.2, "loader": 0.4, "kernel": 3.1, "initrd": 0.8, "userspace": 5.5})

    def test_parse_analyze_without_firmware(self):
        line = "Startup finished in 2.000s (kernel) + 1.000s (userspace) = 3.000s"
        self.assertEqual(parse_analyze(line), {"kernel": 2.0, "userspace": 1.0})

    def test_loader_ms_is_the_difference_of_the_two_timestamps(self):
        self.assertEqual(loader_ms(1_000_000, 1_250_500), 250.5)
        self.assertEqual(loader_ms(5_000, 5_000), 0.0)

    def test_parse_marks_keeps_the_first_mark_of_each_event(self):
        text = "1200.5 bootweave-start\n1500.0 desktop-ready\n1600.0 desktop-ready\nnot a mark line here\n"
        marks = parse_marks(text)
        self.assertEqual(marks, {"bootweave-start": 1200.5, "desktop-ready": 1500.0})

    def test_stage_times_from_marks_and_loader(self):
        marks = {"bootweave-start": 1800.0, "desktop-ready": 3900.0, "desktop-faded": 4400.0}
        st = stage_times(marks, 250.0)
        self.assertEqual(st, {"loader_ms": 250.0, "kernel_to_bootweave_ms": 1800.0,
                              "bootweave_to_desktop_ready_ms": 2100.0, "desktop_faded_ms": 4400.0})

    def test_stage_times_leave_missing_marks_empty(self):
        st = stage_times({"bootweave-start": 1800.0}, None)
        self.assertIsNone(st["bootweave_to_desktop_ready_ms"])
        self.assertIsNone(st["loader_ms"])

    def test_shutdown_is_the_gap_to_the_last_journal_entry(self):
        self.assertEqual(shutdown_ms(1_000_000.0, 1_001_800.5), 1800.5)
        self.assertIsNone(shutdown_ms(None, 5.0))

    def test_parse_boot_mark_and_journal_timestamp(self):
        self.assertEqual(parse_boot_mark("boot-mark shutdown-start 1700000000123 4567.8"), ("shutdown-start", 1700000000123.0))
        self.assertIsNone(parse_boot_mark("some other journal line"))
        self.assertEqual(journal_unix_ms('{"__REALTIME_TIMESTAMP": "1700000000456000"}'), 1700000000456.0)
        self.assertIsNone(journal_unix_ms("garbage"))

    def test_medians_over_runs_and_budgets(self):
        runs = [{"flat": {"loader_ms": 200.0, "kernel_to_bootweave_ms": 1000.0}},
                {"flat": {"loader_ms": 300.0, "kernel_to_bootweave_ms": 1400.0}},
                {"flat": {"loader_ms": 400.0, "kernel_to_bootweave_ms": 1200.0, "shutdown_ms": 2400.0}}]
        meds = medians(runs)
        self.assertEqual(meds["boot_installed_loader_ms"], 300.0)
        self.assertEqual(meds["boot_installed_kernel_to_bootweave_ms"], 1200.0)
        rows = {name: status for name, _, _, status in budget_rows(meds)}
        self.assertEqual(rows["loader_ms"], "FAIL")  # 300 is not under 300
        self.assertEqual(rows["kernel_to_bootweave_ms"], "PASS")
        self.assertEqual(rows["shutdown_ms"], "PASS")
        self.assertEqual(rows["bootweave_to_desktop_ready_ms"], "NO DATA")

    def test_flatten_keys_match_the_perf_history_names(self):
        rec = {"analyze_s": {"kernel": 3.1}, "marks_ms": {"bootweave-start": 1800.0, "desktop-ready": 3900.0},
               "loader_ms": 250.0, "modesets": 0}
        flat = flatten(rec)
        self.assertEqual(flat["stage_kernel_s"], 3.1)
        self.assertEqual(flat["mark_bootweave_start_ms"], 1800.0)
        self.assertEqual(flat["bootweave_to_desktop_ready_ms"], 2100.0)
        self.assertEqual(flat["modesets"], 0)


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="mode", required=True)
    inst = sub.add_parser("installed", help="reboot Midget into the installed system and time it")
    inst.add_argument("--runs", type=int, default=5)
    inst.add_argument("--label", default="")
    inst.add_argument("--record", action="store_true", help="append the medians to docs/perf-history.jsonl")
    inst.add_argument("--no-reboot", action="store_true", help="only collect the boot Midget is in now (one run)")
    sub.add_parser("selftest", help="run the unit tests")
    a = ap.parse_args()
    if a.mode == "selftest":
        unittest.main(module=sys.modules[__name__], argv=[sys.argv[0]], exit=False, verbosity=1)
        return 0
    if a.no_reboot:
        a.runs = 1
    return main_installed(a)


if __name__ == "__main__":
    sys.exit(main())
