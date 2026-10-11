#!/usr/bin/env python3
"""Acceptance for the MCP `browser` tool against a running Midget (ssh HOST bifrost-mcp call browser JSON).
Steps: state, tabs, navigate, eval, dom, console, network, screenshot, source. Exits non-zero on failure."""
import json, subprocess, sys

HOST = sys.argv[1] if len(sys.argv) > 1 else "midget"
URL = "data:text/html,<title>smoke</title><h1 id=h>hi</h1><script>console.error('smoke-err');fetch('/x').catch(()=>{})</script>"


def call(args):
    p = subprocess.run(["ssh", HOST, "bifrost-mcp", "call", "browser", json.dumps(args)], capture_output=True, text=True, timeout=90)
    return p.returncode == 0 and "(error)" not in p.stderr, p.stdout.strip()


STEPS = [
    ("state", {"action": "state"}, "engine_alive"),
    ("tabs", {"action": "tabs"}, ""),
    ("navigate", {"action": "navigate", "url": URL}, "smoke"),
    ("eval", {"action": "eval", "js": "document.getElementById('h').textContent"}, "hi"),
    ("dom", {"action": "dom", "selector": "#h", "mode": "query"}, "h1"),
    ("console", {"action": "console", "level": "error", "session": "smoke"}, "smoke-err"),
    ("network", {"action": "network", "session": "smoke"}, ""),
    ("screenshot", {"action": "screenshot"}, "px"),
    ("source", {"action": "source"}, "smoke"),
]
bad = 0
for name, args, want in STEPS:
    ok, out = call(args)
    ok = ok and want in out
    bad += not ok
    print(f"{'ok  ' if ok else 'FAIL'} {name}: {out[:120]!r}")
sys.exit(1 if bad else 0)
