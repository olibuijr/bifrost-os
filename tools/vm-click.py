#!/usr/bin/env python3
"""Click in the headed ISO VM over QMP: vm-click.py X Y  (guest pixels, default 1280x720)."""
import json, os, socket, sys, time
W, H = 1280, 720
x, y = float(sys.argv[1]), float(sys.argv[2])
s = socket.socket(socket.AF_UNIX); s.connect(os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "out/arch-vm/installer-qmp.sock")); f = s.makefile("rw"); f.readline()
def q(cmd, **a):
    f.write(json.dumps({"execute": cmd, "arguments": a}) + "\n"); f.flush(); return f.readline()
q("qmp_capabilities")
ev = lambda e: q("input-send-event", events=e)
ev([{"type": "abs", "data": {"axis": "x", "value": int(x / W * 32767)}}, {"type": "abs", "data": {"axis": "y", "value": int(y / H * 32767)}}])
time.sleep(0.15)
ev([{"type": "btn", "data": {"down": True, "button": "left"}}]); time.sleep(0.08); ev([{"type": "btn", "data": {"down": False, "button": "left"}}])
