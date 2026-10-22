#!/usr/bin/env python3
"""bifrost pc parity os-verify: run the checks of ready OS-layer rows and promote the rows that pass.

  os-verify plan [--out PLAN.json]
        rows with "ready": true (status not parity or n/a) with their checks; test checks grouped so each
        (crate, filter) pair runs once. A malformed "ready" or "checks" field is an error (also in `parity check`).
  os-verify run PLAN.json --target local|ssh:HOST [--repo DIR] [--out RESULTS.json]
        file -> test -e; cmd -> sh -c with a 10 s timeout, as the login user (never root); unit -> systemctl
        [--user] is-enabled|is-active NAME. test -> one `cargo test --offline --profile fast --no-run` over all the
        plan's crates (features resolve as in the lead's build, so nothing built is rebuilt), then each crate's test
        executables run with its filters, locally in --repo (tests are host logic, even for an ssh target). Passing
        outcomes are cached per executable SHA-256 (~/.cache/bifrost-parity/os-verify-tests.json): an unchanged binary's
        passing filters never run again. A filter that runs 0 tests fails. Exit 1 when any row fails.
  os-verify promote RESULTS.json --version V [--target-name vm] [--dry-run]
        passing rows become status parity (ready dropped, note records the check); failing rows keep their status
        and get "ready": false and "ready_failed": reason.

Stdlib only. Shards live in docs/parity-matrix/os/<category>/<shard>.json (indent 1, ensure_ascii False).
"""
import argparse
import json
import os
import re
import shlex
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
OS_DIR = os.path.join(REPO, "docs", "parity-matrix", "os")
SKIP_STATUS = ("parity", "n/a")
UNIT_STATES = ("enabled", "active")
CHECK_KEYS = {
    "file": {"kind", "path"},
    "cmd": {"kind", "run", "expect_exit"},
    "unit": {"kind", "name", "state", "user"},
    "test": {"kind", "crate", "filter"},
}
CMD_TIMEOUT = 10  # seconds, per cmd check
# A cmd check must be harmless and read-only: no privilege, network, removal, power or service control.
FORBIDDEN_CMD = re.compile(
    r"(^|[\s;&|(])(sudo|su|doas|curl|wget|rm|mv|dd|chmod|chown|shutdown|reboot|poweroff|systemctl|pacman|ssh|nc)(?=\s|$)"
    r"|(?<![\d&])>(?!\s*/dev/null)")  # output may go to /dev/null or to the fd 2>&1 dup; no file writes
LIBTEST_RE = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)\b", re.M)
NOTE_TEXT = "Verified on the {target} with BifrOSt {version} (os_verify)."


# --- schema --------------------------------------------------------------------------------------------------------

def check_problems(check):
    """Problems with one check object (empty list when it is well formed)."""
    if not isinstance(check, dict):
        return ["check is not an object"]
    kind = check.get("kind")
    if kind not in CHECK_KEYS:
        return [f"unknown check kind {kind!r}"]
    out = [f"unknown key {k!r} in {kind} check" for k in check if k not in CHECK_KEYS[kind]]
    if kind == "file":
        p = check.get("path")
        if not (isinstance(p, str) and p.startswith("/")):
            out.append("file check needs an absolute path")
    elif kind == "cmd":
        run = check.get("run")
        if not (isinstance(run, str) and run.strip()):
            out.append("cmd check needs a run string")
        elif FORBIDDEN_CMD.search(run):
            out.append(f"cmd check must be harmless and read-only: {run!r}")
        ex = check.get("expect_exit", 0)
        if not (isinstance(ex, int) and not isinstance(ex, bool)):
            out.append("expect_exit must be an integer")
    elif kind == "unit":
        if not (isinstance(check.get("name"), str) and check["name"].strip()):
            out.append("unit check needs a name")
        if check.get("state") not in UNIT_STATES:
            out.append(f"unit state must be one of {', '.join(UNIT_STATES)}")
        if "user" in check and not isinstance(check["user"], bool):
            out.append("unit user must be true or false")
    else:
        for k in ("crate", "filter"):
            if not (isinstance(check.get(k), str) and check[k].strip()):
                out.append(f"test check needs a {k}")
    return out


def validate_entry(entry):
    """Problems with one OS row's ready and checks fields (empty list when valid)."""
    eid = entry.get("id", "?")
    ready = entry.get("ready")
    checks = entry.get("checks")
    problems = []
    if ready is not None and not isinstance(ready, bool):
        problems.append(f"{eid}: ready must be true or false")
    if checks is not None:
        if not isinstance(checks, list):
            problems.append(f"{eid}: checks must be a list")
        else:
            problems += [f"{eid}: {p}" for c in checks for p in check_problems(c)]
    if ready is True:
        if not checks:
            problems.append(f"{eid}: ready row has no checks")
        if entry.get("status") in SKIP_STATUS:
            problems.append(f"{eid}: ready row has status {entry.get('status')}")
    return problems


# --- plan ----------------------------------------------------------------------------------------------------------

def load_shards(os_dir=OS_DIR):
    """(label 'category/shard', path, doc) for every OS shard file, sorted."""
    out = []
    if not os.path.isdir(os_dir):
        return out
    for cat in sorted(os.listdir(os_dir)):
        d = os.path.join(os_dir, cat)
        if not os.path.isdir(d):
            continue
        for f in sorted(os.listdir(d)):
            if f.endswith(".json"):
                p = os.path.join(d, f)
                out.append((f"{cat}/{f[:-5]}", p, load_doc(p)))
    return out


def load_doc(path):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def dump_doc(path, doc):
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(doc, fh, indent=1, ensure_ascii=False)
        fh.write("\n")


def build_plan(os_dir=OS_DIR):
    """(plan, problems). Plan: rows [{shard, id, name, checks}] and tests [{crate, filter}] (unique, sorted)."""
    rows, tests, problems = [], set(), []
    for label, _, doc in load_shards(os_dir):
        for e in doc.get("entries", []):
            found = validate_entry(e)
            problems += found
            if found or e.get("ready") is not True or e.get("status") in SKIP_STATUS:
                continue
            rows.append({"shard": label, "id": e["id"], "name": e.get("name", ""), "checks": e["checks"]})
            tests |= {(c["crate"], c["filter"]) for c in e["checks"] if c["kind"] == "test"}
    plan = {"rows": rows, "tests": [{"crate": c, "filter": f} for c, f in sorted(tests)]}
    return plan, problems


# --- run -----------------------------------------------------------------------------------------------------------

class Target:
    """'local' or 'ssh:HOST'. Only file, cmd and unit checks go to the target; test checks always run in the repo."""

    def __init__(self, spec):
        if spec == "local":
            self.host = None
        elif spec.startswith("ssh:") and len(spec) > 4:
            self.host = spec[4:]
        else:
            raise SystemExit(f"bad --target {spec!r}: use local or ssh:HOST")
        self.spec = spec

    def argv(self, argv):
        if self.host is None:
            return list(argv)
        return ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", self.host, shlex.join(argv)]


def _run(argv, timeout, cwd=None, env=None):
    """(returncode or None, stdout, stderr). None when the command timed out or could not start."""
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, cwd=cwd, env=env)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return None, "", f"timed out after {timeout} s"
    except OSError as e:
        return None, "", str(e)


def last_line(text):
    lines = [ln for ln in (text or "").splitlines() if ln.strip()]
    return lines[-1].strip() if lines else ""


def parse_libtest(stdout):
    """{test name: 'ok' | 'FAILED' | 'ignored'} from libtest output."""
    return {m.group(1): m.group(2) for m in LIBTEST_RE.finditer(stdout or "")}


def judge_filters(filters, outcomes, rc=0, detail=""):
    """{filter: (passed, reason)}. A filter passes when at least one test it matches ran and none failed; a filter
    that runs 0 tests fails (a typo or a removed module must not pass silently)."""
    out = {}
    for f in filters:
        ran = {n: s for n, s in outcomes.items() if f in n and s in ("ok", "FAILED")}
        failed = sorted(n for n, s in ran.items() if s == "FAILED")
        if failed:
            out[f] = (False, f"{len(failed)} failed, first {failed[0]}")
        elif not ran:
            why = "filter ran 0 tests"
            if rc:
                why += f" (cargo exit {rc}: {detail})" if detail else f" (cargo exit {rc})"
            out[f] = (False, why)
        else:
            out[f] = (True, f"{len(ran)} passed")
    return out


CACHE = os.path.join(os.path.expanduser("~"), ".cache", "bifrost-parity", "os-verify-tests.json")


def test_executables(repo, crates):
    """({crate: [(executable, package dir)]}, error) from one `cargo test --no-run` over every crate together, so the
    features resolve as in the lead's build and nothing already built is compiled again."""
    root = os.path.join(repo, "springboard")
    argv = ["cargo", "test", "--offline", "--profile", "fast", "--no-run", "--message-format=json"]
    for c in sorted(crates):
        argv += ["-p", c]
    rc, out, err = _run(argv, timeout=7200, cwd=root)
    if rc != 0:
        return {}, f"cargo test --no-run exit {rc}: {last_line(err)}"
    return executables_from_messages(out, crates), ""


def executables_from_messages(stdout, crates):
    """{crate: [(executable, package dir)]} from cargo's JSON messages: the test-profile artifacts of `crates`."""
    exes = {}
    for line in (stdout or "").splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") != "compiler-artifact" or not m.get("executable") or not (m.get("profile") or {}).get("test"):
            continue
        pkg_dir = os.path.dirname(m.get("manifest_path", ""))
        name = package_name(m.get("package_id", ""), pkg_dir)
        if name in crates:
            exes.setdefault(name, []).append((m["executable"], pkg_dir))
    return exes


def package_name(package_id, pkg_dir):
    """A package's name from its cargo package id ('path+file:///x/sdk#bifrost-app@0.1.0', or '...#0.1.0' when the
    name is the directory's), falling back to the manifest's name."""
    frag = package_id.rsplit("#", 1)[-1] if "#" in package_id else ""
    if "@" in frag:
        return frag.split("@", 1)[0]
    try:
        with open(os.path.join(pkg_dir, "Cargo.toml"), encoding="utf-8") as fh:
            m = re.search(r'^\[package\][^\[]*?^name\s*=\s*"([^"]+)"', fh.read(), re.M | re.S)
        return m.group(1) if m else os.path.basename(pkg_dir)
    except OSError:
        return os.path.basename(pkg_dir)


def file_digest(path):
    import hashlib
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def load_cache(path=CACHE):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return {}


def save_cache(cache, path=CACHE):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(cache, fh)
    os.replace(tmp, path)


def run_executables(exes, filters, cache):
    """{filter: (passed, reason)} for a crate's test executables. Each executable runs once with the filters it has no
    cached outcome for; passing outcomes are cached by the executable's SHA-256, so an unchanged binary's passing tests
    never run again (failures are retried)."""
    outcomes, problems = {}, []
    for exe, pkg_dir in exes:
        key = file_digest(exe)
        seen = cache.setdefault(key, {})
        todo = [f for f in filters if f not in seen]
        if todo:
            env = dict(os.environ, CARGO_MANIFEST_DIR=pkg_dir)
            rc, out, err = _run([exe] + todo, timeout=3600, cwd=pkg_dir, env=env)
            got = parse_libtest(out)
            if rc not in (0, 101):
                problems.append(f"{os.path.basename(exe)} exit {rc}: {last_line(err)}")
            else:
                # only a filter that passed is cached: a failure, crash or timeout is tried again next time
                for f in todo:
                    mine = {n: st for n, st in got.items() if f in n}
                    if mine and "FAILED" not in mine.values():
                        seen[f] = mine
            for f in todo:
                outcomes.update({n: st for n, st in got.items() if f in n})
        for f in filters:
            outcomes.update(seen.get(f, {}))
    return judge_filters(filters, outcomes, 1 if problems else 0, "; ".join(problems))


def exec_check(target, check):
    """(passed, reason) for a file, cmd or unit check on the target."""
    kind = check["kind"]
    if kind == "file":
        rc, _, err = _run(target.argv(["test", "-e", check["path"]]), timeout=30)
        ok = rc == 0
        return ok, "" if ok else f"{check['path']} is missing" + (f" ({err.strip()})" if err.strip() else "")
    if kind == "cmd":
        inner = ["sh", "-c", check["run"]]
        if target.host is None:
            argv, limit = inner, CMD_TIMEOUT
        else:
            argv, limit = ["timeout", str(CMD_TIMEOUT)] + inner, CMD_TIMEOUT + 20
        rc, out, err = _run(target.argv(argv), timeout=limit)
        want = check.get("expect_exit", 0)
        if rc is None:
            return False, f"timed out after {CMD_TIMEOUT} s"
        if rc == 124 and target.host is not None:
            return False, f"timed out after {CMD_TIMEOUT} s"
        if rc == want:
            return True, ""
        return False, f"exit {rc}, expected {want}" + (f": {last_line(err)}" if last_line(err) else "")
    if kind == "unit":
        name, state = check["name"], check["state"]
        verb = "is-enabled" if state == "enabled" else "is-active"
        argv = ["systemctl"] + (["--user"] if check.get("user") else []) + [verb, name]
        rc, out, err = _run(target.argv(argv), timeout=30)
        got = (out or "").strip() or "unknown"
        if rc is None:
            return False, f"systemctl did not answer ({err})"
        ok = got == state
        return ok, "" if ok else f"{name} is {got}, expected {state}"
    return False, f"unsupported check kind {kind!r}"


def describe(check):
    kind = check["kind"]
    if kind == "file":
        return check["path"]
    if kind == "cmd":
        return check["run"]
    if kind == "unit":
        return check["name"]
    return f"{check['crate']}::{check['filter']}"


def run_plan(plan, target, repo):
    """Results dict: rows with pass/checks. Test checks: one `cargo test --no-run` over all crates (no rebuild of what is
    built), then each test executable runs with its filters; outcomes are cached per executable hash (CACHE)."""
    crates = {}
    for t in plan.get("tests", []):
        crates.setdefault(t["crate"], []).append(t["filter"])
    test_results = {}
    if crates:
        exes, err = test_executables(repo, set(crates))
        cache = load_cache()
        for crate, filters in sorted(crates.items()):
            if not exes.get(crate):
                why = err or "no test executable for the crate"
                test_results.update({(crate, f): (False, why) for f in filters})
                continue
            for f, res in run_executables(exes[crate], filters, cache).items():
                test_results[(crate, f)] = res
            save_cache(cache)
    rows = []
    for row in plan["rows"]:
        checks = []
        for c in row["checks"]:
            if c["kind"] == "test":
                ok, why = test_results.get((c["crate"], c["filter"]), (False, "not run"))
            else:
                ok, why = exec_check(target, c)
            checks.append({"check": c, "pass": ok, "reason": why})
        rows.append({"shard": row["shard"], "id": row["id"], "name": row.get("name", ""),
                     "pass": all(x["pass"] for x in checks), "checks": checks})
    return {"target": target.spec, "repo": repo,
            "test_note": "test checks ran locally against the repo (host logic), not on the target",
            "rows": rows}


def failure_reason(row):
    return "; ".join(f"{x['check']['kind']} {describe(x['check'])}: {x['reason']}" for x in row["checks"] if not x["pass"])


# --- promote -------------------------------------------------------------------------------------------------------

def promote(results, os_dir, version, target_name="vm", dry_run=False):
    """Rewrite the shard files from a results dict. Returns one line per row changed or not found."""
    docs, lines = {}, []
    for row in results["rows"]:
        path = os.path.join(os_dir, *row["shard"].split("/")) + ".json"
        if path not in docs:
            docs[path] = load_doc(path)
        entry = next((e for e in docs[path]["entries"] if e.get("id") == row["id"]), None)
        if entry is None:
            lines.append(f"? {row['shard']} {row['id']}: not in the shard")
            continue
        if row["pass"]:
            entry["status"] = "parity"
            entry.pop("ready", None)
            entry.pop("ready_failed", None)
            text = NOTE_TEXT.format(target=target_name, version=version)
            note = entry.get("note") or ""
            if text not in note:
                entry["note"] = (note + " " + text).strip()
            lines.append(f"+ {row['shard']} {row['id']}: parity")
        else:
            entry["ready"] = False
            entry["ready_failed"] = failure_reason(row)
            lines.append(f"- {row['shard']} {row['id']}: {entry['ready_failed']}")
    if not dry_run:
        for path, doc in docs.items():
            dump_doc(path, doc)
    return lines


# --- CLI -----------------------------------------------------------------------------------------------------------

def cmd_plan(a):
    plan, problems = build_plan(a.os_dir)
    for p in problems:
        print("FAIL " + p)
    if problems:
        return 1
    if a.out:
        with open(a.out, "w", encoding="utf-8") as fh:
            json.dump(plan, fh, indent=1, ensure_ascii=False)
            fh.write("\n")
    crates = {t["crate"] for t in plan["tests"]}
    print(f"{len(plan['rows'])} ready rows, {len(plan['tests'])} test filters in {len(crates)} crates"
          + (f" -> {a.out}" if a.out else ""))
    return 0


def cmd_run(a):
    target = Target(a.target)
    plan = load_doc(a.plan)
    if target.host is None and os.geteuid() == 0 and any(c["kind"] == "cmd" for r in plan["rows"] for c in r["checks"]):
        print("run os-verify as an unprivileged user: cmd checks run as the invoking user")
        return 1
    results = run_plan(plan, target, a.repo)
    failed = [r for r in results["rows"] if not r["pass"]]
    for r in results["rows"]:
        print(f"{'PASS' if r['pass'] else 'FAIL'} {r['shard']} {r['id']}" + ("" if r["pass"] else f": {failure_reason(r)}"))
    print(f"{len(results['rows']) - len(failed)} of {len(results['rows'])} rows passed ({results['test_note']})")
    if a.out:
        with open(a.out, "w", encoding="utf-8") as fh:
            json.dump(results, fh, indent=1, ensure_ascii=False)
            fh.write("\n")
    return 1 if failed else 0


def cmd_promote(a):
    results = load_doc(a.results)
    lines = promote(results, a.os_dir, a.version, a.target_name, a.dry_run)
    for ln in lines:
        print(ln)
    passed = sum(1 for ln in lines if ln.startswith("+"))
    print(f"{passed} rows promoted to parity, {len(lines) - passed} not promoted"
          + (" (dry run, nothing written)" if a.dry_run else ""))
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(prog="bifrost pc parity os-verify", description=__doc__.split("\n\n")[0])
    ap.add_argument("--os-dir", default=OS_DIR, help=argparse.SUPPRESS)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("plan")
    p.add_argument("--out")
    r = sub.add_parser("run")
    r.add_argument("plan")
    r.add_argument("--target", required=True, help="local or ssh:HOST")
    r.add_argument("--repo", default=REPO, help="repo root for the test checks (default: this checkout)")
    r.add_argument("--out")
    m = sub.add_parser("promote")
    m.add_argument("results")
    m.add_argument("--version", required=True)
    m.add_argument("--target-name", default="vm")
    m.add_argument("--dry-run", action="store_true")
    a = ap.parse_args(argv)
    return {"plan": cmd_plan, "run": cmd_run, "promote": cmd_promote}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
