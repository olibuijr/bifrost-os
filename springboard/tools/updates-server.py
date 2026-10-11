#!/usr/bin/env python3
"""Update store for the Tab A8: serves ~/.local/share/tab-a8-updates (Software Update
reads springboard/latest.json from it) and accepts diagnostics bundles.

  POST /diag/<name>   body = text bundle, saved as ~/.local/share/tab-a8-updates/diag/<name>
                      (name: letters, digits, . _ -, at most 100 characters; 2 MB limit)

Usage: updates-server.py [port]   (default 8190, all interfaces)
"""
import os
import re
import sys
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer

ROOT = os.path.expanduser("~/.local/share/tab-a8-updates")
MAX_BODY = 2 * 1024 * 1024
NAME = re.compile(r"^[A-Za-z0-9._-]{1,100}$")


def diag_name(path):
    """The bundle name in a /diag/<name> path, or None when it is not a safe one."""
    prefix = "/diag/"
    if not path.startswith(prefix):
        return None
    name = path[len(prefix):]
    return name if NAME.match(name) and name not in (".", "..") else None


class Handler(SimpleHTTPRequestHandler):
    def do_POST(self):
        name = diag_name(self.path)
        length = int(self.headers.get("Content-Length") or 0)
        if name is None:
            return self.reply(400, "bad name")
        if not 0 < length <= MAX_BODY:
            return self.reply(413 if length else 400, "bad length")
        body = self.rfile.read(length)
        os.makedirs(os.path.join(ROOT, "diag"), exist_ok=True)
        tmp = os.path.join(ROOT, "diag", name + ".part")
        with open(tmp, "wb") as f:
            f.write(body)
        os.replace(tmp, os.path.join(ROOT, "diag", name))
        self.reply(200, "ok")

    def reply(self, code, text):
        data = (text + "\n").encode()
        self.send_response(code)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


if __name__ == "__main__":
    os.makedirs(ROOT, exist_ok=True)
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8190
    ThreadingHTTPServer(("0.0.0.0", port), partial(Handler, directory=ROOT)).serve_forever()
