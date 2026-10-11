#!/usr/bin/env python3
"""MCP-driven smoke test of the BifrOSt Browser on a Midget, over one stdio MCP session (tools/bifrost-mcp-ssh).
Steps: open a page, eval, console, network, dom, screenshot with scale, inspect, new_tab/select_tab (active tab's
url checked), downloads list. Prints PASS/FAIL per step; exits 1 when any step failed.
Usage: tools/browser-smoke.py   (MIDGET_HOST overrides the address; BIFROST_MCP picks the remote binary)"""
import json, os, re, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
PAGE = "data:text/html,<title>smoke-a</title><h1 id=h style='margin:0'>hello</h1><script>console.error('smoke-err');fetch('/smoke-x').catch(()=>{})</script>"
PAGE_B = "data:text/html,<title>smoke-b</title><p>second</p>"


class Mcp:
    def __init__(self):
        self.p = subprocess.Popen([os.path.join(HERE, "bifrost-mcp-ssh")], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.n = 0
        self.rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "browser-smoke", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, m):
        self.p.stdin.write(json.dumps(m) + "\n")
        self.p.stdin.flush()

    def rpc(self, method, params):
        self.n += 1
        self.send({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params})
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("MCP session closed")
            try:
                r = json.loads(line)
            except ValueError:
                continue
            if r.get("id") == self.n:
                return r

    def browser(self, **args):
        r = self.rpc("tools/call", {"name": "browser", "arguments": args})
        res = r.get("result", {})
        text = "\n".join(c.get("text", "") for c in res.get("content", []) if c.get("type") == "text")
        images = [c for c in res.get("content", []) if c.get("type") == "image"]
        if "error" in r:
            return False, json.dumps(r["error"]), images
        return not res.get("isError"), text, images


def active_tab(m):
    """The tabs line of the current tab ('*ID [index] url "title"')."""
    _, out, _ = m.browser(action="tabs")
    cur = [l for l in out.splitlines() if l.startswith("*")]
    return cur[0] if cur else ""


def px(text):
    g = re.search(r"(\d+)x(\d+) px", text)
    return (int(g.group(1)), int(g.group(2))) if g else None


def main():
    bad = 0

    def step(name, ok, detail=""):
        nonlocal bad
        ok = bool(ok)
        bad += not ok
        print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail[:140]!r}" if detail else ""), flush=True)

    try:
        m = Mcp()
    except Exception as e:
        print(f"FAIL connect: {e}")
        return 1
    try:
        ok, out, _ = m.browser(action="navigate", url=PAGE, wait="load")
        step("open page", ok and "smoke-a" in out, out)
        ok, out, _ = m.browser(action="eval", js="document.getElementById('h').textContent")
        step("eval", ok and "hello" in out, out)
        ok, out, _ = m.browser(action="console", level="error", reset=True)
        step("console", ok and "smoke-err" in out, out)
        ok, out, _ = m.browser(action="network", reset=True)
        step("network", ok, out)
        ok, out, _ = m.browser(action="dom", selector="#h", mode="query")
        step("dom", ok and "h1" in out, out)
        ok1, full, i1 = m.browser(action="screenshot", scale=1)
        ok2, half, i2 = m.browser(action="screenshot", scale=0.5)
        a, b = px(full), px(half)
        step("screenshot scale", ok1 and ok2 and i1 and i2 and a and b and abs(b[0] * 2 - a[0]) <= 2, f"{a} -> {b}")
        ok, out, _ = m.browser(action="inspect", x=10, y=10)
        step("inspect", ok and "h1" in out.lower(), out)
        ok, out, _ = m.browser(action="new_tab", url=PAGE_B)
        cur = active_tab(m)
        step("new_tab becomes active", ok and "smoke-b" in cur, cur)
        _, tabs, _ = m.browser(action="tabs")
        first = [l for l in tabs.splitlines() if "smoke-a" in l]
        tid = re.match(r"[-*](\d+)", first[0]).group(1) if first else None
        ok, out, _ = m.browser(action="select_tab", id=int(tid)) if tid else (False, "tab A not found", [])
        cur = active_tab(m)
        step("select_tab shows that tab's url", ok and "smoke-a" in cur, cur)
        ok, out, _ = m.browser(action="downloads")
        step("downloads list", ok, out)
    except Exception as e:
        step("session", False, str(e))
    finally:
        try:
            m.p.stdin.close()
        except Exception:
            pass
    return 1 if bad else 0


sys.exit(main())
