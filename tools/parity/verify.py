#!/usr/bin/env python3
"""bifrost pc parity verify: promote `partial` rows to `parity` on evidence from Midget (the lab machine).

  verify plan [--group G] [--slug S] [--out plan.json]
        resolve each partial row's cited repo paths to a crate and its #[test] fns; class: logic (tests, pure logic),
        ui-headless (drawing code whose cited tests render offscreen and need no compositor), ui / hardware
        (needs a screenshot or manual check), untested (no test found)
  verify run PLAN.json --version X [--out results.json]
        cargo test --no-run per crate, scp the test binaries to midget:/var/tmp/parity-verify/, run the rows' tests there
  verify closures RESULTS.json --version X [--out closures.json]
        apply closures for rows whose tests all passed; needs-manual.md (next to --out) lists the rest by framework

Only `logic` and `ui-headless` rows can be promoted; ui, hardware and untested rows never are.
"""
import argparse
import collections
import json
import os
import re
import subprocess

import parity

SRC_ROOT = parity.ROOT
WORKSPACE = os.path.join(SRC_ROOT, "springboard")
REMOTE_DIR = "/var/tmp/parity-verify"
# The lab machine over the lab network with the lab key (tools/midget.py); MIDGET_HOST overrides the address.
_KEY = os.path.join(parity.ROOT, "out/lab/id_ed25519")
SSH = ["ssh", "-i", _KEY, "-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no", "-o", "BatchMode=yes",
       "-o", "ConnectTimeout=5", "-o", "LogLevel=ERROR", "root@" + os.environ.get("MIDGET_HOST", "10.99.0.200")]
HARDWARE = ("audio", "alsa", "drm", "kms", "evdev", "input", "bluetooth", "camera", "usb", "gpu", "sensor", "display",
            "pcm", "midi", "haptic", "printer", "scanner", "wayland", "libinput", "v4l")
UI = ("draw", "render", "paint", "canvas", "window", "wm", "view", "layer", "gfx", "shell", "widget", "chrome", "scene",
      "screen", "compositor", "animation", "theme")
# A ui row whose cited tests only draw to memory is `ui-headless`; any compositor marker keeps it a manual `ui` row.
HEADLESS_MARKERS = ("offscreen", "raster", "snapshot", "pixel", "headless")
COMPOSITOR_MARKERS = ("wl_", "wayland", "compositor", "xdg_", "winit", "window_server", "windowserver", "layer_shell",
                      "libinput", "drm", "kms")
PROMOTABLE = ("logic", "ui-headless")
CLASSES = ("logic", "ui-headless", "ui", "hardware", "untested")
MOD_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*\{")
TEST_ATTR_RE = re.compile(r"#\[(?:\w+::)?test\b")


def read(path):
    try:
        return open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return ""


def cited_paths(bifrost):
    """Repo paths cited in a row's `bifrost` field (`path::symbol` and prose are fine), duplicates dropped."""
    paths = (p.rstrip(".,;)") for p in parity.PATH_RE.findall(bifrost or ""))
    return list(dict.fromkeys(paths))


def crate_of(path):
    """(crate dir relative to the repo, package name) of the nearest Cargo.toml above `path`, else None."""
    d = os.path.join(SRC_ROOT, path)
    if not os.path.isdir(d):
        d = os.path.dirname(d)
    while d.startswith(SRC_ROOT) and d != SRC_ROOT:
        m = re.search(r'^\[package\].*?^name\s*=\s*"([^"]+)"', read(os.path.join(d, "Cargo.toml")), re.S | re.M)
        if m:
            return os.path.relpath(d, SRC_ROOT), m.group(1)
        d = os.path.dirname(d)
    return None


def module_path(path, crate_dir):
    """Module path of a source file inside its crate: ui/src/a/b.rs -> ['a', 'b']; lib.rs, main.rs, mod.rs add none."""
    rel = os.path.relpath(path, os.path.join(SRC_ROOT, crate_dir, "src"))
    if rel.startswith("..") or not rel.endswith(".rs"):
        return []
    return [p for p in rel[:-3].split(os.sep) if p not in ("lib", "main", "mod")]


def tests_in(source):
    """Names of the #[test] fns in `source`, prefixed by their inline module chain (e.g. 'tests::adds')."""
    stack, depth, pending, out = [], 0, False, []
    for line in source.splitlines():
        m = MOD_RE.match(line)
        if m:
            stack.append((m.group(1), depth))
        pending = pending or bool(TEST_ATTR_RE.search(line))
        f = re.search(r"\bfn\s+(\w+)", line)
        if pending and f:
            out.append("::".join([s for s, _ in stack] + [f.group(1)]))
            pending = False
        depth += line.count("{") - line.count("}")
        while stack and depth <= stack[-1][1]:
            stack.pop()
    return out


def rust_files(path):
    full = os.path.join(SRC_ROOT, path)
    if os.path.isdir(full):
        return sorted(os.path.join(r, f) for r, _, fs in os.walk(full) for f in fs if f.endswith(".rs"))
    return [full] if full.endswith(".rs") and os.path.exists(full) else []


def integration_tests(crate_dir, crate_name, mod):
    """{test target (file stem): test names} for the crate's tests/*.rs that use module `mod` (list of segments)."""
    if not mod:
        return {}
    want = re.compile(r"\b%s::%s\b" % (crate_name.replace("-", "_"), "::".join(mod)))
    tdir = os.path.join(SRC_ROOT, crate_dir, "tests")
    out = {}
    for f in sorted(os.listdir(tdir)) if os.path.isdir(tdir) else []:
        src = read(os.path.join(tdir, f))
        if f.endswith(".rs") and want.search(src) and tests_in(src):
            out[f[:-3]] = tests_in(src)
    return out


def test_code(source, whole=False):
    """The test part of a source: all of an integration test file, else what follows the first #[cfg(test)]."""
    if whole:
        return source
    i = source.find("#[cfg(test)]")
    return source[i:] if i >= 0 else ""


def is_headless(test_sources):
    """True when the tests render to memory (a headless marker) and use no compositor or window server symbol."""
    text = " ".join(test_sources).lower()
    return any(m in text for m in HEADLESS_MARKERS) and not any(m in text for m in COMPOSITOR_MARKERS)


def classify(paths, sources, has_tests, test_sources=()):
    """hardware or ui when the cited code touches devices or drawing (by path and `use` lines), else logic/untested.
    A ui row with tests that only render offscreen is ui-headless."""
    uses = [l.lower() for s in sources for l in s.splitlines() if l.lstrip().startswith("use ")]
    text = " ".join(paths).lower() + " " + " ".join(uses)
    for cls, words in (("hardware", HARDWARE), ("ui", UI)):
        if any(w in text for w in words):
            return "ui-headless" if cls == "ui" and has_tests and is_headless(test_sources) else cls
    return "logic" if has_tests else "untested"


def plan_row(slug, row):
    """Plan entry for one partial row: {slug, feature, crate, test_filters, class}. A filter is 'target:test name',
    target being 'lib' (unit tests) or an integration test file stem."""
    paths = cited_paths(row.get("bifrost"))
    found = [(p, crate_of(p)) for p in paths if rust_files(p)]
    crate = next((c for _, c in found if c), None)
    filters, sources, test_sources = [], [], []
    for p, c in found:
        if c != crate or not c:
            continue
        for f in rust_files(p):
            src = read(f)
            sources.append(src)
            test_sources.append(test_code(src))
            mod = module_path(f, c[0])
            filters += ["lib:" + "::".join(mod + [t]) for t in tests_in(src)]
            for stem, names in integration_tests(c[0], c[1], mod).items():
                test_sources.append(test_code(read(os.path.join(SRC_ROOT, c[0], "tests", stem + ".rs")), whole=True))
                filters += [f"{stem}:{n}" for n in names]
    filters = list(dict.fromkeys(filters))
    return {"slug": slug, "feature": row.get("feature", ""), "crate": crate[1] if crate else None,
            "test_filters": filters, "class": classify(paths, sources, bool(filters), test_sources)}


def summarize(rows, applicable):
    counts = collections.Counter(r["class"] for r in rows)
    gain = 100 * 0.5 * sum(counts[c] for c in PROMOTABLE) / applicable if applicable else 0.0
    return {"rows": len(rows), "classes": {c: counts[c] for c in CLASSES},
            "coverage_gain_pct": round(gain, 2)}


def build_plan(entries, group=None, slug=None):
    rows = []
    for e in entries:
        if (group and e["group"].lower() != group.lower()) or (slug and slug not in (e["slug"], e.get("index_slug"))):
            continue
        rows += [plan_row(e["slug"], r) for r in e["rows"] if r["status"] == "partial"]
    return {"summary": summarize(rows, sum(e["applicable"] for e in entries)), "rows": rows}


def cmd_plan(a):
    entries, _ = parity.load_entries()
    plan = build_plan(entries, a.group, a.slug)
    json.dump(plan, open(a.out, "w"), indent=2)
    print(json.dumps(plan["summary"], indent=2))
    print(f"wrote {a.out}")


# --- run: build, ship and run the test binaries on Midget ------------------------------------------------------------
def sh(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def test_binaries(crate):
    """{target: executable} of a crate's test binaries: 'lib' (unit tests) and integration tests by file stem."""
    r = sh(["cargo", "test", "--offline", "-p", crate, "--no-run", "--message-format=json"], cwd=WORKSPACE)
    if r.returncode:
        raise SystemExit(f"cargo test --no-run -p {crate} failed:\n{r.stderr[-2000:]}")
    out = {}
    for line in r.stdout.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") == "compiler-artifact" and m.get("profile", {}).get("test") and m.get("executable"):
            kind = m["target"]["kind"][0]
            out[m["target"]["name"] if kind == "test" else "lib" if kind != "bin" else "bin"] = m["executable"]
    return out


def resolve(name, listed):
    """The listed test a plan filter means: exact, else the one listed test whose path ends with it."""
    if name in listed:
        return name
    hits = [t for t in listed if t.endswith("::" + name) or name.endswith("::" + t)]
    return hits[0] if len(hits) == 1 else None


def parse_results(stdout):
    """{test name: passed} from libtest lines `test name ... ok|FAILED`."""
    return {m.group(1): m.group(2) == "ok" for m in re.finditer(r"^test (\S+) \.\.\. (ok|FAILED)", stdout, re.M)}


def run_on_midget(exe, names, remote_name):
    """Copy `exe` to Midget and run exactly `names` with 4 threads; {name: passed}."""
    # ssh + cat, not scp: Midget has no sftp server
    with open(exe, "rb") as f:
        copied = subprocess.run(SSH + [f"mkdir -p {REMOTE_DIR} && cat > {REMOTE_DIR}/{remote_name} && chmod +x {REMOTE_DIR}/{remote_name}"],
                                stdin=f, capture_output=True)
    if copied.returncode:
        raise SystemExit("cannot reach midget over ssh")
    # batches keep the remote command under the argument-length limit
    results = {}
    for i in range(0, len(names), 200):
        quoted = " ".join("'%s'" % n for n in names[i:i + 200])
        results.update(parse_results(sh(SSH + [f"cd {REMOTE_DIR} && ./{remote_name} --exact --test-threads 4 {quoted}"]).stdout))
    return results


def row_result(row, outcome):
    """outcome {filter: True/False/None}; the row passes only when it has tests and every one ran and passed."""
    failed = [f for f, ok in outcome.items() if not ok]
    return dict(row, status="pass" if outcome and not failed else "fail", failed=failed)


def run_crate(crate, rows):
    """{(target, name): True/False/None} for every filter of `rows` (None: not found or not run)."""
    bins = test_binaries(crate)
    wanted = collections.defaultdict(set)
    for r in rows:
        for f in r["test_filters"]:
            target, name = f.split(":", 1)
            wanted[target].add(name)
    outcome = {}
    for target, names in sorted(wanted.items()):
        exe = bins.get(target)
        out = sh([exe, "--list"]).stdout if exe else ""
        listed = [l[:-len(": test")] for l in out.splitlines() if l.endswith(": test")]
        resolved = {n: resolve(n, listed) for n in names}
        to_run = sorted(v for v in resolved.values() if v)
        print(f"    {target}: {len(to_run)}/{len(names)} tests found, running on Midget", flush=True)
        ran = run_on_midget(exe, to_run, f"{crate}-{target}") if to_run else {}
        outcome.update({(target, n): ran.get(v) if v else None for n, v in resolved.items()})
    return outcome


def cmd_run(a):
    plan = json.load(open(a.plan))
    by_crate = collections.defaultdict(list)
    for r in plan["rows"]:
        if r["class"] in PROMOTABLE:
            by_crate[r["crate"]].append(r)
    done = {}
    for n, (crate, rows) in enumerate(sorted(by_crate.items()), 1):
        print(f"[{n}/{len(by_crate)}] {crate}: {len(rows)} rows", flush=True)
        outcome = run_crate(crate, rows)
        for r in rows:
            done[(r["slug"], r["feature"])] = row_result(r, {f: outcome[tuple(f.split(":", 1))] for f in r["test_filters"]})
        print(f"    rows passed: {sum(done[(r['slug'], r['feature'])]['status'] == 'pass' for r in rows)}/{len(rows)}", flush=True)
    out = [done.get((r["slug"], r["feature"]), dict(r, status="skipped")) for r in plan["rows"]]
    json.dump({"version": a.version, "rows": out}, open(a.out, "w"), indent=2)
    print(f"wrote {a.out}: {sum(r['status'] == 'pass' for r in out)} pass, {sum(r['status'] == 'fail' for r in out)} fail")


# --- closures --------------------------------------------------------------------------------------------------------
def make_closures(results, rows_by_key, version):
    """`parity apply` closures for logic and ui-headless rows whose tests all passed on Midget. apply prepends the closure's note to the
    row's own, so the note carries only the verification and the old note is kept by apply."""
    return [{"slug": r["slug"], "feature": r["feature"], "status": "parity",
             "bifrost": rows_by_key.get((r["slug"], r["feature"]), {}).get("bifrost", ""),
             "note": f"verified{' headless' if r['class'] == 'ui-headless' else ''} on Midget {version} by {len(r['test_filters'])} tests."}
            for r in results["rows"] if r["class"] in PROMOTABLE and r["status"] == "pass" and r["test_filters"]]


def manual_report(results, names):
    """Markdown listing every row that was not promoted, grouped by framework."""
    groups = collections.defaultdict(list)
    for r in results["rows"]:
        if r["status"] != "pass":
            groups[names.get(r["slug"], r["slug"])].append(r)
    lines = ["# Parity rows needing manual verification", ""]
    for fw in sorted(groups):
        lines += [f"## {fw}", ""]
        lines += [f"- {r['feature']} ({r['class']}{', tests failed' if r['status'] == 'fail' else ''})" for r in groups[fw]]
        lines.append("")
    return "\n".join(lines)


def cmd_closures(a):
    results = json.load(open(a.results))
    entries, _ = parity.load_entries()
    rows_by_key = {(e["slug"], r.get("feature", "")): r for e in entries for r in e["rows"]}
    closures = make_closures(results, rows_by_key, a.version)
    json.dump(closures, open(a.out, "w"), indent=2)
    md = os.path.join(os.path.dirname(os.path.abspath(a.out)), "needs-manual.md")
    open(md, "w").write(manual_report(results, {e["slug"]: e["name"] for e in entries}))
    print(f"{len(closures)} closures -> {a.out}; manual list -> {md}")


def main(argv=None):
    p = argparse.ArgumentParser(prog="bifrost pc parity verify", description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    pl = sub.add_parser("plan")
    pl.add_argument("--group")
    pl.add_argument("--slug")
    pl.add_argument("--out", default="plan.json")
    ru = sub.add_parser("run")
    ru.add_argument("plan")
    ru.add_argument("--version", required=True)
    ru.add_argument("--out", default="results.json")
    cl = sub.add_parser("closures")
    cl.add_argument("results")
    cl.add_argument("--version", required=True)
    cl.add_argument("--out", default="closures.json")
    a = p.parse_args(argv)
    {"plan": cmd_plan, "run": cmd_run, "closures": cmd_closures}[a.cmd](a)


if __name__ == "__main__":
    main()
