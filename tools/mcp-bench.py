#!/usr/bin/env python3
"""Latency benchmark for the BifrOSt MCP server, measured the way Claude Code sees it: one persistent
stdio session through tools/bifrost-mcp-ssh, JSON-RPC round trips timed on Titan. Read-only and
harmless tools only (the pointer moves; nothing is clicked or typed).

usage: tools/mcp-bench.py [-n N] [--baseline] [--record LABEL]
  --baseline  also time the old paths (midget.py state/screen over a fresh ssh each call)
  --record    append p50s to docs/perf-history.jsonl (results keys mcp_<case>_ms)
"""
import argparse, json, os, statistics, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LAUNCH = os.path.join(ROOT, "tools", "bifrost-mcp-ssh")


class Client:
    """A minimal MCP client over a subprocess's stdio."""

    def __init__(self, argv):
        self.p = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        self.n = 0
        t = time.perf_counter()
        self.info = self.req("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "mcp-bench", "version": "1"}})
        self.init_ms = (time.perf_counter() - t) * 1000
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")

    def req(self, method, params=None):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        line = self.p.stdout.readline()
        if not line:
            raise SystemExit("server closed the session")
        msg = json.loads(line)
        if "error" in msg:
            raise RuntimeError(msg["error"])
        return msg["result"]

    def call(self, name, args=None):
        return self.req("tools/call", {"name": name, "arguments": args or {}})

    def close(self):
        self.p.stdin.close()
        self.p.wait(5)


def size_of(result):
    return sum(len(c.get("text", "")) + len(c.get("data", "")) for c in result.get("content", []))


CASES = [
    ("observe", {}),
    ("screenshot_screen", ("screenshot", {})),
    ("screenshot_screen_full", ("screenshot", {"max_size": 4096})),
    ("screenshot_region", ("screenshot", {"region": [0, 0, 800, 60]})),
    ("screenshot_window", ("screenshot", {"window": "FRONT"})),
    ("hover", ("pointer", {"x": 2400, "y": 1400})),
    ("ui_list", ("ui", {"filter": "general"})),
    ("app_list", ("app", {})),
    ("window_list", ("window", {"action": "list"})),
    ("perf_short", ("perf", {"ms": 200})),
    ("wait_app_ready", ("wait", {"app_ready": "notes", "timeout_s": 1})),
    ("system", ("system", {})),
    ("browser_state", ("browser", {"action": "state"})),
    ("exec_true", ("exec", {"command": "true"})),
    ("shell_version", ("shell", {"command": "version"})),
    ("batch3", ("batch", {"steps": [{"tool": "observe"}, {"tool": "system"}, {"tool": "shell", "args": {"command": "version"}}]})),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-n", type=int, default=10)
    ap.add_argument("--baseline", action="store_true")
    ap.add_argument("--record")
    a = ap.parse_args()
    t = time.perf_counter()
    c = Client([LAUNCH])
    print(f"session start (ssh + initialize): {(time.perf_counter() - t) * 1000:.0f} ms; initialize {c.init_ms:.1f} ms")
    tools = c.req("tools/list")["tools"]
    print(f"{len(tools)} tools, tools/list {len(json.dumps(tools))} bytes")
    # the focused window, for the window capture case
    front = next((l.split('"')[1] for l in c.call("observe")["content"][0]["text"].splitlines() if l.startswith("* \"")), None)
    results = {}
    print(f"{'case':28}{'p50 ms':>9}{'p95 ms':>9}{'max ms':>9}{'bytes':>10}")
    for label, spec in CASES:
        name, args = (label, spec) if isinstance(spec, dict) else spec
        if args.get("window") == "FRONT":
            if not front:
                continue
            args = {**args, "window": front}
        times, size, errors = [], 0, 0
        for _ in range(a.n):
            t = time.perf_counter()
            r = c.call(name, args)
            times.append((time.perf_counter() - t) * 1000)
            size = size_of(r)
            errors += bool(r.get("isError"))
        times.sort()
        p95 = times[min(len(times) - 1, int(len(times) * 0.95))]
        results[label] = round(statistics.median(times), 1)
        print(f"{label:28}{statistics.median(times):9.1f}{p95:9.1f}{times[-1]:9.1f}{size:10}{'  errors=' + str(errors) if errors else ''}")
    c.close()
    if a.baseline:
        sys.path.insert(0, os.path.join(ROOT, "tools"))
        for label, argv in [("old_state", ["state"]), ("old_screen", ["screen", "/tmp/mcp-bench-old.png"])]:
            times = []
            for _ in range(max(3, a.n // 3)):
                t = time.perf_counter()
                subprocess.run([sys.executable, os.path.join(ROOT, "tools", "midget.py"), *argv], capture_output=True, timeout=60)
                times.append((time.perf_counter() - t) * 1000)
            results[label] = round(statistics.median(times), 1)
            print(f"{label:28}{statistics.median(times):9.1f}{max(times):18.1f}   (midget.py, ssh per call)")
    if a.record:
        sha = subprocess.run(["git", "-C", ROOT, "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
        row = {"date": time.strftime("%Y-%m-%d %H:%M"), "sha": sha, "version": c.info["serverInfo"]["version"], "host": "midget",
               "label": a.record, "results": {f"mcp_{k}_ms": v for k, v in results.items()}}
        with open(os.path.join(ROOT, "docs", "perf-history.jsonl"), "a") as f:
            f.write(json.dumps(row) + "\n")
        print("recorded", a.record)


if __name__ == "__main__":
    main()
