#!/usr/bin/env python3
"""e2e runner: run.py [--port N] [scenario ...] [--update-baseline] [--json] [--list]
Exit codes: 0 all passed, 1 a scenario failed, 2 usage error or target unreachable."""
import argparse
import json
import os
import sys
import time

HERE = os.path.dirname(os.path.realpath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.dirname(HERE))  # tools/ for fps.py

from client import E2EError, Target  # noqa: E402
from scenarios import SCENARIOS, Ctx  # noqa: E402


def run_one(t, target, name, fn, update):
    c = Ctx(t, target, update)
    t0 = time.time()
    status, err = "pass", ""
    try:
        fn(c)
    except E2EError as e:
        status, err = "fail", str(e)
    except Exception as e:  # keep going: a broken scenario must not hide the others
        status, err = "error", f"{type(e).__name__}: {e}"
    return {"name": name, "status": status, "error": err, "seconds": round(time.time() - t0, 2), "notes": c.notes}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("scenarios", nargs="*")
    ap.add_argument("--target", choices=["vm"], default="vm")
    ap.add_argument("--port", type=int, default=None, help="control port (default: 7777 inside the vm)")
    ap.add_argument("--update-baseline", action="store_true")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--list", action="store_true")
    a = ap.parse_args(argv)
    if a.port is None:
        a.port = 7777
    table = dict(SCENARIOS)
    if a.list:
        print("\n".join(table))
        return 0
    unknown = [s for s in a.scenarios if s not in table]
    if unknown:
        print(f"unknown scenario(s): {', '.join(unknown)} (see --list)", file=sys.stderr)
        return 2
    t = Target(a.target, a.port)
    try:
        t.state()
    except E2EError as e:
        print(f"target {a.target}:{a.port} unreachable: {e}", file=sys.stderr)
        return 2
    chosen = [(n, table[n]) for n in a.scenarios] if a.scenarios else SCENARIOS
    results = []
    for name, fn in chosen:
        r = run_one(t, a.target, name, fn, a.update_baseline)
        results.append(r)
        if not a.json:
            print(f"{r['status'].upper():5} {name:22} {r['seconds']:6.1f}s  {r['error']}")
            for n in r["notes"]:
                print(f"        {n}")
    failed = [r for r in results if r["status"] != "pass"]
    if a.json:
        print(json.dumps({"target": a.target, "passed": len(results) - len(failed), "failed": len(failed), "results": results}, indent=2))
    else:
        print(f"\n{len(results) - len(failed)}/{len(results)} passed" + (f", failed: {', '.join(r['name'] for r in failed)}" if failed else ""))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
