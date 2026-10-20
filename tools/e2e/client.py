"""Control-protocol client, bounded waits, RIMG/PNG helpers and pixel diff for the e2e harness (stdlib only)."""
import os
import shlex
import socket
import struct
import subprocess
import time
import zlib

HERE = os.path.dirname(os.path.realpath(__file__))
TOOLS = os.path.dirname(HERE)
DIFF_DIR = os.path.expanduser("~/Pictures/tab/e2e")
BASELINES = os.path.join(HERE, "baselines")
W, H = 1200, 1920
# The shell serves a Unix socket (root and the installed account only); there is no TCP control port.
CONTROL_SOCKET = "/run/bifrost/shell-control.sock"


class E2EError(Exception):
    pass


class Target:
    """One SpringBoard control socket: the Arch VM through ssh, with python on the guest speaking AF_UNIX (root)."""

    def __init__(self, kind="vm", socket_path=CONTROL_SOCKET):
        self.kind, self.socket_path = kind, socket_path

    def raw(self, line, timeout=10):
        if self.kind == "vm":
            script = (f"import socket,sys; s=socket.socket(socket.AF_UNIX); s.settimeout({int(timeout)}); s.connect({self.socket_path!r}); "
                      "s.sendall((sys.argv[1] + chr(10)).encode()); d=b''\n"
                      "while True:\n try:\n  c=s.recv(65536)\n except socket.timeout:\n  break\n if not c: break\n d+=c\nsys.stdout.buffer.write(d)")
            r = subprocess.run([os.path.join(TOOLS, "arch-vm.sh"), "ssh", f"python3 -c {shlex.quote(script)} {shlex.quote(line)}"],
                               capture_output=True, timeout=timeout + 20)
            return r.stdout
        with socket.socket(socket.AF_UNIX) as s:
            s.settimeout(timeout)
            s.connect(self.socket_path)
            s.sendall(line.encode() + b"\n")
            chunks = []
            while True:
                try:
                    b = s.recv(1 << 20)
                except socket.timeout:
                    break
                if not b:
                    break
                chunks.append(b)
        return b"".join(chunks)

    def cmd(self, line, timeout=10, retries=3):
        """Send one request, retrying on connection errors or an empty answer."""
        last = None
        for i in range(retries):
            try:
                data = self.raw(line, timeout)
                if data:
                    return data
                last = "empty answer"
            except (OSError, subprocess.SubprocessError) as e:
                last = str(e)
            time.sleep(0.3 * (i + 1))
        raise E2EError(f"control '{line}' failed: {last}")

    def text(self, line, **kw):
        return self.cmd(line, **kw).decode(errors="replace").strip()

    # input
    def tap(self, x, y):
        return self.text(f"tap {x} {y}")

    def swipe(self, x0, y0, x1, y1, ms=300):
        return self.text(f"swipe {x0} {y0} {x1} {y1} {ms}")

    def down(self, x, y):
        return self.text(f"down {x} {y}")

    def move(self, x, y):
        return self.text(f"move {x} {y}")

    def up(self, x, y):
        return self.text(f"up {x} {y}")

    def hold(self, x, y, ms=800):
        return self.text(f"hold {x} {y} {ms}")

    def doubletap(self, x, y):
        return self.text(f"doubletap {x} {y}")

    def key(self, name):
        return self.text(f"key {name}")

    def trace(self, on):
        return self.text(f"trace {'on' if on else 'off'}")

    def drag(self, x0, y0, x1, y1, steps=12):
        """Finger drag with explicit down/move/up, so icons can be picked up."""
        self.down(x0, y0)
        for i in range(1, steps + 1):
            self.move(x0 + (x1 - x0) * i // steps, y0 + (y1 - y0) * i // steps)
        self.up(x1, y1)

    # state
    def state(self):
        out = {}
        for ln in self.text("state").splitlines():
            if "=" in ln:
                k, v = ln.split("=", 1)
                out[k.strip()] = v.strip()
        return out

    def wait_state(self, pred, timeout=6.0, what="state", settle=True):
        """Poll `state` until pred(dict) is true (bounded). pred may also be a 'key=value' string. Returns the state."""
        if isinstance(pred, str):
            k, v = pred.split("=", 1)
            pred = lambda s, k=k, v=v: s.get(k) == v
        end, st = time.time() + timeout, {}
        while True:
            try:
                st = self.state()
                if pred(st) and (not settle or st.get("animating") != "true"):
                    return st
            except E2EError:
                pass
            if time.time() > end:
                raise E2EError(f"timeout waiting for {what}; last state: screen={st.get('screen')} cc={st.get('cc')} "
                               f"quicksearch={st.get('quicksearch')} animating={st.get('animating')}")
            time.sleep(0.15)

    def settle(self, timeout=4.0):
        return self.wait_state(lambda s: s.get("animating") != "true", timeout, "animation end", settle=False)

    # pixels
    def shot(self, scale=4):
        blob = self.cmd(f"shot {scale}", timeout=30)
        if blob[:4] != b"RIMG":
            raise E2EError("shot failed: " + blob[:200].decode(errors="replace"))
        w, h = struct.unpack("<II", blob[4:12])
        return Image(w, h, blob[12:12 + w * h * 4])


class Image:
    """RGBA8 image."""

    def __init__(self, w, h, rgba):
        if len(rgba) != w * h * 4:
            raise E2EError(f"bad image data: {len(rgba)} bytes for {w}x{h}")
        self.w, self.h, self.rgba = w, h, bytes(rgba)

    def pixel(self, x, y):
        i = (y * self.w + x) * 4
        return tuple(self.rgba[i:i + 4])

    def to_png(self):
        raw = b"".join(b"\0" + self.rgba[y * self.w * 4:(y + 1) * self.w * 4] for y in range(self.h))

        def chunk(t, d):
            return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
        return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", self.w, self.h, 8, 6, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))

    def save(self, path):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(self.to_png())

    @staticmethod
    def from_png(data):
        """Decode 8-bit RGB/RGBA non-interlaced PNG (what to_png writes, and what tab.py shot writes)."""
        if data[:8] != b"\x89PNG\r\n\x1a\n":
            raise E2EError("not a PNG")
        pos, idat, w, h, bpp = 8, b"", 0, 0, 4
        while pos < len(data):
            n, t = struct.unpack(">I4s", data[pos:pos + 8])
            body = data[pos + 8:pos + 8 + n]
            pos += 12 + n
            if t == b"IHDR":
                w, h, depth, ctype, _, _, inter = struct.unpack(">IIBBBBB", body)
                if depth != 8 or ctype not in (2, 6) or inter:
                    raise E2EError("unsupported PNG (need 8-bit RGB/RGBA, not interlaced)")
                bpp = 4 if ctype == 6 else 3
            elif t == b"IDAT":
                idat += body
        raw = zlib.decompress(idat)
        stride, out, prev = w * bpp, bytearray(), bytearray(w * bpp)
        p = 0
        for _ in range(h):
            ft, row = raw[p], bytearray(raw[p + 1:p + 1 + stride])
            p += 1 + stride
            for i in range(stride):
                a = row[i - bpp] if i >= bpp else 0
                b = prev[i]
                c = prev[i - bpp] if i >= bpp else 0
                if ft == 1:
                    row[i] = (row[i] + a) & 255
                elif ft == 2:
                    row[i] = (row[i] + b) & 255
                elif ft == 3:
                    row[i] = (row[i] + (a + b) // 2) & 255
                elif ft == 4:
                    pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                    row[i] = (row[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
            out += row
            prev = row
        if bpp == 3:
            rgba = bytearray()
            for i in range(0, len(out), 3):
                rgba += out[i:i + 3] + b"\xff"
            out = rgba
        return Image(w, h, out)

    @staticmethod
    def load(path):
        with open(path, "rb") as f:
            return Image.from_png(f.read())


def diff(a, b, tol=12, mask=None):
    """(fraction of differing pixels, diff Image). A pixel differs when any channel differs by more than tol.
    mask: list of (x0, y0, x1, y1) in image pixels to ignore (clock, battery)."""
    if (a.w, a.h) != (b.w, b.h):
        return 1.0, None
    out, bad = bytearray(a.w * a.h * 4), 0
    ra, rb = a.rgba, b.rgba
    for y in range(a.h):
        for x in range(a.w):
            i = (y * a.w + x) * 4
            if mask and any(m[0] <= x < m[2] and m[1] <= y < m[3] for m in mask):
                out[i:i + 4] = b"\x20\x20\x20\xff"
                continue
            d = max(abs(ra[i] - rb[i]), abs(ra[i + 1] - rb[i + 1]), abs(ra[i + 2] - rb[i + 2]))
            if d > tol:
                bad += 1
                out[i:i + 4] = b"\xff\x00\x00\xff"
            else:
                g = (ra[i] + ra[i + 1] + ra[i + 2]) // 6
                out[i:i + 4] = bytes((g, g, g, 255))
    return bad / (a.w * a.h), Image(a.w, a.h, out)


def check_baseline(img, target, name, update=False, tol=12, max_frac=0.01, mask=None):
    """Compare img with baselines/<target>/<name>.png. Returns a note; raises E2EError on a regression.
    A missing baseline is created (and noted); update=True overwrites."""
    path = os.path.join(BASELINES, target, name + ".png")
    if update or not os.path.exists(path):
        img.save(path)
        return f"baseline {'updated' if update else 'created'}: {name}"
    base = Image.load(path)
    frac, dimg = diff(img, base, tol, mask)
    if frac > max_frac:
        os.makedirs(DIFF_DIR, exist_ok=True)
        stamp = f"{target}-{name}"
        img.save(os.path.join(DIFF_DIR, stamp + ".actual.png"))
        if dimg:
            dimg.save(os.path.join(DIFF_DIR, stamp + ".diff.png"))
        raise E2EError(f"pixels differ for {name}: {frac * 100:.2f}% > {max_frac * 100:.2f}% (see {DIFF_DIR}/{stamp}.diff.png)")
    return f"pixels ok {name} ({frac * 100:.2f}% differ)"
