#!/usr/bin/env python3
"""LAN relay for the Midget remote-control app: a fixed set of actions, each run through tools/midget.py.

GET  /api/status            midget.py status (down | up | live | installed)
GET  /api/screen            PNG of Midget's screen (X-Width / X-Height headers give panel pixels)
GET  /api/frame?w=960&q=60  JPEG scaled to width w (live view); X-Width / X-Height are still panel pixels
POST /api/tap      {x, y}   tap in original panel pixels
POST /api/swipe    {x0,y0,x1,y1,ms}
POST /api/key      {key}    enter|esc|tab|space|backspace|up|down|left|right
POST /api/open-app {id}
POST /api/wake
POST /api/reboot   {mode}   netboot|disk

Every request needs `Authorization: Bearer <token>`. The token lives in out/lab/relay-token (0600),
created on first start. Nothing else is exposed: no raw SSH, no control socket, no shell.
usage: tools/midget-relay.py [--bind 192.168.1.12] [--port 8099]
"""
import argparse, io, json, os, re, secrets, struct, subprocess, sys, tempfile, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MIDGET = os.path.join(REPO, "tools", "midget.py")
TOKEN_FILE = os.path.join(REPO, "out", "lab", "relay-token")
KEYS = {"enter", "esc", "tab", "space", "backspace", "up", "down", "left", "right"}
APP_ID = re.compile(r"^[A-Za-z0-9._-]{1,120}$")
LOCK = threading.Lock()  # one midget.py call at a time: the lab link is a single control path


def load_token():
    os.makedirs(os.path.dirname(TOKEN_FILE), exist_ok=True)
    if not os.path.exists(TOKEN_FILE):
        fd = os.open(TOKEN_FILE, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(secrets.token_urlsafe(32) + "\n")
    with open(TOKEN_FILE) as f:
        return f.read().strip()


def midget(*args, timeout=60):
    with LOCK:
        r = subprocess.run([sys.executable, MIDGET, *args], capture_output=True, text=True, errors="replace", timeout=timeout)
    return r.returncode, (r.stdout + r.stderr).strip()


def png_size(data):
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    return struct.unpack(">II", data[16:24])


def number(v, lo, hi):
    if isinstance(v, bool) or not isinstance(v, (int, float)) or not lo <= v <= hi:
        raise ValueError("coordinate out of range")
    return str(round(float(v), 1))


def make_handler(token):
    class Handler(BaseHTTPRequestHandler):
        server_version = "midget-relay/1"

        def log_message(self, fmt, *args):
            sys.stderr.write("%s %s\n" % (self.address_string(), fmt % args))

        def send_json(self, code, obj):
            body = json.dumps(obj).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def authorized(self):
            given = self.headers.get("Authorization", "")
            return given.startswith("Bearer ") and secrets.compare_digest(given[7:], token)

        def body(self):
            n = int(self.headers.get("Content-Length") or 0)
            if n > 4096:
                raise ValueError("body too large")
            data = json.loads(self.rfile.read(n) or b"{}")
            if not isinstance(data, dict):
                raise ValueError("expected a JSON object")
            return data

        def do_GET(self):
            if not self.authorized():
                return self.send_json(401, {"error": "unauthorized"})
            if self.path == "/api/status":
                code, out = midget("status", timeout=30)
                line = next((l for l in out.splitlines() if l.startswith("state:")), "")
                state = line.split()[1] if line else "unknown"
                return self.send_json(200, {"state": state, "status": out[-800:]})
            route = urlsplit(self.path)
            if route.path in ("/api/screen", "/api/frame"):
                fd, path = tempfile.mkstemp(suffix=".png")
                os.close(fd)
                try:
                    code, out = midget("screen", path, timeout=60)
                    data = open(path, "rb").read() if code == 0 else b""
                finally:
                    os.unlink(path)
                if not data:
                    return self.send_json(503, {"error": out or "no screen"})
                size = png_size(data) or (0, 0)
                ctype = "image/png"
                if route.path == "/api/frame":
                    q = parse_qs(route.query)
                    width = min(max(int(q.get("w", ["960"])[0]), 240), 2560)
                    quality = min(max(int(q.get("q", ["60"])[0]), 30), 95)
                    from PIL import Image
                    img = Image.open(io.BytesIO(data)).convert("RGB")
                    if img.width > width:
                        img = img.resize((width, round(img.height * width / img.width)), Image.BILINEAR)
                    buf = io.BytesIO()
                    img.save(buf, "JPEG", quality=quality)
                    data, ctype = buf.getvalue(), "image/jpeg"
                self.send_response(200)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("X-Width", str(size[0]))
                self.send_header("X-Height", str(size[1]))
                self.end_headers()
                self.wfile.write(data)
                return
            self.send_json(404, {"error": "not found"})

        def do_POST(self):
            if not self.authorized():
                return self.send_json(401, {"error": "unauthorized"})
            try:
                b = self.body()
                if self.path == "/api/tap":
                    args = ["tap", number(b.get("x"), 0, 10000), number(b.get("y"), 0, 10000)]
                elif self.path == "/api/swipe":
                    ms = int(b.get("ms", 300))
                    if not 50 <= ms <= 5000:
                        raise ValueError("ms out of range")
                    args = ["swipe"] + [number(b.get(k), 0, 10000) for k in ("x0", "y0", "x1", "y1")] + [str(ms)]
                elif self.path == "/api/key":
                    if b.get("key") not in KEYS:
                        raise ValueError("unknown key")
                    args = ["key", b["key"]]
                elif self.path == "/api/open-app":
                    if not APP_ID.match(str(b.get("id", ""))):
                        raise ValueError("bad app id")
                    args = ["open-app", b["id"]]
                elif self.path == "/api/wake":
                    args = ["wake"]
                elif self.path == "/api/reboot":
                    if b.get("mode") not in ("netboot", "disk"):
                        raise ValueError("mode must be netboot or disk")
                    args = ["reboot", "--" + b["mode"]]
                else:
                    return self.send_json(404, {"error": "not found"})
            except (ValueError, TypeError, KeyError) as e:
                return self.send_json(400, {"error": str(e)})
            try:
                code, out = midget(*args, timeout=120)
            except subprocess.TimeoutExpired:
                return self.send_json(504, {"error": "midget did not answer"})
            self.send_json(200 if code == 0 else 502, {"ok": code == 0, "output": out[-600:]})

    return Handler


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bind", default="192.168.1.12")
    ap.add_argument("--port", type=int, default=8099)
    a = ap.parse_args()
    token = load_token()
    srv = ThreadingHTTPServer((a.bind, a.port), make_handler(token))
    print(f"midget-relay on http://{a.bind}:{a.port} (token: out/lab/relay-token)", flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
