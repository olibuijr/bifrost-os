#!/usr/bin/env python3
"""bifrost pc parity verify os-cli: assess the docs/parity-matrix/os/cli rows and verify each tool on Midget.

  verify os-cli plan [--out plan.json]          per row {id, command, probe}; rows with no command or n/a are left out
  verify os-cli run plan.json --version V [--out results.json]
                                                every probe in ONE batched ssh script (tools/midget.py ssh)
  verify os-cli closures results.json --version V [--dry-run]
                                                rewrite the os/cli shards (method 'assessed'):
                                                parity (probe passed), partial (present, probe failed or not runnable
                                                headless), gap (no equivalent / command missing), n/a (with reason)

The mapping is tools/parity/os_cli_map.json. Stdlib only.
"""
import glob
import json
import os
import re
import shlex
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
MAP = os.path.join(HERE, "os_cli_map.json")
CLI_DIR = os.path.join(ROOT, "docs", "parity-matrix", "os", "cli")
MIDGET = os.path.join(ROOT, "tools", "midget.py")
TIMEOUT = 5  # seconds per probe


def load_map(path=MAP):
    with open(path) as f:
        return json.load(f)


def na_reason(name, path, m):
    """Why a tool is n/a (a justification string), or None when it applies to BifrOSt."""
    for cls in m.get("na_names", {}).values():
        if isinstance(cls, dict) and name in cls.get("names", []):
            return cls["reason"]
    if isinstance(m.get("commands", {}).get(name), str):
        return None  # an explicit rename/mapping beats the generic daemon patterns
    for pat in m.get("na_patterns", []):
        if path.startswith(pat["path_prefix"]) and re.search(pat["name_regex"], name):
            return pat["reason"]
    return None


def command_for(name, m):
    """The command providing `name`: the same name unless the map renames it, None when the map says no equivalent."""
    cmds = m.get("commands", {})
    return cmds[name] if name in cmds else name


def probe_for(command, m):
    """Shell snippet printing `STATE<TAB>PATH` where STATE is MISSING, PRESENT (not run), FAIL or PASS."""
    q = shlex.quote(command)
    if command in m.get("no_exec", {}).get("names", []):
        run = "r=PRESENT"
    else:
        run = (f"r=FAIL; if timeout {TIMEOUT} {q} --version </dev/null >/dev/null 2>&1 "
               f"|| timeout {TIMEOUT} {q} --help </dev/null >/dev/null 2>&1; then r=PASS; fi")
    return f"p=$(command -v {q}) && {{ {run}; }} || r=MISSING; printf '%s\\t%s\\n' \"$r\" \"$p\""


def load_shards(directory=CLI_DIR):
    shards = {}
    for f in sorted(glob.glob(os.path.join(directory, "*.json"))):
        with open(f) as fh:
            shards[f] = json.load(fh)
    return shards


def entries(shards):
    return [e for sh in shards.values() for e in sh["entries"]]


def build_plan(rows, m):
    plan = []
    for e in rows:
        cmd = command_for(e["name"], m)
        if cmd is None or na_reason(e["name"], e["path"], m):
            continue
        plan.append({"id": e["id"], "command": cmd, "probe": probe_for(cmd, m)})
    return plan


def batch_script(plan):
    """One script for all probes; each prints `@@INDEX<TAB>STATE<TAB>PATH`."""
    # Arch keeps Perl core/vendor scripts and ssh helpers off the non-login ssh PATH
    lines = ["#!/bin/sh", 'PATH="$PATH:/usr/bin/core_perl:/usr/bin/vendor_perl:/usr/bin/site_perl"; export PATH']
    for i, item in enumerate(plan):
        lines.append(f"printf '@@%s\\t' {i}; ( {item['probe']} ) 2>/dev/null || printf 'FAIL\\t\\n'")
    return "\n".join(lines) + "\n"


def parse_output(text, plan):
    """{id: {state, path}} from the batched output; a probe that printed nothing is FAIL."""
    out = {item["id"]: {"state": "FAIL", "path": ""} for item in plan}
    for line in text.splitlines():
        if not line.startswith("@@"):
            continue
        parts = line[2:].split("\t")
        if len(parts) < 2 or not parts[0].isdigit() or int(parts[0]) >= len(plan):
            continue
        state = parts[1] if parts[1] in ("MISSING", "PRESENT", "FAIL", "PASS") else "FAIL"
        out[plan[int(parts[0])]["id"]] = {"state": state, "path": parts[2] if len(parts) > 2 else ""}
    return out


def midget_ssh_argv():
    """The ssh command line tools/midget.py uses to reach Midget (host pick, key, options)."""
    sys.path.insert(0, os.path.join(ROOT, "tools"))
    import midget
    return list(midget.SSH)


def run_plan(plan, run=subprocess.run, argv=None):
    """One ssh session reading the whole batched script from stdin (midget.py ssh has a 120 s cap and no stdin)."""
    r = run((argv or midget_ssh_argv()) + ["sh -s"], input=batch_script(plan), capture_output=True, text=True,
            timeout=1800)
    return parse_output(r.stdout, plan)


def apply_result(entry, m, result, version):
    """Update one row in place from its assessment; returns the new status."""
    reason = na_reason(entry["name"], entry["path"], m)
    cmd = command_for(entry["name"], m)
    entry["method"] = "assessed"
    if reason:
        entry.update(status="n/a", bifrost="", note=reason)
    elif cmd is None:
        entry.update(status="gap", bifrost="", note="no BifrOSt or Linux equivalent")
    else:
        state = (result or {}).get("state", "UNPROBED")
        path = (result or {}).get("path") or cmd
        if state == "PASS":
            entry.update(status="parity", bifrost=path,
                         note=f"verified on Midget {version}: {cmd} --version/--help exits 0")
        elif state == "MISSING":
            entry.update(status="gap", bifrost="", note=f"checked on Midget {version}: {cmd} not installed")
        else:
            why = {"PRESENT": "installed, not run headless (changes machine state)",
                   "FAIL": "installed, --version/--help probe failed"}.get(state, "not probed")
            entry.update(status="partial", bifrost=path, note=f"Midget {version}: {cmd} {why}")
    return entry["status"]


def closures(results, version, directory=CLI_DIR, write=True):
    m = load_map()
    shards = load_shards(directory)
    counts = {}
    for e in entries(shards):
        s = apply_result(e, m, results.get(e["id"]), version)
        counts[s] = counts.get(s, 0) + 1
    if write:
        for f, sh in shards.items():
            with open(f, "w") as fh:
                json.dump(sh, fh, indent=1, ensure_ascii=False)
                fh.write("\n")
    return counts


def main(argv):
    if len(argv) < 1 or argv[0] not in ("plan", "run", "closures"):
        sys.exit(__doc__)
    sub, rest = argv[0], argv[1:]

    def opt(k, d=None):
        return rest[rest.index(k) + 1] if k in rest else d

    if sub == "plan":
        plan = build_plan(entries(load_shards()), load_map())
        with open(opt("--out", "plan.json"), "w") as f:
            json.dump(plan, f, indent=1)
        print(f"{len(plan)} probes planned")
    elif sub == "run":
        with open(rest[0]) as f:
            plan = json.load(f)
        res = run_plan(plan)
        with open(opt("--out", "results.json"), "w") as f:
            json.dump(res, f, indent=1)
        print({s: sum(1 for r in res.values() if r["state"] == s) for s in ("PASS", "FAIL", "PRESENT", "MISSING")})
    else:
        with open(rest[0]) as f:
            res = json.load(f)
        print(closures(res, opt("--version", "unknown"), write="--dry-run" not in rest))


if __name__ == "__main__":
    main(sys.argv[1:])
