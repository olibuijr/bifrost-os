#!/usr/bin/env python3
"""Boot frame continuity on Midget: what the screen showed at each handover, and whether the console printed text.

Checks (docs/boot-architecture.md, "Budgets" and "Testing"):
  modesets     display mode programming in this boot's journal (KMS or wlroots "Modesetting" lines): budget 0.
               The loader's one modeset happens before the kernel and is not counted.
  console      kernel messages that the console prints (priority crit or worse with `loglevel=3`) and the
               cmdline options that keep text off tty1 (`quiet`, `loglevel<=3`, `vt.global_cursor_default=0`).
  frames       each handover frame read back from the scanout, compared pixel for pixel with the boot frame that
               `bootweave-frame` renders at the same size (the golden file). Budget: 0 pixels differ.

Readback contract (written by the boot hooks of tasks 06 and 09 on Midget, read here over ssh):
  /run/bifrost/boot-frames/<stage>.json   {"width": W, "height": H, "stride": S (pixels per row), "order": "bgrx"|"rgbx"}
  /run/bifrost/boot-frames/<stage>.raw    S * H little-endian 32-bit pixels, row by row
Stages: `inherited` (Bootweave, before its first draw: the framebuffer the kernel handed over), `bootweave` (its first
drawn frame), `desktop-first` (the window server's first scanout while the boot frame still holds; not golden-compared).
A stage with no readback files is reported NOT MEASURED; `--strict` makes that a failure.

Golden: `--golden DIR` holds `boot.raw` and `boot.json` (the same format), rendered by the bootweave-frame golden test
at the panel's native size.

usage: tools/boot-frames.py check [--stages inherited,bootweave] [--golden DIR] [--strict] [--json]
       tools/boot-frames.py selftest
"""
import argparse, importlib.util, json, os, re, subprocess, sys, unittest

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FRAME_DIR = "/run/bifrost/boot-frames"
DEFAULT_STAGES = ["inherited", "bootweave"]
MODESET_RE = re.compile(r"Modesetting with|display mode .*\(selected\)")
# systemd prints the console's loglevel as printk levels: with loglevel=3 only emerg, alert and crit reach the console.
CONSOLE_PRIORITY_MAX = 2


def count_modesets(lines):
    """Journal lines that report a display mode being programmed (the kernel's and the compositor's wording)."""
    return sum(1 for line in lines if MODESET_RE.search(line))


def console_problems(cmdline, console_lines):
    """Reasons text can reach tty1 on this boot; an empty list means the boot is text-free. `console_lines` are the
    kernel messages at priority crit or worse, the ones the console prints under `loglevel=3`."""
    opts = dict(t.split("=", 1) if "=" in t else (t, None) for t in cmdline.split())
    problems = []
    if "quiet" not in opts:
        problems.append("cmdline has no quiet")
    level = opts.get("loglevel")
    if level is None or not level.isdigit() or int(level) > 3:
        problems.append(f"loglevel is {level or 'unset'}, not 3 or lower")
    if opts.get("vt.global_cursor_default") != "0":
        problems.append("vt.global_cursor_default is not 0 (the cursor shows on tty1)")
    if console_lines:
        problems.append(f"{len(console_lines)} kernel message(s) at crit or worse reach the console")
    return problems


def compare_frames(expected, actual):
    """(mismatched pixels, first mismatch index or None) for two raw 32-bit pixel buffers of the same length."""
    if len(expected) != len(actual):
        raise ValueError(f"frame sizes differ: {len(expected)} and {len(actual)} bytes")
    if len(expected) % 4:
        raise ValueError("pixel buffer is not a whole number of 32-bit pixels")
    bad = 0
    first = None
    for i in range(0, len(expected), 4):
        if expected[i:i + 4] != actual[i:i + 4]:
            bad += 1
            if first is None:
                first = i // 4
    return bad, first


def check_meta(meta):
    """Validates a readback's JSON description; returns (width, height, stride, order) or raises ValueError."""
    try:
        w, h, s = int(meta["width"]), int(meta["height"]), int(meta["stride"])
    except (KeyError, TypeError, ValueError) as error:
        raise ValueError(f"bad frame description: {error}")
    order = meta.get("order")
    if order not in ("bgrx", "rgbx"):
        raise ValueError(f"unknown pixel order {order!r}")
    if w <= 0 or h <= 0 or s < w or s > 16384:
        raise ValueError(f"implausible frame {w}x{h} stride {s}")
    return w, h, s, order


def to_order(raw, order, to="bgrx"):
    """Swaps red and blue when the readback's order differs from the golden's."""
    if order == to:
        return raw
    out = bytearray(raw)
    out[0::4], out[2::4] = raw[2::4], raw[0::4]
    return bytes(out)


def verdict(stage, measured, bad=None, error=None):
    """One result row: (stage, status, detail). status is PASS, FAIL or NOT MEASURED."""
    if not measured:
        return (stage, "NOT MEASURED", error or "no readback on Midget")
    if error:
        return (stage, "FAIL", error)
    return (stage, "PASS" if bad == 0 else "FAIL", f"{bad} pixel(s) differ from the boot frame")


def load_midget():
    spec = importlib.util.spec_from_file_location("midget", os.path.join(REPO, "tools/midget.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def ssh_bytes(midget, cmd, timeout=60):
    return subprocess.run(midget.SSH + [cmd], capture_output=True, timeout=timeout)


def fetch_stage(midget, stage):
    """(meta_text, raw_bytes) for a stage readback, or (None, None) when the hook has not written it."""
    meta = ssh_bytes(midget, f"cat {FRAME_DIR}/{stage}.json 2>/dev/null", 20)
    if meta.returncode != 0 or not meta.stdout:
        return None, None
    raw = ssh_bytes(midget, f"cat {FRAME_DIR}/{stage}.raw 2>/dev/null")
    return meta.stdout.decode(errors="replace"), raw.stdout


def read_golden(golden):
    with open(os.path.join(golden, "boot.json")) as f:
        meta = json.load(f)
    with open(os.path.join(golden, "boot.raw"), "rb") as f:
        return check_meta(meta), f.read()


def run_checks(midget, stages, golden_dir, strict):
    rows = []
    lines = midget.ssh("journalctl -b -o cat --no-pager 2>/dev/null", 60).splitlines()
    modesets = count_modesets(lines)
    rows.append(("modesets", "PASS" if modesets == 0 else "FAIL", f"{modesets} display mode programming line(s) after the loader"))
    cmdline = midget.ssh("cat /proc/cmdline", 10).strip()
    crit = midget.ssh("journalctl -b -k -p crit -o cat --no-pager 2>/dev/null", 30).splitlines()
    problems = console_problems(cmdline, [l for l in crit if l.strip()])
    rows.append(("console", "PASS" if not problems else "FAIL", "; ".join(problems) or "no text reaches tty1"))
    golden = None
    if golden_dir:
        try:
            golden = read_golden(golden_dir)
        except (OSError, ValueError, KeyError) as error:
            rows.append(("golden", "FAIL", f"cannot read {golden_dir}: {error}"))
    for stage in stages:
        meta_text, raw = fetch_stage(midget, stage)
        if meta_text is None:
            rows.append(verdict(stage, False))
            continue
        try:
            w, h, s, order = check_meta(json.loads(meta_text))
            if golden is None:
                rows.append((stage, "NOT MEASURED", f"{w}x{h} read back, no golden to compare (use --golden)"))
                continue
            (gw, gh, gs, gorder), graw = golden
            if (w, h, s) != (gw, gh, gs):
                rows.append(verdict(stage, True, error=f"{w}x{h} stride {s} but the golden is {gw}x{gh} stride {gs}"))
                continue
            bad, first = compare_frames(to_order(graw, gorder), to_order(raw, order))
            rows.append(verdict(stage, True, bad=bad))
            if bad and first is not None:
                rows[-1] = (stage, "FAIL", f"{bad} pixel(s) differ, first at x={first % s} y={first // s}")
        except (ValueError, json.JSONDecodeError) as error:
            rows.append(verdict(stage, True, error=str(error)))
    if strict:
        rows = [(a, "FAIL" if b == "NOT MEASURED" else b, c) for a, b, c in rows]
    return rows


def print_rows(rows):
    for name, status, detail in rows:
        print(f"  {name:14} {status:13} {detail}")
    return 0 if all(status in ("PASS", "NOT MEASURED") for _, status, _ in rows) else 1


class Tests(unittest.TestCase):
    def test_modesets_count_both_wordings_and_ignore_other_lines(self):
        lines = [
            "kms: display mode 2560x1440@60 (selected)",
            "wlroots: Modesetting with 'i915' on /dev/dri/card0",
            "i915 0000:00:02.0: [drm] fastboot inherited the mode",
            "display mode 1920x1080@60 (preferred)",
        ]
        self.assertEqual(count_modesets(lines), 2)
        self.assertEqual(count_modesets([]), 0)

    def test_console_problems_empty_for_a_quiet_text_free_boot(self):
        cmd = "root=UUID=x rw quiet loglevel=3 vt.global_cursor_default=0 splash"
        self.assertEqual(console_problems(cmd, []), [])

    def test_console_problems_name_each_missing_option(self):
        p = console_problems("root=UUID=x rw loglevel=7", [])
        self.assertEqual(len(p), 3)
        self.assertIn("no quiet", p[0])
        self.assertIn("loglevel is 7", p[1])
        self.assertIn("vt.global_cursor_default", p[2])

    def test_console_problems_counts_crit_messages_that_reach_the_console(self):
        cmd = "quiet loglevel=3 vt.global_cursor_default=0"
        p = console_problems(cmd, ["panic-like line", "second"])
        self.assertEqual(p, ["2 kernel message(s) at crit or worse reach the console"])

    def test_compare_frames_counts_pixels_and_reports_the_first(self):
        a = bytes([1, 2, 3, 0] * 4)
        b = bytearray(a)
        b[4 * 2] = 9  # pixel 2 differs in its first byte
        b[4 * 3 + 1] = 9  # pixel 3 differs too
        self.assertEqual(compare_frames(a, bytes(b)), (2, 2))
        self.assertEqual(compare_frames(a, a), (0, None))

    def test_compare_frames_refuses_different_sizes(self):
        with self.assertRaises(ValueError):
            compare_frames(b"\0" * 8, b"\0" * 4)
        with self.assertRaises(ValueError):
            compare_frames(b"\0" * 6, b"\0" * 6)

    def test_check_meta_validates_the_description(self):
        self.assertEqual(check_meta({"width": 1920, "height": 1080, "stride": 1920, "order": "bgrx"}), (1920, 1080, 1920, "bgrx"))
        with self.assertRaises(ValueError):
            check_meta({"width": 1920, "height": 1080, "stride": 1000, "order": "bgrx"})
        with self.assertRaises(ValueError):
            check_meta({"width": 4, "height": 4, "stride": 4, "order": "xxxx"})
        with self.assertRaises(ValueError):
            check_meta({"width": "wide", "height": 4, "stride": 4, "order": "bgrx"})

    def test_to_order_swaps_red_and_blue_only_when_needed(self):
        px = bytes([10, 20, 30, 0])  # B G R X
        self.assertEqual(to_order(px, "bgrx"), px)
        self.assertEqual(to_order(px, "rgbx"), bytes([30, 20, 10, 0]))

    def test_verdict_rows(self):
        self.assertEqual(verdict("inherited", False)[1], "NOT MEASURED")
        self.assertEqual(verdict("bootweave", True, bad=0)[1], "PASS")
        self.assertEqual(verdict("bootweave", True, bad=3)[1], "FAIL")
        self.assertEqual(verdict("bootweave", True, error="size")[1], "FAIL")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check", help="check the last boot on Midget")
    c.add_argument("--stages", default=",".join(DEFAULT_STAGES))
    c.add_argument("--golden", help="directory with boot.raw and boot.json (the bootweave-frame render)")
    c.add_argument("--strict", action="store_true", help="a stage that was not read back is a failure")
    sub.add_parser("selftest", help="run the unit tests")
    a = ap.parse_args()
    if a.cmd == "selftest":
        unittest.main(module=sys.modules[__name__], argv=[sys.argv[0]], exit=False, verbosity=1)
        return 0
    midget = load_midget()
    if midget.state() != "installed":
        print("midget is not running the installed system; boot it first", file=sys.stderr)
        return 1
    stages = [s for s in a.stages.split(",") if s]
    rows = run_checks(midget, stages, a.golden, a.strict)
    print("boot frame checks on midget:")
    return print_rows(rows)


if __name__ == "__main__":
    sys.exit(main())
