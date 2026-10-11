#!/usr/bin/env python3
"""Autonomous BifrOSt reinstall in the x86 QEMU VM: build, boot the live ISO, drive the real installer UI by keyboard,
screenshot every page, time every phase, boot the installed disk, unlock, open apps, write a report. No ISO rebuild.

usage: tools/vm-cycle.py [--skip-build] [--iso PATH] [--user NAME] [--password PW] [--no-desktop] [--label TEXT]
Output: out/vm-runs/<stamp>/{report.md,timings.json,*.png}. Every wait is bounded; a failed phase is recorded and the run goes on
as far as it can. Stdlib only. Needs the lab ISO (it has the lab SSH key and the screenshot hook), Hyprland workspace 2 for the window.
"""
import argparse, glob, json, os, subprocess, sys, time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(REPO)
KEY = "out/lab/id_ed25519"
SSH = ["ssh", "-i", KEY, "-p", "2252", "-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no", "-o", "BatchMode=yes", "-o", "ConnectTimeout=3", "-o", "LogLevel=ERROR", "root@127.0.0.1"]
SCP = ["scp", "-i", KEY, "-P", "2252", "-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no", "-o", "LogLevel=ERROR"]
QMP = ["python3", "tools/arch-vm-qmp.py", "out/arch-vm/installer-qmp.sock", "1280", "720"]
# evdev key codes for the characters the account form needs
CODES = {**{c: 30 + i for i, c in enumerate("asdfghjkl")}, **{c: 16 + i for i, c in enumerate("qwertyuiop")}, **{c: 44 + i for i, c in enumerate("zxcvbnm")},
         **{str(d): 2 + (d - 1) % 10 for d in range(1, 10)}, "0": 11}
ENTER, TAB, DOWN = 28, 15, 108

T0 = time.time()
timings: dict = {}
notes: list = []
out_dir = ""


def sh(cmd, timeout=60, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, **kw)


def ssh(cmd, timeout=30):
    try:
        r = sh(SSH + [cmd], timeout=timeout)
        return r.stdout
    except subprocess.TimeoutExpired:
        return ""


def mark(name, since=None):
    t = time.time()
    timings[name] = round(t - (since or T0), 1)
    print(f"[{t - T0:6.1f}s] {name}: {timings[name]}s", flush=True)
    return t


def wait_for(pred, timeout, step=1.0):
    end = time.time() + timeout
    while time.time() < end:
        try:
            if pred():
                return True
        except Exception:
            pass
        time.sleep(step)
    return False


def shot_vm(name):
    """A QMP screendump of the whole VM screen."""
    path = os.path.join(out_dir, f"{name}.png")
    sh(QMP + ["shot", path], timeout=20)
    return path


def shot_installer(name):
    """The installer's own frame (lab hook): exact pixels, no window chrome."""
    ssh("touch /run/bifrost-shot-on")
    time.sleep(1.2)
    path = os.path.join(out_dir, f"{name}.png")
    r = subprocess.run(SSH + ["cat /run/bifrost-screen.png"], capture_output=True, timeout=30)
    if r.stdout[:4] == b"\x89PNG":
        open(path, "wb").write(r.stdout)
    else:
        shot_vm(name)
    return path


def keys(*codes):
    """Press and release each key code in the installer via its input FIFO."""
    lines = "".join(f"key {c} 1\nkey {c} 0\n" for c in codes)
    ssh(f"printf '{lines}' > /run/bifrost-input")
    time.sleep(0.6)


def type_text(text):
    for ch in text:
        keys(CODES[ch])


def main():
    global out_dir
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip-build", action="store_true")
    ap.add_argument("--iso")
    ap.add_argument("--user", default="tester")
    ap.add_argument("--password", default="testpass12")
    ap.add_argument("--headless", action="store_true", help="no VM window (default: the headed VM on Hyprland workspace 2, so you can watch)")
    ap.add_argument("--loop", type=int, default=0, help="run N cycles back to back and print a timing table")
    ap.add_argument("--target", choices=["vm", "midget"], default="vm", help="vm: the QEMU VM; midget: the real ThinkCentre on titan's eno2 (reboots itself into netboot via UEFI BootNext)")
    ap.add_argument("--disk", default="/dev/nvme0n1", help="midget: the disk the installer must erase (checked against what the live system sees)")
    ap.add_argument("--no-desktop", action="store_true")
    ap.add_argument("--label", default="")
    a = ap.parse_args()
    if a.loop:
        return loop(a)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_dir = os.path.join(REPO, "out/vm-runs", stamp)
    os.makedirs(out_dir, exist_ok=True)

    if a.target == "midget":
        return midget(a)
    # 1. the installer binary (fast profile)
    t = time.time()
    if not a.skip_build:
        r = sh(["cargo", "build", "--profile", "fast", "--bin", "installer"], timeout=900, cwd=os.path.join(REPO, "springboard"))
        if r.returncode:
            notes.append("build failed: " + r.stderr[-400:])
            return finish(a, ok=False)
    mark("build_installer", t)

    # 2. a blank disk and the live ISO
    t = time.time()
    iso = a.iso or (sorted(glob.glob("out/iso-lab-*/*.iso"), key=os.path.getmtime) or [None])[-1]
    if not iso:
        notes.append("no lab ISO found")
        return finish(a, ok=False)
    sh(["pkill", "-f", "name BifrOSt Installer"])
    time.sleep(2)
    sh(["truncate", "-s", "0", "out/arch-vm/installer-target.raw"])
    sh(["truncate", "-s", "16G", "out/arch-vm/installer-target.raw"])
    sh(["tools/arch-vm-iso.sh"], timeout=60, env={**os.environ, "ISO": iso, **({"HEADLESS": "1"} if a.headless else {})})
    up = wait_for(lambda: ssh("echo up").strip() == "up", 120, 2)
    mark("live_boot_to_ssh", t)
    if not up:
        notes.append("live system never answered on ssh")
        return finish(a, ok=False)

    # 3. the installer and live unit under test replace the ISO's copies; screens come from its own frame hook
    t = time.time()
    sh(SCP + ["springboard/target/fast/installer", "root@127.0.0.1:/usr/bin/bifrost-installer"], timeout=60)
    sh(SCP + ["packaging/archiso/airootfs/usr/lib/systemd/system/bifrost-installer.service", "root@127.0.0.1:/tmp/bifrost-installer.service"], timeout=60)
    ssh("install -m644 /tmp/bifrost-installer.service /usr/lib/systemd/system/bifrost-installer.service; systemctl daemon-reload")
    ssh("systemctl restart bifrost-installer", 60)
    wait_for(lambda: ssh("test -p /run/bifrost-input && systemctl is-active bifrost-installer").strip().endswith("active"), 40, 1)
    time.sleep(3)
    mark("installer_ready", t)
    shot_installer("01-welcome")

    # 4. drive the UI with the keyboard: Welcome, Language, Account, Disk, Confirm
    t = time.time()
    keys(ENTER)
    shot_installer("02-language")
    keys(ENTER)
    shot_installer("03-account-empty")
    type_text(a.user)  # full name field
    keys(TAB)
    keys(TAB)  # username is filled from the name; skip to the password
    type_text(a.password)
    keys(TAB)
    type_text(a.password)
    shot_installer("04-account-filled")
    keys(ENTER)
    shot_installer("05-disk")
    keys(DOWN)
    shot_installer("06-disk-selected")
    keys(ENTER)
    shot_installer("07-confirm")
    t_start = time.time()
    keys(ENTER)  # Erase and Install
    mark("ui_walkthrough", t)

    # 5. the install: sample the screen, stop at the unmount
    seen = {}
    saw_target_mount = False

    def progress():
        nonlocal saw_target_mount
        log = ssh("journalctl -b -t bifrost-installer --no-pager | grep -a 'bifrost-installer: \\[' | tail -1")
        if log.strip() and log.strip() not in seen:
            seen[log.strip()] = time.time() - t_start
            n = len(seen)
            if n % 2 == 1:
                shot_installer(f"08-progress-{n:02d}")
        failed = ssh("journalctl -b -t bifrost-installer --no-pager | grep -a 'FAILED' | tail -1").strip()
        # Progress messages are rendered in the native frame and are not
        # always emitted to journald. The target being unmounted is the
        # authoritative completion signal; a FAILED line distinguishes the
        # error page from the installed page.
        mounted = ssh("mountpoint -q /mnt && echo mounted || echo free").strip() == "mounted"
        saw_target_mount |= mounted
        return saw_target_mount and not failed and not mounted

    ok = wait_for(progress, 420, 4)
    stage = {k.split("] ", 1)[-1] if "] " in k else k: round(v, 1) for k, v in seen.items()}
    timings["install_stages_s"] = stage
    mark("install_total", t_start)
    time.sleep(2)
    shot_installer("09-done")
    failed = ssh("journalctl -b -t bifrost-installer --no-pager | grep -a 'FAILED' | tail -2").strip()
    if failed or not ok:
        notes.append(f"install did not finish cleanly: {failed or 'timeout'}")
        return finish(a, ok=False)

    # 5b. the installed ESP boots the BifrOSt loader from its own namespace, and the firmware entry names it
    esp = ssh("p=$(lsblk -lnpo NAME,PARTTYPE /dev/vda | awk '$2==\"c12a7328-f81f-11d2-ba4b-00a0c93ec93b\"{print $1}');"
              " mkdir -p /run/esp && mount -o ro $p /run/esp && cmp /run/esp/EFI/BIFROST/BOOTx64.efi /run/esp/EFI/BOOT/BOOTx64.efi"
              " && test -s /run/esp/EFI/BIFROST/boot.cfg && echo layout-ok; umount /run/esp;"
              " efibootmgr -v | grep -i 'BifrOSt' | grep -qi 'EFI.BIFROST.BOOTX64.EFI' && echo entry-ok", 30)
    timings["esp_namespace"] = esp.split()
    if "layout-ok" not in esp or "entry-ok" not in esp:
        notes.append(f"installed ESP is not on the BifrOSt namespace: {esp.strip() or 'no answer'}")
        return finish(a, ok=False)

    # 6. boot the installed disk, time it to the lock screen
    t = time.time()
    sh(["pkill", "-f", "name BifrOSt Installer"])
    time.sleep(2)
    sh(["tools/arch-vm-iso.sh"], timeout=60, env={**os.environ, "ISO_NONE": "1", "KEEP_VARS": "1", **({"HEADLESS": "1"} if a.headless else {})})
    first: list = [None]

    def lock_shown():
        p = shot_vm("tmp-boot")
        size = os.path.getsize(p) if os.path.exists(p) else 0
        if size > 200_000 and first[0] is None:  # a wallpaper compresses far worse than black or a text console
            first[0] = time.time()
        return first[0] is not None

    booted = wait_for(lock_shown, 90, 1)
    mark("installed_boot_to_lock", t)
    if os.path.exists(os.path.join(out_dir, "tmp-boot.png")):
        os.replace(os.path.join(out_dir, "tmp-boot.png"), os.path.join(out_dir, "10-lock.png"))
    if not booted:
        notes.append("the installed system never showed the lock screen (see 10-lock.png)")
        return finish(a, ok=False)

    # 7. unlock and open apps in desktop mode
    if not a.no_desktop:
        sh(QMP + ["key", "enter"], timeout=20)
        time.sleep(3)
        shot_vm("11-desktop")
        for name, x in ([("settings", 543), ("terminal", 589), ("files", 720)]):
            sh(QMP + ["tap", str(x), "700"], timeout=20)
            time.sleep(2)
            shot_vm(f"12-{name}")
    return finish(a, ok=True)


def loop(a):
    """N cycles in fresh processes (each leaves its own report); then one table of the timings."""
    rows = []
    for i in range(a.loop):
        cmd = [sys.executable, os.path.abspath(__file__), "--label", f"loop-{i + 1}/{a.loop}"] + ([] if i == 0 else ["--skip-build"]) + [x for x in sys.argv[1:] if x not in ("--loop", str(a.loop))]
        subprocess.run(cmd)
        runs = sorted(glob.glob(os.path.join(REPO, "out/vm-runs/*/timings.json")))
        if runs:
            rows.append(json.load(open(runs[-1])))
    keys = ["live_boot_to_ssh", "installer_ready", "ui_walkthrough", "install_total", "installed_boot_to_lock", "total_s"]
    print("\n" + " ".join(f"{k[:14]:>14}" for k in ["run"] + keys))
    for i, r in enumerate(rows):
        print(" ".join(f"{str(v):>14}" for v in [f"{i + 1}{'' if r.get('ok') else ' FAIL'}"] + [r.get(k, "-") for k in keys]))
    return 0 if all(r.get("ok") for r in rows) else 1



MIDGET = os.environ.get("MIDGET_HOST", "192.168.20.140")


def use_midget():
    """Point ssh/scp at midget (the lab key works on a live lab ISO and, since the installer leaves it, on the installed system)."""
    opts = ["-i", KEY, "-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no", "-o", "BatchMode=yes", "-o", "ConnectTimeout=3", "-o", "LogLevel=ERROR"]
    SSH[:] = ["ssh", *opts, f"root@{MIDGET}"]
    SCP[:] = ["scp", *opts]


def ping():
    return sh(["ping", "-c1", "-W1", MIDGET], timeout=5).returncode == 0


def midget_state():
    """down | live (netbooted lab ISO) | installed (our key works, no /run/archiso) | up (answers ping, no ssh)"""
    if not ping():
        return "down"
    out = ssh("test -d /run/archiso && echo live || echo installed", 8).strip()
    return out if out in ("live", "installed") else "up"


def to_netboot(timings_key):
    """Make midget boot the iVentoy live ISO next, without touching it: UEFI BootNext = the PXE entry, then reboot."""
    t = time.time()
    st = midget_state()
    if st == "live":
        print("midget is already on the live ISO", flush=True)
        mark(timings_key, t)
        return True
    if st in ("live", "installed"):
        boot = ssh("efibootmgr", 15)
        pxe = [l for l in boot.splitlines() if l.startswith("Boot") and ("PXE" in l or "IPv4" in l)]
        pxe = sorted(pxe, key=lambda l: ("IPv4" not in l, l))
        if not pxe:
            notes.append("no PXE entry in the UEFI boot list: " + boot[-300:])
            return False
        num = pxe[0][4:8]
        ssh(f"efibootmgr -n {num} >/dev/null; nohup sh -c 'sleep 1; systemctl reboot --force --force' >/dev/null 2>&1 &", 15)
        print(f"midget was {st}; BootNext={num} ({pxe[0][9:60].strip()}), rebooting", flush=True)
        wait_for(lambda: not ping(), 60, 1)
    else:
        print("midget is unreachable: power-cycle it into the network boot (F12 / boot menu) - waiting up to 5 min", flush=True)
    ok = wait_for(lambda: midget_state() == "live", 300 if st not in ("live", "installed") else 180, 2)
    mark(timings_key, t)
    return ok


def midget_shot(name):
    """The installed shell's own frame through its control port (bash /dev/tcp over ssh)."""
    script = "exec 3<>/dev/tcp/127.0.0.1/7777; echo 'shot 1' >&3; timeout 20 cat <&3"
    r = subprocess.run(SSH + [f"bash -c {json.dumps(script)}"], capture_output=True, timeout=40)
    blob = r.stdout
    if blob[:4] == b"RIMG":
        import struct
        w, h = struct.unpack("<II", blob[4:12])
        try:
            from PIL import Image
            im = Image.frombuffer("RGBA", (w, h), blob[12:12 + w * h * 4], "raw", "BGRA", 0, 1).convert("RGB")
            path = os.path.join(out_dir, f"{name}.png")
            im.save(path)
            return path
        except Exception as e:  # PIL missing or odd format: keep the raw frame
            open(os.path.join(out_dir, f"{name}.raw"), "wb").write(blob)
            notes.append(f"could not convert {name}: {e}")
    return None


def midget(a):
    global out_dir
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_dir = os.path.join(REPO, "out/vm-runs", stamp + "-midget")
    os.makedirs(out_dir, exist_ok=True)
    use_midget()
    timings["target"] = "midget"

    t = time.time()
    if not a.skip_build:
        r = sh(["cargo", "build", "--profile", "fast", "--bin", "installer"], timeout=900, cwd=os.path.join(REPO, "springboard"))
        if r.returncode:
            notes.append("build failed: " + r.stderr[-400:])
            return finish(a, ok=False)
    mark("build_installer", t)

    sh([sys.executable, "tools/midget.py", "labboot", "on"], timeout=60)
    if not to_netboot("reboot_to_live"):
        notes.append("midget never came up as a live system")
        return finish(a, ok=False)

    # safety: exactly one erasable disk, and it is the one we were told to erase
    disks = [l.split() for l in ssh("lsblk -dnpo NAME,SIZE,TYPE,RM -b | awk '$3==\"disk\" && $4==0 && $2>8000000000'", 15).splitlines() if l.strip()]
    names = [d[0] for d in disks if "/loop" not in d[0] and "/nbd" not in d[0] and "/sr" not in d[0]]
    notes.append(f"disks the installer will list: {names}")
    if names != [a.disk]:
        notes.append(f"refusing to erase: expected exactly {a.disk}, found {names}")
        return finish(a, ok=False)

    t = time.time()
    # the service pulls its binary from titan (bifrost-lab-sync) on every start, so publish through that path, then prove it landed
    sh(["tools/lab-push.sh", "--skip-cargo"], timeout=60)
    ssh("systemctl restart bifrost-installer", 60)
    time.sleep(4)
    want = sh(["sha256sum", "out/lab/sync/bifrost-installer"]).stdout.split()[0]  # lab-push syncs a stripped copy
    have = ssh("sha256sum /usr/bin/bifrost-installer").split()[:1]
    if have != [want]:
        notes.append(f"midget runs a different installer than the one just built ({have} vs {want}); is the lab sync server on :8078 reachable?")
        return finish(a, ok=False)
    mark("installer_ready", t)
    shot_installer("01-welcome")
    t = time.time()
    keys(ENTER)
    shot_installer("02-language")
    keys(ENTER)
    shot_installer("03-account-empty")
    type_text(a.user)
    keys(TAB)
    keys(TAB)
    type_text(a.password)
    keys(TAB)
    type_text(a.password)
    shot_installer("04-account-filled")
    keys(ENTER)
    shot_installer("05-disk")
    keys(DOWN)
    shot_installer("06-disk-selected")
    keys(ENTER)
    shot_installer("07-confirm")
    t_start = time.time()
    keys(ENTER)
    mark("ui_walkthrough", t)

    seen = {}

    def progress():
        log = ssh("journalctl -b -t bifrost-installer --no-pager | grep -a 'bifrost-installer: \\[' | tail -1")
        if log.strip() and log.strip() not in seen:
            seen[log.strip()] = time.time() - t_start
            if len(seen) % 2 == 1:
                shot_installer(f"08-progress-{len(seen):02d}")
        return "Finishing" in log and ssh("mountpoint -q /mnt && echo mounted || echo free").strip() == "free"

    ok = wait_for(progress, 900, 5)
    timings["install_stages_s"] = {(k.split("] ", 1)[-1] if "] " in k else k): round(v, 1) for k, v in seen.items()}
    mark("install_total", t_start)
    time.sleep(2)
    shot_installer("09-done")
    failed = ssh("journalctl -b -t bifrost-installer --no-pager | grep -a 'FAILED' | tail -2").strip()
    if failed or not ok:
        notes.append(f"install did not finish cleanly: {failed or 'timeout'}")
        ssh("journalctl -b -t bifrost-installer --no-pager | tail -40 > /tmp/install-tail.log")
        open(os.path.join(out_dir, "install-tail.log"), "w").write(ssh("cat /tmp/install-tail.log"))
        return finish(a, ok=False)

    # boot the installed disk, time it to the first ssh, then judge the shell
    t = time.time()
    ssh("nohup sh -c 'sleep 1; systemctl reboot --force --force' >/dev/null 2>&1 &", 15)
    wait_for(lambda: not ping(), 60, 1)
    up = wait_for(lambda: midget_state() == "installed", 240, 2)
    mark("installed_boot_to_ssh", t)
    if not up:
        notes.append("the installed system never answered on ssh with the lab key (is the installer lab-aware? is the NIC up?)")
        return finish(a, ok=False)
    time.sleep(8)
    facts = ssh("echo springboard=$(systemctl is-active springboard); echo uptime=$(cut -d' ' -f1 /proc/uptime); ls /dev/dri | tr '\\n' ' '; echo; "
                "systemd-analyze 2>&1 | head -1; journalctl -b -u springboard --no-pager | tail -15", 30)
    open(os.path.join(out_dir, "installed-facts.txt"), "w").write(facts)
    timings["springboard"] = next((l.split("=", 1)[1] for l in facts.splitlines() if l.startswith("springboard=")), "?")
    if timings["springboard"] != "active":
        notes.append("springboard is " + timings["springboard"] + " on the installed system (see installed-facts.txt)")
    midget_shot("10-installed")
    return finish(a, ok=timings["springboard"] == "active")


def finish(a, ok):
    timings["total_s"] = round(time.time() - T0, 1)
    timings["ok"] = ok
    timings["label"] = a.label
    json.dump({**timings, "notes": notes}, open(os.path.join(out_dir, "timings.json"), "w"), indent=1)
    lines = [f"# VM cycle {os.path.basename(out_dir)} {'OK' if ok else 'FAILED'}", ""]
    for k, v in timings.items():
        if k not in ("install_stages_s", "ok", "label"):
            lines.append(f"- {k}: {v}")
    for k, v in timings.get("install_stages_s", {}).items():
        lines.append(f"  - {v:>6}s  {k}")
    lines += [""] + [f"- NOTE: {n}" for n in notes] + ["", "Screens: " + ", ".join(sorted(os.path.basename(p) for p in glob.glob(out_dir + "/*.png")))]
    open(os.path.join(out_dir, "report.md"), "w").write("\n".join(lines) + "\n")
    print("\n".join(lines))
    print(f"\nreport: {out_dir}/report.md")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
