#!/usr/bin/env python3
"""Collect journals streamed by debug ISO builds (bifrost-debug-log) into out/lab/logs/. usage: lab-log-sink.py [bind-ip]"""
import socket, sys, threading, time
from pathlib import Path

ip = sys.argv[1] if len(sys.argv) > 1 else "10.99.0.1"
out = Path(__file__).resolve().parents[1] / "out/lab/logs"
out.mkdir(parents=True, exist_ok=True)
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind((ip, 9100)); s.listen(8)
def serve(c, a):
    f = out / f"{a[0]}-{time.strftime('%H%M%S')}.log"
    with c, open(f, "ab", buffering=0) as fh:
        while d := c.recv(65536): fh.write(d)
print(f"lab log sink on {ip}:9100 -> {out}", flush=True)
while True:
    c, a = s.accept(); threading.Thread(target=serve, args=(c, a), daemon=True).start()
