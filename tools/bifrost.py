#!/usr/bin/env python3
"""bifrost: BifrOSt PC development CLI.

  bifrost pc control|app|publish|midget|cycle|install|boot|ssh   PC workflow (docs/bifrost-pc-workflow.md)
  bifrost pc acceptance [--quick] [--midget] [--bless]                    cargo tests (incl. per-app compliance), SDK coverage, app compliance, tool tests, app smoke, bench
  bifrost pc audio-bench [host|midget|compare] [--append]        audio latency benches (docs/audio-latency.md)
  bifrost pc names | browser-smoke                                name guard; Browser smoke on Midget via the MCP
  bifrost pc smoke|adoption|gallery                              host app previews and Golden Gate SDK adoption (docs/golden-gate-sdk-adoption.md)
  bifrost pc parity status|coverage|show|gaps|check|build|apps|search|...   reference SDK vs BifrOSt parity matrix (docs/parity-matrix/)
      subcommands: os-status, import, db-export, refresh-index, related, themes, apply, verify os-cli|plan|run|closures
      --platform macos|ipados  (status, coverage, show, gaps, check, refresh-index; default macos; ipados is the tablet
      platform, docs/parity-matrix/ipados/, e.g. `bifrost pc parity check --platform ipados`)
  bifrost pc parity os-verify plan|run|promote                    run the OS rows' checks (ready rows), promote the passing ones (tools/parity/os_verify.py)
  bifrost pc os-inventory [--index TSV]                           OS layer of the matrix from the reference OS 27 file index (parity os-status)
  bifrost pc ref extract REGEX... [--out DIR] | ref index         stream files out of the LZRAVEN reference OS 27 reference (clean-room reading)
  bifrost pc shot|tap|rtap|swipe|key|type|wait-change            computer use on the headed x86 VM
  bifrost site build|check|doctest|shot|ship                      bifrost.olibuijr.com (~/Projects/bifrost-site/tools/site.py)
  bifrost e2e [--list] [scenario ...]                            end-to-end scenarios on the x86 VM
"""
import argparse
import os
import subprocess
import sys


def cmd_e2e(a):
    """Scenario runner against the Arch x86 VM (tools/e2e, docs/e2e.md)."""
    sys.exit(subprocess.run([sys.executable, os.path.join(os.path.dirname(os.path.realpath(__file__)), "e2e", "run.py")] + a.args).returncode)


# SDK API coverage floor (tools/check-sdk-coverage.py); raise it as coverage grows, never lower it.
SDK_COVERAGE_MIN = 79


def acceptance(rest):
    """One acceptance run for a batch: cargo tests, SDK coverage, tools unit tests, app smoke, host bench.

    Stops at the first failing step and prints a summary table with timings.
    Flags: --quick skips the app smoke and bench; --midget also runs the Midget and hardware unit tests and the
    Midget budgets (docs/budgets.toml, needs the machine);
    --bless rewrites the SDK snapshot goldens (sdk/tests/snapshots, review and commit them).
    SDK API coverage must stay at or above SDK_COVERAGE_MIN.
    App compliance (static ratchet, tools/check-app-compliance.py) must not get worse than its baseline; the runtime
    half is each app's tests/compliance.rs, run by the cargo test step (workspace members include every app crate).
    """
    import time
    here = os.path.dirname(os.path.realpath(__file__))
    root = os.path.dirname(here)
    quick, midget = "--quick" in rest, "--midget" in rest
    if "--bless" in rest:
        os.environ["BIFROST_BLESS"] = "1"
    # unit tests: midget/hardware ones need a machine or fake one, so they run only with --midget
    stems = sorted(f[:-3] for f in os.listdir(here) if f.startswith("test_") and f.endswith(".py"))
    if not midget:
        stems = [t for t in stems if "midget" not in t and "hardware" not in t]
    steps = [
        # owner rule: no reference-platform product names in code (tools/check-names.py, allowlist check-names-allow.txt)
        ("names guard", root, [sys.executable, os.path.join(here, "check-names.py")], False),
        ("cargo test", os.path.join(root, "springboard"), ["cargo", "test", "--offline", "--profile", "fast", "--workspace", "--no-fail-fast"], False),
        ("sdk coverage", root, [sys.executable, os.path.join(here, "check-sdk-coverage.py"), "--min", str(SDK_COVERAGE_MIN)], False),
        # static app gate, ratchet mode: an app may not get worse than docs/app-compliance-baseline.json (docs/app-compliance.md)
        ("app compliance", root, [sys.executable, os.path.join(here, "check-app-compliance.py")], False),
        ("tools unit tests", here, [sys.executable, "-m", "unittest"] + stems, False),
    ]
    if midget:
        # Midget idle and footprint against docs/budgets.toml (Snow Leopard); exit 3 lists each key over budget
        steps.append(("midget budgets", root, [sys.executable, os.path.join(here, "bench.py"), "--midget", "--no-record"], False))
    steps += [
        ("app smoke", root, [sys.executable, os.path.join(here, "pc-app-smoke.py")], True),
        ("host bench", root, [sys.executable, os.path.join(here, "bench.py"), "--no-record"], True),
    ]
    rows, failed = [], False
    for name, cwd, cmd, skippable in steps:
        if quick and skippable:
            rows.append((name, "skip", 0.0, "--quick"))
            continue
        t0 = time.monotonic()
        code = subprocess.run(cmd, cwd=cwd).returncode
        dt = time.monotonic() - t0
        rows.append((name, "pass" if code == 0 else f"FAIL ({code})", dt, ""))
        if code:
            failed = True
            break
    if failed:
        rows += [(n, "not run", 0.0, "earlier step failed") for n, *_ in steps[len(rows):]]
    print("\nacceptance summary")
    print(f"  {'step':<18}{'result':<14}{'seconds':>9}  note")
    for name, result, dt, note in rows:
        print(f"  {name:<18}{result:<14}{dt:>9.1f}  {note}")
    if not failed and not midget:
        print("  (midget and hardware unit tests skipped; add --midget to include them)")
    return 1 if failed else 0


def browser_engine(rest):
    """Builds Servo engine VERSION (tools/build-webview-engine.sh) and pins it in the Browser app."""
    import json, re
    if len(rest) != 1:
        return "usage: bifrost pc browser-engine VERSION   (for example 0.7.0-bifrost.8)"
    ver, root = rest[0], os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
    if subprocess.run([os.path.join(root, "tools/build-webview-engine.sh"), ver]).returncode:
        return 1
    tar = os.path.join(root, f"out/browser-engine/servo-{ver}.tar.gz")
    sha = subprocess.run(["sha256sum", tar], capture_output=True, text=True, check=True).stdout.split()[0]
    app = os.path.join(root, "springboard/apps/browser")
    ej = os.path.join(app, "engine.json")
    e = json.load(open(ej))
    e["url"] = e["url"].replace(e["version"], ver)
    e.update(version=ver, sha256=sha)
    open(ej, "w").write(json.dumps(e, indent=2) + "\n")
    mr = os.path.join(app, "src/main.rs")
    src, n = re.subn(r'const ENGINE: &str = "servo-[^"]*";', f'const ENGINE: &str = "servo-{ver}-{sha[:12]}";', open(mr).read())
    if n != 1:
        return "ENGINE constant not found in src/main.rs"
    open(mr, "w").write(src)
    print(f"pinned servo-{ver}-{sha[:12]}; bump the app version and CHANGELOG, then bifrost pc app publish browser")
    return 0


def cmd_pc(a):
    """BifrOSt PC VM: autonomous reinstall cycles and computer-use primitives on the headed x86 VM (docs/bifrost-pc-workflow.md)."""
    here = os.path.dirname(os.path.realpath(__file__))
    qmp = [sys.executable, os.path.join(here, "arch-vm-qmp.py"), os.path.join(os.path.dirname(here), "out/arch-vm/installer-qmp.sock"), "1280", "720"]
    op, rest = (a.args or ["help"])[0], (a.args or ["help"])[1:]
    if op == "app":
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "pc-app.py")] + rest).returncode)
    if op == "browser-engine":
        # browser-engine VERSION: build the Servo engine, then pin it in engine.json and ENGINE (src/main.rs)
        sys.exit(browser_engine(rest))
    if op == "names":
        # trademarked platform and product names in code (tools/check-names.py, allowlist tools/check-names-allow.txt)
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "check-names.py")] + rest).returncode)
    if op == "browser-smoke":
        # drive BifrOSt Browser on Midget through the MCP browser tool: PASS/FAIL per step
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "browser-smoke.py")] + rest).returncode)
    if op == "acceptance":
        # one batch acceptance run: acceptance [--quick] [--midget] [--bless]
        sys.exit(acceptance(rest))
    if op == "smoke":
        # host preview of real app binaries: smoke APP... [--light] [--sizes WxH,...] [--scale PX_PER_PT] [--out DIR]
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "pc-app-smoke.py")] + rest).returncode)
    if op == "parity":
        # reference SDK vs BifrOSt parity matrix: status|os-status|coverage|show|gaps|check|build|apps|import|db-export|refresh-index
        #   |search|related|themes|apply|verify|os-verify (the full list is in the docstring of tools/parity/parity.py)
        # --platform macos|ipados selects the reference platform for status, coverage, show, gaps, check and refresh-index
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "parity", "parity.py")] + rest).returncode)
    if op == "os-inventory":
        # the OS layer of the parity matrix from the reference file index: os-inventory [--index TSV] (default: the
        # reference OS 27 index ~/Projects/macos-ref/extract/ref27-index.tsv); writes docs/parity-matrix/os/<category>/*.json
        index = os.path.expanduser("~/Projects/macos-ref/extract/ref27-index.tsv")
        args = rest if "--index" in rest else ["--index", index] + rest
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "parity", "os_inventory.py")] + args).returncode)
    if op == "ref":
        # the reference OS 27 reference (LZRAVEN OTA payload, decoded with liblzraven): ref extract REGEX... [--out DIR]
        # streams the payload and writes only matching paths (default out: ~/Projects/macos-ref/extract/ref27),
        # ref index rebuilds the file index; clean-room: read and measure, never copy into the repo
        refdir = os.path.expanduser("~/Projects/macos-ref")
        tool = os.path.join(refdir, "tools", "aa_stream.py")
        payload = os.path.join(refdir, "extract/system/26A434__MacOS/AssetData/payloadv2")
        if not rest or rest[0] not in ("extract", "index"):
            sys.exit("usage: bifrost pc ref extract REGEX... [--out DIR] | bifrost pc ref index")
        out = rest[rest.index("--out") + 1] if "--out" in rest else os.path.join(refdir, "extract/ref27")
        pats = [a for i, a in enumerate(rest[1:], 1) if a != "--out" and rest[i - 1] != "--out"]
        args = [sys.executable, "-I", tool, payload, out]
        if rest[0] == "index":
            args += ["--index", os.path.join(refdir, "extract/ref27-index.tsv")]
        args += [x for p in pats for x in ("--match", p)]
        sys.exit(subprocess.run(args).returncode)
    if op == "adoption":
        # how far each app has moved to the SDK kit: fixed dark colours left, widgets used, full-size window
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "sdk-adoption.py")] + rest).returncode)
    if op == "gallery":
        # every SDK widget in Light and Dark: gallery [OUT_DIR] writes gallery-light.png and gallery-dark.png
        out = os.path.abspath(rest[0] if rest else "/tmp/gallery")
        os.makedirs(out, exist_ok=True)
        r = subprocess.run(["cargo", "test", "--offline", "-q", "-p", "bifrost-app", "preview_sdk_gallery", "--", "--ignored"],
                           cwd=os.path.join(os.path.dirname(here), "springboard"), env={**os.environ, "PREVIEW_DIR": out})
        if r.returncode == 0:
            print(os.path.join(out, "gallery-light.png") + "\n" + os.path.join(out, "gallery-dark.png"))
        sys.exit(r.returncode)
    if op == "mcp":
        # agent access (docs/bifrost-mcp.md): mcp call TOOL [JSON] | bench [-n N] | eval [--only IDS] | push | deploy-app APP | token
        sub, more = (rest or ["help"])[0], (rest or ["help"])[1:]
        run = {"call": [os.path.join(here, "bifrost-mcp-ssh"), "call"], "bench": [sys.executable, os.path.join(here, "mcp-bench.py")],
               "eval": [sys.executable, "-u", os.path.join(here, "mcp-eval.py")], "push": [os.path.join(here, "mcp-push.sh")],
               "deploy-app": [sys.executable, os.path.join(here, "mcp-deploy-app.py")],
               "token": [sys.executable, os.path.join(here, "midget.py"), "ssh", "cat /var/lib/bifrost-mcp/token"]}.get(sub)
        if not run:
            sys.exit("usage: bifrost pc mcp call TOOL [JSON] | bench [-n N] [--baseline] | eval [--only IDS] [--record LABEL] | push | deploy-app APP [--skip-build] [--no-open] | token")
        sys.exit(subprocess.run(run + more).returncode)
    if op == "audio-bench":
        # audio-bench [host|midget|compare] [--append] [--quanta 16,32] [--bench NAME]: tools/audio-bench.py, docs/audio-latency.md
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "audio-bench.py")] + (rest or ["host"])).returncode)
    if op == "log":
        # Unified Logging on Midget (docs/milestone-unified-logging.md): log show|stream|config|subsystems [ARGS]
        import shlex
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "midget.py"), "ssh", "bifrost-log " + shlex.join(rest or ["show", "--last", "5m"])]).returncode)
    if op == "publish":
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "pc-publish.py")] + rest).returncode)
    if op == "upstream":
        # third-party prebuilt apps (upstream.toml): check / republish new upstream releases
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "upstream-watch.py")] + rest).returncode)
    if op == "control":
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "app-control.py")] + rest).returncode)
    if op == "midget":
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "midget.py")] + rest).returncode)
    if op == "cycle":
        sys.exit(subprocess.run([sys.executable, os.path.join(here, "vm-cycle.py")] + rest).returncode)
    if op in ("install", "boot", "ssh"):
        sys.exit(subprocess.run([os.path.join(here, "vm-install.sh"), op] + rest).returncode)
    if op in ("shot", "tap", "rtap", "swipe", "key", "type", "wait-change"):
        sys.exit(subprocess.run(qmp + [op] + rest).returncode)
    sys.exit("usage: bifrost pc log show|stream|config|subsystems [ARGS] | control list|info APP|methods APP|call APP METHOD [JSON]  (running PC apps over their control sockets) | app build|publish|info APP [--skip-build] | browser-engine VERSION | acceptance [--quick] [--midget] [--bless] | smoke APP... [--light] [--sizes WxH,...] [--scale N] | adoption [--json] | gallery [OUT_DIR] | parity status|coverage|show|gaps|check|build|apps|os-verify|... [--platform macos|ipados] | publish [--skip-build] | upstream [--dry-run] [--app NAME] | cycle [--loop N] [--headless] | midget status|state|ota|bios|kbench LABEL|kab|amt-info|amt-activate|power state|on|off|cycle|reset|wake|reboot [--netboot|--disk]|poweroff|bootnext|efi|hw|screen|logs|amt|ssh|appearance light|dark|auto|wallpaper ID|open-app ID|lock|tap-window APP X Y | install | boot | ssh CMD | shot OUT.png | tap X Y | rtap X Y | swipe X0 Y0 X1 Y1 [MS] | key K | type TEXT | wait-change [MS]  (guest pixels, 1280x720)")


def main():
    # passthrough subcommands take their own options (argparse REMAINDER cannot start with --option)
    if sys.argv[1:2] == ["site"]:
        sys.exit(subprocess.run([sys.executable, os.path.expanduser("~/Projects/bifrost-site/tools/site.py")] + sys.argv[2:]).returncode)
    if sys.argv[1:2] == ["e2e"]:
        return cmd_e2e(argparse.Namespace(args=sys.argv[2:]))
    p = argparse.ArgumentParser(prog="bifrost", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("e2e", help="end-to-end scenarios on the x86 VM: e2e [--list] [scenario ...] [--update-baseline] [--json]")
    s.add_argument("args", nargs=argparse.REMAINDER)
    s.set_defaults(fn=cmd_e2e)
    s = sub.add_parser("pc", help="BifrOSt PC: acceptance [--quick] [--midget] [--bless] (batch check), control list|info|methods|call (running apps over their control sockets), app build|publish|info, smoke APP [--light], adoption [--json], gallery [DIR], publish, upstream [--dry-run] [--app NAME], midget ..., cycle [--target vm|midget] [--loop N] (autonomous reinstall + report), install, boot, ssh, and computer use: shot tap rtap swipe key type wait-change")
    s.add_argument("args", nargs=argparse.REMAINDER)
    s.set_defaults(fn=cmd_pc)
    a = p.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
