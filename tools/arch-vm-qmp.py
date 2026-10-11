#!/usr/bin/env python3
"""Tiny QMP client for tools/arch-vm.sh: screendump, key, tap, swipe over the VM's unix socket.

usage: arch-vm-qmp.py SOCK W H shot OUT.png | key KEY | type TEXT | tap X Y | rtap X Y | swipe X0 Y0 X1 Y1 [MS] | wait-change [MS]
Pointer coordinates are guest pixels (W x H); the virtio-tablet-pci absolute pointer takes 0..32767 absolute axes.
"""
import json
import socket
import sys
import time

KEYS = {"home": "home", "power": "power", "esc": "esc", "enter": "ret", "return": "ret", "back": "esc",
        "up": "up", "down": "down", "left": "left", "right": "right", "space": "spc", "tab": "tab",
        "volup": "volume_up", "voldown": "volume_down"}


SHIFTED = {c: k for c, k in zip('!@#$%^&*()_+{}|:"<>?~', "1234567890-=[]\\;',./`")}
PUNCT = {" ": "spc", "\n": "ret", "\t": "tab", "-": "minus", "=": "equal", "[": "bracket_left", "]": "bracket_right", "\\": "backslash", ";": "semicolon",
         "'": "apostrophe", ",": "comma", ".": "dot", "/": "slash", "`": "grave_accent"}


def qcodes(ch):
    """The qcode chord for one character: (keys, needs_shift)."""
    if ch.isupper():
        return ch.lower(), True
    if ch in SHIFTED:
        base = SHIFTED[ch]
        return PUNCT.get(base, base), True
    return PUNCT.get(ch, ch), False


class Qmp:
    def __init__(self, path):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.settimeout(20)
        self.s.connect(path)
        self.f = self.s.makefile("rw")
        self.f.readline()  # greeting
        self.cmd("qmp_capabilities")

    def cmd(self, name, **args):
        self.f.write(json.dumps({"execute": name, "arguments": args}) + "\n")
        self.f.flush()
        while True:
            r = json.loads(self.f.readline())
            if "return" in r:
                return r["return"]
            if "error" in r:
                sys.exit(f"qmp {name}: {r['error']['desc']}")

    def events(self, evs):
        self.cmd("input-send-event", events=evs)


def axis(ax, v, size):
    v = max(0, min(size - 1, v))
    return {"type": "abs", "data": {"axis": ax, "value": int(v * 32767 / max(size - 1, 1))}}


def btn(down):
    return {"type": "btn", "data": {"button": "left", "down": down}}


def main():
    sock, w, h, op, *a = sys.argv[1:]
    w, h = int(w), int(h)
    q = Qmp(sock)
    if op == "shot":
        q.cmd("screendump", filename=a[0], format="png")
    elif op == "key":
        k = KEYS.get(a[0].lower(), a[0].lower())
        q.cmd("send-key", keys=[{"type": "qcode", "data": k}])
    elif op == "tap":
        x, y = int(a[0]), int(a[1])
        q.events([axis("x", x, w), axis("y", y, h)])
        q.events([btn(True)])
        time.sleep(0.06)
        q.events([btn(False)])
    elif op == "type":
        for ch in " ".join(a):
            k, shift = qcodes(ch)
            q.cmd("send-key", keys=([{"type": "qcode", "data": "shift"}] if shift else []) + [{"type": "qcode", "data": k}])
            time.sleep(0.03)
    elif op == "rtap":
        x, y = int(a[0]), int(a[1])
        q.events([axis("x", x, w), axis("y", y, h)])
        q.events([{"type": "btn", "data": {"button": "right", "down": True}}])
        time.sleep(0.06)
        q.events([{"type": "btn", "data": {"button": "right", "down": False}}])
    elif op == "wait-change":
        # block until the screen differs from now (or the timeout, default 5000 ms); exit 0 when it changed, 3 when not
        import hashlib, os, tempfile
        def grab():
            f = tempfile.mktemp(suffix=".png")
            q.cmd("screendump", filename=f, format="png")
            d = hashlib.md5(open(f, "rb").read()).hexdigest()
            os.unlink(f)
            return d
        first, end = grab(), time.time() + (int(a[0]) if a else 5000) / 1000
        while time.time() < end:
            time.sleep(0.25)
            if grab() != first:
                print("changed")
                return
        print("unchanged")
        sys.exit(3)
    elif op == "swipe":
        x0, y0, x1, y1 = (int(v) for v in a[:4])
        ms = int(a[4]) if len(a) > 4 else 300
        steps = max(4, ms // 16)
        q.events([axis("x", x0, w), axis("y", y0, h)])
        q.events([btn(True)])
        for i in range(1, steps + 1):
            t = i / steps
            q.events([axis("x", x0 + (x1 - x0) * t, w), axis("y", y0 + (y1 - y0) * t, h)])
            time.sleep(ms / 1000 / steps)
        q.events([btn(False)])
    else:
        sys.exit("unknown op " + op)


if __name__ == "__main__":
    main()
