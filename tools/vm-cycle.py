#!/usr/bin/env python3
"""Autonomous BifrOSt reinstall in the x86 QEMU VM, no ISO rebuild, no LLM in the loop.

Default (the installer's unattended path, as `midget reinstall`): publish the installer to the lab server
(tools/lab-push.sh), wipe the VM disk and seed it with an ESP holding EFI/BIFROST/unattended.toml (disk WWN, VM MAC,
password URL on titan), boot the lab ISO; the live system pulls the new installer (bifrost-lab-sync), which finds the
preset, installs without input and boots the new system. Then: update guard, shell ready, unlock, apps by id.
--ui drives the on-screen installer by keyboard instead (Welcome, Language, Account, Disk, Confirm).

usage: tools/vm-cycle.py [--ui] [--skip-build] [--iso PATH] [--user NAME] [--password PW] [--no-desktop] [--label TEXT]
Output: out/vm-runs/<stamp>/{report.md,timings.json,*.png}. Every wait is bounded; a failed phase is recorded and the run goes on
as far as it can. Stdlib only. Needs the lab ISO (it has the lab SSH key and the screenshot hook), Hyprland workspace 2 for the window.
"""
import argparse, glob, json, os, shlex, subprocess, sys, time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(REPO)
KEY = "out/lab/id_ed25519"
SSH = ["ssh", "-i", KEY, "-p", "2252", "-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no", "-o", "BatchMode=yes", "-o", "ConnectTimeout=3", "-o", "LogLevel=ERROR", "root@127.0.0.1"]
SCP = ["scp", "-i", KEY, "-P", "2252", "-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no", "-o", "LogLevel=ERROR"]
QMP = ["python3", "tools/arch-vm-qmp.py", "out/arch-vm/installer-qmp.sock", "1280", "720"]
# the VM's identity in the unattended preset (tools/arch-vm-iso.sh gives the disk this WWN and the NIC this MAC)
VM_WWN = os.environ.get("VM_WWN", "0x5000c500b1f00001")
VM_MAC = os.environ.get("VM_MAC", "52:54:00:b1:f0:01")
VM_DISK = "/dev/sda"
TARGET = "out/arch-vm/installer-target.raw"
LAB_SYNC = "out/lab/sync"
PASSWORD_URL = "http://10.99.0.1:8078/lab/tester.pw"  # the guest reaches titan's lab address through QEMU's user network
MIRROR_URL = "http://10.99.0.1:8078/mirror/"  # tools/lab-mirror.sh: pacstrap installs every package from titan
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
    ap.add_argument("--ui", action="store_true", help="drive the on-screen installer by keyboard instead of the unattended preset")
    ap.add_argument("--reuse", action="store_true", help="no reinstall: boot the installed VM disk and run the checks after the install")
    ap.add_argument("--compositor", choices=["labwc", "bifrost-wm"], default="labwc",
                    help="bifrost-wm: after the stock checks, install the window server from this tree and check again under it")
    ap.add_argument("--label", default="")
    a = ap.parse_args()
    if a.loop:
        return loop(a)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_dir = os.path.join(REPO, "out/vm-runs", stamp)
    os.makedirs(out_dir, exist_ok=True)

    if a.target == "midget":
        return midget(a)
    if a.reuse:
        return vm_installed(a)
    if not a.ui:
        return vm_unattended(a)
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
    esp = ssh(f"p=$(lsblk -lnpo NAME,PARTTYPE {VM_DISK} | awk '$2==\"c12a7328-f81f-11d2-ba4b-00a0c93ec93b\"{{print $1}}');"
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
        desktop_checks()
    return finish(a, ok=True)


def control(line, timeout=15):
    """One line to the shell's control socket (127.0.0.1:7777) over ssh; the reply text."""
    script = ("import socket; s=socket.create_connection(('127.0.0.1',7777),5); s.settimeout(10); s.sendall(" + repr((line + "\n").encode()) + "); "
              "d=b''\nwhile True:\n c=s.recv(65536)\n if not c: break\n d+=c\nprint(d.decode(),end='')")
    return ssh("python3 -c " + shlex.quote(script), timeout)


def screen():
    st = control("state")
    return next((l.split("=", 1)[1] for l in st.splitlines() if l.startswith("screen=")), "")


def desktop_checks(prefix=""):
    """Unlock (the lab account has no passcode: Enter), then open Settings, Terminal and Files by id, one screen each."""
    t = time.time()
    shot_vm(f"{prefix}10-lock")
    sh(QMP + ["key", "enter"], timeout=20)
    if not wait_for(lambda: screen() not in ("", "lock"), 30, 1):
        notes.append(f"the lock screen did not unlock (screen={screen() or 'no answer'})")
    mark(f"{prefix}unlock", t)
    time.sleep(2)
    shot_vm(f"{prefix}11-desktop")
    for name, app in (("settings", "bifrost.settings"), ("terminal", "is.olibuijr.term"), ("files", "is.olibuijr.files.desktop")):
        reply = control(f"open-app {app}").strip()
        if reply != "ok":
            notes.append(f"open-app {app}: {reply or 'no answer'}")
        time.sleep(3)
        shot_vm(f"{prefix}12-{name}")


def lab_password():
    """The tester password: out/lab/tester.password (untracked, 0600, made once), published for the installer only."""
    pw_file = "out/lab/tester.password"
    if not os.path.exists(pw_file):
        import secrets
        fd = os.open(pw_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(secrets.token_urlsafe(12) + "\n")
    os.makedirs(os.path.join(LAB_SYNC, "lab"), exist_ok=True)
    with open(os.path.join(LAB_SYNC, "lab", "tester.pw"), "w") as f:
        f.write(open(pw_file).read().strip() + "\n")
    return pw_file


def lab_http():
    """The lab server bifrost-lab-sync and the preset's password URL use (10.99.0.1:8078, serving out/lab/sync). A server
    left running from a replaced out/lab/sync answers 404 for everything (its working directory is gone): restart it."""
    served = lambda: sh(["curl", "-s", "--max-time", "3", "-o", "/dev/null", "-w", "%{http_code}", "http://10.99.0.1:8078/manifest"]).stdout == "200"
    if served():
        return True
    if not os.path.exists(os.path.join(LAB_SYNC, "bifrost-installer")):
        return False
    sh(["pkill", "-f", "http.server 8078 --bind 10.99.0.1"])
    time.sleep(1)
    subprocess.Popen(["python3", "-m", "http.server", "8078", "--bind", "10.99.0.1"], cwd=LAB_SYNC, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, start_new_session=True)
    return wait_for(served, 10, 0.5)


def preset_toml(user):
    """The same preset `bifrost-mcp lifecycle reinstall` writes (springboard/mcp/src/lifecycle.rs), for the VM's disk and NIC."""
    now = int(time.time())
    return (f"# written by tools/vm-cycle.py; read by bifrost-installer on the lab ISO\nversion = 1\ndisk_id = \"{VM_WWN}\"\n"
            f"mac = \"{VM_MAC}\"\nhostname = \"bifrost\"\nlocale = \"en_US.UTF-8\"\nkeyboard = \"us\"\ntimezone = \"Atlantic/Reykjavik\"\n"
            f"username = \"{user}\"\nfull_name = \"Tester\"\nautologin = true\nmcp = true\npassword_url = \"{PASSWORD_URL}\"\n"
            f"mirror = \"{MIRROR_URL}\"\n"
            f"created_epoch = {now}\nexpires_s = 1800\njob = \"{now}-vm-reinstall\"\n")


def seed_disk(preset):
    """Wipes the VM disk and leaves a GPT with one ESP holding EFI/BIFROST/unattended.toml (the installer erases it all)."""
    sh(["truncate", "-s", "0", TARGET])
    sh(["truncate", "-s", "16G", TARGET])
    r = sh(["sfdisk", "--quiet", TARGET], input="label: gpt\nstart=2048, size=131072, type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B\n")
    if r.returncode:
        return "sfdisk: " + r.stderr.strip()
    img = f"{TARGET}@@1M"
    path = os.path.join(out_dir, "unattended.toml")
    open(path, "w").write(preset)
    for cmd in (["mformat", "-i", img, "-F", "-T", "131072", "-v", "ESP", "::"], ["mmd", "-i", img, "::/EFI", "::/EFI/BIFROST"],
                ["mcopy", "-i", img, path, "::/EFI/BIFROST/unattended.toml"]):
        r = sh(cmd)
        if r.returncode:
            return " ".join(cmd[:1]) + ": " + r.stderr.strip()
    return ""


def vm_unattended(a):
    """The installer's unattended path in the VM; phases timed like `midget reinstall`."""
    # 1. the installer, shell and session under test, published where the live system pulls them
    t = time.time()
    r = sh(["tools/lab-push.sh"] + (["--skip-cargo"] if a.skip_build else []), timeout=1200)
    if r.returncode:
        notes.append("lab-push failed: " + (r.stderr or r.stdout)[-400:])
        return finish(a, ok=False)
    lab_password()
    # every package pacstrap installs, local (tools/lab-mirror.sh; only changed packages download)
    r = sh(["tools/lab-mirror.sh"], timeout=1800)
    if r.returncode:
        notes.append("lab-mirror failed: " + (r.stderr or r.stdout)[-400:])
        return finish(a, ok=False)
    if not lab_http():
        notes.append("the lab server (10.99.0.1:8078) is not up and out/lab/sync has no installer")
        return finish(a, ok=False)
    mark("publish", t)

    # 2. a wiped disk carrying only the preset, then the live ISO
    t = time.time()
    iso = a.iso or (sorted(glob.glob("out/iso-lab-*/*.iso"), key=os.path.getmtime) or [None])[-1]
    if not iso:
        notes.append("no lab ISO found")
        return finish(a, ok=False)
    sh(["pkill", "-f", "name BifrOSt Installer"])
    time.sleep(2)
    err = seed_disk(preset_toml(a.user))
    if err:
        notes.append("could not seed the preset: " + err)
        return finish(a, ok=False)
    sh(["tools/arch-vm-iso.sh"], timeout=60, env={**os.environ, "ISO": iso, "VM_WWN": VM_WWN, "VM_MAC": VM_MAC, **({"HEADLESS": "1"} if a.headless else {})})
    live = lambda: ssh("test -d /run/archiso && echo live || echo installed", 8).strip()
    if not wait_for(lambda: live() == "live", 180, 2):
        notes.append("the live system never answered on ssh")
        return finish(a, ok=False)
    mark("live_boot_to_ssh", t)
    # the boot-time sync gives up after two 1 s tries; run it again now the network is up and prove the new installer
    # landed (lab-sync restarts the installer when it changed, and the new one finds the preset)
    want = sh(["sha256sum", os.path.join(LAB_SYNC, "bifrost-installer")]).stdout.split()[:1]
    synced = lambda: ssh("sha256sum /usr/bin/bifrost-installer", 10).split()[:1] == want
    if not synced():
        ssh("systemctl restart bifrost-lab-sync", 90)
    if not wait_for(synced, 60, 3):
        notes.append("the live system runs a different installer than the one just published (lab-sync could not fetch it)")
        return finish(a, ok=False)
    # an unattended run may already have started with the network below: stop it and undo a half-mounted target first
    # (the preset stays in /run/bifrost-unattended.toml, so the restarted installer finds it even on a wiped disk)
    ssh("systemctl stop bifrost-installer; umount -R /mnt 2>/dev/null; true", 30)
    # QEMU's user network: the ISO's resolver (Quad9 through the NAT) is slow and answers IPv6 the VM cannot route, so
    # pacstrap's downloads timed out. QEMU's own DNS proxy (10.0.2.3) uses titan's resolver.
    ssh("for i in /sys/class/net/*; do n=${i##*/}; [ $n = lo ] && continue; resolvectl dns $n 10.0.2.3; resolvectl domain $n '~.'; done;"
        " resolvectl flush-caches", 15)
    # the unattended run starts now the network is right
    ssh("systemctl start bifrost-installer", 30)

    # 3. the unattended install: follow the installer's own log until the machine reboots into the new system
    t = time.time()
    seen = {}

    def installing():
        j = ssh("journalctl -u bifrost-installer -b --no-pager -n 400 2>/dev/null | grep -aE 'unattended|bifrost-installer: \\[|ERROR' | tail -n 1", 15).strip()
        if j and j not in seen:
            seen[j] = round(time.time() - t, 1)
            print("    " + j[-150:], flush=True)
            if len(seen) % 2 == 1:
                shot_vm(f"08-progress-{len(seen):02d}")
        if "refused or failed" in j or "Failed to install packages" in j:
            raise RuntimeError(j)
        return live() == "installed"

    try:
        ok = wait_for(installing, 1200, 5)
    except RuntimeError as e:
        notes.append(f"the installer refused or failed the preset: {e}")
        return finish(a, ok=False)
    timings["install_log"] = {(k.split("] ", 1)[-1] if "] " in k else k)[-90:]: v for k, v in seen.items()}
    mark("install_to_installed_ssh", t)
    if not ok:
        notes.append("the installed system did not come up (no ssh with the lab key)")
        return finish(a, ok=False)
    if not any("unattended" in k for k in seen):
        notes.append("never saw an 'unattended' installer line: was the preset found?")

    return vm_installed(a, booted=True)


def vm_installed(a, booted=False):
    """The checks after an install: clean boot, shell ready, unlock, apps, then optionally the window server."""
    live = lambda: ssh("test -d /run/archiso && echo live || echo installed", 8).strip()
    if not booted:
        t = time.time()
        if live() != "installed":
            sh(["pkill", "-f", "name BifrOSt Installer"])
            time.sleep(2)
            sh(["tools/arch-vm-iso.sh"], timeout=60, env={**os.environ, "ISO_NONE": "1", "KEEP_VARS": "1", "VM_WWN": VM_WWN, "VM_MAC": VM_MAC,
                                                         **({"HEADLESS": "1"} if a.headless else {})})
        if not wait_for(lambda: live() == "installed", 180, 3):
            notes.append("the installed VM disk did not boot to ssh")
            return finish(a, ok=False)
        mark("installed_boot_to_ssh", t)
    # 4. the new system booted cleanly and the shell is ready
    t = time.time()
    def booted_ok():
        g = ssh("bifrost-update-guard status", 15) or ""
        return ("phase=accepted" in g or "phase=\n" in g + "\n") and "ready=1" in control("state")
    if not wait_for(booted_ok, 300, 5):
        notes.append("the new system did not boot cleanly (update guard phase or shell not ready)")
        open(os.path.join(out_dir, "installed-journal.txt"), "w").write(ssh("journalctl -b -u springboard --no-pager | tail -60", 30))
        return finish(a, ok=False)
    mark("shell_ready", t)
    timings["shell_version"] = ssh("ls /var/lib/pacman/local | grep springboard-shell", 10).strip()
    if not a.no_desktop:
        desktop_checks()
    if a.compositor == "bifrost-wm" and not window_server(a):
        return finish(a, ok=False)
    return finish(a, ok=not any(n.startswith(("open-app", "the lock")) for n in notes))


def window_server(a):
    """Switch the installed session to bifrost-wm (docs/window-server.md) and run the checks again under it."""
    t = time.time()
    # the window server, the session and the shell from this tree (the shell's Wayland side must match the compositor)
    if not a.skip_build:
        for cmd in (["cargo", "build", "--profile", "fast", "-p", "bifrost-wm"],
                    ["cargo", "build", "--profile", "fast", "--features", "desktop-wayland", "--bin", "shell"]):
            r = sh(cmd, timeout=1200, cwd=os.path.join(REPO, "springboard"))
            if r.returncode:
                notes.append(f"{' '.join(cmd[-2:])} build failed: " + r.stderr[-400:])
                return False
    for src, dst in (("springboard/target/fast/bifrost-wm", "/tmp/bifrost-wm"), ("packaging/arch/wayland/session", "/tmp/bifrost-session"),
                     ("springboard/target/fast/shell", "/tmp/springboard")):
        sh(SCP + [src, f"root@127.0.0.1:{dst}"], timeout=120)
    ssh("install -m755 /tmp/bifrost-wm /usr/bin/bifrost-wm && install -m755 /tmp/bifrost-session /usr/lib/bifrost/session"
        " && install -m755 /tmp/springboard /usr/bin/springboard && rm -f /var/lib/springboard/boot_attempts && systemctl reset-failed springboard"
        " && mkdir -p /etc/bifrost && echo bifrost-wm > /etc/bifrost/compositor && systemctl restart springboard", 60)
    up = wait_for(lambda: "ready=1" in control("state") and "bifrost-wm" in ssh("pgrep -a bifrost-wm", 10), 90, 3)
    timings["wm_session_ready"] = round(time.time() - t, 1)
    open(os.path.join(out_dir, "wm-journal.txt"), "w").write(ssh("journalctl -b -u springboard --no-pager | grep -aE 'bifrost-wm|springboard:' | tail -60", 30))
    if not up:
        notes.append("the session did not come up under bifrost-wm (see wm-journal.txt)")
        shot_vm("wm-10-failed")
        return False
    if not a.no_desktop:
        desktop_checks("wm-")
    return True


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
