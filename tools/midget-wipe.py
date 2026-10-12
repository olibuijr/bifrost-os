#!/usr/bin/env python3
"""Autonomous, guarded wipe-and-reinstall of midget (ThinkCentre M900, 192.168.20.140) from titan.

usage: tools/midget-wipe.py --disk /dev/nvme0n1 --confirm "WIPE 192.168.20.140 /dev/nvme0n1" [--dry-run] [--user NAME]
                            [--password-env VAR] [--timeout SECONDS]

Steps (each one logged to out/lab/wipes/<stamp>/log.txt, final report in report.json):
  1. preflight (read-only): midget answers on ssh, the NIC carries the expected MAC, exactly one erasable disk
     and it is --disk, --disk is not the root device, the confirm token matches exactly.
  2. if midget is installed: BootNext to PXE and reboot into the live lab ISO (never erase from an installed system).
  3. re-check the live system, stop the on-screen installer, run `BIFROST_AUTO=user:pw:DISK bifrost-installer`
     headless (plan.rs Erase mode; keeps the lab key when the live ISO carries lab.token).
  4. BootNext to the BifrOSt disk, reboot, wait for ssh, record springboard state.

--dry-run runs step 1 only and prints the plan. Nothing here touches a disk unless step 3 runs.
"""
import argparse, json, os, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import midget as m  # noqa: E402  (ssh/state/efibootmgr helpers live there)

REPO = m.REPO
EXPECT_MAC = m.MAC.lower()


def log(msg, fh):
    line = f"[{time.strftime('%H:%M:%S')}] {msg}"
    print(line, flush=True)
    fh.write(line + "\n")
    fh.flush()


def erasable_disks():
    out = m.ssh("lsblk -dnpo NAME,SIZE,TYPE,RM -b", 15)
    names = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) < 4 or parts[2] != "disk" or parts[3] != "0":
            continue
        if int(parts[1]) <= 8_000_000_000 or any(x in parts[0] for x in ("/loop", "/nbd", "/sr")):
            continue
        names.append(parts[0])
    return names


def preflight(disk, fh):
    """Read-only checks. Refusals are returned as problems; an installed system is expected here (the wipe reboots it first)."""
    problems = []
    st = m.state()
    log(f"state: {st}", fh)
    if st not in ("live", "installed"):
        problems.append(f"midget is {st}; it must answer on ssh (use `midget wake` or power it on by hand)")
        return st, problems
    macs = m.ssh("cat /sys/class/net/*/address 2>/dev/null", 10).lower().split()
    log(f"mac addresses: {' '.join(macs)}", fh)
    if EXPECT_MAC not in macs:
        problems.append(f"expected NIC {EXPECT_MAC} not present; a different machine may be answering at {m.HOST}")
    names = erasable_disks()
    log(f"erasable disks: {names}", fh)
    if names != [disk]:
        problems.append(f"erasable disks are {names}, expected exactly [{disk}]")
    if st == "live":
        root = m.ssh("findmnt -no SOURCE / 2>/dev/null", 10).strip()
        log(f"root source: {root}", fh)
        if root.startswith(disk):
            problems.append(f"{disk} is the live system's root device; refusing")
    else:
        log("installed: the target disk holds this system; it will be rebooted to the live ISO before erasing", fh)
    rec = m.ssh("bifrost-update-guard status 2>&1 | head -3", 20).strip()
    log(f"update guard: {rec.replace(chr(10), ' | ')}", fh)
    return st, problems


def wait_live(timeout):
    return m.wait(lambda: m.state() == "live", timeout, 2)


def run(a):
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_dir = os.path.join(REPO, "out/lab/wipes", stamp)
    os.makedirs(out_dir, exist_ok=True)
    fh = open(os.path.join(out_dir, "log.txt"), "a")
    report = {"stamp": stamp, "host": m.HOST, "disk": a.disk, "dry_run": a.dry_run, "ok": False, "steps": {}}

    def finish(ok, why):
        report["ok"], report["why"] = ok, why
        with open(os.path.join(out_dir, "report.json"), "w") as f:
            json.dump(report, f, indent=2)
        log(("OK" if ok else "FAILED") + f": {why}  (report: {out_dir}/report.json)", fh)
        fh.close()
        return 0 if ok else 1

    expected_token = f"WIPE {m.HOST} {a.disk}"
    if a.confirm != expected_token:
        return finish(False, f"confirm token mismatch; pass --confirm \"{expected_token}\"")

    st, problems = preflight(a.disk, fh)
    report["state_before"] = st
    if problems:
        return finish(False, "preflight refused: " + "; ".join(problems))
    log("preflight passed", fh)
    if a.dry_run:
        log("dry run: would reboot to netboot (if installed), run the headless installer on " + a.disk +
            ", then boot the installed disk and verify ssh + springboard", fh)
        return finish(True, "dry run only")

    pw = os.environ.get(a.password_env) or m.login_password() or "testpass12"
    if st == "installed":
        log("midget is installed: BootNext to PXE and reboot into the live lab ISO", fh)
        if m.cmd_bootnext("pxe"):
            return finish(False, "no PXE boot entry")
        m.schedule_power("reboot")
        if not wait_live(360):
            return finish(False, "midget did not come back on the live ISO within 6 min")
    st = m.state()
    if st != "live":
        return finish(False, f"expected the live ISO before erasing, found {st}")
    st, problems = preflight(a.disk, fh)
    if problems:
        return finish(False, "re-check on the live system refused: " + "; ".join(problems))

    # The live medium only fetches the installer at boot; a push made after that boot is not running yet.
    # Pull the lab sync now and refuse to erase unless the installer on the live system is the pushed build.
    import hashlib
    sync_bin = os.path.join(REPO, "out/lab/sync/bifrost-installer")
    if not os.path.exists(sync_bin):
        return finish(False, f"{sync_bin} missing; run tools/lab-push.sh first")
    want = hashlib.sha256(open(sync_bin, "rb").read()).hexdigest()
    log(m.ssh("/usr/local/bin/bifrost-lab-sync 2>&1 | tail -n 3", 120).strip(), fh)
    got = m.ssh("sha256sum /usr/bin/bifrost-installer | cut -d' ' -f1", 20).strip()
    log(f"installer on live: {got[:16]}  pushed: {want[:16]}", fh)
    if got != want:
        return finish(False, "live installer is not the pushed build; refusing to erase")
    log(f"erasing {a.disk} with the headless installer (user {a.user})", fh)
    m.ssh("systemctl stop bifrost-installer 2>/dev/null; rm -f /tmp/wipe.rc /tmp/wipe.log", 30)
    cmd = (f"nohup sh -c 'BIFROST_TRACE=1 BIFROST_AUTO={a.user}:{pw}:{a.disk} bifrost-installer > /tmp/wipe.log 2>&1; "
           f"echo $? > /tmp/wipe.rc' >/dev/null 2>&1 &")
    m.ssh(cmd, 20)
    t0 = time.time()
    rc = None
    while time.time() - t0 < a.timeout:
        time.sleep(15)
        got = m.ssh("cat /tmp/wipe.rc 2>/dev/null", 10).strip()
        tail = m.ssh("tail -n 1 /tmp/wipe.log 2>/dev/null", 10).strip()
        log(f"installing ({int(time.time() - t0)}s): {tail[:160]}", fh)
        if got:
            rc = int(got) if got.lstrip("-").isdigit() else -1
            break
    report["install_seconds"] = round(time.time() - t0)
    report["install_rc"] = rc
    m.ssh("tail -n 200 /tmp/wipe.log > /tmp/wipe-tail.log 2>/dev/null", 20)
    open(os.path.join(out_dir, "installer-tail.log"), "w").write(m.ssh("cat /tmp/wipe-tail.log", 20))
    if rc != 0:
        return finish(False, f"installer exit {rc} (see installer-tail.log)")

    log("installed: BootNext to the BifrOSt disk, reboot", fh)
    if m.cmd_bootnext("disk"):
        return finish(False, "no BifrOSt boot entry after install")
    m.schedule_power("reboot")
    ok = m.wait(lambda: m.state() == "installed", 300, 2)
    report["state_after"] = m.state()
    if not ok:
        return finish(False, "installed system did not answer on ssh with the lab key within 5 min")
    time.sleep(8)
    facts = m.ssh("echo springboard=$(systemctl is-active springboard 2>&1); echo kernel=$(uname -r); ls /dev/dri | tr '\\n' ' '", 30)
    report["installed_facts"] = facts.strip()
    open(os.path.join(out_dir, "installed-facts.txt"), "w").write(facts)
    springboard_ok = "springboard=active" in facts
    return finish(springboard_ok, "wiped and reinstalled; " + ("springboard active" if springboard_ok else "springboard not active, see installed-facts.txt"))


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--disk", required=True, help="the only disk that may be erased, e.g. /dev/nvme0n1")
    p.add_argument("--confirm", required=True, help='must equal "WIPE <host> <disk>"')
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("--user", default=m.lab_account()[0], help="account to create (default: out/lab/account.conf, else tester)")
    p.add_argument("--password-env", default="MIDGET_WIPE_PASSWORD", help="env var holding the new account password (default: the lab account's password file)")
    p.add_argument("--timeout", type=int, default=1800, help="seconds to wait for the installer")
    return run(p.parse_args())


if __name__ == "__main__":
    sys.exit(main())
