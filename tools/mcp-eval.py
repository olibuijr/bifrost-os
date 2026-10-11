#!/usr/bin/env python3
"""Agentic evals for the BifrOSt MCP server: each task in tools/mcp-evals.json is given to Claude Code
headless (`claude -p`, only the bifrost MCP tools allowed), then checked against ground truth read
straight from Midget over a separate MCP session. Records success, turns, tool calls by name, tool
errors, wall time and cost, so tool changes can be compared run against run.

usage: tools/mcp-eval.py [--model claude-haiku-5-5] [--only ID[,ID]] [--repeat N] [--record LABEL]
Results: printed table; with --record, one JSON line per run in docs/mcp-evals.jsonl and the
transcripts in out/mcp-evals/<label>/.
"""
import argparse, collections, json, os, re, subprocess, sys, time, uuid

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools"))
from importlib import import_module

Client = import_module("mcp-bench").Client
LAUNCH = os.path.join(ROOT, "tools", "bifrost-mcp-ssh")
MCP_CONFIG = os.path.join(ROOT, ".mcp.json")


def text_of(r):
    return "\n".join(c.get("text", "") for c in r.get("content", []))


def fill(v, nonce):
    if isinstance(v, str):
        return v.replace("{nonce}", nonce)
    if isinstance(v, list):
        return [fill(x, nonce) for x in v]
    if isinstance(v, dict):
        return {k: fill(x, nonce) for k, x in v.items()}
    return v


def run_claude(prompt, model, max_turns, log_path):
    argv = ["claude", "-p", prompt, "--mcp-config", MCP_CONFIG, "--strict-mcp-config",
            "--allowedTools", "mcp__bifrost__*", "--disallowedTools", "Bash,Read,Write,Edit,Glob,Grep,WebFetch,WebSearch,Agent,Task",
            "--model", model, "--output-format", "stream-json", "--verbose", "--no-session-persistence",
            "--setting-sources", "", "--max-turns", str(max_turns)]
    t = time.time()
    p = subprocess.run(argv, capture_output=True, text=True, timeout=900, cwd="/tmp")
    wall = time.time() - t
    if log_path:
        with open(log_path, "w") as f:
            f.write(p.stdout)
    tools, errors, result, answer = collections.Counter(), 0, {}, ""
    pending = {}
    for line in p.stdout.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("type") == "assistant":
            for c in m["message"].get("content", []):
                if c.get("type") == "tool_use":
                    name = c["name"].removeprefix("mcp__bifrost__")
                    tools[name] += 1
                    pending[c["id"]] = name
        elif m.get("type") == "user":
            for c in m["message"].get("content", []) if isinstance(m["message"].get("content"), list) else []:
                if c.get("type") == "tool_result" and c.get("is_error"):
                    errors += 1
        elif m.get("type") == "result":
            result = m
            answer = m.get("result") or ""
    return {"answer": answer, "tools": dict(tools), "tool_errors": errors, "wall_s": round(wall, 1),
            "turns": result.get("num_turns"), "cost_usd": round(result.get("total_cost_usd") or 0, 4),
            "api_ms": result.get("duration_api_ms"), "status": result.get("subtype", "no-result")}


def check(spec, run, c, nonce):
    """True/False with a reason, against Midget's real state."""
    ans = run["answer"]
    if "all" in spec:
        for s in spec["all"]:
            ok, why = check(s, run, c, nonce)
            if not ok:
                return ok, why
        return True, "all"
    if "answer_has" in spec:
        return spec["answer_has"] in re.sub(r"(?<=\d)[,.\u00a0 ](?=\d{3})", "", ans), f"answer has {spec['answer_has']!r}"
    if "answer_any" in spec:
        return any(w.lower() in ans.lower() for w in spec["answer_any"]), "answer names a folder"
    if "answer_has_call" in spec:
        tool, args, rx = spec["answer_has_call"]
        m = re.search(rx, text_of(c.call(tool, args)))
        return bool(m and m.group(1) in ans), f"answer has {m and m.group(1)!r}"
    if "answer_has_exec" in spec:
        want = text_of(c.call("exec", {"command": spec["answer_has_exec"]})).split("\n")[0].strip()
        return bool(want) and want.lower() in ans.lower(), f"answer has {want!r}"
    if "answer_time_near" in spec:
        now = text_of(c.call("exec", {"command": "date +%H:%M"})).split("\n")[0].strip()
        m = re.search(r"(\d{1,2}):(\d{2})", ans)
        if not m:
            return False, f"no HH:MM in answer (now {now})"
        h, mi = map(int, now.split(":"))
        diff = abs((int(m.group(1)) * 60 + int(m.group(2))) - (h * 60 + mi)) % (24 * 60)
        return min(diff, 1440 - diff) <= spec["answer_time_near"], f"answer {m.group(0)} vs {now}"
    if "window_open" in spec:
        obs = text_of(c.call("observe")).lower()
        return f'"{spec["window_open"]}' in obs, f"window {spec['window_open']} open"
    if "window_state" in spec:
        name, state = spec["window_state"]
        line = next((l for l in text_of(c.call("observe")).lower().splitlines() if f'"{name}' in l), "")
        return state in line, f"window line {line!r}"
    if "app_has" in spec:
        app, method, params, needle = spec["app_has"]
        return needle in text_of(c.call("app", {"app": app, "method": method, "params": params})), f"{app} {method} has {needle!r}"
    if "exec_ok" in spec:
        return not c.call("exec", {"command": spec["exec_ok"]}).get("isError"), spec["exec_ok"]
    if "no_tool" in spec:
        return spec["no_tool"] not in run["tools"], f"did not use {spec['no_tool']}"
    return False, f"unknown check {spec}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", default="claude-haiku-5-5")
    ap.add_argument("--only")
    ap.add_argument("--repeat", type=int, default=1)
    ap.add_argument("--max-turns", type=int, default=30)
    ap.add_argument("--record")
    a = ap.parse_args()
    tasks = json.load(open(os.path.join(ROOT, "tools", "mcp-evals.json")))
    if a.only:
        keep = a.only.split(",")
        tasks = [t for t in tasks if t["id"] in keep]
    logdir = os.path.join(ROOT, "out", "mcp-evals", a.record) if a.record else None
    if logdir:
        os.makedirs(logdir, exist_ok=True)
    c = Client([LAUNCH])
    server = c.info["serverInfo"]["version"]
    sha = subprocess.run(["git", "-C", ROOT, "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    rows = []
    print(f"{'task':22}{'ok':>4}{'turns':>7}{'calls':>7}{'err':>5}{'wall s':>8}{'$':>8}  tools / why")
    for rep in range(a.repeat):
        for t in tasks:
            nonce = "mcp" + uuid.uuid4().hex[:6]
            for tool, args in fill(t.get("setup", []), nonce):
                c.call(tool, args)
            log = os.path.join(logdir, f"{t['id']}-{rep}.jsonl") if logdir else None
            run = run_claude(fill(t["prompt"], nonce), a.model, t.get("max_turns", a.max_turns), log)
            ok, why = check(fill(t["check"], nonce), run, c, nonce)
            for tool, args in fill(t.get("cleanup", []), nonce):
                c.call(tool, args)
            calls = sum(run["tools"].values())
            tl = " ".join(f"{k}:{v}" for k, v in sorted(run["tools"].items(), key=lambda x: -x[1]))
            print(f"{t['id']:22}{'✓' if ok else '✗':>4}{run['turns'] or 0:7}{calls:7}{run['tool_errors']:5}{run['wall_s']:8.1f}{run['cost_usd']:8.3f}  {tl}" + ("" if ok else f"  [{why}] answer={run['answer'][:120]!r}"))
            rows.append({"task": t["id"], "ok": ok, "why": why, "calls": calls, **run})
    c.close()
    n = len(rows)
    passed = sum(r["ok"] for r in rows)
    print(f"\npass {passed}/{n}  mean calls {sum(r['calls'] for r in rows) / n:.1f}  mean wall {sum(r['wall_s'] for r in rows) / n:.1f}s  "
          f"tool errors {sum(r['tool_errors'] for r in rows)}  cost ${sum(r['cost_usd'] for r in rows):.3f}")
    if a.record:
        with open(os.path.join(ROOT, "docs", "mcp-evals.jsonl"), "a") as f:
            for r in rows:
                f.write(json.dumps({"date": time.strftime("%Y-%m-%d %H:%M"), "sha": sha, "server": server, "model": a.model, "label": a.record, **r}, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
