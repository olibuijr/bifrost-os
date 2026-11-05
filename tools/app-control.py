#!/usr/bin/env python3
"""Talk to running BifrOSt PC apps over their control sockets (tasks/app-control.md).

  app-control.py list                      running apps that serve a control socket
  app-control.py info APP                  app.info (id, pid, data directory)
  app-control.py methods APP               methods the app serves
  app-control.py call APP METHOD [JSON]    one JSON-RPC call; prints the result

APP is the reverse-DNS id (is.olibuijr.notes.desktop) or a socket path.
Sockets live in the app's data directory, <root>/appdata/<id>/control.sock (root is $BIFROST_SHELL_DIR,
default /var/lib/springboard; the same rule as bifrost_core::app_control_socket), and accept only the app's user.
"""
import glob
import json
import os
import socket
import sys

TIMEOUT = 2.0


def data_root():
    return os.path.join(os.environ.get("BIFROST_SHELL_DIR") or "/var/lib/springboard", "appdata")


def path_for(app):
    return app if app.endswith(".sock") else os.path.join(data_root(), app, "control.sock")


def rpc(app, method, params=None):
    """One request on a fresh connection. Returns the result, or raises SystemExit with the error."""
    req = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params if params is not None else {}}
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(TIMEOUT)
    try:
        s.connect(path_for(app))
    except OSError as e:
        sys.exit(f"{app}: no control socket ({e})")
    try:
        s.sendall((json.dumps(req) + "\n").encode())
        s.shutdown(socket.SHUT_WR)
        data = b""
        while True:
            chunk = s.recv(65536)
            if not chunk:
                break
            data += chunk
    except OSError as e:
        sys.exit(f"{app}: {e}")
    finally:
        s.close()
    try:
        reply = json.loads(data.decode())
    except ValueError:
        sys.exit(f"{app}: malformed reply")
    if "error" in reply:
        sys.exit(f"{app}: error {reply['error'].get('code')}: {reply['error'].get('message')}")
    return reply.get("result")


def cmd_list(_):
    found = False
    for path in sorted(glob.glob(os.path.join(data_root(), "*", "control.sock"))):
        try:
            info = rpc(path, "app.info")
            print(f"{info['id']:40} pid {info['pid']:<8} {info['data']}")
            found = True
        except SystemExit:
            print(f"{os.path.basename(os.path.dirname(path)):40} (not answering: stale socket)")
    if not found:
        print("no apps with a control socket")


def main(argv):
    if len(argv) < 2 or argv[1] not in ("list", "info", "methods", "call"):
        print(__doc__)
        return 2
    op, args = argv[1], argv[2:]
    if op == "list":
        cmd_list(args)
        return 0
    if not args:
        print(__doc__)
        return 2
    app = args[0]
    if op == "info":
        result = rpc(app, "app.info")
    elif op == "methods":
        result = rpc(app, "app.methods")
    else:
        if len(args) < 2:
            print(__doc__)
            return 2
        params = json.loads(args[2]) if len(args) > 2 else {}
        result = rpc(app, args[1], params)
    print(json.dumps(result, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
