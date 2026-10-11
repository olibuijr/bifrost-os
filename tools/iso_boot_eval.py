#!/usr/bin/env python3
"""Boot-phase eval for the headed ISO VM (tools/arch-vm-iso.sh).

usage: iso_boot_eval.py [--seconds N] [--pick right,ret] [--interval S]
Samples the guest display over QMP every --interval seconds, classifies each frame
(black / picker / text console / ui), snapshots serial-log growth, optionally sends keys at the
first non-black frame (the OpenCanopy picker), and writes out/arch-vm/eval/<stamp>/:
frames/*.png, timeline.tsv, report.txt (first-seen time per phase, longest black gap).
"""
import argparse, json, socket, time
from datetime import datetime
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
V = ROOT / "out/arch-vm"
ap = argparse.ArgumentParser()
ap.add_argument("--seconds", type=int, default=240)
ap.add_argument("--interval", type=float, default=3)
ap.add_argument("--pick", default="right,ret", help="qcodes sent once when the picker first appears")
a = ap.parse_args()

out = V / "eval" / datetime.now().strftime("%Y%m%d-%H%M%S"); (out / "frames").mkdir(parents=True)
s = socket.socket(socket.AF_UNIX); s.connect(str(V / "installer-qmp.sock")); f = s.makefile("rw"); f.readline()
def qmp(cmd, **args):
    f.write(json.dumps({"execute": cmd, "arguments": args} if args else {"execute": cmd}) + "\n"); f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r: return r
qmp("qmp_capabilities")

def classify(im):
    small = im.convert("RGB").resize((160, 90)); blue = sum(1 for r, g_, b in small.getdata() if b > 180 and r < 90)
    if 15 < blue < 400 and not picked: return "picker"   # OpenCanopy's blue icons
    g = im.convert("L").resize((160, 90)); px = list(g.getdata()); n = len(px)
    bright = sum(p > 128 for p in px) / n; lit = sum(p > 24 for p in px) / n
    if lit < 0.002: return "black"
    if lit < 0.12 and bright < 0.05: return "picker"   # sparse icons on black
    if bright < 0.25 and lit < 0.6: return "console"   # text on black
    return "ui"

first = {}
serial = V / "installer-serial.log"; t0 = time.time(); rows = []; picked = False; last = None
while (t := time.time() - t0) < a.seconds:
    ppm = out / "frame.ppm"; qmp("screendump", filename=str(ppm))
    im = Image.open(ppm); ph = classify(im); im.resize((640, 360)).save(out / "frames" / f"{t:06.1f}-{ph}.png")
    sz = serial.stat().st_size if serial.exists() else 0
    rows.append((round(t, 1), ph, sz)); first.setdefault(ph, round(t, 1))
    if ph != last: print(f"{t:6.1f}s  {ph}  serial={sz}B", flush=True); last = ph
    if ph == "picker" and not picked and a.pick:
        picked = True; time.sleep(1.5)
        for k in a.pick.split(","):
            qmp("send-key", keys=[{"type": "qcode", "data": k}]); time.sleep(0.5)
        first["picked_at"] = round(time.time() - t0, 1)
    time.sleep(a.interval)
ppm.unlink(missing_ok=True)
gaps, run = [], None
for t, ph, _ in rows:
    if ph == "black": run = run or t
    elif run is not None: gaps.append((t - run, run)); run = None
if run is not None: gaps.append((rows[-1][0] - run, run))
(out / "timeline.tsv").write_text("t\tphase\tserial_bytes\n" + "".join(f"{t}\t{p}\t{z}\n" for t, p, z in rows))
rep = [f"first seen: {first}", f"final phase: {rows[-1][1]}", f"serial bytes: {rows[-1][2]}",
       "black gaps (s, start): " + ", ".join(f"{d:.0f}@{s_:.0f}" for d, s_ in sorted(gaps, reverse=True)[:5])]
(out / "report.txt").write_text("\n".join(rep) + "\n"); print("\n".join(rep)); print("report:", out)
