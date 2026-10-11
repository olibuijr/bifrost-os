"""fps: frame-log helpers shared with arch-vm-bench.py.

parse_frames reads the shell's per-frame log lines; summarize reports the presented frame rate while animating
(1000 / mean gap between presented frames) and draw time.
"""
import os
import re
import subprocess

HERE = os.path.dirname(os.path.realpath(__file__))
FRAME = re.compile(r"frame (?:draw|gpu) ([\d.]+) present ([\d.]+) damage (\d+)% gap ([\d.]+|inf)")


# page left, page right, again with a longer swipe: every one animates the whole screen
WORKLOAD = [(1000, 900, 200, 900, 300), (200, 900, 1000, 900, 300), (1000, 900, 200, 900, 500), (200, 900, 1000, 900, 500)] * 3


def vm_ssh(cmd):
    return subprocess.run([os.path.join(HERE, "arch-vm.sh"), "ssh", cmd], capture_output=True, text=True).stdout


def parse_frames(text):
    """(draw ms, present ms, gap ms) of every frame that changed the screen."""
    out = []
    for m in FRAME.finditer(text):
        draw, present, damage, gap = float(m[1]), float(m[2]), int(m[3]), m[4]
        if damage > 0 and gap != "inf":
            out.append((draw, present, float(gap)))
    return out


def percentile(v, p):
    s = sorted(v)
    return s[min(len(s) - 1, int(len(s) * p))] if s else 0.0


def summarize(frames):
    """fps while animating: gaps over 200 ms are pauses between gestures, not frames."""
    live = [f for f in frames if f[2] < 200]
    if not live:
        return None
    gap = sum(f[2] for f in live) / len(live)
    draws = [f[0] for f in live]
    return {"frames": len(live), "fps": 1000.0 / gap if gap else 0.0, "draw_avg": sum(draws) / len(draws),
            "draw_p95": percentile(draws, 0.95), "draw_max": max(draws), "over_16_7": sum(1 for f in live if f[0] + f[1] > 16.7)}
